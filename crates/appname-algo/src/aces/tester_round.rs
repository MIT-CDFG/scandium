// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Item 11 tester-round tests (2026-08-13): edge-domain, invariance-sweep,
//! and policy-pinning tests for the ACES 2.0 output-transform translation,
//! complementing the builder's oracle set in `super::tests`. Oracles are
//! either hand-evaluated from the CTL at the staged pin (`aces-dev`
//! 5ac93ba2, derivations noted per test) or measured-and-pinned properties
//! of this implementation with the measurement noted honestly.
//! `#[cfg(test)]`-only module; no implementation code is touched.

use super::display_encoding::{display_decoding, display_encoding, Eotf};
use super::output_transform::{gamut_compress_fwd, jmh_to_rgb, output_transform_fwd, rgb_to_jmh};
use super::tonescale::{init_ts_params, tonescale_fwd};
use super::utilities::{clamp_f3, mult_f3_f33};
use super::*;

const IDENT: M33 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// FNV-1a over little-endian f64 bit patterns (local helper; the builder's
/// twin in `super::tests` is not visible from a sibling test module).
fn fnv1a_f64(values: &[f64]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for v in values {
        for b in v.to_bits().to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    h
}

/// splitmix64 — tiny seeded PRNG for the determinism fixture (fixed
/// sequence; deliberately a different generator and seed than the builder's
/// deterministic exposure-sweep fixture).
struct SplitMix64(u64);
impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }
    /// Uniform in [0, 1).
    fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }
}

#[test]
fn tonescale_matches_hand_evaluated_ctl_constants() {
    // Oracle: the init_TSParams + tonescale_fwd formulas of
    // Lib.Academy.Tonescale.ctl (staged pin 5ac93ba2) hand-evaluated
    // independently in IEEE f64 (Python, transcribed from the CTL text, not
    // from the Rust translation). Constants below are those evaluations.
    let p = init_ts_params(100.0);
    // CTL-derived expected values: printed precision is faithful to the
    // derivation record; the comparison is bit-level via parsed f64.
    #[allow(clippy::excessive_precision)]
    for (x, expect) in [
        (0.001, 0.000412917745789f64),
        (0.02, 0.297911968689214),
        (0.18, 9.999932511260242),
        (1.0, 45.756579000767786),
        (10.0, 90.794686899740071),
    ] {
        let y = tonescale_fwd(x, &p);
        let rel = ((y - expect) / expect).abs();
        assert!(rel < 1e-12, "ts({x}) = {y:.15} vs CTL-derived {expect:.15}");
    }
}

#[test]
fn tonescale_r_hit_maps_exactly_to_peak() {
    // Hand-derived CTL identity (derivation from Lib.Academy.Tonescale.ctl):
    // u_2 = ((r_hit/m_1)/((r_hit/m_1)+w_2))^g = (r_hit/(r_hit+s_2))^g since
    // s_2 = w_2·m_1, so f(r_hit) = m_2·u_2 = m_1. The flare numerator then
    // telescopes: m_1 solves m_1² − m_0·m_1 − m_0·t_1 = 0, hence
    // m_1²/(m_1+t_1) = m_0, and the output is m_0·n_r = n exactly. So the
    // scene value "hitting the roof" (r_hit) maps to peak luminance
    // *exactly* in exact arithmetic — pinned here within fp rounding.
    // r_hit(100) = 128 (log term vanishes); r_hit(1000) = 128 + 768·(ln 10 /
    // ln 100) = 512.
    let p100 = init_ts_params(100.0);
    assert!((tonescale_fwd(128.0, &p100) - 100.0).abs() < 1e-9);
    let p1000 = init_ts_params(1000.0);
    assert!((tonescale_fwd(512.0, &p1000) - 1000.0).abs() < 1e-8);
}

