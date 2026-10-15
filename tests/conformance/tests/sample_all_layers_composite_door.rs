// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! §45.2 / §13.3 SAMPLE-ALL-LAYERS — proved at the **COMPOSITE/EXPORT
//! DOOR**, not merely at the command door.
//!
//! The standing rule this file exists for: there are live rows where the
//! command door ACCEPTS and the composite door REFUSES typed (the Q-3
//! masked-transform family is the worked example). This slice makes wand /
//! heal / clone SAMPLE the composite, so "the command returned Ok" is not
//! evidence that the feature works — the deposit has to survive to the
//! reference export. Every serve leg below therefore ends in
//! `export_png_bytes` and reads the exported pixels back.
//!
//! Driven through `assembly_point::assembled_engine()` — the PRODUCTION
//! chain (`appname_assembly::PRODUCTION_CHAIN`), never a hand-rolled
//! partial rig. That matters twice here:
//!   * `clone.stroke` / `heal.stroke` / `selection.set` live in
//!     appname-tools while `layer.add` / `adjustment.add` / `image.flatten`
//!     live in appname-core — only the production composition carries both;
//!   * the `ignoreAdjustments` legs use REAL §17.2 adjustment ops
//!     (`appname_adjust::register` is a chain row), not a test-local probe
//!     op, so the adjustment-layer arms are exercised as shipped.

use appname_conformance::assembly_point;
use appname_core::{decode_png_to_buffer, export_png_bytes, Engine, IVec2, ProfileId};
use appname_tools::brush::{encode_samples, InputSample};
use serde_json::{json, Value};

const RED_ID: &str = "5a3b0000-0000-4000-8000-0000000000a1";
const TOP_ID: &str = "5a3b0000-0000-4000-8000-0000000000a2";
const ADJ_LAYER: &str = "5a3b0000-0000-4000-8000-0000000000a3";
const ADJ_OP: &str = "5a3b0000-0000-4000-8000-0000000000a4";

const W: i32 = 200;
const H: i32 = 120;

fn cmd(e: &mut Engine, n: u32, kind: &str, target: Option<Value>, params: Value) {
    e.apply_json(&envelope(n, kind, target, params, None))
        .unwrap_or_else(|err| panic!("{kind} -> {:?} {}", err.code, err.message));
}

fn envelope(
    n: u32,
    kind: &str,
    target: Option<Value>,
    params: Value,
    samples: Option<String>,
) -> String {
    let mut v = json!({
        "schema": "appname.command.v1",
        "id": format!("5a3b0000-0000-4000-8000-{n:012x}"),
        "kind": kind,
        "params": params,
    });
    if let Some(t) = target {
        v["target"] = t;
    }
    if let Some(s) = samples {
        v["samples"] = json!(s);
    }
    v.to_string()
}

/// A short horizontal stroke across the middle of the RIGHT half.
fn samples_blob() -> Vec<u8> {
    let pts = [
        (130.0, 60.0, 0.0),
        (150.0, 60.0, 0.025),
        (170.0, 60.0, 0.05),
    ];
    let samples: Vec<InputSample> = pts
        .iter()
        .map(|(x, y, t)| InputSample {
            x: *x,
            y: *y,
            pressure: 1.0,
            tilt_x: 0.0,
            tilt_y: 0.0,
            rotation: 0.0,
            velocity: 0.0,
            t: *t,
        })
        .collect();
    encode_samples(&samples).expect("fixed samples encode")
}

/// The exported composite, decoded back to pixels. THE door under test.
fn exported(e: &Engine) -> (appname_core::Buffer, u32, u32) {
    let png = export_png_bytes(e).expect("the composite/export door must serve");
    let (buf, w, h) = decode_png_to_buffer(&png, ProfileId::nil()).expect("decode");
    (buf, w, h)
}

/// How many pixels of the EXPORTED composite read red, in `x >= x_min`.
fn red_count_from(e: &Engine, x_min: i32) -> usize {
    let (buf, w, h) = exported(e);
    let mut n = 0usize;
    for y in 0..h as i32 {
        for x in x_min..w as i32 {
            let p = buf.read_pixel_f32(IVec2::new(x, y));
            if p[0] > 0.9 && p[1] < 0.1 && p[2] < 0.1 && p[3] > 0.9 {
                n += 1;
            }
        }
    }
    n
}

/// White doc · an opaque RED block over the LEFT half (`x < 100`) on its
/// own layer · an empty raster layer on top (the retouch target).
///
/// The red lives on a layer that is NOT the target, so a clone that
/// deposits red into the right half can only have read it through the
/// COMPOSITE — the target layer alone is empty and the "current" scope
/// would have nothing to copy.
fn layered_doc() -> (Engine, String) {
    let mut e = assembly_point::assembled_engine();
    cmd(
        &mut e,
        1,
        "doc.new",
        None,
        json!({"width": W, "height": H, "background": "white"}),
    );
    cmd(
        &mut e,
        2,
        "layer.add",
        None,
        json!({"kind": "raster", "newLayerId": RED_ID}),
    );
    cmd(
        &mut e,
        3,
        "selection.set",
        None,
        json!({"source": "rect", "rect": {"x": 0, "y": 0, "w": 100, "h": H},
               "op": "replace", "antialias": false}),
    );
    cmd(
        &mut e,
        4,
        "fill",
        Some(json!({"layerId": RED_ID})),
        json!({"content": "color",
               "color": {"model": "rgb", "comps": [1.0, 0.0, 0.0, 0.0]},
               "antialias": false}),
    );
    cmd(&mut e, 5, "selection.none", None, json!({}));
    cmd(
        &mut e,
        6,
        "layer.add",
        None,
        json!({"kind": "raster", "newLayerId": TOP_ID}),
    );
    let samples = e.add_samples_asset(samples_blob()).to_string();
    (e, samples)
}

