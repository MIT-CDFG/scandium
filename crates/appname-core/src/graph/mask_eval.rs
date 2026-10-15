// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §11.2 (mask evaluation — effective-coverage formula), §2.6
//! (`MaskNode` props), §40.3 (mask.* family). DD ㉙.
//!
//! **The ONE shared §11.2 effective-coverage routine.** CP-0.5 wave-3
//! mask compositing serves raster masks — Sub-slice A unfeathered, and now
//! **Sub-slice B FEATHERED** — on BOTH the CPU reference path
//! ([`crate::io::png_io`]) and the render/GPU path (`appname-render`). Both
//! surfaces, plus the §12.1 per-op mask leg
//! ([`crate::engine::op_eval`]), resolve coverage through
//! [`resolve_effective_coverage_with`] here — one function, so the
//! normative order can never diverge between paths (O-INT ruling, on the
//! record).
//!
//! **Normative order (frozen §11.2 step list, verbatim):** sample `v`
//! (absent reads 0) → **feather** `v ← GaussianBlur(v, σ = feather_px/3)`
//! → density `v ← 1 − density·(1 − v)` → invert `v ← 1 − v` (iff
//! `inverted`). Feather/density come FIRST, invert LAST — so a
//! zero-density INVERTED mask correctly evaluates `E = 0` (NOT a no-op).
//! A DISABLED mask is a no-op (`E = 1`); the caller iterates ENABLED
//! masks only ([`ResolvedMasks::resolve`] does this).
//!
//! **THE KERNEL IS NOT OURS AND MUST NOT BE.** The feather Gaussian is
//! [`appname_algo::rgba_blur::gaussian_blur_rgba`] — the SAME code the
//! §16.3 `filter.gaussian_blur` block runs through
//! `appname_tools::blur`'s adapter. There is exactly one implementation in
//! the workspace, so §16.3 and §11.2 are at parity BY CONSTRUCTION rather
//! than by two hand-matched copies. (The kernel lives in `appname-algo`
//! because `appname-tools` already depends on THIS crate — `core → tools`
//! is a hard cargo cycle — while `appname-algo` is a true leaf. O-INT
//! architecture re-ruling.)
//!
//! **The kernel is RGBA and a mask is single-channel — deliberately.** The
//! evaluator splats coverage `v` to `[v, v, v, v]` and blurs that. Both
//! kernel paths accumulate each channel independently with no
//! cross-channel term, so every lane computes the identical scalar and
//! channel 0 of the splatted blur is BYTE-IDENTICAL to a single-channel
//! blur — measured, not assumed, by
//! `appname_algo::rgba_blur::tests::splatted_single_channel_coverage_blurs_identically_in_all_four_lanes`.
//! **The price, MEASURED, not assumed.** Transient memory is exactly 4×
//! during the blur (16 B/px vs 4 B/px); the cached plane is single-channel
//! ([`FeatheredCoverage`] keeps only channel 0), so steady state is 1×.
//! Wall-clock is NOT a flat 4×, and the shape matters:
//!
//! | feather px | σ | path | 4-lane | 1-lane twin | ratio |
//! |---|---|---|---|---|---|
//! | 12 (536² rect) | 4 | direct | 8.5 ms | 7.3 ms | **1.17×** |
//! | 24 (1072²) | 8 | direct | 76.8 ms | 56.0 ms | **1.37×** |
//! | 90 (1204²) | 30 | direct | 375 ms | 316 ms | **1.19×** |
//! | 300 (1624²) | 100 | box | 67.5 ms | 19.1 ms | **3.53×** |
//! | 1000 (3024²) | 333 | box | 649 ms | 77.4 ms | **8.39×** |
//!
//! (release build, this machine; the 1-lane twin is a structurally
//! identical single-channel port of both paths, so the two are priced on
//! the same computation — channel 0 agreed to the byte on the direct
//! rows.) The DIRECT path is dominated by per-tap addressing and clamp
//! logic that all four lanes share, so the extra lanes are nearly free.
//! The BOX path has almost no per-pixel work besides the running sum, so
//! lane count dominates — and at the top of the range the 4-lane working
//! set (140 MiB) leaves cache while the 1-lane one (35 MiB) does not,
//! which is why the ratio EXCEEDS 4×.
//!
//! So: reusing the RGBA kernel is close to free for ordinary feathers and
//! costs up to ~8× at `feather_px = 1000`. That is a real number for the
//! follow-on to argue with; it is NOT a reason to fork a second kernel
//! here, because a fork re-opens the §16.3↔§11.2 divergence this whole
//! arrangement exists to close. Generalizing the ONE kernel to N channels
//! is the shape of the fix, and it is a follow-on, never a prerequisite.
//!
//! **★ ONE CANONICAL RECT, NEVER PER TILE [load-bearing].** Mask feather
//! ranges to 1000 px and `σ = feather_px/3`, so every `feather_px > 96`
//! takes the kernel's Kovesi 3-box path — which carries a running sum
//! along each scanline and is therefore NOT rect-invariant: the same
//! output pixel gets different low bits depending on which `out_rect`
//! asked for it. A compositor that blurred per output tile would show
//! those differences as seams. So [`FeatheredCoverage`] is built ONCE per
//! mask over ONE canonical rect ([`FeatheredCoverage::rect`]) and every
//! tile SAMPLES it. [`ResolvedMasks`] exists to make that the only shape
//! the call sites can express: it is built where the mask stack is built
//! (once per composite) and sampled per pixel.
//!
//! ORDER ERRATUM (recorded by the predecessor slice): the pre-Sub-slice-B
//! text here listed
//! *density → feather*. The spec lists *feather → density* at BOTH the
//! v5.5 freeze and v5.6-dev (FI-10 changed only the σ mapping, never the
//! order — verified against the bogos history first-hand). The two orders
//! commute in exact arithmetic — density is affine with constant
//! coefficients, `m' = (1−d) + d·v`, and the normalized Gaussian
//! reproduces constants — so Sub-slice A's shipped per-pixel result was
//! never wrong; but the ORDER is now load-bearing (it decides what value
//! the blur sees outside stored content), so this module follows the spec
//! step list literally.
//!
//! **The FI-10 Option B payoff (`σ = feather_px/3`).** The §16.3 kernel
//! reach is `⌈3σ⌉`, so `⌈3σ⌉ == ⌈feather_px⌉ == Rmask`: the blur's support
//! and the mask's ROI/dirty margin are THE SAME NUMBER
//! ([`feather_reach_px`]). Two consequences this module depends on:
//!   1. a feathered mask's support is exactly
//!      `coverage.bounds().inflate(Rmask)` — beyond it every kernel tap
//!      sees only absent (0) coverage, so the feathered value is EXACTLY
//!      0, not merely small. That makes the canonical plane finite and
//!      its outside representable without storing it.
//!   2. an invalidation margin of `Rmask` is exactly sufficient — which is
//!      what removes the incremental-compositor seams that blocked this
//!      sub-slice.
//!
//! Masking a premultiplied-linear source is a per-channel multiply by `E`
//! ([`apply_coverage`]) — the CPU applies it host-side; the GPU multiplies
//! the identical host-computed `E` (a single correctly-rounded multiply,
//! so §34.3 parity stays exactly as strong as the un-masked kernel).

