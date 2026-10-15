// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §4.4 (Porter-Duff source-over with blend, linear premultiplied),
//! §4.5 (blend formulas — W3C Compositing and Blending Level 1 published
//! math, era/standards basis per plan §3.4; identity table `u32` ↔ Rust ↔
//! JSON), §4.8 (the CPU reference skeleton this mirrors), §6.4 (HDR f32);
//! DD ⑤; plan §2.2 "blend set v1: normal, multiply, screen, overlay,
//! darken, lighten, difference, add".
//!
//! **This CPU path is the permanent parity oracle** (ES §34.3): the CP-0.5
//! GPU pipelines will be asserted against it at ≤ 1/255. Correctness beats
//! speed, but the loops are written for the later explicit-SIMD pass:
//! per-tile dispatch (mode resolved once, never per pixel), flat interleaved
//! f32, and only formula-mandated per-pixel selects (un-premultiply guards
//! and the Overlay branch — both selects in the §4.5 WGSL too).
//!
//! Toolchain note (plan §2.2 "explicit SIMD later"): `std::simd` (ES §4.8)
//! is not stabilized on the pinned stable toolchain and the sanctioned
//! `wide` fallback has no deps/ record yet, so v1 is scalar with the §4.8
//! memory layout; lane math lands as a follow-up without API change.
//!
//! **CP-0.5 slice A: the full 27-mode catalog** (plan §2.3 "all 27 blend
//! modes GPU+CPU" — CPU half; formulas verbatim from Appendix A, the
//! [NORMATIVE] "Complete blend-mode formula reference", which the §4.5
//! text table matches). Routing per the §4.5 identity table:
//!
//! * **20 separable modes** — `B(b, s)` per channel: Normal, Darken,
//!   Multiply, ColorBurn, LinearBurn, Lighten, Screen, ColorDodge,
//!   LinearDodge (Add), Overlay, SoftLight, HardLight, VividLight,
//!   LinearLight, PinLight, HardMix, Difference, Exclusion, Subtract,
//!   Divide. (VividLight follows Appendix A —
//!   `s≤0.5 ? ColorBurn(b,2s) : ColorDodge(b,2s−1)` — NOT the §4.5
//!   inline-WGSL sample's "simplified branch"; discrepancy recorded in
//!   the lane log for the v5.6 fold-in.)
//! * **2 whole-pixel modes** — DarkerColor/LighterColor compare `Lum` of
//!   the whole un-premultiplied pixel and return one input untouched;
//!   ties resolve to the **backdrop** [NORMATIVE §4.5].
//! * **4 non-separable modes** — Hue/Saturation/Color/Luminosity via the
//!   Appendix A `Lum/Sat/ClipColor/SetLum/SetSat` helpers on the RGB
//!   triple. `SetSat` uses the §4.5 WGSL vectorized form, proven
//!   equivalent to Appendix A's sorted-channel form by test.
//! * **Dissolve** — alpha-threshold routing (§4.5 [NORMATIVE]): Normal
//!   compositing with `αs` hardened per pixel against the committed
//!   [`crate::bn64::BN64`] blue-noise table in **document** space
//!   (tile seams invisible); deterministic across runs, machines, and
//!   the GPU/CPU paths, compared against its own baseline (Appendix A
//!   parity exception). Needs document context — [`DissolveCtx`]; the
//!   context-free [`composite_tile`] uses the documented zero default.
//!
//! Blend set v1 ([`BLEND_SET_V1`]) remains exported as the CP-0.3
//! golden-suite mode set (O-QA's corpus keys off it until the wave-2
//! extension).
//!
//! **HDR domain note** (§6.4 vs §4.5): §4.5 defines the formulas on
//! normalized values in [0, 1]; f32 working tiles legitimately carry values
//! above 1 (§6.4 super-bright premult). v1 pins the same behavior as the
//! §4.5 WGSL, which performs the identical raw arithmetic on whatever the
//! f32 texture holds: **formulas apply verbatim, with no input clamping**;
//! only clamps that appear in a formula (Linear Dodge's `min(1, …)`) apply.
//! Out-of-[0,1] inputs are therefore a defined formula extension, and the
//! oracle tests pin sample values so the CP-0.5 GPU path inherits exactly
//! this behavior.

use crate::bn64::bn_threshold;
use crate::error::CompositeError;
use crate::tile_rgba::TileRgba;
use appname_core::BlendMode;

/// Blend set v1 (plan §2.2), in ES §4.5 discriminant order.
pub const BLEND_SET_V1: [BlendMode; 8] = [
    BlendMode::Normal,
    BlendMode::Darken,
    BlendMode::Multiply,
    BlendMode::Lighten,
    BlendMode::Screen,
    BlendMode::LinearDodge,
    BlendMode::Overlay,
    BlendMode::Difference,
];

// --- The separable blend functions, one per §4.5 formula line. -----------

#[inline]
fn b_normal(_b: f32, s: f32) -> f32 {
    s
}

#[inline]
fn b_darken(b: f32, s: f32) -> f32 {
    b.min(s)
}

#[inline]
fn b_multiply(b: f32, s: f32) -> f32 {
    b * s
}

#[inline]
fn b_lighten(b: f32, s: f32) -> f32 {
    b.max(s)
}

#[inline]
fn b_screen(b: f32, s: f32) -> f32 {
    b + s - b * s
}

#[inline]
fn b_linear_dodge(b: f32, s: f32) -> f32 {
    (b + s).min(1.0)
}

/// §4.5: `Overlay: B = HardLight(s, b)` with
/// `HardLight(b, s) = (s ≤ 0.5) ? Multiply(b, 2s) : Screen(b, 2s−1)`;
/// swapping arguments gives the branch on the backdrop below.
#[inline]
fn b_overlay(b: f32, s: f32) -> f32 {
    if b <= 0.5 {
        2.0 * s * b
    } else {
        1.0 - 2.0 * (1.0 - s) * (1.0 - b)
    }
}

#[inline]
fn b_difference(b: f32, s: f32) -> f32 {
    (b - s).abs()
}

/// §4.5/A: `Color Burn: B = (b==1) ? 1 : (s==0) ? 0 : 1 − min(1, (1−b)/s)`.
/// W3C Compositing and Blending Level 1: the leading `Cb == 1 -> 1` backdrop
/// guard (a fully-lit backdrop stays lit) precedes the `Cs == 0 -> 0` case.
#[inline]
fn b_color_burn(b: f32, s: f32) -> f32 {
    if b == 1.0 {
        1.0
    } else if s == 0.0 {
        0.0
    } else {
        1.0 - ((1.0 - b) / s).min(1.0)
    }
}

/// §4.5/A: `Linear Burn: B = max(0, b + s − 1)`.
#[inline]
fn b_linear_burn(b: f32, s: f32) -> f32 {
    (b + s - 1.0).max(0.0)
}

/// §4.5/A: `Color Dodge: B = (b==0) ? 0 : (s==1) ? 1 : min(1, b/(1−s))`.
/// W3C Compositing and Blending Level 1: the leading `Cb == 0 -> 0` backdrop
/// guard (a fully-dark backdrop stays dark) precedes the `Cs == 1 -> 1` case.
#[inline]
fn b_color_dodge(b: f32, s: f32) -> f32 {
    if b == 0.0 {
        0.0
    } else if s == 1.0 {
        1.0
    } else {
        (b / (1.0 - s)).min(1.0)
    }
}

/// §4.5/A: `Soft Light: s≤0.5 ? b−(1−2s)·b·(1−b) : b+(2s−1)·(D(b)−b)`,
/// `D(b) = b≤0.25 ? ((16b−12)b+4)b : √b`.
#[inline]
fn b_soft_light(b: f32, s: f32) -> f32 {
    if s <= 0.5 {
        b - (1.0 - 2.0 * s) * b * (1.0 - b)
    } else {
        let d = if b <= 0.25 {
            ((16.0 * b - 12.0) * b + 4.0) * b
        } else {
            b.sqrt()
        };
        b + (2.0 * s - 1.0) * (d - b)
    }
}

/// §4.5/A: `Hard Light: B = (s≤0.5) ? Multiply(b, 2s) : Screen(b, 2s−1)`
/// (expanded: `s≤0.5 ? 2·b·s : 1−2(1−b)(1−s)`). [`b_overlay`] is this
/// function with swapped arguments.
#[inline]
fn b_hard_light(b: f32, s: f32) -> f32 {
    if s <= 0.5 {
        b_multiply(b, 2.0 * s)
    } else {
        b_screen(b, 2.0 * s - 1.0)
    }
}

/// Appendix A row 15 (the [NORMATIVE] formula table; also the §4.5 text
/// line): `Vivid Light: B = (s≤0.5) ? ColorBurn(b, 2s) : ColorDodge(b,
/// 2s−1)`. The §4.5 inline-WGSL sample's case 15u differs and is
/// annotated "simplified" — the formula table governs (lane-log record).
#[inline]
fn b_vivid_light(b: f32, s: f32) -> f32 {
    if s <= 0.5 {
        b_color_burn(b, 2.0 * s)
    } else {
        b_color_dodge(b, 2.0 * s - 1.0)
    }
}

/// §4.5/A: `Linear Light: B = clamp(b + 2s − 1, 0, 1)`.
#[inline]
fn b_linear_light(b: f32, s: f32) -> f32 {
    (b + 2.0 * s - 1.0).clamp(0.0, 1.0)
}

/// §4.5/A: `Pin Light: B = (s≤0.5) ? min(b, 2s) : max(b, 2s−1)`.
#[inline]
fn b_pin_light(b: f32, s: f32) -> f32 {
    if s <= 0.5 {
        b.min(2.0 * s)
    } else {
        b.max(2.0 * s - 1.0)
    }
}

/// §4.5: `Hard Mix: B = (b + s ≥ 1) ? 1 : 0` — the concrete per-channel
/// rule of both the §4.5 text table and its WGSL (`(b + s) >= 1.0`), and
/// the [NORMATIVE] implemented form. Appendix A states it as the
/// VividLight≥0.5 threshold; the two forms agree on the whole grid EXCEPT
/// the single backdrop endpoint `b=0, s=1`, where this text form gives 1
/// (0+1≥1) but the Appendix-A form gives 0 because the now-correct
/// [`b_color_dodge`] preserves the black backdrop (ColorDodge(0,1)=0).
/// The symmetric burn endpoint `b=1, s=0` DOES agree once
/// [`b_color_burn`] carries its `Cb==1` guard (both give 1). There is no
/// state with both W3C backdrop guards present in which the two published
/// statements coincide everywhere — recorded for O-INT/Appendix-A
/// adjudication (aud-corr CRITICAL-1); the §4.5 text form governs here.
#[inline]
fn b_hard_mix(b: f32, s: f32) -> f32 {
    if b + s >= 1.0 {
        1.0
    } else {
        0.0
    }
}

/// §4.5/A: `Exclusion: B = b + s − 2·b·s`.
#[inline]
fn b_exclusion(b: f32, s: f32) -> f32 {
    b + s - 2.0 * b * s
}

