// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Derived by translation from ACES Core Transforms (aces-dev),
// https://github.com/ampas/aces-dev, commit
// 5ac93ba2d48a00aca6e4811fa0b700911c2a8809 (census key `aces-dev`):
// lib/Lib.Academy.OutputTransform.ctl. Apache-2.0 at the pin (LICENSE sha256
// 43070e2d4e532684de521b885f385d0841030efa2b1a20bafb76133a5e1379c1, read
// first-hand 2026-08-13). Per Apache-2.0 Section 4(c) the upstream per-file
// notice "Copyright Contributors to the ACES Project." is retained in this
// header; repo-level notice carriage per the `appname.port-record.v1` draft
// (reports/O-ALGO-log.md, Item 11; O-BUILD ratification pending).

//! The ACES 2.0 output transform core: AP0/AP1 primaries and matrices, the
//! CAM16-derived JMh appearance conversion (Hellwig2022-model lineage per
//! the upstream comments), in-gamut chroma compression, hue-table-driven
//! gamut compression, table pre-computation, and the forward/inverse
//! transform drivers.
//!
//! Every function notes its origin (`// from Lib.Academy.OutputTransform.ctl`
//! throughout this file). Skipped from that file (no consumer in the ported
//! path): `hueDependentUpperHullGamma` (defined but unused by the lib at the
//! pin — table lookup happens via `init_hue_dependent_gamut_params`).
//! Deterministic by construction: pure scalar code, fixed iteration order,
//! no RNG, no threading (D-48).

use super::tonescale::{init_ts_params, tonescale_fwd, tonescale_inv, TsParams};
use super::utilities::{
    clamp_f3, copysign_ctl, ctl_max, ctl_min, degrees_to_radians, f3_from_f, invert_f33, lerp,
    lerp_f3, mult_f33_f33, mult_f3_f33, mult_f_f3, mult_f_f33, radians_to_degrees, round_ctl,
    scale_matrix_diagonal_f33_f3, xyz_to_rgb_f33, Chromaticities, M33,
};

// Chromaticities & Conversion matrices
// from Lib.Academy.OutputTransform.ctl — Academy Primaries 0 (the ACES2065-1
// primaries from SMPTE ST 2065-1).
pub const AP0: Chromaticities = Chromaticities {
    red: [0.73470, 0.26530],
    green: [0.00000, 1.00000],
    blue: [0.00010, -0.07700],
    white: [0.32168, 0.33767],
};

// from Lib.Academy.OutputTransform.ctl — Academy Primaries 1.
pub const AP1: Chromaticities = Chromaticities {
    red: [0.713, 0.293],
    green: [0.165, 0.830],
    blue: [0.128, 0.044],
    white: [0.32168, 0.33767],
};

// "Reach" Primaries - equal to AP1 (from Lib.Academy.OutputTransform.ctl).
pub const REACH_PRI: Chromaticities = AP1;

// Table generation constants (from Lib.Academy.OutputTransform.ctl).
pub const TABLE_SIZE: usize = 360;
pub const ADDITIONAL_TABLE_ENTRIES: usize = 2; // allots for extra entries to wrap the hues
pub const TOTAL_TABLE_SIZE: usize = TABLE_SIZE + ADDITIONAL_TABLE_ENTRIES;
pub const BASE_INDEX: usize = 1; // array index for smallest filled entry of padded table

const HUE_LIMIT: f64 = 360.0;

const CUSP_CORNER_COUNT: usize = 6;
const TOTAL_CORNER_COUNT: usize = CUSP_CORNER_COUNT + 2;
const MAX_SORTED_CORNERS: usize = 2 * CUSP_CORNER_COUNT;
const REACH_CUSP_TOLERANCE: f64 = 1e-3;
const DISPLAY_CUSP_TOLERANCE: f64 = 1e-7;

const GAMMA_MINIMUM: f64 = 0.0;
const GAMMA_MAXIMUM: f64 = 5.0;
const GAMMA_SEARCH_STEP: f64 = 0.4;
const GAMMA_ACCURACY: f64 = 1e-5;

// CAM Parameters (from Lib.Academy.OutputTransform.ctl).
pub const REF_LUMINANCE: f64 = 100.0;
const L_A: f64 = 100.0;
const Y_B: f64 = 20.0;
const SURROUND: [f64; 3] = [0.9, 0.59, 0.9]; // Dim surround

const J_SCALE: f64 = 100.0;
const CAM_NL_Y_REFERENCE: f64 = 100.0;
const CAM_NL_OFFSET: f64 = 0.2713 * CAM_NL_Y_REFERENCE;
const CAM_NL_SCALE: f64 = 4.0 * CAM_NL_Y_REFERENCE;

// `model_gamma` needs a sqrt, so it is a function rather than a const.
fn model_gamma() -> f64 {
    SURROUND[1] * (1.48 + (Y_B / REF_LUMINANCE).sqrt())
}

// Chroma compression (from Lib.Academy.OutputTransform.ctl).
const CHROMA_COMPRESS: f64 = 2.4;
const CHROMA_COMPRESS_FACT: f64 = 3.3;
const CHROMA_EXPAND: f64 = 1.3;
const CHROMA_EXPAND_FACT: f64 = 0.69;
const CHROMA_EXPAND_THR: f64 = 0.5;

// Gamut compression (from Lib.Academy.OutputTransform.ctl).
const SMOOTH_CUSPS: f64 = 0.12;
const SMOOTH_M: f64 = 0.27;
const CUSP_MID_BLEND: f64 = 1.3;

const FOCUS_GAIN_BLEND: f64 = 0.3;
#[allow(dead_code)] // upstream declares it; only the blend/distance pair is consumed at the pin
const FOCUS_ADJUST_GAIN: f64 = 0.55;
const FOCUS_DISTANCE: f64 = 1.35;
const FOCUS_DISTANCE_SCALING: f64 = 1.75;

const COMPRESSION_THRESHOLD: f64 = 0.75;

const MATRIX_IDENTITY: M33 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// Pre-computed conversion matrices and constants for conversions to/from
/// JMh. from Lib.Academy.OutputTransform.ctl `JMhParams`.
#[derive(Clone, Copy, Debug)]
pub struct JMhParams {
    pub matrix_rgb_to_cam16_c: M33,
    pub matrix_cam16_c_to_rgb: M33,
    pub matrix_cone_response_to_aab: M33,
    pub matrix_aab_to_cone_response: M33,
    pub f_l_n: f64, // F_L normalised
    pub cz: f64,
    pub inv_cz: f64, // 1/cz
    pub a_w_j: f64,
    pub inv_a_w_j: f64, // 1/A_w_J
}

/// Hue-dependent gamut parameters.
/// from Lib.Academy.OutputTransform.ctl `HueDependentGamutParams`.
#[derive(Clone, Copy, Debug)]
pub struct HueDependentGamutParams {
    pub jm_cusp: [f64; 2],
    pub gamma_bottom_inv: f64,
    pub gamma_top_inv: f64,
    pub focus_j: f64,
    pub analytical_threshold: f64,
}

/// Full output-transform parameter block.
/// from Lib.Academy.OutputTransform.ctl `ODTParams`.
///
/// NOTE (translation): upstream also holds `AP0_TO_AP1`/`AP1_TO_AP0` as
/// translation-unit constants evaluated once by the CTL interpreter; this
/// struct carries those two pre-computed matrices as fields so the per-pixel
/// path does not rebuild them.
#[derive(Clone, Debug)]
pub struct OdtParams {
    pub peak_luminance: f64,

    // JMh parameters
    pub input_params: JMhParams,
    pub reach_params: JMhParams,
    pub limit_params: JMhParams,

    // Tonescale parameters
    pub ts: TsParams,

    // Shared compression parameters
    pub limit_j_max: f64,
    pub model_gamma_inv: f64,
    pub table_reach_m: [f64; TOTAL_TABLE_SIZE],

    // Chroma compression parameters
    pub sat: f64,
    pub sat_thr: f64,
    pub compr: f64,
    pub chroma_compress_scale: f64,

    // Gamut compression parameters
    pub mid_j: f64,
    pub focus_dist: f64,
    pub lower_hull_gamma_inv: f64,
    pub table_hues: [f64; TOTAL_TABLE_SIZE],
    pub table_gamut_cusps: [[f64; 3]; TOTAL_TABLE_SIZE],
    pub table_upper_hull_gamma: [f64; TOTAL_TABLE_SIZE],
    pub hue_linearity_search_range: [i64; 2],

    // Translation-unit constants carried as fields (see struct note).
    pub ap0_to_ap1: M33,
    pub ap1_to_ap0: M33,
}

