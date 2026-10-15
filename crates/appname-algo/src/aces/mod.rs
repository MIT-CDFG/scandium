// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Derived by translation from ACES Core Transforms (aces-dev),
// https://github.com/ampas/aces-dev, commit
// 5ac93ba2d48a00aca6e4811fa0b700911c2a8809 (census key `aces-dev`):
// the lib/ transform library (Lib.Academy.{Utilities,ColorSpaces,Tonescale,
// OutputTransform,DisplayEncoding}.ctl — per-file derivation noted in each
// submodule header). Apache-2.0 at the pin (LICENSE sha256
// 43070e2d4e532684de521b885f385d0841030efa2b1a20bafb76133a5e1379c1, read
// first-hand 2026-08-13). Per Apache-2.0 Section 4(c) the upstream per-file
// notice "Copyright Contributors to the ACES Project." is retained in these
// headers; repo-level notice carriage per the `appname.port-record.v1`
// draft (reports/O-ALGO-log.md, Item 11; O-BUILD ratification pending).

//! Spec: ALGORITHMS.md §16 `hdr_tonemap` — `tmo_set` entry
//! `aces_view_transform` ("ACES-reference view transform (L0, public spec)
//! covering the display-referred path") per
//! `contracts/algorithms/algorithms.yaml`; ES §17.2 HDR rows (32-bit →
//! display range) with the display-transform context of ES §5.7/§24.4 (the
//! §16.A "SPEC §5.6/§24.3" anchors are stale against current ES text —
//! noted in the Item 11 log entry); INTEGRATION-NOTES §6 order 11
//! (`aces-dev` transforms, "own CTL→Rust translation of the needed
//! transforms only").
//!
//! ACES 2.0 output transforms, translated from the Academy's CTL library
//! (see the attribution header): forward and inverse SDR output transform
//! from scene-linear ACES2065-1 (AP0) to display code values. "ACES" and
//! Academy marks appear here as descriptive references to the transforms'
//! origin only (census trademark note; D-43) — never in product-facing
//! feature naming.
//!
//! Submodules mirror the upstream lib/ files:
//! * [`utilities`] — scalar/matrix helpers (`Lib.Academy.Utilities.ctl`).
//! * [`colorspaces`] — xyY/XYZ + chromatic adaptation
//!   (`Lib.Academy.ColorSpaces.ctl`).
//! * [`tonescale`] — the ACES 2.0 tonescale (`Lib.Academy.Tonescale.ctl`).
//! * [`output_transform`] — JMh appearance conversion, chroma compression,
//!   gamut compression, tables (`Lib.Academy.OutputTransform.ctl`).
//! * [`display_encoding`] — sRGB / BT.1886 encode
//!   (`Lib.Academy.DisplayEncoding.ctl`).
//!
//! The [`SdrOutputTransform`] wrapper below is Appname's own composition of
//! the translated library stages (the upstream top-level Output.* CTLs are
//! not part of the staged lib/ port surface): forward = output-transform
//! core → clamp to `[0, peak/ref]` (the "white limiting step" the upstream
//! `eotf_inv` comment references) → display encoding; inverse = display
//! decoding → inverse core.
//!
//! ADAPTER SEAM (brokered; this crate never touches documents): the CP-1.0
//! HDR develop stage (`hdr_tonemap` TMO set) converts working-space
//! scene-linear tiles to ACES2065-1, calls [`SdrOutputTransform::forward`]
//! per pixel, and lands results through the develop-stage
//! `appname.command.v1` envelope path with captured values (D-48). Any GPU
//! pair is a later O-RENDER parity concern; this CPU path is the reference.
//!
//! Determinism (D-48): pure scalar `f64` functions, single-threaded fixed
//! iteration order, no RNG; the parameter tables are rebuilt identically on
//! every [`init_odt_params`] call (two-run bit-hash test below).

pub mod colorspaces;
pub mod display_encoding;
pub mod output_transform;
pub mod tonescale;
pub mod utilities;

