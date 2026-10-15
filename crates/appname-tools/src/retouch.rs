// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Retouch suite stroke engines (CP-1.0, plan §2.5 row "Retouch suite") —
//! pure, deterministic, no document mutation (D-48).
//!
//! Spec: ES §21.1 (clone stamp source math), §21.2 (healing Poisson /
//! gradient-domain), §21.5 (dodge/burn/sponge/blur/sharpen/smudge exact
//! per-tool math), §45.1 (healing edge cases: texture/color separation,
//! effective mask `m = foot·sel`, spot auto-source search, alpha rule),
//! §45.2 (clone source model, sample scope, edge rules), §32.3 (T-23…T-28
//! option rows), §15.3–§15.5 via the `brush` module (planning, tip
//! coverage, presentation model); §40.5 stroke family rules; contracts
//! `clone.stroke.yaml`, `heal.stroke.yaml`, `dodge_burn.stroke.yaml`,
//! `smudge.stroke.yaml`, `blur_sharpen.stroke.yaml`. DD ⑮.
//!
//! Healing consumes the ONE merged §45.1 solver library —
//! `appname_algo::poisson::poisson_heal` (O-ALGO wave-1 item 5, perez03
//! from-paper port; the adapter seam quoted in `reports/O-ALGO-log.md`) —
//! per un-premultiplied channel, `Guidance::Seamless`, multigrid. The
//! math is NOT reimplemented here. The register's gated
//! `heal_texture_contrast_variant` (US7605821B1, gate 2028-03-17) is NOT
//! implemented and no texture/contrast-matching structure exists in this
//! module.
//!
//! # Pipeline shapes
//!
//! Two shapes cover the five kinds, both starting from the brush
//! engine's deterministic [`crate::brush::plan_stroke`] dab sequence:
//!
//! * **Accumulate-then-present** (clone; the §15.5 two-buffer model grown
//!   the `S.colw` accumulator §15.5 reserves for per-dab-color tools):
//!   per dab, per pixel, `a = flow·t·(1 − S.cov)` accumulates coverage
//!   AND the bilinearly-sampled premultiplied source color; presentation
//!   moves the destination toward the coverage-weighted mean source by
//!   `gate = opacity·cov·sel(p)` — for an opaque constant color this is
//!   exactly the §15.5 stamp path, and for sampled sources it "copies
//!   source coverage through the §15.5 stamp path" (§45.2: off-canvas
//!   samples are fully-transparent premultiplied pixels, so a clone dab
//!   can erase toward transparency at canvas edges).
//! * **Per-dab evolving buffer** (dodge/burn/sponge, smudge,
//!   blur/sharpen; §21.5 letter — each dab applies incrementally with
//!   `cov = t(p)·flow`, march order): dabs mutate a working window in
//!   sequence (`sel(p)` gates each deposit); presentation applies the
//!   stroke `opacity` once as the §15.5 step-4 ceiling —
//!   `out = lerp(pre, evolved, opacity)`.
//!
//! Healing is neither: the whole-stroke union coverage (the same
//! [`crate::brush::rasterize_plan`] scratch) is the §45.1 footprint;
//! `m(p) = foot(p)·sel(p)`, `Ω = {m > 0.5}`; the solve runs once at
//! commit and blends back `out = lerp(dest, f, m·opacity)` (opacity as
//! the step-4 ceiling folded into the §45.1 blend mask — reading
//! recorded in the lane log). Unselected/uncovered pixels are BIT-EXACT
//! unchanged: write-back masks to `m > 0` (§45.1
//! `prop_heal_outside_untouched`).
//!
//! # v1 determinism + reading pins (lane log carries the full table)
//!
//! * Working space: the surface's STORAGE encoding, straight
//!   (un-premultiplied) for the per-channel math (§21.2/§45.1 letter for
//!   heal; §21.5 "linear working RGB" pends the §5 unification exactly
//!   like the brush engine's recorded divergence), canonical
//!   premultiplied for the color-dragging tools (clone/smudge).
//! * Clone sampling is bilinear (§21.1; the `quality: bicubic` tool
//!   option has no wire param — unexpressed, recorded). Flips (§21.1
//!   `F`) have no wire params either — identity, recorded.
//! * Spot-heal candidate translations are integer-rounded
//!   (`round(r·cosθ), round(r·sinθ)`) so scoring and guidance stay
//!   on-grid — deterministic, no resampling (pin).
//! * Poisson: `Solver::Multigrid { cycles: 20, smooth: 2 }` — the §21.2
//!   20-V-cycle quality floor run as a fixed count (the shared seam
//!   exposes no residual early-stop; a fixed count is deterministic and
//!   ≥ the floor). The §21.2 small-region CG *fallback* is a perf
//!   election the seam does not expose — multigrid converges the same
//!   masked system (recorded divergence).
//! * Off-canvas / zero-coverage sampled guidance takes `g = dest` at
//!   those pixels — the solve reconstructs the destination there
//!   (§45.1 failure table: "destination-colored diffusion", no error);
//!   the exact `v_pq = 0` form needs a guidance-mask seam on the shared
//!   library (noted for O-ALGO, lane log).
//! * `protectTones` (dodge/burn) is carried but pins no §21.5 math term
//!   (§32.3 lists the bar toggle; the pixel contract is the §21.5
//!   formula set, which has none) — accepted, behavior identical either
//!   way, contract gap recorded. `protectDetail` (sharpen) maps to the
//!   §21.5 3×3 min/max ringing clamp ("`protectDetail` param = true
//!   from the bar" beside "clamped USM with 3×3 min/max ringing
//!   suppression", §32.3 T-27) — `false` runs the unclamped USM.
//! * Sponge amount = the wire `exposure` (0..=1); dodge/burn
//!   `e = exposure·0.5` (§21.5: UI 0–100 % → e ∈ [0, 0.5]); sponge and
//!   the §21.5 sponge formula ignore the tonal-range weighting (the
//!   contract's "where §21.5 says so").

use appname_core::{IRect, IVec2, TileCoord, TILE, TILE_I32};

use crate::brush::present::{Backdrop, CoverageRead, TilePatch};
use crate::brush::{
    dab_coverage_at, plan_stroke, rasterize_plan, BrushPreset, StrokeError, StrokePlan,
};

/// Everything the five retouch kinds share: the planned stroke inputs,
/// the destination surface, and the §40.5 gates the command owns.
pub struct RetouchIo<'a> {
    /// Decoded `samples` asset (§9.6 rule 2).
    pub samples: &'a [crate::brush::InputSample],
    /// The resolved brush preset (§15.7).
    pub preset: &'a BrushPreset,
    /// Contract `sizePx` (0.1..=10 000; absent = preset size).
    pub size_px: Option<f32>,
    /// Contract `opacity` (0..=1, default 1) — the §15.5 step-4 ceiling.
    pub opacity: f32,
    /// Contract `flow` (0..=1, default 1).
    pub flow: f32,
    /// Contract `seed` (§15.4 per-stroke dynamics seed).
    pub seed: u64,
    /// Pre-stroke destination surface (storage-normalized f32 reads).
    pub backdrop: &'a dyn Backdrop,
    /// Whether the destination storage is premultiplied.
    pub premultiplied: bool,
    /// Rasterization clip (the canvas rect).
    pub clip: IRect,
    /// `sel(p)` (§13.1); `None` = fully selected.
    pub selection: Option<&'a dyn CoverageRead>,
    /// §40.5 TRANSPARENCY-lock constrain (clone/smudge: coverage clamps
    /// to existing destination alpha — alpha exactly invariant).
    pub clamp_to_backdrop_alpha: bool,
}

/// The deterministic result: composited tile patches + the changed-pixel
/// bounds + the dab plan (exposed for tests and O-QA fixtures).
pub struct RetouchCommit {
    pub patches: Vec<TilePatch>,
    pub bounds: IRect,
    pub plan: StrokePlan,
}

/// Typed retouch failures beyond the shared stroke-param errors.
#[derive(Debug)]
pub enum RetouchError {
    /// Shared §10.3 param validation (ranges via `plan_stroke`).
    Stroke(StrokeError),
    /// §45.1 spot healing: no valid auto-source after the widened pass.
    NoSourceFound,
}

impl From<StrokeError> for RetouchError {
    fn from(e: StrokeError) -> Self {
        RetouchError::Stroke(e)
    }
}