// from Lib.Academy.OutputTransform.ctl
pub fn wrap_to_360(hue: f64) -> f64 {
    let mut y = hue % 360.0;
    if y < 0.0 {
        y += 360.0;
    }
    y
}

// from Lib.Academy.OutputTransform.ctl — int truncation of the scaled hue,
// as in the CTL float→int conversion.
pub fn hue_position_in_uniform_table(hue: f64, table_size: usize) -> i64 {
    let wrapped_hue = wrap_to_360(hue);
    (wrapped_hue / HUE_LIMIT * table_size as f64) as i64
}

// (`next_position_in_table` is not translated: its only upstream consumer
// is `hueDependentUpperHullGamma`, itself unused by the lib at the pin.)

// from Lib.Academy.OutputTransform.ctl
fn base_hue_for_position(i_lo: i64, table_size: usize) -> f64 {
    i_lo as f64 * HUE_LIMIT / table_size as f64
}

/* ---- CAM Functions ---- */

// from Lib.Academy.OutputTransform.ctl
fn post_adaptation_cone_response_compression_fwd_pos(rc: f64) -> f64 {
    let f_l_y = rc.powf(0.42);
    f_l_y / (CAM_NL_OFFSET + f_l_y)
}

// from Lib.Academy.OutputTransform.ctl
fn post_adaptation_cone_response_compression_inv_pos(ra: f64) -> f64 {
    let ra_lim = ctl_min(ra, 0.99);
    let f_l_y = (CAM_NL_OFFSET * ra_lim) / (1.0 - ra_lim);
    f_l_y.powf(1.0 / 0.42)
}

// from Lib.Academy.OutputTransform.ctl — sign-symmetric wrappers.
fn post_adaptation_cone_response_compression_fwd(v: f64) -> f64 {
    let ra = post_adaptation_cone_response_compression_fwd_pos(v.abs());
    copysign_ctl(ra, v)
}

// from Lib.Academy.OutputTransform.ctl
fn post_adaptation_cone_response_compression_inv(v: f64) -> f64 {
    let rc = post_adaptation_cone_response_compression_inv_pos(v.abs());
    copysign_ctl(rc, v)
}

// from Lib.Academy.OutputTransform.ctl
fn achromatic_n_to_j(a: f64, cz: f64) -> f64 {
    J_SCALE * a.powf(cz)
}

// from Lib.Academy.OutputTransform.ctl
fn j_to_achromatic_n(j: f64, inv_cz: f64) -> f64 {
    (j * (1.0 / J_SCALE)).powf(inv_cz)
}

// from Lib.Academy.OutputTransform.ctl — optimization for achromatic values.
fn a_to_y(a: f64, p: &JMhParams) -> f64 {
    let ra = p.a_w_j * a;
    post_adaptation_cone_response_compression_inv_pos(ra) / p.f_l_n
}

// from Lib.Academy.OutputTransform.ctl
pub fn j_to_y(j: f64, p: &JMhParams) -> f64 {
    let abs_j = j.abs();
    a_to_y(j_to_achromatic_n(abs_j, p.inv_cz), p)
}

// from Lib.Academy.OutputTransform.ctl
pub fn y_to_j(y: f64, p: &JMhParams) -> f64 {
    let abs_y = y.abs();
    let ra = post_adaptation_cone_response_compression_fwd_pos(abs_y * p.f_l_n);
    let j = achromatic_n_to_j(ra * p.inv_a_w_j, p.cz);
    copysign_ctl(j, y)
}

// from Lib.Academy.OutputTransform.ctl
pub fn rgb_to_aab(rgb: [f64; 3], p: &JMhParams) -> [f64; 3] {
    let rgb_m = mult_f3_f33(rgb, &p.matrix_rgb_to_cam16_c);

    let rgb_a = [
        post_adaptation_cone_response_compression_fwd(rgb_m[0]),
        post_adaptation_cone_response_compression_fwd(rgb_m[1]),
        post_adaptation_cone_response_compression_fwd(rgb_m[2]),
    ];

    mult_f3_f33(rgb_a, &p.matrix_cone_response_to_aab)
}

// from Lib.Academy.OutputTransform.ctl
pub fn aab_to_jmh(aab: [f64; 3], p: &JMhParams) -> [f64; 3] {
    if aab[0] <= 0.0 {
        return [0.0, 0.0, 0.0];
    }
    let j = achromatic_n_to_j(aab[0], p.cz);
    let m = (aab[1] * aab[1] + aab[2] * aab[2]).sqrt();
    let h_rad = aab[2].atan2(aab[1]);
    let h = wrap_to_360(radians_to_degrees(h_rad));

    [j, m, h]
}

// from Lib.Academy.OutputTransform.ctl
pub fn rgb_to_jmh(rgb: [f64; 3], p: &JMhParams) -> [f64; 3] {
    aab_to_jmh(rgb_to_aab(rgb, p), p)
}

// from Lib.Academy.OutputTransform.ctl
pub fn jmh_to_aab(jmh: [f64; 3], p: &JMhParams) -> [f64; 3] {
    let (j, m, h) = (jmh[0], jmh[1], jmh[2]);
    let h_rad = degrees_to_radians(h);
    let cos_hr = h_rad.cos();
    let sin_hr = h_rad.sin();

    let a = j_to_achromatic_n(j, p.inv_cz);
    [a, m * cos_hr, m * sin_hr]
}

// from Lib.Academy.OutputTransform.ctl
pub fn aab_to_rgb(aab: [f64; 3], p: &JMhParams) -> [f64; 3] {
    let rgb_a = mult_f3_f33(aab, &p.matrix_aab_to_cone_response);

    let rgb_m = [
        post_adaptation_cone_response_compression_inv(rgb_a[0]),
        post_adaptation_cone_response_compression_inv(rgb_a[1]),
        post_adaptation_cone_response_compression_inv(rgb_a[2]),
    ];

    mult_f3_f33(rgb_m, &p.matrix_cam16_c_to_rgb)
}

// from Lib.Academy.OutputTransform.ctl
pub fn jmh_to_rgb(jmh: [f64; 3], p: &JMhParams) -> [f64; 3] {
    aab_to_rgb(jmh_to_aab(jmh, p), p)
}

// from Lib.Academy.OutputTransform.ctl — clamp ACES (AP0) through AP1.
fn clamp_ap0_to_ap1(
    aces: [f64; 3],
    clamp_lower_limit: f64,
    clamp_upper_limit: f64,
    ap0_to_ap1: &M33,
    ap1_to_ap0: &M33,
) -> [f64; 3] {
    let ap1 = mult_f3_f33(aces, ap0_to_ap1);
    let ap1_clamped = clamp_f3(ap1, clamp_lower_limit, clamp_upper_limit);
    mult_f3_f33(ap1_clamped, ap1_to_ap0)
}

// from Lib.Academy.OutputTransform.ctl — linear interpolation on the
// uniform reach table; the fractional weight is `h - base` (the table holds
// one entry per degree, so `base` doubles as the entry's hue).
pub fn reach_m_from_table(h: f64, table: &[f64; TOTAL_TABLE_SIZE]) -> f64 {
    let base = hue_position_in_uniform_table(h, TABLE_SIZE);
    let t = h - base as f64;
    let i_lo = base as usize + BASE_INDEX;
    let i_hi = i_lo + 1;

    lerp(table[i_lo], table[i_hi], t)
}

// from Lib.Academy.OutputTransform.ctl
fn reinhard_remap(scale: f64, nd: f64, invert: bool) -> f64 {
    if invert {
        if nd >= 1.0 {
            return scale;
        } else {
            return scale * -(nd / (nd - 1.0));
        }
    }
    scale * nd / (1.0 + nd)
}

// from Lib.Academy.OutputTransform.ctl
fn midpoint(low: f64, high: f64) -> f64 {
    (low + high) / 2.0
}

// from Lib.Academy.OutputTransform.ctl — a "toe" that remaps x between 0
// and limit; k1/k2 change the size and shape of the toe.
fn toe(x: f64, limit: f64, k1_in: f64, k2_in: f64, invert: bool) -> f64 {
    if x > limit {
        return x;
    }

    let k2 = ctl_max(k2_in, 0.001);
    let k1 = (k1_in * k1_in + k2 * k2).sqrt();
    let k3 = (limit + k1) / (limit + k2);

    if invert {
        (x * x + k1 * x) / (k3 * (x + k2))
    } else {
        let minus_b = k3 * x - k1;
        let minus_c = k2 * k3 * x;
        0.5 * (minus_b + (minus_b * minus_b + 4.0 * minus_c).sqrt())
    }
}

