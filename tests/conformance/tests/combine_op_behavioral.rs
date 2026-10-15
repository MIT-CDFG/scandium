// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Combine-op BEHAVIORAL corpus — `selection.from_path` and
//! `selection.set{source:"quick"}` §13.2 combine ops over a prior
//! selection WIDER than the new cover, with INTERMEDIATE-STATE
//! assertions in the review-proven form (O-QA CP-1.0 ratification
//! batch; flagged by the core door-slice review [the intersect MAJOR's
//! coverage-gap MINOR] and by the O-TOOLS consolidated round
//! [subtract-over-wider-prior note]).
//!
//! Spec: ES §13.2 (combine expressions: replace / add = min(1, a+b) /
//! subtract = max(0, a−b) / intersect = min(a, b) — EVERYWHERE, not
//! only under the new cover); §13.3 (quick, step 4: alt ops = the same
//! growth under the §13.2 op); §20.2 (from_path exact-coverage fill);
//! contracts selection.from_path.yaml / selection.set.yaml (`op` enum);
//! D-48 (journal replay identity).
//!
//! The review-proven form (the shape that caught the intersect MAJOR
//! and that a broken before-capture could NOT survive, per the
//! confirming reviewer's intermediate-state extension): for every
//! combine op the battery captures the pre-command state hash, asserts
//! the op's OUTSIDE-the-cover witness (the pixel the MAJOR silently
//! left stale), then UNDOES and asserts (a) the state hash restores
//! BYTE-EXACT and (b) the outside witness pixel reads its prior value
//! again (outside tiles restore), then redoes and re-asserts the
//! post-hash. Journal replay closes each walk.
//!
//! The from_path leg drives core's Door-1 seam through a QA-side PROBE
//! geometry service (rect-sidecar algebra, coverage 0.75 — the same
//! probe convention as core's in-crate battery; the production impl is
//! appname-vector's, TEXTVEC Door 1). The probe exercises the SEAM and
//! the §13.2 kernel, not real geometry.
//!
//! RE-PIN ROUND (TEXTVEC Door-1 wiring merged, main e8f5f9b): the real
//! VectorGeometry service is LIVE on both production assemblies, so the
//! battery gains SERVICE-BACKED TWINS driven through
//! `assembly_point::assembled_engine()` (which regained shipped parity
//! this round): the same intersect/subtract/add walk over a REAL
//! `rasterize_fill` (§20.2 exact coverage — interior 1.0, not the
//! probe's 0.75), and a shape_add value-capture walk through the real
//! §20 constructor. The probe legs are KEPT verbatim — they guard the
//! bare-mirror seam (a service-less core still carries the Door-1
//! contract) and the 0.75-witness AA-vs-binarize distinguishability.

use std::sync::Arc;

// `assembly` (the bare-core mirror) left this import at the door-slice-2
// manifest-derived rig swap — the probe rig now folds PRODUCTION_CHAIN
// from seed() directly (O-QA ratification round: lint residue cleared).
use appname_conformance::assembly_point;
use appname_core::engine::vector_ops::{
    CoveragePlaneData, GeometryResult, VectorBoolOp, VectorGeometry, VECTOR_SIDECAR_SCHEMA,
};
use appname_core::{Engine, EngineError, ErrorCode, IRect, IVec2};
use serde_json::{json, Map, Value};

// ---------------------------------------------------------------------------
// QA probe geometry service (rect sidecars; mirrors core's in-crate probe
// convention: interior coverage 0.75 so AA-vs-binarize stays
// distinguishable and the recorded 0.75-witness shapes hold verbatim).
// ---------------------------------------------------------------------------

struct QaProbeGeom;

