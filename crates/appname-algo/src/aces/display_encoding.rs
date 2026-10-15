// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Derived by translation from ACES Core Transforms (aces-dev),
// https://github.com/ampas/aces-dev, commit
// 5ac93ba2d48a00aca6e4811fa0b700911c2a8809 (census key `aces-dev`):
// lib/Lib.Academy.DisplayEncoding.ctl. Apache-2.0 at the pin (LICENSE sha256
// 43070e2d4e532684de521b885f385d0841030efa2b1a20bafb76133a5e1379c1, read
// first-hand 2026-08-13). Per Apache-2.0 Section 4(c) the upstream per-file
// notice "Copyright Contributors to the ACES Project." is retained in this
// header; repo-level notice carriage per the `appname.port-record.v1` draft
// (reports/O-ALGO-log.md, Item 11; O-BUILD ratification pending).

//! Display encoding/decoding for the SDR output transforms: creative-white
//! scaling, the sRGB piecewise transfer (IEC 61966-2-1:1999 shape) and the
//! BT.1886 power EOTF, plus the encode/decode dispatch.
//!
//! Skipped from Lib.Academy.DisplayEncoding.ctl (no editor consumer — the
//! `hdr_tonemap` `aces_view_transform` row is the SDR display-referred
//! path; HDR PQ/HLG masters have no v1 surface): SMPTE range scaling
//! (`smpteRange_to_fullRange` pair), ST 2084/PQ (`ST2084_to_Y` family),
//! HLG (`ST2084_to_HLG_1000nits`/`HLG_to_ST2084_1000nits`), and the
//! gamma-2.6 / gamma-2.2 encode arms of the `eotf`/`eotf_inv` dispatch.

use super::utilities::{clamp_f3, ctl_max, f3_from_f, mult_f3_f33, mult_f_f3, M33};

// from Lib.Academy.DisplayEncoding.ctl — white point scaling.
// If the creative white differs from the calibration white of the display,
// the largest channel of the transformed white is scaled to hit 1.0 so all
// three channels "fit" within the peak value (avoiding a hue shift at
// white); inverted, the scaling is removed.
pub fn apply_white_scale(rgb: [f64; 3], mat_limit_to_display: &M33, invert: bool) -> [f64; 3] {
    let rgb_w_f = mult_f3_f33(f3_from_f(1.0), mat_limit_to_display);
    let scale = 1.0 / ctl_max(ctl_max(rgb_w_f[0], rgb_w_f[1]), rgb_w_f[2]);
    // scale factor is equal to 1/largestChannel

    if invert {
        clamp_f3(mult_f_f3(1.0 / scale, rgb), 0.0, 1.0)
    } else {
        mult_f_f3(scale, rgb)
    }
}

// from Lib.Academy.DisplayEncoding.ctl — forward monitor curve; with
// gamma=2.4, offset=0.055 this matches the inverse EOTF of
// IEC 61966-2-1:1999 (sRGB).
pub fn moncurve_fwd(x: f64, gamma: f64, offs: f64) -> f64 {
    let fs = ((gamma - 1.0) / offs) * (offs * gamma / ((gamma - 1.0) * (1.0 + offs))).powf(gamma);
    let xb = offs / (gamma - 1.0);
    if x >= xb {
        ((x + offs) / (1.0 + offs)).powf(gamma)
    } else {
        x * fs
    }
}

// from Lib.Academy.DisplayEncoding.ctl — reverse monitor curve.
pub fn moncurve_inv(y: f64, gamma: f64, offs: f64) -> f64 {
    let yb = (offs * gamma / ((gamma - 1.0) * (1.0 + offs))).powf(gamma);
    let rs = ((gamma - 1.0) / offs).powf(gamma - 1.0) * ((1.0 + offs) / gamma).powf(gamma);
    if y >= yb {
        (1.0 + offs) * y.powf(1.0 / gamma) - offs
    } else {
        y * rs
    }
}

// from Lib.Academy.DisplayEncoding.ctl
pub fn moncurve_fwd_f3(x: [f64; 3], gamma: f64, offs: f64) -> [f64; 3] {
    [
        moncurve_fwd(x[0], gamma, offs),
        moncurve_fwd(x[1], gamma, offs),
        moncurve_fwd(x[2], gamma, offs),
    ]
}