/// Clone params pulling the source from 60 px to the LEFT — i.e. from the
/// red half — into the right half of the target.
fn clone_params(source: Value, extra: Value) -> Value {
    let mut p = json!({
        "brushPreset": assembly_point::preset_hard_round_16_id().to_string(),
        "source": source,
        "offset": [-60.0, 0.0],
        "sampleAllLayers": true,
        "sizePx": 40.0,
        "seed": 5,
    });
    for (k, v) in extra.as_object().unwrap() {
        p[k.as_str()] = v.clone();
    }
    p
}

/// ★ THE COMPOSITE-DOOR PROOF for `source:"composite"` + `sampleAllLayers:
/// true`. Command-door acceptance is asserted, and then the deposit is
/// read back OUT OF THE EXPORTED PNG — so a row that served the command
/// and walled the export could not pass this test.
#[test]
fn composite_sampled_clone_reaches_the_export_door() {
    let (mut e, samples) = layered_doc();

    // Control: before the clone, the right half carries NO red at all.
    assert_eq!(
        red_count_from(&e, 110),
        0,
        "fixture control: the right half must start free of red, or the \
         post-clone assertion proves nothing"
    );
    let before = e.state_hash();

    let params = clone_params(json!("composite"), json!({}));
    e.apply_json(&envelope(
        10,
        "clone.stroke",
        Some(json!({"layerId": TOP_ID})),
        params,
        Some(samples.clone()),
    ))
    .expect("command door: §45.2 `all` serves");
    assert_ne!(before, e.state_hash(), "the clone must actually deposit");

    // THE DOOR UNDER TEST: the deposit must survive to the reference
    // export, not merely to the command receipt.
    //
    // The floor is DERIVED from the fixture geometry, not fitted to an
    // observed run. Dab centers run along y = 60 from x = 130 to x = 170
    // at sizePx 40 (radius 20), so the swept footprint covers
    // x ∈ [110, 190] × y ∈ [40, 80]. `offset` is [-60, 0], so a
    // destination pixel at x samples x − 60, and the RED block spans
    // x ∈ [0, 100) opaque — hence every destination pixel with x < 160
    // draws from red. Intersecting that with the counted half-plane
    // (x ≥ 110) and with the footprint's full-height span (x ≥ 130)
    // leaves a strictly-inside rectangle of 30 × 40 = 1200 px, BEFORE
    // adding the left cap and before any discount for brush falloff at
    // the capsule edge. 200 is that floor cut by ~6× so the check tests
    // "the deposit reached the export", never the pixel-exact falloff —
    // which the stroke goldens own, not this door. (Observed on this
    // build: 1796.)
    let n = red_count_from(&e, 110);
    assert!(
        n > 200,
        "the composite-sampled deposit must appear in the EXPORTED composite; \
         found {n} red px at x >= 110 (command door said Ok — this is exactly \
         the accept-then-wall shape the composite-door rule exists to catch)"
    );
}

/// The same proof for the §13.3 wand: the selection is invisible to the
/// export by itself, so it is made visible — fill THROUGH the wand's
/// selection and read the fill back out of the exported composite. If the
/// wand had sampled the empty target layer instead of the composite, the
/// selection would cover the whole canvas and the green would not stop at
/// the red/white seam.
#[test]
fn composite_sampled_wand_selection_gates_a_fill_at_the_export_door() {
    let (mut e, _samples) = layered_doc();
    cmd(
        &mut e,
        20,
        "selection.set",
        None,
        json!({"source": "wand", "seed": [150, 60], "sampleAllLayers": true,
               "antialias": false}),
    );
    cmd(
        &mut e,
        21,
        "fill",
        Some(json!({"layerId": TOP_ID})),
        json!({"content": "color",
               "color": {"model": "rgb", "comps": [0.0, 1.0, 0.0, 0.0]},
               "antialias": false}),
    );

    let (buf, _, _) = exported(&e);
    let green = |x: i32, y: i32| {
        let p = buf.read_pixel_f32(IVec2::new(x, y));
        p[1] > 0.9 && p[0] < 0.1 && p[3] > 0.9
    };
    assert!(
        green(150, 60),
        "the wand's own (white) region must be filled in the EXPORT"
    );
    assert!(
        !green(50, 60),
        "the RED half must be excluded — the wand read the composite, where \
         that half is red, not the empty target layer where it is nothing"
    );
}