/// §4.5/A: `Subtract: B = max(0, b − s)`.
#[inline]
fn b_subtract(b: f32, s: f32) -> f32 {
    (b - s).max(0.0)
}

/// §4.5/A: `Divide: B = (s==0) ? 1 : min(1, b/s)`.
#[inline]
fn b_divide(b: f32, s: f32) -> f32 {
    if s == 0.0 {
        1.0
    } else {
        (b / s).min(1.0)
    }
}

// --- Non-separable helpers (Appendix A, W3C published algorithms). -------

/// Appendix A: `Lum(C) = 0.3·R + 0.59·G + 0.11·B`.
#[inline]
pub(crate) fn lum(c: [f32; 3]) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

/// Appendix A: `Sat(C) = max(R,G,B) − min(R,G,B)`.
#[inline]
fn sat(c: [f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

/// Appendix A `ClipColor` — pull out-of-gamut channels back toward the
/// luminance, preserving `Lum`. `l`, `n`, `x` are computed once up front,
/// exactly as the Appendix A pseudocode and the §4.5 WGSL do (the `x > 1`
/// branch deliberately reads the pre-`n<0`-fix extremum).
///
/// **HDR-domain degenerate pin** (module-doc domain note): with inputs
/// confined to [0, 1] the formula's denominators are strictly positive
/// wherever a branch fires (`x > 1 ⇒ x > Lum`, since `Lum ≤ max` and a
/// uniform triple above 1 cannot arise in gamut). The §6.4 HDR extension
/// CAN produce a uniform out-of-range triple (e.g. `SetLum` of a gray
/// backdrop to an HDR source's `Lum > 1`), where the formula evaluates
/// `0 · ∞ = NaN`. NaN must never enter working tiles, so the degenerate
/// case takes the formula's continuous limit — the 1-triple above, the
/// 0-triple below (`hdr_degenerate_clip_color_pins_limit_values`). The
/// slice-D WGSL implements the identical strict-inequality guards
/// (parity-harness handoff, lane log).
#[inline]
fn clip_color(mut c: [f32; 3]) -> [f32; 3] {
    let l = lum(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    if n < 0.0 {
        if l > n {
            for ch in &mut c {
                *ch = l + (*ch - l) * l / (l - n);
            }
        } else {
            c = [0.0; 3]; // degenerate uniform ≤ 0: the l→n limit
        }
    }
    if x > 1.0 {
        if x > l {
            for ch in &mut c {
                *ch = l + (*ch - l) * (1.0 - l) / (x - l);
            }
        } else {
            c = [1.0; 3]; // degenerate uniform > 1: the x→l limit
        }
    }
    c
}

/// Appendix A: `SetLum(C, l) = ClipColor(C + (l − Lum(C)))`.
#[inline]
fn set_lum(c: [f32; 3], l: f32) -> [f32; 3] {
    let d = l - lum(c);
    clip_color([c[0] + d, c[1] + d, c[2] + d])
}

/// `SetSat(C, s)` in the §4.5 WGSL vectorized form — maps min→0, max→s,
/// mid→(mid−min)·s/(max−min); identical to Appendix A's sorted-channel
/// form with channel identity preserved (equivalence pinned by
/// `set_sat_vectorized_equals_appendix_a_sorted_form`).
#[inline]
fn set_sat(c: [f32; 3], s: f32) -> [f32; 3] {
    let cmin = c[0].min(c[1]).min(c[2]);
    let cmax = c[0].max(c[1]).max(c[2]);
    if cmax > cmin {
        [
            (c[0] - cmin) * s / (cmax - cmin),
            (c[1] - cmin) * s / (cmax - cmin),
            (c[2] - cmin) * s / (cmax - cmin),
        ]
    } else {
        [0.0, 0.0, 0.0]
    }
}

/// A row 23: `Hue = SetLum(SetSat(Cs, Sat(Cb)), Lum(Cb))`.
#[inline]
fn b_hue(cb: [f32; 3], cs: [f32; 3]) -> [f32; 3] {
    set_lum(set_sat(cs, sat(cb)), lum(cb))
}

/// A row 24: `Saturation = SetLum(SetSat(Cb, Sat(Cs)), Lum(Cb))`.
#[inline]
fn b_saturation(cb: [f32; 3], cs: [f32; 3]) -> [f32; 3] {
    set_lum(set_sat(cb, sat(cs)), lum(cb))
}

/// A row 25: `Color = SetLum(Cs, Lum(Cb))`.
#[inline]
fn b_color(cb: [f32; 3], cs: [f32; 3]) -> [f32; 3] {
    set_lum(cs, lum(cb))
}

/// A row 26: `Luminosity = SetLum(Cb, Lum(Cs))`.
#[inline]
fn b_luminosity(cb: [f32; 3], cs: [f32; 3]) -> [f32; 3] {
    set_lum(cb, lum(cs))
}

/// §4.5 whole-pixel row 6: smaller luminance wins; **tie → backdrop**
/// [NORMATIVE] (deterministic, order-stable).
#[inline]
fn b_darker_color(cb: [f32; 3], cs: [f32; 3]) -> [f32; 3] {
    if lum(cb) <= lum(cs) {
        cb
    } else {
        cs
    }
}

/// §4.5 whole-pixel row 11: larger luminance wins; **tie → backdrop**
/// [NORMATIVE].
#[inline]
fn b_lighter_color(cb: [f32; 3], cs: [f32; 3]) -> [f32; 3] {
    if lum(cb) >= lum(cs) {
        cb
    } else {
        cs
    }
}

// --- Routing (the §4.5 identity table's "Routing" column). ---------------

/// How a mode enters the Porter-Duff wrapper — the §4.5 identity table's
/// Routing column, verbatim: `sep`, `whole-pixel` (6/11), `non-sep`
/// (23–26), `alpha-threshold` (Dissolve).
#[derive(Clone, Copy)]
pub enum BlendRouting {
    /// Per-channel `B(b, s)`.
    Separable(fn(f32, f32) -> f32),
    /// Whole-pixel luminance pick (DarkerColor/LighterColor).
    WholePixel(fn([f32; 3], [f32; 3]) -> [f32; 3]),
    /// RGB-triple `B(Cb, Cs)` (Hue/Saturation/Color/Luminosity).
    NonSeparable(fn([f32; 3], [f32; 3]) -> [f32; 3]),
    /// Dissolve: Normal compositing with `αs` hardened against blue noise
    /// per document-space pixel — needs a [`DissolveCtx`].
    Dissolve,
}

impl core::fmt::Debug for BlendRouting {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            BlendRouting::Separable(_) => "Separable",
            BlendRouting::WholePixel(_) => "WholePixel",
            BlendRouting::NonSeparable(_) => "NonSeparable",
            BlendRouting::Dissolve => "Dissolve",
        })
    }
}

/// Document context for Dissolve (§4.5): the per-layer blue-noise offsets
/// (seed rule — [`crate::bn64::bn_offsets`]) and the composited tile's
/// document-space origin (`p_doc = tile_origin + p`, so tile seams are
/// invisible).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct DissolveCtx {
    /// Layer-decorrelated table offsets (`seed[0] & 63`, `seed[1] & 63`).
    pub bn_off: (i32, i32),
    /// Document-space pixel of the tile's (0, 0) — for tile (tx, ty) at
    /// lod 0 this is `(tx·256, ty·256)`.
    pub tile_origin: (i32, i32),
}

/// Resolve a mode's routing — the per-tile dispatch point (ES §4.8 "match
/// once per tile, never per pixel"). Total over all 27 modes.
#[inline]
pub fn routing(mode: BlendMode) -> BlendRouting {
    use BlendRouting::{Dissolve, NonSeparable, Separable, WholePixel};
    match mode {
        BlendMode::Normal => Separable(b_normal),
        BlendMode::Dissolve => Dissolve,
        BlendMode::Darken => Separable(b_darken),
        BlendMode::Multiply => Separable(b_multiply),
        BlendMode::ColorBurn => Separable(b_color_burn),
        BlendMode::LinearBurn => Separable(b_linear_burn),
        BlendMode::DarkerColor => WholePixel(b_darker_color),
        BlendMode::Lighten => Separable(b_lighten),
        BlendMode::Screen => Separable(b_screen),
        BlendMode::ColorDodge => Separable(b_color_dodge),
        BlendMode::LinearDodge => Separable(b_linear_dodge),
        BlendMode::LighterColor => WholePixel(b_lighter_color),
        BlendMode::Overlay => Separable(b_overlay),
        BlendMode::SoftLight => Separable(b_soft_light),
        BlendMode::HardLight => Separable(b_hard_light),
        BlendMode::VividLight => Separable(b_vivid_light),
        BlendMode::LinearLight => Separable(b_linear_light),
        BlendMode::PinLight => Separable(b_pin_light),
        BlendMode::HardMix => Separable(b_hard_mix),
        BlendMode::Difference => Separable(b_difference),
        BlendMode::Exclusion => Separable(b_exclusion),
        BlendMode::Subtract => Separable(b_subtract),
        BlendMode::Divide => Separable(b_divide),
        BlendMode::Hue => NonSeparable(b_hue),
        BlendMode::Saturation => NonSeparable(b_saturation),
        BlendMode::Color => NonSeparable(b_color),
        BlendMode::Luminosity => NonSeparable(b_luminosity),
    }
}

// --- The §14.4 kernel-injection seam (CP-1.0 wave 1, O-INT dispatch). --

/// A §4.5 separable blend kernel `B(b, s)` — the [`render_separable_kernel`]
/// export shape (named so consumer graphs stay clippy-clean; C4-builder
/// routing note).
pub type SeparableKernel = fn(f32, f32) -> f32;

/// A §4.5 RGB-triple kernel `B(Cb, Cs)` (non-separable + whole-pixel
/// modes) — the [`render_triple_kernel`] export shape.
pub type TripleKernel = fn([f32; 3], [f32; 3]) -> [f32; 3];

/// The §14.4 injection seam, separable arm: the per-mode CPU kernel for
/// core's `channel.apply_image`/`channel.calculations` doors, consumed
/// as `apply_blend_plane(render_separable_kernel(mode), …)` (the C4
/// call shape, O-COLOR log addendum "D1 fix, §14.4 injection seam made
/// total"). `None` ⟺ the mode routes to the triple arm
/// ([`render_triple_kernel`]) or is Dissolve — which exports NOTHING
/// from this seam: §14.4's math block is purely per-value while §4.5
/// Dissolve [NORMATIVE] is BN64 coordinate-hashed coverage, so the
/// color side types it reserved (`dissolve_reserved`, fold-in record
/// in the O-COLOR log; this crate deliberately has no coordinate-free
/// dissolve kernel to leak).
///
/// ONE-MATH-LIBRARY CLAUSE (ES §14.4): these are the very fn pointers
/// the §4.4 compositor dispatches — not copies — so `apply multiply ≡
/// Multiply-layer composite` holds by construction on the kernel side
/// (pointer-identity pinned by test; the document-level bit-identity
/// test lands core-side per the C4 record).
pub fn render_separable_kernel(mode: BlendMode) -> Option<SeparableKernel> {
    match routing(mode) {
        BlendRouting::Separable(f) => Some(f),
        BlendRouting::WholePixel(_) | BlendRouting::NonSeparable(_) | BlendRouting::Dissolve => {
            None
        }
    }
}