// from Lib.Academy.OutputTransform.ctl — hue-dependent normalization curve
// (a 3-harmonic cosine/sine fit).
fn chroma_compress_norm(h: f64, chroma_compress_scale: f64) -> f64 {
    let hr = degrees_to_radians(h);

    let a = hr.cos();
    let b = hr.sin();
    let cos_hr2 = a * a - b * b;
    let sin_hr2 = 2.0 * a * b;
    let cos_hr3 = 4.0 * a * a * a - 3.0 * a;
    let sin_hr3 = 3.0 * b - 4.0 * b * b * b;

    let m = 11.34072 * a + 16.46899 * cos_hr2 + 7.88380 * cos_hr3 + 14.66441 * b
        - 6.37224 * sin_hr2
        + 9.19364 * sin_hr3
        + 77.12896;

    m * chroma_compress_scale
}

// from Lib.Academy.OutputTransform.ctl — in-gamut chroma compression:
// compresses colors inside the gamut so colorfulness has an appropriate
// rate of change from display black to display white and from achromatic
// outward to purer colors.
fn chroma_compress_fwd(jmh: [f64; 3], tonemapped_j: f64, p: &OdtParams) -> [f64; 3] {
    let (j, m, h) = (jmh[0], jmh[1], jmh[2]);

    let mut m_compr = m;

    if m != 0.0 {
        let nj = tonemapped_j / p.limit_j_max;
        let snj = ctl_max(0.0, 1.0 - nj);
        let mnorm = chroma_compress_norm(h, p.chroma_compress_scale);
        let limit = nj.powf(p.model_gamma_inv) * reach_m_from_table(h, &p.table_reach_m) / mnorm;

        let toe_limit = limit - 0.001;
        let toe_snj_sat = snj * p.sat;
        let toe_sqrt_nj_sat_thr = (nj * nj + p.sat_thr).sqrt();
        let toe_nj_compr = nj * p.compr;

        // Rescaling of M with the tonescaled J to get the M to the same
        // range as J after the tonescale. The rescaling uses the
        // Hellwig2022 model gamma to keep the M/J ratio correct (keeping
        // the chromaticities constant).
        m_compr = m * (tonemapped_j / j).powf(p.model_gamma_inv);

        // Normalize M with the rendering space cusp M
        m_compr /= mnorm;

        // Expand the colorfulness by running the toe function in reverse.
        // Expands less saturated colors less and more saturated colors
        // more; increases saturation in the shadows and mid-tones but not
        // in the highlights. The 0.001 offset starts the expansion
        // slightly above zero; sat_thr makes the toe less aggressive near
        // black to reduce the expansion of noise.
        m_compr = limit
            - toe(
                limit - m_compr,
                toe_limit,
                toe_snj_sat,
                toe_sqrt_nj_sat_thr,
                false,
            );

        // Compress the colorfulness. Compresses less saturated colors more
        // and more saturated colors less, especially in the highlights,
        // creating the saturation roll-off in the highlights while
        // attempting to preserve pure colors.
        m_compr = toe(m_compr, limit, toe_nj_compr, snj, false);

        // Denormalize
        m_compr *= mnorm;
    }

    [tonemapped_j, m_compr, h]
}

// from Lib.Academy.OutputTransform.ctl
fn chroma_compress_inv(jmh: [f64; 3], j: f64, p: &OdtParams) -> [f64; 3] {
    let (tonemapped_j, m_compr, h) = (jmh[0], jmh[1], jmh[2]);

    let mut m = m_compr;

    if m_compr != 0.0 {
        let nj = tonemapped_j / p.limit_j_max;
        let snj = ctl_max(0.0, 1.0 - nj);
        let mnorm = chroma_compress_norm(h, p.chroma_compress_scale);
        let limit = nj.powf(p.model_gamma_inv) * reach_m_from_table(h, &p.table_reach_m) / mnorm;

        let toe_limit = limit - 0.001;
        let toe_snj_sat = snj * p.sat;
        let toe_sqrt_nj_sat_thr = (nj * nj + p.sat_thr).sqrt();
        let toe_nj_compr = nj * p.compr;

        m = m_compr / mnorm;
        m = toe(m, limit, toe_nj_compr, snj, true);
        m = limit - toe(limit - m, toe_limit, toe_snj_sat, toe_sqrt_nj_sat_thr, true);
        m *= mnorm;
        m *= (tonemapped_j / j).powf(-p.model_gamma_inv);
    }

    [j, m, h]
}

// from Lib.Academy.OutputTransform.ctl — forward tonescale then chroma
// compression; returns { tonemapped J, compressed M, h }.
pub fn tonemap_and_compress_fwd(jmh: [f64; 3], p: &OdtParams) -> [f64; 3] {
    let linear = j_to_y(jmh[0], &p.input_params) / REF_LUMINANCE;

    let tonemapped_y = tonescale_fwd(linear, &p.ts);

    let j_ts = y_to_j(tonemapped_y, &p.input_params);

    chroma_compress_fwd(jmh, j_ts, p)
}

// from Lib.Academy.OutputTransform.ctl — inverse tonescale then chroma
// un-compression; returns { J, M, h }.
pub fn tonemap_and_compress_inv(jmh_tc: [f64; 3], p: &OdtParams) -> [f64; 3] {
    let luminance = j_to_y(jmh_tc[0], &p.input_params);

    let linear = tonescale_inv(luminance / REF_LUMINANCE, &p.ts);

    let j = y_to_j(linear * REF_LUMINANCE, &p.input_params);

    chroma_compress_inv(jmh_tc, j, p)
}

// from Lib.Academy.OutputTransform.ctl
fn compute_compression_vector_slope(
    intersect_j: f64,
    focus_j: f64,
    limit_j_max: f64,
    slope_gain: f64,
) -> f64 {
    let direction_scalar = if intersect_j < focus_j {
        intersect_j
    } else {
        limit_j_max - intersect_j
    };
    direction_scalar * (intersect_j - focus_j) / (focus_j * slope_gain)
}

// from Lib.Academy.OutputTransform.ctl
fn solve_j_intersect(j: f64, m: f64, focus_j: f64, max_j: f64, slope_gain: f64) -> f64 {
    let m_scaled = m / slope_gain;
    let a = m_scaled / focus_j;

    if j < focus_j {
        let b = 1.0 - m_scaled;
        let c = -j;
        let det = b * b - 4.0 * a * c;
        let root = det.sqrt();
        -2.0 * c / (b + root)
    } else {
        let b = -(1.0 + m_scaled + max_j * a);
        let c = max_j * m_scaled + j;
        let det = b * b - 4.0 * a * c;
        let root = det.sqrt();
        -2.0 * c / (b - root)
    }
}

// from Lib.Academy.OutputTransform.ctl — smooth minimum about the scaled
// reference, based upon a cubic polynomial.
fn smin_scaled(a: f64, b: f64, scale_reference: f64) -> f64 {
    let s_scaled = SMOOTH_CUSPS * scale_reference;
    let h = ctl_max(s_scaled - (a - b).abs(), 0.0) / s_scaled;
    ctl_min(a, b) - h * h * h * s_scaled * (1.0 / 6.0)
}

// from Lib.Academy.OutputTransform.ctl — approximate intersection of the
// compression line with an exponential boundary (deliberately not solved
// iteratively upstream).
fn estimate_line_and_boundary_intersection_m(
    j_axis_intersect: f64,
    slope: f64,
    inv_gamma: f64,
    j_max: f64,
    m_max: f64,
    j_intersection_reference: f64,
) -> f64 {
    // Line defined by     J = slope * x + J_axis_intersect
    // Boundary defined by J = J_max * (x / M_max) ^ (1/inv_gamma)
    let normalised_j = j_axis_intersect / j_intersection_reference;
    let shifted_intersection = j_intersection_reference * normalised_j.powf(inv_gamma);

    shifted_intersection * m_max / (j_max - slope * m_max)
}

// from Lib.Academy.OutputTransform.ctl
fn find_gamut_boundary_intersection(
    jm_cusp: [f64; 2],
    j_max: f64,
    gamma_top_inv: f64,
    gamma_bottom_inv: f64,
    j_intersect_source: f64,
    slope: f64,
    j_intersect_cusp: f64,
) -> f64 {
    let m_boundary_lower = estimate_line_and_boundary_intersection_m(
        j_intersect_source,
        slope,
        gamma_bottom_inv,
        jm_cusp[0],
        jm_cusp[1],
        j_intersect_cusp,
    );

    // The upper hull is flipped and thus 'zeroed' at J_max; the slope is
    // negated.
    let f_j_intersect_cusp = j_max - j_intersect_cusp;
    let f_j_intersect_source = j_max - j_intersect_source;
    let f_jm_cusp_j = j_max - jm_cusp[0];
    let m_boundary_upper = estimate_line_and_boundary_intersection_m(
        f_j_intersect_source,
        -slope,
        gamma_top_inv,
        f_jm_cusp_j,
        jm_cusp[1],
        f_j_intersect_cusp,
    );

    // Smooth minimum between the two calculated values for the M component
    smin_scaled(m_boundary_lower, m_boundary_upper, jm_cusp[1])
}

