// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Derived by translation from ACES Core Transforms (aces-dev),
// https://github.com/ampas/aces-dev, commit
// 5ac93ba2d48a00aca6e4811fa0b700911c2a8809 (census key `aces-dev`):
// lib/Lib.Academy.Utilities.ctl. Apache-2.0 at the pin (LICENSE sha256
// 43070e2d4e532684de521b885f385d0841030efa2b1a20bafb76133a5e1379c1, read
// first-hand 2026-08-13). Per Apache-2.0 Section 4(c) the upstream per-file
// notice "Copyright Contributors to the ACES Project." is retained in this
// header; repo-level notice carriage per the `appname.port-record.v1` draft
// (reports/O-ALGO-log.md, Item 11; O-BUILD ratification pending).

//! Utility scalar/matrix helpers for the ACES 2.0 output-transform port.
//!
//! Sections translated `// from Lib.Academy.Utilities.ctl`; the small
//! linear-algebra primitives at the bottom (`mult_f3_f33`, `mult_f33_f33`,
//! `mult_f_f3`, `mult_f_f33`, `invert_f33`, `Chromaticities`) are CTL
//! *standard-library* builtins that the lib files call — implemented here
//! from their standard mathematical definitions (row-vector convention:
//! `out = v * M`, i.e. `out[j] = Σ_i v[i]·M[i][j]`, matching how
//! `RGBtoXYZ_f33` matrices are laid out and consumed upstream).
//!
//! Numeric type: `f64` throughout (the CTL interpreter evaluates in double
//! precision; goldens are pinned from this implementation's own runs).

/// 3x3 matrix in the CTL row-vector convention.
pub type M33 = [[f64; 3]; 3];

/// Chromaticity set (CTL standard-library `Chromaticities` struct shape):
/// CIE x,y for the three primaries and the white point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chromaticities {
    pub red: [f64; 2],
    pub green: [f64; 2],
    pub blue: [f64; 2],
    pub white: [f64; 2],
}

// from Lib.Academy.Utilities.ctl — min/max via explicit comparison (CTL
// semantics; not f64::min/max, whose NaN behavior differs).
#[inline]
pub fn ctl_min(a: f64, b: f64) -> f64 {
    if a < b {
        a
    } else {
        b
    }
}

#[inline]
pub fn ctl_max(a: f64, b: f64) -> f64 {
    if a > b {
        a
    } else {
        b
    }
}

// from Lib.Academy.Utilities.ctl
#[inline]
pub fn clamp(v: f64, clamp_min: f64, clamp_max: f64) -> f64 {
    ctl_max(clamp_min, ctl_min(v, clamp_max))
}

// from Lib.Academy.Utilities.ctl
#[inline]
pub fn clamp_f3(v: [f64; 3], clamp_min: f64, clamp_max: f64) -> [f64; 3] {
    [
        clamp(v[0], clamp_min, clamp_max),
        clamp(v[1], clamp_min, clamp_max),
        clamp(v[2], clamp_min, clamp_max),
    ]
}

// from Lib.Academy.Utilities.ctl
#[inline]
pub fn f3_from_f(a: f64) -> [f64; 3] {
    [a, a, a]
}

// from Lib.Academy.Utilities.ctl — CTL `round`: adds ±0.5 then truncates
// toward zero via the int conversion (NOT Rust's `f64::round`, which is
// identical for the non-negative arguments used here but differs at ±0.5
// boundaries for negatives).
#[inline]
pub fn round_ctl(x: f64) -> f64 {
    let x1: i64 = if x < 0.0 {
        (x - 0.5) as i64
    } else {
        (x + 0.5) as i64
    };
    x1 as f64
}

// from Lib.Academy.Utilities.ctl — signum returning {-1, 0, 1}.
#[inline]
pub fn sign(x: f64) -> i32 {
    if x < 0.0 {
        -1
    } else if x > 0.0 {
        1
    } else {
        0
    }
}

// from Lib.Academy.Utilities.ctl — magnitude of x with the sign of y.
// NOTE: CTL's version returns 0 when y == 0 (sign(0) = 0), unlike
// `f64::copysign`; translated faithfully.
#[inline]
pub fn copysign_ctl(x: f64, y: f64) -> f64 {
    sign(y) as f64 * x.abs()
}

// from Lib.Academy.Utilities.ctl — linear interpolation with bias t.
#[inline]
pub fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + t * (b - a)
}