impl QaProbeGeom {
    fn parse(bytes: &[u8]) -> Result<(Vec<[i32; 4]>, String), EngineError> {
        let v: Value = serde_json::from_slice(bytes)
            .map_err(|e| EngineError::range("path", format!("sidecar not JSON: {e}")))?;
        if v.get("schema").and_then(Value::as_str) != Some(VECTOR_SIDECAR_SCHEMA) {
            return Err(EngineError::new(
                ErrorCode::WrongSchema,
                "sidecar schema must be appname.vector.v1",
            ));
        }
        let subs = v
            .get("subpaths")
            .and_then(Value::as_array)
            .ok_or_else(|| EngineError::missing("path.subpaths"))?;
        if subs.is_empty() {
            return Err(EngineError::range("path.subpaths", "empty path (§40.7)"));
        }
        let mut rects = Vec::new();
        for s in subs {
            let r = s
                .get("rect")
                .and_then(Value::as_array)
                .ok_or_else(|| EngineError::range("path.subpaths", "probe wants rects"))?;
            let g: Vec<i32> = r
                .iter()
                .filter_map(|x| x.as_i64())
                .map(|x| x as i32)
                .collect();
            if g.len() != 4 {
                return Err(EngineError::range(
                    "path.subpaths",
                    "rect must be [x,y,w,h]",
                ));
            }
            rects.push([g[0], g[1], g[2], g[3]]);
        }
        let rule = v
            .get("fillRule")
            .and_then(Value::as_str)
            .unwrap_or("nonzero")
            .to_string();
        Ok((rects, rule))
    }

    fn emit(rects: &[[i32; 4]], rule: &str) -> GeometryResult {
        let body = json!({
            "schema": VECTOR_SIDECAR_SCHEMA,
            "fillRule": rule,
            "subpaths": rects.iter().map(|r| json!({"rect": r})).collect::<Vec<_>>(),
        });
        GeometryResult {
            sidecar: serde_json::to_vec(&body).expect("probe emits"),
            bounds: Self::bounds_of(rects),
        }
    }

    fn bounds_of(rects: &[[i32; 4]]) -> IRect {
        let mut b = IRect::EMPTY;
        for [x, y, w, h] in rects {
            b = b.union(&IRect::new(*x, *y, x + w, y + h));
        }
        b
    }
}

impl VectorGeometry for QaProbeGeom {
    fn schema(&self) -> &'static str {
        VECTOR_SIDECAR_SCHEMA
    }
    fn validate_sidecar(&self, bytes: &[u8]) -> Result<GeometryResult, EngineError> {
        let (rects, rule) = Self::parse(bytes)?;
        Ok(Self::emit(&rects, &rule))
    }
    fn path_param_sidecar(&self, path: &Map<String, Value>) -> Result<GeometryResult, EngineError> {
        let mut v = Value::Object(path.clone());
        v["schema"] = json!(VECTOR_SIDECAR_SCHEMA);
        let (rects, rule) = Self::parse(&serde_json::to_vec(&v).expect("json"))?;
        Ok(Self::emit(&rects, &rule))
    }
    fn primitive_sidecar(
        &self,
        _kind: &str,
        primitive: &Map<String, Value>,
    ) -> Result<GeometryResult, EngineError> {
        // Battery never constructs primitives; keep the arm honest anyway.
        let g = primitive
            .get("geometry")
            .and_then(Value::as_object)
            .ok_or_else(|| EngineError::missing("primitive.geometry"))?;
        let get = |k: &str| g.get(k).and_then(Value::as_f64).unwrap_or(0.0) as i32;
        Ok(Self::emit(
            &[[get("x"), get("y"), get("w"), get("h")]],
            "nonzero",
        ))
    }
    fn path_boolean(
        &self,
        a: &[u8],
        b: &[u8],
        op: VectorBoolOp,
    ) -> Result<GeometryResult, EngineError> {
        let (ra, rule) = Self::parse(a)?;
        let (rb, _) = Self::parse(b)?;
        let rects = match op {
            VectorBoolOp::Union | VectorBoolOp::Exclude => {
                let mut r = ra;
                r.extend(rb);
                r
            }
            VectorBoolOp::Subtract => ra,
            VectorBoolOp::Intersect => {
                let ia = Self::bounds_of(&ra);
                let ib = Self::bounds_of(&rb);
                let i = ia.intersect(&ib);
                vec![[i.min.x, i.min.y, i.width(), i.height()]]
            }
        };
        Ok(Self::emit(&rects, &rule))
    }
    fn rasterize_fill(&self, sidecar: &[u8]) -> Result<CoveragePlaneData, EngineError> {
        let (rects, _) = Self::parse(sidecar)?;
        let rect = Self::bounds_of(&rects);
        let mut values = vec![0.0f32; (rect.width() * rect.height()).max(0) as usize];
        for [x, y, w, h] in &rects {
            for yy in *y..y + h {
                for xx in *x..x + w {
                    if xx >= rect.min.x && xx < rect.max.x && yy >= rect.min.y && yy < rect.max.y {
                        let idx = (yy - rect.min.y) as usize * rect.width() as usize
                            + (xx - rect.min.x) as usize;
                        values[idx] = 0.75;
                    }
                }
            }
        }
        Ok(CoveragePlaneData { rect, values })
    }
    fn apply_edit_ops(
        &self,
        sidecar: &[u8],
        _ops: &[Value],
    ) -> Result<GeometryResult, EngineError> {
        let (rects, rule) = Self::parse(sidecar)?;
        Ok(Self::emit(&rects, &rule))
    }
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// The production composition + the QA probe geometry, MANIFEST-DERIVED
/// (core door slice 2 MAJOR-1 ruled fix, TEST-FILE-ONLY cross-fence
/// companion flagged for O-QA ratification): the old hand chain here was
/// a STALE pre-vector copy whose "shipped-parity superset" claim went
/// false the moment production registered the REAL service — a direct
/// `production_assembly()` swap would double-register the seam under the
/// probe. The rig now folds `PRODUCTION_CHAIN` itself, filtering exactly
/// the `appname_vector::register` row it deliberately replaces (the
/// probe serves Door 1 — bare-mirror + 0.75-witness legs), so every
/// FUTURE production wrap reaches this rig automatically and the
/// partiality is explicit in one named dimension. The service-backed
/// twins ride `assembly_point::assembled_engine()` (the real chain).
fn assembled_engine_with_geom() -> Engine {
    appname_assembly::PRODUCTION_CHAIN
        .iter()
        .filter(|(name, _)| *name != "appname_vector::register")
        .fold(appname_assembly::seed(), |a, (_, wrap)| wrap(a))
        .with_vector_geometry(Arc::new(QaProbeGeom))
        .build()
}