#[test]
fn negative_scene_values_match_ap1_clamp_semantics() {
    // Oracle from the CTL driver: outputTransform_fwd's first step is
    // clamp_AP0_to_AP1(aces, 0, forward_limit), so any input with negative
    // components (camera noise below black) must render identically to its
    // explicitly AP1-clamped counterpart. The pre-clamped path re-enters
    // the same clamp, so equality is up to the AP1→AP0→AP1 matrix round
    // trip (measured ≤ 2.8e-16 on these cases; pinned at 1e-12).
    let ot = SdrOutputTransform::sdr_100nit_rec709_srgb();
    let cases = [
        [-0.05, 0.1, 0.2],
        [-1.0, 0.18, 0.18],
        [0.3, -0.02, 0.5],
        [-1e-6, 0.0, 0.0],
        [0.05, 0.4, -0.3],
    ];
    for aces in cases {
        let ap1 = mult_f3_f33(aces, &ot.params.ap0_to_ap1);
        let ap1_clamped = clamp_f3(ap1, 0.0, ot.params.ts.forward_limit);
        let pre_clamped = mult_f3_f33(ap1_clamped, &ot.params.ap1_to_ap0);
        let direct = ot.forward(aces);
        let via_clamp = ot.forward(pre_clamped);
        for c in 0..3 {
            assert!(
                (direct[c] - via_clamp[c]).abs() < 1e-12,
                "{aces:?}: {direct:?} vs {via_clamp:?}"
            );
        }
    }
    // All-negative input clamps to AP1 zero → exact display black.
    assert_eq!(ot.forward([-0.5, -0.5, -0.5]), [0.0, 0.0, 0.0]);
}

#[test]
fn neutral_axis_no_tint_across_luminance_sweep() {
    // Translation-fidelity sweep: neutral AP0 stays neutral through the
    // whole forward chain (the JMh chroma stages are pass-through at M = 0,
    // and equal cone responses annihilate the a/b rows — see the builder's
    // derivation in `super::tests`). Any matrix or table slip introduces a
    // tint. Measured max channel spread on this 200-point log sweep:
    // 8.9e-16; pinned at 1e-12.
    let ot = SdrOutputTransform::sdr_100nit_rec709_srgb();
    let n = 200;
    for i in 0..n {
        let t = i as f64 / (n - 1) as f64;
        let x = 1e-6 * (127.9f64 / 1e-6f64).powf(t);
        let out = ot.forward([x, x, x]);
        let mx = out[0].max(out[1]).max(out[2]);
        let mn = out[0].min(out[1]).min(out[2]);
        assert!(
            mx - mn < 1e-12,
            "tint at neutral x={x}: spread {:e} ({out:?})",
            mx - mn
        );
    }
}

#[test]
fn neutral_display_luminance_strictly_monotone_then_saturates() {
    // Monotonicity along neutrals across ~8 decades: display code value is
    // strictly increasing in scene luminance up to r_hit = 128 (tonescale
    // strictly monotone there, encode strictly monotone, no clamping below
    // peak), then pinned to display white at and beyond r_hit (the r_hit →
    // peak identity above plus the white-limiting clamp).
    let ot = SdrOutputTransform::sdr_100nit_rec709_srgb();
    let n = 300;
    let mut prev = -1.0f64;
    for i in 0..n {
        let t = i as f64 / (n - 1) as f64;
        let x = 1e-6 * (127.9f64 / 1e-6f64).powf(t);
        let y = ot.forward([x, x, x])[1];
        assert!(y > prev, "not strictly increasing at x={x}: {y} <= {prev}");
        prev = y;
    }
    // Saturation region: at/above r_hit the output stays within one ulp-ish
    // band of display white and never exceeds 1.0 or decreases below the
    // r_hit value.
    let at_hit = ot.forward([128.0, 128.0, 128.0]);
    for x in [128.0, 200.0, 1000.0, 65504.0] {
        let out = ot.forward([x, x, x]);
        for c in 0..3 {
            assert!(out[c] <= 1.0);
            assert!(
                out[c] >= at_hit[c] - 1e-12,
                "saturated value fell below the r_hit level at x={x}: {out:?}"
            );
            assert!(out[c] > 1.0 - 1e-9, "not saturated at x={x}: {out:?}");
        }
    }
}

#[test]
fn full_chain_preserves_jmh_hue_and_ordering() {
    // The tonescale/chroma/gamut stages all pass h through unchanged, so
    // the forward core must preserve JMh hue end-to-end for in-AP1 inputs
    // (a matrix or table slip breaks this). Ring at J=40, M=20 (inside the
    // AP1 clamp for every hue), hues 0.5° + 5°·k. Measured max hue
    // deviation 5.3e-13 and no ordering crossings; pinned at 1e-9.
    let ot = SdrOutputTransform::sdr_100nit_rec709_srgb();
    let p = &ot.params;
    let mut prev_h = -1.0f64;
    for step in 0..72 {
        let h = step as f64 * 5.0 + 0.5;
        let scene = jmh_to_rgb([40.0, 20.0, h], &p.input_params);
        let disp = output_transform_fwd(scene, p);
        let jmh_out = rgb_to_jmh(disp, &p.limit_params);
        let dh = (jmh_out[2] - h).abs();
        let dh = dh.min(360.0 - dh);
        assert!(dh < 1e-9, "hue drift at h={h}: out {}", jmh_out[2]);
        assert!(
            jmh_out[2] > prev_h,
            "hue ordering crossing at h={h}: {} <= {prev_h}",
            jmh_out[2]
        );
        prev_h = jmh_out[2];
    }
}

