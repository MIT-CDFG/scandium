// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! `filter.gaussian_blur` — ES §16.3 Gaussian Blur block, implemented
//! verbatim:
//!
//! * Param: `radius` f32, 0.1–1000, default 8.0, px.
//! * [NORMATIVE] `σ = radius / 3` (the kernel's 3σ envelope equals the UI
//!   radius); 1-D kernel `w_k = exp(−k²/2σ²)`, `k ∈ [−⌈3σ⌉, +⌈3σ⌉]`,
//!   normalized `Σ = 1`; horizontal then vertical pass on premultiplied
//!   RGBA (all four channels).
//! * For `σ > 32` the implementation SHOULD switch to the 3-box
//!   approximation [Kovesi 2010, "Fast almost-Gaussian filtering" — the ES
//!   citation]: ideal `w = sqrt(12σ²/3 + 1)`, `wl` largest odd ≤ w,
//!   `wu = wl + 2`, `m = round((12σ² − 3wl² − 12wl − 9)/(−4wl − 4))`; `m`
//!   box passes of width `wl` and `3 − m` of `wu` per axis.
//! * ROI: `out.inflate(⌈3σ⌉)`. Edges: clamp ("transparent surroundings
//!   darken naturally because data is premultiplied — that is the correct
//!   look" applies to canvas-transparent reads via the evaluator's plane
//!   materialization; the kernel itself clamps at the canvas per the
//!   block). Preview: LiveFull. GPU pair: two compute passes,
//!   `rgba16float` intermediate — pipeline key `filter.gaussian_blur`
//!   (WGSL = O-RENDER handoff).
//!
//! Rights basis: Gaussian filtering is textbook signal processing (§16.3
//! [LEGAL] blanket: "every algorithm below is from public literature …
//! effect concepts are unprotectable functional ideas and each
//! implementation here is original"); Kovesi 2010 is the ES-cited
//! published basis for the box path. Select-and-screen judgment only, no
//! release claim.
//!
//! Non-finite policy (§0.3/§40.1): NaN radius clamps to the default 8.0,
//! ±Inf to the range ends (clamps recorded by the command layer when the
//! seam lands).

use appname_core::IRect;
use appname_tools::resample::Plane;
use serde_json::Value;

use crate::registry::FilterError;
use crate::traits::{Filter, FilterCtx, FilterGroup, FilterUi, PreviewPolicy};

pub struct GaussianBlur;

pub const RADIUS_MIN: f64 = 0.1;
pub const RADIUS_MAX: f64 = 1000.0;
pub const RADIUS_DEFAULT: f64 = 8.0;

fn radius_of(params: &Value) -> Result<f64, FilterError> {
    match params.get("radius") {
        None => Ok(RADIUS_DEFAULT),
        Some(v) => {
            let r = v.as_f64().unwrap_or(f64::NAN);
            // §40.1 non-finite policy: NaN → default, ±Inf → range ends.
            let r = if r.is_nan() {
                RADIUS_DEFAULT
            } else if r == f64::INFINITY {
                RADIUS_MAX
            } else if r == f64::NEG_INFINITY {
                RADIUS_MIN
            } else {
                r
            };
            if !(RADIUS_MIN..=RADIUS_MAX).contains(&r) {
                return Err(FilterError::ParamOutOfRange {
                    param: "radius",
                    detail: format!("{r} outside 0.1..=1000 (§16.3)"),
                });
            }
            Ok(r)
        }
    }
}