/// The §14.4 injection seam, triple arm: the RGB-triple kernel for the
/// non-separable (Hue/Saturation/Color/Luminosity) and whole-pixel
/// (DarkerColor/LighterColor) modes, consumed as
/// `apply_blend_planes3(render_triple_kernel(mode), [t_r, t_g, t_b],
/// [s_r, s_g, s_b], …)` (C4 call shape). `None` ⟺ separable arm or
/// Dissolve (see [`render_separable_kernel`] — the partition over the
/// 27-mode catalog is exactly 20 separable + 6 triple + Dissolve,
/// test-pinned).
pub fn render_triple_kernel(mode: BlendMode) -> Option<TripleKernel> {
    match routing(mode) {
        BlendRouting::WholePixel(f) | BlendRouting::NonSeparable(f) => Some(f),
        BlendRouting::Separable(_) | BlendRouting::Dissolve => None,
    }
}

/// Resolve a mode to its separable blend function; `None` for the 7
/// non-separable-routed modes (Dissolve, DarkerColor, LighterColor,
/// Hue, Saturation, Color, Luminosity). Test-suite convenience today;
/// the slice-C DAG's per-pass dispatch consumes it next.
#[cfg_attr(not(test), allow(dead_code))]
#[inline]
pub(crate) fn separable_fn(mode: BlendMode) -> Option<fn(f32, f32) -> f32> {
    match routing(mode) {
        BlendRouting::Separable(f) => Some(f),
        _ => None,
    }
}

/// Is `mode` implemented? Since CP-0.5 slice A: always `true` (the full
/// §4.5 catalog). Kept exported for the CP-0.3 conformance surface until
/// O-QA's wave-2 extension retires it.
pub fn blend_implemented(_mode: BlendMode) -> bool {
    true
}

/// Composite one source tile over one destination tile in place — the CPU
/// mirror of the §4.5 `cs_composite` compute kernel (ES §4.8 skeleton),
/// scalar form.
///
/// Inputs are working tiles (linear premultiplied f32, §4.4). `opacity` is
/// the effective source-alpha scale in `[0, 1]` — at CP-0.3 (no layer
/// effects, §11.3) both layer opacity and fill opacity reduce to exactly
/// this scale, so callers pass their product (the WGSL's `u.opacity` slot).
///
/// Per pixel, with stored source alpha `sa`, `αs = sa·opacity`, backdrop
/// alpha `αb` (§4.4):
///
/// ```text
/// Cs = premult_src / sa   (sa > 0, else 0)      — §4.5 un-premultiply guard
/// Cb = premult_dst / αb   (αb > 0, else 0)
/// Co = αs·(1−αb)·Cs + αs·αb·B(Cb,Cs) + (1−αs)·αb·Cb
/// αo = αs + αb·(1−αs)
/// ```
///
/// All 27 modes composite (CP-0.5 slice A); the `Result` is kept for the
/// CP-0.3 conformance-suite surface and is always `Ok` — `Err` never
/// occurs since the full catalog landed.
///
/// Dissolve through this context-free door uses the **documented zero
/// default** `DissolveCtx { bn_off: (0, 0), tile_origin: (0, 0) }` — the
/// deterministic pattern of a layer whose seed hashes to offset (0, 0)
/// composited at tile (0, 0). Document walks supply real context via
/// [`composite_tile_ctx`].
pub fn composite_tile(
    dst: &mut TileRgba,
    src: &TileRgba,
    mode: BlendMode,
    opacity: f32,
) -> Result<(), CompositeError> {
    composite_tile_ctx(dst, src, mode, opacity, DissolveCtx::default());
    Ok(())
}

/// Composite with explicit Dissolve context — the full §4.5 routing
/// (`cs_composite_full`'s CPU mirror). `ctx` is read only by Dissolve.
pub fn composite_tile_ctx(
    dst: &mut TileRgba,
    src: &TileRgba,
    mode: BlendMode,
    opacity: f32,
    ctx: DissolveCtx,
) {
    match routing(mode) {
        BlendRouting::Separable(f) => composite_tile_with(dst, src, f, opacity),
        BlendRouting::WholePixel(f) | BlendRouting::NonSeparable(f) => {
            composite_tile_triple(dst, src, f, opacity)
        }
        BlendRouting::Dissolve => composite_tile_dissolve(dst, src, opacity, ctx),
    }
}

/// Uniform-source composite (MINOR-5): one working-space source value `w`
/// (linear premultiplied RGBA — a `Uniform` tile's converted value) over
/// every pixel of `dst`, WITHOUT materializing a 256² source tile.
///
/// Bit-equality contract with the materialized path: the per-pixel kernel
/// computes `sa`, `a_s`, and `cs = sp[c]/sa` from source scalars that are
/// IDENTICAL at every pixel of a splatted tile, so hoisting them out of
/// the loop reuses the very same f32 values; the remaining per-pixel
/// arithmetic (`a_b`, `a_o`, `cb`, `B`, Porter-Duff) is expression-
/// identical. Proven by `uniform_src_kernel_is_bitequal_to_materialized`
/// (all 8 modes × alpha/HDR/opacity grid, `to_bits` equality).
pub(crate) fn composite_tile_uniform_src(
    dst: &mut TileRgba,
    w: [f32; 4],
    f: fn(f32, f32) -> f32,
    opacity: f32,
) {
    debug_assert!(
        (0.0..=1.0).contains(&opacity),
        "opacity is a [0,1] alpha scale (§4.4)"
    );
    let sa = w[3];
    let a_s = sa * opacity;
    // Hoisted un-premultiply (§4.5/§4.8 guard): a == 0 ⇒ color 0.
    let cs = [
        if sa > 0.0 { w[0] / sa } else { 0.0 },
        if sa > 0.0 { w[1] / sa } else { 0.0 },
        if sa > 0.0 { w[2] / sa } else { 0.0 },
    ];
    for dp in dst.as_mut_slice().chunks_exact_mut(4) {
        let a_b = dp[3];
        let a_o = a_s + a_b * (1.0 - a_s);
        for c in 0..3 {
            let cb = if a_b > 0.0 { dp[c] / a_b } else { 0.0 };
            let bl = f(cb, cs[c]);
            dp[c] = a_s * (1.0 - a_b) * cs[c] + a_s * a_b * bl + (1.0 - a_s) * a_b * cb;
        }
        dp[3] = a_o;
    }
}

/// The dispatched inner kernel (mode already resolved — per-tile, §4.8).
pub(crate) fn composite_tile_with(
    dst: &mut TileRgba,
    src: &TileRgba,
    f: fn(f32, f32) -> f32,
    opacity: f32,
) {
    debug_assert!(
        (0.0..=1.0).contains(&opacity),
        "opacity is a [0,1] alpha scale (§4.4)"
    );
    let d = dst.as_mut_slice();
    let s = src.as_slice();
    for (dp, sp) in d.chunks_exact_mut(4).zip(s.chunks_exact(4)) {
        let sa = sp[3];
        let a_s = sa * opacity;
        let a_b = dp[3];
        let a_o = a_s + a_b * (1.0 - a_s);
        for c in 0..3 {
            // Un-premultiply guards per §4.5/§4.8: a == 0 ⇒ color 0
            // (select-style — the §4.8 lanes use the same guarded divide).
            let cs = if sa > 0.0 { sp[c] / sa } else { 0.0 };
            let cb = if a_b > 0.0 { dp[c] / a_b } else { 0.0 };
            let bl = f(cb, cs);
            dp[c] = a_s * (1.0 - a_b) * cs + a_s * a_b * bl + (1.0 - a_s) * a_b * cb;
        }
        dp[3] = a_o;
    }
}

/// Triple-function kernel: whole-pixel (DarkerColor/LighterColor) and
/// non-separable (Hue/Saturation/Color/Luminosity) modes. Identical
/// Porter-Duff tail to [`composite_tile_with`]; only `B` takes the RGB
/// triple instead of one channel (§4.5 `cs_composite_full` routing).
pub(crate) fn composite_tile_triple(
    dst: &mut TileRgba,
    src: &TileRgba,
    f: fn([f32; 3], [f32; 3]) -> [f32; 3],
    opacity: f32,
) {
    debug_assert!(
        (0.0..=1.0).contains(&opacity),
        "opacity is a [0,1] alpha scale (§4.4)"
    );
    let d = dst.as_mut_slice();
    let s = src.as_slice();
    for (dp, sp) in d.chunks_exact_mut(4).zip(s.chunks_exact(4)) {
        let sa = sp[3];
        let a_s = sa * opacity;
        let a_b = dp[3];
        let a_o = a_s + a_b * (1.0 - a_s);
        let cs = unpremult3(sp, sa);
        let cb = unpremult3(dp, a_b);
        let bl = f(cb, cs);
        for c in 0..3 {
            dp[c] = a_s * (1.0 - a_b) * cs[c] + a_s * a_b * bl[c] + (1.0 - a_s) * a_b * cb[c];
        }
        dp[3] = a_o;
    }
}

/// Dissolve kernel (§4.5 [NORMATIVE]): per document-space pixel, `αs`
/// hardens to 1 or 0 by `α_eff > n(x, y)` against [`crate::bn64::BN64`];
/// the Porter-Duff tail then runs with `B = Cs` (Normal) — the full
/// expression, matching `cs_composite_full` (no per-pixel shortcut), so
/// the GPU path is the same arithmetic. Failing pixels therefore take
/// the αs = 0 expression `(1)·αb·(p/αb)` — the documented CP-0.3
/// dense-kernel convention for transparent pixels (only *uniform* tiles
/// carry the normative zero-alpha skip, at the compositor).
pub(crate) fn composite_tile_dissolve(
    dst: &mut TileRgba,
    src: &TileRgba,
    opacity: f32,
    ctx: DissolveCtx,
) {
    debug_assert!(
        (0.0..=1.0).contains(&opacity),
        "opacity is a [0,1] alpha scale (§4.4)"
    );
    let d = dst.as_mut_slice();
    let s = src.as_slice();
    for (i, (dp, sp)) in d.chunks_exact_mut(4).zip(s.chunks_exact(4)).enumerate() {
        let (x, y) = (
            (i % appname_core::TILE) as i32,
            (i / appname_core::TILE) as i32,
        );
        let n = bn_threshold(ctx.tile_origin.0 + x, ctx.tile_origin.1 + y, ctx.bn_off);
        let sa = sp[3];
        let a_eff = sa * opacity;
        let a_s = if a_eff > n { 1.0 } else { 0.0 };
        let a_b = dp[3];
        let a_o = a_s + a_b * (1.0 - a_s);
        for c in 0..3 {
            let cs = if sa > 0.0 { sp[c] / sa } else { 0.0 };
            let cb = if a_b > 0.0 { dp[c] / a_b } else { 0.0 };
            // B = Cs (Normal) — Dissolve's blend is Normal by definition.
            dp[c] = a_s * (1.0 - a_b) * cs + a_s * a_b * cs + (1.0 - a_s) * a_b * cb;
        }
        dp[3] = a_o;
    }
}