// from Lib.Academy.OutputTransform.ctl
fn get_focus_gain(j: f64, analytical_threshold: f64, limit_j_max: f64, focus_dist: f64) -> f64 {
    let mut gain = limit_j_max * focus_dist;

    if j > analytical_threshold {
        // Approximate inverse required above threshold due to the
        // introduction of J in the calculation
        let mut gain_adjustment =
            ((limit_j_max - analytical_threshold) / ctl_max(0.0001, limit_j_max - j)).log10();
        gain_adjustment = gain_adjustment * gain_adjustment + 1.0;
        gain *= gain_adjustment;
    }

    gain
}

// from Lib.Academy.OutputTransform.ctl
fn remap_m(m: f64, gamut_boundary_m: f64, reach_boundary_m: f64, invert: bool) -> f64 {
    let boundary_ratio = gamut_boundary_m / reach_boundary_m;
    let proportion = ctl_max(boundary_ratio, COMPRESSION_THRESHOLD);
    let threshold = proportion * gamut_boundary_m;

    if m <= threshold || proportion >= 1.0 {
        return m;
    }

    // Translate to place threshold at zero
    let m_offset = m - threshold;
    let gamut_offset = gamut_boundary_m - threshold;
    let reach_offset = reach_boundary_m - threshold;

    let scale = reach_offset / ((reach_offset / gamut_offset) - 1.0);
    let nd = m_offset / scale;

    // Shift result back to absolute
    threshold + reinhard_remap(scale, nd, invert)
}

// from Lib.Academy.OutputTransform.ctl
fn compress_gamut(
    jmh: [f64; 3],
    jx: f64,
    p: &OdtParams,
    hdp: &HueDependentGamutParams,
    invert: bool,
) -> [f64; 3] {
    let (j, m, h) = (jmh[0], jmh[1], jmh[2]);

    let slope_gain = get_focus_gain(jx, hdp.analytical_threshold, p.limit_j_max, p.focus_dist);
    let j_intersect_source = solve_j_intersect(j, m, hdp.focus_j, p.limit_j_max, slope_gain);
    let gamut_slope = compute_compression_vector_slope(
        j_intersect_source,
        hdp.focus_j,
        p.limit_j_max,
        slope_gain,
    );

    let j_intersect_cusp = solve_j_intersect(
        hdp.jm_cusp[0],
        hdp.jm_cusp[1],
        hdp.focus_j,
        p.limit_j_max,
        slope_gain,
    );

    let gamut_boundary_m = find_gamut_boundary_intersection(
        hdp.jm_cusp,
        p.limit_j_max,
        hdp.gamma_top_inv,
        hdp.gamma_bottom_inv,
        j_intersect_source,
        gamut_slope,
        j_intersect_cusp,
    );

    if gamut_boundary_m <= 0.0 {
        return [j, 0.0, h];
    }

    let reach_max_m = reach_m_from_table(h, &p.table_reach_m);

    let reach_boundary_m = estimate_line_and_boundary_intersection_m(
        j_intersect_source,
        gamut_slope,
        p.model_gamma_inv,
        p.limit_j_max,
        reach_max_m,
        p.limit_j_max,
    );

    let remapped_m = remap_m(m, gamut_boundary_m, reach_boundary_m, invert);

    [j_intersect_source + remapped_m * gamut_slope, remapped_m, h]
}

// from Lib.Academy.OutputTransform.ctl — binary search on the cusp table
// (hue column is monotone; the float `midpoint` int-truncation upstream
// equals integer division for these non-negative indices).
fn cusp_from_table(h: f64, table: &[[f64; 3]; TOTAL_TABLE_SIZE]) -> [f64; 2] {
    let mut low_i: usize = 0;
    let mut high_i: usize = BASE_INDEX + TABLE_SIZE;
    let mut i = (hue_position_in_uniform_table(h, TABLE_SIZE) + BASE_INDEX as i64) as usize;

    while low_i + 1 < high_i {
        if h > table[i][2] {
            low_i = i;
        } else {
            high_i = i;
        }
        i = (low_i + high_i) / 2;
    }
    let lo = table[high_i - 1];
    let hi = table[high_i];

    let t = (h - lo[2]) / (hi[2] - lo[2]);
    let cusp_j = lerp(lo[0], hi[0], t);
    let cusp_m = lerp(lo[1], hi[1], t);

    [cusp_j, cusp_m]
}

// from Lib.Academy.OutputTransform.ctl — search the hue table for the
// interval containing the desired hue; returns the interval's upper index.
// NOTE (translation): upstream computes the clamped bounds in `unsigned
// int`; a negative `i + range[0]` would wrap there. Signed arithmetic with
// the same clamps implements the evident intent (the wrap can only occur
// for hues within a degree of 0 with a strongly non-uniform table).
fn lookup_hue_interval(
    h: f64,
    hue_table: &[f64; TOTAL_TABLE_SIZE],
    hue_linearity_search_range: [i64; 2],
) -> usize {
    // We can narrow the search range based on the hues being almost uniform
    let mut i = BASE_INDEX as i64 + hue_position_in_uniform_table(h, TOTAL_TABLE_SIZE);
    let mut i_lo = (BASE_INDEX as i64).max(i + hue_linearity_search_range[0]);
    let mut i_hi = ((BASE_INDEX + TABLE_SIZE) as i64).min(i + hue_linearity_search_range[1]);

    while i_lo + 1 < i_hi {
        if h > hue_table[i as usize] {
            i_lo = i;
        } else {
            i_hi = i;
        }
        i = (i_lo + i_hi) / 2;
    }

    i_hi = i_hi.max(1);

    i_hi as usize
}

// from Lib.Academy.OutputTransform.ctl
fn interpolation_weight(h: f64, h_lo: f64, _h_hi: f64) -> f64 {
    h - h_lo
}

// from Lib.Academy.OutputTransform.ctl
fn compute_focus_j(cusp_j: f64, mid_j: f64, limit_j_max: f64) -> f64 {
    lerp(
        cusp_j,
        mid_j,
        ctl_min(1.0, CUSP_MID_BLEND - (cusp_j / limit_j_max)),
    )
}

// from Lib.Academy.OutputTransform.ctl
pub fn init_hue_dependent_gamut_params(hue: f64, p: &OdtParams) -> HueDependentGamutParams {
    let gamma_bottom_inv = p.lower_hull_gamma_inv;

    let i_hi = lookup_hue_interval(hue, &p.table_hues, p.hue_linearity_search_range);
    let t = interpolation_weight(hue, p.table_hues[i_hi - 1], p.table_hues[i_hi]);

    let jm_cusp = cusp_from_table(hue, &p.table_gamut_cusps);
    let gamma_top_inv = lerp(
        p.table_upper_hull_gamma[i_hi - 1],
        p.table_upper_hull_gamma[i_hi],
        t,
    );
    let focus_j = compute_focus_j(jm_cusp[0], p.mid_j, p.limit_j_max);
    let analytical_threshold = lerp(jm_cusp[0], p.limit_j_max, FOCUS_GAIN_BLEND);

    HueDependentGamutParams {
        jm_cusp,
        gamma_bottom_inv,
        gamma_top_inv,
        focus_j,
        analytical_threshold,
    }
}

// from Lib.Academy.OutputTransform.ctl
pub fn gamut_compress_fwd(jmh: [f64; 3], p: &OdtParams) -> [f64; 3] {
    let (j, m, h) = (jmh[0], jmh[1], jmh[2]);

    if j <= 0.0 {
        return [0.0, 0.0, h];
    }
    if m < 0.0 || j > p.limit_j_max {
        return [j, 0.0, h];
    }

    let hdp = init_hue_dependent_gamut_params(h, p);

    compress_gamut(jmh, j, p, &hdp, false)
}