use appname_algo::rgba_blur::{gaussian_blur_rgba, PlaneRect, RgbaPlane};

use crate::geom::{IRect, IVec2};
use crate::graph::mask_node::MaskNode;

/// The feather-deferral message for the **sampling** door (§18 sampling
/// scope), prefixed by callers with `layer "{name}" `.
///
/// ITS GUARANTEE CHANGED WITH SUB-SLICE B AND THE OLD ONE MUST NOT BE
/// RESTORED. This constant began as an ANTI-SPLIT mechanism: the CPU
/// reference door and the render compositor had to refuse a feathered mask
/// with the SAME typed message so the two COMPOSITING surfaces could never
/// split serve↔refuse (requirement 6). Sub-slice B SERVES feather at both of
/// those surfaces, so that requirement is **DISCHARGED, NOT DELETED** —
/// there is no second refusing surface left for it to bind.
///
/// What remains is ONE consumer on a DIFFERENT door for a DIFFERENT reason:
/// [`crate::engine::sampling`] refuses a feathered mask because the sampling
/// scope cannot resample one — the §18 resample row, not §11.2 compositing.
///
/// SO: DO NOT ADD A SECOND CALL SITE TO RE-ESTABLISH THE PAIRING, and do not
/// replace this with a locally-authored string at the one remaining site. The
/// first re-introduces a serve↔refuse split Sub-slice B removed; the second is
/// the repair O-UI flagged, which satisfies the compiler while silently
/// deleting a normative guarantee.
pub const FEATHER_DEFERRED_MSG: &str =
    "carries a feathered mask (feather > 0); the sampling scope cannot resample a \
     feathered mask — refused typed, never silently unfeathered (QA-F8)";

/// [NORMATIVE, §11.2 step 2 — FI-10 Option B, bogos `51aca00`] The feather
/// Gaussian's sigma: `σ = feather_px / 3`. `feather_px` IS the 3σ radius,
/// exactly as §16.3 defines `σ = radius/3` for `filter.gaussian_blur` — the
/// two are the same mapping, which is what lets the §11.2 feather reuse the
/// §16.3 kernel unchanged.
#[inline]
pub fn feather_sigma(feather_px: f32) -> f64 {
    feather_px as f64 / 3.0
}

/// **`Rmask`** — [NORMATIVE] the §11.2 feather's kernel reach AND its
/// ROI / dirty-margin, in px. By FI-10 Option B these are ONE number:
///
/// ```text
/// ⌈3σ⌉ == ⌈feather_px⌉ == Rmask          (σ = feather_px/3)
/// ```
///
/// Pinned by [`tests::feather_reach_equals_ceil_feather_px_is_rmask`] over
/// a 100 000-point grid of the contract range. The `debug_assert_eq!`
/// below makes a violation a PANIC in every debug build, reachable from a
/// `mask.set_props` param, so the grid was backed by an EXHAUSTIVE
/// off-gate sweep of **every one of the 1 148 846 080 f32 bit patterns in
/// `(0, 1000]`** — zero violations. The grid stays on the gate (the
/// exhaustive sweep is ~1 s in release but minutes in the gate's debug
/// profile); the sweep is the evidence that the assert cannot fire.
///
/// Everything downstream — the feathered mask's
/// exact support, the incremental compositor's invalidation margin —
/// keys on this single value; that identity is what removes the seams
/// that blocked this sub-slice.
#[inline]
pub fn feather_reach_px(feather_px: f32) -> i32 {
    if !(feather_px > 0.0) {
        return 0;
    }
    let by_sigma = (3.0 * feather_sigma(feather_px)).ceil() as i32;
    debug_assert_eq!(
        by_sigma,
        (feather_px as f64).ceil() as i32,
        "§11.2 FI-10 invariant broken: ⌈3σ⌉ != ⌈feather_px⌉ at {feather_px}"
    );
    by_sigma
}

