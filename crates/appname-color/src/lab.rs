// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §5.5 (XYZ→Lab with the document white point; ΔE roles: "CIE76
//! for warnings, CIEDE2000 for tests"), §5.2 (Lab crossings use float
//! formats only; the normalized-storage ↔ real-valued Lab marshalling
//! formulas), Appendix B (the `f(t)` piecewise definition, δ = 6/29).
//!
//! Lab conversion groundwork (§2.3 row "Lab conversions groundwork"). The
//! CIEDE2000 test metric lands with the acceptance-suite slice (it is only
//! consumed by §5.8 tests that need the ICC backend anyway); ΔE76 is here
//! because the §5.6 gamut flag and §5.5 warnings are defined on it.

use crate::chromatic::WhitePoint;

/// δ = 6/29 (Appendix B).
const DELTA: f64 = 6.0 / 29.0;

/// Appendix B: `f(t) = t > δ³ ? t^(1/3) : t/(3δ²) + 4/29`.
fn f_lab(t: f64) -> f64 {
    if t > DELTA * DELTA * DELTA {
        t.cbrt()
    } else {
        t / (3.0 * DELTA * DELTA) + 4.0 / 29.0
    }
}

/// Inverse of [`f_lab`]: `t = f > δ ? f³ : 3δ²·(f − 4/29)`.
fn f_lab_inv(f: f64) -> f64 {
    if f > DELTA {
        f * f * f
    } else {
        3.0 * DELTA * DELTA * (f - 4.0 / 29.0)
    }
}

/// XYZ (relative to `white`, Y of white = 1) → CIE L*a*b* (ES §5.5).
pub fn xyz_to_lab(xyz: [f64; 3], white: WhitePoint) -> [f64; 3] {
    let wp = white.xyz();
    let fx = f_lab(xyz[0] / wp[0]);
    let fy = f_lab(xyz[1] / wp[1]);
    let fz = f_lab(xyz[2] / wp[2]);
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// CIE L*a*b* → XYZ relative to `white` (computed inverse of the §5.5
/// forward formulas).
pub fn lab_to_xyz(lab: [f64; 3], white: WhitePoint) -> [f64; 3] {
    let wp = white.xyz();
    let fy = (lab[0] + 16.0) / 116.0;
    let fx = fy + lab[1] / 500.0;
    let fz = fy - lab[2] / 200.0;
    [
        wp[0] * f_lab_inv(fx),
        wp[1] * f_lab_inv(fy),
        wp[2] * f_lab_inv(fz),
    ]
}

/// ΔE76 — Euclidean distance in Lab (ES §5.5: the warnings/gamut-flag
/// metric; §5.6 pins the out-of-gamut threshold at ΔE76 = 2.0).
pub fn delta_e76(a: [f64; 3], b: [f64; 3]) -> f64 {
    let dl = a[0] - b[0];
    let da = a[1] - b[1];
    let db = a[2] - b[2];
    (dl * dl + da * da + db * db).sqrt()
}

/// Normalized Lab storage → real-valued Lab (ES §5.2 marshalling:
/// `L = 100·v₀`, `a = 255·v₁ − 128`, `b = 255·v₂ − 128`).
pub fn lab_from_normalized(v: [f64; 3]) -> [f64; 3] {
    [100.0 * v[0], 255.0 * v[1] - 128.0, 255.0 * v[2] - 128.0]
}

/// Real-valued Lab → normalized storage (inverse of the §5.2 formulas).
pub fn lab_to_normalized(lab: [f64; 3]) -> [f64; 3] {
    [
        lab[0] / 100.0,
        (lab[1] + 128.0) / 255.0,
        (lab[2] + 128.0) / 255.0,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn white_maps_to_l100_a0_b0() {
        for w in [WhitePoint::D50, WhitePoint::D65] {
            let lab = xyz_to_lab(w.xyz(), w);
            assert!((lab[0] - 100.0).abs() < 1e-10, "{lab:?}");
            assert!(lab[1].abs() < 1e-10 && lab[2].abs() < 1e-10, "{lab:?}");
        }
    }

    #[test]
    fn black_maps_to_l0() {
        let lab = xyz_to_lab([0.0, 0.0, 0.0], WhitePoint::D50);
        assert_eq!(lab, [0.0, 0.0, 0.0]);
    }

    /// 18% gray: L* = 116·f(0.18) − 16; f(0.18) = 0.18^(1/3) on the cube
    /// branch — evaluates the spec formula against an independent
    /// spot-computation of the same constants.
    #[test]
    fn mid_gray_l_star() {
        let lab = xyz_to_lab([0.96422 * 0.18, 0.18, 0.82521 * 0.18], WhitePoint::D50);
        let expect_l = 116.0 * 0.18f64.cbrt() - 16.0;
        assert!((lab[0] - expect_l).abs() < 1e-10);
        assert!(
            lab[1].abs() < 1e-9 && lab[2].abs() < 1e-9,
            "neutral axis: {lab:?}"
        );
    }

    #[test]
    fn f_pair_is_continuous_at_the_break() {
        let t = DELTA * DELTA * DELTA;
        assert!((f_lab(t) - f_lab(t + 1e-12)).abs() < 1e-9);
        assert!((f_lab_inv(DELTA) - f_lab_inv(DELTA + 1e-12)).abs() < 1e-9);
    }

    /// Round trip across both branches of the piecewise pair, fixed seeds.
    #[test]
    fn xyz_lab_round_trip() {
        // Deterministic LCG (MMIX constants) — fixed seed per plan test rule.
        let mut state: u64 = SEED;
        for _ in 0..2000 {
            let x = next01(&mut state) * 1.1;
            let y = next01(&mut state) * 1.05;
            let z = next01(&mut state) * 1.1;
            for w in [WhitePoint::D50, WhitePoint::D65] {
                let lab = xyz_to_lab([x, y, z], w);
                let back = lab_to_xyz(lab, w);
                for (orig, rt) in [x, y, z].iter().zip(back.iter()) {
                    assert!((orig - rt).abs() < 1e-12, "{orig} vs {rt}");
                }
            }
        }
    }

    #[test]
    fn normalized_marshalling_round_trips() {
        let mut state: u64 = SEED;
        for _ in 0..1000 {
            let v = [next01(&mut state), next01(&mut state), next01(&mut state)];
            let back = lab_to_normalized(lab_from_normalized(v));
            for i in 0..3 {
                assert!((v[i] - back[i]).abs() < 1e-13);
            }
        }
        // Endpoint checks of the §5.2 formulas.
        assert_eq!(lab_from_normalized([1.0, 0.0, 1.0]), [100.0, -128.0, 127.0]);
        assert_eq!(
            lab_from_normalized([0.0, 128.0 / 255.0, 128.0 / 255.0])[0],
            0.0
        );
    }

    #[test]
    fn delta_e76_axioms() {
        let a = [50.0, 10.0, -10.0];
        let b = [55.0, 10.0, -10.0];
        assert_eq!(delta_e76(a, a), 0.0);
        assert_eq!(delta_e76(a, b), 5.0);
        assert_eq!(delta_e76(a, b), delta_e76(b, a));
    }

    // ---- deterministic test RNG (fixed seed, MMIX LCG constants) ----
    const SEED: u64 = 0x5eed_c010;
    fn next01(state: &mut u64) -> f64 {
        *state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        // top 53 bits → [0, 1)
        ((*state >> 11) as f64) / ((1u64 << 53) as f64)
    }
}