// ---------------------------------------------------------------------------
// Shared window plumbing.
// ---------------------------------------------------------------------------

#[inline]
fn sel_at(io: &RetouchIo<'_>, p: IVec2) -> f32 {
    match io.selection {
        Some(s) => s.coverage(p).clamp(0.0, 1.0),
        None => 1.0,
    }
}

/// A dense working window over `rect` (document space), holding the
/// pre-stroke values in BOTH storage form and the tool's working form.
struct Window {
    rect: IRect,
    w: usize,
    /// Pre-stroke values in the tool's working form (straight or premul).
    pre: Vec<[f32; 4]>,
    /// Evolving values, same form.
    cur: Vec<[f32; 4]>,
    /// Deposit mask: true where the tool wrote (bit-exactness guard).
    changed: Vec<bool>,
}

impl Window {
    fn new(io: &RetouchIo<'_>, rect: IRect, canonical_premul: bool) -> Window {
        let w = rect.width().max(0) as usize;
        let h = rect.height().max(0) as usize;
        let mut pre = vec![[0.0f32; 4]; w * h];
        let mut i = 0;
        for y in rect.min.y..rect.max.y {
            for x in rect.min.x..rect.max.x {
                let v = io.backdrop.read_pixel(IVec2::new(x, y));
                pre[i] = convert_form(v, io.premultiplied, canonical_premul);
                i += 1;
            }
        }
        let cur = pre.clone();
        Window {
            rect,
            w,
            pre,
            cur,
            changed: vec![false; w * h],
        }
    }

    #[inline]
    fn idx(&self, p: IVec2) -> Option<usize> {
        if !self.rect.contains(p) {
            return None;
        }
        Some(((p.y - self.rect.min.y) as usize) * self.w + (p.x - self.rect.min.x) as usize)
    }
}

/// Storage → working-form conversion (`premul` in, `canonical_premul`
/// out). Straight color of an `a = 0` pixel is 0 (§45.1 note: fully
/// transparent pixels un-premultiply to color 0).
#[inline]
fn convert_form(v: [f32; 4], storage_premul: bool, want_premul: bool) -> [f32; 4] {
    match (storage_premul, want_premul) {
        (true, true) | (false, false) => v,
        (true, false) => {
            if v[3] > 0.0 {
                [v[0] / v[3], v[1] / v[3], v[2] / v[3], v[3]]
            } else {
                [0.0, 0.0, 0.0, 0.0]
            }
        }
        (false, true) => [v[0] * v[3], v[1] * v[3], v[2] * v[3], v[3]],
    }
}

/// Build the commit patches: pre-stroke tiles with the changed pixels
/// overwritten by `final_of(idx)` (working form → storage form here).
fn commit_from_window(
    io: &RetouchIo<'_>,
    win: &Window,
    canonical_premul: bool,
    plan: StrokePlan,
    final_of: impl Fn(usize) -> [f32; 4],
) -> RetouchCommit {
    // Tight changed-pixel bounds.
    let mut bounds = IRect::EMPTY;
    let mut i = 0;
    for y in win.rect.min.y..win.rect.max.y {
        for x in win.rect.min.x..win.rect.max.x {
            if win.changed[i] {
                bounds = bounds.union(&IRect::new(x, y, x + 1, y + 1));
            }
            i += 1;
        }
    }
    if bounds.is_empty() {
        return RetouchCommit {
            patches: Vec::new(),
            bounds,
            plan,
        };
    }
    let mut patches = Vec::new();
    for (tx, ty) in bounds.tiles(TILE_I32) {
        let coord = TileCoord::new(0, tx, ty);
        let trect = coord.pixel_rect();
        let mut pixels = vec![[0.0f32; 4]; TILE * TILE];
        io.backdrop.read_tile_dense(coord, &mut pixels);
        let mut any = false;
        let mut ti = 0;
        for y in trect.min.y..trect.max.y {
            for x in trect.min.x..trect.max.x {
                if let Some(wi) = win.idx(IVec2::new(x, y)) {
                    if win.changed[wi] {
                        let out = final_of(wi);
                        pixels[ti] = convert_form(out, canonical_premul, io.premultiplied);
                        any = true;
                    }
                }
                ti += 1;
            }
        }
        if any {
            patches.push(TilePatch { coord, pixels });
        }
    }
    RetouchCommit {
        patches,
        bounds,
        plan,
    }
}

fn make_plan(io: &RetouchIo<'_>) -> Result<StrokePlan, StrokeError> {
    let params = crate::brush::StrokeParams {
        size_px: io.size_px,
        hardness: None, // retouch strokes expose no hardness override in v1
        opacity: io.opacity,
        flow: io.flow,
        seed: io.seed,
    };
    plan_stroke(io.samples, io.preset, &params)
}

/// The §45.1 effective stroke mask over `bounds = plan.bounds ∩ clip`:
/// `m(p) = foot(p)·sel(p)`, row-major over `bounds`. Exposed for the
/// command-level spot content-aware arm (H = Ω = {m > 0.5}, §45.1
/// fill-mode default riding the §45.3 exemplar engine).
pub fn footprint_mask(io: &RetouchIo<'_>) -> Result<(StrokePlan, IRect, Vec<f32>), StrokeError> {
    let plan = make_plan(io)?;
    let scratch = rasterize_plan(&plan, io.clip);
    let bounds = plan.bounds.intersect(&io.clip);
    let w = bounds.width().max(0) as usize;
    let h = bounds.height().max(0) as usize;
    let mut m = vec![0.0f32; w * h];
    let mut i = 0;
    for y in bounds.min.y..bounds.max.y {
        for x in bounds.min.x..bounds.max.x {
            let p = IVec2::new(x, y);
            m[i] = scratch.coverage_at(p) * sel_at(io, p);
            i += 1;
        }
    }
    Ok((plan, bounds, m))
}

/// Bilinear sample of a [`Backdrop`] at a sub-pixel document position, in
/// PREMULTIPLIED form (§18.2 rule: interpolation is always
/// premultiplied). Positions whose 4 taps all fall outside `valid` read
/// fully-transparent (§45.2 edge rule); partial taps blend with
/// transparent, which is the same rule per-tap.
fn sample_premul(src: &dyn Backdrop, src_premul: bool, valid: IRect, x: f64, y: f64) -> [f32; 4] {
    // Tap grid: pixel centers at integer + 0.5.
    let fx = x - 0.5;
    let fy = y - 0.5;
    let x0 = fx.floor();
    let y0 = fy.floor();
    let tx = (fx - x0) as f32;
    let ty = (fy - y0) as f32;
    let mut acc = [0.0f32; 4];
    for (dy, wy) in [(0i32, 1.0 - ty), (1, ty)] {
        for (dx, wx) in [(0i32, 1.0 - tx), (1, tx)] {
            let wgt = wx * wy;
            if wgt <= 0.0 {
                continue;
            }
            let p = IVec2::new(x0 as i32 + dx, y0 as i32 + dy);
            if !valid.contains(p) {
                continue; // transparent tap (§45.2)
            }
            let v = src.read_pixel(p);
            let v = convert_form(v, src_premul, true);
            for k in 0..4 {
                acc[k] += v[k] * wgt;
            }
        }
    }
    acc
}

// ---------------------------------------------------------------------------
// clone.stroke — §21.1/§45.2.
// ---------------------------------------------------------------------------

/// Clone source description (the command resolves `params.source`).
pub struct CloneSpec<'a> {
    /// The sampling surface the command resolved for the §45.2
    /// sample-scope tri-state: a single layer buffer (`current`) or the
    /// lazy multi-layer sampling composite (`current_and_below` / `all`,
    /// `appname_core::resolve_sampling_composite`). Either way it is
    /// resolved ONCE and read for the whole coalesced stroke — see the
    /// per-dab-liveness note on `kind_impls_retouch`'s
    /// `resolve_sampling_surface`.
    pub source: &'a dyn Backdrop,
    /// Whether the source storage is premultiplied.
    pub source_premultiplied: bool,
    /// Contract `offset` `[dx, dy]` px — `C(P) = P + offset` (§21.1: the
    /// armed offset is constant within one coalesced stroke session in
    /// both aligned modes).
    pub offset: [f64; 2],
    /// Contract `rotationDeg` (per-dab source rotation θ, §21.1).
    pub rotation_deg: f64,
    /// Contract `scale` (0.01..=10, §10.3).
    pub scale: f64,
}

