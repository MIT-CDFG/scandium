// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Ported from rgb2spec (https://github.com/mitsuba-renderer/rgb2spec,
// commit 15ca298354772794a1d955945d848a28d75ce0f1, vendored at
// appname-proto/vendor/rgb2spec/): rgb2spec_opt.cpp (quadrature tables +
// Levenberg-Marquardt sigmoid fit, i.e. the offline LUT bake) and
// rgb2spec.c (model fetch: mono shortcut, trilinear lookup, sigmoid eval).
// BSD-3-Clause, Copyright (c) 2020 Wenzel Jakob. License text carried in
// THIRD-PARTY-NOTICES.md and vendor/rgb2spec/LICENSE; the bake tool's
// help/About surface additionally credits rgb2spec per the census
// obligations row.

//! Spec: ALGORITHMS.md §23 `paint_engine` — `pigment: {primary:
//! kubelka_munk_spectral, basis: [jakob19, spectraljs], luts:
//! own_baked_33cube_f16}`; ES §22.3 pigment mode; INTEGRATION-NOTES §6
//! order 3a (`rgb2spec` LUT bake).
//!
//! Jakob–Hanika low-dimensional spectral upsampling: RGB → 3-coefficient
//! sigmoid spectra. This module carries BOTH halves:
//!
//! * the **offline bake** ([`bake`]): Simpson-3/8 quadrature tables over the
//!   CIE curves + D65, and the Levenberg–Marquardt fit per LUT node —
//!   producing the own-baked coefficient tables the contract's
//!   `own_baked_33cube_f16` asset names (the f16 quantization step is the
//!   packaging concern of the asset pipeline; the bake emits f32);
//! * the **runtime fetch** ([`Rgb2SpecModel::fetch`]): monochromatic
//!   closed form + trilinear coefficient interpolation, and the sigmoid
//!   spectrum evaluation ([`eval_precise`]).
//!
//! Determinism: the bake is single-threaded here (upstream's thread pool is
//! deliberately not ported — LUT bake determinism joins the asset
//! provenance chain); all math is scalar f64 with fixed iteration order.

use super::cie::{
    cie_interp, CIE_D65, CIE_LAMBDA_MAX, CIE_LAMBDA_MIN, CIE_SAMPLES, CIE_X, CIE_Y, CIE_Z,
    SRGB_TO_XYZ, XYZ_TO_SRGB,
};

/// Fine quadrature sample count: `(CIE_SAMPLES - 1) * 3 + 1` (Simpson 3/8).
const CIE_FINE_SAMPLES: usize = (CIE_SAMPLES - 1) * 3 + 1;

/// Number of sigmoid polynomial coefficients.
pub const N_COEFFS: usize = 3;

/// Precomputed spectral-integration tables (port of `init_tables`, sRGB/D65
/// path).
pub struct Tables {
    lambda: Vec<f64>,
    rgb_tbl: [Vec<f64>; 3],
    rgb_to_xyz: [[f64; 3]; 3],
    whitepoint: [f64; 3],
}

impl Tables {
    /// Build the quadrature tables for the sRGB gamut under D65.
    pub fn new_srgb() -> Tables {
        let h = (CIE_LAMBDA_MAX - CIE_LAMBDA_MIN) / (CIE_FINE_SAMPLES - 1) as f64;
        let mut lambda = vec![0f64; CIE_FINE_SAMPLES];
        let mut rgb_tbl = [
            vec![0f64; CIE_FINE_SAMPLES],
            vec![0f64; CIE_FINE_SAMPLES],
            vec![0f64; CIE_FINE_SAMPLES],
        ];
        let mut whitepoint = [0f64; 3];
        for i in 0..CIE_FINE_SAMPLES {
            let lam = CIE_LAMBDA_MIN + i as f64 * h;
            let xyz = [
                cie_interp(&CIE_X, lam),
                cie_interp(&CIE_Y, lam),
                cie_interp(&CIE_Z, lam),
            ];
            let illum = cie_interp(&CIE_D65, lam);
            let mut weight = 3.0 / 8.0 * h;
            if i == 0 || i == CIE_FINE_SAMPLES - 1 {
                // endpoint weight stays 3h/8
            } else if (i - 1) % 3 == 2 {
                weight *= 2.0;
            } else {
                weight *= 3.0;
            }
            lambda[i] = lam;
            for k in 0..3 {
                let mut acc = 0.0;
                for j in 0..3 {
                    acc += XYZ_TO_SRGB[k][j] * xyz[j] * illum * weight;
                }
                rgb_tbl[k][i] = acc;
            }
            for c in 0..3 {
                whitepoint[c] += xyz[c] * illum * weight;
            }
        }
        Tables {
            lambda,
            rgb_tbl,
            rgb_to_xyz: SRGB_TO_XYZ,
            whitepoint,
        }
    }