/// Uniform-source triple kernel — the [`composite_tile_uniform_src`]
/// analog for whole-pixel/non-separable modes: hoists the source-side
/// scalars (identical at every pixel of a splatted tile — same f32
/// values, bit-equal contract proven by
/// `uniform_src_triple_kernel_is_bitequal_to_materialized`); `B` still
/// evaluates per pixel (it reads the backdrop triple).
pub(crate) fn composite_tile_uniform_src_triple(
    dst: &mut TileRgba,
    w: [f32; 4],
    f: fn([f32; 3], [f32; 3]) -> [f32; 3],
    opacity: f32,
) {
    debug_assert!(
        (0.0..=1.0).contains(&opacity),
        "opacity is a [0,1] alpha scale (§4.4)"
    );
    let sa = w[3];
    let a_s = sa * opacity;
    let cs = unpremult3(&w, sa);
    for dp in dst.as_mut_slice().chunks_exact_mut(4) {
        let a_b = dp[3];
        let a_o = a_s + a_b * (1.0 - a_s);
        let cb = unpremult3(dp, a_b);
        let bl = f(cb, cs);
        for c in 0..3 {
            dp[c] = a_s * (1.0 - a_b) * cs[c] + a_s * a_b * bl[c] + (1.0 - a_s) * a_b * cb[c];
        }
        dp[3] = a_o;
    }
}

/// Uniform-source Dissolve kernel — hoists `cs`/`α_eff`; the threshold
/// still reads per document-space pixel. Bit-equal to splat +
/// [`composite_tile_dissolve`] (proven by
/// `uniform_src_dissolve_kernel_is_bitequal_to_materialized`).
pub(crate) fn composite_tile_uniform_src_dissolve(
    dst: &mut TileRgba,
    w: [f32; 4],
    opacity: f32,
    ctx: DissolveCtx,
) {
    debug_assert!(
        (0.0..=1.0).contains(&opacity),
        "opacity is a [0,1] alpha scale (§4.4)"
    );
    let sa = w[3];
    let a_eff = sa * opacity;
    let cs = unpremult3(&w, sa);
    for (i, dp) in dst.as_mut_slice().chunks_exact_mut(4).enumerate() {
        let (x, y) = (
            (i % appname_core::TILE) as i32,
            (i / appname_core::TILE) as i32,
        );
        let n = bn_threshold(ctx.tile_origin.0 + x, ctx.tile_origin.1 + y, ctx.bn_off);
        let a_s = if a_eff > n { 1.0 } else { 0.0 };
        let a_b = dp[3];
        let a_o = a_s + a_b * (1.0 - a_s);
        for c in 0..3 {
            let cb = if a_b > 0.0 { dp[c] / a_b } else { 0.0 };
            dp[c] = a_s * (1.0 - a_b) * cs[c] + a_s * a_b * cs[c] + (1.0 - a_s) * a_b * cb;
        }
        dp[3] = a_o;
    }
}