/// Render one `clone.stroke` (§21.1 dab sampling through the §15.5 stamp
/// path with the `S.colw` per-dab-color accumulator).
pub fn render_clone_stroke(
    io: &RetouchIo<'_>,
    spec: &CloneSpec<'_>,
) -> Result<RetouchCommit, RetouchError> {
    let plan = make_plan(io)?;
    let bounds = plan.bounds.intersect(&io.clip);
    let win_rect = bounds;
    let mut win = Window::new(io, win_rect, true);
    if win_rect.is_empty() {
        return Ok(commit_from_window(io, &win, true, plan, |_| [0.0; 4]));
    }
    let n = win.pre.len();
    let mut cov = vec![0.0f32; n];
    let mut colw = vec![0.0f32; n];
    let mut col = vec![[0.0f32; 4]; n];

    // §21.1: src(d) = C(P) + F·R(−θ)·d/s, F = identity (no wire flips),
    // C(P) = P + offset. θ in radians; θ = 0 short-circuits like the tip
    // math (no libm on the default path).
    let theta = spec.rotation_deg.to_radians();
    let (cos_t, sin_t) = if spec.rotation_deg == 0.0 {
        (1.0f64, 0.0f64)
    } else {
        (theta.cos(), theta.sin())
    };
    let inv_s = 1.0 / spec.scale;

    for dab in &plan.dabs {
        let rect = dab.pixel_rect().intersect(&win_rect);
        if rect.is_empty() {
            continue;
        }
        for y in rect.min.y..rect.max.y {
            for x in rect.min.x..rect.max.x {
                let p = IVec2::new(x, y);
                let t = dab_coverage_at(dab, p);
                if t <= 0.0 {
                    continue;
                }
                let i = win.idx(p).expect("rect ⊆ window");
                let a = dab.flow * t * (1.0 - cov[i]);
                if a <= 0.0 {
                    continue;
                }
                // d = pixel center − dab center; R(−θ)·d/s.
                let dx = (x as f64 + 0.5) - dab.center[0];
                let dy = (y as f64 + 0.5) - dab.center[1];
                let rx = (cos_t * dx + sin_t * dy) * inv_s;
                let ry = (-sin_t * dx + cos_t * dy) * inv_s;
                let sx = dab.center[0] + spec.offset[0] + rx;
                let sy = dab.center[1] + spec.offset[1] + ry;
                let s = sample_premul(spec.source, spec.source_premultiplied, io.clip, sx, sy);
                for k in 0..4 {
                    col[i][k] += s[k] * a;
                }
                colw[i] += a;
                cov[i] += a;
            }
        }
    }

    // Presentation (§15.5 step 4 with the sampled mean source).
    let mut i = 0;
    for y in win_rect.min.y..win_rect.max.y {
        for x in win_rect.min.x..win_rect.max.x {
            if cov[i] > 0.0 && colw[i] > 0.0 {
                let sel = sel_at(io, IVec2::new(x, y));
                let gate = io.opacity * cov[i] * sel;
                if gate > 0.0 {
                    let mean = [
                        col[i][0] / colw[i],
                        col[i][1] / colw[i],
                        col[i][2] / colw[i],
                        col[i][3] / colw[i],
                    ];
                    let dst = win.pre[i];
                    if io.clamp_to_backdrop_alpha {
                        // §11.1 TRANSPARENCY constrain — source-atop
                        // (alpha exactly invariant; brush-module rule).
                        if dst[3] > 0.0 && mean[3] > 0.0 {
                            // Straight source color, deposited under the
                            // destination's own alpha.
                            let sc = [mean[0] / mean[3], mean[1] / mean[3], mean[2] / mean[3]];
                            let ia = 1.0 - gate;
                            win.cur[i] = [
                                sc[0] * gate * dst[3] + dst[0] * ia,
                                sc[1] * gate * dst[3] + dst[1] * ia,
                                sc[2] * gate * dst[3] + dst[2] * ia,
                                dst[3],
                            ];
                            win.changed[i] = true;
                        }
                    } else {
                        // Premultiplied lerp toward the sampled source —
                        // the stamp path generalized to per-dab color:
                        // copies source coverage (§45.2), erases toward
                        // transparency where the source is transparent.
                        let mut out = [0.0f32; 4];
                        for k in 0..4 {
                            out[k] = dst[k] + (mean[k] - dst[k]) * gate;
                        }
                        win.cur[i] = out;
                        win.changed[i] = true;
                    }
                }
            }
            i += 1;
        }
    }
    let cur = win.cur.clone();
    Ok(commit_from_window(io, &win, true, plan, move |i| cur[i]))
}

// ---------------------------------------------------------------------------
// heal.stroke — §21.2/§45.1.
// ---------------------------------------------------------------------------

/// Healing-mode guidance (the command resolves source/pattern upstream;
/// pattern guidance answers `error.retouch.pattern_missing` there until a
/// pattern arena exists).
pub struct HealSampledSpec<'a> {
    /// The §45.2 sampling surface, resolved once per coalesced stroke —
    /// see [`CloneSpec::source`].
    pub source: &'a dyn Backdrop,
    pub source_premultiplied: bool,
    /// Contract `offset` `[dx, dy]` px.
    pub offset: [f64; 2],
}

/// Spot-healing fill mode (§45.1): the proximity arm lives here; the
/// content-aware arm rides the §45.3 exemplar engine at the command
/// level (H = Ω), CAF-consumer precedent.
pub enum SpotArm {
    Proximity,
}

/// The §45.1 blend apron: the solve window inflates the footprint by
/// this many pixels (boundary ring + Dirichlet band).
const HEAL_APRON: i32 = 4;

/// Render one healing-mode `heal.stroke` commit (sampled guidance).
pub fn render_heal_stroke(
    io: &RetouchIo<'_>,
    spec: &HealSampledSpec<'_>,
) -> Result<RetouchCommit, RetouchError> {
    let plan = make_plan(io)?;
    let scratch = rasterize_plan(&plan, io.clip);
    let g = |p: IVec2, dest: [f32; 4]| -> [f32; 4] {
        let sx = p.x as f64 + 0.5 + spec.offset[0];
        let sy = p.y as f64 + 0.5 + spec.offset[1];
        let sp = IVec2::new((sx - 0.5).round() as i32, (sy - 0.5).round() as i32);
        // Off-canvas / zero-coverage sample → g = dest (module pin:
        // reconstructs the destination there; §45.1 failure table).
        if !io.clip.contains(sp) {
            return dest;
        }
        let s = sample_premul(spec.source, spec.source_premultiplied, io.clip, sx, sy);
        if s[3] <= 0.0 {
            return dest;
        }
        [s[0] / s[3], s[1] / s[3], s[2] / s[3], s[3]]
    };
    heal_common(io, plan, scratch, g)
}