// from Lib.Academy.Utilities.ctl
#[inline]
pub fn lerp_f3(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    [
        lerp(a[0], b[0], t),
        lerp(a[1], b[1], t),
        lerp(a[2], b[2], t),
    ]
}

// from Lib.Academy.Utilities.ctl
#[inline]
pub fn radians_to_degrees(radians: f64) -> f64 {
    radians * 180.0 / std::f64::consts::PI
}

// from Lib.Academy.Utilities.ctl
#[inline]
pub fn degrees_to_radians(degrees: f64) -> f64 {
    degrees / 180.0 * std::f64::consts::PI
}

// from Lib.Academy.Utilities.ctl — B = A with the diagonal scaled by v.
#[inline]
pub fn scale_matrix_diagonal_f33_f3(a: M33, v: [f64; 3]) -> M33 {
    let mut b = a;
    b[0][0] = a[0][0] * v[0];
    b[1][1] = a[1][1] * v[1];
    b[2][2] = a[2][2] * v[2];
    b
}

// from Lib.Academy.Utilities.ctl — RGB→XYZ matrix from chromaticities,
// normalized so RGB (1,1,1) maps to the white point at luminance Y
// (row-vector convention: XYZ = rgb * M).
pub fn rgb_to_xyz_f33(c: &Chromaticities, y: f64) -> M33 {
    // X and Z values of RGB value (1, 1, 1), or "white"
    let x = c.white[0] * y / c.white[1];
    let z = (1.0 - c.white[0] - c.white[1]) * y / c.white[1];

    // Scale factors for matrix rows
    let d = c.red[0] * (c.blue[1] - c.green[1])
        + c.blue[0] * (c.green[1] - c.red[1])
        + c.green[0] * (c.red[1] - c.blue[1]);

    let sr = (x * (c.blue[1] - c.green[1])
        - c.green[0] * (y * (c.blue[1] - 1.0) + c.blue[1] * (x + z))
        + c.blue[0] * (y * (c.green[1] - 1.0) + c.green[1] * (x + z)))
        / d;

    let sg = (x * (c.red[1] - c.blue[1])
        + c.red[0] * (y * (c.blue[1] - 1.0) + c.blue[1] * (x + z))
        - c.blue[0] * (y * (c.red[1] - 1.0) + c.red[1] * (x + z)))
        / d;

    let sb = (x * (c.green[1] - c.red[1])
        - c.red[0] * (y * (c.green[1] - 1.0) + c.green[1] * (x + z))
        + c.green[0] * (y * (c.red[1] - 1.0) + c.red[1] * (x + z)))
        / d;

    [
        [
            sr * c.red[0],
            sr * c.red[1],
            sr * (1.0 - c.red[0] - c.red[1]),
        ],
        [
            sg * c.green[0],
            sg * c.green[1],
            sg * (1.0 - c.green[0] - c.green[1]),
        ],
        [
            sb * c.blue[0],
            sb * c.blue[1],
            sb * (1.0 - c.blue[0] - c.blue[1]),
        ],
    ]
}

// from Lib.Academy.Utilities.ctl
pub fn xyz_to_rgb_f33(c: &Chromaticities, y: f64) -> M33 {
    invert_f33(&rgb_to_xyz_f33(c, y))
}

/* ---- CTL standard-library builtins (own implementations of the
 *      standard definitions; row-vector convention) ---- */

/// `mult_f3_f33` — row vector times matrix: `out[j] = Σ_i v[i]·m[i][j]`.
#[inline]
pub fn mult_f3_f33(v: [f64; 3], m: &M33) -> [f64; 3] {
    [
        v[0] * m[0][0] + v[1] * m[1][0] + v[2] * m[2][0],
        v[0] * m[0][1] + v[1] * m[1][1] + v[2] * m[2][1],
        v[0] * m[0][2] + v[1] * m[1][2] + v[2] * m[2][2],
    ]
}

/// `mult_f33_f33` — matrix product `(A·B)[i][j] = Σ_k A[i][k]·B[k][j]`,
/// so `v · (A·B) = (v·A) · B` under the row-vector convention.
pub fn mult_f33_f33(a: &M33, b: &M33) -> M33 {
    let mut out = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            out[i][j] = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
    out
}

/// `mult_f_f3` — scalar times vector.
#[inline]
pub fn mult_f_f3(s: f64, v: [f64; 3]) -> [f64; 3] {
    [s * v[0], s * v[1], s * v[2]]
}

