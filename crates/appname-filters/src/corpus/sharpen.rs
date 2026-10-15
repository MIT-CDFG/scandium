// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! `filter.unsharp_mask` + `filter.high_pass` — ES §16.3 blocks verbatim.
//!
//! USM: `out = in + amount · h(in − G_σ(in))` per channel on
//! **un-premultiplied** RGB (alpha passes through — "sharpening
//! premultiplied data would sharpen the alpha ramp into halos"); `σ =
//! radius` (NOT radius/3 — the block's own ROI asymmetry vs Gaussian);
//! soft-knee threshold `h(d) = d · smoothstep(0.5t, 1.5t, |d|)` (`h = d`
//! at `t = 0`); params amount 0.01..=5 def 0.8, radius 0.1..=1000 def 2,
//! threshold 0..=1 def 0; ROI `out.inflate(⌈3·radius⌉)`; edges clamp.
//!
//! High Pass: `out = clamp(in − G_σ(in) + 0.5)` per channel,
//! un-premultiplied, alpha unchanged; `σ = radius` 0.1..=1000 def 10; ROI
//! `⌈3σ⌉`; the +0.5 centers the residual for §4.5 overlay-sharpening.
//!
//! Rights: USM is L0 ("SPEC §16.3 binds the math" — ALGORITHMS.md sharpen
//! row; dossier sharpen-deconvolution.yaml `selected: "USM per §16.3
//! exact math"`, citations richardson72/lucy74 cover the deconvolution
//! members, not USM which is elementary unsharp masking); High Pass =
//! elementary signal processing (§16.3 blanket). Select-and-screen
//! judgment only, no release claim.

use appname_core::IRect;
use appname_tools::resample::{EdgePolicy, Plane};
use serde_json::Value;

use super::gaussian::gaussian_blur_plane;
use super::shared::{premul, unpremul};
use crate::registry::FilterError;
use crate::traits::{Filter, FilterCtx, FilterGroup, FilterUi, PreviewPolicy};

pub struct UnsharpMask;
pub struct HighPass;

#[inline]
fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    if e1 <= e0 {
        return if x >= e1 { 1.0 } else { 0.0 };
    }
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// §16.3 USM soft knee.
#[inline]
fn soft_knee(d: f32, t: f32) -> f32 {
    if t <= 0.0 {
        d
    } else {
        d * smoothstep(0.5 * t, 1.5 * t, d.abs())
    }
}

fn f32_param(
    params: &Value,
    name: &'static str,
    default: f64,
    lo: f64,
    hi: f64,
) -> Result<f64, FilterError> {
    match params.get(name) {
        None => Ok(default),
        Some(v) => {
            let x = v.as_f64().unwrap_or(f64::NAN);
            let x = if x.is_nan() {
                default
            } else if x == f64::INFINITY {
                hi
            } else if x == f64::NEG_INFINITY {
                lo
            } else {
                x
            };
            if !(lo..=hi).contains(&x) {
                return Err(FilterError::ParamOutOfRange {
                    param: name,
                    detail: format!("{x} outside {lo}..={hi} (§16.3)"),
                });
            }
            Ok(x)
        }
    }
}

/// Blur the un-premultiplied color planes with σ, preserving alpha: the
/// shared first stage of both filters.
fn unpremul_gaussian(input: &Plane, rect: IRect, sigma: f64) -> (Plane, Plane) {
    // Un-premultiply over the apron the blur will read.
    let reach = (3.0 * sigma).ceil() as i32;
    let work_rect = rect.inflate(reach, reach);
    let mut straight = Plane::new(work_rect);
    for y in work_rect.min.y..work_rect.max.y {
        for x in work_rect.min.x..work_rect.max.x {
            straight.set(x, y, unpremul(input.get(x, y, EdgePolicy::Clamp)));
        }
    }
    let blurred = gaussian_blur_plane(&straight, rect, sigma);
    (straight, blurred)
}