fn env(n: u32, kind: &str, params: Value) -> String {
    json!({
        "schema": "appname.command.v1",
        "id": format!("c0b1e000-0000-4000-8000-0001{n:08x}"),
        "kind": kind,
        "params": params,
    })
    .to_string()
}

fn env_t(n: u32, kind: &str, layer: &str, params: Value) -> String {
    json!({
        "schema": "appname.command.v1",
        "id": format!("c0b1e000-0000-4000-8000-0001{n:08x}"),
        "kind": kind,
        "target": {"layerId": layer},
        "params": params,
    })
    .to_string()
}

fn sel_at(e: &Engine, x: i32, y: i32) -> f32 {
    e.document()
        .unwrap()
        .selection()
        .mask()
        .read_pixel_f32(IVec2::new(x, y))[0]
}

fn rect_path(x: i32, y: i32, w: i32, h: i32) -> Value {
    json!({"fillRule": "nonzero", "subpaths": [{"rect": [x, y, w, h]}]})
}

/// Apply `envelope`, assert the combine-op witnesses via `check`, then
/// walk the review-proven intermediate-state form: undo → state hash
/// byte-exact at `pre` AND every `(x, y, prior)` witness restored
/// (outside tiles restore) → redo → post-hash byte-exact again.
fn combine_walk(
    e: &mut Engine,
    envelope: &str,
    witnesses_prior: &[(i32, i32, f32)],
    check: impl Fn(&Engine),
) {
    let pre = e.state_hash();
    for &(x, y, want) in witnesses_prior {
        assert_eq!(sel_at(e, x, y), want, "prior witness at ({x},{y})");
    }
    e.apply_json(envelope).unwrap();
    check(e);
    let post = e.state_hash();
    e.undo().unwrap();
    assert_eq!(
        e.state_hash(),
        pre,
        "undo restores the pre-combine state BYTE-EXACT"
    );
    for &(x, y, want) in witnesses_prior {
        assert_eq!(
            sel_at(e, x, y),
            want,
            "outside tiles restore after undo at ({x},{y})"
        );
    }
    e.redo().unwrap();
    assert_eq!(e.state_hash(), post, "redo re-lands the combine byte-exact");
}