pub use display_encoding::{display_decoding, display_encoding, Eotf};
pub use output_transform::{
    init_odt_params, output_transform_fwd, output_transform_inv, OdtParams, AP0, AP1, REF_LUMINANCE,
};
pub use tonescale::{init_ts_params, tonescale_fwd, tonescale_inv, TsParams};
pub use utilities::{Chromaticities, M33};

use utilities::clamp_f3;

/// Rec. ITU-R BT.709 / sRGB primaries and D65 white (standard published
/// chromaticities; supplied to the transform the way the upstream top-level
/// Output.* CTLs pass their limiting primaries).
pub const REC709_PRI: Chromaticities = Chromaticities {
    red: [0.640, 0.330],
    green: [0.300, 0.600],
    blue: [0.150, 0.060],
    white: [0.3127, 0.3290],
};

const MATRIX_IDENTITY: M33 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// An SDR output transform preset: the pre-computed parameter block plus
/// the display-encoding leg. See the module docs for the composition note.
#[derive(Clone, Debug)]
pub struct SdrOutputTransform {
    pub params: OdtParams,
    pub eotf: Eotf,
    /// Limiting → display primary matrix (identity when the limiting and
    /// display primaries coincide, as in both presets here).
    pub limit_to_display: M33,
    /// Display → limiting primary matrix (inverse of the above).
    pub display_to_limit: M33,
    /// Linear scale factor on the display-linear leg (1.0 for SDR).
    pub linear_scale: f64,
}

impl SdrOutputTransform {
    /// SDR 100-nit, Rec.709 limit, sRGB piecewise encode
    /// (IEC 61966-2-1:1999) — the editor's primary display-referred path.
    pub fn sdr_100nit_rec709_srgb() -> Self {
        SdrOutputTransform {
            params: init_odt_params(100.0, &REC709_PRI),
            eotf: Eotf::SrgbIec61966,
            limit_to_display: MATRIX_IDENTITY,
            display_to_limit: MATRIX_IDENTITY,
            linear_scale: 1.0,
        }
    }

    /// SDR 100-nit, Rec.709 limit, BT.1886 (gamma 2.4) encode — same
    /// display-linear rendering as the sRGB preset, different transfer.
    pub fn sdr_100nit_rec709_bt1886() -> Self {
        SdrOutputTransform {
            params: init_odt_params(100.0, &REC709_PRI),
            eotf: Eotf::Bt1886,
            limit_to_display: MATRIX_IDENTITY,
            display_to_limit: MATRIX_IDENTITY,
            linear_scale: 1.0,
        }
    }

    /// Forward: scene-linear ACES2065-1 (AP0) → display code values in
    /// [0, 1].
    pub fn forward(&self, aces: [f64; 3]) -> [f64; 3] {
        let rgb = output_transform_fwd(aces, &self.params);
        // White limiting step: clamp to [0, peak/ref] display-linear
        // (referenced by the upstream eotf_inv negative-clamp comment).
        let limited = clamp_f3(rgb, 0.0, self.params.peak_luminance / REF_LUMINANCE);
        display_encoding(
            limited,
            &self.limit_to_display,
            self.eotf,
            self.linear_scale,
        )
    }

    /// Inverse: display code values → scene-linear ACES2065-1 (AP0).
    /// Near-inverse — the gamut/chroma stages invert analytical
    /// approximations (bounds pinned in tests).
    pub fn inverse(&self, cv: [f64; 3]) -> [f64; 3] {
        let limited = display_decoding(cv, &self.display_to_limit, self.eotf, self.linear_scale);
        output_transform_inv(limited, &self.params)
    }
}

#[cfg(test)]
mod tester_round;

#[cfg(test)]
mod tests {
    use super::display_encoding::moncurve_inv;
    use super::output_transform::{
        gamut_compress_fwd, init_jmh_params, jmh_to_rgb, rgb_to_jmh, y_to_j, TOTAL_TABLE_SIZE,
    };
    use super::tonescale::tonescale_fwd;
    use super::*;