impl Filter for UnsharpMask {
    fn id(&self) -> &'static str {
        "filter.unsharp_mask"
    }

    fn param_schema(&self) -> &'static str {
        r#"{"$id":"https://appname.org/spec/filter/unsharp_mask.v1.json","type":"object","additionalProperties":false,"properties":{"amount":{"type":"number","minimum":0.01,"maximum":5.0,"default":0.8},"radius":{"type":"number","minimum":0.1,"maximum":1000,"default":2.0,"description":"px; sigma = radius (§16.3)"},"threshold":{"type":"number","minimum":0,"maximum":1,"default":0}}}"#
    }

    fn ui_hints(&self) -> FilterUi {
        FilterUi {
            preview: PreviewPolicy::LiveFull,
            has_dialog: true,
            group: FilterGroup::Sharpen,
            checkpoint: false,
        }
    }

    fn roi(&self, out: IRect, params: &Value, _ctx: &FilterCtx) -> IRect {
        let r = f32_param(params, "radius", 2.0, 0.1, 1000.0).unwrap_or(2.0);
        let a = (3.0 * r).ceil() as i32;
        out.inflate(a, a)
    }

    fn edge(&self) -> crate::traits::EdgeRule {
        // ES §16.3 sharpen family blocks: "edges clamp".
        crate::traits::EdgeRule::Clamp
    }

    fn gpu_key(&self) -> &'static str {
        "filter.unsharp_mask"
    }

    fn validate(&self, params: &Value) -> Result<(), FilterError> {
        f32_param(params, "amount", 0.8, 0.01, 5.0)?;
        f32_param(params, "radius", 2.0, 0.1, 1000.0)?;
        f32_param(params, "threshold", 0.0, 0.0, 1.0)?;
        Ok(())
    }

    fn cpu(
        &self,
        input: &Plane,
        out_rect: IRect,
        params: &Value,
        _ctx: &FilterCtx,
    ) -> Result<Plane, FilterError> {
        if input.rect.intersect(&out_rect) != out_rect {
            return Err(FilterError::RoiNotCovered);
        }
        let amount = f32_param(params, "amount", 0.8, 0.01, 5.0)? as f32;
        let sigma = f32_param(params, "radius", 2.0, 0.1, 1000.0)?;
        let threshold = f32_param(params, "threshold", 0.0, 0.0, 1.0)? as f32;
        let (straight, blurred) = unpremul_gaussian(input, out_rect, sigma);
        let mut out = Plane::new(out_rect);
        for y in out_rect.min.y..out_rect.max.y {
            for x in out_rect.min.x..out_rect.max.x {
                let s = straight.get(x, y, EdgePolicy::Clamp);
                let b = blurred.get(x, y, EdgePolicy::Clamp);
                let mut v = [0.0f32; 4];
                for c in 0..3 {
                    v[c] = s[c] + amount * soft_knee(s[c] - b[c], threshold);
                }
                v[3] = s[3]; // alpha passes through
                out.set(x, y, premul(v));
            }
        }
        Ok(out)
    }
}