// ---------------------------------------------------------------------------
// selection.from_path — the recorded shapes verbatim (the intersect
// MAJOR's probe pair + the add case), through the assembled surface.
// ---------------------------------------------------------------------------

#[test]
fn from_path_combine_ops_over_wider_prior_intermediate_state() {
    let mut e = assembled_engine_with_geom();
    e.apply_json(&env(
        1,
        "doc.new",
        json!({"width": 512, "height": 512, "background": "white"}),
    ))
    .unwrap();

    let big = uuid::Uuid::new_v4();
    let small = uuid::Uuid::new_v4();
    e.apply_json(&env(
        2,
        "vector.path_new",
        json!({"path": rect_path(0, 0, 512, 512), "newPathId": big.to_string()}),
    ))
    .unwrap();
    e.apply_json(&env(
        3,
        "vector.path_new",
        json!({"path": rect_path(10, 10, 20, 20), "newPathId": small.to_string()}),
    ))
    .unwrap();

    let wide_prior = |e: &mut Engine, n: u32| {
        e.apply_json(&env(
            n,
            "selection.from_path",
            json!({"pathId": big.to_string()}),
        ))
        .unwrap();
        assert_eq!(
            sel_at(e, 400, 400),
            0.75,
            "wide prior covers the far witness"
        );
    };
    // far = (400,400): outside the small path, in another tile — the
    // pixel the intersect MAJOR silently left stale. inside = (15,15).
    let priors: &[(i32, i32, f32)] = &[(400, 400, 0.75), (15, 15, 0.75)];

    // intersect: min(a, b) with b = 0 outside — far witness FLIPS to 0,
    // bounds tighten off the 512 canvas hull (tile-granular recompute).
    wide_prior(&mut e, 4);
    combine_walk(
        &mut e,
        &env(
            5,
            "selection.from_path",
            json!({"pathId": small.to_string(), "op": "intersect"}),
        ),
        priors,
        |e| {
            assert_eq!(
                sel_at(e, 400, 400),
                0.0,
                "intersect clears OUTSIDE the path cover (§13.2 min(a, b), b = 0)"
            );
            assert_eq!(sel_at(e, 15, 15), 0.75, "inside: min(0.75, 0.75)");
            assert_eq!(
                e.document().unwrap().selection().bounds(),
                IRect::new(0, 0, 256, 256),
                "bounds tighten to the surviving tile hull (tile-granular \
                 recompute, conservative superset of the exact 10..30 rect)"
            );
        },
    );

    // subtract: max(0, a − b) — outside the path the prior STAYS.
    wide_prior(&mut e, 6);
    combine_walk(
        &mut e,
        &env(
            7,
            "selection.from_path",
            json!({"pathId": small.to_string(), "op": "subtract"}),
        ),
        priors,
        |e| {
            assert_eq!(sel_at(e, 400, 400), 0.75, "subtract: outside stays");
            assert_eq!(sel_at(e, 15, 15), 0.0, "inside: max(0, 0.75 − 0.75)");
        },
    );

    // add: min(1, a + b) — outside unchanged, inside saturates.
    wide_prior(&mut e, 8);
    combine_walk(
        &mut e,
        &env(
            9,
            "selection.from_path",
            json!({"pathId": small.to_string(), "op": "add"}),
        ),
        priors,
        |e| {
            assert_eq!(sel_at(e, 400, 400), 0.75, "add: outside unchanged");
            assert_eq!(sel_at(e, 15, 15), 1.0, "inside: min(1, 0.75 + 0.75)");
        },
    );

    // D-48: the whole walk replays bit-exactly (same assembly, fresh
    // engine — the probe service is deterministic by construction).
    let replayed = e.replay_assembled(e.journal()).unwrap();
    assert_eq!(
        replayed.state_hash(),
        e.state_hash(),
        "combine-op journal replays byte-identically"
    );
}

// ---------------------------------------------------------------------------
// SERVICE-BACKED TWINS (re-pin round): the same walks through the
// production assembly — appname-vector's real VectorGeometry, real §20.2
// exact-coverage rasterize_fill, real §20 constructors.
// ---------------------------------------------------------------------------