#[test]
fn pure_primaries_dominant_channel_and_gamut() {
    // Saturated single-channel primaries: after the full chain the
    // corresponding display channel must remain the (weak) maximum, and
    // everything stays in display range. At moderate luminance the
    // dominance is strict; at extreme luminance channels may saturate to
    // white together (ties allowed).
    let ot = SdrOutputTransform::sdr_100nit_rec709_srgb();
    for (i, e) in [[8.0, 0.0, 0.0], [0.0, 8.0, 0.0], [0.0, 0.0, 8.0]]
        .iter()
        .enumerate()
    {
        let out = ot.forward(*e);
        for c in 0..3 {
            assert!((0.0..=1.0).contains(&out[c]), "{e:?} → {out:?}");
            if c != i {
                assert!(
                    out[i] > out[c],
                    "channel {i} not strictly dominant for {e:?}: {out:?}"
                );
            }
        }
    }
    for (i, e) in [[500.0, 0.0, 0.0], [0.0, 500.0, 0.0], [0.0, 0.0, 500.0]]
        .iter()
        .enumerate()
    {
        let out = ot.forward(*e);
        for c in 0..3 {
            assert!((0.0..=1.0).contains(&out[c]), "{e:?} → {out:?}");
            assert!(
                out[i] >= out[c],
                "channel {i} not dominant for {e:?}: {out:?}"
            );
        }
    }
}

#[test]
fn gamut_compressor_never_expands_m() {
    // Hand-derived from the CTL remap: in the compression branch
    // (gamut boundary < reach boundary) scale > 0 and the Reinhard remap
    // satisfies nd/(1+nd) ≤ nd, so remapped M ≤ M with equality below the
    // threshold — the forward compressor can only move colors inward.
    // Measured on this grid: zero expansions.
    let p = init_odt_params(100.0, &REC709_PRI);
    for h_step in 0..72 {
        let h = h_step as f64 * 5.0;
        for j in [5.0, 20.0, 40.0, 60.0, 80.0, 95.0] {
            for m in [5.0, 20.0, 50.0, 100.0, 200.0, 400.0] {
                let out = gamut_compress_fwd([j, m, h], &p);
                assert!(
                    out[1] <= m + 1e-12,
                    "M expanded at h={h} J={j} M={m}: {}",
                    out[1]
                );
            }
        }
    }
}

#[test]
fn inverse_consistency_at_display_black_and_peak() {
    // Round-trip in the hardest regions: display code values within a few
    // thousandths of black and of peak white (the builder's grid stopped at
    // [0.08, 0.95]), both neutral and slightly chromatic, including
    // cv = 1.0 exactly (which decodes to the tonescale inverse limit).
    // Measured max code-value error: 5.6e-16 neutral, 1.8e-14 chromatic;
    // pinned at 1e-10.
    let ot = SdrOutputTransform::sdr_100nit_rec709_srgb();
    let mut cases: Vec<[f64; 3]> = vec![
        [0.005, 0.004, 0.006],
        [0.02, 0.015, 0.03],
        [0.95, 0.9, 0.92],
        [0.99, 0.97, 0.985],
        [0.999, 0.99, 0.995],
    ];
    for cv in [0.001, 0.005, 0.01, 0.98, 0.99, 0.995, 0.999, 1.0] {
        cases.push([cv, cv, cv]);
    }
    for c in cases {
        let aces = ot.inverse(c);
        let back = ot.forward(aces);
        for i in 0..3 {
            assert!(aces[i].is_finite());
            assert!(
                (back[i] - c[i]).abs() < 1e-10,
                "cv {c:?} → aces {aces:?} → {back:?}"
            );
        }
    }
}