/// ONE feathered mask's coverage, evaluated over ONE canonical rect and
/// sampled from — **never** re-blurred per output tile (module doc, the
/// box path is not rect-invariant).
///
/// Only channel 0 of the RGBA blur is retained: the four lanes carry
/// identical bits for a splatted single-channel input, so the other three
/// are redundant and keeping them would cost 4× steady-state memory for
/// nothing.
///
/// **Support.** The rect is `content.inflate(Rmask)` where `content` is
/// the coverage buffer's stored extent. Outside it the feathered value is
/// EXACTLY 0, not merely small — every kernel tap there sees only absent
/// (0) coverage, and the reach bound `⌈3σ⌉` holds on BOTH kernel paths
/// (`appname_algo::rgba_blur::tests::box_support_never_exceeds_the_declared_reach`).
/// That is what makes [`sample`](Self::sample) able to answer 0 outside
/// the plane without storing an infinite one, and it is the same identity
/// that makes `Rmask` a sufficient invalidation margin.
#[derive(Clone, Debug, PartialEq)]
pub struct FeatheredCoverage {
    rect: IRect,
    v: Vec<f32>,
}

impl FeatheredCoverage {
    /// The canonical rect this plane was evaluated over
    /// (`content.inflate(Rmask)`). Outside it the feathered coverage is
    /// exactly 0.
    #[inline]
    pub fn rect(&self) -> IRect {
        self.rect
    }

    /// Feathered coverage at `p`; exactly 0 outside [`rect`](Self::rect).
    #[inline]
    pub fn sample(&self, p: IVec2) -> f32 {
        if !self.rect.contains(p) {
            return 0.0;
        }
        let (ix, iy) = (p.x - self.rect.min.x, p.y - self.rect.min.y);
        self.v[(iy * self.rect.width() + ix) as usize]
    }
}

/// Build the canonical feathered coverage plane for `mask`, or `None` when
/// it carries no feather (`feather_px <= 0` — nothing to blur, and the
/// unfeathered path stays byte-for-byte what it was).
///
/// **Cost, MEASURED — and the one flagged gap in this sub-slice.**
/// `O(area(content.inflate(Rmask)))` with a 4-lane kernel, paid ONCE per
/// mask **per composite**, and NOT cached across composites: the
/// [`FeatheredCoverage`] lives inside the [`ResolvedMasks`] the compositor
/// builds at flatten time and dies with it. `Rmask` reaches 1000 px, and
/// `content` is tile-granular, so the canonical rect grows fast. For a
/// full-canvas `reveal_all` mask (release build, this machine):
///
/// | canvas | feather | canonical rect | transient | plane | build |
/// |---|---|---|---|---|---|
/// | 512² | 1000 | 2512² | 96 MiB | 24 MiB | 489 ms |
/// | 1024² | 1000 | 3024² | 139 MiB | 34 MiB | 568 ms |
/// | 2048² | 1000 | 4048² | 250 MiB | 62 MiB | 753 ms |
/// | 2048² | 100 | 2248² | 77 MiB | 19 MiB | 113 ms |
/// | 4096² | 1000 | 6096² | 567 MiB | 141 MiB | 1.78 s |
///
/// The area is quadratic in the canvas and `doc.new` admits 1 000 000 px
/// per side, so this is UNBOUNDED at the schema limit — a large enough
/// feathered mask aborts on allocation rather than refusing typed. And
/// because [`crate::io::png_io`] and the compositor rebuild it per
/// composite, an incremental redraw
/// ([`recomposite_dirty_with`](../../../appname_render/fn.recomposite_dirty_with.html))
/// pays the full build on every dirty frame.
///
/// **FLAGGED, deliberately NOT fixed here** (it is a design call, not a
/// §11.2 correctness question, and the fix must not break the
/// one-canonical-rect rule the box path imposes). The three candidate
/// shapes: cache the plane on `(coverage content hash, feather)`;
/// refuse typed above an area threshold instead of aborting; or evaluate
/// tiled with the box path's running sum handled. Correctness is
/// unaffected — every number above is a cost, not a wrong pixel.
pub fn feather_coverage(mask: &MaskNode) -> Option<FeatheredCoverage> {
    let feather_px = mask.feather();
    if !(feather_px > 0.0) {
        return None;
    }
    let sigma = feather_sigma(feather_px);
    let r = feather_reach_px(feather_px);
    let cov = mask.coverage();
    // Stored extent: the declared bounds UNIONED with the occupied tiles.
    // `read_pixel_f32` answers from the tile map, so it can return nonzero
    // anywhere a tile is resident; keying only on `bounds()` would drop
    // content the UNFEATHERED path would have read, which is exactly the
    // kind of quiet serve↔serve divergence this slice must not introduce.
    let content = cov.bounds().union(&cov.tiles().occupied_bounds());
    if content.is_empty() {
        // No stored coverage anywhere: blurring 0 gives 0. An empty plane
        // samples as 0 everywhere, which is the same answer.
        return Some(FeatheredCoverage {
            rect: IRect::EMPTY,
            v: Vec::new(),
        });
    }
    let canonical = content.inflate(r, r);
    let rect = PlaneRect::new(
        canonical.min.x,
        canonical.min.y,
        canonical.max.x,
        canonical.max.y,
    );

    // Splat coverage into all four lanes over the canonical rect. Only the
    // `content` sub-rect is written: `RgbaPlane::new` zero-fills, and
    // coverage outside stored content reads 0 (§2.4 absent) — which is
    // also exactly what the kernel's Clamp edge policy then replicates
    // beyond the canonical boundary, since that boundary ring is itself
    // zero (it sits `Rmask` outside `content`). So Clamp and the §11.2
    // "absent reads 0" rule agree here, and no false edge value leaks in.
    let mut src = RgbaPlane::new(rect);
    for y in content.min.y..content.max.y {
        for x in content.min.x..content.max.x {
            let v = cov.read_pixel_f32(IVec2::new(x, y))[0];
            src.set(x, y, [v, v, v, v]);
        }
    }
    let blurred = gaussian_blur_rgba(src.view(), rect, sigma);
    Some(FeatheredCoverage {
        rect: canonical,
        v: blurred.px.iter().map(|p| p[0]).collect(),
    })
}

