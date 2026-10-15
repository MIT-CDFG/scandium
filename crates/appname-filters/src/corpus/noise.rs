// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! `filter.add_noise` — ES §16.3 Add Noise block verbatim: per pixel on
//! un-premultiplied RGB (alpha untouched): `uniform`: `n = amount·(2u−1)`;
//! `gaussian`: `n = amount·0.5·z`, z from Box–Muller on two uniforms
//! [LEGAL: Box & Muller 1958 — the ES citation]; `monochromatic` draws one
//! n per pixel for all three channels, else three independent draws; no
//! mid-pipeline clamp (f32 keeps excursions; store-clamp at integer
//! depths). Params: amount 0.001..=4.0 def 0.1; distribution
//! uniform|gaussian; monochromatic bool def false; seed u32.
//! [NORMATIVE deterministic] seeded per (tile, seed) exactly per §16.3's
//! convention — bit-identical re-runs, tile-order independent. ROI = out
//! (pointwise). Rights: elementary signal processing + the cited 1958
//! published transform (L0, citation-only; select-and-screen judgment, no
//! release claim). libm caveat (ln/cos in Box–Muller): per-build pins,
//! the brush-module caveat class.
//!
//! v1 pins: the per-pixel draw budget is FIXED per (distribution,
//! monochromatic) — uniform: 1/3 draws; gaussian: 2/6 (each z consumes two
//! uniforms; z = √(−2 ln u₁)·cos(2π u₂), u₁ clamped ≥ 2⁻³² to keep ln
//! finite) — and pixels consume draws in row-major order WITHIN their
//! 256² document tile (stream = pass_index 0), so any evaluator tiling
//! reproduces these bytes.

use appname_core::{IRect, TILE_I32};
use appname_tools::resample::{EdgePolicy, Plane};
use serde_json::Value;

use super::shared::{premul, tile_rng, unpremul};
use crate::registry::FilterError;
use crate::traits::{Filter, FilterCtx, FilterGroup, FilterUi, PreviewPolicy};

pub struct AddNoise;

struct NoiseParams {
    amount: f32,
    gaussian: bool,
    monochromatic: bool,
    seed: u32,
}

fn params_of(params: &Value) -> Result<NoiseParams, FilterError> {
    let amount = match params.get("amount") {
        None => 0.1,
        Some(v) => {
            let x = v.as_f64().unwrap_or(f64::NAN);
            let x = if x.is_nan() {
                0.1
            } else {
                x.clamp(f64::NEG_INFINITY, f64::INFINITY)
            };
            if !(0.001..=4.0).contains(&x) {
                return Err(FilterError::ParamOutOfRange {
                    param: "amount",
                    detail: format!("{x} outside 0.001..=4.0 (§16.3)"),
                });
            }
            x
        }
    } as f32;
    let gaussian = match params.get("distribution").and_then(Value::as_str) {
        None | Some("uniform") => false,
        Some("gaussian") => true,
        Some(other) => {
            return Err(FilterError::ParamOutOfRange {
                param: "distribution",
                detail: format!("unknown distribution {other:?} (§16.3: uniform|gaussian)"),
            })
        }
    };
    let monochromatic = params
        .get("monochromatic")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let seed = match params.get("seed") {
        None => 0,
        Some(v) => v
            .as_u64()
            .filter(|s| *s <= u32::MAX as u64)
            .ok_or_else(|| FilterError::ParamOutOfRange {
                param: "seed",
                detail: "seed is u32 (§16.3)".into(),
            })? as u32,
    };
    Ok(NoiseParams {
        amount,
        gaussian,
        monochromatic,
        seed,
    })
}