    /// CIELab of a linear-RGB triple under the table's whitepoint (port of
    /// `cie_lab`).
    fn cie_lab(&self, p: [f64; 3]) -> [f64; 3] {
        let mut xyz = [0f64; 3];
        for (a, xv) in xyz.iter_mut().enumerate() {
            for (j, pv) in p.iter().enumerate() {
                *xv += pv * self.rgb_to_xyz[a][j];
            }
        }
        let f = |t: f64| -> f64 {
            let delta: f64 = 6.0 / 29.0;
            if t > delta * delta * delta {
                t.cbrt()
            } else {
                t / (delta * delta * 3.0) + 4.0 / 29.0
            }
        };
        let fx = f(xyz[0] / self.whitepoint[0]);
        let fy = f(xyz[1] / self.whitepoint[1]);
        let fz = f(xyz[2] / self.whitepoint[2]);
        [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
    }

    /// Integrate the sigmoid spectrum of scaled-domain `coeffs` against the
    /// RGB tables (shared by residual + Jacobian evaluation).
    fn integrate(&self, coeffs: &[f64; 3]) -> ([f64; 3], [[f64; 3]; 3]) {
        let lo = CIE_LAMBDA_MIN;
        let inv = 1.0 / (CIE_LAMBDA_MAX - CIE_LAMBDA_MIN);
        let mut out = [0f64; 3];
        let mut dout = [[0f64; 3]; 3];
        for i in 0..CIE_FINE_SAMPLES {
            let lambda = (self.lambda[i] - lo) * inv;
            let x = (coeffs[0] * lambda + coeffs[1]) * lambda + coeffs[2];
            let q = 1.0 / (1.0 + x * x).sqrt();
            let s = 0.5 * x * q + 0.5;
            let sp = 0.5 * q * q * q;
            let dx0 = lambda * lambda;
            let dx1 = lambda;
            for j in 0..3 {
                let w = self.rgb_tbl[j][i];
                out[j] += w * s;
                let wsp = w * sp;
                dout[j][0] += wsp * dx0;
                dout[j][1] += wsp * dx1;
                dout[j][2] += wsp;
            }
        }
        (out, dout)
    }

    /// Residual in CIELab (port of `eval_residual`).
    fn eval_residual(&self, coeffs: &[f64; 3], rgb: [f64; 3]) -> [f64; 3] {
        let (out, _) = self.integrate(coeffs);
        let out_lab = self.cie_lab(out);
        let tgt_lab = self.cie_lab(rgb);
        [
            tgt_lab[0] - out_lab[0],
            tgt_lab[1] - out_lab[1],
            tgt_lab[2] - out_lab[2],
        ]
    }

    /// Residual + analytic Jacobian (port of `eval_residual_jac` +
    /// `cie_lab_jac`).
    fn eval_residual_jac(&self, coeffs: &[f64; 3], rgb: [f64; 3]) -> ([f64; 3], [[f64; 3]; 3]) {
        let (out, dout) = self.integrate(coeffs);

        // Lab + Jacobian at the reproduced color.
        let mut xyz = [0f64; 3];
        for (a, xv) in xyz.iter_mut().enumerate() {
            for (j, ov) in out.iter().enumerate() {
                *xv += ov * self.rgb_to_xyz[a][j];
            }
        }
        let mut fv = [0f64; 3];
        let mut gd = [0f64; 3];
        for k in 0..3 {
            let t = xyz[k] / self.whitepoint[k];
            let delta: f64 = 6.0 / 29.0;
            if t > delta * delta * delta {
                let cr = t.cbrt();
                fv[k] = cr;
                gd[k] = (1.0 / 3.0) / (cr * cr * self.whitepoint[k]);
            } else {
                fv[k] = t / (delta * delta * 3.0) + 4.0 / 29.0;
                gd[k] = 1.0 / (delta * delta * 3.0 * self.whitepoint[k]);
            }
        }
        let out_lab = [
            116.0 * fv[1] - 16.0,
            500.0 * (fv[0] - fv[1]),
            200.0 * (fv[1] - fv[2]),
        ];
        let g = [
            [0.0, 116.0 * gd[1], 0.0],
            [500.0 * gd[0], -500.0 * gd[1], 0.0],
            [0.0, 200.0 * gd[1], -200.0 * gd[2]],
        ];
        let mut lab_jac = [[0f64; 3]; 3];
        for (a, row) in lab_jac.iter_mut().enumerate() {
            for (b, cell) in row.iter_mut().enumerate() {
                let mut s = 0.0;
                for (k, gk) in g[a].iter().enumerate() {
                    s += gk * self.rgb_to_xyz[k][b];
                }
                *cell = s;
            }
        }

        let tgt_lab = self.cie_lab(rgb);
        let residual = [
            tgt_lab[0] - out_lab[0],
            tgt_lab[1] - out_lab[1],
            tgt_lab[2] - out_lab[2],
        ];
        let mut jac = [[0f64; 3]; 3];
        for a in 0..3 {
            for b in 0..3 {
                let mut s = 0.0;
                for k in 0..3 {
                    s += lab_jac[a][k] * dout[k][b];
                }
                jac[a][b] = -s;
            }
        }
        (residual, jac)
    }
}

/// Solve 3x3 `A x = b` by Gaussian elimination with partial pivoting.
/// Returns `None` if singular (port of `rgb2spec_solve3` / the LUP pair).
fn solve3(a: [[f64; 3]; 3], b: [f64; 3]) -> Option<[f64; 3]> {
    let mut m = [
        [a[0][0], a[0][1], a[0][2], b[0]],
        [a[1][0], a[1][1], a[1][2], b[1]],
        [a[2][0], a[2][1], a[2][2], b[2]],
    ];
    for c in 0..3 {
        let mut p = c;
        for r in c + 1..3 {
            if m[r][c].abs() > m[p][c].abs() {
                p = r;
            }
        }
        if m[p][c].abs() < 1e-30 {
            return None;
        }
        m.swap(c, p);
        for r in 0..3 {
            if r != c {
                let fct = m[r][c] / m[c][c];
                // Row operation reads pivot row `c` while writing row `r`:
                // two rows of `m` at once, the index math is load-bearing.
                #[allow(clippy::needless_range_loop)]
                for k in c..4 {
                    m[r][k] -= fct * m[c][k];
                }
            }
        }
    }
    Some([m[0][3] / m[0][0], m[1][3] / m[1][1], m[2][3] / m[2][2]])
}

/// Levenberg–Marquardt sigmoid fit (port of `LM`): find the coefficients
/// whose spectrum reproduces `rgb` with minimal squared CIELab distance.
/// Returns the final residual norm.
pub fn lm_fit(tables: &Tables, rgb: [f64; 3], coeffs: &mut [f64; 3], iterations: usize) -> f64 {
    let (mut residual, mut jac) = tables.eval_residual_jac(coeffs, rgb);
    let mut cost =
        residual[0] * residual[0] + residual[1] * residual[1] + residual[2] * residual[2];
    let mut lambda = 1e-3f64;
    for _ in 0..iterations {
        if cost <= 1e-12 {
            break;
        }
        let mut a = [[0f64; 3]; 3];
        let mut g = [0f64; 3];
        for i in 0..3 {
            for j in 0..3 {
                let mut s = 0.0;
                for jrow in &jac {
                    s += jrow[i] * jrow[j];
                }
                a[i][j] = s;
            }
            for (jrow, res) in jac.iter().zip(&residual) {
                g[i] += jrow[i] * res;
            }
        }
        let mut accepted = false;
        for _ in 0..10 {
            let mut m = a;
            for (d, row) in m.iter_mut().enumerate() {
                row[d] += lambda;
            }
            if let Some(step) = solve3(m, g) {
                let trial = [
                    coeffs[0] - step[0],
                    coeffs[1] - step[1],
                    coeffs[2] - step[2],
                ];
                let tr = tables.eval_residual(&trial, rgb);
                let tc = tr[0] * tr[0] + tr[1] * tr[1] + tr[2] * tr[2];
                if tc < cost {
                    *coeffs = trial;
                    cost = tc;
                    lambda = (lambda * 0.5).max(1e-12);
                    accepted = true;
                    break;
                }
            }
            lambda *= 10.0;
            if lambda > 1e12 {
                break;
            }
        }
        if !accepted {
            break;
        }
        let (r, j) = tables.eval_residual_jac(coeffs, rgb);
        residual = r;
        jac = j;
    }
    cost.sqrt()
}

/// A baked RGB2Spec coefficient model (in-memory equivalent of the upstream
/// `.spec` payload: scale nodes + `3 * res³ * 3` coefficients).
pub struct Rgb2SpecModel {
    /// LUT resolution per axis.
    pub res: usize,
    /// Non-uniform max-component scale nodes (double smoothstep).
    pub scale: Vec<f32>,
    /// Coefficients `data[((l * res + z) * res + y) * res + x) * 3 + j]`,
    /// wavelength-domain.
    pub data: Vec<f32>,
}

/// Bake a coefficient model at `res³` (port of the `rgb2spec_opt` main
/// loop, sRGB gamut, single-threaded, coefficient continuation along the
/// scale axis exactly as upstream: ascending from `res/5`, then descending).
pub fn bake(tables: &Tables, res: usize) -> Rgb2SpecModel {
    let smoothstep = |x: f64| x * x * (3.0 - 2.0 * x);
    let scale: Vec<f32> = (0..res)
        .map(|k| smoothstep(smoothstep(k as f64 / (res - 1) as f64)) as f32)
        .collect();
    let mut out = vec![0f32; 3 * res * res * res * 3];

    let c0 = 360.0f64;
    let c1 = 1.0 / (830.0 - 360.0);

    for l in 0..3 {
        for j in 0..res {
            let y = j as f64 / (res - 1) as f64;
            for i in 0..res {
                let x = i as f64 / (res - 1) as f64;
                let mut coeffs = [0f64; 3];
                let start = res / 5;
                let store = |k: usize, coeffs: &[f64; 3], out: &mut Vec<f32>| {
                    let a = coeffs[0];
                    let b = coeffs[1];
                    let c = coeffs[2];
                    let idx = ((l * res + k) * res + j) * res + i;
                    out[3 * idx] = (a * (c1 * c1)) as f32;
                    out[3 * idx + 1] = (b * c1 - 2.0 * a * c0 * (c1 * c1)) as f32;
                    out[3 * idx + 2] = (c - b * c0 * c1 + a * (c0 * c1) * (c0 * c1)) as f32;
                };
                for (k, &sv) in scale.iter().enumerate().take(res).skip(start) {
                    let b = sv as f64;
                    let mut rgb = [0f64; 3];
                    rgb[l] = b;
                    rgb[(l + 1) % 3] = x * b;
                    rgb[(l + 2) % 3] = y * b;
                    lm_fit(tables, rgb, &mut coeffs, 15);
                    store(k, &coeffs, &mut out);
                }
                coeffs = [0f64; 3];
                for k in (0..=start.min(res - 1)).rev() {
                    let b = scale[k] as f64;
                    let mut rgb = [0f64; 3];
                    rgb[l] = b;
                    rgb[(l + 1) % 3] = x * b;
                    rgb[(l + 2) % 3] = y * b;
                    lm_fit(tables, rgb, &mut coeffs, 15);
                    store(k, &coeffs, &mut out);
                }
            }
        }
    }
    Rgb2SpecModel {
        res,
        scale,
        data: out,
    }
}

impl Rgb2SpecModel {
    /// Monochromatic closed form (port of `rgb2spec_fetch_mono`).
    fn fetch_mono(rgb: [f32; 3]) -> Option<[f32; N_COEFFS]> {
        if rgb[0] != rgb[1] || rgb[1] != rgb[2] {
            return None;
        }
        let v = rgb[0];
        let r = if v <= 0.0 {
            -8192.0
        } else if v >= 1.0 {
            8192.0
        } else {
            (v - 0.5) / (v * (1.0 - v)).sqrt()
        };
        Some([0.0, 0.0, r])
    }