impl Filter for GaussianBlur {
    fn id(&self) -> &'static str {
        "filter.gaussian_blur"
    }

    fn param_schema(&self) -> &'static str {
        r#"{"$id":"https://appname.org/spec/filter/gaussian_blur.v1.json","type":"object","additionalProperties":false,"properties":{"radius":{"type":"number","minimum":0.1,"maximum":1000,"default":8.0,"description":"px; sigma = radius/3 (§16.3)"}}}"#
    }

    fn ui_hints(&self) -> FilterUi {
        FilterUi {
            preview: PreviewPolicy::LiveFull,
            has_dialog: true,
            group: FilterGroup::Blur,
            checkpoint: false,
        }
    }

    fn roi(&self, out: IRect, params: &Value, _ctx: &FilterCtx) -> IRect {
        let sigma = radius_of(params)
            .map(|r| r / 3.0)
            .unwrap_or(RADIUS_DEFAULT / 3.0);
        // ONE definition of the kernel reach (⌈3σ⌉), shared with the
        // §11.2 feather's Rmask margin.
        let r = gaussian_reach(sigma);
        out.inflate(r, r)
    }

    fn edge(&self) -> crate::traits::EdgeRule {
        // ES §16.3 Gaussian block: "edges: clamp".
        crate::traits::EdgeRule::Clamp
    }

    fn gpu_key(&self) -> &'static str {
        "filter.gaussian_blur" // O-RENDER pipeline handoff (two passes, rgba16f mid)
    }

    fn validate(&self, params: &Value) -> Result<(), FilterError> {
        radius_of(params).map(|_| ())
    }

    fn cpu(
        &self,
        input: &Plane,
        out_rect: IRect,
        params: &Value,
        ctx: &FilterCtx,
    ) -> Result<Plane, FilterError> {
        let sigma = radius_of(params)? / 3.0;
        if !input
            .rect
            .contains_rect(&self.roi(out_rect, params, ctx).intersect(&input.rect))
            || input.rect.intersect(&out_rect) != out_rect
        {
            // The evaluator owes at least roi ∩ available data; out itself
            // must be inside the provided plane's coordinate coverage.
            return Err(FilterError::RoiNotCovered);
        }
        Ok(gaussian_blur_plane(input, out_rect, sigma))
    }
}

// The plane-level §16.3 kernel now lives in `appname-tools`
// (`appname_tools::blur`) — O-INT architecture ruling for the §11.2
// feather sub-slice: one implementation, in the crate that owns `Plane`.
// Re-exported here so `filter.gaussian_blur`'s historical call site and
// every existing importer keep working unchanged.
pub use appname_tools::blur::{gaussian_blur_plane, gaussian_reach};

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> FilterCtx {
        FilterCtx {
            canvas: IRect::new(0, 0, 512, 512),
            seed: 0,
            lod: 0,
        }
    }

    fn impulse(at: (i32, i32), extent: IRect) -> Plane {
        let mut p = Plane::new(extent);
        p.set(at.0, at.1, [1.0, 0.5, 0.25, 1.0]);
        p
    }

    #[test]
    fn params_validate_with_the_non_finite_policy() {
        let f = GaussianBlur;
        assert!(
            f.validate(&serde_json::json!({})).is_ok(),
            "absent = default 8.0"
        );
        assert!(f.validate(&serde_json::json!({"radius": 0.1})).is_ok());
        assert!(f.validate(&serde_json::json!({"radius": 1000.0})).is_ok());
        assert!(f.validate(&serde_json::json!({"radius": 0.05})).is_err());
        assert!(f.validate(&serde_json::json!({"radius": 1001.0})).is_err());
        // NaN clamps to the default; ±Inf to the ends (§40.1) — all valid.
        assert!(f.validate(&serde_json::json!({"radius": f64::NAN})).is_ok());
        assert!(f
            .validate(&serde_json::json!({"radius": f64::INFINITY}))
            .is_ok());
    }

    #[test]
    fn roi_is_out_inflated_by_ceil_3_sigma() {
        let f = GaussianBlur;
        let out = IRect::new(10, 10, 20, 20);
        // radius 9 → σ = 3 → ⌈3σ⌉ = 9.
        let roi = f.roi(out, &serde_json::json!({"radius": 9.0}), &ctx());
        assert_eq!(roi, out.inflate(9, 9));
    }

    #[test]
    fn cpu_entry_checks_roi_and_is_deterministic() {
        let f = GaussianBlur;
        let extent = IRect::new(0, 0, 64, 64);
        let src = impulse((32, 32), extent);
        let params = serde_json::json!({"radius": 6.0});
        let a = f
            .cpu(&src, IRect::new(16, 16, 48, 48), &params, &ctx())
            .unwrap();
        let b = f
            .cpu(&src, IRect::new(16, 16, 48, 48), &params, &ctx())
            .unwrap();
        assert_eq!(a.px, b.px, "bit-identical re-runs");
        // Out rect escaping the provided plane's coverage → typed error.
        let err = f
            .cpu(&src, IRect::new(60, 60, 80, 80), &params, &ctx())
            .unwrap_err();
        assert_eq!(err, FilterError::RoiNotCovered);
    }
}