// from Lib.Academy.OutputTransform.ctl
pub fn gamut_compress_inv(jmh: [f64; 3], p: &OdtParams) -> [f64; 3] {
    let (j, m, h) = (jmh[0], jmh[1], jmh[2]);

    if j <= 0.0 {
        return [0.0, 0.0, h];
    }
    if m < 0.0 || j > p.limit_j_max {
        return [j, 0.0, h];
    }

    let hdp = init_hue_dependent_gamut_params(h, p);

    let mut jx = j;

    if jx > hdp.analytical_threshold {
        jx = compress_gamut(jmh, jx, p, &hdp, true)[0];
    }

    compress_gamut(jmh, jx, p, &hdp, true)
}

/* ---- Table building functions ---- */

// from Lib.Academy.OutputTransform.ctl
fn any_below_zero(rgb: [f64; 3]) -> bool {
    rgb[0] < 0.0 || rgb[1] < 0.0 || rgb[2] < 0.0
}

// from Lib.Academy.OutputTransform.ctl — CAM16-lineage appearance-model
// parameter pre-computation for a primary set.
pub fn init_jmh_params(prims: &Chromaticities) -> JMhParams {
    let cam16_pri = Chromaticities {
        red: [0.8336, 0.1735],
        green: [2.3854, -1.4659],
        blue: [0.087, -0.125],
        white: [0.333, 0.333],
    };

    let matrix_16 = xyz_to_rgb_f33(&cam16_pri, 1.0);

    let base_cone_response_to_aab: M33 = [
        [2.0, 1.0, 1.0 / 9.0],
        [1.0, -12.0 / 11.0, 1.0 / 9.0],
        [1.0 / 20.0, 1.0 / 11.0, -2.0 / 9.0],
    ];

    let rgb_to_xyz = super::utilities::rgb_to_xyz_f33(prims, 1.0);
    let xyz_w = mult_f3_f33(f3_from_f(REF_LUMINANCE), &rgb_to_xyz);

    let y_w = xyz_w[1];

    // Step 0 - Converting CIE XYZ tristimulus values to sharpened RGB values
    let rgb_w = mult_f3_f33(xyz_w, &matrix_16);

    // Viewing condition dependent parameters
    let k = 1.0 / (5.0 * L_A + 1.0);
    let k4 = k * k * k * k;
    let f_l = 0.2 * k4 * (5.0 * L_A) + 0.1 * (1.0 - k4).powf(2.0) * (5.0 * L_A).powf(1.0 / 3.0);

    let f_l_n = f_l / REF_LUMINANCE;
    let cz = model_gamma();

    let d_rgb = [
        f_l_n * y_w / rgb_w[0],
        f_l_n * y_w / rgb_w[1],
        f_l_n * y_w / rgb_w[2],
    ];

    let rgb_wc = [
        d_rgb[0] * rgb_w[0],
        d_rgb[1] * rgb_w[1],
        d_rgb[2] * rgb_w[2],
    ];

    let rgb_aw = [
        post_adaptation_cone_response_compression_fwd(rgb_wc[0]),
        post_adaptation_cone_response_compression_fwd(rgb_wc[1]),
        post_adaptation_cone_response_compression_fwd(rgb_wc[2]),
    ];

    let cone_response_to_aab = mult_f33_f33(
        &mult_f_f33(CAM_NL_SCALE, &MATRIX_IDENTITY),
        &base_cone_response_to_aab,
    );
    let a_w = cone_response_to_aab[0][0] * rgb_aw[0]
        + cone_response_to_aab[1][0] * rgb_aw[1]
        + cone_response_to_aab[2][0] * rgb_aw[2];
    let a_w_j = post_adaptation_cone_response_compression_fwd_pos(f_l);

    // Prescale the CAM16 LMS responses to directly provide for chromatic
    // adaptation
    let m1 = mult_f33_f33(&rgb_to_xyz, &matrix_16);
    let m2 = mult_f_f33(REF_LUMINANCE, &MATRIX_IDENTITY);
    let matrix_rgb_to_cam16 = mult_f33_f33(&m1, &m2);
    let matrix_rgb_to_cam16_c = mult_f33_f33(
        &matrix_rgb_to_cam16,
        &scale_matrix_diagonal_f33_f3(MATRIX_IDENTITY, d_rgb),
    );

    let s2 = 43.0 * SURROUND[2];
    let matrix_cone_response_to_aab: M33 = [
        [
            cone_response_to_aab[0][0] / a_w,
            cone_response_to_aab[0][1] * s2,
            cone_response_to_aab[0][2] * s2,
        ],
        [
            cone_response_to_aab[1][0] / a_w,
            cone_response_to_aab[1][1] * s2,
            cone_response_to_aab[1][2] * s2,
        ],
        [
            cone_response_to_aab[2][0] / a_w,
            cone_response_to_aab[2][1] * s2,
            cone_response_to_aab[2][2] * s2,
        ],
    ];

    JMhParams {
        matrix_rgb_to_cam16_c,
        matrix_cam16_c_to_rgb: invert_f33(&matrix_rgb_to_cam16_c),
        matrix_cone_response_to_aab,
        matrix_aab_to_cone_response: invert_f33(&matrix_cone_response_to_aab),
        f_l_n,
        cz,
        inv_cz: 1.0 / cz,
        a_w_j,
        inv_a_w_j: 1.0 / a_w_j,
    }
}

// from Lib.Academy.OutputTransform.ctl — generation order R, Y, G, C, B, M
// to ensure hues rotate in correct order.
fn generate_unit_cube_cusp_corners(corner: usize) -> [f64; 3] {
    let mut result = [0.0; 3];

    if ((corner + 1) % CUSP_CORNER_COUNT) < 3 {
        result[0] = 1.0;
    }
    if ((corner + 5) % CUSP_CORNER_COUNT) < 3 {
        result[1] = 1.0;
    }
    if ((corner + 3) % CUSP_CORNER_COUNT) < 3 {
        result[2] = 1.0;
    }

    result
}

// from Lib.Academy.OutputTransform.ctl — RGB and JMh values for the
// limiting gamut cusp corners, arranged into a cycle with the lowest-hue
// entry at [1] to allow for hue wrapping.
//
// NOTE (translation fidelity): the upstream minimum-hue scan sets
// `min_index = 1` (a literal) rather than `min_index = i` inside the loop.
// This is preserved verbatim. It is latent for the Rec.709-class limiting
// primaries used here: the red corner (index 0) already has the lowest JMh
// hue, so the branch never fires.
fn build_limiting_cusp_corners_tables(
    rgb_corners: &mut [[f64; 3]; TOTAL_CORNER_COUNT],
    jmh_corners: &mut [[f64; 3]; TOTAL_CORNER_COUNT],
    params: &JMhParams,
    peak_luminance: f64,
) {
    let mut temp_rgb_corners = [[0.0f64; 3]; CUSP_CORNER_COUNT];
    let mut temp_jmh_corners = [[0.0f64; 3]; CUSP_CORNER_COUNT];

    let mut min_index: usize = 0;
    for i in 0..CUSP_CORNER_COUNT {
        temp_rgb_corners[i] = mult_f_f3(
            peak_luminance / REF_LUMINANCE,
            generate_unit_cube_cusp_corners(i),
        );
        temp_jmh_corners[i] = rgb_to_jmh(temp_rgb_corners[i], params);
        if temp_jmh_corners[i][2] < temp_jmh_corners[min_index][2] {
            min_index = 1; // sic — see the translation-fidelity note above
        }
    }

    // Rotate entries placing lowest at [1] (not [0])
    for i in 0..CUSP_CORNER_COUNT {
        rgb_corners[i + 1] = temp_rgb_corners[(i + min_index) % CUSP_CORNER_COUNT];
        jmh_corners[i + 1] = temp_jmh_corners[(i + min_index) % CUSP_CORNER_COUNT];
    }

    // Copy end elements to create a cycle
    rgb_corners[0] = rgb_corners[CUSP_CORNER_COUNT];
    rgb_corners[CUSP_CORNER_COUNT + 1] = rgb_corners[1];
    jmh_corners[0] = jmh_corners[CUSP_CORNER_COUNT];
    jmh_corners[CUSP_CORNER_COUNT + 1] = jmh_corners[1];

    // Wrap the hues; to maintain monotonicity these entries fall outside
    // [0.0, hue_limit)
    jmh_corners[0][2] -= HUE_LIMIT;
    jmh_corners[CUSP_CORNER_COUNT + 1][2] += HUE_LIMIT;
}

