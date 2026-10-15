// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! `filter.liquify` — ES §16.3 "Liquify" block + §45.4 [NORMATIVE]
//! commit-record reconciliation: "the raster commit is ONE
//! `filter.apply{filterId:"filter.liquify", params:{meshPitch, disp}}`
//! with `checkpoint:true` declared in the filter's `FilterUi` — class C,
//! inverse snap, history label `history.filter.apply` — executing exactly
//! one full-res §18.2 bicubic resample through `D`; on a smart-object
//! target the same params land as a re-editable smart filter via
//! `filter.smart_add` (§12.2)". The compute engine is
//! `appname_tools::liquify::commit_plane`; the workspace machinery
//! (brushes, field editing, `.bmesh`) lives in that module — this file is
//! the §16.1 registry citizen the command seam executes.
//!
//! Params (schema below): `meshPitch ∈ {4, 8, 16, 32}` = 16 ·
//! `origin:[x, y]` (doc px, node (0,0)) · `size:[nx, ny]` (nodes per
//! axis, ≥ 1) · `disp:[2·nx·ny numbers]` (row-major (dx, dy) px pairs) ·
//! `backdropOpacity 0..=1` = 0.5 (workspace preview state — accepted and
//! validated for §16.3 param-table fidelity, INERT at commit; recorded
//! pin).
//!
//! RECORDED PIN + ESCALATION (lane log this date): the §45.4 resolution's
//! shorthand names the commit params `{meshPitch, disp}`, but a
//! displacement FIELD is not self-describing without its grid geometry —
//! the §16.3 `.bmesh` schema itself carries `pitch`/`origin`/`size`/
//! `disp` as the field's intrinsic shape, and §16.1 requires a filter's
//! params to validate standalone ("adding a filter requires no core
//! edits" — the registry cannot reach into workspace state). The wire
//! params here are therefore the complete field payload (the `.bmesh`
//! body minus the schema id), which also makes §45.4's
//! "commit-equals-recorded-params replay" well-defined. Flagged to O-INT
//! for the fold-in register (contract-shorthand vs self-describing-params
//! tension); never-invent rule honored by carrying the §16.3 mesh-schema
//! field set verbatim.
//!
//! ROI: `out.inflate(⌈max|disp|⌉ + 2)` — the backward map reads at most
//! the field's max displacement away, plus the Catmull-Rom 2-px apron.
//! Never pointwise (an empty/default field still declares the kernel
//! apron — the op-bridge pointwise probe must not degrade the evaluator's
//! apron materialization; bridge-pin consequence). **Edges.** transparent
//! (§16.3; §45.4 "no wraparound, ever"). **GPU.** field in a small
//! `rg32float` texture, one remap pass — pipeline key `filter.liquify`
//! (O-RENDER §34.3 handoff; CPU here is the parity oracle). **Preview.**
//! `LiveProxy` (§16.3/§45.4). Rights: `warps_upright` row basis — see
//! the `appname_tools::liquify` module docs (P-7312805 quoted-status
//! expired-lifetime, verified 2026-07-24; Wolberg-era L0; V39
//! select-and-screen judgment, no release claim).

use appname_core::IRect;
use appname_tools::liquify::{LiquifyField, LIQUIFY_PITCHES};
use appname_tools::resample::Plane;
use serde_json::Value;

use crate::registry::FilterError;
use crate::traits::{Filter, FilterCtx, FilterGroup, FilterUi, PreviewPolicy};

pub struct Liquify;

fn int_of(v: &Value, param: &'static str) -> Result<i64, FilterError> {
    v.as_i64().ok_or(FilterError::ParamOutOfRange {
        param,
        detail: "must be an integer".into(),
    })
}

