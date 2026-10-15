// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Derived by translation from ACES Core Transforms (aces-dev),
// https://github.com/ampas/aces-dev, commit
// 5ac93ba2d48a00aca6e4811fa0b700911c2a8809 (census key `aces-dev`):
// lib/Lib.Academy.Tonescale.ctl. Apache-2.0 at the pin (LICENSE sha256
// 43070e2d4e532684de521b885f385d0841030efa2b1a20bafb76133a5e1379c1, read
// first-hand 2026-08-13). Per Apache-2.0 Section 4(c) the upstream per-file
// notice "Copyright Contributors to the ACES Project." is retained in this
// header; repo-level notice carriage per the `appname.port-record.v1` draft
// (reports/O-ALGO-log.md, Item 11; O-BUILD ratification pending).

//! ACES 2.0 tonescale: parameter pre-calculation and the forward/inverse
//! curve (Michaelis–Menten-style sigmoid plus a flare/glare toe).

use super::utilities::{ctl_max, ctl_min};

/// Tonescale parameter block. from Lib.Academy.Tonescale.ctl `TSParams`.
#[derive(Clone, Copy, Debug)]
pub struct TsParams {
    pub n: f64,
    pub n_r: f64,
    pub g: f64,
    pub t_1: f64,
    pub c_t: f64,
    pub s_2: f64,
    pub u_2: f64,
    pub m_2: f64,
    pub forward_limit: f64,
    pub inverse_limit: f64,
    pub log_peak: f64,
}

// from Lib.Academy.Tonescale.ctl — tonescale pre-calculations.
pub fn init_ts_params(peak_luminance: f64) -> TsParams {
    // Preset constants that set the desired behavior for the curve
    let n = peak_luminance;

    let n_r = 100.0; // normalized white in nits (what 1.0 should be)
    let g = 1.15; // surround / contrast
    let c = 0.18; // anchor for 18% grey
    let c_d = 10.013; // output luminance of 18% grey (in nits)
    let w_g = 0.14; // change in grey between different peak luminance
    let t_1 = 0.04; // shadow toe or flare/glare compensation
    let r_hit_min = 128.0; // scene-referred value "hitting the roof" for SDR (e.g. when n = 100 nits)
    let r_hit_max = 896.0; // scene-referred value "hitting the roof" for when n = 10000 nits

    // Calculate output constants
    let r_hit = r_hit_min + (r_hit_max - r_hit_min) * ((n / n_r).ln() / (10000.0f64 / 100.0).ln());
    let m_0 = n / n_r;
    let m_1 = 0.5 * (m_0 + (m_0 * (m_0 + 4.0 * t_1)).sqrt());
    let u = ((r_hit / m_1) / ((r_hit / m_1) + 1.0)).powf(g);
    let m = m_1 / u;
    let w_i = (n / 100.0).ln() / 2.0f64.ln();
    let c_t = c_d / n_r * (1.0 + w_i * w_g);
    let g_ip = 0.5 * (c_t + (c_t * (c_t + 4.0 * t_1)).sqrt());
    let g_ipp2 = -(m_1 * (g_ip / m).powf(1.0 / g)) / ((g_ip / m).powf(1.0 / g) - 1.0);
    let w_2 = c / g_ipp2;
    let s_2 = w_2 * m_1;
    let u_2 = ((r_hit / m_1) / ((r_hit / m_1) + w_2)).powf(g);
    let m_2 = m_1 / u_2;

    TsParams {
        n,
        n_r,
        g,
        t_1,
        c_t,
        s_2,
        u_2,
        m_2,
        forward_limit: 8.0 * r_hit,
        inverse_limit: n / (u_2 * n_r),
        log_peak: (n / n_r).log10(),
    }
}

// from Lib.Academy.Tonescale.ctl — forward MM tone scale + forward flare.
// Input: scene-referred linear (ACES2065-1 scale); output: luminance cd/m².
pub fn tonescale_fwd(x: f64, params: &TsParams) -> f64 {
    let f = params.m_2 * (ctl_max(0.0, x) / (x + params.s_2)).powf(params.g);
    let h = ctl_max(0.0, f * f / (f + params.t_1)); // forward flare

    h * params.n_r // output is luminance in cd/m^2
}