impl Filter for HighPass {
    fn id(&self) -> &'static str {
        "filter.high_pass"
    }

    fn param_schema(&self) -> &'static str {
        r#"{"$id":"https://appname.org/spec/filter/high_pass.v1.json","type":"object","additionalProperties":false,"properties":{"radius":{"type":"number","minimum":0.1,"maximum":1000,"default":10,"description":"px; sigma = radius (§16.3)"}}}"#
    }

    fn ui_hints(&self) -> FilterUi {
        FilterUi {
            preview: PreviewPolicy::LiveFull,
            has_dialog: true,
            group: FilterGroup::Sharpen,
            checkpoint: false,
        }
    }

    fn roi(&self, out: IRect, params: &Value, _ctx: &FilterCtx) -> IRect {
        let r = f32_param(params, "radius", 10.0, 0.1, 1000.0).unwrap_or(10.0);
        let a = (3.0 * r).ceil() as i32;
        out.inflate(a, a)
    }

    fn edge(&self) -> crate::traits::EdgeRule {
        // ES §16.3 sharpen family blocks: "edges clamp".
        crate::traits::EdgeRule::Clamp
    }

    fn gpu_key(&self) -> &'static str {
        "filter.high_pass"
    }

    fn validate(&self, params: &Value) -> Result<(), FilterError> {
        f32_param(params, "radius", 10.0, 0.1, 1000.0).map(|_| ())
    }

    fn cpu(
        &self,
        input: &Plane,
        out_rect: IRect,
        params: &Value,
        _ctx: &FilterCtx,
    ) -> Result<Plane, FilterError> {
        if input.rect.intersect(&out_rect) != out_rect {
            return Err(FilterError::RoiNotCovered);
        }
        let sigma = f32_param(params, "radius", 10.0, 0.1, 1000.0)?;
        let (straight, blurred) = unpremul_gaussian(input, out_rect, sigma);
        let mut out = Plane::new(out_rect);
        for y in out_rect.min.y..out_rect.max.y {
            for x in out_rect.min.x..out_rect.max.x {
                let s = straight.get(x, y, EdgePolicy::Clamp);
                let b = blurred.get(x, y, EdgePolicy::Clamp);
                let mut v = [0.0f32; 4];
                for c in 0..3 {
                    v[c] = (s[c] - b[c] + 0.5).clamp(0.0, 1.0); // §16.3 clamp
                }
                v[3] = s[3];
                out.set(x, y, premul(v));
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(extent: IRect, v: [f32; 4]) -> Plane {
        let mut p = Plane::new(extent);
        p.px.fill(v);
        p
    }

    #[test]
    fn usm_flat_regions_are_untouched_and_edges_sharpen() {
        let extent = IRect::new(0, 0, 64, 8);
        // Step edge at x = 32 (opaque).
        let mut src = Plane::new(extent);
        for y in 0..8 {
            for x in 0..64 {
                let v = if x < 32 { 0.25 } else { 0.75 };
                src.set(x, y, [v, v, v, 1.0]);
            }
        }
        let f = UnsharpMask;
        let ctx = FilterCtx {
            canvas: extent,
            seed: 0,
            lod: 0,
        };
        let params = serde_json::json!({"amount": 1.0, "radius": 2.0, "threshold": 0.0});
        let out = f.cpu(&src, extent, &params, &ctx).unwrap();
        // Far from the edge: unchanged (G(in) == in on flats).
        let far = out.get(4, 4, EdgePolicy::Clamp);
        assert!(
            (far[0] - 0.25).abs() < 1e-4,
            "flat left untouched: {}",
            far[0]
        );
        // Just left of the edge: darker (negative residual amplified);
        // just right: brighter — the classic overshoot pair.
        let l = out.get(30, 4, EdgePolicy::Clamp)[0];
        let r = out.get(33, 4, EdgePolicy::Clamp)[0];
        assert!(l < 0.25 - 0.01, "undershoot left {l}");
        assert!(r > 0.75 + 0.01, "overshoot right {r}");
        // amount scales the effect linearly (t = 0 ⇒ h = d).
        let out2 = f
            .cpu(
                &src,
                extent,
                &serde_json::json!({"amount": 2.0, "radius": 2.0}),
                &ctx,
            )
            .unwrap();
        let l2 = out2.get(30, 4, EdgePolicy::Clamp)[0];
        assert!(
            ((0.25 - l2) / (0.25 - l) - 2.0).abs() < 0.02,
            "amount linearity: {} vs {}",
            0.25 - l2,
            0.25 - l
        );
    }

    #[test]
    fn usm_threshold_soft_knee_gates_small_deltas() {
        // d below t/2 untouched; above 3t/2 fully sharpened.
        assert_eq!(soft_knee(0.04, 0.1), 0.0, "below the knee");
        assert!((soft_knee(0.2, 0.1) - 0.2).abs() < 1e-6, "above the knee");
        let mid = soft_knee(0.1, 0.1); // |d| = t → smoothstep midpoint 0.5
        assert!((mid - 0.05).abs() < 1e-6, "knee midpoint {mid}");
        assert_eq!(soft_knee(-0.04, 0.1), 0.0, "symmetric");
        assert!((soft_knee(-0.2, 0.1) + 0.2).abs() < 1e-6);
        assert_eq!(soft_knee(0.3, 0.0), 0.3, "t = 0 is the identity knee");
    }

    #[test]
    fn usm_alpha_passes_through_and_stays_premul_valid() {
        let extent = IRect::new(0, 0, 32, 8);
        let mut src = Plane::new(extent);
        for y in 0..8 {
            for x in 0..32 {
                let v = if x < 16 { 0.2 } else { 0.9 };
                let a = 0.5f32;
                src.set(x, y, [v * a, v * a, v * a, a]);
            }
        }
        let f = UnsharpMask;
        let ctx = FilterCtx {
            canvas: extent,
            seed: 0,
            lod: 0,
        };
        let out = f
            .cpu(
                &src,
                extent,
                &serde_json::json!({"amount": 1.0, "radius": 1.5}),
                &ctx,
            )
            .unwrap();
        for p in &out.px {
            assert!((p[3] - 0.5).abs() < 1e-6, "alpha untouched: {}", p[3]);
        }
    }

    #[test]
    fn high_pass_flat_is_half_gray_and_edges_deviate() {
        let extent = IRect::new(0, 0, 64, 8);
        let f = HighPass;
        let ctx = FilterCtx {
            canvas: extent,
            seed: 0,
            lod: 0,
        };
        let out = f
            .cpu(
                &flat(extent, [0.7, 0.3, 0.1, 1.0]),
                extent,
                &serde_json::json!({"radius": 5.0}),
                &ctx,
            )
            .unwrap();
        for p in &out.px {
            for &v in p.iter().take(3) {
                assert!((v - 0.5).abs() < 1e-4, "flat → 0.5: {}", v);
            }
            assert!((p[3] - 1.0).abs() < 1e-6);
        }
        // Step edge: residual departs 0.5 on both sides.
        let mut src = Plane::new(extent);
        for y in 0..8 {
            for x in 0..64 {
                let v = if x < 32 { 0.0 } else { 1.0 };
                src.set(x, y, [v, v, v, 1.0]);
            }
        }
        let out = f
            .cpu(&src, extent, &serde_json::json!({"radius": 3.0}), &ctx)
            .unwrap();
        assert!(out.get(30, 4, EdgePolicy::Clamp)[0] < 0.5 - 0.05);
        assert!(out.get(33, 4, EdgePolicy::Clamp)[0] > 0.5 + 0.05);
        // Clamp is part of the contract.
        for p in &out.px {
            for v in p.iter().take(3) {
                assert!((0.0..=1.0).contains(v));
            }
        }
    }
}