/// Guarded triple un-premultiply (§4.5): `a == 0 ⇒ color 0`.
#[inline]
fn unpremult3(p: &[f32], a: f32) -> [f32; 3] {
    if a > 0.0 {
        [p[0] / a, p[1] / a, p[2] / a]
    } else {
        [0.0, 0.0, 0.0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Scalar §4.4 composite of one pixel — used by the oracle tests to
    /// state expectations in un-premultiplied terms.
    fn pd(cs: f32, a_s: f32, cb: f32, a_b: f32, b: f32) -> (f32, f32) {
        (
            a_s * (1.0 - a_b) * cs + a_s * a_b * b + (1.0 - a_s) * a_b * cb,
            a_s + a_b * (1.0 - a_s),
        )
    }

    /// Composite one pixel through the tile kernel (premultiplied inputs).
    fn run(mode: BlendMode, src: [f32; 4], dst: [f32; 4], opacity: f32) -> [f32; 4] {
        let s = TileRgba::splat(src);
        let mut d = TileRgba::splat(dst);
        composite_tile(&mut d, &s, mode, opacity).expect("v1 mode");
        // Every pixel saw identical inputs; probe two corners to catch any
        // stride bug, then return one.
        assert_eq!(d.get(0, 0), d.get(255, 255));
        d.get(0, 0)
    }

    const EPS: f32 = 1e-6;

    fn assert_px(got: [f32; 4], want_c: f32, want_a: f32) {
        for (c, &g) in got.iter().enumerate().take(3) {
            assert!(
                (g - want_c).abs() <= EPS,
                "channel {c}: got {g} want {want_c}"
            );
        }
        assert!(
            (got[3] - want_a).abs() <= EPS,
            "alpha: got {} want {want_a}",
            got[3]
        );
    }

    /// The 8-mode oracle table on the ES §4.4 worked-example pixel pair:
    /// source Cs = 0.6 at αs = 0.5 (premult 0.30/0.50) over backdrop
    /// Cb = 0.5 at αb = 0.8 (premult 0.40/0.80).
    ///
    /// Common derivation (hand-computed from the §4.4 formula):
    ///   Co = 0.5·(1−0.8)·0.6 + 0.5·0.8·B + (1−0.5)·0.8·0.5
    ///      = 0.06 + 0.4·B + 0.20 = 0.26 + 0.4·B
    ///   αo = 0.5 + 0.8·(1−0.5) = 0.90
    ///
    /// Per-mode B (W3C formulas, §4.5) and the resulting premultiplied Co:
    ///   Normal:      B = s               = 0.6                → Co = 0.50
    ///   Darken:      B = min(0.5,0.6)    = 0.5                → Co = 0.46
    ///   Multiply:    B = 0.5·0.6         = 0.30               → Co = 0.38
    ///   Lighten:     B = max(0.5,0.6)    = 0.6                → Co = 0.50
    ///   Screen:      B = 0.5+0.6−0.30    = 0.8                → Co = 0.58
    ///   LinearDodge: B = min(1, 1.1)     = 1.0                → Co = 0.66
    ///   Overlay:     b=0.5 ≤ 0.5 → B = 2·0.6·0.5 = 0.6        → Co = 0.50
    ///   Difference:  B = |0.5−0.6|       = 0.1                → Co = 0.30
    ///
    /// Normal (0.50/0.90) and Multiply (0.38/0.90) are the §4.4 worked
    /// example's own numbers — the spec-mandated golden-suite unit test.
    #[test]
    fn oracle_table_worked_example_pair() {
        let src = [0.30, 0.30, 0.30, 0.50]; // premult: 0.6 × 0.5
        let dst = [0.40, 0.40, 0.40, 0.80]; // premult: 0.5 × 0.8
        let cases: [(BlendMode, f32); 8] = [
            (BlendMode::Normal, 0.50),
            (BlendMode::Darken, 0.46),
            (BlendMode::Multiply, 0.38),
            (BlendMode::Lighten, 0.50),
            (BlendMode::Screen, 0.58),
            (BlendMode::LinearDodge, 0.66),
            (BlendMode::Overlay, 0.50),
            (BlendMode::Difference, 0.30),
        ];
        for (mode, want) in cases {
            let got = run(mode, src, dst, 1.0);
            assert_px(got, want, 0.90);
        }
    }

    /// Channels blend independently (separable modes, §4.5): distinct
    /// per-channel inputs produce the per-channel formula outputs.
    ///
    /// Multiply, opaque over opaque: Cs = (0.5, 0.25, 1.0),
    /// Cb = (0.5, 1.0, 0.25) → B = (0.25, 0.25, 0.25); with αs = αb = 1
    /// the §4.4 formula reduces to Co = B.
    #[test]
    fn channels_are_independent() {
        let got = run(
            BlendMode::Multiply,
            [0.5, 0.25, 1.0, 1.0],
            [0.5, 1.0, 0.25, 1.0],
            1.0,
        );
        assert!((got[0] - 0.25).abs() <= EPS);
        assert!((got[1] - 0.25).abs() <= EPS);
        assert!((got[2] - 0.25).abs() <= EPS);
        assert_eq!(got[3], 1.0);
    }

    /// Overlay's upper branch (b > 0.5): b = 0.75, s = 0.6 →
    /// B = 1 − 2·(1−0.6)·(1−0.75) = 1 − 2·0.4·0.25 = 0.8; opaque/opaque so
    /// Co = B.
    #[test]
    fn overlay_upper_branch() {
        let got = run(
            BlendMode::Overlay,
            [0.6, 0.6, 0.6, 1.0],
            [0.75, 0.75, 0.75, 1.0],
            1.0,
        );
        assert_px(got, 0.8, 1.0);
    }

    /// Overlay branches on the BACKDROP (it is HardLight with swapped
    /// arguments): at b = 0.5 exactly, the ≤ branch applies →
    /// B = 2·s·b = s. Derivation: b_overlay(0.5, 0.9) = 2·0.9·0.5 = 0.9.
    #[test]
    fn overlay_branches_on_backdrop_at_half() {
        let got = run(
            BlendMode::Overlay,
            [0.9, 0.9, 0.9, 1.0],
            [0.5, 0.5, 0.5, 1.0],
            1.0,
        );
        assert_px(got, 0.9, 1.0);
    }

    /// Linear Dodge clamps by formula: 0.75 + 0.5 = 1.25 → min(1, ·) = 1.
    #[test]
    fn linear_dodge_clamps_at_one() {
        let got = run(
            BlendMode::LinearDodge,
            [0.5, 0.5, 0.5, 1.0],
            [0.75, 0.75, 0.75, 1.0],
            1.0,
        );
        assert_px(got, 1.0, 1.0);
    }

    /// Difference is symmetric in magnitude: |0.25 − 0.6| = 0.35 both ways.
    #[test]
    fn difference_is_magnitude() {
        let a = run(
            BlendMode::Difference,
            [0.6, 0.6, 0.6, 1.0],
            [0.25, 0.25, 0.25, 1.0],
            1.0,
        );
        let b = run(
            BlendMode::Difference,
            [0.25, 0.25, 0.25, 1.0],
            [0.6, 0.6, 0.6, 1.0],
            1.0,
        );
        assert_px(a, 0.35, 1.0);
        assert_px(b, 0.35, 1.0);
    }

    /// HDR pins (§6.4 + module-doc domain note): formulas verbatim on f32
    /// values above 1, no input clamp — the behavior the CP-0.5 GPU path
    /// must reproduce.
    ///
    ///   Normal:      s = 3.5 over transparent → Co = αs·Cs = 3.5 (αb = 0)
    ///   Multiply:    2.0 · 1.5 = 3.0
    ///   Screen:      2 + 2 − 2·2 = 0            (formula-verbatim extension)
    ///   Darken:      min(2, 1.5) = 1.5
    ///   LinearDodge: min(1, 2+2) = 1            (formula's own clamp)
    ///   Difference:  |2 − 3.5| = 1.5
    #[test]
    fn hdr_values_use_formulas_verbatim() {
        // Opaque HDR source over transparent backdrop: pass-through.
        let got = run(BlendMode::Normal, [3.5, 3.5, 3.5, 1.0], [0.0; 4], 1.0);
        assert_px(got, 3.5, 1.0);

        let got = run(
            BlendMode::Multiply,
            [1.5, 1.5, 1.5, 1.0],
            [2.0, 2.0, 2.0, 1.0],
            1.0,
        );
        assert_px(got, 3.0, 1.0);

        let got = run(
            BlendMode::Screen,
            [2.0, 2.0, 2.0, 1.0],
            [2.0, 2.0, 2.0, 1.0],
            1.0,
        );
        assert_px(got, 0.0, 1.0);

        let got = run(
            BlendMode::Darken,
            [1.5, 1.5, 1.5, 1.0],
            [2.0, 2.0, 2.0, 1.0],
            1.0,
        );
        assert_px(got, 1.5, 1.0);

        let got = run(
            BlendMode::LinearDodge,
            [2.0, 2.0, 2.0, 1.0],
            [2.0, 2.0, 2.0, 1.0],
            1.0,
        );
        assert_px(got, 1.0, 1.0);

        let got = run(
            BlendMode::Difference,
            [3.5, 3.5, 3.5, 1.0],
            [2.0, 2.0, 2.0, 1.0],
            1.0,
        );
        assert_px(got, 1.5, 1.0);
    }

    /// Premultiplied edge case a = 0 (source): §4.4 with αs = 0 —
    /// Co = αb·Cb, αo = αb, for EVERY mode (identity). Also pins the §4.5
    /// un-premultiply guard (0/0 → 0, no NaN).
    #[test]
    fn zero_alpha_source_is_identity_for_all_v1_modes() {
        let dst = [0.40, 0.10, 0.20, 0.80];
        for mode in BLEND_SET_V1 {
            let got = run(mode, [0.0; 4], dst, 1.0);
            assert_eq!(
                got, dst,
                "{mode:?}: αs = 0 must leave the backdrop untouched"
            );
            assert!(got.iter().all(|v| v.is_finite()));
        }
    }

    /// Premultiplied edge case a = 0 (backdrop): §4.4 with αb = 0 —
    /// Co = αs·Cs (premultiplied source scaled by opacity), αo = αs.
    #[test]
    fn zero_alpha_backdrop_takes_scaled_source() {
        for mode in BLEND_SET_V1 {
            let got = run(mode, [0.30, 0.30, 0.30, 0.50], [0.0; 4], 1.0);
            assert_px(got, 0.30, 0.50);
            let got = run(mode, [0.30, 0.30, 0.30, 0.50], [0.0; 4], 0.5);
            // αs = 0.25; Co = αs·Cs = 0.25·0.6 = 0.15.
            assert_px(got, 0.15, 0.25);
        }
    }

    /// §34.4 property 3: Normal with αs = 1 over an opaque backdrop is
    /// exact source replacement.
    #[test]
    fn opaque_normal_is_source_replacement() {
        let got = run(
            BlendMode::Normal,
            [0.123, 0.456, 0.789, 1.0],
            [0.9, 0.8, 0.7, 1.0],
            1.0,
        );
        assert_eq!(got, [0.123, 0.456, 0.789, 1.0]);
    }

    /// §34.4 property 4: layer opacity is linear for Normal —
    /// composite(opacity k) == lerp(backdrop, composite(opacity 1), k).
    /// Fixed-seed pseudo-random pixels (SplitMix64).
    #[test]
    fn normal_opacity_is_linear_lerp() {
        let mut state = 0xB1E55EDu64;
        let mut next = move || {
            state = state.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            z ^ (z >> 31)
        };
        let mut unit = move || (next() % 4096) as f32 / 4096.0;
        for _ in 0..200 {
            let a_src = unit();
            let a_dst = unit();
            let src = [unit() * a_src, unit() * a_src, unit() * a_src, a_src];
            let dst = [unit() * a_dst, unit() * a_dst, unit() * a_dst, a_dst];
            let k = unit();
            let at_k = run(BlendMode::Normal, src, dst, k);
            let at_1 = run(BlendMode::Normal, src, dst, 1.0);
            for c in 0..4 {
                let lerp = dst[c] + (at_1[c] - dst[c]) * k;
                assert!(
                    (at_k[c] - lerp).abs() <= 1e-5,
                    "c={c}: {} vs lerp {}",
                    at_k[c],
                    lerp
                );
            }
        }
    }

    /// The kernel result equals the un-premultiplied §4.4 formula evaluated
    /// independently (fractional alphas, all 8 modes) — guards the
    /// premultiply/un-premultiply plumbing around the blend function.
    /// Un-premultiplied scalar blend function `B(Cb, Cs)`.
    type ScalarBlendFn = fn(f32, f32) -> f32;

    #[test]
    fn kernel_matches_independent_scalar_porter_duff() {
        let scalar_b: [(BlendMode, ScalarBlendFn); 8] = [
            (BlendMode::Normal, |_b, s| s),
            (BlendMode::Darken, |b, s| b.min(s)),
            (BlendMode::Multiply, |b, s| b * s),
            (BlendMode::Lighten, |b, s| b.max(s)),
            (BlendMode::Screen, |b, s| b + s - b * s),
            (BlendMode::LinearDodge, |b, s| (b + s).min(1.0)),
            (BlendMode::Overlay, |b, s| {
                if b <= 0.5 {
                    2.0 * s * b
                } else {
                    1.0 - 2.0 * (1.0 - s) * (1.0 - b)
                }
            }),
            (BlendMode::Difference, |b, s| (b - s).abs()),
        ];
        // Dyadic inputs keep the premult↔straight conversions exact so the
        // two evaluation orders agree bit-for-bit at f32.
        let (cs, a_s_stored, cb, a_b) = (0.75f32, 0.5f32, 0.25f32, 0.25f32);
        let opacity = 0.5f32;
        for (mode, bf) in scalar_b {
            let got = run(
                mode,
                [
                    cs * a_s_stored,
                    cs * a_s_stored,
                    cs * a_s_stored,
                    a_s_stored,
                ],
                [cb * a_b, cb * a_b, cb * a_b, a_b],
                opacity,
            );
            let (co, ao) = pd(cs, a_s_stored * opacity, cb, a_b, bf(cb, cs));
            assert_px(got, co, ao);
        }
    }

    /// CP-0.5 slice A: the full 27-mode catalog composites through the
    /// context-free door; the kept `Result` is always `Ok`, outputs are
    /// finite, and every routing is exercised (never silent wrong math —
    /// now there is no unimplemented mode left to refuse).
    #[test]
    fn all_27_modes_composite_finite() {
        let src = TileRgba::splat([0.1, 0.1, 0.1, 0.5]);
        let mut n = 0;
        for mode in BlendMode::ALL {
            let mut dst = TileRgba::splat([0.2, 0.2, 0.2, 0.5]);
            composite_tile(&mut dst, &src, mode, 1.0)
                .unwrap_or_else(|e| panic!("{mode:?}: full catalog never errs: {e:?}"));
            assert!(blend_implemented(mode));
            assert!(
                dst.get(7, 7).iter().all(|v| v.is_finite()),
                "{mode:?}: non-finite"
            );
            n += 1;
        }
        assert_eq!(n, 27, "the whole §4.5 catalog");
        let _ = CompositeError::BlendModeNotImplemented {
            mode: BlendMode::Normal,
            layer: None,
        }; // variant kept compilable for the CP-0.3 conformance surface
    }

    /// MINOR-5 bit-equality: the uniform-source kernel equals
    /// splat-then-materialized-kernel to the bit, per pixel, across all
    /// 8 modes × source grid (a = 0 / fractional / 1 / HDR>1, dyadic and
    /// non-dyadic) × backdrops (transparent / fractional / opaque) ×
    /// opacities. This is the license for the compositor's Uniform fast
    /// path to skip materialization.
    #[test]
    fn uniform_src_kernel_is_bitequal_to_materialized() {
        let sources: [[f32; 4]; 6] = [
            [0.0, 0.0, 0.0, 0.0],     // a = 0 (identity)
            [0.30, 0.30, 0.30, 0.50], // the §4.4 worked pair source
            [0.5, 0.25, 0.125, 0.5],  // dyadic fractional
            [0.1, 0.7, 0.3, 0.9],     // non-dyadic fractional
            [1.0, 1.0, 1.0, 1.0],     // opaque white
            [3.5, 0.0, 1.25, 0.5],    // HDR super-bright premult (§6.4)
        ];
        let backdrops: [[f32; 4]; 4] = [
            [0.0, 0.0, 0.0, 0.0],
            [0.40, 0.40, 0.40, 0.80], // the §4.4 worked pair backdrop
            [0.2, 0.9, 0.05, 0.95],
            [0.0, 0.0, 0.0, 1.0],
        ];
        for mode in BLEND_SET_V1 {
            let f = separable_fn(mode).unwrap();
            for src in sources {
                for dst0 in backdrops {
                    for opacity in [0.25f32, 1.0] {
                        let mut via_materialized = TileRgba::splat(dst0);
                        let src_tile = TileRgba::splat(src);
                        composite_tile_with(&mut via_materialized, &src_tile, f, opacity);
                        let mut via_uniform = TileRgba::splat(dst0);
                        composite_tile_uniform_src(&mut via_uniform, src, f, opacity);
                        let (a, b) = (via_materialized.get(13, 200), via_uniform.get(13, 200));
                        for c in 0..4 {
                            assert_eq!(
                                a[c].to_bits(),
                                b[c].to_bits(),
                                "{mode:?} src {src:?} dst {dst0:?} op {opacity}: ch {c} {} vs {}",
                                a[c],
                                b[c]
                            );
                        }
                        // Whole-tile equality, not just the probe pixel.
                        assert!(
                            via_materialized == via_uniform,
                            "{mode:?}: full-tile mismatch"
                        );
                    }
                }
            }
        }
    }

    /// The full-catalog oracle table on the ES §4.4 worked-example pixel
    /// pair (source Cs = 0.6 at αs = 0.5 over backdrop Cb = 0.5 at
    /// αb = 0.8): every coordinate-free mode's `B`, hand-derived from the
    /// Appendix A formula text, wrapped by Co = 0.26 + 0.4·B, αo = 0.90.
    ///
    /// Per-mode B (f64 hand derivations in-line):
    ///   ColorBurn:    1 − min(1, 0.5/0.6)          = 1/6
    ///   LinearBurn:   max(0, 1.1 − 1)              = 0.1
    ///   ColorDodge:   min(1, 0.5/0.4)              = 1
    ///   SoftLight:    0.5 + 0.2·(√0.5 − 0.5)       = 0.5414213562…
    ///   HardLight:    Screen(0.5, 0.2)             = 0.6
    ///   VividLight:   ColorDodge(0.5, 0.2)         = 0.625   (Appendix A)
    ///   LinearLight:  clamp(0.5 + 1.2 − 1)         = 0.7
    ///   PinLight:     max(0.5, 0.2)                = 0.5
    ///   HardMix:      1.1 ≥ 1                      = 1
    ///   Exclusion:    1.1 − 2·0.3                  = 0.5
    ///   Subtract:     max(0, −0.1)                 = 0
    ///   Divide:       min(1, 0.5/0.6)              = 5/6
    ///   DarkerColor:  Lum 0.5 < 0.6 → backdrop     = 0.5
    ///   LighterColor: Lum 0.6 > 0.5 → source       = 0.6
    ///   Hue:          gray source → SetSat = 0 → SetLum → Lum(Cb) = 0.5
    ///   Saturation:   Sat(gray) = 0 → 0-triple → SetLum → 0.5
    ///   Color:        SetLum(Cs, Lum(Cb))          = 0.5
    ///   Luminosity:   SetLum(Cb, Lum(Cs))          = 0.6
    #[test]
    fn oracle_table_full_catalog_worked_example_pair() {
        let src = [0.30, 0.30, 0.30, 0.50];
        let dst = [0.40, 0.40, 0.40, 0.80];
        let cases: [(BlendMode, f64); 18] = [
            (BlendMode::ColorBurn, 1.0 - 0.5 / 0.6),
            (BlendMode::LinearBurn, 0.1),
            (BlendMode::ColorDodge, 1.0),
            (BlendMode::SoftLight, 0.5 + 0.2 * (0.5f64.sqrt() - 0.5)),
            (BlendMode::HardLight, 0.6),
            (BlendMode::VividLight, 0.625),
            (BlendMode::LinearLight, 0.7),
            (BlendMode::PinLight, 0.5),
            (BlendMode::HardMix, 1.0),
            (BlendMode::Exclusion, 0.5),
            (BlendMode::Subtract, 0.0),
            (BlendMode::Divide, 0.5 / 0.6),
            (BlendMode::DarkerColor, 0.5),
            (BlendMode::LighterColor, 0.6),
            (BlendMode::Hue, 0.5),
            (BlendMode::Saturation, 0.5),
            (BlendMode::Color, 0.5),
            (BlendMode::Luminosity, 0.6),
        ];
        for (mode, b) in cases {
            let got = run(mode, src, dst, 1.0);
            let want = (0.26 + 0.4 * b) as f32;
            for (c, &g) in got.iter().enumerate().take(3) {
                assert!(
                    (g - want).abs() <= 2e-6,
                    "{mode:?} ch{c}: got {g} want {want}"
                );
            }
            assert!((got[3] - 0.90).abs() <= EPS, "{mode:?} alpha: {}", got[3]);
        }
    }

    /// Branch pins for the new separable formulas (opaque over opaque, so
    /// Co = B): every `if` in the Appendix A formula text is exercised on
    /// both sides, with f64 hand values in-line.
    #[test]
    fn separable_branch_pins() {
        let cases: [(BlendMode, f32, f32, f64); 16] = [
            // (mode, backdrop b, source s, expected B)
            (BlendMode::ColorBurn, 0.3, 0.0, 0.0),   // s==0 arm
            (BlendMode::ColorBurn, 0.9, 0.5, 0.8),   // 1−0.1/0.5
            (BlendMode::ColorBurn, 0.2, 0.5, 0.0),   // min clamps: 0.8/0.5>1
            (BlendMode::ColorDodge, 0.5, 1.0, 1.0),  // s==1 arm
            (BlendMode::ColorDodge, 0.25, 0.5, 0.5), // 0.25/0.5
            (BlendMode::SoftLight, 0.5, 0.25, 0.375), // s≤0.5 arm
            (BlendMode::SoftLight, 0.16, 0.75, 0.279168), // D's b≤0.25 arm
            (BlendMode::VividLight, 0.5, 0.25, 0.0), // burn arm: ColorBurn(0.5,0.5)
            (BlendMode::VividLight, 0.5, 0.0, 0.0),  // burn arm, inner s==0
            (BlendMode::VividLight, 0.5, 1.0, 1.0),  // dodge arm, inner s==1
            (BlendMode::PinLight, 0.3, 0.25, 0.3),   // min arm: min(0.3, 0.5)
            (BlendMode::HardMix, 0.5, 0.5, 1.0),     // boundary: b+s == 1 → 1
            (BlendMode::HardMix, 0.25, 0.5, 0.0),    // below threshold
            (BlendMode::LinearLight, 0.9, 0.9, 1.0), // upper clamp (1.7)
            (BlendMode::LinearLight, 0.1, 0.2, 0.0), // lower clamp (−0.5)
            (BlendMode::Divide, 0.3, 0.0, 1.0),      // s==0 arm
        ];
        for (mode, b, s, want) in cases {
            let got = run(mode, [s, s, s, 1.0], [b, b, b, 1.0], 1.0);
            assert_px(got, want as f32, 1.0);
        }
        // Divide clamp + plain arms.
        assert_px(
            run(
                BlendMode::Divide,
                [0.4, 0.4, 0.4, 1.0],
                [0.8, 0.8, 0.8, 1.0],
                1.0,
            ),
            1.0,
            1.0,
        );
        assert_px(
            run(
                BlendMode::Divide,
                [0.6, 0.6, 0.6, 1.0],
                [0.3, 0.3, 0.3, 1.0],
                1.0,
            ),
            0.5,
            1.0,
        );
    }

    /// HDR pins for the new modes (module-doc domain note): formulas
    /// verbatim on f32 values above 1 — the exact behavior the slice-D
    /// GPU path must reproduce (its WGSL runs the same expressions).
    ///
    ///   Subtract:  max(0, 3.5 − 2)                        = 1.5
    ///   ColorBurn: b=2, s=0.5 → 1 − min(1, (1−2)/0.5)     = 1 − (−2) = 3
    ///   HardMix:   b=2, s=0 → 2 ≥ 1                       = 1
    ///   LinearBurn: b=2, s=0.5 → max(0, 1.5)              = 1.5
    #[test]
    fn hdr_pins_for_new_modes() {
        let opaque = |v: f32| [v, v, v, 1.0f32];
        assert_px(
            run(BlendMode::Subtract, opaque(2.0), opaque(3.5), 1.0),
            1.5,
            1.0,
        );
        assert_px(
            run(BlendMode::ColorBurn, opaque(0.5), opaque(2.0), 1.0),
            3.0,
            1.0,
        );
        assert_px(
            run(BlendMode::HardMix, opaque(0.0), opaque(2.0), 1.0),
            1.0,
            1.0,
        );
        assert_px(
            run(BlendMode::LinearBurn, opaque(0.5), opaque(2.0), 1.0),
            1.5,
            1.0,
        );
    }

    /// Whole-pixel modes pick ONE input pixel — not per-channel extrema
    /// (the §4.5 "whole-pixel picks" routing). Cb = (0.9, 0.1, 0.1),
    /// Lum = 0.34; Cs = (0.1, 0.5, 0.1), Lum = 0.336: DarkerColor takes
    /// the source triple whole; LighterColor the backdrop triple whole. A
    /// per-channel min/max would produce (0.1, 0.1, 0.1)/(0.9, 0.5, 0.1)
    /// — this test distinguishes.
    #[test]
    fn whole_pixel_modes_pick_one_pixel() {
        let src = [0.1, 0.5, 0.1, 1.0];
        let dst = [0.9, 0.1, 0.1, 1.0];
        let d = run(BlendMode::DarkerColor, src, dst, 1.0);
        assert!(
            (d[0] - 0.1).abs() <= EPS && (d[1] - 0.5).abs() <= EPS && (d[2] - 0.1).abs() <= EPS
        );
        let l = run(BlendMode::LighterColor, src, dst, 1.0);
        assert!(
            (l[0] - 0.9).abs() <= EPS && (l[1] - 0.1).abs() <= EPS && (l[2] - 0.1).abs() <= EPS
        );
    }

    /// [NORMATIVE §4.5] Ties resolve to the BACKDROP for both whole-pixel
    /// modes. Constructs a bitwise f32 luminance tie from two DISTINCT
    /// triples: (x, 0, 0) and (0, y, 0) with fl(0.3·x) == fl(0.59·y) ==
    /// 0.25 exactly, found by a bounded deterministic ulp search around
    /// the real-valued solutions (adding 0.0 terms is exact, so lum
    /// reduces to the single product).
    #[test]
    fn whole_pixel_tie_resolves_to_backdrop() {
        let find = |coef: f32, target: f32, guess: f32| -> f32 {
            let mut lo = guess;
            for _ in 0..256 {
                if coef * lo == target {
                    return lo;
                }
                lo = f32::from_bits(lo.to_bits() - 1);
            }
            let mut hi = guess;
            for _ in 0..256 {
                if coef * hi == target {
                    return hi;
                }
                hi = f32::from_bits(hi.to_bits() + 1);
            }
            panic!("no x with fl({coef}·x) == {target} within ±256 ulp of {guess}");
        };
        let x = find(0.3, 0.25, 0.25 / 0.3);
        let y = find(0.59, 0.25, 0.25 / 0.59);
        let cb = [x, 0.0, 0.0];
        let cs = [0.0, y, 0.0];
        assert_eq!(lum(cb).to_bits(), lum(cs).to_bits(), "constructed tie");
        assert_ne!(cb, cs);
        // Backdrop wins both modes on a tie.
        let d = run(
            BlendMode::DarkerColor,
            [0.0, y, 0.0, 1.0],
            [x, 0.0, 0.0, 1.0],
            1.0,
        );
        assert!(
            (d[0] - x).abs() <= EPS && d[1] <= EPS && d[2] <= EPS,
            "darker tie → backdrop"
        );
        let l = run(
            BlendMode::LighterColor,
            [0.0, y, 0.0, 1.0],
            [x, 0.0, 0.0, 1.0],
            1.0,
        );
        assert!(
            (l[0] - x).abs() <= EPS && l[1] <= EPS && l[2] <= EPS,
            "lighter tie → backdrop"
        );
    }

    /// Hand-computed non-separable pin (Appendix A helpers, f64 by hand):
    /// Hue with Cb = (0.25, 0.5, 0.75), Cs = (0.9, 0.3, 0.1), opaque:
    ///   Sat(Cb) = 0.5; SetSat(Cs, 0.5) = (0.5, 0.125, 0)
    ///   Lum(Cb) = 0.075 + 0.295 + 0.0825 = 0.4525
    ///   Lum(0.5, 0.125, 0) = 0.15 + 0.07375 = 0.22375; d = 0.22875
    ///   → (0.72875, 0.35375, 0.22875), ClipColor no-op (in gamut).
    #[test]
    fn hue_hand_computed_pin() {
        let got = run(
            BlendMode::Hue,
            [0.9, 0.3, 0.1, 1.0],
            [0.25, 0.5, 0.75, 1.0],
            1.0,
        );
        let want = [0.72875f32, 0.35375, 0.22875];
        for c in 0..3 {
            assert!(
                (got[c] - want[c]).abs() <= 1e-5,
                "ch{c}: {} vs {}",
                got[c],
                want[c]
            );
        }
        assert_eq!(got[3], 1.0);
    }

    /// ClipColor's two branches engage and preserve luminance (Appendix A):
    ///   x>1: Color, Cs = (1,1,0) over gray 0.9 → SetLum d = +0.01 →
    ///        (1.01, 1.01, 0.01), clipped to (1, 1, 0.0909090…).
    ///   n<0: Color, Cs = (0,0,1) over gray 0.05 → d = −0.06 →
    ///        (−0.06, −0.06, 0.94), clipped to (0, 0, 0.4545454…).
    #[test]
    fn clip_color_branches_engage_and_preserve_lum() {
        let hi = run(
            BlendMode::Color,
            [1.0, 1.0, 0.0, 1.0],
            [0.9, 0.9, 0.9, 1.0],
            1.0,
        );
        let want_hi = [1.0f32, 1.0, 0.090_909_09];
        for c in 0..3 {
            assert!((hi[c] - want_hi[c]).abs() <= 1e-5, "x>1 ch{c}: {}", hi[c]);
        }
        assert!(
            (lum([hi[0], hi[1], hi[2]]) - 0.9).abs() <= 1e-5,
            "SetLum preserves Lum"
        );

        let lo = run(
            BlendMode::Color,
            [0.0, 0.0, 1.0, 1.0],
            [0.05, 0.05, 0.05, 1.0],
            1.0,
        );
        let want_lo = [0.0f32, 0.0, 0.454_545_47];
        for c in 0..3 {
            assert!((lo[c] - want_lo[c]).abs() <= 1e-5, "n<0 ch{c}: {}", lo[c]);
        }
        assert!(
            (lum([lo[0], lo[1], lo[2]]) - 0.05).abs() <= 1e-5,
            "SetLum preserves Lum"
        );
    }

    /// The WGSL-vectorized `SetSat` (implemented) ≡ Appendix A's
    /// sorted-channel form (independent test-local implementation),
    /// including the tie cases (x,x,y), (x,y,y), (x,x,x), across a
    /// deterministic grid. Tolerance 1e-6: the max channel computes
    /// ((cmax−cmin)·s)/(cmax−cmin) which is s to ≤ 2 ulp, not bitwise.
    #[test]
    fn set_sat_vectorized_equals_appendix_a_sorted_form() {
        // Appendix A sorted form, channel identity preserved.
        fn sorted_form(c: [f32; 3], s: f32) -> [f32; 3] {
            let mut order = [0usize, 1, 2];
            order.sort_by(|&a, &b| c[a].partial_cmp(&c[b]).unwrap());
            let (imin, imid, imax) = (order[0], order[1], order[2]);
            let mut out = [0.0f32; 3];
            if c[imax] > c[imin] {
                out[imid] = (c[imid] - c[imin]) * s / (c[imax] - c[imin]);
                out[imax] = s;
            }
            out
        }
        let grid = [0.0f32, 0.1, 0.25, 0.4, 0.5, 0.75, 0.9, 1.0];
        for &r in &grid {
            for &g in &grid {
                for &b in &grid {
                    for &s in &[0.0f32, 0.3, 0.5, 1.0] {
                        let v = set_sat([r, g, b], s);
                        let w = sorted_form([r, g, b], s);
                        for c in 0..3 {
                            assert!(
                                (v[c] - w[c]).abs() <= 1e-6,
                                "set_sat(({r},{g},{b}), {s}) ch{c}: {} vs {}",
                                v[c],
                                w[c]
                            );
                        }
                    }
                }
            }
        }
    }

    /// The four non-separable modes match an INDEPENDENT f64
    /// re-implementation of the Appendix A helper text (sorted-form
    /// SetSat) over a seeded pseudo-random grid — guards transcription
    /// errors in the production helpers.
    #[test]
    fn nonseparable_matches_independent_f64_reference() {
        fn lum64(c: [f64; 3]) -> f64 {
            0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
        }
        fn clip64(mut c: [f64; 3]) -> [f64; 3] {
            let l = lum64(c);
            let n = c[0].min(c[1]).min(c[2]);
            let x = c[0].max(c[1]).max(c[2]);
            if n < 0.0 {
                for ch in &mut c {
                    *ch = l + (*ch - l) * l / (l - n);
                }
            }
            if x > 1.0 {
                for ch in &mut c {
                    *ch = l + (*ch - l) * (1.0 - l) / (x - l);
                }
            }
            c
        }
        fn set_lum64(c: [f64; 3], l: f64) -> [f64; 3] {
            let d = l - lum64(c);
            clip64([c[0] + d, c[1] + d, c[2] + d])
        }
        fn set_sat64(c: [f64; 3], s: f64) -> [f64; 3] {
            let mut order = [0usize, 1, 2];
            order.sort_by(|&a, &b| c[a].partial_cmp(&c[b]).unwrap());
            let (imin, imid, imax) = (order[0], order[1], order[2]);
            let mut out = [0.0f64; 3];
            if c[imax] > c[imin] {
                out[imid] = (c[imid] - c[imin]) * s / (c[imax] - c[imin]);
                out[imax] = s;
            }
            out
        }
        fn sat64(c: [f64; 3]) -> f64 {
            c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
        }
        let reference = |mode: BlendMode, cb: [f64; 3], cs: [f64; 3]| -> [f64; 3] {
            match mode {
                BlendMode::Hue => set_lum64(set_sat64(cs, sat64(cb)), lum64(cb)),
                BlendMode::Saturation => set_lum64(set_sat64(cb, sat64(cs)), lum64(cb)),
                BlendMode::Color => set_lum64(cs, lum64(cb)),
                BlendMode::Luminosity => set_lum64(cb, lum64(cs)),
                _ => unreachable!(),
            }
        };
        let mut state = 0x4E53455Bu64; // deterministic
        let mut next = move || {
            state = state.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            z ^ (z >> 31)
        };
        let mut unit = move || (next() % 4096) as f32 / 4096.0;
        for _ in 0..250 {
            let cb = [unit(), unit(), unit()];
            let cs = [unit(), unit(), unit()];
            for mode in [
                BlendMode::Hue,
                BlendMode::Saturation,
                BlendMode::Color,
                BlendMode::Luminosity,
            ] {
                let got = run(
                    mode,
                    [cs[0], cs[1], cs[2], 1.0],
                    [cb[0], cb[1], cb[2], 1.0],
                    1.0,
                );
                let want = reference(
                    mode,
                    [cb[0] as f64, cb[1] as f64, cb[2] as f64],
                    [cs[0] as f64, cs[1] as f64, cs[2] as f64],
                );
                for c in 0..3 {
                    assert!(
                        (got[c] as f64 - want[c]).abs() <= 1e-5,
                        "{mode:?} cb {cb:?} cs {cs:?} ch{c}: {} vs {}",
                        got[c],
                        want[c]
                    );
                }
            }
        }
    }

    /// Dissolve coverage is EXACT for dyadic α_eff: every threshold
    /// `(v + 0.5)/256` is a dyadic f32 (computed exactly), the 256×256
    /// tile holds exactly 16 copies of the 64×64 table, and each byte
    /// value appears 16× per copy — so `α_eff = k/4` passes exactly
    /// `k/4 · 65536` pixels. Over a transparent backdrop the output alpha
    /// IS `αs` (αo = αs + 0·(1−αs)), directly countable.
    #[test]
    fn dissolve_coverage_is_exact_for_dyadic_alphas() {
        for (a_eff, want_pass) in [(1.0f32, 65536usize), (0.5, 32768), (0.25, 16384), (0.0, 0)] {
            let src = TileRgba::splat([0.5 * a_eff, 0.5 * a_eff, 0.5 * a_eff, a_eff]);
            let mut dst = TileRgba::transparent();
            composite_tile(&mut dst, &src, BlendMode::Dissolve, 1.0).unwrap();
            let passed = dst
                .as_slice()
                .chunks_exact(4)
                .filter(|px| px[3] == 1.0)
                .count();
            assert_eq!(passed, want_pass, "α_eff = {a_eff}");
            // And nothing in between: alpha is exactly 0 or 1.
            assert!(dst
                .as_slice()
                .chunks_exact(4)
                .all(|px| px[3] == 0.0 || px[3] == 1.0));
        }
    }

    /// Dissolve semantics per pixel (§4.5): over an OPAQUE backdrop,
    /// passing pixels replace with the un-premultiplied source color at
    /// α = 1; failing pixels are the bitwise backdrop (αb = 1 makes the
    /// αs = 0 expression exact: cb = p/1, ·1). Deterministic: two runs
    /// are bit-identical.
    #[test]
    fn dissolve_passes_replace_fails_keep_backdrop() {
        let src = TileRgba::splat([0.30, 0.30, 0.30, 0.50]); // Cs = 0.6
        let backdrop = TileRgba::splat([0.8, 0.8, 0.8, 1.0]);
        let mut a = backdrop.clone();
        composite_tile(&mut a, &src, BlendMode::Dissolve, 1.0).unwrap();
        let mut b = backdrop.clone();
        composite_tile(&mut b, &src, BlendMode::Dissolve, 1.0).unwrap();
        assert!(a == b, "bit-identical across runs");
        let cs = 0.30f32 / 0.50; // the kernel's own un-premultiply
        let mut passed = 0;
        for (i, px) in a.as_slice().chunks_exact(4).enumerate() {
            let (x, y) = ((i % 256) as i32, (i / 256) as i32);
            let n = bn_threshold(x, y, (0, 0));
            if 0.5 > n {
                assert_eq!(px[3], 1.0);
                for (c, &p) in px.iter().enumerate().take(3) {
                    assert!((p - cs).abs() <= EPS, "passing pixel ch{c}: {p}");
                }
                passed += 1;
            } else {
                assert_eq!(
                    px,
                    &[0.8, 0.8, 0.8, 1.0][..],
                    "failing pixel must be the bitwise backdrop (αb = 1)"
                );
            }
        }
        assert_eq!(passed, 32768);
    }

    /// Dissolve thresholds in DOCUMENT space: the same document pixel gets
    /// the same on/off decision regardless of which tile renders it —
    /// verified by compositing with `tile_origin = (256, 0)` and checking
    /// every pixel against the formula at its document coordinate; plus
    /// the layer offsets shift the pattern.
    #[test]
    fn dissolve_is_document_space_seamless_and_offset_aware() {
        let src = TileRgba::splat([0.25, 0.25, 0.25, 0.5]); // α_eff = 0.5
        let ctx = DissolveCtx {
            bn_off: (17, 42),
            tile_origin: (256, 0),
        };
        let mut dst = TileRgba::transparent();
        composite_tile_ctx(&mut dst, &src, BlendMode::Dissolve, 1.0, ctx);
        for (i, px) in dst.as_slice().chunks_exact(4).enumerate() {
            let (x, y) = ((i % 256) as i32, (i / 256) as i32);
            let n = bn_threshold(256 + x, y, (17, 42));
            let want_pass = 0.5 > n;
            assert_eq!(
                px[3] == 1.0,
                want_pass,
                "doc-space decision at local ({x},{y})"
            );
        }
        // A different offset pair changes the pattern (decorrelation).
        let mut other = TileRgba::transparent();
        composite_tile_ctx(
            &mut other,
            &src,
            BlendMode::Dissolve,
            1.0,
            DissolveCtx {
                bn_off: (0, 0),
                tile_origin: (256, 0),
            },
        );
        assert!(dst != other, "offsets must move the pattern");
    }

    /// MINOR-5 analog for the triple kernel: uniform-source path ≡
    /// splat-then-materialized path, to the bit, for whole-pixel and
    /// non-separable modes across the alpha/HDR/opacity grid.
    #[test]
    fn uniform_src_triple_kernel_is_bitequal_to_materialized() {
        let sources: [[f32; 4]; 5] = [
            [0.30, 0.30, 0.30, 0.50],
            [0.5, 0.25, 0.125, 0.5],
            [0.1, 0.7, 0.3, 0.9],
            [1.0, 1.0, 1.0, 1.0],
            [3.5, 0.0, 1.25, 0.5], // HDR super-bright premult (§6.4)
        ];
        let backdrops: [[f32; 4]; 4] = [
            [0.0, 0.0, 0.0, 0.0],
            [0.40, 0.40, 0.40, 0.80],
            [0.2, 0.9, 0.05, 0.95],
            [0.0, 0.0, 0.0, 1.0],
        ];
        for mode in [
            BlendMode::DarkerColor,
            BlendMode::LighterColor,
            BlendMode::Hue,
            BlendMode::Saturation,
            BlendMode::Color,
            BlendMode::Luminosity,
        ] {
            let f = match routing(mode) {
                BlendRouting::WholePixel(f) | BlendRouting::NonSeparable(f) => f,
                _ => unreachable!(),
            };
            for src in sources {
                for dst0 in backdrops {
                    for opacity in [0.25f32, 1.0] {
                        let mut via_materialized = TileRgba::splat(dst0);
                        composite_tile_triple(
                            &mut via_materialized,
                            &TileRgba::splat(src),
                            f,
                            opacity,
                        );
                        let mut via_uniform = TileRgba::splat(dst0);
                        composite_tile_uniform_src_triple(&mut via_uniform, src, f, opacity);
                        assert!(
                            via_materialized == via_uniform,
                            "{mode:?} src {src:?} dst {dst0:?} op {opacity}: mismatch"
                        );
                    }
                }
            }
        }
    }

    /// MINOR-5 analog for the Dissolve kernel: uniform-source path ≡
    /// splat-then-materialized path, to the bit (thresholds still
    /// evaluate per document-space pixel in both).
    #[test]
    fn uniform_src_dissolve_kernel_is_bitequal_to_materialized() {
        let ctx = DissolveCtx {
            bn_off: (5, 61),
            tile_origin: (-256, 512),
        };
        for src in [
            [0.30f32, 0.30, 0.30, 0.50],
            [0.1, 0.7, 0.3, 0.9],
            [1.0, 1.0, 1.0, 1.0],
            [3.5, 0.0, 1.25, 0.5],
        ] {
            for dst0 in [[0.0f32; 4], [0.40, 0.40, 0.40, 0.80], [0.0, 0.0, 0.0, 1.0]] {
                for opacity in [0.25f32, 1.0] {
                    let mut via_materialized = TileRgba::splat(dst0);
                    composite_tile_dissolve(
                        &mut via_materialized,
                        &TileRgba::splat(src),
                        opacity,
                        ctx,
                    );
                    let mut via_uniform = TileRgba::splat(dst0);
                    composite_tile_uniform_src_dissolve(&mut via_uniform, src, opacity, ctx);
                    assert!(
                        via_materialized == via_uniform,
                        "src {src:?} dst {dst0:?} op {opacity}: mismatch"
                    );
                }
            }
        }
    }

    /// The §14.4 injection seam partitions the catalog EXACTLY: 20
    /// separable + 6 triple + Dissolve (which exports nothing — the
    /// color side types it reserved). Every exported pointer IS the
    /// §4.4 compositor's own dispatch target (one-math-library clause:
    /// pointer identity, not behavioral coincidence).
    #[test]
    fn section_14_4_seam_partitions_and_aliases_the_routing_table() {
        let (mut sep, mut triple, mut dissolve) = (0, 0, 0);
        for mode in BlendMode::ALL {
            let s = render_separable_kernel(mode);
            let t3 = render_triple_kernel(mode);
            match (s, t3, mode) {
                (Some(f), None, m) => {
                    sep += 1;
                    match routing(m) {
                        BlendRouting::Separable(g) => {
                            assert!(std::ptr::fn_addr_eq(f, g), "{m:?}: seam must alias routing");
                        }
                        other => panic!("{m:?}: exported separable but routes {other:?}"),
                    }
                }
                (None, Some(f), m) => {
                    triple += 1;
                    match routing(m) {
                        BlendRouting::WholePixel(g) | BlendRouting::NonSeparable(g) => {
                            assert!(std::ptr::fn_addr_eq(f, g), "{m:?}: seam must alias routing");
                        }
                        other => panic!("{m:?}: exported triple but routes {other:?}"),
                    }
                }
                (None, None, BlendMode::Dissolve) => dissolve += 1,
                (s, t3, m) => panic!(
                    "{m:?}: broken partition (sep {}, triple {})",
                    s.is_some(),
                    t3.is_some()
                ),
            }
        }
        assert_eq!(
            (sep, triple, dissolve),
            (20, 6, 1),
            "the exact §4.5 partition"
        );
    }

    /// Behavior spot-pins THROUGH the exported pointers (the values the
    /// C4 door will compute): the §4.4 worked-pair B for Multiply; the
    /// ColorDodge s==1 arm + an argument-ORDER pin (flipped (b, s)
    /// would give 2/3, not 0.5); a DarkerColor smaller-Lum pick (the
    /// [NORMATIVE] tie→backdrop rule is pinned on the SAME pointer by
    /// `whole_pixel_tie_resolves_to_backdrop` — the partition test
    /// proves the seam aliases it); and the Hue hand pin.
    #[test]
    fn section_14_4_exported_kernels_compute_the_pinned_values() {
        let mul = render_separable_kernel(BlendMode::Multiply).unwrap();
        assert_eq!(mul(0.5, 0.6), 0.30000001192092896_f32 * 1.0); // 0.5·0.6 in f32
        assert!((mul(0.5, 0.6) - 0.3).abs() <= 1e-7);
        let dodge = render_separable_kernel(BlendMode::ColorDodge).unwrap();
        assert_eq!(dodge(0.5, 1.0), 1.0, "s == 1 arm");
        assert_eq!(dodge(0.25, 0.5), 0.5);
        let darker = render_triple_kernel(BlendMode::DarkerColor).unwrap();
        let (cb, cs) = ([0.9, 0.1, 0.1], [0.1, 0.5, 0.1]);
        assert_eq!(darker(cb, cs), cs, "smaller Lum wins whole-pixel");
        let hue = render_triple_kernel(BlendMode::Hue).unwrap();
        let got = hue([0.25, 0.5, 0.75], [0.9, 0.3, 0.1]);
        let want = [0.72875f32, 0.35375, 0.22875];
        for c in 0..3 {
            assert!((got[c] - want[c]).abs() <= 1e-5, "hue ch{c}: {}", got[c]);
        }
    }

    /// HDR-degenerate ClipColor pin (the `clip_color` doc note): a
    /// uniform gray triple beyond [0, 1] takes the formula's continuous
    /// limit instead of NaN — Luminosity of an HDR source over a
    /// transparent backdrop is the natural repro (cb guard yields the
    /// 0-triple; SetLum lifts it to uniform `Lum(Cs) > 1`). The composite
    /// must be finite, and the blended triple must be the 1-triple limit.
    /// The slice-D WGSL mirrors these guards exactly.
    #[test]
    fn hdr_degenerate_clip_color_pins_limit_values() {
        // Cs = (7, 0, 2.5) → Lum = 2.375 > 1; backdrop transparent.
        let got = run(
            BlendMode::Luminosity,
            [3.5, 0.0, 1.25, 0.5],
            [0.0, 0.0, 0.0, 0.0],
            1.0,
        );
        assert!(
            got.iter().all(|v| v.is_finite()),
            "no NaN into working tiles: {got:?}"
        );
        // αb = 0 ⇒ Co = αs·Cs — the blend result B never enters the
        // output where the backdrop is absent (§4.4). Force the B term
        // with an opaque backdrop instead:
        let forced = run(
            BlendMode::Luminosity,
            [3.5, 0.0, 1.25, 0.5],
            [0.5, 0.5, 0.5, 1.0], // gray backdrop, opaque
            1.0,
        );
        // B = clip_color(SetLum(gray 0.5 → 2.375)) = degenerate uniform
        // 2.375-triple → 1-triple. Co = 0.5·1 + 0.5·0.5 = 0.75 per
        // channel (αs = 0.5, αb = 1).
        for (c, &f) in forced.iter().enumerate().take(3) {
            assert!((f - 0.75).abs() <= 2e-6, "ch{c}: {f}");
        }
        // 0-triple side: Color mode pushes a positive triple to a
        // negative uniform target: Cs gray −? — negatives don't arise
        // from premultiplied storage; exercise the helper directly.
        assert_eq!(super::clip_color([-0.5, -0.5, -0.5]), [0.0, 0.0, 0.0]);
        assert_eq!(super::clip_color([2.0, 2.0, 2.0]), [1.0, 1.0, 1.0]);
    }

    /// Compositing is associative-free/order-dependent by design; pin one
    /// three-layer fold so stack semantics never drift: white 25% over
    /// red 50% over opaque black (all Normal).
    ///
    /// Derivation (§4.4, bottom-up; Normal has B = Cs, so over an opaque
    /// backdrop Co = αs·Cs + (1−αs)·Cb):
    ///   step 1: red (Cs=(1,0,0), αs=0.5) over black (Cb=0, αb=1):
    ///     Co = 0.5·Cs + 0.5·Cb = (0.5, 0, 0); αo = 1.
    ///   step 2: white (Cs=(1,1,1), αs=0.25) over (Cb=(0.5,0,0), αb=1):
    ///     Co = 0.25·Cs + 0.75·Cb = (0.625, 0.25, 0.25); αo = 1.
    #[test]
    fn three_layer_fold_hand_computed() {
        let mut acc = TileRgba::splat([0.0, 0.0, 0.0, 1.0]); // opaque black
        let red = TileRgba::splat([0.5, 0.0, 0.0, 0.5]); // premult red @ .5
        let white = TileRgba::splat([0.25, 0.25, 0.25, 0.25]); // premult white @ .25
        composite_tile(&mut acc, &red, BlendMode::Normal, 1.0).unwrap();
        composite_tile(&mut acc, &white, BlendMode::Normal, 1.0).unwrap();
        let got = acc.get(100, 100);
        assert!((got[0] - 0.625).abs() <= EPS);
        assert!((got[1] - 0.25).abs() <= EPS);
        assert!((got[2] - 0.25).abs() <= EPS);
        assert_eq!(got[3], 1.0);
    }
}
