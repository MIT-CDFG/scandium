// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// REGRESSION PIN for aud-corr CRITICAL-1 (v1.0.1 train, lane/fix-blend).
// Adopted verbatim from the audit lane's finding instrument
// (audit/aud-corr:crates/appname-render/tests/audit_corr_blend_edges.rs):
// probes 1/2/4/5 and 3 FAILED against v1.0.0-beta and are the evidence
// lines for reports/AUDIT-aud-corr-log.md CRITICAL-1. With the W3C
// backdrop guard restored in b_color_burn/b_color_dodge (blend.rs) and
// mirrored in composite.wgsl, all probes here now PASS and MUST stay
// green — this file is the standing regression pin.
//
// Ground truth: the blend module's own cited basis — W3C "Compositing and
// Blending Level 1", §4.5's declared era/standards basis (blend.rs module
// doc line 2). Appendix A confirms the same three-case guards. NOTE: the
// old blend.rs HardMix "equivalent for the canonical case" note is revised
// by this fix — with both guards correct the two published HardMix forms
// diverge at the (0,1) dodge endpoint (probe 3); the §4.5 text form
// governs the implementation.

use appname_core::BlendMode;
use appname_render::blend::{render_separable_kernel, routing, BlendRouting};

// ---------------------------------------------------------------------
// W3C Compositing and Blending Level 1 reference forms (§ blending).
// Transcribed from the published algorithm text, THREE-case guards intact.
// ---------------------------------------------------------------------

/// W3C: `if Cb == 1 -> 1; else if Cs == 0 -> 0; else 1 - min(1,(1-Cb)/Cs)`.
fn w3c_color_burn(b: f32, s: f32) -> f32 {
    if b == 1.0 {
        1.0
    } else if s == 0.0 {
        0.0
    } else {
        1.0 - ((1.0 - b) / s).min(1.0)
    }
}

/// W3C: `if Cb == 0 -> 0; else if Cs == 1 -> 1; else min(1, Cb/(1-Cs))`.
fn w3c_color_dodge(b: f32, s: f32) -> f32 {
    if b == 0.0 {
        0.0
    } else if s == 1.0 {
        1.0
    } else {
        (b / (1.0 - s)).min(1.0)
    }
}

/// Grid of exactly-representable [0,1] probe values, including the
/// endpoints where the guards fire.
fn grid() -> Vec<f32> {
    vec![0.0, 0.25, 0.5, 0.75, 1.0]
}

/// REGRESSION PROBE 1 — ColorBurn carries W3C's `Cb == 1` guard.
///
/// Reachable input: white backdrop under a black ColorBurn source, i.e.
/// b = 1.0, s = 0.0. W3C (and the "white is preserved by Color Burn"
/// behavior the mode is defined by) yields 1.0. Before the fix blend.rs
/// yielded 0.0 because its `s == 0` arm was tested FIRST and
/// unconditionally.
#[test]
fn probe_color_burn_backdrop_one_guard() {
    let k = render_separable_kernel(BlendMode::ColorBurn).expect("separable");
    let mut divergences = Vec::new();
    for &b in &grid() {
        for &s in &grid() {
            let got = k(b, s);
            let want = w3c_color_burn(b, s);
            if (got - want).abs() > 1e-6 {
                divergences.push(format!("  ColorBurn(b={b}, s={s}): impl={got}  W3C={want}"));
            }
        }
    }
    assert!(
        divergences.is_empty(),
        "blend.rs ColorBurn diverges from its cited W3C Level 1 basis at \
         {} of 25 grid points:\n{}\n\
         Cause: crates/appname-render/src/blend.rs tests `s == 0.0` before \
         the W3C `Cb == 1` case, so a WHITE backdrop burns to BLACK.",
        divergences.len(),
        divergences.join("\n")
    );
}

