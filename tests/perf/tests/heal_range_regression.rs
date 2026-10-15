// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! CRITICAL-1 regression gate (v1.0.1 heal fix) — an ordinary
//! `heal.stroke` over ordinary LDR content (every input in `[0, 1]`) must
//! leave the layer within a sane gradient-domain range. The assertion
//! form of the audit's `audit_verify_heal` probe (aud-perf lane): it
//! drives ONLY the shipped command surface (no instrumentation in the
//! measured path), composites the result, and fails if any scalar escapes
//! a generous bound.
//!
//! Root cause this pins against: the §45.1 multigrid commit
//! (`appname_algo::poisson`) used a coarse-grid correction scaled 2×
//! past the Galerkin-consistent value, so the V-cycle DIVERGED on any
//! non-rectangular Ω (a brush disk) — writing `-215`- and `1.4e10`-scale
//! pixels that the 8-bit export clamped into saturated black/white
//! blotches. A correct convergent solve stays near `[0, 1]` with only the
//! bounded overshoot gradient-domain blending is entitled to
//! (~`[-0.5, 1.5]` on these fixtures). The bound below (`|v| <= 3.0`) sits
//! well above legitimate overshoot and far below the divergent class, so
//! it reds the moment the solver stops converging on a masked window.

use appname_conformance::assembly_point;
use appname_conformance::stroke_goldens::fixed_samples_basic;
use appname_core::{io::png_io::composite_plane_f32, Engine};

/// Deterministic LDR noise as an RGBA8 PNG — the incompressible worst case
/// for a photo heal (independent guidance gradients at every edge).
fn noise_png(seed: u64, w: u32, h: u32) -> Vec<u8> {
    let mut state = seed;
    let mut next = move || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let mut data = vec![0u8; (w * h * 4) as usize];
    for chunk in data.chunks_exact_mut(8) {
        chunk.copy_from_slice(&next().to_le_bytes());
    }
    for px in data.chunks_exact_mut(4) {
        px[3] = 255;
    }
    encode_rgba8(&data, w, h)
}

/// Smooth photographic-ish content: a two-axis gradient with a soft blob —
/// rules out "incompressible noise" as the trigger.
fn gradient_png(w: u32, h: u32) -> Vec<u8> {
    let mut data = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let fx = x as f32 / w as f32;
            let fy = y as f32 / h as f32;
            let dx = fx - 0.5;
            let dy = fy - 0.5;
            let blob = (-(dx * dx + dy * dy) * 12.0).exp();
            data[i] = ((0.2 + 0.6 * fx) * 255.0) as u8;
            data[i + 1] = ((0.2 + 0.6 * fy) * 255.0) as u8;
            data[i + 2] = ((0.15 + 0.7 * blob) * 255.0) as u8;
            data[i + 3] = 255;
        }
    }
    encode_rgba8(&data, w, h)
}

fn encode_rgba8(data: &[u8], w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut wr = enc.write_header().unwrap();
        wr.write_image_data(data).unwrap();
    }
    out
}

/// Sane bound: legitimate gradient-domain overshoot on these fixtures
/// peaks near `[-0.5, 1.5]`; the divergent class produced `-215` upward.
/// `3.0` separates the two by a wide margin.
const SANE_ABS: f32 = 3.0;

/// Run the shipped healing-mode `heal.stroke` (self-source at an offset —
/// the intended gesture, the same form the `heal_poisson_4mp` bench uses)
/// over `content` at `canvas²`, then assert the composited layer stays
/// finite and within `[-SANE_ABS, SANE_ABS]`.
fn assert_heal_in_range(canvas: u32, noise: bool, size_px: u32, offset: (i32, i32)) {
    let mut engine: Engine = assembly_point::assembled_engine();
    engine
        .apply_bytes(
            format!(
                r#"{{"schema":"appname.command.v1","id":"be9c0000-0000-4000-8000-00000000000b","kind":"doc.new","params":{{"width":{canvas},"height":{canvas},"background":"white"}}}}"#
            )
            .as_bytes(),
        )
        .expect("doc.new");
    let asset = engine
        .add_raster_asset(if noise {
            noise_png(0x5eed, canvas, canvas)
        } else {
            gradient_png(canvas, canvas)
        })
        .expect("ingest");
    let paste = format!(
        r#"{{"schema":"appname.command.v1","id":"be9c0000-0000-4000-8000-00000000000d","kind":"edit.paste","params":{{"assetId":"{asset}","at":[0,0],"newLayerId":"be9c0000-0000-4000-8000-00000000000e"}}}}"#
    );
    engine.apply_bytes(paste.as_bytes()).expect("paste");
    let layer = "be9c0000-0000-4000-8000-00000000000e";

    let samples = engine.add_samples_asset(fixed_samples_basic());
    let env = format!(
        r#"{{"schema":"appname.command.v1","id":"be9c0000-0000-4000-8000-00000000000c","kind":"heal.stroke","target":{{"layerId":"{layer}"}},"params":{{"brushPreset":"{preset}","mode":"healing","source":{{"layerId":"{layer}"}},"offset":[{ox},{oy}],"sizePx":{size_px}}},"samples":"{samples}"}}"#,
        preset = assembly_point::preset_hard_round_16_id(),
        ox = offset.0,
        oy = offset.1,
    );
    engine.apply_bytes(env.as_bytes()).expect("heal");

    let (_, _, after) = composite_plane_f32(&engine).expect("composite");
    let (mut lo, mut hi, mut nonfinite) = (f32::INFINITY, f32::NEG_INFINITY, 0usize);
    for &v in &after {
        if !v.is_finite() {
            nonfinite += 1;
            continue;
        }
        lo = lo.min(v);
        hi = hi.max(v);
    }
    let content = if noise { "noise" } else { "gradient" };
    assert_eq!(
        nonfinite, 0,
        "heal wrote {nonfinite} non-finite scalars (canvas={canvas} {content} sizePx={size_px})"
    );
    assert!(
        lo >= -SANE_ABS && hi <= SANE_ABS,
        "heal escaped sane range: [{lo}, {hi}] (canvas={canvas} {content} \
         sizePx={size_px} offset={offset:?}); a bound of ±{SANE_ABS} \
         allows legitimate gradient-domain overshoot but reds the \
         divergent-solver class (CRITICAL-1)"
    );
}

#[test]
fn ordinary_spot_heal_noise_stays_in_range() {
    // The everyday gesture on incompressible content, the cases the audit
    // recorded diverging to [-39.47, 3.90], [-10.40, 19.12], [-215, 60].
    assert_heal_in_range(512, true, 64, (64, -32));
    assert_heal_in_range(512, true, 128, (40, 40));
    assert_heal_in_range(1024, true, 96, (64, -32));
}

#[test]
fn ordinary_spot_heal_gradient_stays_in_range() {
    // Smooth content — the audit recorded [-2.74, 1.00] / [-3.05, 1.00].
    assert_heal_in_range(512, false, 64, (64, -32));
    assert_heal_in_range(1024, false, 96, (64, -32));
}

#[test]
fn blanketing_heal_stays_in_range() {
    // Whole-canvas footprint (the shipped bench's shape) — always
    // converged; pinned so the fix cannot regress it either.
    assert_heal_in_range(512, true, 2500, (64, -32));
}