/// Render one spot-mode `heal.stroke` commit, proximity fill (§45.1
/// auto-source search; content-aware mode is the command-level exemplar
/// arm).
pub fn render_spot_heal_stroke(
    io: &RetouchIo<'_>,
    _arm: SpotArm,
) -> Result<RetouchCommit, RetouchError> {
    let plan = make_plan(io)?;
    let scratch = rasterize_plan(&plan, io.clip);
    let r_brush = (io.size_px.unwrap_or(plan.base_diameter) as f64) * 0.5;

    // Ω on the footprint window (m = foot·sel > 0.5).
    let bounds = plan.bounds.intersect(&io.clip);
    if bounds.is_empty() {
        let win = Window::new(io, bounds, false);
        return Ok(commit_from_window(io, &win, false, plan, |_| [0.0; 4]));
    }
    let w = bounds.width() as usize;
    let h = bounds.height() as usize;
    let mut omega = vec![false; w * h];
    let mut any = false;
    for y in bounds.min.y..bounds.max.y {
        for x in bounds.min.x..bounds.max.x {
            let p = IVec2::new(x, y);
            let m = scratch.coverage_at(p) * sel_at(io, p);
            if m > 0.5 {
                omega[((y - bounds.min.y) as usize) * w + (x - bounds.min.x) as usize] = true;
                any = true;
            }
        }
    }
    if !any {
        // Empty effective region: §40.1 empty push (the front-end
        // normally suppresses these; headless they are a no-op accept).
        let win = Window::new(io, IRect::EMPTY, false);
        return Ok(commit_from_window(io, &win, false, plan, |_| [0.0; 4]));
    }

    // 3-px boundary ring just outside Ω (chamfer-free: 3 dilation passes).
    let mut ring = vec![false; w * h];
    {
        let mut cur = omega.clone();
        for _ in 0..3 {
            let mut next = cur.clone();
            for y in 0..h as i32 {
                for x in 0..w as i32 {
                    if cur[(y as usize) * w + x as usize] {
                        for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                            let (nx, ny) = (x + dx, y + dy);
                            if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                                next[(ny as usize) * w + nx as usize] = true;
                            }
                        }
                    }
                }
            }
            cur = next;
        }
        for i in 0..w * h {
            ring[i] = cur[i] && !omega[i];
        }
    }

    // Straight destination read (scoring runs in working space, §21.2).
    let dest_straight =
        |p: IVec2| -> [f32; 4] { convert_form(io.backdrop.read_pixel(p), io.premultiplied, false) };

    // Candidate passes: radii {2,3,4}·R then the widened {4,6,8}·R
    // (§45.1), 32 uniformly-spaced angles, fixed order, argmin with
    // strict `<` (ties → lowest (radius, angle) index).
    let mut best: Option<(f64, [i32; 2])> = None;
    for radii in [[2.0f64, 3.0, 4.0], [4.0, 6.0, 8.0]] {
        for r_mult in radii {
            let r = r_mult * r_brush;
            for k in 0..32u32 {
                let ang = (k as f64) * (std::f64::consts::TAU / 32.0);
                let t = [
                    (r * ang.cos()).round() as i32,
                    (r * ang.sin()).round() as i32,
                ];
                if t == [0, 0] {
                    continue; // degenerate candidate (identity source)
                }
                // Exclusions (§45.1 rule 1).
                let mut ok = true;
                'check: for y in 0..h {
                    for x in 0..w {
                        if !omega[y * w + x] {
                            continue;
                        }
                        let p = IVec2::new(
                            bounds.min.x + x as i32 + t[0],
                            bounds.min.y + y as i32 + t[1],
                        );
                        // (b) leaves the canvas.
                        if !io.clip.contains(p) {
                            ok = false;
                            break 'check;
                        }
                        // (a) overlaps Ω.
                        let lx = p.x - bounds.min.x;
                        let ly = p.y - bounds.min.y;
                        if lx >= 0
                            && ly >= 0
                            && (lx as usize) < w
                            && (ly as usize) < h
                            && omega[(ly as usize) * w + lx as usize]
                        {
                            ok = false;
                            break 'check;
                        }
                        // (c) zero coverage in the sampling scope.
                        if dest_straight(p)[3] <= 0.0 {
                            ok = false;
                            break 'check;
                        }
                    }
                }
                if !ok {
                    continue;
                }
                // Score: SSD between the destination boundary ring and
                // the correspondingly-translated source ring.
                let mut ssd = 0.0f64;
                let mut votes = 0usize;
                for y in 0..h {
                    for x in 0..w {
                        if !ring[y * w + x] {
                            continue;
                        }
                        let p = IVec2::new(bounds.min.x + x as i32, bounds.min.y + y as i32);
                        let q = IVec2::new(p.x + t[0], p.y + t[1]);
                        if !io.clip.contains(q) {
                            ssd = f64::INFINITY;
                            break;
                        }
                        let a = dest_straight(p);
                        let b = dest_straight(q);
                        if b[3] <= 0.0 {
                            ssd = f64::INFINITY; // all/partly transparent ring
                            break;
                        }
                        for c in 0..3 {
                            let d = (a[c] - b[c]) as f64;
                            ssd += d * d;
                        }
                        votes += 1;
                    }
                }
                if !ssd.is_finite() || votes == 0 {
                    continue;
                }
                if best.map(|(b, _)| ssd < b).unwrap_or(true) {
                    best = Some((ssd, t));
                }
            }
        }
        if best.is_some() {
            break; // the widened pass runs only if pass 1 rejected all
        }
    }
    let Some((_, t)) = best else {
        return Err(RetouchError::NoSourceFound);
    };
    let g = move |p: IVec2, _dest: [f32; 4]| -> [f32; 4] {
        // On-grid translated read from the destination itself (validity
        // proven by the exclusion pass).
        convert_form(
            io.backdrop.read_pixel(IVec2::new(p.x + t[0], p.y + t[1])),
            io.premultiplied,
            false,
        )
    };
    heal_common(io, plan, scratch, g)
}

/// The shared §21.2/§45.1 commit: build `m`, Ω, guidance; solve per
/// channel via THE merged library; blend `out = lerp(dest, f, m·opacity)`
/// on color only (alpha untouched).
fn heal_common(
    io: &RetouchIo<'_>,
    plan: StrokePlan,
    scratch: crate::brush::StrokeScratch,
    guidance: impl Fn(IVec2, [f32; 4]) -> [f32; 4],
) -> Result<RetouchCommit, RetouchError> {
    use appname_algo::plane::Plane;
    use appname_algo::poisson::{poisson_heal, Guidance, Solver};

    let foot_bounds = plan.bounds.intersect(&io.clip);
    let win_rect = IRect::new(
        foot_bounds.min.x - HEAL_APRON,
        foot_bounds.min.y - HEAL_APRON,
        foot_bounds.max.x + HEAL_APRON,
        foot_bounds.max.y + HEAL_APRON,
    )
    .intersect(&io.clip);
    let mut win = Window::new(io, win_rect, false);
    if win_rect.is_empty() {
        return Ok(commit_from_window(io, &win, false, plan, |_| [0.0; 4]));
    }
    let w = win_rect.width() as usize;
    let h = win_rect.height() as usize;

    // m(p) = foot·sel; Ω = {m > 0.5} (§45.1).
    let mut m = vec![0.0f32; w * h];
    let mut mask = Plane::new(w, h);
    let mut any = false;
    let mut i = 0;
    for y in win_rect.min.y..win_rect.max.y {
        for x in win_rect.min.x..win_rect.max.x {
            let p = IVec2::new(x, y);
            let mv = scratch.coverage_at(p) * sel_at(io, p);
            m[i] = mv;
            if mv > 0.5 {
                mask.data[i] = 1.0;
                any = true;
            }
            i += 1;
        }
    }
    if !any {
        // m ≤ 0.5 everywhere: nothing solves; soft coverage below the Ω
        // threshold also deposits nothing (the §45.1 region rule).
        return Ok(commit_from_window(io, &win, false, plan, |_| [0.0; 4]));
    }

    // Per-channel planes: target = straight dest; source = guidance.
    let mut solved = [Plane::new(w, h), Plane::new(w, h), Plane::new(w, h)];
    let mut target = Plane::new(w, h);
    let mut source = Plane::new(w, h);
    for c in 0..3 {
        let mut i = 0;
        for y in win_rect.min.y..win_rect.max.y {
            for x in win_rect.min.x..win_rect.max.x {
                let dest = win.pre[i];
                target.data[i] = dest[c];
                source.data[i] = guidance(IVec2::new(x, y), dest)[c];
                i += 1;
            }
        }
        // THE §45.1 commit path: multigrid over the masked system
        // (Guidance::Seamless — source gradients; the mixed/Transparent
        // option has no wire param on heal.stroke, recorded).
        let out = poisson_heal(
            &target,
            &source,
            &mask,
            Guidance::Seamless,
            Solver::Multigrid {
                cycles: 20,
                smooth: 2,
            },
        );
        solved[c] = out.plane;
    }

    // Blend back on color only: out = lerp(dest, f, m·opacity); alpha
    // untouched (§45.1); write-back masks to m > 0.
    let mut i = 0;
    for _y in 0..h {
        for _x in 0..w {
            let mv = m[i] * io.opacity;
            if m[i] > 0.0 && mv > 0.0 {
                let dest = win.pre[i];
                let mut out = dest;
                for c in 0..3 {
                    out[c] = dest[c] + (solved[c].data[i] - dest[c]) * mv;
                }
                out[3] = dest[3]; // never modified (§45.1)
                win.cur[i] = out;
                win.changed[i] = true;
            }
            i += 1;
        }
    }
    let cur = win.cur.clone();
    Ok(commit_from_window(io, &win, false, plan, move |i| cur[i]))
}

// ---------------------------------------------------------------------------
// dodge_burn.stroke — §21.5.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToneMode {
    Dodge,
    Burn,
    Saturate,
    Desaturate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToneRange {
    Shadows,
    Midtones,
    Highlights,
}