    /// Interval search over the scale nodes (port of
    /// `rgb2spec_find_interval`).
    fn find_interval(&self, z: f32) -> usize {
        let values = &self.scale;
        let mut left = 0usize;
        let last_interval = values.len() - 2;
        let mut size = last_interval;
        while size > 0 {
            let half = size >> 1;
            let middle = left + half + 1;
            if values[middle] <= z {
                left = middle;
                size -= half + 1;
            } else {
                size = half;
            }
        }
        left.min(last_interval)
    }

    /// RGB -> sigmoid coefficients (port of `rgb2spec_fetch`): mono
    /// shortcut, else trilinear interpolation over the max-component
    /// parameterization.
    pub fn fetch(&self, rgb_in: [f32; 3]) -> [f32; N_COEFFS] {
        let rgb = [
            rgb_in[0].clamp(0.0, 1.0),
            rgb_in[1].clamp(0.0, 1.0),
            rgb_in[2].clamp(0.0, 1.0),
        ];
        if let Some(out) = Self::fetch_mono(rgb) {
            return out;
        }
        let res = self.res;
        let mut i = 0usize;
        for j in 1..3 {
            if rgb[j] >= rgb[i] {
                i = j;
            }
        }
        let z = rgb[i];
        if z <= 0.0 {
            return [self.data[0], self.data[1], self.data[2]];
        }
        let scale = (res - 1) as f32 / z;
        let x = rgb[(i + 1) % 3] * scale;
        let y = rgb[(i + 2) % 3] * scale;
        let xi = (x as usize).min(res - 2);
        let yi = (y as usize).min(res - 2);
        let zi = self.find_interval(z);
        let n = N_COEFFS;
        let offset = (((i * res + zi) * res + yi) * res + xi) * n;
        let dx = n;
        let dy = n * res;
        let dz = n * res * res;
        let x1 = x - xi as f32;
        let x0 = 1.0 - x1;
        let y1 = y - yi as f32;
        let y0 = 1.0 - y1;
        let z1 = (z - self.scale[zi]) / (self.scale[zi + 1] - self.scale[zi]);
        let z0 = 1.0 - z1;
        let mut out = [0f32; N_COEFFS];
        for (offset, o) in (offset..).zip(out.iter_mut()) {
            let d = &self.data;
            *o = ((d[offset] * x0 + d[offset + dx] * x1) * y0
                + (d[offset + dy] * x0 + d[offset + dy + dx] * x1) * y1)
                * z0
                + ((d[offset + dz] * x0 + d[offset + dz + dx] * x1) * y0
                    + (d[offset + dz + dy] * x0 + d[offset + dz + dy + dx] * x1) * y1)
                    * z1;
        }
        out
    }
}

/// Evaluate the sigmoid spectrum at wavelength `lambda` nm (port of
/// `rgb2spec_eval_precise`).
pub fn eval_precise(coeff: [f32; N_COEFFS], lambda: f32) -> f32 {
    let x = (coeff[0] * lambda + coeff[1]) * lambda + coeff[2];
    let y = 1.0 / (x * x + 1.0).sqrt();
    0.5 * x * y + 0.5
}

/// Integrate a coefficient spectrum back to linear sRGB using the bake
/// tables (round-trip check used by tests and the LUT bake validation).
pub fn spectrum_to_srgb(tables: &Tables, coeff: [f32; N_COEFFS]) -> [f64; 3] {
    let mut out = [0f64; 3];
    for i in 0..CIE_FINE_SAMPLES {
        let s = eval_precise(coeff, tables.lambda[i] as f32) as f64;
        for (j, ov) in out.iter_mut().enumerate() {
            *ov += tables.rgb_tbl[j][i] * s;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plane::plane_hash;

    #[test]
    fn whitepoint_is_d65_normalized() {
        let t = Tables::new_srgb();
        // D65 is unit-luminance normalized, so the Y integral is ~1.
        assert!(
            (t.whitepoint[1] - 1.0).abs() < 1e-3,
            "Y whitepoint = {}",
            t.whitepoint[1]
        );
    }

    #[test]
    fn lm_fit_reproduces_in_gamut_colors() {
        let t = Tables::new_srgb();
        // Mid-gamut colors are exactly reachable: residual ~ 0.
        for rgb in [[0.3f64, 0.5, 0.2], [0.6, 0.4, 0.5], [0.2, 0.2, 0.7]] {
            let mut coeffs = [0f64; 3];
            let resid = lm_fit(&t, rgb, &mut coeffs, 25);
            assert!(resid < 1e-4, "residual {resid} for {rgb:?}");
        }
    }

    #[test]
    fn bake_and_fetch_round_trip() {
        // Small-resolution bake (res 9): fetch + spectral integration must
        // reproduce arbitrary in-gamut colors within LUT-interpolation
        // tolerance. Upstream ships res 64 and the engine asset is the
        // 33-cube LUT (asset pipeline); the test LUT trades resolution for
        // speed, so the tolerance is the res-9 interpolation bound observed
        // at port time (~0.023 worst channel), not the shipping accuracy.
        let t = Tables::new_srgb();
        let model = bake(&t, 9);
        for rgb in [
            [0.7f32, 0.3, 0.2],
            [0.25, 0.55, 0.35],
            [0.15, 0.3, 0.6],
            [0.8, 0.7, 0.3],
        ] {
            let coeff = model.fetch(rgb);
            let back = spectrum_to_srgb(&t, coeff);
            for c in 0..3 {
                assert!(
                    (back[c] - rgb[c] as f64).abs() < 0.03,
                    "{rgb:?} -> {back:?} channel {c}"
                );
            }
        }
    }

    #[test]
    fn mono_shortcut_hits_black_white_and_grays() {
        let t = Tables::new_srgb();
        let model = bake(&t, 5);
        // Black and white pin to +/- 8192 curvature-free spectra.
        assert_eq!(model.fetch([0.0; 3]), [0.0, 0.0, -8192.0]);
        assert_eq!(model.fetch([1.0; 3]), [0.0, 0.0, 8192.0]);
        // A mid gray becomes a flat spectrum whose value reproduces it.
        let coeff = model.fetch([0.5; 3]);
        let flat = eval_precise(coeff, 550.0);
        let back = spectrum_to_srgb(&t, coeff);
        assert!(
            (back[1] - 0.5).abs() < 5e-3,
            "gray -> {back:?} (flat {flat})"
        );
    }

    #[test]
    fn bake_is_deterministic() {
        let t = Tables::new_srgb();
        let m1 = bake(&t, 5);
        let m2 = bake(&t, 5);
        assert_eq!(plane_hash(&m1.data), plane_hash(&m2.data));
        assert_eq!(plane_hash(&m1.scale), plane_hash(&m2.scale));
    }
}