// from Lib.Academy.OutputTransform.ctl — the JMh of the reach-gamut
// "corners" at limit J, found by bisection on the corner vector scale using
// the achromatic channel (avoiding the non-linear transform mid-search).
fn find_reach_corners_table(
    params_reach: &JMhParams,
    p: &OdtParams,
) -> [[f64; 3]; TOTAL_CORNER_COUNT] {
    let mut temp_jmh_corners = [[0.0f64; 3]; CUSP_CORNER_COUNT];
    let mut jmh_corners = [[0.0f64; 3]; TOTAL_CORNER_COUNT];

    let limit_a = j_to_achromatic_n(p.limit_j_max, params_reach.inv_cz);

    let mut min_index: usize = 0;
    for i in 0..CUSP_CORNER_COUNT {
        let rgb_vector = generate_unit_cube_cusp_corners(i);

        let mut lower = 0.0f64;
        let mut upper = p.ts.forward_limit;

        while (upper - lower) > REACH_CUSP_TOLERANCE {
            let test = (lower + upper) / 2.0;
            let test_corner = mult_f_f3(test, rgb_vector);
            let a = rgb_to_aab(test_corner, params_reach)[0];
            if a < limit_a {
                lower = test;
            } else {
                upper = test;
            }
        }

        temp_jmh_corners[i] = rgb_to_jmh(mult_f_f3(upper, rgb_vector), params_reach);

        if temp_jmh_corners[i][2] < temp_jmh_corners[min_index][2] {
            min_index = i;
        }
    }

    // Rotate entries placing lowest at [1] (not [0])
    for i in 0..CUSP_CORNER_COUNT {
        jmh_corners[i + 1] = temp_jmh_corners[(i + min_index) % CUSP_CORNER_COUNT];
    }

    // Copy end elements to create a cycle
    jmh_corners[0] = jmh_corners[CUSP_CORNER_COUNT];
    jmh_corners[CUSP_CORNER_COUNT + 1] = jmh_corners[1];

    // Wrap the hues
    jmh_corners[0][2] -= HUE_LIMIT;
    jmh_corners[CUSP_CORNER_COUNT + 1][2] += HUE_LIMIT;

    jmh_corners
}

// from Lib.Academy.OutputTransform.ctl — merge of two sorted corner-hue
// lists into the unique sorted hues. NOTE (translation): if any hue pair
// were exactly equal, the tail entries would remain at their initial 0.0
// (upstream relies on the interpreter's zero-init); this does not occur for
// the primary sets used here (reach and limit corner hues never coincide
// exactly).
fn extract_sorted_cube_hues(
    reach_jmh: &[[f64; 3]; TOTAL_CORNER_COUNT],
    limit_jmh: &[[f64; 3]; TOTAL_CORNER_COUNT],
) -> [f64; MAX_SORTED_CORNERS] {
    let mut sorted_hues = [0.0f64; MAX_SORTED_CORNERS];

    let mut idx = 0usize;
    let mut reach_idx = 1usize;
    let mut limit_idx = 1usize;
    while (reach_idx < (CUSP_CORNER_COUNT + 1)) || (limit_idx < (CUSP_CORNER_COUNT + 1)) {
        let reach_hue = reach_jmh[reach_idx][2];
        let limit_hue = limit_jmh[limit_idx][2];
        if reach_hue == limit_hue {
            sorted_hues[idx] = reach_hue;
            reach_idx += 1;
            limit_idx += 1; // When equal consume both
        } else if reach_hue < limit_hue {
            sorted_hues[idx] = reach_hue;
            reach_idx += 1;
        } else {
            sorted_hues[idx] = limit_hue;
            limit_idx += 1;
        }
        idx += 1;
    }
    sorted_hues
}

// from Lib.Academy.OutputTransform.ctl — fill `samples` uniform hue steps
// of [lower, upper) starting at `base`.
fn build_hue_sample_interval(
    samples: i64,
    lower: f64,
    upper: f64,
    hue_table: &mut [f64; TOTAL_TABLE_SIZE],
    base: usize,
) {
    let delta = (upper - lower) / samples as f64;
    // NOTE (translation): a negative `samples` would infinite-loop in the
    // CTL `i != samples` form; the Rust range is simply empty. The count is
    // non-negative for all real corner-hue configurations.
    for i in 0..samples.max(0) {
        hue_table[base + i as usize] = lower + i as f64 * delta;
    }
}

// from Lib.Academy.OutputTransform.ctl — build the (nearly uniform) hue
// sample table forced through the sorted corner hues. Upstream leaves
// `samples_count`/`last_idx` to interpreter zero-init; both start at 0
// here, matching.
fn build_hue_table(sorted_hues: &[f64; MAX_SORTED_CORNERS]) -> [f64; TOTAL_TABLE_SIZE] {
    let mut hue_table = [0.0f64; TOTAL_TABLE_SIZE];

    let ideal_spacing = TABLE_SIZE as f64 / HUE_LIMIT;
    let mut samples_count = [0i64; 2 * CUSP_CORNER_COUNT + 2];
    let mut last_idx: i64 = 0;
    let mut min_index: i64 = if sorted_hues[0] == 0.0 { 0 } else { 1 };

    for hue_idx in 0..MAX_SORTED_CORNERS {
        let mut nominal_idx = (round_ctl(sorted_hues[hue_idx] * ideal_spacing) as i64)
            .max(min_index)
            .min((TABLE_SIZE - 1) as i64);

        if last_idx == nominal_idx {
            // Last two hues would sample at the same index; adjust the
            // previous sample down if possible
            if hue_idx > 1 && samples_count[hue_idx - 2] != (samples_count[hue_idx - 1] - 1) {
                samples_count[hue_idx - 1] -= 1;
            } else {
                nominal_idx += 1;
            }
        }
        samples_count[hue_idx] = nominal_idx.min((TABLE_SIZE - 1) as i64);
        min_index = nominal_idx;
        last_idx = min_index;
    }

    let mut total_samples: i64 = 0;
    // Special cases for ends
    build_hue_sample_interval(
        samples_count[0],
        0.0,
        sorted_hues[0],
        &mut hue_table,
        (total_samples + 1) as usize,
    );
    total_samples += samples_count[0];

    for i in 1..MAX_SORTED_CORNERS {
        let samples = samples_count[i] - samples_count[i - 1];
        build_hue_sample_interval(
            samples,
            sorted_hues[i - 1],
            sorted_hues[i],
            &mut hue_table,
            (total_samples + 1) as usize,
        );
        total_samples += samples;
    }
    build_hue_sample_interval(
        TABLE_SIZE as i64 - total_samples,
        sorted_hues[MAX_SORTED_CORNERS - 1],
        HUE_LIMIT,
        &mut hue_table,
        (total_samples + 1) as usize,
    );

    hue_table[0] = hue_table[BASE_INDEX + TABLE_SIZE - 1] - HUE_LIMIT;
    hue_table[BASE_INDEX + TABLE_SIZE] = hue_table[BASE_INDEX] + HUE_LIMIT;

    hue_table
}

// from Lib.Academy.OutputTransform.ctl — find the limiting-gamut cusp JM
// for a hue by bisecting along the RGB edge between the two bracketing cusp
// corners. `previous` is carried (and the acceleration writes to it remain
// commented out upstream), so it stays {0, 0} and the related branch never
// fires — translated as-is.
fn find_display_cusp_for_hue(
    hue: f64,
    rgb_corners: &[[f64; 3]; TOTAL_CORNER_COUNT],
    jmh_corners: &[[f64; 3]; TOTAL_CORNER_COUNT],
    params: &JMhParams,
    previous: &[f64; 2],
) -> [f64; 2] {
    let mut upper_corner: usize = 1;
    let mut found = false;
    for (i, corner) in jmh_corners.iter().enumerate().skip(1) {
        if found {
            break;
        }
        if corner[2] > hue {
            upper_corner = i;
            found = true;
        }
    }
    let lower_corner = upper_corner - 1;

    // hue is now within [lower_corner, upper_corner); handle exact match
    if jmh_corners[lower_corner][2] == hue {
        return [jmh_corners[lower_corner][0], jmh_corners[lower_corner][1]];
    }

    // search by lerping between RGB corners for the hue
    let cusp_lower = rgb_corners[lower_corner];
    let cusp_upper = rgb_corners[upper_corner];

    let mut lower_t = 0.0f64;
    if upper_corner as f64 == previous[0] {
        lower_t = previous[1];
    }
    let mut upper_t = 1.0f64;

    let mut sample_t;
    let mut sample;
    let mut jmh;

    // Edge case handling when across the [0, hue_limit] boundary: compare
    // against the corner hues to stay in the expected range.
    while (upper_t - lower_t) > DISPLAY_CUSP_TOLERANCE {
        sample_t = midpoint(lower_t, upper_t);
        sample = lerp_f3(cusp_lower, cusp_upper, sample_t);
        jmh = rgb_to_jmh(sample, params);
        if jmh[2] < jmh_corners[lower_corner][2] {
            upper_t = sample_t;
        } else if jmh[2] >= jmh_corners[upper_corner][2] {
            lower_t = sample_t;
        } else if jmh[2] > hue {
            upper_t = sample_t;
        } else {
            lower_t = sample_t;
        }
    }

    // Use the midpoint of the final interval for the actual samples
    sample_t = midpoint(lower_t, upper_t);
    sample = lerp_f3(cusp_lower, cusp_upper, sample_t);
    jmh = rgb_to_jmh(sample, params);

    [jmh[0], jmh[1]]
}