    /// FNV-1a over the little-endian f64 bit patterns — local golden
    /// hashing helper (no dependencies, fixed order).
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

    /// Deterministic synthetic HDR-ish fixture: an exposure sweep crossed
    /// with chroma perturbations, plus edge values (zeros, negatives, far
    /// out-of-range). Fixed order, no RNG.
    fn hdr_fixture() -> Vec<[f64; 3]> {
        let mut v = Vec::new();
        for ev in -8..=8 {
            let base = 0.18 * (2.0f64).powi(ev);
            for k in 0..6 {
                let t = k as f64 / 5.0;
                v.push([base * (1.0 + t), base, base * (1.0 - 0.8 * t)]);
                v.push([base * (1.0 - 0.8 * t), base * (1.0 + t), base]);
                v.push([base, base * (1.0 - 0.8 * t), base * (1.0 + t)]);
            }
        }
        v.push([0.0, 0.0, 0.0]);
        v.push([-0.1, 0.05, 0.2]);
        v.push([1000.0, 1000.0, 1000.0]);
        v.push([500.0, 0.0, 0.0]);
        v
    }

    #[test]
    fn white_maps_to_j_100_and_limit_j_max() {
        // Hand-derivable CAM anchor: at the reference white (Y = 100 nits)
        // the achromatic response A equals A_w by construction, so
        // J = 100·(A/A_w)^cz = 100 exactly; for the 100-nit transform,
        // limit_J_max = Y_to_J(100) = 100 likewise.
        let p = init_jmh_params(&AP0);
        assert!((y_to_j(100.0, &p) - 100.0).abs() < 1e-9);
        let odt = init_odt_params(100.0, &REC709_PRI);
        assert!((odt.limit_j_max - 100.0).abs() < 1e-9);
    }

    #[test]
    fn neutral_axis_is_achromatic_in_jmh() {
        // Hand-derived from the CTL (derivation): for neutral AP0 input
        // (x,x,x), rgb·RGB_TO_XYZ = x·XYZ_w/100, and the D_RGB diagonal is
        // built so XYZ_w·MATRIX_16·diag(D_RGB) has equal channels
        // (= F_L_n·Y_w each). Equal cone responses annihilate the a/b rows
        // of the Aab matrix (columns 1,2 of the base matrix sum to zero:
        // 1 − 12/11 + 1/11 = 0 and 1/9 + 1/9 − 2/9 = 0), so M = 0.
        let p = init_jmh_params(&AP0);
        for x in [1e-4, 0.01, 0.18, 1.0, 4.0, 64.0] {
            let jmh = rgb_to_jmh([x, x, x], &p);
            assert!(jmh[1].abs() < 1e-9, "M at neutral {x}: {}", jmh[1]);
            // and J agrees with the achromatic shortcut Y_to_J
            let j_direct = y_to_j(x * 100.0, &p);
            assert!(
                (jmh[0] - j_direct).abs() < 1e-9,
                "J at neutral {x}: {} vs {}",
                jmh[0],
                j_direct
            );
        }
    }