/// §21.5 range weights (linear Rec. 709 luma).
#[inline]
pub fn range_weight(range: ToneRange, l: f32) -> f32 {
    #[inline]
    fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
        let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }
    match range {
        ToneRange::Shadows => 1.0 - smoothstep(0.0, 0.5, l),
        ToneRange::Midtones => smoothstep(0.0, 1.0, 1.0 - (2.0 * l - 1.0).abs()),
        ToneRange::Highlights => smoothstep(0.5, 1.0, l),
    }
}

#[inline]
fn luma(c: [f32; 4]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// RGB→HSL and back (§21.5 sponge: lightness preserved exactly).
fn rgb_to_hsl(c: [f32; 3]) -> [f32; 3] {
    let max = c[0].max(c[1]).max(c[2]);
    let min = c[0].min(c[1]).min(c[2]);
    let l = 0.5 * (max + min);
    if max == min {
        return [0.0, 0.0, l];
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == c[0] {
        ((c[1] - c[2]) / d).rem_euclid(6.0)
    } else if max == c[1] {
        (c[2] - c[0]) / d + 2.0
    } else {
        (c[0] - c[1]) / d + 4.0
    } / 6.0;
    [h, s, l]
}

fn hsl_to_rgb(hsl: [f32; 3]) -> [f32; 3] {
    let [h, s, l] = hsl;
    if s == 0.0 {
        return [l, l, l];
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let hue = |mut t: f32| -> f32 {
        t = t.rem_euclid(1.0);
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    [hue(h + 1.0 / 3.0), hue(h), hue(h - 1.0 / 3.0)]
}

pub struct ToneSpec {
    pub mode: ToneMode,
    pub range: ToneRange,
    /// Wire `exposure` (0..=1): dodge/burn `e = exposure·0.5`; sponge
    /// amount = exposure (module pins).
    pub exposure: f32,
    /// Carried per the contract; no §21.5 math term (recorded gap).
    pub protect_tones: bool,
}

/// Render one `dodge_burn.stroke` (per-dab incremental §21.5 math on the
/// evolving window; alpha untouched).
pub fn render_dodge_burn_stroke(
    io: &RetouchIo<'_>,
    spec: &ToneSpec,
) -> Result<RetouchCommit, RetouchError> {
    let plan = make_plan(io)?;
    let win_rect = plan.bounds.intersect(&io.clip);
    let mut win = Window::new(io, win_rect, false);
    let e = spec.exposure * 0.5; // §21.5: UI 0–100 % → e ∈ [0, 0.5]
    for dab in &plan.dabs {
        let rect = dab.pixel_rect().intersect(&win_rect);
        for y in rect.min.y..rect.max.y {
            for x in rect.min.x..rect.max.x {
                let p = IVec2::new(x, y);
                let t = dab_coverage_at(dab, p);
                if t <= 0.0 {
                    continue;
                }
                let c = t * dab.flow * sel_at(io, p);
                if c <= 0.0 {
                    continue;
                }
                let i = win.idx(p).expect("rect ⊆ window");
                let v = win.cur[i];
                if v[3] <= 0.0 {
                    continue; // straight color of transparent is 0 — no-op
                }
                let out = match spec.mode {
                    ToneMode::Dodge | ToneMode::Burn => {
                        let wgt = range_weight(spec.range, luma(v));
                        let f = if spec.mode == ToneMode::Dodge {
                            1.0 + e * wgt * c
                        } else {
                            1.0 - e * wgt * c
                        };
                        [
                            (v[0] * f).max(0.0),
                            (v[1] * f).max(0.0),
                            (v[2] * f).max(0.0),
                            v[3],
                        ]
                    }
                    ToneMode::Saturate | ToneMode::Desaturate => {
                        // §21.5 sponge: s′ = clamp(s ± amount·cov);
                        // lightness preserved exactly; range ignored.
                        let mut hsl = rgb_to_hsl([v[0], v[1], v[2]]);
                        let step = spec.exposure * c;
                        hsl[1] = if spec.mode == ToneMode::Saturate {
                            (hsl[1] + step).clamp(0.0, 1.0)
                        } else {
                            (hsl[1] - step).clamp(0.0, 1.0)
                        };
                        let rgb = hsl_to_rgb(hsl);
                        [rgb[0], rgb[1], rgb[2], v[3]]
                    }
                };
                win.cur[i] = out;
                win.changed[i] = true;
            }
        }
    }
    present_evolved(io, win, plan)
}

/// Shared §15.5 step-4 presentation for the evolving-buffer tools:
/// `out = lerp(pre, evolved, opacity)` (alpha follows the same lerp; the
/// alpha-preserving tools never changed it).
fn present_evolved(
    io: &RetouchIo<'_>,
    win: Window,
    plan: StrokePlan,
) -> Result<RetouchCommit, RetouchError> {
    let op = io.opacity;
    let pre = win.pre.clone();
    let cur = win.cur.clone();
    Ok(commit_from_window(io, &win, false, plan, move |i| {
        let mut out = [0.0f32; 4];
        for k in 0..4 {
            out[k] = pre[i][k] + (cur[i][k] - pre[i][k]) * op;
        }
        out
    }))
}

// ---------------------------------------------------------------------------
// smudge.stroke — §21.5/§15.7 pickup-buffer mode.
// ---------------------------------------------------------------------------

pub struct SmudgeSpec {
    /// Contract `strength` (0..=1, default 0.5).
    pub strength: f32,
}

/// Render one `smudge.stroke` (§21.5: `dst′ = lerp(dst, pickup,
/// strength·cov)` then `pickup′ = lerp(dst′, pickup, strength)` — the
/// re-sample after each write is what drags color). Runs in canonical
/// premultiplied form (color + coverage drag together); the TRANSPARENCY
/// constrain re-pins alpha at presentation.
pub fn render_smudge_stroke(
    io: &RetouchIo<'_>,
    spec: &SmudgeSpec,
) -> Result<RetouchCommit, RetouchError> {
    let plan = make_plan(io)?;
    let win_rect = plan.bounds.intersect(&io.clip);
    let mut win = Window::new(io, win_rect, true);
    if win_rect.is_empty() || plan.dabs.is_empty() {
        return Ok(commit_from_window(io, &win, true, plan, |_| [0.0; 4]));
    }

    // Tip-local pickup buffer, indexed by integer offset from the dab
    // center (radius = the largest dab radius + AA margin).
    let max_d = plan.dabs.iter().map(|d| d.diameter).fold(0.0f32, f32::max);
    let pr = (max_d * 0.5).ceil() as i32 + 2;
    let pw = (2 * pr + 1) as usize;
    let mut pickup = vec![[0.0f32; 4]; pw * pw];

    // Initialize from the canvas under the first dab (§21.5).
    let first = &plan.dabs[0];
    let (c0x, c0y) = (
        first.center[0].floor() as i32,
        first.center[1].floor() as i32,
    );
    for oy in -pr..=pr {
        for ox in -pr..=pr {
            let p = IVec2::new(c0x + ox, c0y + oy);
            let v = io.backdrop.read_pixel(p);
            pickup[((oy + pr) as usize) * pw + (ox + pr) as usize] =
                convert_form(v, io.premultiplied, true);
        }
    }

    for dab in &plan.dabs {
        let rect = dab.pixel_rect().intersect(&win_rect);
        let (cx, cy) = (dab.center[0].floor() as i32, dab.center[1].floor() as i32);
        for y in rect.min.y..rect.max.y {
            for x in rect.min.x..rect.max.x {
                let p = IVec2::new(x, y);
                let t = dab_coverage_at(dab, p);
                if t <= 0.0 {
                    continue;
                }
                let (ox, oy) = (x - cx, y - cy);
                if ox < -pr || ox > pr || oy < -pr || oy > pr {
                    continue;
                }
                let pi = ((oy + pr) as usize) * pw + (ox + pr) as usize;
                let i = win.idx(p).expect("rect ⊆ window");
                // Deposits are gated (sel), pick-up reads are unmasked
                // (§40.5 selectionBehavior row).
                let c = spec.strength * t * dab.flow * sel_at(io, p);
                let dst = win.cur[i];
                let pk = pickup[pi];
                let mut dst2 = [0.0f32; 4];
                for k in 0..4 {
                    dst2[k] = dst[k] + (pk[k] - dst[k]) * c;
                }
                if c > 0.0 {
                    win.cur[i] = dst2;
                    win.changed[i] = true;
                }
                // pickup′ = lerp(dst′, pickup, strength) — runs even for
                // gated deposits (the read is unmasked; dst′ = dst then).
                let basis = if c > 0.0 { dst2 } else { dst };
                for k in 0..4 {
                    pickup[pi] = {
                        let mut np = pickup[pi];
                        np[k] = basis[k] + (pk[k] - basis[k]) * spec.strength;
                        np
                    };
                }
            }
        }
    }

    // Presentation: opacity ceiling; TRANSPARENCY constrain re-pins the
    // destination alpha (coverage clamps to existing alpha, §11.1).
    let op = io.opacity;
    let clamp = io.clamp_to_backdrop_alpha;
    let pre = win.pre.clone();
    let cur = win.cur.clone();
    Ok(commit_from_window(io, &win, true, plan, move |i| {
        let mut out = [0.0f32; 4];
        for k in 0..4 {
            out[k] = pre[i][k] + (cur[i][k] - pre[i][k]) * op;
        }
        if clamp {
            // Alpha exactly invariant; color rescales to the kept alpha.
            let a = pre[i][3];
            if out[3] > 0.0 && a > 0.0 {
                let k = a / out[3];
                out = [out[0] * k, out[1] * k, out[2] * k, a];
            } else {
                out = pre[i];
            }
        }
        out
    }))
}

// ---------------------------------------------------------------------------
// blur_sharpen.stroke — §21.5.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusMode {
    Blur,
    Sharpen,
}

pub struct FocusSpec {
    pub mode: FocusMode,
    /// Contract `strength` (0..=1, default 0.5).
    pub strength: f32,
    /// Sharpen only: the §21.5 3×3 min/max ringing clamp (§32.3 T-27
    /// reading — `false` runs the unclamped USM).
    pub protect_detail: bool,
}

/// Gaussian σ for one blur dab (§21.5: σ_dab = 0.5 + 1.5·strength).
#[inline]
pub fn blur_sigma(strength: f32) -> f32 {
    0.5 + 1.5 * strength
}

/// Sampled 1-D Gaussian kernel, radius ⌈3σ⌉, normalized (deterministic
/// fixed-order sum).
fn gaussian_kernel(sigma: f32) -> Vec<f32> {
    let r = (3.0 * sigma).ceil() as i32;
    let mut k = Vec::with_capacity((2 * r + 1) as usize);
    let inv = 1.0 / (2.0 * sigma * sigma);
    let mut sum = 0.0f32;
    for d in -r..=r {
        let v = (-(d as f32) * (d as f32) * inv).exp();
        k.push(v);
        sum += v;
    }
    for v in &mut k {
        *v /= sum;
    }
    k
}

/// Separable Gaussian of the evolving window over `rect`, reading the
/// window where present and the backdrop outside it (canvas-edge reads
/// are §2.4 zeros through the backdrop). Straight color + alpha; alpha
/// itself is NOT written back (the §40.5 "alpha untouched" row) but
/// participates in the blur reads via straight values.
fn gaussian_of_window(io: &RetouchIo<'_>, win: &Window, rect: IRect, sigma: f32) -> Vec<[f32; 4]> {
    let k = gaussian_kernel(sigma);
    let r = (k.len() as i32 - 1) / 2;
    let read = |p: IVec2| -> [f32; 4] {
        match win.idx(p) {
            Some(i) => win.cur[i],
            None => convert_form(io.backdrop.read_pixel(p), io.premultiplied, false),
        }
    };
    let w = rect.width().max(0) as usize;
    let h = rect.height().max(0) as usize;
    // Horizontal pass over rect inflated vertically by r.
    let hrect = IRect::new(rect.min.x, rect.min.y - r, rect.max.x, rect.max.y + r);
    let hh = hrect.height() as usize;
    let mut tmp = vec![[0.0f32; 4]; w * hh];
    for (yi, y) in (hrect.min.y..hrect.max.y).enumerate() {
        for (xi, x) in (rect.min.x..rect.max.x).enumerate() {
            let mut acc = [0.0f32; 4];
            for (di, kv) in k.iter().enumerate() {
                let sx = x + di as i32 - r;
                let v = read(IVec2::new(sx, y));
                for c in 0..4 {
                    acc[c] += v[c] * kv;
                }
            }
            tmp[yi * w + xi] = acc;
        }
    }
    // Vertical pass.
    let mut out = vec![[0.0f32; 4]; w * h];
    for (yi, _y) in (rect.min.y..rect.max.y).enumerate() {
        for xi in 0..w {
            let mut acc = [0.0f32; 4];
            for (di, kv) in k.iter().enumerate() {
                let sy = yi + di; // (yi + r) − r + di in tmp rows
                for c in 0..4 {
                    acc[c] += tmp[sy * w + xi][c] * kv;
                }
            }
            out[yi * w + xi] = acc;
        }
    }
    out
}

/// Render one `blur_sharpen.stroke` (per-dab incremental §21.5 math).
pub fn render_blur_sharpen_stroke(
    io: &RetouchIo<'_>,
    spec: &FocusSpec,
) -> Result<RetouchCommit, RetouchError> {
    let plan = make_plan(io)?;
    let win_rect = plan.bounds.intersect(&io.clip);
    let mut win = Window::new(io, win_rect, false);
    let sigma = match spec.mode {
        FocusMode::Blur => blur_sigma(spec.strength),
        FocusMode::Sharpen => 1.0, // §21.5: USM σ = 1.0 px
    };
    for dab in &plan.dabs {
        let rect = dab.pixel_rect().intersect(&win_rect);
        if rect.is_empty() {
            continue;
        }
        let blurred = gaussian_of_window(io, &win, rect, sigma);
        let w = rect.width() as usize;
        // Snapshot for the 3×3 clamp basis (sharpen reads "the input").
        let snap: Vec<[f32; 4]> = (rect.min.y..rect.max.y)
            .flat_map(|y| (rect.min.x..rect.max.x).map(move |x| IVec2::new(x, y)))
            .map(|p| match win.idx(p) {
                Some(i) => win.cur[i],
                None => convert_form(io.backdrop.read_pixel(p), io.premultiplied, false),
            })
            .collect();
        for (yi, y) in (rect.min.y..rect.max.y).enumerate() {
            for (xi, x) in (rect.min.x..rect.max.x).enumerate() {
                let p = IVec2::new(x, y);
                let t = dab_coverage_at(dab, p);
                if t <= 0.0 {
                    continue;
                }
                let cov = t * dab.flow * sel_at(io, p);
                if cov <= 0.0 {
                    continue;
                }
                let i = win.idx(p).expect("rect ⊆ window");
                let v = win.cur[i];
                let b = blurred[yi * w + xi];
                let out = match spec.mode {
                    FocusMode::Blur => {
                        // out = lerp(in, G_σ(in), cov) — quadrature
                        // build-up across dabs (§21.5).
                        let mut o = v;
                        for c in 0..3 {
                            o[c] = v[c] + (b[c] - v[c]) * cov;
                        }
                        o
                    }
                    FocusMode::Sharpen => {
                        // out = in + a·(in − G_1.0(in)), a = 0.25·s·cov,
                        // clamped to the 3×3 local min/max of the input.
                        let a = 0.25 * spec.strength * cov;
                        let mut o = v;
                        for c in 0..3 {
                            let mut val = v[c] + a * (v[c] - b[c]);
                            if spec.protect_detail {
                                let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
                                for dy in -1i32..=1 {
                                    for dx in -1i32..=1 {
                                        let (nx, ny) = (xi as i32 + dx, yi as i32 + dy);
                                        let s = if nx >= 0
                                            && ny >= 0
                                            && (nx as usize) < w
                                            && ny < rect.height()
                                        {
                                            snap[(ny as usize) * w + nx as usize][c]
                                        } else {
                                            let q = IVec2::new(x + dx, y + dy);
                                            match win.idx(q) {
                                                Some(j) => win.cur[j][c],
                                                None => convert_form(
                                                    io.backdrop.read_pixel(q),
                                                    io.premultiplied,
                                                    false,
                                                )[c],
                                            }
                                        };
                                        lo = lo.min(s);
                                        hi = hi.max(s);
                                    }
                                }
                                val = val.clamp(lo, hi);
                            }
                            o[c] = val;
                        }
                        o
                    }
                };
                win.cur[i] = out;
                win.changed[i] = true;
            }
        }
    }
    present_evolved(io, win, plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brush::present::UniformBackdrop;
    use crate::brush::InputSample;

    fn line_samples(pts: &[(f64, f64)]) -> Vec<InputSample> {
        pts.iter()
            .enumerate()
            .map(|(i, (x, y))| InputSample {
                x: *x,
                y: *y,
                pressure: 1.0,
                tilt_x: 0.0,
                tilt_y: 0.0,
                rotation: 0.0,
                velocity: 0.0,
                t: i as f64 * 0.02,
            })
            .collect()
    }

    /// A raster the tests fully control: straight-storage checkerboard.
    struct TestSurface {
        w: i32,
        h: i32,
        px: Vec<[f32; 4]>,
    }

    impl TestSurface {
        fn gradient(w: i32, h: i32) -> TestSurface {
            let mut px = Vec::with_capacity((w * h) as usize);
            for y in 0..h {
                for x in 0..w {
                    let g = (x as f32) / (w as f32);
                    let b = (y as f32) / (h as f32);
                    px.push([g, 0.5, b, 1.0]);
                }
            }
            TestSurface { w, h, px }
        }
    }

    impl Backdrop for TestSurface {
        fn read_pixel(&self, p: IVec2) -> [f32; 4] {
            if p.x < 0 || p.y < 0 || p.x >= self.w || p.y >= self.h {
                return [0.0; 4];
            }
            self.px[(p.y * self.w + p.x) as usize]
        }
    }

    fn io<'a>(
        surface: &'a TestSurface,
        samples: &'a [InputSample],
        preset: &'a BrushPreset,
    ) -> RetouchIo<'a> {
        RetouchIo {
            samples,
            preset,
            size_px: Some(12.0),
            opacity: 1.0,
            flow: 1.0,
            seed: 7,
            backdrop: surface,
            premultiplied: false,
            clip: IRect::new(0, 0, surface.w, surface.h),
            selection: None,
            clamp_to_backdrop_alpha: false,
        }
    }

    fn patch_value(commit: &RetouchCommit, p: IVec2) -> Option<[f32; 4]> {
        for patch in &commit.patches {
            let r = patch.coord.pixel_rect();
            if r.contains(p) {
                let i = ((p.y - r.min.y) as usize) * TILE + (p.x - r.min.x) as usize;
                return Some(patch.pixels[i]);
            }
        }
        None
    }

    #[test]
    fn clone_copies_offset_content_and_is_deterministic() {
        let canvas_img = TestSurface::gradient(96, 96);
        let samples = line_samples(&[(48.0, 48.0), (52.0, 48.0)]);
        let preset = BrushPreset::hard_round(12.0);
        let io = io(&canvas_img, &samples, &preset);
        let spec = CloneSpec {
            source: &canvas_img,
            source_premultiplied: false,
            offset: [-24.0, 0.0],
            rotation_deg: 0.0,
            scale: 1.0,
        };
        let a = render_clone_stroke(&io, &spec).unwrap();
        let b = render_clone_stroke(&io, &spec).unwrap();
        // Determinism: identical bytes.
        assert_eq!(a.patches.len(), b.patches.len());
        for (pa, pb) in a.patches.iter().zip(&b.patches) {
            assert_eq!(pa.coord, pb.coord);
            assert!(pa.pixels.iter().zip(&pb.pixels).all(|(x, y)| x == y));
        }
        // The dab center now carries the source 24 px to the left
        // (gradient in x → red channel shifted by −24/96).
        let v = patch_value(&a, IVec2::new(48, 48)).expect("center painted");
        let src = canvas_img.read_pixel(IVec2::new(24, 48));
        assert!((v[0] - src[0]).abs() < 1e-3, "got {v:?} want {src:?}");
    }

    #[test]
    fn clone_respects_selection_gate() {
        let canvas_img = TestSurface::gradient(96, 96);
        let samples = line_samples(&[(48.0, 48.0), (52.0, 48.0)]);
        let preset = BrushPreset::hard_round(12.0);
        struct NoSel;
        impl CoverageRead for NoSel {
            fn coverage(&self, _p: IVec2) -> f32 {
                0.0
            }
        }
        let mut io = io(&canvas_img, &samples, &preset);
        io.selection = Some(&NoSel);
        let spec = CloneSpec {
            source: &canvas_img,
            source_premultiplied: false,
            offset: [-24.0, 0.0],
            rotation_deg: 0.0,
            scale: 1.0,
        };
        let commit = render_clone_stroke(&io, &spec).unwrap();
        assert!(commit.patches.is_empty(), "sel(p) = 0 deposits nothing");
    }

    #[test]
    fn heal_matches_boundary_and_keeps_outside_bit_exact() {
        // Flat dark destination, bright source: the §21.2 seamless
        // property — healed interior stays near the destination's level
        // (color from destination, texture from source).
        struct Flat(f32);
        impl Backdrop for Flat {
            fn read_pixel(&self, _p: IVec2) -> [f32; 4] {
                [self.0, self.0, self.0, 1.0]
            }
        }
        let dest = TestSurface {
            w: 96,
            h: 96,
            px: vec![[0.2, 0.2, 0.2, 1.0]; 96 * 96],
        };
        let src = Flat(0.9);
        let samples = line_samples(&[(48.0, 48.0), (50.0, 48.0)]);
        let preset = BrushPreset::hard_round(16.0);
        let mut io = io(&dest, &samples, &preset);
        io.size_px = Some(16.0);
        let spec = HealSampledSpec {
            source: &src,
            source_premultiplied: false,
            offset: [10.0, 0.0],
        };
        let commit = render_heal_stroke(&io, &spec).unwrap();
        assert!(!commit.patches.is_empty());
        let v = patch_value(&commit, IVec2::new(48, 48)).expect("healed center");
        // A flat source has zero gradients → solve = boundary value =
        // destination: NO visible seam, value ≈ 0.2, NOT 0.9.
        assert!(
            (v[0] - 0.2).abs() < 0.02,
            "healed flat region matches destination color, got {v:?}"
        );
        assert_eq!(v[3], 1.0, "alpha untouched");
        // Outside the footprint: no patch pixels differ from backdrop.
        for patch in &commit.patches {
            let r = patch.coord.pixel_rect();
            let mut i = 0;
            for y in r.min.y..r.max.y {
                for x in r.min.x..r.max.x {
                    let p = IVec2::new(x, y);
                    if !commit.bounds.contains(p) {
                        assert_eq!(
                            patch.pixels[i],
                            dest.read_pixel(p),
                            "outside-footprint pixel drifted at {p:?}"
                        );
                    }
                    i += 1;
                }
            }
        }
    }

    #[test]
    fn spot_heal_finds_a_source_or_fails_typed() {
        // Uniform field: every candidate scores 0 → the first valid
        // (radius 2R, angle 0) wins; solve heals to the flat value.
        let dest = TestSurface {
            w: 128,
            h: 128,
            px: vec![[0.5, 0.5, 0.5, 1.0]; 128 * 128],
        };
        let samples = line_samples(&[(64.0, 64.0)]);
        let preset = BrushPreset::hard_round(10.0);
        let mut io = io(&dest, &samples, &preset);
        io.size_px = Some(10.0);
        let commit = render_spot_heal_stroke(&io, SpotArm::Proximity).unwrap();
        let v = patch_value(&commit, IVec2::new(64, 64)).expect("healed");
        assert!((v[0] - 0.5).abs() < 1e-3);

        // A canvas smaller than every annulus candidate → typed failure.
        let tiny = TestSurface {
            w: 12,
            h: 12,
            px: vec![[0.5, 0.5, 0.5, 1.0]; 144],
        };
        let samples2 = line_samples(&[(6.0, 6.0)]);
        let mut io2 = RetouchIo {
            samples: &samples2,
            preset: &preset,
            size_px: Some(10.0),
            opacity: 1.0,
            flow: 1.0,
            seed: 0,
            backdrop: &tiny,
            premultiplied: false,
            clip: IRect::new(0, 0, 12, 12),
            selection: None,
            clamp_to_backdrop_alpha: false,
        };
        io2.size_px = Some(10.0);
        match render_spot_heal_stroke(&io2, SpotArm::Proximity) {
            Err(RetouchError::NoSourceFound) => {}
            other => panic!(
                "expected NoSourceFound, got {:?}",
                other.map(|c| c.patches.len())
            ),
        }
    }

    #[test]
    fn dodge_lightens_burn_darkens_range_weighted() {
        let dest = TestSurface {
            w: 64,
            h: 64,
            px: vec![[0.5, 0.5, 0.5, 1.0]; 64 * 64],
        };
        let samples = line_samples(&[(32.0, 32.0), (34.0, 32.0)]);
        let preset = BrushPreset::hard_round(12.0);
        let io = io(&dest, &samples, &preset);
        let dodge = render_dodge_burn_stroke(
            &io,
            &ToneSpec {
                mode: ToneMode::Dodge,
                range: ToneRange::Midtones,
                exposure: 1.0,
                protect_tones: true,
            },
        )
        .unwrap();
        let v = patch_value(&dodge, IVec2::new(32, 32)).expect("dodged");
        assert!(v[0] > 0.5, "dodge lightens, got {v:?}");
        let burn = render_dodge_burn_stroke(
            &io,
            &ToneSpec {
                mode: ToneMode::Burn,
                range: ToneRange::Midtones,
                exposure: 1.0,
                protect_tones: true,
            },
        )
        .unwrap();
        let v = patch_value(&burn, IVec2::new(32, 32)).expect("burned");
        assert!(v[0] < 0.5, "burn darkens, got {v:?}");
        // Highlights range on a mid-gray: w(0.5) = 0 → no-op (empty or
        // unchanged pixels).
        let hl = render_dodge_burn_stroke(
            &io,
            &ToneSpec {
                mode: ToneMode::Dodge,
                range: ToneRange::Highlights,
                exposure: 1.0,
                protect_tones: true,
            },
        )
        .unwrap();
        if let Some(v) = patch_value(&hl, IVec2::new(32, 32)) {
            assert!((v[0] - 0.5).abs() < 1e-6, "w_highlights(0.5) = 0");
        }
    }

    #[test]
    fn sponge_preserves_lightness_exactly() {
        let dest = TestSurface {
            w: 64,
            h: 64,
            px: vec![[0.7, 0.3, 0.3, 1.0]; 64 * 64],
        };
        let samples = line_samples(&[(32.0, 32.0), (34.0, 32.0)]);
        let preset = BrushPreset::hard_round(12.0);
        let io = io(&dest, &samples, &preset);
        for mode in [ToneMode::Saturate, ToneMode::Desaturate] {
            let commit = render_dodge_burn_stroke(
                &io,
                &ToneSpec {
                    mode,
                    range: ToneRange::Midtones,
                    exposure: 1.0,
                    protect_tones: true,
                },
            )
            .unwrap();
            let v = patch_value(&commit, IVec2::new(32, 32)).expect("sponged");
            let l_in = 0.5 * (0.7 + 0.3);
            let l_out = 0.5 * (v[0].max(v[1]).max(v[2]) + v[0].min(v[1]).min(v[2]));
            assert!(
                (l_in - l_out).abs() < 1e-4,
                "{mode:?}: HSL lightness preserved, {l_in} vs {l_out}"
            );
            let sat_moved = (v[0] - v[1]).abs();
            if mode == ToneMode::Saturate {
                assert!(sat_moved > 0.4, "saturate widens, got {v:?}");
            } else {
                assert!(sat_moved < 0.4, "desaturate narrows, got {v:?}");
            }
        }
    }

    #[test]
    fn smudge_drags_color_along_the_stroke() {
        // Left half red, right half blue; stroke dragging rightward
        // carries red into the blue half.
        let mut px = vec![[0.0f32, 0.0, 1.0, 1.0]; 96 * 96];
        for y in 0..96 {
            for x in 0..48 {
                px[y * 96 + x] = [1.0, 0.0, 0.0, 1.0];
            }
        }
        let dest = TestSurface { w: 96, h: 96, px };
        let samples = line_samples(&[(44.0, 48.0), (52.0, 48.0), (60.0, 48.0)]);
        let preset = BrushPreset::hard_round(10.0);
        let io = io(&dest, &samples, &preset);
        let commit = render_smudge_stroke(&io, &SmudgeSpec { strength: 0.9 }).unwrap();
        let v = patch_value(&commit, IVec2::new(52, 48)).expect("smudged");
        assert!(v[0] > 0.1, "red dragged into the blue half, got {v:?}");
    }

    #[test]
    fn blur_softens_and_sharpen_steepens_an_edge() {
        // Vertical step edge at x = 48.
        let mut px = vec![[0.0f32, 0.0, 0.0, 1.0]; 96 * 96];
        for y in 0..96 {
            for x in 48..96 {
                px[y * 96 + x] = [1.0, 1.0, 1.0, 1.0];
            }
        }
        let dest = TestSurface { w: 96, h: 96, px };
        let samples = line_samples(&[(48.0, 46.0), (48.0, 50.0)]);
        let preset = BrushPreset::hard_round(14.0);
        let io = io(&dest, &samples, &preset);
        let blur = render_blur_sharpen_stroke(
            &io,
            &FocusSpec {
                mode: FocusMode::Blur,
                strength: 1.0,
                protect_detail: false,
            },
        )
        .unwrap();
        // Just left of the edge brightens (blur pulls white across).
        let v = patch_value(&blur, IVec2::new(47, 48)).expect("blurred");
        assert!(v[0] > 0.05, "edge softened leftward, got {v:?}");

        let sharpen = render_blur_sharpen_stroke(
            &io,
            &FocusSpec {
                mode: FocusMode::Sharpen,
                strength: 1.0,
                protect_detail: true,
            },
        )
        .unwrap();
        // The protect-detail clamp keeps values inside the local 3×3
        // min/max — on a pure step, output stays within [0, 1] and the
        // dark side never undershoots.
        if let Some(v) = patch_value(&sharpen, IVec2::new(46, 48)) {
            assert!(v[0] >= 0.0, "no undershoot, got {v:?}");
        }
    }

    #[test]
    fn opacity_is_the_presentation_ceiling() {
        let dest = TestSurface {
            w: 64,
            h: 64,
            px: vec![[0.5, 0.5, 0.5, 1.0]; 64 * 64],
        };
        let samples = line_samples(&[(32.0, 32.0), (34.0, 32.0)]);
        let preset = BrushPreset::hard_round(12.0);
        let mut io_half = io(&dest, &samples, &preset);
        io_half.opacity = 0.5;
        let full = render_dodge_burn_stroke(
            &io(&dest, &samples, &preset),
            &ToneSpec {
                mode: ToneMode::Dodge,
                range: ToneRange::Midtones,
                exposure: 1.0,
                protect_tones: true,
            },
        )
        .unwrap();
        let half = render_dodge_burn_stroke(
            &io_half,
            &ToneSpec {
                mode: ToneMode::Dodge,
                range: ToneRange::Midtones,
                exposure: 1.0,
                protect_tones: true,
            },
        )
        .unwrap();
        let vf = patch_value(&full, IVec2::new(32, 32)).unwrap();
        let vh = patch_value(&half, IVec2::new(32, 32)).unwrap();
        let want = 0.5 + (vf[0] - 0.5) * 0.5;
        assert!(
            (vh[0] - want).abs() < 1e-6,
            "lerp(pre, evolved, 0.5): {vh:?} vs {want}"
        );
    }

    #[test]
    fn uniform_backdrop_transparent_clone_erases_at_canvas_edge() {
        // Cloning from off-canvas (§45.2): source samples are
        // fully-transparent premultiplied → the clone erases toward
        // transparency.
        let dest = TestSurface {
            w: 64,
            h: 64,
            px: vec![[0.5, 0.5, 0.5, 1.0]; 64 * 64],
        };
        let src = UniformBackdrop([0.0; 4]);
        let samples = line_samples(&[(32.0, 32.0), (34.0, 32.0)]);
        let preset = BrushPreset::hard_round(12.0);
        let io = io(&dest, &samples, &preset);
        let spec = CloneSpec {
            source: &src,
            source_premultiplied: true,
            offset: [500.0, 0.0], // far off-canvas → transparent taps
            rotation_deg: 0.0,
            scale: 1.0,
        };
        let commit = render_clone_stroke(&io, &spec).unwrap();
        let v = patch_value(&commit, IVec2::new(32, 32)).expect("center");
        assert!(v[3] < 0.5, "alpha erased toward transparency, got {v:?}");
    }
}