/// Parse + validate the complete field payload (module-doc param set).
fn field_of(params: &Value) -> Result<LiquifyField, FilterError> {
    let pitch = match params.get("meshPitch") {
        None => 16,
        Some(v) => {
            let p = int_of(v, "meshPitch")?;
            if !LIQUIFY_PITCHES.contains(&(p as i32)) {
                return Err(FilterError::ParamOutOfRange {
                    param: "meshPitch",
                    detail: format!("{p} outside {{4, 8, 16, 32}} (§45.4)"),
                });
            }
            p as i32
        }
    };
    let pair = |name: &'static str| -> Result<[i64; 2], FilterError> {
        let arr = params
            .get(name)
            .ok_or(FilterError::ParamMissing(name))?
            .as_array()
            .ok_or(FilterError::ParamOutOfRange {
                param: "origin",
                detail: "must be a two-entry array".into(),
            })?;
        if arr.len() != 2 {
            return Err(FilterError::ParamOutOfRange {
                param: "origin",
                detail: "must be a two-entry array".into(),
            });
        }
        Ok([int_of(&arr[0], "origin")?, int_of(&arr[1], "origin")?])
    };
    let origin = pair("origin")?;
    let (ox, oy) = (
        i32::try_from(origin[0]).map_err(|_| FilterError::ParamOutOfRange {
            param: "origin",
            detail: "out of i32 range".into(),
        })?,
        i32::try_from(origin[1]).map_err(|_| FilterError::ParamOutOfRange {
            param: "origin",
            detail: "out of i32 range".into(),
        })?,
    );
    let size = {
        let arr = params
            .get("size")
            .ok_or(FilterError::ParamMissing("size"))?
            .as_array()
            .ok_or(FilterError::ParamOutOfRange {
                param: "size",
                detail: "must be [nx, ny]".into(),
            })?;
        if arr.len() != 2 {
            return Err(FilterError::ParamOutOfRange {
                param: "size",
                detail: "must be [nx, ny]".into(),
            });
        }
        let nx = int_of(&arr[0], "size")?;
        let ny = int_of(&arr[1], "size")?;
        if nx < 1 || ny < 1 {
            return Err(FilterError::ParamOutOfRange {
                param: "size",
                detail: "node counts must be >= 1 (§45.4)".into(),
            });
        }
        [nx as usize, ny as usize]
    };
    let disp = params
        .get("disp")
        .ok_or(FilterError::ParamMissing("disp"))?
        .as_array()
        .ok_or(FilterError::ParamOutOfRange {
            param: "disp",
            detail: "must be a number array".into(),
        })?;
    let expected = size[0]
        .checked_mul(size[1])
        .and_then(|n| n.checked_mul(2))
        .ok_or(FilterError::ParamOutOfRange {
            param: "size",
            detail: "node-count product overflows".into(),
        })?;
    if disp.len() != expected {
        return Err(FilterError::ParamOutOfRange {
            param: "disp",
            detail: format!(
                "length {} != 2·size.x·size.y = {expected} (§45.4)",
                disp.len()
            ),
        });
    }
    let mut nodes = Vec::with_capacity(size[0] * size[1]);
    for pairv in disp.chunks_exact(2) {
        let dx = pairv[0].as_f64();
        let dy = pairv[1].as_f64();
        let (Some(dx), Some(dy)) = (dx, dy) else {
            return Err(FilterError::ParamOutOfRange {
                param: "disp",
                detail: "entries must be numbers".into(),
            });
        };
        if !dx.is_finite() || !dy.is_finite() {
            return Err(FilterError::ParamOutOfRange {
                param: "disp",
                detail: "entries must be finite (§0.3)".into(),
            });
        }
        nodes.push([dx as f32, dy as f32]);
    }
    if let Some(v) = params.get("backdropOpacity") {
        let x = v.as_f64().unwrap_or(f64::NAN);
        if !(0.0..=1.0).contains(&x) {
            return Err(FilterError::ParamOutOfRange {
                param: "backdropOpacity",
                detail: "outside 0..=1 (§16.3)".into(),
            });
        }
        // Workspace preview state — inert at commit (module-doc pin).
    }
    Ok(LiquifyField {
        pitch,
        origin: [ox, oy],
        nodes_x: size[0],
        nodes_y: size[1],
        disp: nodes,
        spacing: [f64::from(pitch), f64::from(pitch)],
    })
}