/// A closed axis-aligned rectangle in the REAL `appname.vector.v1` path
/// wire form (cubic segments with collinear control points at the third
/// points — the strict-writer segment shape).
fn real_rect_path(x: f64, y: f64, w: f64, h: f64) -> Value {
    let corners = [(x, y), (x + w, y), (x + w, y + h), (x, y + h)];
    let segments: Vec<Value> = (0..4)
        .map(|i| {
            let (ax, ay) = corners[i];
            let (bx, by) = corners[(i + 1) % 4];
            json!({
                "p0": [ax, ay],
                "c1": [ax + (bx - ax) / 3.0, ay + (by - ay) / 3.0],
                "c2": [ax + 2.0 * (bx - ax) / 3.0, ay + 2.0 * (by - ay) / 3.0],
                "p1": [bx, by],
            })
        })
        .collect();
    json!({"fillRule": "nonzero",
           "subpaths": [{"closed": true, "segments": segments}]})
}

/// The intersect/subtract/add walk of the probe leg, verbatim shapes,
/// through the PRODUCTION service: real exact-coverage fill — the
/// interior of an integer-aligned rect covers at 1.0 (the probe twin
/// pins 0.75 there; the pair keeps AA-vs-binarize distinguishable
/// ACROSS legs).
#[test]
fn from_path_combine_ops_service_backed_twin() {
    let mut e = assembly_point::assembled_engine();
    e.apply_json(&env(
        1,
        "doc.new",
        json!({"width": 512, "height": 512, "background": "white"}),
    ))
    .unwrap();

    let big = uuid::Uuid::new_v4();
    let small = uuid::Uuid::new_v4();
    e.apply_json(&env(
        2,
        "vector.path_new",
        json!({"path": real_rect_path(0.0, 0.0, 512.0, 512.0),
               "newPathId": big.to_string()}),
    ))
    .unwrap();
    e.apply_json(&env(
        3,
        "vector.path_new",
        json!({"path": real_rect_path(10.0, 10.0, 20.0, 20.0),
               "newPathId": small.to_string()}),
    ))
    .unwrap();

    let wide_prior = |e: &mut Engine, n: u32| {
        e.apply_json(&env(
            n,
            "selection.from_path",
            json!({"pathId": big.to_string()}),
        ))
        .unwrap();
        assert_eq!(
            sel_at(e, 400, 400),
            1.0,
            "REAL exact coverage: rect interior is fully selected"
        );
    };
    let priors: &[(i32, i32, f32)] = &[(400, 400, 1.0), (15, 15, 1.0)];

    // intersect: min(a, b) with b = 0 outside — the far witness FLIPS.
    wide_prior(&mut e, 4);
    combine_walk(
        &mut e,
        &env(
            5,
            "selection.from_path",
            json!({"pathId": small.to_string(), "op": "intersect"}),
        ),
        priors,
        |e| {
            assert_eq!(
                sel_at(e, 400, 400),
                0.0,
                "intersect clears OUTSIDE the path cover (§13.2 min(a, b), \
                 b = 0) — the MAJOR's class, now pinned on the REAL service"
            );
            assert_eq!(sel_at(e, 15, 15), 1.0, "inside: min(1, 1)");
            assert_eq!(
                e.document().unwrap().selection().bounds(),
                IRect::new(0, 0, 256, 256),
                "bounds tighten to the surviving tile hull (tile-granular \
                 recompute — core-side, geometry-independent: same hull as \
                 the probe leg)"
            );
        },
    );

    // subtract: max(0, a − b) — outside the path the prior STAYS.
    wide_prior(&mut e, 6);
    combine_walk(
        &mut e,
        &env(
            7,
            "selection.from_path",
            json!({"pathId": small.to_string(), "op": "subtract"}),
        ),
        priors,
        |e| {
            assert_eq!(sel_at(e, 400, 400), 1.0, "subtract: outside stays");
            assert_eq!(sel_at(e, 15, 15), 0.0, "inside: max(0, 1 − 1)");
        },
    );

    // add: min(1, a + b) — outside unchanged, inside saturates.
    wide_prior(&mut e, 8);
    combine_walk(
        &mut e,
        &env(
            9,
            "selection.from_path",
            json!({"pathId": small.to_string(), "op": "add"}),
        ),
        priors,
        |e| {
            assert_eq!(sel_at(e, 400, 400), 1.0, "add: outside unchanged");
            assert_eq!(sel_at(e, 15, 15), 1.0, "inside: min(1, 1 + 1)");
        },
    );

    // D-48: the walk replays bit-exactly through the REAL service (the
    // §20.2 rasterizer is deterministic; from_path re-rasterizes on
    // replay — coverage is derived state, not a capture).
    let replayed = e.replay_assembled(e.journal()).unwrap();
    assert_eq!(
        replayed.state_hash(),
        e.state_hash(),
        "service-backed combine-op journal replays byte-identically"
    );
}