/// REGRESSION PROBE 2 — ColorDodge carries W3C's `Cb == 0` guard.
///
/// Reachable input: black backdrop under a white ColorDodge source
/// (b = 0.0, s = 1.0). W3C yields 0.0 (black is preserved by Color
/// Dodge); before the fix blend.rs yielded 1.0.
#[test]
fn probe_color_dodge_backdrop_zero_guard() {
    let k = render_separable_kernel(BlendMode::ColorDodge).expect("separable");
    let mut divergences = Vec::new();
    for &b in &grid() {
        for &s in &grid() {
            let got = k(b, s);
            let want = w3c_color_dodge(b, s);
            if (got - want).abs() > 1e-6 {
                divergences.push(format!(
                    "  ColorDodge(b={b}, s={s}): impl={got}  W3C={want}"
                ));
            }
        }
    }
    assert!(
        divergences.is_empty(),
        "blend.rs ColorDodge diverges from its cited W3C Level 1 basis at \
         {} of 25 grid points:\n{}\n\
         Cause: crates/appname-render/src/blend.rs tests `s == 1.0` before \
         the W3C `Cb == 0` case, so a BLACK backdrop dodges to WHITE.",
        divergences.len(),
        divergences.join("\n")
    );
}

/// REGRESSION PROBE 3 — INTERNAL CONSISTENCY, no external oracle needed.
///
/// blend.rs's HardMix note documents two published statements:
///   (a) §4.5 text/WGSL:  `B = (b + s >= 1) ? 1 : 0`   [what is implemented]
///   (b) Appendix A:      `B = VividLight(b, s) >= 0.5 ? 1 : 0`
/// Both are computed here from the SHIPPED kernels.
///
/// REVISES the audit's original probe-3 prediction. The audit predicted
/// that restoring ColorBurn's `Cb == 1` guard alone makes the two forms
/// "agree everywhere" — but that was computed with ColorDodge STILL
/// broken. With BOTH W3C backdrop guards correct (the actual CRITICAL-1
/// fix), the burn endpoint `(1,0)` now agrees (both give 1), while the
/// symmetric dodge endpoint `(0,1)` DIVERGES: the §4.5-text form gives 1
/// (0+1>=1) but the Appendix-A form gives 0 (correct ColorDodge(0,1)=0
/// preserves the black backdrop). This single-point divergence is
/// inherent — no state with both guards present makes the two coincide
/// everywhere — and is the true, pinned post-fix relationship. blend.rs's
/// HardMix note is corrected to match; §4.5 text form governs the
/// implementation. Flagged for O-INT/Appendix-A adjudication.
#[test]
fn probe_hard_mix_vivid_light_threshold_diverges_only_at_dodge_endpoint() {
    let hard_mix = render_separable_kernel(BlendMode::HardMix).expect("separable");
    let vivid = render_separable_kernel(BlendMode::VividLight).expect("separable");
    let mut divergences = Vec::new();
    for &b in &grid() {
        for &s in &grid() {
            let a = hard_mix(b, s);
            let vl = vivid(b, s);
            let bb = if vl >= 0.5 { 1.0 } else { 0.0 };
            if (a - bb).abs() > 1e-6 {
                divergences.push((b, s, a, vl, bb));
            }
        }
    }
    // Exactly one divergence, at the ColorDodge backdrop endpoint (0, 1).
    assert_eq!(
        divergences.len(),
        1,
        "expected the two HardMix forms to diverge at exactly the (0,1) \
         dodge endpoint; got {} divergence(s): {:?}",
        divergences.len(),
        divergences
    );
    let (b, s, text_form, vl, appendix_a) = divergences[0];
    assert_eq!(
        (b, s, text_form, appendix_a),
        (0.0, 1.0, 1.0, 0.0),
        "the sole HardMix-form divergence must be (b=0,s=1): §4.5-text=1, \
         Appendix-A=0 (VividLight={vl}); ColorDodge(0,1)=0 preserves black."
    );
    // The burn endpoint (1,0) must AGREE post-fix (both give 1).
    let a10 = hard_mix(1.0, 0.0);
    let vl10 = vivid(1.0, 0.0);
    assert_eq!(
        (a10, if vl10 >= 0.5 { 1.0 } else { 0.0 }),
        (1.0, 1.0),
        "burn endpoint (1,0) must reconcile once ColorBurn carries `Cb==1`"
    );
}

