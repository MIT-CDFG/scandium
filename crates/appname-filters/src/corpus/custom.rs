// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! `filter.custom` — ES §16.3 Custom convolution block verbatim: user 5×5
//! integer kernel (−999..=999 per entry) on un-premultiplied RGB (alpha
//! passes through — "negative lobes on premultiplied data would corrupt
//! the alpha ramp"); `out = clamp(Σ kernel_k·src(p+o_k)/divisor + offset,
//! 0, 1)`; divisor 1..=9999 def 1; offset −1..=1 def 0; zero entries
//! skipped; [NORMATIVE] the clamp is part of this filter's contract at
//! EVERY depth, f32 included — the documented exception to the
//! no-mid-pipeline-clamp convention. ROI `out.inflate(2)`; edges clamp;
//! preview LiveFull. Rights: elementary convolution (§16.3 blanket, L0
//! citation-only; select-and-screen judgment, no release claim).

use appname_core::IRect;
use appname_tools::resample::{EdgePolicy, Plane};
use serde_json::Value;

use super::shared::{premul, unpremul};
use crate::registry::FilterError;
use crate::traits::{Filter, FilterCtx, FilterGroup, FilterUi, PreviewPolicy};

pub struct CustomConvolve;

struct CustomParams {
    kernel: [[i32; 5]; 5],
    divisor: i32,
    offset: f32,
}

fn params_of(params: &Value) -> Result<CustomParams, FilterError> {
    let kernel = match params.get("kernel") {
        None => {
            let mut k = [[0i32; 5]; 5];
            k[2][2] = 1; // identity default
            k
        }
        Some(v) => {
            let rows = v.as_array().ok_or(FilterError::ParamOutOfRange {
                param: "kernel",
                detail: "kernel is a 5×5 integer matrix (§16.3)".into(),
            })?;
            if rows.len() != 5 {
                return Err(FilterError::ParamOutOfRange {
                    param: "kernel",
                    detail: "kernel needs exactly 5 rows".into(),
                });
            }
            let mut k = [[0i32; 5]; 5];
            for (j, row) in rows.iter().enumerate() {
                let cols = row.as_array().filter(|c| c.len() == 5).ok_or_else(|| {
                    FilterError::ParamOutOfRange {
                        param: "kernel",
                        detail: "each kernel row needs exactly 5 integers".into(),
                    }
                })?;
                for (i, c) in cols.iter().enumerate() {
                    let e = c.as_i64().unwrap_or(i64::MAX);
                    if !(-999..=999).contains(&e) {
                        return Err(FilterError::ParamOutOfRange {
                            param: "kernel",
                            detail: format!("entry ({j},{i}) outside −999..=999 (§16.3)"),
                        });
                    }
                    k[j][i] = e as i32;
                }
            }
            k
        }
    };
    let divisor = match params.get("divisor") {
        None => 1,
        Some(v) => {
            let d = v.as_i64().unwrap_or(0);
            if !(1..=9999).contains(&d) {
                return Err(FilterError::ParamOutOfRange {
                    param: "divisor",
                    detail: format!("{v} outside 1..=9999 (§16.3)"),
                });
            }
            d as i32
        }
    };
    let offset = match params.get("offset") {
        None => 0.0,
        Some(v) => {
            let o = v.as_f64().unwrap_or(f64::NAN);
            let o = if o.is_nan() { 0.0 } else { o };
            if !(-1.0..=1.0).contains(&o) {
                return Err(FilterError::ParamOutOfRange {
                    param: "offset",
                    detail: format!("{o} outside −1..=1 (§16.3)"),
                });
            }
            o as f32
        }
    };
    Ok(CustomParams {
        kernel,
        divisor,
        offset,
    })
}