    #[test]
    fn jmh_round_trip_in_display_range() {
        let p = init_jmh_params(&REC709_PRI);
        for r in [0.02, 0.25, 0.9] {
            for g in [0.05, 0.5, 1.0] {
                for b in [0.03, 0.4, 0.8] {
                    let rgb = [r, g, b];
                    let back = jmh_to_rgb(rgb_to_jmh(rgb, &p), &p);
                    for i in 0..3 {
                        assert!(
                            (back[i] - rgb[i]).abs() < 1e-9,
                            "round trip {rgb:?} → {back:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn parameter_tables_are_sane() {
        let p = init_odt_params(100.0, &REC709_PRI);
        // Hue table strictly increasing across the padded range.
        for i in 1..TOTAL_TABLE_SIZE {
            assert!(
                p.table_hues[i] > p.table_hues[i - 1],
                "hue table not strictly increasing at {i}: {} <= {}",
                p.table_hues[i],
                p.table_hues[i - 1]
            );
        }
        // Cusp J within (0, limit_J_max); cusp M positive; upper-hull
        // gamma inverses positive and bounded by the search window.
        for i in 0..TOTAL_TABLE_SIZE {
            assert!(p.table_gamut_cusps[i][0] > 0.0);
            assert!(p.table_gamut_cusps[i][0] < p.limit_j_max);
            assert!(p.table_gamut_cusps[i][1] > 0.0);
            assert!(p.table_upper_hull_gamma[i] > 1.0 / 5.0);
            assert!(p.table_upper_hull_gamma[i].is_finite());
            assert!(p.table_reach_m[i] > 0.0);
            assert!(p.table_reach_m[i] < 1300.0);
        }
    }

    #[test]
    fn neutral_axis_full_pipeline_matches_tonescale() {
        // Hand-evaluated CTL cross-value (derivation in the CTL terms):
        // for neutral input the JMh chroma stages are pass-through (M = 0),
        // so the display-linear output is exactly
        // tonescale_fwd(x)/ref_luminance on every channel, and the code
        // value is the sRGB encode of that. Each step of the collapse:
        //   RGB_to_JMh(x,x,x)      → (Y_to_J(100x), 0, 0)
        //   tonemap_and_compress   → (Y_to_J(tonescale_fwd(x)), 0, 0)
        //   gamut_compress (M=0)   → unchanged (remap_M(0) = 0)
        //   JMh_to_RGB             → (ts(x)/100, ts(x)/100, ts(x)/100)
        let ot = SdrOutputTransform::sdr_100nit_rec709_srgb();
        for x in [0.001, 0.05, 0.18, 0.5, 1.0, 8.0] {
            let out = ot.forward([x, x, x]);
            let expect = moncurve_inv(tonescale_fwd(x, &ot.params.ts) / 100.0, 2.4, 0.055);
            // Index loop kept: the assertion references out[c] by index.
            #[allow(clippy::needless_range_loop)]
            for c in 0..3 {
                assert!(
                    (out[c] - expect).abs() < 1e-6,
                    "neutral {x} channel {c}: {} vs {}",
                    out[c],
                    expect
                );
            }
        }
    }

    #[test]
    fn gray_18_maps_near_srgb_0_35() {
        // 18% grey → 10 nits (tonescale anchor test; the CTL constants
        // block documents the grey anchor) → sRGB encode of 0.100 ≈ 0.349
        // code value.
        let ot = SdrOutputTransform::sdr_100nit_rec709_srgb();
        let out = ot.forward([0.18, 0.18, 0.18]);
        let expect = moncurve_inv(10.0 / 100.0, 2.4, 0.055);
        assert!(
            (out[0] - expect).abs() < 1e-3,
            "18% grey code value {} vs {expect}",
            out[0]
        );
    }

    #[test]
    fn black_and_peak_limits() {
        let ot = SdrOutputTransform::sdr_100nit_rec709_srgb();
        // Black floor: 0 in → 0 out (tonescale flare toe passes 0; the
        // clamp and encode preserve it).
        let black = ot.forward([0.0, 0.0, 0.0]);
        assert_eq!(black, [0.0, 0.0, 0.0]);
        // Far-above-peak neutral pins to display white.
        let white = ot.forward([1000.0, 1000.0, 1000.0]);
        // Index loop kept: the assertion references white[c] by index.
        #[allow(clippy::needless_range_loop)]
        for c in 0..3 {
            assert!(
                white[c] > 0.995 && white[c] <= 1.0,
                "channel {c}: {}",
                white[c]
            );
        }
    }

    #[test]
    fn extreme_inputs_stay_in_range() {
        let ot = SdrOutputTransform::sdr_100nit_rec709_srgb();
        let extremes = [
            [500.0, 0.0, 0.0],
            [0.0, 500.0, 0.0],
            [0.0, 0.0, 500.0],
            [-0.2, 0.1, 0.05],
            [65504.0, 32.0, 0.0],
            [1e-8, 1e-8, 1e-8],
        ];
        for e in extremes {
            let out = ot.forward(e);
            for c in 0..3 {
                assert!(out[c].is_finite(), "{e:?} → non-finite");
                assert!(
                    (0.0..=1.0).contains(&out[c]),
                    "{e:?} → out of range {out:?}"
                );
            }
        }
    }

    #[test]
    fn gamut_compressor_below_threshold_is_pass_through() {
        // Translation-fidelity oracle: solve_J_intersect is constructed so
        // the compression vector through (J, M) re-crosses (J, M); when M
        // is below the compression threshold (remap_M returns M
        // unchanged), the output must reconstruct J exactly (up to fp).
        // Measured: 835/864 of these small-M probes are below threshold,
        // worst J deviation 2.9e-14.
        let p = init_odt_params(100.0, &REC709_PRI);
        let mut unchanged = 0;
        let mut total = 0;
        for h_step in 0..72 {
            let h = h_step as f64 * 5.0;
            for j in [10.0, 30.0, 50.0, 70.0] {
                for m in [1.0, 3.0, 6.0] {
                    let out = gamut_compress_fwd([j, m, h], &p);
                    total += 1;
                    if out[1] == m {
                        unchanged += 1;
                        assert!(
                            (out[0] - j).abs() < 1e-9,
                            "pass-through J drift at h={h} J={j} M={m}: {}",
                            out[0]
                        );
                        assert_eq!(out[2], h);
                    }
                }
            }
        }
        assert_eq!(total, 864);
        assert!(unchanged > 700, "only {unchanged}/864 below threshold");
    }

    #[test]
    fn gamut_compressed_values_bounded_near_display_hull() {
        // Reach-bounded JMh values through the forward compressor decode
        // to display-linear values bounded near the hull. The boundary is
        // an analytical approximation by design (see
        // estimate_line_and_boundary_intersection_M) and the pipeline's
        // white-limiting clamp is the final cleanup, so the honest bound
        // is loose: measured excursion on this grid [-0.037, 1.175];
        // pinned at [-0.05, 1.20]. A translation slip (wrong matrix/table)
        // blows far past this.
        let p = init_odt_params(100.0, &REC709_PRI);
        for h_step in 0..72 {
            let h = h_step as f64 * 5.0;
            let reach = super::output_transform::reach_m_from_table(h, &p.table_reach_m);
            for j in [5.0, 20.0, 40.0, 60.0, 80.0, 95.0] {
                for mf in [0.25, 0.5, 0.75, 1.0] {
                    let jmh = [j, mf * reach, h];
                    let out = gamut_compress_fwd(jmh, &p);
                    let rgb = jmh_to_rgb(out, &p.limit_params);
                    for c in 0..3 {
                        assert!(
                            rgb[c] > -0.05 && rgb[c] < 1.20,
                            "h={h} J={j} M={}: rgb={rgb:?}",
                            jmh[1]
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn display_round_trip_inverse_then_forward() {
        // The inverse direction is specified against display code values:
        // decode → inverse core → forward core → encode must return the
        // starting code value for in-gamut colors. Bound measured on this
        // implementation (analytically-approximated stages): max abs code
        // value error observed ~2e-5 on this grid; pinned at 1e-4.
        let ot = SdrOutputTransform::sdr_100nit_rec709_srgb();
        for r in [0.1, 0.35, 0.7, 0.95] {
            for g in [0.08, 0.4, 0.85] {
                for b in [0.12, 0.5, 0.9] {
                    let cv = [r, g, b];
                    let aces = ot.inverse(cv);
                    let back = ot.forward(aces);
                    for c in 0..3 {
                        assert!(
                            (back[c] - cv[c]).abs() < 1e-4,
                            "cv {cv:?} → aces {aces:?} → {back:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn scene_round_trip_forward_then_inverse() {
        // Forward → inverse on moderate in-gamut scene values, compared in
        // scene space. Honest bound: the chroma/gamut inverse pair is an
        // approximate analytical inverse; relative error stays below 1e-3
        // on this grid (measured; typical values are ~1e-6).
        let ot = SdrOutputTransform::sdr_100nit_rec709_srgb();
        for x in [0.05, 0.18, 0.5] {
            for (dr, dg, db) in [(1.0, 1.0, 1.0), (1.2, 1.0, 0.8), (0.9, 1.1, 1.0)] {
                let aces = [x * dr, x * dg, x * db];
                let cv = ot.forward(aces);
                let back = ot.inverse(cv);
                for c in 0..3 {
                    let rel = ((back[c] - aces[c]) / aces[c].max(1e-6)).abs();
                    assert!(rel < 1e-3, "aces {aces:?} → {back:?} (rel {rel})");
                }
            }
        }
    }

    #[test]
    fn srgb_and_bt1886_presets_share_display_linear() {
        // Same rendering, different transfer: decoding each preset's
        // output through its own EOTF must give the same display-linear
        // values.
        let srgb = SdrOutputTransform::sdr_100nit_rec709_srgb();
        let bt = SdrOutputTransform::sdr_100nit_rec709_bt1886();
        for aces in [[0.18, 0.18, 0.18], [0.4, 0.1, 0.05], [0.02, 0.3, 0.6]] {
            let a = display_decoding(
                srgb.forward(aces),
                &MATRIX_IDENTITY,
                Eotf::SrgbIec61966,
                1.0,
            );
            let b = display_decoding(bt.forward(aces), &MATRIX_IDENTITY, Eotf::Bt1886, 1.0);
            for c in 0..3 {
                assert!(
                    (a[c] - b[c]).abs() < 1e-9,
                    "display linear diverges: {a:?} vs {b:?}"
                );
            }
        }
    }

    #[test]
    fn determinism_two_run_bit_hash_and_golden() {
        // Two full rebuilds (parameter tables included) must be
        // bit-identical, and the fixture hash must match the pinned golden
        // (pinned from this implementation's own run after the oracle
        // tests above were green — rule 8).
        let run = || -> u64 {
            let ot = SdrOutputTransform::sdr_100nit_rec709_srgb();
            let mut flat = Vec::new();
            for px in hdr_fixture() {
                let out = ot.forward(px);
                flat.extend_from_slice(&out);
            }
            fnv1a_f64(&flat)
        };
        let h1 = run();
        let h2 = run();
        assert_eq!(h1, h2, "two-run bit-hash mismatch");
        // Pinned 2026-08-13 from this implementation's own run with all
        // oracle tests green (rule 8 — never hand-invented).
        const GOLDEN_FWD_SRGB_100NIT: u64 = 0x7ba94a941b41cb19;
        assert_eq!(
            h1, GOLDEN_FWD_SRGB_100NIT,
            "pinned golden mismatch: got {h1:#018x}"
        );
    }

    #[test]
    fn determinism_param_tables_bit_identical() {
        let a = init_odt_params(100.0, &REC709_PRI);
        let b = init_odt_params(100.0, &REC709_PRI);
        let mut flat_a = Vec::new();
        let mut flat_b = Vec::new();
        flat_a.extend_from_slice(&a.table_reach_m);
        flat_b.extend_from_slice(&b.table_reach_m);
        flat_a.extend_from_slice(&a.table_hues);
        flat_b.extend_from_slice(&b.table_hues);
        flat_a.extend_from_slice(&a.table_upper_hull_gamma);
        flat_b.extend_from_slice(&b.table_upper_hull_gamma);
        for i in 0..TOTAL_TABLE_SIZE {
            flat_a.extend_from_slice(&a.table_gamut_cusps[i]);
            flat_b.extend_from_slice(&b.table_gamut_cusps[i]);
        }
        assert_eq!(fnv1a_f64(&flat_a), fnv1a_f64(&flat_b));
    }
}