#[test]
fn eotf_presets_decode_side_cross_check_and_divergence() {
    // Same display-linear input x: encode+decode through the sRGB pair and
    // through the BT.1886 pair must each return x (decode-side cross-check
    // of the two transfer legs; measured ≤ 3.4e-16, pinned 1e-12) — while
    // the *code values* the two presets emit genuinely differ (18% grey
    // codes differ by 0.0339 measured; asserted > 0.03 so the presets can
    // never silently collapse into one transfer).
    for i in 0..=1000 {
        let x = i as f64 / 1000.0;
        let v = [x, x, x];
        let cs = display_encoding(v, &IDENT, Eotf::SrgbIec61966, 1.0);
        let cb = display_encoding(v, &IDENT, Eotf::Bt1886, 1.0);
        let ds = display_decoding(cs, &IDENT, Eotf::SrgbIec61966, 1.0);
        let db = display_decoding(cb, &IDENT, Eotf::Bt1886, 1.0);
        for d in [ds, db] {
            assert!((d[0] - x).abs() < 1e-12, "decode mismatch at {x}");
        }
    }
    let srgb = SdrOutputTransform::sdr_100nit_rec709_srgb();
    let bt = SdrOutputTransform::sdr_100nit_rec709_bt1886();
    let a = srgb.forward([0.18, 0.18, 0.18]);
    let b = bt.forward([0.18, 0.18, 0.18]);
    assert!(
        (a[0] - b[0]).abs() > 0.03,
        "presets should emit different code values at 18% grey: {a:?} vs {b:?}"
    );
}

#[test]
fn non_finite_inputs_pin_current_behavior() {
    // Documents current behavior, not a contract. The CTL-semantics clamp
    // (`if a > b {a} else {b}` comparisons) absorbs NaN and ±Inf at the
    // AP1 clamp: ctl_min(NaN, hi) and ctl_min(+Inf, hi) both take `hi`
    // (comparison false / clamp), so corrupted inputs land on the
    // forward_limit and render as display white — finite and in range, no
    // NaN propagation to the display side. Inverse of a NaN code value
    // currently returns exact black.
    let ot = SdrOutputTransform::sdr_100nit_rec709_srgb();
    for e in [
        [f64::NAN, 0.5, 0.5],
        [f64::NAN, f64::NAN, f64::NAN],
        [f64::INFINITY, 0.5, 0.5],
        [f64::INFINITY, f64::INFINITY, f64::INFINITY],
        [f64::NEG_INFINITY, 0.5, 0.5],
    ] {
        let out = ot.forward(e);
        for c in 0..3 {
            assert!(out[c].is_finite(), "{e:?} → non-finite {out:?}");
            assert!((0.0..=1.0).contains(&out[c]), "{e:?} → {out:?}");
            assert!(
                out[c] > 0.999,
                "non-finite input no longer renders as white: {e:?} → {out:?}"
            );
        }
    }
    let inv = ot.inverse([f64::NAN, 0.5, 0.5]);
    assert_eq!(inv, [0.0, 0.0, 0.0], "inverse-of-NaN behavior changed");
}

#[test]
fn determinism_two_run_hash_on_seeded_fixture() {
    // Two-run bit-hash on a splitmix64-seeded pseudo-random HDR fixture
    // (seed 0x1735_ACE5 — a different generator and sequence than the
    // builder's exposure-sweep fixture), through BOTH SDR presets. Also
    // pinned to a golden from this implementation's own run with the
    // oracle tests above green (rule 8 — never hand-invented).
    let run = || -> u64 {
        let srgb = SdrOutputTransform::sdr_100nit_rec709_srgb();
        let bt = SdrOutputTransform::sdr_100nit_rec709_bt1886();
        let mut rng = SplitMix64(0x1735_ACE5);
        let mut flat = Vec::new();
        for _ in 0..500 {
            // log-uniform magnitude in [1e-5, 1e3), independent per channel,
            // occasional negatives (1 in 8 draws) for the noise-floor path.
            let mut px = [0.0f64; 3];
            for chan in px.iter_mut() {
                let mag = 1e-5 * (1e8f64).powf(rng.next_f64());
                let neg = rng.next_u64().is_multiple_of(8);
                *chan = if neg { -mag } else { mag };
            }
            flat.extend_from_slice(&srgb.forward(px));
            flat.extend_from_slice(&bt.forward(px));
        }
        fnv1a_f64(&flat)
    };
    let h1 = run();
    let h2 = run();
    assert_eq!(h1, h2, "two-run bit-hash mismatch");
    // Pinned 2026-08-13 from this implementation's own run (see above).
    const GOLDEN_TESTER_FIXTURE: u64 = 0xff88d7eb3e4f177c;
    assert_eq!(
        h1, GOLDEN_TESTER_FIXTURE,
        "pinned golden mismatch: got {h1:#018x}"
    );
}
