// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Derived by translation from ACES Core Transforms (aces-dev),
// https://github.com/ampas/aces-dev, commit
// 5ac93ba2d48a00aca6e4811fa0b700911c2a8809 (census key `aces-dev`):
// lib/Lib.Academy.ColorSpaces.ctl. Apache-2.0 at the pin (LICENSE sha256
// 43070e2d4e532684de521b885f385d0841030efa2b1a20bafb76133a5e1379c1, read
// first-hand 2026-08-13). Per Apache-2.0 Section 4(c) the upstream per-file
// notice "Copyright Contributors to the ACES Project." is retained in this
// header; repo-level notice carriage per the `appname.port-record.v1` draft
// (reports/O-ALGO-log.md, Item 11; O-BUILD ratification pending).

//! Color-space conversion helpers: xyY↔XYZ, Von Kries chromatic adaptation
//! (Bradford cone primaries), and RGB→RGB matrix assembly.
//!
//! Skipped from Lib.Academy.ColorSpaces.ctl (no consumer in the ported
//! path): `CONE_RESP_MAT_CAT02` (Bradford is the default and only response
//! set the SDR transforms use).

use super::utilities::{
    invert_f33, mult_f33_f33, mult_f3_f33, rgb_to_xyz_f33, xyz_to_rgb_f33, Chromaticities, M33,
};

// from Lib.Academy.ColorSpaces.ctl — CIE XYZ tristimulus → CIE x,y,Y.
pub fn xyz_2_xyy(xyz: [f64; 3]) -> [f64; 3] {
    let mut divisor = xyz[0] + xyz[1] + xyz[2];
    if divisor == 0.0 {
        divisor = 1e-10;
    }
    [xyz[0] / divisor, xyz[1] / divisor, xyz[1]]
}

// from Lib.Academy.ColorSpaces.ctl — CIE x,y,Y → CIE XYZ tristimulus.
pub fn xyy_2_xyz(xyy: [f64; 3]) -> [f64; 3] {
    let denom = if xyy[1] > 1e-10 { xyy[1] } else { 1e-10 };
    [
        xyy[0] * xyy[2] / denom,
        xyy[2],
        (1.0 - xyy[0] - xyy[1]) * xyy[2] / denom,
    ]
}

// from Lib.Academy.ColorSpaces.ctl
pub const CONE_RESP_MAT_BRADFORD: M33 = [
    [0.89510, -0.75020, 0.03890],
    [0.26640, 1.71350, -0.06850],
    [-0.16140, 0.03670, 1.02960],
];

// from Lib.Academy.ColorSpaces.ctl — 3x3 Von Kries chromatic adaptation
// transform from src_xy to des_xy using the given cone response primaries
// (Bradford by default upstream; passed explicitly here).
pub fn calculate_cat_matrix(src_xy: [f64; 2], des_xy: [f64; 2], cone_resp_mat: &M33) -> M33 {
    let src_xyy = [src_xy[0], src_xy[1], 1.0];
    let des_xyy = [des_xy[0], des_xy[1], 1.0];

    let src_xyz = xyy_2_xyz(src_xyy);
    let des_xyz = xyy_2_xyz(des_xyy);

    let src_cone_resp = mult_f3_f33(src_xyz, cone_resp_mat);
    let des_cone_resp = mult_f3_f33(des_xyz, cone_resp_mat);

    let vk_mat: M33 = [
        [des_cone_resp[0] / src_cone_resp[0], 0.0, 0.0],
        [0.0, des_cone_resp[1] / src_cone_resp[1], 0.0],
        [0.0, 0.0, des_cone_resp[2] / src_cone_resp[2]],
    ];

    mult_f33_f33(
        cone_resp_mat,
        &mult_f33_f33(&vk_mat, &invert_f33(cone_resp_mat)),
    )
}

// from Lib.Academy.ColorSpaces.ctl — RGB→RGB matrix between primary sets:
// source RGB → XYZ → chromatic adaptation (source white → destination
// white) → XYZ → destination RGB. Upstream slices its 4x4 standard-library
// `RGBtoXYZ`/`XYZtoRGB` down to 3x3; the 3x3 builders here are identical to
// that upper-left block.
pub fn calculate_rgb_to_rgb_matrix(
    source_primaries: &Chromaticities,
    dest_primaries: &Chromaticities,
    cone_resp_mat: &M33,
) -> M33 {
    let rgb_to_xyz_mat = rgb_to_xyz_f33(source_primaries, 1.0);

    // Chromatic adaptation from source white to destination white
    let cat = calculate_cat_matrix(source_primaries.white, dest_primaries.white, cone_resp_mat);

    let xyz_to_rgb_mat = xyz_to_rgb_f33(dest_primaries, 1.0);

    mult_f33_f33(&rgb_to_xyz_mat, &mult_f33_f33(&cat, &xyz_to_rgb_mat))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aces::output_transform::{AP0, AP1};

    #[test]
    fn xyy_xyz_round_trip() {
        let xyz = [0.4124, 0.3576, 0.1805];
        let back = xyy_2_xyz(xyz_2_xyy(xyz));
        for i in 0..3 {
            assert!((back[i] - xyz[i]).abs() < 1e-12);
        }
    }

    #[test]
    fn cat_matrix_identity_for_same_white() {
        // Adapting a white point to itself must be the identity transform.
        let d65 = [0.3127, 0.3290];
        let m = calculate_cat_matrix(d65, d65, &CONE_RESP_MAT_BRADFORD);
        // Index loop kept: the assertion references m[i][j] by index pair.
        #[allow(clippy::needless_range_loop)]
        for i in 0..3 {
            for j in 0..3 {
                let expect = if i == j { 1.0 } else { 0.0 };
                assert!((m[i][j] - expect).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn cat_matrix_round_trip() {
        // ACES white (~D60) → D65 → back must compose to identity.
        let aces_white = AP0.white;
        let d65 = [0.3127, 0.3290];
        let fwd = calculate_cat_matrix(aces_white, d65, &CONE_RESP_MAT_BRADFORD);
        let inv = calculate_cat_matrix(d65, aces_white, &CONE_RESP_MAT_BRADFORD);
        let prod = mult_f33_f33(&fwd, &inv);
        // Index loop kept: the assertion references prod[i][j] by index pair.
        #[allow(clippy::needless_range_loop)]
        for i in 0..3 {
            for j in 0..3 {
                let expect = if i == j { 1.0 } else { 0.0 };
                assert!((prod[i][j] - expect).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn rgb_to_rgb_matrix_ap0_ap1_round_trip() {
        // AP0 and AP1 share the ACES white, so the CAT leg is identity and
        // the AP0→AP1→AP0 composition must round-trip.
        let fwd = calculate_rgb_to_rgb_matrix(&AP0, &AP1, &CONE_RESP_MAT_BRADFORD);
        let inv = calculate_rgb_to_rgb_matrix(&AP1, &AP0, &CONE_RESP_MAT_BRADFORD);
        let v = [0.18, 0.42, 0.07];
        let back = mult_f3_f33(mult_f3_f33(v, &fwd), &inv);
        for i in 0..3 {
            assert!((back[i] - v[i]).abs() < 1e-12);
        }
    }
}