/// `mult_f_f33` — scalar times matrix.
pub fn mult_f_f33(s: f64, m: &M33) -> M33 {
    let mut out = *m;
    for row in out.iter_mut() {
        for v in row.iter_mut() {
            *v *= s;
        }
    }
    out
}

/// `invert_f33` — 3x3 inverse via the adjugate (standard closed form).
pub fn invert_f33(m: &M33) -> M33 {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    let inv_det = 1.0 / det;
    [
        [
            (m[1][1] * m[2][2] - m[1][2] * m[2][1]) * inv_det,
            (m[0][2] * m[2][1] - m[0][1] * m[2][2]) * inv_det,
            (m[0][1] * m[1][2] - m[0][2] * m[1][1]) * inv_det,
        ],
        [
            (m[1][2] * m[2][0] - m[1][0] * m[2][2]) * inv_det,
            (m[0][0] * m[2][2] - m[0][2] * m[2][0]) * inv_det,
            (m[0][2] * m[1][0] - m[0][0] * m[1][2]) * inv_det,
        ],
        [
            (m[1][0] * m[2][1] - m[1][1] * m[2][0]) * inv_det,
            (m[0][1] * m[2][0] - m[0][0] * m[2][1]) * inv_det,
            (m[0][0] * m[1][1] - m[0][1] * m[1][0]) * inv_det,
        ],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aces::output_transform::{AP0, AP1};

    const IDENT_TOL: f64 = 1e-12;

    fn assert_m33_close(a: &M33, b: &M33, tol: f64) {
        for i in 0..3 {
            for j in 0..3 {
                assert!(
                    (a[i][j] - b[i][j]).abs() < tol,
                    "[{i}][{j}]: {} vs {}",
                    a[i][j],
                    b[i][j]
                );
            }
        }
    }

    #[test]
    fn invert_f33_round_trips_primaries_matrices() {
        let ident: M33 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        for prims in [&AP0, &AP1] {
            let m = rgb_to_xyz_f33(prims, 1.0);
            let mi = invert_f33(&m);
            assert_m33_close(&mult_f33_f33(&m, &mi), &ident, IDENT_TOL);
            assert_m33_close(&mult_f33_f33(&mi, &m), &ident, IDENT_TOL);
        }
    }

    #[test]
    fn rgb_to_xyz_white_normalization() {
        // (1,1,1) must map to the white point's XYZ at Y = 1 by construction.
        let m = rgb_to_xyz_f33(&AP0, 1.0);
        let white = mult_f3_f33([1.0, 1.0, 1.0], &m);
        let wx = AP0.white[0];
        let wy = AP0.white[1];
        assert!((white[1] - 1.0).abs() < 1e-12);
        assert!((white[0] - wx / wy).abs() < 1e-12);
        assert!((white[2] - (1.0 - wx - wy) / wy).abs() < 1e-12);
    }

    #[test]
    fn ap0_to_xyz_matches_published_matrix() {
        // Anchor: the ACES2065-1 (AP0) RGB→XYZ matrix published in
        // Academy S-2014-004 / SMPTE ST 2065-1 (column-vector convention);
        // our row-vector convention matrix must equal its transpose.
        let published: M33 = [
            [0.9525523959, 0.0000000000, 0.0000936786],
            [0.3439664498, 0.7281660966, -0.0721325464],
            [0.0000000000, 0.0000000000, 1.0088251844],
        ];
        let ours = rgb_to_xyz_f33(&AP0, 1.0);
        for i in 0..3 {
            for j in 0..3 {
                assert!(
                    (ours[i][j] - published[j][i]).abs() < 1e-9,
                    "[{i}][{j}]: {} vs published^T {}",
                    ours[i][j],
                    published[j][i]
                );
            }
        }
    }

    #[test]
    fn ctl_copysign_zero_semantics() {
        // CTL copysign: sign(0) = 0, so copysign(x, 0) = 0 — unlike
        // f64::copysign. The CAM cone-response wrappers depend on this at
        // exactly v = 0 (result 0 either way, but pin the primitive).
        assert_eq!(copysign_ctl(3.5, 0.0), 0.0);
        assert_eq!(copysign_ctl(3.5, -2.0), -3.5);
        assert_eq!(copysign_ctl(-3.5, 2.0), 3.5);
    }

    #[test]
    fn ctl_round_truncation_semantics() {
        assert_eq!(round_ctl(2.5), 3.0);
        assert_eq!(round_ctl(2.49), 2.0);
        assert_eq!(round_ctl(0.0), 0.0);
        assert_eq!(round_ctl(-2.5), -3.0);
    }
}