impl Filter for CustomConvolve {
    fn id(&self) -> &'static str {
        "filter.custom"
    }

    fn param_schema(&self) -> &'static str {
        r#"{"$id":"https://appname.org/spec/filter/custom.v1.json","type":"object","additionalProperties":false,"properties":{"kernel":{"type":"array","minItems":5,"maxItems":5,"items":{"type":"array","minItems":5,"maxItems":5,"items":{"type":"integer","minimum":-999,"maximum":999}}},"divisor":{"type":"integer","minimum":1,"maximum":9999,"default":1},"offset":{"type":"number","minimum":-1,"maximum":1,"default":0}}}"#
    }

    fn ui_hints(&self) -> FilterUi {
        FilterUi {
            preview: PreviewPolicy::LiveFull,
            has_dialog: true,
            group: FilterGroup::Other,
            checkpoint: false,
        }
    }

    fn roi(&self, out: IRect, _params: &Value, _ctx: &FilterCtx) -> IRect {
        out.inflate(2, 2)
    }

    fn edge(&self) -> crate::traits::EdgeRule {
        // ES §16.3 Custom block: "edges clamp".
        crate::traits::EdgeRule::Clamp
    }

    fn gpu_key(&self) -> &'static str {
        "filter.custom" // one gather pass, nonzero taps unrolled at build — O-RENDER handoff
    }

    fn validate(&self, params: &Value) -> Result<(), FilterError> {
        params_of(params).map(|_| ())
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
        let cp = params_of(params)?;
        // Sparse tap list (zero entries skipped — §16.3).
        let taps: Vec<(i32, i32, f32)> = (0..5)
            .flat_map(|j| (0..5).map(move |i| (i, j)))
            .filter(|&(i, j)| cp.kernel[j as usize][i as usize] != 0)
            .map(|(i, j)| (i - 2, j - 2, cp.kernel[j as usize][i as usize] as f32))
            .collect();
        let inv_div = 1.0f32 / cp.divisor as f32;
        // Un-premultiply the apron once.
        let apron = out_rect.inflate(2, 2);
        let mut straight = Plane::new(apron);
        for y in apron.min.y..apron.max.y {
            for x in apron.min.x..apron.max.x {
                straight.set(x, y, unpremul(input.get(x, y, EdgePolicy::Clamp)));
            }
        }
        let mut out = Plane::new(out_rect);
        for y in out_rect.min.y..out_rect.max.y {
            for x in out_rect.min.x..out_rect.max.x {
                let mut acc = [0.0f32; 3];
                for &(dx, dy, w) in &taps {
                    let v = straight.get(x + dx, y + dy, EdgePolicy::Clamp);
                    for c in 0..3 {
                        acc[c] += v[c] * w;
                    }
                }
                let a = straight.get(x, y, EdgePolicy::Clamp)[3];
                let v = [
                    (acc[0] * inv_div + cp.offset).clamp(0.0, 1.0), // NORMATIVE clamp
                    (acc[1] * inv_div + cp.offset).clamp(0.0, 1.0),
                    (acc[2] * inv_div + cp.offset).clamp(0.0, 1.0),
                    a,
                ];
                out.set(x, y, premul(v));
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> FilterCtx {
        FilterCtx {
            canvas: IRect::new(0, 0, 64, 64),
            seed: 0,
            lod: 0,
        }
    }

    #[test]
    fn identity_kernel_is_a_no_op_and_defaults_apply() {
        let f = CustomConvolve;
        let extent = IRect::new(0, 0, 16, 16);
        let mut src = Plane::new(extent);
        for y in 0..16 {
            for x in 0..16 {
                let v = ((x * 5 + y * 11) % 13) as f32 / 12.0;
                src.set(x, y, [v, v * 0.5, 1.0 - v, 1.0]);
            }
        }
        let out = f.cpu(&src, extent, &serde_json::json!({}), &ctx()).unwrap();
        for (a, b) in out.px.iter().zip(src.px.iter()) {
            for c in 0..4 {
                assert!((a[c] - b[c]).abs() < 1e-6, "identity default");
            }
        }
    }

    #[test]
    fn box_kernel_matches_the_mean_and_divisor_scales() {
        // All-ones 5×5 with divisor 25 = the 5×5 box mean on straight RGB.
        let f = CustomConvolve;
        let extent = IRect::new(0, 0, 12, 12);
        let mut src = Plane::new(extent);
        for y in 0..12 {
            for x in 0..12 {
                let v = ((x + y) % 2) as f32; // checker
                src.set(x, y, [v, v, v, 1.0]);
            }
        }
        let ones = vec![vec![1; 5]; 5];
        let out = f
            .cpu(
                &src,
                extent,
                &serde_json::json!({"kernel": ones, "divisor": 25}),
                &ctx(),
            )
            .unwrap();
        // Interior checker mean: 12 or 13 of 25 ones → 0.48 / 0.52.
        let c = out.get(6, 6, EdgePolicy::Clamp)[0];
        assert!(
            (c - 0.48).abs() < 1e-5 || (c - 0.52).abs() < 1e-5,
            "box mean {c}"
        );
    }

    #[test]
    fn emboss_offset_and_normative_clamp() {
        let f = CustomConvolve;
        let extent = IRect::new(0, 0, 16, 4);
        let mut src = Plane::new(extent);
        for y in 0..4 {
            for x in 0..16 {
                let v = if x < 8 { 0.0 } else { 1.0 };
                src.set(x, y, [v, v, v, 1.0]);
            }
        }
        // Horizontal derivative kernel + 0.5 offset: flats → 0.5; the edge
        // saturates against the NORMATIVE clamp.
        let mut k = vec![vec![0; 5]; 5];
        k[2][1] = -1;
        k[2][3] = 1;
        let out = f
            .cpu(
                &src,
                extent,
                &serde_json::json!({"kernel": k, "offset": 0.5}),
                &ctx(),
            )
            .unwrap();
        assert!(
            (out.get(2, 2, EdgePolicy::Clamp)[0] - 0.5).abs() < 1e-6,
            "flat → offset"
        );
        assert_eq!(
            out.get(7, 2, EdgePolicy::Clamp)[0],
            1.0,
            "clamped high at the rising edge"
        );
        for p in &out.px {
            for v in p.iter().take(3) {
                assert!((0.0..=1.0).contains(v), "clamp contract at every depth");
            }
        }
    }

    #[test]
    fn alpha_passes_through_untouched() {
        let f = CustomConvolve;
        let extent = IRect::new(0, 0, 8, 8);
        let mut src = Plane::new(extent);
        for y in 0..8 {
            for x in 0..8 {
                let a = (1 + (x + y) % 4) as f32 / 4.0;
                src.set(x, y, [0.5 * a, 0.25 * a, a, a]);
            }
        }
        let mut k = vec![vec![0; 5]; 5];
        k[2][2] = 3; // brighten ×3, clamps
        let out = f
            .cpu(&src, extent, &serde_json::json!({"kernel": k}), &ctx())
            .unwrap();
        for (o, s) in out.px.iter().zip(src.px.iter()) {
            assert_eq!(o[3], s[3], "alpha untouched");
            assert!(o[0] <= o[3] + 1e-6, "premul validity after clamp");
        }
    }

    #[test]
    fn params_gate() {
        let f = CustomConvolve;
        assert!(f
            .validate(&serde_json::json!({"kernel": vec![vec![1000; 5]; 5]}))
            .is_err());
        assert!(f
            .validate(&serde_json::json!({"kernel": vec![vec![1; 4]; 5]}))
            .is_err());
        assert!(f.validate(&serde_json::json!({"divisor": 0})).is_err());
        assert!(f.validate(&serde_json::json!({"divisor": 10000})).is_err());
        assert!(f.validate(&serde_json::json!({"offset": 1.5})).is_err());
    }
}