/// §45.2 `ignoreAdjustments`, exercised against REAL §17.2 adjustment ops
/// from the production chain — and carried through to the export door.
///
/// Leg 1: an adjustment layer above the target puts a parametric layer in
/// the sampled scope; the storage-space sampling composite cannot evaluate
/// it, so the command REFUSES typed rather than sampling a composite that
/// silently lacks the adjustment.
///
/// Leg 2: with the toggle on, the adjustment is disabled for the sample —
/// the command serves AND the export serves, with the deposit visible.
/// (Leg 2 is also the accept-then-wall guard: the document still CONTAINS
/// the adjustment layer at export time, so the export door has to handle
/// it even though the sampler skipped it.)
#[test]
fn ignore_adjustments_serves_through_the_export_door() {
    // Baseline: adding the adjustment must not, by itself, wall the export
    // — otherwise leg 2's export assertion would be testing the wrong
    // thing.
    let (mut base, _) = layered_doc();
    cmd(
        &mut base,
        29,
        "adjustment.add",
        Some(json!({"layerId": TOP_ID})),
        json!({"opKind": "adjustment.invert", "newLayerId": ADJ_LAYER,
               "newOpId": ADJ_OP, "params": {}}),
    );
    export_png_bytes(&base).expect(
        "control: an inverted-adjustment document must export on its own, or \
         the leg-2 export assertion below is not about my change",
    );

    // Leg 1 — toggle OFF: typed refusal, nothing mutated.
    let (mut e, samples) = layered_doc();
    cmd(
        &mut e,
        30,
        "adjustment.add",
        Some(json!({"layerId": TOP_ID})),
        json!({"opKind": "adjustment.invert", "newLayerId": ADJ_LAYER,
               "newOpId": ADJ_OP, "params": {}}),
    );
    let pre = e.state_hash();
    let params = clone_params(json!("composite"), json!({}));
    let err = e
        .apply_json(&envelope(
            31,
            "clone.stroke",
            Some(json!({"layerId": TOP_ID})),
            params,
            Some(samples.clone()),
        ))
        .expect_err("an adjustment layer in the sampled scope must refuse typed");
    assert_eq!(err.code.as_str(), "error.command.not_implemented");
    assert!(
        err.message.contains("adjustment layer"),
        "the refusal must name its row; got: {}",
        err.message
    );
    assert_eq!(pre, e.state_hash(), "a refused command mutates nothing");

    // Leg 2 — toggle ON: serves, and the deposit reaches the export.
    let params = clone_params(json!("composite"), json!({"ignoreAdjustments": true}));
    e.apply_json(&envelope(
        32,
        "clone.stroke",
        Some(json!({"layerId": TOP_ID})),
        params,
        Some(samples.clone()),
    ))
    .expect("§45.2: ignoreAdjustments disables adjustments above the target");
    assert_ne!(pre, e.state_hash(), "the clone must deposit");
    export_png_bytes(&e).expect(
        "the composite/export door must serve a document whose clone sampled \
         past an adjustment layer that is still present in the tree",
    );
}

/// The §45.2 tri-state's INVALID combination, asserted where it matters:
/// it is a typed refusal at the COMMAND door and it leaves the EXPORTED
/// composite byte-identical. ES §40.5's `clone.stroke` entry:
/// "'composite'+false -> error.schema.out_of_range (inexpressible scope)".
///
/// Both spellings of false are covered — the explicit one and the
/// schema-defaulted absence, which is what a real client actually sends.
#[test]
fn the_invalid_tri_state_combination_refuses_and_moves_no_exported_pixel() {
    for (n, spelling) in [
        (40u32, json!({"sampleAllLayers": false})),
        (42, json!({})), // defaulted absence
    ] {
        let (mut e, samples) = layered_doc();
        let png_before = export_png_bytes(&e).expect("export serves");
        let mut params = clone_params(json!("composite"), spelling.clone());
        if spelling.as_object().unwrap().is_empty() {
            params.as_object_mut().unwrap().remove("sampleAllLayers");
        }
        let err = e
            .apply_json(&envelope(
                n,
                "clone.stroke",
                Some(json!({"layerId": TOP_ID})),
                params,
                Some(samples.clone()),
            ))
            .expect_err("composite + sampleAllLayers:false has no §45.2 row");
        assert_eq!(
            err.code.as_str(),
            "error.schema.out_of_range",
            "spelling {spelling}: the invalid combination is out_of_range, never \
             an acceptance and never a not_implemented"
        );
        assert_eq!(
            png_before,
            export_png_bytes(&e).expect("export still serves"),
            "spelling {spelling}: a refused command must move no exported pixel"
        );

        // GREEN control on the same fixture: flipping ONLY the flag serves
        // and DOES move exported pixels.
        let params = clone_params(json!("composite"), json!({}));
        e.apply_json(&envelope(
            n + 1,
            "clone.stroke",
            Some(json!({"layerId": TOP_ID})),
            params,
            Some(samples.clone()),
        ))
        .expect("the `all` row serves");
        assert_ne!(
            png_before,
            export_png_bytes(&e).expect("export serves"),
            "spelling {spelling}: the control must actually change the export, \
             else the refusal assertion above is vacuous"
        );
    }
}