/// shape_add through the REAL §20 constructor: live accept on the
/// production assembly, the D-48 value-captured sidecar surviving
/// replay AND the package round trip (the public-surface counterpart of
/// core's in-crate nondeterministic-probe pin — here the REAL
/// constructor is deterministic, so the capture pin rides on the
/// persisted-hash equalities plus save→load→save byte stability).
#[test]
fn shape_add_value_capture_through_the_real_constructor() {
    let mut e = assembly_point::assembled_engine();
    e.apply_json(&env(
        1,
        "doc.new",
        json!({"width": 96, "height": 64, "background": "white"}),
    ))
    .unwrap();
    let pre = e.state_hash();
    let nl = uuid::Uuid::new_v4();
    e.apply_json(&env(
        2,
        "vector.shape_add",
        json!({"primitive": {"kind": "ellipse",
                             "geometry": {"x": 10, "y": 10, "w": 30, "h": 20}},
               "fill": {"type": "solid",
                        "color": {"model": "rgba", "comps": [0.9, 0.2, 0.2, 1]}},
               "stroke": {"widthPx": 0},
               "newLayerId": nl.to_string()}),
    ))
    .unwrap();
    let doc = e.document().unwrap();
    assert_eq!(doc.z_order_walk().len(), 2, "shape layer created");
    let post = e.state_hash();

    // Undo/redo byte-identity around the real constructor.
    e.undo().unwrap();
    assert_eq!(e.state_hash(), pre, "undo removes the shape byte-exact");
    e.redo().unwrap();
    assert_eq!(e.state_hash(), post, "redo re-lands the capture byte-exact");

    // Replay consumes the capture in-process…
    let replayed = e.replay_assembled(e.journal()).unwrap();
    assert_eq!(replayed.state_hash(), post, "replay byte-identity (D-48)");

    // …and across the package boundary (captures persist beside the
    // journal; the reopened engine replays them too).
    let bytes = appname_core::save_package_bytes(&e).unwrap();
    let asm = assembly_point::assembled_assembly();
    let loaded = appname_core::load_package_bytes_with(&asm, &bytes).unwrap();
    assert_eq!(
        loaded.engine.state_hash_persistent(),
        e.state_hash_persistent(),
        "package round trip preserves the shape doc through the REAL \
         service's validate_sidecar"
    );
    let replayed = loaded
        .engine
        .replay_assembled(loaded.engine.journal())
        .unwrap();
    assert_eq!(
        replayed.state_hash_persistent(),
        e.state_hash_persistent(),
        "post-reopen replay consumes the PERSISTED capture (D-48)"
    );
    let again = appname_core::save_package_bytes(&loaded.engine).unwrap();
    let third = appname_core::load_package_bytes_with(&asm, &again).unwrap();
    assert_eq!(
        third.engine.state_hash_persistent(),
        loaded.engine.state_hash_persistent(),
        "save→load→save byte stability with vector content aboard"
    );
}

// ---------------------------------------------------------------------------
// selection.set{source:"quick"} — the O-TOOLS-flagged shapes: the same
// §13.2 combine walk with the quick growth as the incoming cover.
// ---------------------------------------------------------------------------