// from Lib.Academy.OutputTransform.ctl
fn build_cusp_table(
    hue_table: &[f64; TOTAL_TABLE_SIZE],
    rgb_corners: &[[f64; 3]; TOTAL_CORNER_COUNT],
    jmh_corners: &[[f64; 3]; TOTAL_CORNER_COUNT],
    params: &JMhParams,
) -> [[f64; 3]; TOTAL_TABLE_SIZE] {
    let previous = [0.0f64, 0.0f64];
    let mut output_table = [[0.0f64; 3]; TOTAL_TABLE_SIZE];

    for i in BASE_INDEX..TOTAL_TABLE_SIZE {
        let hue = hue_table[i];
        let jm = find_display_cusp_for_hue(hue, rgb_corners, jmh_corners, params, &previous);
        output_table[i][0] = jm[0];
        output_table[i][1] = jm[1] * (1.0 + SMOOTH_M * SMOOTH_CUSPS);
        output_table[i][2] = hue;
    }

    // Copy last nominal entry to start
    output_table[0][0] = output_table[TABLE_SIZE][0];
    output_table[0][1] = output_table[TABLE_SIZE][1];
    output_table[0][2] = hue_table[0];

    // Copy first nominal entry to end
    output_table[BASE_INDEX + TABLE_SIZE][0] = output_table[BASE_INDEX][0];
    output_table[BASE_INDEX + TABLE_SIZE][1] = output_table[BASE_INDEX][1];
    output_table[BASE_INDEX + TABLE_SIZE][2] = hue_table[BASE_INDEX + TABLE_SIZE];

    output_table
}

// from Lib.Academy.OutputTransform.ctl — sample hues as uniformly as
// possible while ensuring the limiting-gamut corners and the reach
// primaries at limit J max are sampled exactly.
fn make_uniform_hue_gamut_table(
    reach_params: &JMhParams,
    limit_params: &JMhParams,
    p: &OdtParams,
) -> [[f64; 3]; TOTAL_TABLE_SIZE] {
    let mut limiting_rgb_corners = [[0.0f64; 3]; TOTAL_CORNER_COUNT];
    let mut limiting_jmh_corners = [[0.0f64; 3]; TOTAL_CORNER_COUNT];

    let reach_jmh_corners = find_reach_corners_table(reach_params, p);
    build_limiting_cusp_corners_tables(
        &mut limiting_rgb_corners,
        &mut limiting_jmh_corners,
        limit_params,
        p.peak_luminance,
    );
    let sorted_hues = extract_sorted_cube_hues(&reach_jmh_corners, &limiting_jmh_corners);

    let hue_table = build_hue_table(&sorted_hues);

    build_cusp_table(
        &hue_table,
        &limiting_rgb_corners,
        &limiting_jmh_corners,
        limit_params,
    )
}

// from Lib.Academy.OutputTransform.ctl — reach gamut M at limitJmax per
// uniform hue step (coarse march then bisection on gamut exit).
fn make_reach_m_table(params: &JMhParams, limit_j_max: f64) -> [f64; TOTAL_TABLE_SIZE] {
    let mut reach_table = [0.0f64; TOTAL_TABLE_SIZE];

    for i in 0..TABLE_SIZE {
        let hue = base_hue_for_position(i as i64, TABLE_SIZE);

        const SEARCH_RANGE: f64 = 50.0;
        const SEARCH_MAXIMUM: f64 = 1300.0;
        let mut low = 0.0f64;
        let mut high = low + SEARCH_RANGE;
        let mut outside = false;

        while !outside && (high < SEARCH_MAXIMUM) {
            let search_jmh = [limit_j_max, high, hue];
            let new_limit_rgb = jmh_to_rgb(search_jmh, params);
            outside = any_below_zero(new_limit_rgb);
            if !outside {
                low = high;
                high += SEARCH_RANGE;
            }
        }

        while high - low > 1e-2 {
            let sample_m = (high + low) / 2.0;
            let search_jmh = [limit_j_max, sample_m, hue];
            let new_limit_rgb = jmh_to_rgb(search_jmh, params);
            outside = any_below_zero(new_limit_rgb);
            if outside {
                high = sample_m;
            } else {
                low = sample_m;
            }
        }

        reach_table[i + BASE_INDEX] = high;
    }

    // Copy last populated entry to first empty spot
    reach_table[0] = reach_table[TABLE_SIZE];

    // Copy first populated entry to last empty spot
    reach_table[BASE_INDEX + TABLE_SIZE] = reach_table[BASE_INDEX];

    reach_table
}

// from Lib.Academy.OutputTransform.ctl
fn outside_hull(rgb: [f64; 3], max_rgb_test_val: f64) -> bool {
    rgb[0] > max_rgb_test_val || rgb[1] > max_rgb_test_val || rgb[2] > max_rgb_test_val
}

const TEST_COUNT: usize = 5;
const TEST_POSITIONS: [f64; TEST_COUNT] = [0.01, 0.1, 0.5, 0.8, 0.99];

// from Lib.Academy.OutputTransform.ctl
#[allow(clippy::too_many_arguments)]
fn generate_gamma_test_data(
    jm_cusp: [f64; 2],
    hue: f64,
    limit_j_max: f64,
    mid_j: f64,
    focus_dist: f64,
    test_jmh: &mut [[f64; 3]; TEST_COUNT],
    j_intersect_source: &mut [f64; TEST_COUNT],
    slopes: &mut [f64; TEST_COUNT],
    j_intersect_cusp: &mut [f64; TEST_COUNT],
) {
    let analytical_threshold = lerp(jm_cusp[0], limit_j_max, FOCUS_GAIN_BLEND);
    let focus_j = compute_focus_j(jm_cusp[0], mid_j, limit_j_max);

    for test_index in 0..TEST_COUNT {
        let test_j = lerp(jm_cusp[0], limit_j_max, TEST_POSITIONS[test_index]);
        let slope_gain = get_focus_gain(test_j, analytical_threshold, limit_j_max, focus_dist);
        let j_intersect = solve_j_intersect(test_j, jm_cusp[1], focus_j, limit_j_max, slope_gain);
        let slope = compute_compression_vector_slope(j_intersect, focus_j, limit_j_max, slope_gain);
        let j_cusp = solve_j_intersect(jm_cusp[0], jm_cusp[1], focus_j, limit_j_max, slope_gain);

        test_jmh[test_index] = [test_j, jm_cusp[1], hue];
        j_intersect_source[test_index] = j_intersect;
        slopes[test_index] = slope;
        j_intersect_cusp[test_index] = j_cusp;
    }
}

// from Lib.Academy.OutputTransform.ctl
#[allow(clippy::too_many_arguments)]
fn evaluate_gamma_fit(
    jm_cusp: [f64; 2],
    test_jmh: &[[f64; 3]; TEST_COUNT],
    j_intersect_source: &[f64; TEST_COUNT],
    slopes: &[f64; TEST_COUNT],
    j_intersect_cusp: &[f64; TEST_COUNT],
    top_gamma_inv: f64,
    peak_luminance: f64,
    limit_j_max: f64,
    lower_hull_gamma_inv: f64,
    limit_params: &JMhParams,
) -> bool {
    let luminance_limit = peak_luminance / REF_LUMINANCE;

    for test_index in 0..TEST_COUNT {
        let approx_limit_m = find_gamut_boundary_intersection(
            jm_cusp,
            limit_j_max,
            top_gamma_inv,
            lower_hull_gamma_inv,
            j_intersect_source[test_index],
            slopes[test_index],
            j_intersect_cusp[test_index],
        );
        let approx_limit_j = j_intersect_source[test_index] + slopes[test_index] * approx_limit_m;

        let approximate_jmh = [approx_limit_j, approx_limit_m, test_jmh[test_index][2]];

        let new_limit_rgb = jmh_to_rgb(approximate_jmh, limit_params);

        // If no value exceeds the luminance limit we are inside the top
        // gamut shell — this gamma does not fit.
        if !outside_hull(new_limit_rgb, luminance_limit) {
            return false;
        }
    }

    true
}