impl Filter for AddNoise {
    fn id(&self) -> &'static str {
        "filter.add_noise"
    }

    fn param_schema(&self) -> &'static str {
        r#"{"$id":"https://appname.org/spec/filter/add_noise.v1.json","type":"object","additionalProperties":false,"properties":{"amount":{"type":"number","minimum":0.001,"maximum":4.0,"default":0.1},"distribution":{"enum":["uniform","gaussian"],"default":"uniform"},"monochromatic":{"type":"boolean","default":false},"seed":{"type":"integer","minimum":0,"maximum":4294967295}}}"#
    }

    fn ui_hints(&self) -> FilterUi {
        FilterUi {
            preview: PreviewPolicy::LiveFull,
            has_dialog: true,
            group: FilterGroup::Noise,
            checkpoint: false,
        }
    }

    fn roi(&self, out: IRect, _params: &Value, _ctx: &FilterCtx) -> IRect {
        out // pointwise
    }

    fn edge(&self) -> crate::traits::EdgeRule {
        // pointwise (ROI identity) — clamp declared for uniformity; no apron is ever materialized.
        crate::traits::EdgeRule::Clamp
    }

    fn gpu_key(&self) -> &'static str {
        "filter.add_noise" // WGSL PCG32 must match this crate bit-for-bit (§16.3) — O-RENDER handoff
    }

    fn validate(&self, params: &Value) -> Result<(), FilterError> {
        params_of(params).map(|_| ())
    }

    fn cpu(
        &self,
        input: &Plane,
        out_rect: IRect,
        params: &Value,
        ctx: &FilterCtx,
    ) -> Result<Plane, FilterError> {
        if input.rect.intersect(&out_rect) != out_rect {
            return Err(FilterError::RoiNotCovered);
        }
        let np = params_of(params)?;
        let combined_seed = ctx.seed ^ ((np.seed as u64) << 32 | np.seed as u64);
        let draws_per_px = match (np.gaussian, np.monochromatic) {
            (false, true) => 1,
            (false, false) => 3,
            (true, true) => 2,
            (true, false) => 6,
        };
        let mut out = Plane::new(out_rect);
        // Walk the 256² document-tile grid covering out_rect; per tile,
        // consume the tile's stream row-major so tiling never changes bytes.
        for (tx, ty) in out_rect.tiles(TILE_I32) {
            let tile_rect = IRect::new(
                tx * TILE_I32,
                ty * TILE_I32,
                (tx + 1) * TILE_I32,
                (ty + 1) * TILE_I32,
            );
            let mut rng = tile_rng(combined_seed, tx, ty, ctx.lod, 0);
            for y in tile_rect.min.y..tile_rect.max.y {
                for x in tile_rect.min.x..tile_rect.max.x {
                    if !out_rect.contains(appname_core::IVec2::new(x, y)) {
                        // Off-plane pixels still consume their fixed budget
                        // (v1 pin: budget is positional, not conditional).
                        for _ in 0..draws_per_px {
                            let _ = rng.u01();
                        }
                        continue;
                    }
                    let draw_n = |rng: &mut appname_tools::brush::rng::Pcg32| -> f32 {
                        if np.gaussian {
                            let u1 = rng.u01().max(2.0_f32.powi(-32));
                            let u2 = rng.u01();
                            let z = (-2.0 * (u1 as f64).ln()).sqrt()
                                * (core::f64::consts::TAU * u2 as f64).cos();
                            np.amount * 0.5 * z as f32
                        } else {
                            np.amount * (2.0 * rng.u01() - 1.0)
                        }
                    };
                    let s = unpremul(input.get(x, y, EdgePolicy::Clamp));
                    let mut v = s;
                    if np.monochromatic {
                        let n = draw_n(&mut rng);
                        for c in 0..3 {
                            v[c] = s[c] + n;
                        }
                    } else {
                        for c in 0..3 {
                            v[c] = s[c] + draw_n(&mut rng);
                        }
                    }
                    out.set(x, y, premul(v));
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gray(extent: IRect) -> Plane {
        let mut p = Plane::new(extent);
        p.px.fill([0.5, 0.5, 0.5, 1.0]);
        p
    }

    fn ctx(seed: u64) -> FilterCtx {
        FilterCtx {
            canvas: IRect::new(0, 0, 512, 512),
            seed,
            lod: 0,
        }
    }

    #[test]
    fn deterministic_and_seed_sensitive() {
        let f = AddNoise;
        let extent = IRect::new(0, 0, 64, 64);
        let src = gray(extent);
        let p = serde_json::json!({"amount": 0.2, "seed": 7});
        let a = f.cpu(&src, extent, &p, &ctx(1)).unwrap();
        let b = f.cpu(&src, extent, &p, &ctx(1)).unwrap();
        assert_eq!(a.px, b.px, "same seed bit-identical (§16.3/§16.4)");
        let c = f.cpu(&src, extent, &p, &ctx(2)).unwrap();
        assert_ne!(
            a.px, c.px,
            "different seed measurably different (§16.4 guard)"
        );
        let d = f
            .cpu(
                &src,
                extent,
                &serde_json::json!({"amount": 0.2, "seed": 8}),
                &ctx(1),
            )
            .unwrap();
        assert_ne!(
            a.px, d.px,
            "the filter's own seed param feeds the stream too"
        );
    }

    #[test]
    fn tiling_invariance_of_the_noise_field() {
        // Producing two disjoint out-rects reproduces exactly the same
        // bytes as producing them in one call — the §16.4 tile-invariance
        // bar for stochastic filters.
        let f = AddNoise;
        let extent = IRect::new(0, 0, 300, 40); // crosses the x=256 tile seam
        let src = gray(extent);
        let p = serde_json::json!({"amount": 0.3, "seed": 5});
        let whole = f.cpu(&src, extent, &p, &ctx(9)).unwrap();
        let left = f.cpu(&src, IRect::new(0, 0, 150, 40), &p, &ctx(9)).unwrap();
        let right = f
            .cpu(&src, IRect::new(150, 0, 300, 40), &p, &ctx(9))
            .unwrap();
        for y in 0..40 {
            for x in 0..300 {
                let w = whole.get(x, y, EdgePolicy::Clamp);
                let split = if x < 150 {
                    left.get(x, y, EdgePolicy::Clamp)
                } else {
                    right.get(x, y, EdgePolicy::Clamp)
                };
                assert_eq!(w, split, "({x},{y})");
            }
        }
    }

    #[test]
    fn monochromatic_correlates_channels_uniform_stats_hold() {
        let f = AddNoise;
        let extent = IRect::new(0, 0, 128, 128);
        let src = gray(extent);
        let mono = f
            .cpu(
                &src,
                extent,
                &serde_json::json!({"amount": 0.2, "monochromatic": true}),
                &ctx(3),
            )
            .unwrap();
        for p in &mono.px {
            assert!(
                (p[0] - p[1]).abs() < 1e-6 && (p[1] - p[2]).abs() < 1e-6,
                "one n per pixel"
            );
        }
        let color = f
            .cpu(&src, extent, &serde_json::json!({"amount": 0.2}), &ctx(3))
            .unwrap();
        assert!(
            color.px.iter().any(|p| (p[0] - p[1]).abs() > 1e-4),
            "independent draws"
        );
        // Uniform bounds: |n| ≤ amount; mean ≈ 0.5.
        let mut mean = 0.0f64;
        for p in &color.px {
            assert!(p[0] >= 0.5 - 0.2 - 1e-5 && p[0] <= 0.5 + 0.2 + 1e-5);
            mean += p[0] as f64;
        }
        mean /= color.px.len() as f64;
        assert!((mean - 0.5).abs() < 0.01, "uniform mean {mean}");
        // Alpha untouched everywhere.
        assert!(color.px.iter().all(|p| p[3] == 1.0));
    }

    #[test]
    fn gaussian_distribution_spread_and_no_clamp() {
        let f = AddNoise;
        let extent = IRect::new(0, 0, 128, 128);
        let src = gray(extent);
        let out = f
            .cpu(
                &src,
                extent,
                &serde_json::json!({"amount": 1.0, "distribution": "gaussian"}),
                &ctx(4),
            )
            .unwrap();
        // σ = amount·0.5 = 0.5: expect excursions past [0,1] preserved
        // (no mid-pipeline clamp) and an empirical std near 0.5.
        assert!(
            out.px.iter().any(|p| p[0] < 0.0 || p[0] > 1.0),
            "excursions kept"
        );
        let mean: f64 = out.px.iter().map(|p| p[0] as f64).sum::<f64>() / out.px.len() as f64;
        let var: f64 = out
            .px
            .iter()
            .map(|p| (p[0] as f64 - mean).powi(2))
            .sum::<f64>()
            / out.px.len() as f64;
        assert!(
            (var.sqrt() - 0.5).abs() < 0.03,
            "σ ≈ 0.5, got {}",
            var.sqrt()
        );
    }

    #[test]
    fn params_gate() {
        let f = AddNoise;
        assert!(f.validate(&serde_json::json!({})).is_ok());
        assert!(f.validate(&serde_json::json!({"amount": 4.1})).is_err());
        assert!(f
            .validate(&serde_json::json!({"distribution": "poisson"}))
            .is_err());
        assert!(f
            .validate(&serde_json::json!({"seed": 4294967296u64}))
            .is_err());
    }
}
