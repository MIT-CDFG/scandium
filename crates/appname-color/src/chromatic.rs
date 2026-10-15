// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §5.5 "Bradford adaptation D65→D50 — the evaluated matrix
//! [NORMATIVE]" (the 7-decimal matrix and the two white points), Appendix B
//! (the Bradford cone matrix `M_A` and the construction
//! `M = M_A⁻¹ · diag(ρ_D50/ρ_D65) · M_A`).
//!
//! Constants policy: every hand-typed number in this file is copied from a
//! spec table (§5.5 / Appendix B) — nothing is re-derived from outside
//! sources. Derived directions (D50→D65, composites) are COMPUTED from the
//! normative parts, per the §5.5 rule "the D50→D65 direction is the matrix
//! inverse, computed — never a second hand-typed constant."

use crate::mat3::Mat3;

/// White point tags for the bundled spaces (values below are the §5.5
/// evaluation white points).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum WhitePoint {
    D50,
    D65,
}

impl WhitePoint {
    /// XYZ of the white point, Y = 1 (ES §5.5: D65 = (0.95047, 1.00000,
    /// 1.08883), D50 = (0.96422, 1.00000, 0.82521)).
    pub const fn xyz(self) -> [f64; 3] {
        match self {
            WhitePoint::D50 => [0.96422, 1.00000, 0.82521],
            WhitePoint::D65 => [0.95047, 1.00000, 1.08883],
        }
    }
}

/// Bradford cone response matrix `M_A` (Appendix B, verbatim).
pub const BRADFORD_MA: Mat3 = Mat3::new([
    [0.8951, 0.2664, -0.1614],
    [-0.7502, 1.7135, 0.0367],
    [0.0389, -0.0685, 1.0296],
]);

/// The evaluated `XYZ(D65) → XYZ(D50)` Bradford matrix (ES §5.5, verbatim,
/// 7 decimals — [NORMATIVE]).
pub const BRADFORD_D65_TO_D50: Mat3 = Mat3::new([
    [1.0478112, 0.0228866, -0.0501270],
    [0.0295424, 0.9904844, -0.0170491],
    [-0.0092345, 0.0150436, 0.7521316],
]);

/// `XYZ(D50) → XYZ(D65)`: the computed inverse of the normative matrix
/// (§5.5 rule — never a second hand-typed constant).
///
/// Infallible in practice: [`BRADFORD_D65_TO_D50`] is well-conditioned; the
/// `unwrap_or(IDENTITY)` arm is unreachable and exists only to keep this
/// function panic-free (§1.6).
pub fn bradford_d50_to_d65() -> Mat3 {
    BRADFORD_D65_TO_D50.inverse().unwrap_or(Mat3::IDENTITY)
}

/// Appendix B construction `M = M_A⁻¹ · diag(ρ_dst/ρ_src) · M_A` for
/// arbitrary white pairs. Used by tests to cross-verify the §5.5 evaluated
/// matrix, and by later slices for adapting user-supplied matrix profiles.
pub fn bradford_adaptation(src: WhitePoint, dst: WhitePoint) -> Mat3 {
    if src == dst {
        return Mat3::IDENTITY;
    }
    let rho_src = BRADFORD_MA.mul_vec(src.xyz());
    let rho_dst = BRADFORD_MA.mul_vec(dst.xyz());
    let gain = Mat3::diag([
        rho_dst[0] / rho_src[0],
        rho_dst[1] / rho_src[1],
        rho_dst[2] / rho_src[2],
    ]);
    // M_A is constant and invertible; identity fallback is unreachable.
    let ma_inv = BRADFORD_MA.inverse().unwrap_or(Mat3::IDENTITY);
    ma_inv.mul(&gain.mul(&BRADFORD_MA))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The §5.5-mandated verification, verbatim: "a unit test in
    /// `appname-color`: M · (0.95047, 1.0, 1.08883)ᵀ = (0.96422, 1.0,
    /// 0.82521)ᵀ exactly to 5 decimals."
    #[test]
    fn normative_matrix_maps_d65_white_to_d50_white() {
        let got = BRADFORD_D65_TO_D50.mul_vec(WhitePoint::D65.xyz());
        let want = WhitePoint::D50.xyz();
        for i in 0..3 {
            let rounded = (got[i] * 1e5).round() / 1e5;
            assert_eq!(rounded, want[i], "component {i}: {} vs {}", got[i], want[i]);
        }
    }

    /// Cross-check: the Appendix B construction reproduces the §5.5
    /// evaluated matrix within its 7-decimal rounding (≤ 5e-8 per element,
    /// tested at 1e-7 for rounding-boundary slack).
    #[test]
    fn construction_reproduces_the_evaluated_matrix() {
        let built = bradford_adaptation(WhitePoint::D65, WhitePoint::D50);
        let diff = built.max_abs_diff(&BRADFORD_D65_TO_D50);
        assert!(diff < 1e-7, "max element diff {diff}");
    }

    #[test]
    fn inverse_direction_is_computed_and_round_trips() {
        let fwd = BRADFORD_D65_TO_D50;
        let back = bradford_d50_to_d65();
        let rt = back.mul(&fwd);
        assert!(rt.max_abs_diff(&Mat3::IDENTITY) < 1e-12);
        // And it carries D50 white back to D65 white.
        let got = back.mul_vec(WhitePoint::D50.xyz());
        let want = WhitePoint::D65.xyz();
        for i in 0..3 {
            assert!((got[i] - want[i]).abs() < 1e-6, "component {i}");
        }
    }

    #[test]
    fn same_white_adaptation_is_identity() {
        assert_eq!(
            bradford_adaptation(WhitePoint::D50, WhitePoint::D50),
            Mat3::IDENTITY
        );
    }

    #[test]
    fn adaptation_is_bit_deterministic() {
        let a = bradford_adaptation(WhitePoint::D65, WhitePoint::D50);
        let b = bradford_adaptation(WhitePoint::D65, WhitePoint::D50);
        for r in 0..3 {
            for c in 0..3 {
                assert_eq!(a.m[r][c].to_bits(), b.m[r][c].to_bits());
            }
        }
    }
}