// from Lib.Academy.Tonescale.ctl — inverse: luminance → scene linear.
// NOTE on units: the upstream comment says "luminance in cd/m^2", but the
// only call site (Lib.Academy.OutputTransform.ctl `tonemap_and_compress_inv`)
// passes `luminance / ref_luminance`, i.e. luminance normalized by n_r —
// consistent with the clamp `n / (u_2 · n_r)` here. Translated as-called.
pub fn tonescale_inv(y: f64, params: &TsParams) -> f64 {
    let z = ctl_max(0.0, ctl_min(params.n / (params.u_2 * params.n_r), y));
    let h = (z + (z * (4.0 * params.t_1 + z)).sqrt()) / 2.0;

    params.s_2 / ((params.m_2 / h).powf(1.0 / params.g) - 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tonescale_monotone_over_domain() {
        // Strictly increasing over the scene-referred domain, checked on a
        // dense log sweep up to the forward_limit for SDR and HDR peaks.
        for peak in [100.0, 1000.0] {
            let p = init_ts_params(peak);
            let mut prev = tonescale_fwd(0.0, &p);
            assert_eq!(prev, 0.0, "black must map to 0 (flare toe passes 0)");
            let n = 4000;
            for i in 0..n {
                // log sweep 1e-6 .. forward_limit
                let t = i as f64 / (n - 1) as f64;
                let x = 1e-6 * (p.forward_limit / 1e-6f64).powf(t);
                let y = tonescale_fwd(x, &p);
                assert!(
                    y > prev,
                    "not strictly increasing at x={x} (peak {peak}): {y} <= {prev}"
                );
                prev = y;
            }
        }
    }

    #[test]
    fn tonescale_18_gray_anchor() {
        // Anchor: Lib.Academy.Tonescale.ctl documents the constants as
        // "c = 0.18; anchor for 18% grey" and "c_d = 10.013; output
        // luminance of 18% grey (in nits)". c_d is the *intermediate*
        // (first-pass) target: after the second normalization pass
        // (u/u_2 rescale) the final curve lands scene 0.18 on 10.000 nits
        // at peak 100 — the SDR 18%-grey → 10 nit mapping the ACES 2.0
        // documentation states. Measured here: 9.999932511 nits.
        let p = init_ts_params(100.0);
        let y = tonescale_fwd(0.18, &p);
        assert!(
            (y - 10.0).abs() < 1e-3,
            "18% grey mapped to {y} nits, expected ~10.0"
        );
    }

    #[test]
    fn tonescale_fwd_inv_round_trip() {
        let p = init_ts_params(100.0);
        let n = 500;
        for i in 0..n {
            let t = i as f64 / (n - 1) as f64;
            let x = 1e-4 * (100.0f64 / 1e-4).powf(t); // 1e-4 .. 100 scene-linear
            let y = tonescale_fwd(x, &p);
            // The inverse takes luminance normalized by n_r (see the units
            // note on `tonescale_inv`).
            let back = tonescale_inv(y / p.n_r, &p);
            assert!(
                ((back - x) / x).abs() < 1e-9,
                "round trip at x={x}: got {back}"
            );
        }
    }

    #[test]
    fn tonescale_limit_behavior() {
        let p = init_ts_params(100.0);
        // At the forward_limit clamp point the curve is within ~1% of peak,
        // and the asymptote stays bounded near peak (sigmoid ceiling m_2).
        let y_at_limit = tonescale_fwd(p.forward_limit, &p);
        assert!(y_at_limit > 0.99 * p.n);
        assert!(tonescale_fwd(1e9, &p) < 1.2 * p.n);
        // The inverse clamps at inverse_limit (normalized units) rather
        // than diverging.
        let inv_hi = tonescale_inv(1e6, &p);
        let inv_at_lim = tonescale_inv(p.inverse_limit, &p);
        assert!((inv_hi - inv_at_lim).abs() < 1e-12);
        assert!(inv_hi.is_finite());
    }
}