/// Control probe: the 27-mode routing partition (20 separable + 6 triple +
/// Dissolve) claimed at blend.rs:497. Guards the probes above against a
/// mis-routed mode silently returning the wrong kernel.
#[test]
fn probe_routing_partition_is_total() {
    const ALL: [BlendMode; 27] = [
        BlendMode::Normal,
        BlendMode::Dissolve,
        BlendMode::Darken,
        BlendMode::Multiply,
        BlendMode::ColorBurn,
        BlendMode::LinearBurn,
        BlendMode::DarkerColor,
        BlendMode::Lighten,
        BlendMode::Screen,
        BlendMode::ColorDodge,
        BlendMode::LinearDodge,
        BlendMode::LighterColor,
        BlendMode::Overlay,
        BlendMode::SoftLight,
        BlendMode::HardLight,
        BlendMode::VividLight,
        BlendMode::LinearLight,
        BlendMode::PinLight,
        BlendMode::HardMix,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::Subtract,
        BlendMode::Divide,
        BlendMode::Hue,
        BlendMode::Saturation,
        BlendMode::Color,
        BlendMode::Luminosity,
    ];
    let (mut sep, mut triple, mut dis) = (0, 0, 0);
    for m in ALL {
        match routing(m) {
            BlendRouting::Separable(_) => sep += 1,
            BlendRouting::WholePixel(_) | BlendRouting::NonSeparable(_) => triple += 1,
            BlendRouting::Dissolve => dis += 1,
        }
    }
    assert_eq!((sep, triple, dis), (20, 6, 1), "routing partition drifted");
}

// ---------------------------------------------------------------------
// END-TO-END: the guard restoration reaches composited PIXELS, not just
// the kernel seam. Uses the shipping `composite_tile` door with
// fully-opaque tiles, where Porter-Duff reduces to Co = B(Cb, Cs).
// ---------------------------------------------------------------------

use appname_render::blend::composite_tile;
use appname_render::tile_rgba::TileRgba;

/// REGRESSION PROBE 4 — the ColorBurn guard in a real composite.
///
/// Opaque WHITE backdrop, opaque BLACK ColorBurn source, opacity 1.
/// With αs = αb = 1 the §4.4 equation collapses to Co = B(Cb, Cs), so the
/// output pixel is the kernel value directly: W3C preserves white
/// (1,1,1). Before the fix it produced black (0,0,0) on every pixel.
#[test]
fn probe_color_burn_white_backdrop_composites_black() {
    let mut dst = TileRgba::splat([1.0, 1.0, 1.0, 1.0]); // opaque white
    let src = TileRgba::splat([0.0, 0.0, 0.0, 1.0]); // opaque black
    composite_tile(&mut dst, &src, BlendMode::ColorBurn, 1.0).expect("composite");
    let px = dst.get(0, 0);
    assert_eq!(
        [px[0], px[1], px[2]],
        [1.0, 1.0, 1.0],
        "opaque white backdrop + opaque black ColorBurn source \
         composited to {px:?}; W3C Level 1 preserves white (1,1,1,1)."
    );
}

/// REGRESSION PROBE 5 — the ColorDodge guard in a real composite.
/// Opaque BLACK backdrop, opaque WHITE ColorDodge source: W3C preserves
/// black. Before the fix it produced white.
#[test]
fn probe_color_dodge_black_backdrop_composites_white() {
    let mut dst = TileRgba::splat([0.0, 0.0, 0.0, 1.0]); // opaque black
    let src = TileRgba::splat([1.0, 1.0, 1.0, 1.0]); // opaque white
    composite_tile(&mut dst, &src, BlendMode::ColorDodge, 1.0).expect("composite");
    let px = dst.get(0, 0);
    assert_eq!(
        [px[0], px[1], px[2]],
        [0.0, 0.0, 0.0],
        "opaque black backdrop + opaque white ColorDodge source \
         composited to {px:?}; W3C Level 1 preserves black (0,0,0,1)."
    );
}

/// REGRESSION PROBE 6 — blast radius. Only the exact endpoints were
/// affected; the strict interior always matched W3C and must stay exact.
#[test]
fn probe_burn_dodge_interior_is_correct() {
    let burn = render_separable_kernel(BlendMode::ColorBurn).expect("separable");
    let dodge = render_separable_kernel(BlendMode::ColorDodge).expect("separable");
    // Strict interior: no guard can fire.
    for i in 1..20u32 {
        for j in 1..20u32 {
            let (b, s) = (i as f32 / 20.0, j as f32 / 20.0);
            assert!((burn(b, s) - w3c_color_burn(b, s)).abs() < 1e-6);
            assert!((dodge(b, s) - w3c_color_dodge(b, s)).abs() < 1e-6);
        }
    }
}