// from Lib.Academy.OutputTransform.ctl — per-hue upper-hull gamma via
// coarse march + bisection on the gamma fit.
fn make_upper_hull_gamma_table(
    gamut_cusp_table: &[[f64; 3]; TOTAL_TABLE_SIZE],
    p: &OdtParams,
) -> [f64; TOTAL_TABLE_SIZE] {
    let mut upper_hull_gamma = [0.0f64; TOTAL_TABLE_SIZE];

    for i in BASE_INDEX..(BASE_INDEX + TABLE_SIZE) {
        let hue = gamut_cusp_table[i][2];
        let jm_cusp = [gamut_cusp_table[i][0], gamut_cusp_table[i][1]];

        let mut test_jmh = [[0.0f64; 3]; TEST_COUNT];
        let mut j_intersect_source = [0.0f64; TEST_COUNT];
        let mut slopes = [0.0f64; TEST_COUNT];
        let mut j_intersect_cusp = [0.0f64; TEST_COUNT];

        generate_gamma_test_data(
            jm_cusp,
            hue,
            p.limit_j_max,
            p.mid_j,
            p.focus_dist,
            &mut test_jmh,
            &mut j_intersect_source,
            &mut slopes,
            &mut j_intersect_cusp,
        );

        let search_range = GAMMA_SEARCH_STEP;
        let mut low = GAMMA_MINIMUM;
        let mut high = low + search_range;
        let mut outside = false;
        while !outside && (high < GAMMA_MAXIMUM) {
            let gamma_found = evaluate_gamma_fit(
                jm_cusp,
                &test_jmh,
                &j_intersect_source,
                &slopes,
                &j_intersect_cusp,
                1.0 / high,
                p.peak_luminance,
                p.limit_j_max,
                p.lower_hull_gamma_inv,
                &p.limit_params,
            );
            if !gamma_found {
                low = high;
                high += search_range;
            } else {
                outside = true;
            }
        }

        while (high - low) > GAMMA_ACCURACY {
            let test_gamma = midpoint(high, low);
            let gamma_found = evaluate_gamma_fit(
                jm_cusp,
                &test_jmh,
                &j_intersect_source,
                &slopes,
                &j_intersect_cusp,
                1.0 / test_gamma,
                p.peak_luminance,
                p.limit_j_max,
                p.lower_hull_gamma_inv,
                &p.limit_params,
            );
            if gamma_found {
                high = test_gamma;
            } else {
                low = test_gamma;
            }
        }

        upper_hull_gamma[i] = 1.0 / high;
    }

    // Copy last populated entry to first empty spot
    upper_hull_gamma[0] = upper_hull_gamma[TABLE_SIZE];

    // Copy first populated entry to last empty spot
    upper_hull_gamma[TABLE_SIZE + BASE_INDEX] = upper_hull_gamma[BASE_INDEX];

    upper_hull_gamma
}

// from Lib.Academy.OutputTransform.ctl — largest deviations of the hue
// table from a linear distribution, to narrow the per-pixel binary search.
fn determine_hue_linearity_search_range(hue_table: &[f64; TOTAL_TABLE_SIZE]) -> [i64; 2] {
    const LOWER_PADDING: i64 = 0;
    const UPPER_PADDING: i64 = 1;

    let mut range = [LOWER_PADDING, UPPER_PADDING];

    for (i, &h) in hue_table
        .iter()
        .enumerate()
        .skip(BASE_INDEX)
        .take(TABLE_SIZE)
    {
        let pos = hue_position_in_uniform_table(h, TOTAL_TABLE_SIZE);
        let delta = i as i64 - pos;
        range[0] = range[0].min(delta + LOWER_PADDING);
        range[1] = range[1].max(delta + UPPER_PADDING);
    }

    range
}

// from Lib.Academy.OutputTransform.ctl — full parameter pre-computation,
// field-fill order preserved.
pub fn init_odt_params(peak_luminance: f64, limiting_primaries: &Chromaticities) -> OdtParams {
    let input_params = init_jmh_params(&AP0);
    let reach_params = init_jmh_params(&REACH_PRI);
    let limit_params = init_jmh_params(limiting_primaries);

    let ts = init_ts_params(peak_luminance);

    let limit_j_max = y_to_j(peak_luminance, &input_params);
    let model_gamma_inv = 1.0 / model_gamma();

    // AP0/AP1 conversion matrices (upstream translation-unit constants).
    let ap0_rgb_to_xyz = super::utilities::rgb_to_xyz_f33(&AP0, 1.0);
    let ap0_xyz_to_rgb = xyz_to_rgb_f33(&AP0, 1.0);
    let ap1_rgb_to_xyz = super::utilities::rgb_to_xyz_f33(&AP1, 1.0);
    let ap1_xyz_to_rgb = xyz_to_rgb_f33(&AP1, 1.0);
    let ap0_to_ap1 = mult_f33_f33(&ap0_rgb_to_xyz, &ap1_xyz_to_rgb);
    let ap1_to_ap0 = mult_f33_f33(&ap1_rgb_to_xyz, &ap0_xyz_to_rgb);

    let mut p = OdtParams {
        peak_luminance,
        input_params,
        reach_params,
        limit_params,
        ts,
        limit_j_max,
        model_gamma_inv,
        table_reach_m: [0.0; TOTAL_TABLE_SIZE],
        // Chroma compression parameters
        sat: ctl_max(
            0.2,
            CHROMA_EXPAND - (CHROMA_EXPAND * CHROMA_EXPAND_FACT) * ts.log_peak,
        ),
        sat_thr: CHROMA_EXPAND_THR / peak_luminance,
        compr: CHROMA_COMPRESS + (CHROMA_COMPRESS * CHROMA_COMPRESS_FACT) * ts.log_peak,
        chroma_compress_scale: (0.03379 * peak_luminance).powf(0.30596) - 0.45135,
        // Gamut compression parameters
        mid_j: y_to_j(ts.c_t * REF_LUMINANCE, &input_params),
        focus_dist: FOCUS_DISTANCE + FOCUS_DISTANCE * FOCUS_DISTANCE_SCALING * ts.log_peak,
        lower_hull_gamma_inv: 1.0 / (1.14 + 0.07 * ts.log_peak),
        table_hues: [0.0; TOTAL_TABLE_SIZE],
        table_gamut_cusps: [[0.0; 3]; TOTAL_TABLE_SIZE],
        table_upper_hull_gamma: [0.0; TOTAL_TABLE_SIZE],
        hue_linearity_search_range: [0, 1],
        ap0_to_ap1,
        ap1_to_ap0,
    };

    p.table_reach_m = make_reach_m_table(&p.reach_params, p.limit_j_max);

    let cusps = make_uniform_hue_gamut_table(&p.reach_params, &p.limit_params, &p);
    p.table_gamut_cusps = cusps;
    for i in 0..TOTAL_TABLE_SIZE {
        p.table_hues[i] = p.table_gamut_cusps[i][2];
    }
    let upper_hull = make_upper_hull_gamma_table(&p.table_gamut_cusps, &p);
    p.table_upper_hull_gamma = upper_hull;
    p.hue_linearity_search_range = determine_hue_linearity_search_range(&p.table_hues);

    p
}

// from Lib.Academy.OutputTransform.ctl — forward core: scene-linear ACES
// (AP0) → display-linear RGB in the limiting primaries (display encoding is
// a separate step, Lib.Academy.DisplayEncoding.ctl).
pub fn output_transform_fwd(aces: [f64; 3], p: &OdtParams) -> [f64; 3] {
    let ap0_clamped = clamp_ap0_to_ap1(aces, 0.0, p.ts.forward_limit, &p.ap0_to_ap1, &p.ap1_to_ap0);

    let jmh = rgb_to_jmh(ap0_clamped, &p.input_params);

    let tonemapped_jmh = tonemap_and_compress_fwd(jmh, p);

    let compressed_jmh = gamut_compress_fwd(tonemapped_jmh, p);

    jmh_to_rgb(compressed_jmh, &p.limit_params)
}

// from Lib.Academy.OutputTransform.ctl — inverse core: display-linear RGB
// in the limiting primaries → scene-linear ACES (AP0). The gamut and
// chroma stages invert analytically-approximated forward stages, so the
// round trip is close but not bit-exact (bounds pinned in the mod tests).
pub fn output_transform_inv(rgb_out: [f64; 3], p: &OdtParams) -> [f64; 3] {
    let compressed_jmh = rgb_to_jmh(rgb_out, &p.limit_params);

    let tonemapped_jmh = gamut_compress_inv(compressed_jmh, p);

    let jmh = tonemap_and_compress_inv(tonemapped_jmh, p);

    jmh_to_rgb(jmh, &p.input_params)
}