// from Lib.Academy.DisplayEncoding.ctl
pub fn moncurve_inv_f3(y: [f64; 3], gamma: f64, offs: f64) -> [f64; 3] {
    [
        moncurve_inv(y[0], gamma, offs),
        moncurve_inv(y[1], gamma, offs),
        moncurve_inv(y[2], gamma, offs),
    ]
}

// from Lib.Academy.DisplayEncoding.ctl — the forward OTF specified in
// Rec. ITU-R BT.1886: L = a·(max[(V+b),0])^g.
pub fn bt1886_fwd(v: f64, gamma: f64, lw: f64, lb: f64) -> f64 {
    let a = (lw.powf(1.0 / gamma) - lb.powf(1.0 / gamma)).powf(gamma);
    let b = lb.powf(1.0 / gamma) / (lw.powf(1.0 / gamma) - lb.powf(1.0 / gamma));
    a * ctl_max(v + b, 0.0).powf(gamma)
}

// from Lib.Academy.DisplayEncoding.ctl — the reverse EOTF of BT.1886.
pub fn bt1886_inv(l: f64, gamma: f64, lw: f64, lb: f64) -> f64 {
    let a = (lw.powf(1.0 / gamma) - lb.powf(1.0 / gamma)).powf(gamma);
    let b = lb.powf(1.0 / gamma) / (lw.powf(1.0 / gamma) - lb.powf(1.0 / gamma));
    ctl_max(l / a, 0.0).powf(1.0 / gamma) - b
}

// from Lib.Academy.DisplayEncoding.ctl
pub fn bt1886_fwd_f3(v: [f64; 3], gamma: f64, lw: f64, lb: f64) -> [f64; 3] {
    [
        bt1886_fwd(v[0], gamma, lw, lb),
        bt1886_fwd(v[1], gamma, lw, lb),
        bt1886_fwd(v[2], gamma, lw, lb),
    ]
}

// from Lib.Academy.DisplayEncoding.ctl
pub fn bt1886_inv_f3(l: [f64; 3], gamma: f64, lw: f64, lb: f64) -> [f64; 3] {
    [
        bt1886_inv(l[0], gamma, lw, lb),
        bt1886_inv(l[1], gamma, lw, lb),
        bt1886_inv(l[2], gamma, lw, lb),
    ]
}

/// Ported subset of the upstream `eotf_enum` dispatch (CTL enums 0, 4, 6).
/// The PQ (1), HLG (2), gamma-2.6 (3) and gamma-2.2 (5) arms are
/// deliberately not translated (see module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Eotf {
    /// CTL enum 0 — display linear (no encode).
    DisplayLinear,
    /// CTL enum 4 — BT.1886 with gamma 2.4 (Lw = 1, Lb = 0).
    Bt1886,
    /// CTL enum 6 — sRGB IEC 61966-2-1:1999 (moncurve 2.4 / 0.055).
    SrgbIec61966,
}

// from Lib.Academy.DisplayEncoding.ctl `eotf_inv` — display linear → code
// values. The extra clamp of negatives protects against edge-case negative
// values reintroduced by precision errors after the white limiting step.
pub fn eotf_inv(rgb_linear_in: [f64; 3], eotf_enum: Eotf) -> [f64; 3] {
    let rgb_linear = [
        ctl_max(0.0, rgb_linear_in[0]),
        ctl_max(0.0, rgb_linear_in[1]),
        ctl_max(0.0, rgb_linear_in[2]),
    ];

    match eotf_enum {
        Eotf::DisplayLinear => rgb_linear,
        Eotf::Bt1886 => bt1886_inv_f3(rgb_linear, 2.4, 1.0, 0.0),
        Eotf::SrgbIec61966 => moncurve_inv_f3(rgb_linear, 2.4, 0.055),
    }
}

// from Lib.Academy.DisplayEncoding.ctl `eotf` — code values → display
// linear.
pub fn eotf(rgb_cv: [f64; 3], eotf_enum: Eotf) -> [f64; 3] {
    match eotf_enum {
        Eotf::DisplayLinear => rgb_cv,
        Eotf::Bt1886 => bt1886_fwd_f3(rgb_cv, 2.4, 1.0, 0.0),
        Eotf::SrgbIec61966 => moncurve_fwd_f3(rgb_cv, 2.4, 0.055),
    }
}

