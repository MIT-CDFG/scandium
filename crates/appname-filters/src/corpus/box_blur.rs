// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! `filter.box_blur` — ES §16.3 Box Blur block verbatim: separable box
//! mean of width `2r+1` via running sums (add entering, subtract leaving —
//! O(1) per pixel per pass), premultiplied RGBA, f64 accumulators on the
//! CPU (the block's drift guard at r ~ 1000); ROI `out.inflate(r)`; edges
//! clamp; preview LiveFull. Param: `radius` i32 1..=1000, default 4.
//! GPU pair: pipeline key `filter.box_blur` (running-sum row strips —
//! O-RENDER handoff). Rights: elementary signal processing (§16.3 blanket;
//! ALGORITHMS.md L0 — citation-only, select-and-screen judgment, no
//! release claim).

use appname_core::IRect;
use appname_tools::resample::{EdgePolicy, Plane};
use serde_json::Value;

use crate::registry::FilterError;
use crate::traits::{Filter, FilterCtx, FilterGroup, FilterUi, PreviewPolicy};

pub struct BoxBlur;

fn radius_of(params: &Value) -> Result<i64, FilterError> {
    match params.get("radius") {
        None => Ok(4),
        Some(v) => {
            let r = v.as_i64().unwrap_or(-1);
            if !(1..=1000).contains(&r) {
                return Err(FilterError::ParamOutOfRange {
                    param: "radius",
                    detail: format!("{v} outside 1..=1000 (§16.3)"),
                });
            }
            Ok(r)
        }
    }
}

impl Filter for BoxBlur {
    fn id(&self) -> &'static str {
        "filter.box_blur"
    }

    fn param_schema(&self) -> &'static str {
        r#"{"$id":"https://appname.org/spec/filter/box_blur.v1.json","type":"object","additionalProperties":false,"properties":{"radius":{"type":"integer","minimum":1,"maximum":1000,"default":4,"description":"px (§16.3)"}}}"#
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
        let r = radius_of(params).unwrap_or(4) as i32;
        out.inflate(r, r)
    }

    fn edge(&self) -> crate::traits::EdgeRule {
        // ES §16.3 Box Blur block: "edges clamp".
        crate::traits::EdgeRule::Clamp
    }

    fn gpu_key(&self) -> &'static str {
        "filter.box_blur"
    }

    fn validate(&self, params: &Value) -> Result<(), FilterError> {
        radius_of(params).map(|_| ())
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
        let r = radius_of(params)?;
        Ok(box_blur_plane(input, out_rect, r as i32))
    }
}

/// The running-sum separable box mean (f64 accumulators per the block).
pub fn box_blur_plane(input: &Plane, out_rect: IRect, r: i32) -> Plane {
    let norm = 1.0f64 / (2 * r + 1) as f64;
    // Horizontal pass over the rows the vertical pass needs.
    let mid_rect = IRect::new(
        out_rect.min.x,
        out_rect.min.y - r,
        out_rect.max.x,
        out_rect.max.y + r,
    );
    let mut mid = Plane::new(mid_rect);
    for y in mid_rect.min.y..mid_rect.max.y {
        let mut acc = [0.0f64; 4];
        for x in (out_rect.min.x - r)..=(out_rect.min.x + r) {
            let v = input.get(x, y, EdgePolicy::Clamp);
            for c in 0..4 {
                acc[c] += v[c] as f64;
            }
        }
        for x in out_rect.min.x..out_rect.max.x {
            mid.set(
                x,
                y,
                [
                    (acc[0] * norm) as f32,
                    (acc[1] * norm) as f32,
                    (acc[2] * norm) as f32,
                    (acc[3] * norm) as f32,
                ],
            );
            let add = input.get(x + r + 1, y, EdgePolicy::Clamp);
            let sub = input.get(x - r, y, EdgePolicy::Clamp);
            for c in 0..4 {
                acc[c] += add[c] as f64 - sub[c] as f64;
            }
        }
    }
    let mut out = Plane::new(out_rect);
    for x in out_rect.min.x..out_rect.max.x {
        let mut acc = [0.0f64; 4];
        for y in (out_rect.min.y - r)..=(out_rect.min.y + r) {
            let v = mid.get(x, y, EdgePolicy::Clamp);
            for c in 0..4 {
                acc[c] += v[c] as f64;
            }
        }
        for y in out_rect.min.y..out_rect.max.y {
            out.set(
                x,
                y,
                [
                    (acc[0] * norm) as f32,
                    (acc[1] * norm) as f32,
                    (acc[2] * norm) as f32,
                    (acc[3] * norm) as f32,
                ],
            );
            let add = mid.get(x, y + r + 1, EdgePolicy::Clamp);
            let sub = mid.get(x, y - r, EdgePolicy::Clamp);
            for c in 0..4 {
                acc[c] += add[c] as f64 - sub[c] as f64;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_mean_matches_the_direct_window_average() {
        // Deterministic pseudo-random plane; r = 3: every output pixel
        // equals the direct 7×7 clamped mean (running sums are exact).
        let extent = IRect::new(0, 0, 24, 20);
        let mut src = Plane::new(extent);
        for y in 0..20 {
            for x in 0..24 {
                let v = ((x * 31 + y * 17) % 97) as f32 / 96.0;
                src.set(x, y, [v, 1.0 - v, v * 0.5, 1.0]);
            }
        }
        let out = box_blur_plane(&src, extent, 3);
        for y in 0..20i32 {
            for x in 0..24i32 {
                let mut want = [0.0f64; 4];
                for dy in -3..=3 {
                    for dx in -3..=3 {
                        let v = src.get(x + dx, y + dy, EdgePolicy::Clamp);
                        for c in 0..4 {
                            want[c] += v[c] as f64;
                        }
                    }
                }
                let got = out.get(x, y, EdgePolicy::Clamp);
                for c in 0..4 {
                    assert!(
                        (got[c] as f64 - want[c] / 49.0).abs() < 1e-6,
                        "({x},{y})[{c}]"
                    );
                }
            }
        }
    }

    #[test]
    fn constants_survive_and_params_gate() {
        let extent = IRect::new(0, 0, 16, 16);
        let mut src = Plane::new(extent);
        src.px.fill([0.3, 0.6, 0.1, 1.0]);
        let out = box_blur_plane(&src, extent, 9);
        for p in &out.px {
            for c in 0..4 {
                assert!((p[c] - [0.3, 0.6, 0.1, 1.0][c]).abs() < 1e-6);
            }
        }
        let f = BoxBlur;
        assert!(f.validate(&serde_json::json!({"radius": 1})).is_ok());
        assert!(f.validate(&serde_json::json!({"radius": 0})).is_err());
        assert!(f.validate(&serde_json::json!({"radius": 1001})).is_err());
        assert!(
            f.validate(&serde_json::json!({"radius": 2.5})).is_err(),
            "integer per §16.3"
        );
        assert_eq!(
            f.roi(
                IRect::new(0, 0, 4, 4),
                &serde_json::json!({"radius": 7}),
                &crate::traits::FilterCtx {
                    canvas: extent,
                    seed: 0,
                    lod: 0
                }
            ),
            IRect::new(-7, -7, 11, 11)
        );
    }
}