impl Filter for Liquify {
    fn id(&self) -> &'static str {
        "filter.liquify"
    }

    fn param_schema(&self) -> &'static str {
        r#"{"$id":"https://appname.org/spec/filter/liquify.v1.json","type":"object","additionalProperties":false,"required":["origin","size","disp"],"properties":{"meshPitch":{"type":"integer","enum":[4,8,16,32],"default":16,"description":"px (§16.3/§45.4)"},"origin":{"type":"array","minItems":2,"maxItems":2,"items":{"type":"integer"},"description":"doc-px position of node (0,0) (§45.4 field over the layer bounds)"},"size":{"type":"array","minItems":2,"maxItems":2,"items":{"type":"integer","minimum":1},"description":"nodes per axis (§45.4)"},"disp":{"type":"array","items":{"type":"number"},"description":"row-major (dx, dy) px pairs, length 2*size.x*size.y (§16.3)"},"backdropOpacity":{"type":"number","minimum":0,"maximum":1,"default":0.5,"description":"workspace preview state; inert at commit (§16.3, recorded pin)"}}}"#
    }

    fn ui_hints(&self) -> FilterUi {
        FilterUi {
            preview: PreviewPolicy::LiveProxy,
            has_dialog: true, // the §32.5 modal workspace
            group: FilterGroup::Distort,
            // §45.4 [NORMATIVE] commit-record resolution: the checkpoint
            // arm's named instance.
            checkpoint: true,
        }
    }

    fn roi(&self, out: IRect, params: &Value, _ctx: &FilterCtx) -> IRect {
        // Max |displacement| + the CR 2-px apron; an unparsable field
        // contributes 0 (validation rejects it before execution — the
        // apron floor keeps the probe honest, module doc).
        let maxd = field_of(params).map(|f| f.max_disp()).unwrap_or(0.0);
        let r = maxd.ceil() as i32 + 2;
        out.inflate(r, r)
    }

    fn edge(&self) -> crate::traits::EdgeRule {
        // §16.3: outside canvas → transparent; §45.4 "no wraparound".
        crate::traits::EdgeRule::Transparent
    }

    fn gpu_key(&self) -> &'static str {
        "filter.liquify"
    }

    fn validate(&self, params: &Value) -> Result<(), FilterError> {
        field_of(params).map(|_| ())
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
        let field = field_of(params)?;
        Ok(appname_tools::liquify::commit_plane(
            input, &field, out_rect,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use appname_tools::resample::EdgePolicy;
    use serde_json::json;

    fn ctx(rect: IRect) -> FilterCtx {
        FilterCtx {
            canvas: rect,
            seed: 0,
            lod: 0,
        }
    }

    fn zero_params(nx: usize, ny: usize) -> Value {
        json!({
            "meshPitch": 16,
            "origin": [0, 0],
            "size": [nx, ny],
            "disp": vec![0.0; nx * ny * 2],
        })
    }

    fn checker(rect: IRect) -> Plane {
        let mut p = Plane::new(rect);
        for y in rect.min.y..rect.max.y {
            for x in rect.min.x..rect.max.x {
                let v = if (x / 4 + y / 4) % 2 == 0 { 1.0 } else { 0.25 };
                p.set(x, y, [v, v * 0.5, 1.0 - v, 1.0]);
            }
        }
        p
    }

    #[test]
    fn schema_and_hints_declare_the_45_4_shape() {
        let f = Liquify;
        assert_eq!(f.id(), "filter.liquify");
        let ui = f.ui_hints();
        assert!(ui.checkpoint, "§45.4: checkpoint:true in FilterUi");
        assert_eq!(ui.preview as u8, PreviewPolicy::LiveProxy as u8);
        let schema: Value = serde_json::from_str(f.param_schema()).unwrap();
        assert_eq!(
            schema["properties"]["meshPitch"]["enum"],
            json!([4, 8, 16, 32])
        );
    }

    #[test]
    fn validate_reject_matrix() {
        let f = Liquify;
        assert!(f.validate(&zero_params(3, 3)).is_ok());
        let cases = vec![
            (json!({"origin": [0,0], "size": [1,1]}), "disp missing"),
            (json!({"origin": [0,0], "disp": [0.0, 0.0]}), "size missing"),
            (json!({"size": [1,1], "disp": [0.0, 0.0]}), "origin missing"),
            (
                json!({"meshPitch": 5, "origin": [0,0], "size": [1,1], "disp": [0.0, 0.0]}),
                "pitch outside the set",
            ),
            (
                json!({"origin": [0,0], "size": [2,1], "disp": [0.0, 0.0]}),
                "disp length mismatch",
            ),
            (
                json!({"origin": [0,0], "size": [0,1], "disp": []}),
                "zero node axis",
            ),
            (
                json!({"origin": [0,0], "size": [1,1], "disp": [0.0, "x"]}),
                "non-number disp entry",
            ),
            (
                json!({"origin": [0,0], "size": [1,1], "disp": [0.0, 0.0], "backdropOpacity": 1.5}),
                "backdropOpacity outside 0..=1",
            ),
            (
                json!({"origin": [0.5, 0], "size": [1,1], "disp": [0.0, 0.0]}),
                "fractional origin",
            ),
        ];
        for (p, why) in cases {
            assert!(f.validate(&p).is_err(), "{why}");
        }
    }

    #[test]
    fn roi_tracks_max_displacement_and_is_never_pointwise() {
        let f = Liquify;
        let out = IRect::new(0, 0, 8, 8);
        let c = ctx(out);
        // Zero field: kernel apron only.
        assert_eq!(f.roi(out, &zero_params(2, 2), &c), out.inflate(2, 2));
        // A 10.4-px displacement: ⌈10.4⌉ + 2 = 13.
        let mut p = zero_params(2, 2);
        p["disp"][0] = json!(-10.4);
        assert_eq!(f.roi(out, &p, &c), out.inflate(13, 13));
        // The op-bridge pointwise probe (empty params) keeps the apron.
        assert_ne!(
            f.roi(IRect::new(0, 0, 1, 1), &json!({}), &c),
            IRect::new(0, 0, 1, 1)
        );
    }

    #[test]
    fn identity_field_is_bit_identical_through_the_filter() {
        // §45.8 prop_liquify_identity at the registry-citizen level.
        let rect = IRect::new(0, 0, 32, 32);
        let src = checker(rect);
        let f = Liquify;
        let out = f.cpu(&src, rect, &zero_params(3, 3), &ctx(rect)).unwrap();
        assert_eq!(out, src);
    }

    #[test]
    fn displaced_field_reads_through_the_backward_map() {
        // The lower-right grid CELL pushed by (+4, 0): pixels inside the
        // cell sample exactly 4 px right (all four cell corners agree,
        // and the bilinear weights sum exactly to 1 in f64); pixels
        // OUTSIDE the node extent read undisplaced (module pin).
        let rect = IRect::new(0, 0, 48, 48);
        let src = checker(rect);
        let f = Liquify;
        // 3×3 grid at pitch 16 covers [0, 32]²; the canvas extends past
        // it so the outside-extent rule is observable.
        let mut params = zero_params(3, 3);
        for idx in [4usize, 5, 7, 8] {
            // Nodes (1,1), (2,1), (1,2), (2,2) — the cell [16,32]².
            params["disp"][2 * idx] = json!(4.0);
        }
        let roi = f.roi(rect, &params, &ctx(rect));
        // Materialize the padded input (evaluator duty in production).
        let mut input = Plane::new(roi);
        for y in roi.min.y..roi.max.y {
            for x in roi.min.x..roi.max.x {
                input.set(x, y, src.get(x, y, EdgePolicy::Transparent));
            }
        }
        let out = f.cpu(&input, rect, &params, &ctx(rect)).unwrap();
        // Inside the pushed cell: out(p) = src(p + 4) exactly.
        for (x, y) in [(16, 16), (20, 24), (31, 31)] {
            assert_eq!(
                out.get(x, y, EdgePolicy::Transparent),
                src.get(x + 4, y, EdgePolicy::Transparent),
                "({x},{y})"
            );
        }
        // Outside the node extent: bit-exact unchanged.
        assert_eq!(
            out.get(40, 40, EdgePolicy::Transparent),
            src.get(40, 40, EdgePolicy::Transparent)
        );
        // Determinism ×2.
        let out2 = f.cpu(&input, rect, &params, &ctx(rect)).unwrap();
        assert_eq!(out, out2);
    }

    #[test]
    fn roi_contract_violation_is_typed() {
        let rect = IRect::new(0, 0, 8, 8);
        let src = checker(IRect::new(0, 0, 4, 4));
        let err = Liquify
            .cpu(&src, rect, &zero_params(2, 2), &ctx(rect))
            .unwrap_err();
        assert_eq!(err, FilterError::RoiNotCovered);
    }
}