/// Two-tone doc for the quick legs (the tools battery's convention):
/// left half (x < 48) red, right half white, 96×64, selection cleared.
fn two_tone_engine() -> (Engine, String) {
    let mut e = assembled_engine_with_geom();
    e.apply_json(&env(
        1,
        "doc.new",
        json!({"width": 96, "height": 64, "background": "white"}),
    ))
    .unwrap();
    let layer = e.document().unwrap().z_order_walk()[0].to_string();
    e.apply_json(&env(
        2,
        "selection.set",
        json!({"source": "rect", "rect": {"x": 0, "y": 0, "w": 48, "h": 64},
                "antialias": false}),
    ))
    .unwrap();
    e.apply_json(&env_t(
        3,
        "fill",
        &layer,
        json!({"content": "color",
                "color": {"model": "rgb", "comps": [1.0, 0.0, 0.0, 1.0]}}),
    ))
    .unwrap();
    e.apply_json(&env(4, "selection.none", json!({}))).unwrap();
    (e, layer)
}

#[test]
fn quick_subtract_over_wider_prior_intermediate_state() {
    let (mut e, _) = two_tone_engine();
    // Prior WIDER than the quick growth: the whole canvas.
    e.apply_json(&env(5, "selection.all", json!({}))).unwrap();
    let priors: &[(i32, i32, f32)] = &[(10, 32, 1.0), (40, 5, 1.0), (70, 32, 1.0)];
    combine_walk(
        &mut e,
        &env(
            6,
            "selection.set",
            json!({"source": "quick", "seed": [10, 32], "op": "subtract"}),
        ),
        priors,
        |e| {
            assert_eq!(sel_at(e, 10, 32), 0.0, "brushed red surface removed");
            assert_eq!(sel_at(e, 40, 5), 0.0, "far red corner removed");
            assert_eq!(
                sel_at(e, 70, 32),
                1.0,
                "OUTSIDE the quick growth the wider prior stays (§13.2 \
                 subtract identity, a − 0 = a)"
            );
        },
    );
    let replayed = e.replay_assembled(e.journal()).unwrap();
    assert_eq!(replayed.state_hash(), e.state_hash());
}

#[test]
fn quick_intersect_over_wider_prior_intermediate_state() {
    let (mut e, _) = two_tone_engine();
    e.apply_json(&env(5, "selection.all", json!({}))).unwrap();
    let priors: &[(i32, i32, f32)] = &[(10, 32, 1.0), (70, 32, 1.0), (90, 5, 1.0)];
    combine_walk(
        &mut e,
        &env(
            6,
            "selection.set",
            json!({"source": "quick", "seed": [10, 32], "op": "intersect"}),
        ),
        priors,
        |e| {
            assert_eq!(sel_at(e, 10, 32), 1.0, "inside: min(1, 1)");
            assert_eq!(
                sel_at(e, 70, 32),
                0.0,
                "intersect clears OUTSIDE the quick growth (min(a, b), b = 0) \
                 — the from_path MAJOR's class, pinned on the quick arm too"
            );
            assert_eq!(sel_at(e, 90, 5), 0.0, "far outside tile clears");
        },
    );
    let replayed = e.replay_assembled(e.journal()).unwrap();
    assert_eq!(replayed.state_hash(), e.state_hash());
}

#[test]
fn quick_add_over_wider_prior_intermediate_state() {
    let (mut e, _) = two_tone_engine();
    // Prior wider than the quick growth but NOT the whole canvas
    // (x < 72), so add has both an inside-prior and an outside-everything
    // witness.
    e.apply_json(&env(
        5,
        "selection.set",
        json!({"source": "rect", "rect": {"x": 0, "y": 0, "w": 72, "h": 64},
                "antialias": false}),
    ))
    .unwrap();
    let priors: &[(i32, i32, f32)] = &[(10, 32, 1.0), (60, 32, 1.0), (90, 5, 0.0)];
    combine_walk(
        &mut e,
        &env(
            6,
            "selection.set",
            json!({"source": "quick", "seed": [10, 32], "op": "add"}),
        ),
        priors,
        |e| {
            assert_eq!(sel_at(e, 10, 32), 1.0, "inside both: min(1, 1 + 1)");
            assert_eq!(
                sel_at(e, 60, 32),
                1.0,
                "prior-only region unchanged (add identity, a + 0 = a)"
            );
            assert_eq!(
                sel_at(e, 90, 5),
                0.0,
                "outside both the prior and the growth stays empty"
            );
        },
    );
    let replayed = e.replay_assembled(e.journal()).unwrap();
    assert_eq!(replayed.state_hash(), e.state_hash());
}