/// One enabled mask paired with its canonical feathered plane (`None` when
/// the mask carries no feather).
#[derive(Clone, Debug)]
struct ResolvedMask<'a> {
    mask: &'a MaskNode,
    feathered: Option<FeatheredCoverage>,
}

/// A layer's §11.2 mask stack RESOLVED for evaluation: its **ENABLED**
/// masks, each with its canonical feathered plane already computed.
///
/// **This type is the enforcement of the one-canonical-rect rule.** Build
/// it where the mask stack is built — once per layer per composite — and
/// sample it per pixel. There is deliberately no way to ask for coverage
/// at a point without having resolved the stack first, so no call site can
/// accidentally blur per output tile and reintroduce the box-path seams.
#[derive(Clone, Debug, Default)]
pub struct ResolvedMasks<'a> {
    entries: Vec<ResolvedMask<'a>>,
}

impl<'a> ResolvedMasks<'a> {
    /// An empty stack — full coverage, `E = 1` everywhere.
    pub const fn none() -> ResolvedMasks<'a> {
        ResolvedMasks {
            entries: Vec::new(),
        }
    }

    /// Resolve a stack: keep the **ENABLED** masks (a disabled mask is the
    /// §11.2 no-op and is dropped here, so no later step has to remember),
    /// and feather each one that carries feather.
    pub fn resolve(masks: &[&'a MaskNode]) -> ResolvedMasks<'a> {
        ResolvedMasks {
            entries: masks
                .iter()
                .filter(|m| m.enabled())
                .map(|m| ResolvedMask {
                    mask: m,
                    feathered: feather_coverage(m),
                })
                .collect(),
        }
    }

    /// True when no ENABLED mask remains: the unmasked fast path.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Number of ENABLED masks in the stack.
    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// The ENABLED masks, in stack order (for callers that need the mask
    /// nodes themselves — e.g. the render DAG's cache-key fold).
    pub fn masks(&self) -> impl Iterator<Item = &'a MaskNode> + '_ {
        self.entries.iter().map(|e| e.mask)
    }

    /// The widest `Rmask` in the stack, `0` when nothing is feathered.
    /// **This is the invalidation margin** a cache over this stack owes:
    /// a coverage edit at `q` changes the feathered result throughout
    /// `q.inflate(Rmask)` and nowhere beyond.
    pub fn feather_reach(&self) -> i32 {
        self.entries
            .iter()
            .map(|e| feather_reach_px(e.mask.feather()))
            .max()
            .unwrap_or(0)
    }

    /// The layer's §11.2 effective coverage at `p`: the product over its
    /// ENABLED masks. An empty stack is `E = 1.0` (the layer composites at
    /// full coverage). Order-independent (multiplication commutes).
    #[inline]
    pub fn coverage_at(&self, p: IVec2) -> f32 {
        let mut e = 1.0f32;
        for entry in &self.entries {
            e *= resolve_effective_coverage_with(entry.mask, entry.feathered.as_ref(), p);
        }
        e
    }
}

/// §11.2 effective coverage of ONE **enabled** raster mask at `p`
/// (document space). **The one routine every surface resolves through.**
///
/// Normative order (module doc): sample → feather → density → invert.
/// `feathered` is the mask's canonical plane from [`feather_coverage`]
/// (`None` for an unfeathered mask, in which case coverage is sampled
/// straight from the buffer and the result is byte-for-byte what the
/// pre-feather code produced).
///
/// A disabled mask must be skipped by the caller (it contributes `E = 1`),
/// which [`ResolvedMasks::resolve`] enforces by dropping it.
pub fn resolve_effective_coverage_with(
    mask: &MaskNode,
    feathered: Option<&FeatheredCoverage>,
    p: IVec2,
) -> f32 {
    // Sample (absent reads 0, §2.4); coverage is Gray/f32 — component 0.
    // Feather FIRST (§11.2 step 2): when the mask carries feather, the
    // sampled value comes from the canonical Gaussian plane instead of
    // raw coverage. Blurring is a neighbourhood operation, so it cannot
    // happen at this per-point granularity — it already happened, once,
    // in `feather_coverage`.
    let c = match feathered {
        Some(f) => f.sample(p),
        None => mask.coverage().read_pixel_f32(p)[0],
    };
    // Density: m' = 1 − density·(1 − c) (§11.2 [NORMATIVE], lerp toward 1).
    let mut m = 1.0 - mask.density() * (1.0 - c);
    // Invert LAST (§11.2 [NORMATIVE] order, O-INT ruling).
    if mask.inverted() {
        m = 1.0 - m;
    }
    m.clamp(0.0, 1.0)
}

/// §11.2 coverage of ONE enabled **UNFEATHERED** mask at `p` — the
/// narrow door for callers that have already refused feather upstream.
/// The §12.1 per-op mask leg ([`crate::engine::op_eval`]) is the only such
/// caller: op masks are unlinked v1 and a feathered op mask refuses typed
/// before evaluation begins.
///
/// Layer masks must NOT come through here — they go through
/// [`ResolvedMasks`], which feathers. The `debug_assert` below is the
/// tripwire for anyone who wires a feathered mask into this door and would
/// otherwise get a SILENTLY UNFEATHERED result.
#[inline]
pub fn resolve_effective_coverage(mask: &MaskNode, p: IVec2) -> f32 {
    debug_assert!(
        !mask_needs_feather(mask),
        "resolve_effective_coverage is the UNFEATHERED door; a feathered mask \
         reaching it would evaluate silently unfeathered — resolve the stack \
         through ResolvedMasks instead"
    );
    resolve_effective_coverage_with(mask, None, p)
}

/// The layer's §11.2 effective coverage at `p` over a RESOLVED stack —
/// the shared per-pixel `E` both compositing surfaces and the GPU apply.
///
/// Takes [`ResolvedMasks`] rather than a bare mask slice precisely so a
/// feathered mask cannot reach the multiply unfeathered.
#[inline]
pub fn layer_effective_coverage(masks: &ResolvedMasks<'_>, p: IVec2) -> f32 {
    masks.coverage_at(p)
}

/// Mask a premultiplied-linear source pixel: multiply ALL 4 channels by
/// `E` (§11.2 — masking a premultiplied source scales every channel; the
/// straight-alpha color is unchanged, only its weight). The GPU kernel
/// applies the identical `E` as `src *= coverage` (gpu.rs determinism
/// note), so CPU/GPU are bit-identical at this multiply.
#[inline]
pub fn apply_coverage(premul: [f32; 4], e: f32) -> [f32; 4] {
    [premul[0] * e, premul[1] * e, premul[2] * e, premul[3] * e]
}

/// Does this ENABLED mask need the §11.2 Gaussian? Disabled masks are
/// no-ops and never do.
///
/// **This is now a DESCRIPTIVE predicate, not a refusal gate.** Sub-slice
/// B serves feather; the three doors that used to refuse on it
/// (`png_io` ×2, the render compositor) resolve the stack instead. It
/// survives as the cheap "is there blur work here?" test for cost
/// accounting and cache-margin decisions, and as the tripwire in
/// [`resolve_effective_coverage`].
#[inline]
pub fn mask_needs_feather(mask: &MaskNode) -> bool {
    mask.enabled() && mask.feather() > 0.0
}

/// Does any mask in the stack need the §11.2 Gaussian? Descriptive, not a
/// refusal gate — see [`mask_needs_feather`].
///
/// **No in-workspace caller today, deliberately.** Before Sub-slice B this
/// was the refusal predicate at all three compositing doors; those doors
/// now resolve instead of refusing, and the render DAG's cache-key widening
/// keys on the PER-MASK [`feather_reach_px`] rather than a stack-wide
/// boolean (a stack-wide flag would widen every mask's key by the widest
/// mask's margin — sound, but needlessly cache-hostile). It survives as
/// core's §11.2 vocabulary for the cheap "is there blur work in this
/// stack?" question that cost accounting and O-UI's feather affordances
/// will ask.
pub fn stack_needs_feather(masks: &[&MaskNode]) -> bool {
    masks.iter().any(|m| mask_needs_feather(m))
}

#[cfg(test)]
mod tests {
    use super::*;
    use appname_algo::rgba_blur::EdgeMode;

    use crate::graph::id::MaskId;
    use crate::graph::mask_node::MaskNode;
    use crate::mask::Selection;
    use crate::tile::model::ColorModelKind;

    /// A Gray/f32 coverage mask whose whole content is the constant `c`.
    fn mask_const(c: f32) -> MaskNode {
        let mut cov = Selection::new_mask_buffer();
        assert_eq!(cov.model(), ColorModelKind::Gray, "mask coverage is Gray");
        // Write the constant over a small region; reads elsewhere are 0
        // (absent), which the tests below account for.
        for y in 0..4 {
            for x in 0..4 {
                cov.write_pixel_f32(IVec2::new(x, y), [c, 0.0, 0.0, 0.0]);
            }
        }
        MaskNode::new_raster(MaskId::new(), cov)
    }

    /// A Gray/f32 coverage mask whose content is the constant `c` over
    /// `rect`, absent (0) elsewhere.
    fn mask_over(rect: IRect, c: f32) -> MaskNode {
        let mut cov = Selection::new_mask_buffer();
        for y in rect.min.y..rect.max.y {
            for x in rect.min.x..rect.max.x {
                cov.write_pixel_f32(IVec2::new(x, y), [c, 0.0, 0.0, 0.0]);
            }
        }
        MaskNode::new_raster(MaskId::new(), cov)
    }

    fn with_feather(mut m: MaskNode, f: f32) -> MaskNode {
        let mut p = m.props();
        p.feather = f;
        m.set_props(p);
        m
    }

    /// Splat the mask's ACTUAL stored coverage over `rect` and blur it
    /// ONCE — an independent re-derivation of what [`feather_coverage`]
    /// must produce. It reads the coverage buffer through the same public
    /// door the evaluator does and is therefore independent of HOW the
    /// evaluator chooses its canonical rect: only the blur, the splat and
    /// the retained lane are under test.
    fn oracle_plane(m: &MaskNode, rect: IRect, sigma: f64) -> RgbaPlane {
        let pr = PlaneRect::new(rect.min.x, rect.min.y, rect.max.x, rect.max.y);
        let mut src = RgbaPlane::new(pr);
        for y in rect.min.y..rect.max.y {
            for x in rect.min.x..rect.max.x {
                let v = m.coverage().read_pixel_f32(IVec2::new(x, y))[0];
                src.set(x, y, [v, v, v, v]);
            }
        }
        gaussian_blur_rgba(src.view(), pr, sigma)
    }

    /// **The evaluator's own seam.** [`feather_coverage`] must be exactly
    /// ONE blur of the splatted coverage over `content.inflate(Rmask)`,
    /// with channel 0 retained — the canonical plane every tile then
    /// samples. Checked BYTE-for-byte against an oracle built straight
    /// from the kernel, so a wrong canonical rect, a wrong splat, a wrong
    /// retained lane, or a stray re-blur all red here.
    #[test]
    fn feather_coverage_is_one_canonical_blur_over_content_inflated_by_rmask() {
        let written = IRect::new(0, 0, 16, 16);
        let m = with_feather(mask_over(written, 1.0), 6.0); // σ = 2, Rmask = 6
        let fc = feather_coverage(&m).expect("a feathered mask has a plane");

        assert_eq!(feather_reach_px(6.0), 6);
        // SUFFICIENCY, stated without assuming how `content` is derived:
        // the canonical rect must cover every pixel the written coverage
        // can reach, i.e. `written.inflate(Rmask)`.
        assert!(
            fc.rect().contains_rect(&written.inflate(6, 6)),
            "the canonical rect {:?} must cover written.inflate(Rmask) {:?}",
            fc.rect(),
            written.inflate(6, 6)
        );

        let oracle = oracle_plane(&m, fc.rect(), feather_sigma(6.0));
        for y in fc.rect().min.y..fc.rect().max.y {
            for x in fc.rect().min.x..fc.rect().max.x {
                let want = oracle.get(x, y, EdgeMode::Transparent)[0];
                assert_eq!(
                    fc.sample(IVec2::new(x, y)),
                    want,
                    "canonical plane differs from one direct blur at ({x},{y})"
                );
            }
        }
    }

    /// **The truncation at `Rmask` is EXACT, not an approximation.** The
    /// canonical plane stops at `content.inflate(Rmask)` and
    /// [`FeatheredCoverage::sample`] answers 0 beyond it. That is only
    /// sound if the true feathered value out there is exactly 0 — so this
    /// blurs over a DELIBERATELY WIDER rect and demands hard zeros in the
    /// margin the representation drops. It also shows the bound is TIGHT:
    /// the outermost retained ring is nonzero, so `Rmask − 1` would clip
    /// real signal.
    #[test]
    fn the_rmask_truncation_is_exact_and_tight() {
        let written = IRect::new(0, 0, 4, 4);
        let m = with_feather(mask_over(written, 1.0), 3.0); // σ = 1, Rmask = 3
        let fc = feather_coverage(&m).expect("feathered");
        assert!(fc.rect().contains_rect(&written.inflate(3, 3)));

        // TIGHT: a pixel at Chebyshev distance exactly Rmask from the
        // nearest stored coverage still carries signal, so a margin of
        // Rmask − 1 would clip it.
        assert!(
            fc.sample(IVec2::new(-3, -3)) > 0.0,
            "the ring at distance Rmask must carry signal, else Rmask is loose"
        );
        assert_eq!(
            fc.sample(IVec2::new(-4, -4)),
            0.0,
            "and one px further out is exactly 0"
        );

        // EXACT: blur over a rect four px wider on every side than the
        // canonical one and demand the dropped margin is hard zero, not
        // merely small — that is what licenses `sample` to answer 0 there.
        let wide = fc.rect().inflate(4, 4);
        let oracle = oracle_plane(&m, wide, feather_sigma(3.0));
        for y in wide.min.y..wide.max.y {
            for x in wide.min.x..wide.max.x {
                let p = IVec2::new(x, y);
                let want = oracle.get(x, y, EdgeMode::Transparent)[0];
                if fc.rect().contains(p) {
                    assert_eq!(fc.sample(p), want, "retained region differs at ({x},{y})");
                } else {
                    assert_eq!(
                        want, 0.0,
                        "the dropped margin must be EXACTLY 0 at ({x},{y}), else \
                         sampling 0 outside the canonical rect loses signal"
                    );
                    assert_eq!(fc.sample(p), 0.0);
                }
            }
        }
    }

    /// **A COST PIN, recorded so it is a known quantity and not a
    /// surprise.** `content` unions the declared bounds with
    /// [`crate::tile::map::TileMap::occupied_bounds`], which is
    /// TILE-GRANULAR (`TILE` = 256). So a 16×16 painted mask living in one
    /// resident tile canonicalizes over the whole 256×256 tile inflated by
    /// `Rmask`, not over 16×16 inflated by `Rmask`.
    ///
    /// The union is CORRECT and deliberate — `read_pixel_f32` answers from
    /// the tile map, so keying on `bounds()` alone would drop coverage the
    /// unfeathered path would have read — and the extra area is all zeros,
    /// which blur to zero, so no pixel is wrong. What it costs is area:
    /// at the top of the feather range (`Rmask` = 1000) a one-tile mask
    /// blurs a 2256² plane. Tightening `content` to the true painted
    /// extent is a pure optimization, and this test is what will notice
    /// when someone does it.
    #[test]
    fn the_canonical_rect_is_tile_granular_a_recorded_cost_not_a_bug() {
        let written = IRect::new(0, 0, 16, 16);
        let m = with_feather(mask_over(written, 1.0), 6.0);
        let fc = feather_coverage(&m).expect("feathered");
        let content = m
            .coverage()
            .bounds()
            .union(&m.coverage().tiles().occupied_bounds());
        assert_eq!(
            fc.rect(),
            content.inflate(6, 6),
            "the canonical rect is exactly content.inflate(Rmask)"
        );
        assert!(
            fc.rect().width() > written.inflate(6, 6).width(),
            "and `content` is tile-granular, so it is WIDER than the painted \
             extent inflated by Rmask — the recorded cost"
        );
        // Every pixel beyond the painted extent's reach is exactly 0, so
        // the extra area buys nothing but costs area — which is precisely
        // why tightening it is safe when someone gets to it.
        assert_eq!(fc.sample(IVec2::new(23, 8)), 0.0);
        assert!(fc.sample(IVec2::new(21, 8)) > 0.0);
    }

    /// **The §11.2 step order with feather in play.** The density lerp must
    /// consume the FEATHERED value, not the raw one, and invert must come
    /// last. The probe point is a boundary pixel where raw coverage is 1.0
    /// but feathered coverage is strictly less — so an implementation that
    /// densified the raw sample lands on a different number.
    #[test]
    fn density_and_invert_consume_the_feathered_value_not_the_raw_one() {
        let content = IRect::new(0, 0, 16, 16);
        let mut m = with_feather(mask_over(content, 1.0), 6.0);
        let mut props = m.props();
        props.density = 0.5;
        props.inverted = true;
        m.set_props(props);

        let fc = feather_coverage(&m).expect("feathered");
        let p = IVec2::new(0, 8); // on the content edge
        let raw = m.coverage().read_pixel_f32(p)[0];
        let feathered = fc.sample(p);
        assert_eq!(raw, 1.0, "the probe point has FULL raw coverage");
        assert!(
            feathered > 0.0 && feathered < 1.0,
            "the probe point must be mid-ramp, else raw and feathered agree \
             and this test cannot tell them apart: {feathered}"
        );

        // E = 1 − (1 − density·(1 − c)) = density·(1 − c), with c FEATHERED.
        let got = resolve_effective_coverage_with(&m, Some(&fc), p);
        assert!(
            (got - 0.5 * (1.0 - feathered)).abs() < 1e-7,
            "feather → density → invert: got {got}, want {}",
            0.5 * (1.0 - feathered)
        );
        // And it is NOT the raw-sampled answer (which would be exactly 0).
        assert_ne!(got, 0.5 * (1.0 - raw), "density consumed the RAW sample");
    }

    /// A DISABLED mask is the §11.2 no-op, so [`ResolvedMasks::resolve`]
    /// drops it — and therefore never pays for its Gaussian. Feather on a
    /// disabled mask must not resurrect it.
    #[test]
    fn a_disabled_feathered_mask_is_dropped_before_any_blur() {
        let mut m = with_feather(mask_over(IRect::new(0, 0, 8, 8), 1.0), 12.0);
        let mut props = m.props();
        props.enabled = false;
        m.set_props(props);

        let resolved = ResolvedMasks::resolve(&[&m]);
        assert!(resolved.is_empty(), "a disabled mask is dropped at resolve");
        assert_eq!(
            resolved.feather_reach(),
            0,
            "and costs no invalidation margin"
        );
        assert_eq!(
            layer_effective_coverage(&resolved, IVec2::new(0, 0)),
            1.0,
            "a disabled mask contributes E = 1"
        );
        assert!(!mask_needs_feather(&m), "disabled ⇒ no feather work");
    }

    /// The stack's invalidation margin is the WIDEST `Rmask` in it — a
    /// narrower margin would under-invalidate the widest mask, which is
    /// the stale-tile failure.
    #[test]
    fn the_stack_invalidation_margin_is_the_widest_rmask() {
        let a = with_feather(mask_over(IRect::new(0, 0, 4, 4), 1.0), 3.0);
        let b = with_feather(mask_over(IRect::new(0, 0, 4, 4), 1.0), 12.0);
        let plain = mask_over(IRect::new(0, 0, 4, 4), 1.0);

        assert_eq!(ResolvedMasks::resolve(&[&a, &b]).feather_reach(), 12);
        assert_eq!(
            ResolvedMasks::resolve(&[&b, &a]).feather_reach(),
            12,
            "order-free"
        );
        assert_eq!(
            ResolvedMasks::resolve(&[&plain]).feather_reach(),
            0,
            "an unfeathered stack owes no margin"
        );
        assert_eq!(ResolvedMasks::none().feather_reach(), 0);
    }

    /// `feather_px <= 0` takes the UNFEATHERED path: no plane is built,
    /// and coverage is byte-for-byte the pre-Sub-slice-B answer. This is
    /// the guard on Sub-slice A's shipped pixels.
    #[test]
    fn zero_feather_keeps_the_unfeathered_path_byte_for_byte() {
        let content = IRect::new(0, 0, 4, 4);
        let m = mask_over(content, 0.75);
        assert!(feather_coverage(&m).is_none(), "no feather ⇒ no plane");
        assert!(!mask_needs_feather(&m));

        let resolved = ResolvedMasks::resolve(&[&m]);
        for y in -2..6 {
            for x in -2..6 {
                let p = IVec2::new(x, y);
                assert_eq!(
                    resolved.coverage_at(p),
                    resolve_effective_coverage(&m, p),
                    "the resolved door must equal the narrow unfeathered door at ({x},{y})"
                );
            }
        }
    }

    /// A mask with no stored coverage anywhere: blurring 0 is 0, so the
    /// canonical plane is empty and samples 0 everywhere — the same answer
    /// the unfeathered path gives, at no allocation.
    #[test]
    fn empty_coverage_feathers_to_an_empty_plane_that_samples_zero() {
        let m = with_feather(mask_over(IRect::EMPTY, 1.0), 9.0);
        let fc = feather_coverage(&m).expect("feather ⇒ Some, even when empty");
        assert!(fc.rect().is_empty(), "no content ⇒ no canonical rect");
        for (x, y) in [(0, 0), (-50, -50), (1000, 1000)] {
            assert_eq!(fc.sample(IVec2::new(x, y)), 0.0);
        }
    }

    /// **THE FI-10 invariant.** `⌈3σ⌉ == ⌈feather_px⌉ == Rmask` across the
    /// whole contract range for mask feather (`0.0..=1000.0` px, kinds.rs
    /// range check), stepped finely enough to catch a float-rounding
    /// mismatch at the integer boundaries where `f/3*3` could land a ULP
    /// high and push the ceil up a whole pixel.
    #[test]
    fn feather_reach_equals_ceil_feather_px_is_rmask() {
        let mut i = 1u32;
        while i <= 100_000 {
            let f = i as f32 / 100.0; // 0.01 ..= 1000.00 px
            let by_sigma = (3.0 * feather_sigma(f)).ceil() as i32;
            let by_px = (f as f64).ceil() as i32;
            assert_eq!(
                by_sigma,
                by_px,
                "⌈3σ⌉ != ⌈feather_px⌉ at feather_px = {f} (σ = {})",
                feather_sigma(f)
            );
            assert_eq!(feather_reach_px(f), by_px, "Rmask != ⌈feather_px⌉ at {f}");
            i += 1;
        }
        // Zero feather has no kernel and no margin.
        assert_eq!(feather_reach_px(0.0), 0);
    }

    /// PERTURBATION WITNESS for the invariant above — the input on which
    /// it says NO. Under the OTHER sigma mapping in the spec (§13.2
    /// `selection.modify` feather uses `σ = r/2`, and the pre-FI-10 §11.2
    /// text used `σ = feather_px`), the reach is NOT `⌈feather_px⌉`, so a
    /// dirty margin of `Rmask = ⌈feather_px⌉` would be too small and the
    /// seams would come back. This is exactly what FI-10 Option B bought.
    #[test]
    fn the_other_sigma_mappings_break_the_rmask_identity() {
        // σ = feather/2 (the §13.2 selection-feather mapping).
        let f = 8.0f32;
        let reach_half = (3.0 * (f as f64 / 2.0)).ceil() as i32; // ⌈12⌉ = 12
        assert_ne!(
            reach_half,
            (f as f64).ceil() as i32,
            "σ = f/2 must NOT satisfy the Rmask identity (it reaches 12 px, not 8)"
        );
        // σ = feather (the pre-FI-10 §11.2 text): reach 3× too wide.
        let reach_one = (3.0 * f as f64).ceil() as i32; // 24
        assert_ne!(reach_one, (f as f64).ceil() as i32);
        // And the mapping the code actually uses does satisfy it.
        assert_eq!(feather_reach_px(f), 8);
    }

    #[test]
    fn density_then_invert_order() {
        let p = IVec2::new(0, 0);
        // c = 0.25 at (0,0).
        let mut m = mask_const(0.25);

        // density 1, not inverted: E = c.
        assert!((resolve_effective_coverage(&m, p) - 0.25).abs() < 1e-6);

        // density 0.5, not inverted: m' = 1 − 0.5·(1 − 0.25) = 0.625.
        let mut props = m.props();
        props.density = 0.5;
        m.set_props(props);
        assert!((resolve_effective_coverage(&m, p) - 0.625).abs() < 1e-6);

        // density 0.5, inverted: E = 1 − m' = 0.375 (invert LAST).
        let mut props = m.props();
        props.inverted = true;
        m.set_props(props);
        assert!((resolve_effective_coverage(&m, p) - 0.375).abs() < 1e-6);
    }

    #[test]
    fn zero_density_inverted_is_fully_masked_not_noop() {
        // The corner the order ruling turns on: density 0 + inverted ⇒
        // m' = 1, invert ⇒ E = 0 (fully masked out), NOT E = 1.
        let p = IVec2::new(0, 0);
        let mut m = mask_const(0.9);
        let mut props = m.props();
        props.density = 0.0;
        props.inverted = true;
        m.set_props(props);
        assert_eq!(resolve_effective_coverage(&m, p), 0.0);

        // density 0, NOT inverted ⇒ m' = 1 ⇒ E = 1 (the no-op form).
        let mut props = m.props();
        props.inverted = false;
        m.set_props(props);
        assert_eq!(resolve_effective_coverage(&m, p), 1.0);
    }

    #[test]
    fn disabled_masks_are_skipped_by_layer_coverage() {
        let p = IVec2::new(0, 0);
        let mut hide = mask_const(0.0); // hide_all-like at (0,0)
                                        // Enabled: E = 0 pulls the product to 0.
        assert_eq!(
            layer_effective_coverage(&ResolvedMasks::resolve(&[&hide]), p),
            0.0
        );
        // Disabled: dropped by `resolve` ⇒ product stays 1 (the no-op).
        let mut props = hide.props();
        props.enabled = false;
        hide.set_props(props);
        let resolved = ResolvedMasks::resolve(&[&hide]);
        assert!(resolved.is_empty(), "a disabled mask is dropped at resolve");
        assert_eq!(layer_effective_coverage(&resolved, p), 1.0);
    }

    #[test]
    fn apply_coverage_scales_all_four_channels() {
        assert_eq!(
            apply_coverage([0.8, 0.4, 0.2, 1.0], 0.5),
            [0.4, 0.2, 0.1, 0.5]
        );
        // E = 0 ⇒ transparent.
        assert_eq!(apply_coverage([0.8, 0.4, 0.2, 1.0], 0.0), [0.0; 4]);
    }
}
