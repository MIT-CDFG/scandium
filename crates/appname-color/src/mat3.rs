// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §5.5 (working-space matrix math; "the D50→D65 direction is the
//! matrix inverse, computed — never a second hand-typed constant"),
//! Appendix B (the normative constants these operate on).
//!
//! A small row-major 3×3 over `f64`. Deliberately local (this crate sits
//! below `appname-core` in the DAG and takes no `appname-*` or external
//! deps in slice 1). All derivation math is f64 with fixed evaluation
//! order — same inputs, same bits, every run (D-48); results are narrowed
//! to f32 only at the pixel boundary.

/// Row-major 3×3 matrix of `f64`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Mat3 {
    /// `m[row][col]`.
    pub m: [[f64; 3]; 3],
}

impl Mat3 {
    pub const IDENTITY: Mat3 = Mat3 {
        m: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
    };

    pub const fn new(m: [[f64; 3]; 3]) -> Mat3 {
        Mat3 { m }
    }

    /// Diagonal matrix from a 3-vector.
    pub const fn diag(d: [f64; 3]) -> Mat3 {
        Mat3 {
            m: [[d[0], 0.0, 0.0], [0.0, d[1], 0.0], [0.0, 0.0, d[2]]],
        }
    }

    /// `self · rhs` (fixed accumulation order: k = 0, 1, 2).
    pub fn mul(&self, rhs: &Mat3) -> Mat3 {
        let mut out = [[0.0f64; 3]; 3];
        for (r, out_row) in out.iter_mut().enumerate() {
            for (c, out_cell) in out_row.iter_mut().enumerate() {
                // Explicit order, no fastmath reassociation: a + b + c
                // evaluated left to right.
                *out_cell = self.m[r][0] * rhs.m[0][c]
                    + self.m[r][1] * rhs.m[1][c]
                    + self.m[r][2] * rhs.m[2][c];
            }
        }
        Mat3 { m: out }
    }

    /// `self · v` (column vector), same fixed order.
    pub fn mul_vec(&self, v: [f64; 3]) -> [f64; 3] {
        let mut out = [0.0f64; 3];
        for (r, out_cell) in out.iter_mut().enumerate() {
            *out_cell = self.m[r][0] * v[0] + self.m[r][1] * v[1] + self.m[r][2] * v[2];
        }
        out
    }

    /// Closed-form adjugate inverse. `None` when the determinant is exactly
    /// zero or non-finite (never expected for the Appendix B matrices; the
    /// caller treats it as a structural error, not a panic).
    pub fn inverse(&self) -> Option<Mat3> {
        let m = &self.m;
        let c00 = m[1][1] * m[2][2] - m[1][2] * m[2][1];
        let c01 = m[1][2] * m[2][0] - m[1][0] * m[2][2];
        let c02 = m[1][0] * m[2][1] - m[1][1] * m[2][0];
        let det = m[0][0] * c00 + m[0][1] * c01 + m[0][2] * c02;
        if det == 0.0 || !det.is_finite() {
            return None;
        }
        let inv_det = 1.0 / det;
        Some(Mat3 {
            m: [
                [
                    c00 * inv_det,
                    (m[0][2] * m[2][1] - m[0][1] * m[2][2]) * inv_det,
                    (m[0][1] * m[1][2] - m[0][2] * m[1][1]) * inv_det,
                ],
                [
                    c01 * inv_det,
                    (m[0][0] * m[2][2] - m[0][2] * m[2][0]) * inv_det,
                    (m[0][2] * m[1][0] - m[0][0] * m[1][2]) * inv_det,
                ],
                [
                    c02 * inv_det,
                    (m[0][1] * m[2][0] - m[0][0] * m[2][1]) * inv_det,
                    (m[0][0] * m[1][1] - m[0][1] * m[1][0]) * inv_det,
                ],
            ],
        })
    }

    /// Largest absolute element-wise difference (test/oracle helper).
    pub fn max_abs_diff(&self, other: &Mat3) -> f64 {
        let mut worst = 0.0f64;
        for r in 0..3 {
            for c in 0..3 {
                let d = (self.m[r][c] - other.m[r][c]).abs();
                if d > worst {
                    worst = d;
                }
            }
        }
        worst
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_round_trip() {
        let a = Mat3::new([[2.0, 1.0, 0.5], [0.0, 3.0, -1.0], [4.0, 0.0, 1.0]]);
        let inv = a.inverse().unwrap();
        let prod = a.mul(&inv);
        assert!(prod.max_abs_diff(&Mat3::IDENTITY) < 1e-14, "{prod:?}");
        let prod2 = inv.mul(&a);
        assert!(prod2.max_abs_diff(&Mat3::IDENTITY) < 1e-14, "{prod2:?}");
    }

    #[test]
    fn singular_matrix_refused_without_panic() {
        let s = Mat3::new([[1.0, 2.0, 3.0], [2.0, 4.0, 6.0], [0.0, 0.0, 1.0]]);
        assert!(s.inverse().is_none());
    }

    #[test]
    fn mul_vec_matches_mul() {
        let a = Mat3::new([[0.5, 0.25, 0.0], [0.1, 0.9, 0.0], [0.0, 0.2, 0.8]]);
        let v = [0.3, 0.6, 0.9];
        let direct = a.mul_vec(v);
        let via_diag = a.mul(&Mat3::diag([v[0], v[1], v[2]]));
        // column sums of via_diag rows equal direct components
        for (r, want) in direct.iter().enumerate() {
            let s = via_diag.m[r][0] + via_diag.m[r][1] + via_diag.m[r][2];
            assert!((s - want).abs() < 1e-15);
        }
    }

    #[test]
    fn derivation_is_bit_deterministic() {
        // Same inputs, same bits (D-48): repeat the exact composition and
        // compare bit patterns, not tolerances.
        let a = Mat3::new([
            [0.8951, 0.2664, -0.1614],
            [-0.7502, 1.7135, 0.0367],
            [0.0389, -0.0685, 1.0296],
        ]);
        let first = a
            .inverse()
            .unwrap()
            .mul(&a.mul(&Mat3::diag([1.02, 0.98, 0.76])));
        let second = a
            .inverse()
            .unwrap()
            .mul(&a.mul(&Mat3::diag([1.02, 0.98, 0.76])));
        for r in 0..3 {
            for c in 0..3 {
                assert_eq!(first.m[r][c].to_bits(), second.m[r][c].to_bits());
            }
        }
    }
}