// from Lib.Academy.DisplayEncoding.ctl — limiting-primaries linear →
// display code values.
pub fn display_encoding(
    rgb: [f64; 3],
    mat_limit_to_display: &M33,
    eotf_enum: Eotf,
    linear_scale: f64,
) -> [f64; 3] {
    // Limiting to display primary encoding
    let rgb_display_linear = mult_f3_f33(rgb, mat_limit_to_display);

    // Linear scale factor
    let rgb_display_scaled = mult_f_f3(linear_scale, rgb_display_linear);

    // Apply inverse EOTF
    eotf_inv(rgb_display_scaled, eotf_enum)
}

// from Lib.Academy.DisplayEncoding.ctl — display code values → limiting
// primaries linear.
pub fn display_decoding(
    cv: [f64; 3],
    mat_display_to_limit: &M33,
    eotf_enum: Eotf,
    linear_scale: f64,
) -> [f64; 3] {
    // Apply EOTF
    let rgb_display_scaled = eotf(cv, eotf_enum);

    // Linear scale factor
    let rgb_display_linear = mult_f_f3(1.0 / linear_scale, rgb_display_scaled);

    // Display to limiting primary encoding
    mult_f3_f33(rgb_display_linear, mat_display_to_limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_moncurve_round_trip_and_anchors() {
        // Encode/decode round trip over the full [0,1] range plus the
        // canonical IEC 61966-2-1 anchor: linear 1.0 ↔ code value 1.0, and
        // the linear-segment boundary continuity.
        for i in 0..=1000 {
            let y = i as f64 / 1000.0;
            let cv = moncurve_inv(y, 2.4, 0.055);
            let back = moncurve_fwd(cv, 2.4, 0.055);
            assert!((back - y).abs() < 1e-12, "round trip at {y}: {back}");
        }
        assert!((moncurve_inv(1.0, 2.4, 0.055) - 1.0).abs() < 1e-12);
        assert!(moncurve_fwd(0.0, 2.4, 0.055) == 0.0);
        // Continuity at the toe boundary xb = offs/(gamma-1).
        let xb = 0.055 / 1.4;
        let below = moncurve_fwd(xb - 1e-9, 2.4, 0.055);
        let above = moncurve_fwd(xb + 1e-9, 2.4, 0.055);
        assert!((above - below).abs() < 1e-7);
    }

    #[test]
    fn bt1886_round_trip() {
        for i in 0..=1000 {
            let l = i as f64 / 1000.0;
            let v = bt1886_inv(l, 2.4, 1.0, 0.0);
            let back = bt1886_fwd(v, 2.4, 1.0, 0.0);
            assert!((back - l).abs() < 1e-12, "round trip at {l}: {back}");
        }
        assert_eq!(bt1886_fwd(0.0, 2.4, 1.0, 0.0), 0.0);
        assert!((bt1886_fwd(1.0, 2.4, 1.0, 0.0) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn white_scale_identity_when_whites_match() {
        // With an identity limit→display matrix (calibration white equals
        // creative white) the scale factor is exactly 1.
        let ident: M33 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let v = [0.25, 0.5, 0.75];
        assert_eq!(apply_white_scale(v, &ident, false), v);
        assert_eq!(apply_white_scale(v, &ident, true), v);
    }

    #[test]
    fn eotf_dispatch_round_trip() {
        let ident_cases = [Eotf::DisplayLinear, Eotf::Bt1886, Eotf::SrgbIec61966];
        let v = [0.1, 0.4, 0.9];
        for e in ident_cases {
            let cv = eotf_inv(v, e);
            let back = eotf(cv, e);
            for i in 0..3 {
                assert!((back[i] - v[i]).abs() < 1e-12, "{e:?} channel {i}");
            }
        }
        // Negative inputs clamp to 0 before encode.
        let neg = eotf_inv([-1e-9, 0.0, 0.5], Eotf::SrgbIec61966);
        assert_eq!(neg[0], 0.0);
    }
}
