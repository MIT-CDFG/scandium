// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: contracts `filter.apply.yaml` / `filter.smart_add.yaml`; ES
//! §16.1/§16.2 (registry route, ROI evaluation, selection masking),
//! §11.1 (locks — the section names filter.apply under PIXELS;
//! TRANSPARENCY constrains alpha writes), §40.6 (family rules), §9.6
//! (creation UUIDs), §10.4/§34.4 (undo byte-identity, replay).
//!
//! End-to-end battery for the two `filter.*` kinds through a FULLY
//! ASSEMBLED engine — THE production constructor (core door slice 2
//! MAJOR-1 ruled swap, TEST-FILE-ONLY cross-fence companion flagged for
//! O-TOOLS ratification: the old hand chain here was a STALE pre-vector
//! copy claiming cli/cabi parity). The blur pins are the kinds'
//! deterministic corpus companions (in-crate per the fence; flagged to
//! O-QA in the lane log).

use appname_core::{Engine, EngineAssembly, ErrorCode, IVec2};
use serde_json::{json, Value};

fn assembly() -> EngineAssembly {
    appname_assembly::production_assembly()
}

fn env_t(n: u32, kind: &str, layer: Option<&str>, params: Value) -> String {
    let mut v = json!({
        "schema": "appname.command.v1",
        "id": format!("f1704d00-0000-4000-8000-0000000{n:05x}"),
        "kind": kind,
        "params": params,
    });
    if let Some(l) = layer {
        v["target"] = json!({"layerId": l});
    }
    v.to_string()
}

fn doc_new(e: &mut Engine, n: u32, background: &str) -> appname_core::LayerId {
    e.apply_json(&env_t(
        n,
        "doc.new",
        None,
        json!({"width": 64, "height": 64, "background": background}),
    ))
    .unwrap();
    e.document().unwrap().z_order_walk()[0]
}

fn select_rect(e: &mut Engine, n: u32, x: i32, y: i32, w: i32, h: i32) {
    e.apply_json(&env_t(
        n,
        "selection.set",
        None,
        json!({"source": "rect", "rect": {"x": x, "y": y, "w": w, "h": h}, "antialias": false}),
    ))
    .unwrap();
}

fn fill_rgb(e: &mut Engine, n: u32, layer: appname_core::LayerId, rgb: [f64; 3]) {
    e.apply_json(&env_t(
        n,
        "fill",
        Some(&layer.to_string()),
        json!({"content": "color",
                "color": {"model": "rgb", "comps": [rgb[0], rgb[1], rgb[2], 1.0]}}),
    ))
    .unwrap();
}

fn deselect(e: &mut Engine, n: u32) {
    e.apply_json(&env_t(n, "selection.none", None, json!({})))
        .unwrap();
}

fn pixel(e: &Engine, layer: appname_core::LayerId, x: i32, y: i32) -> [f32; 4] {
    e.document()
        .unwrap()
        .layer(layer)
        .unwrap()
        .pixels()
        .unwrap()
        .read_pixel_f32(IVec2::new(x, y))
}

/// Straight encoded readback (test axis of the §16.3 blur pins).
fn encoded_at(e: &Engine, layer: appname_core::LayerId, x: i32, y: i32) -> [f32; 3] {
    let s = appname_core::io::srgb::unpremul(pixel(e, layer, x, y));
    [
        appname_core::io::srgb::srgb_encode(s[0]),
        appname_core::io::srgb::srgb_encode(s[1]),
        appname_core::io::srgb::srgb_encode(s[2]),
    ]
}

/// Two-tone doc: left half encoded-51 gray, right half white.
fn two_tone(e: &mut Engine) -> appname_core::LayerId {
    let layer = doc_new(e, 1, "white");
    select_rect(e, 2, 0, 0, 32, 64);
    fill_rgb(e, 3, layer, [0.2, 0.2, 0.2]);
    deselect(e, 4);
    layer
}

fn gaussian(radius: f64) -> Value {
    json!({"filterId": "filter.gaussian_blur", "params": {"radius": radius}})
}

#[test]
fn filter_apply_gaussian_bakes_undoes_and_replays() {
    let mut e = assembly().build();
    let layer = two_tone(&mut e);
    let before = e.state_hash();
    let receipt = e
        .apply_json(&env_t(
            5,
            "filter.apply",
            Some(&layer.to_string()),
            gaussian(12.0),
        ))
        .unwrap();
    assert!(!receipt.dirty.is_empty(), "blur wrote tiles");
    let after = e.state_hash();
    // The tone boundary softens: the midline pixel sits between the two
    // fields; far-field pixels are effectively untouched (σ = 4).
    let mid = encoded_at(&e, layer, 32, 32);
    assert!(mid[0] > 0.3 && mid[0] < 0.95, "softened boundary: {mid:?}");
    let far = encoded_at(&e, layer, 4, 32);
    assert!((far[0] - 0.2).abs() < 0.02, "far field ~stable: {far:?}");
    // §10.4 tile-delta inverse + D-48 replay.
    e.undo().unwrap();
    assert_eq!(e.state_hash(), before);
    e.redo().unwrap();
    assert_eq!(e.state_hash(), after);
    let replayed = e.replay_assembled(e.journal()).unwrap();
    assert_eq!(replayed.state_hash(), after, "replay determinism");
}

#[test]
fn filter_apply_selection_masks_and_opacity_fades() {
    let mut e = assembly().build();
    let layer = two_tone(&mut e);
    let left_before = pixel(&e, layer, 8, 32);
    // Mask to the RIGHT half: the left field must not move (§16.2).
    select_rect(&mut e, 5, 32, 0, 32, 64);
    e.apply_json(&env_t(
        6,
        "filter.apply",
        Some(&layer.to_string()),
        gaussian(12.0),
    ))
    .unwrap();
    assert_eq!(
        pixel(&e, layer, 8, 32),
        left_before,
        "outside the selection nothing changes (bit-exact)"
    );
    deselect(&mut e, 7);
    // opacity: 0 fade is the §40.1 empty push.
    let receipt = e
        .apply_json(&env_t(
            8,
            "filter.apply",
            Some(&layer.to_string()),
            json!({"filterId": "filter.gaussian_blur", "params": {"radius": 12.0},
                    "blend": "normal", "opacity": 0.0}),
        ))
        .unwrap();
    assert!(receipt.dirty.is_empty(), "opacity 0 → no delta");
}

#[test]
fn filter_apply_transparency_lock_pins_alpha() {
    // Unlocked: a blur SPREADS alpha past the filled rect.
    let mut e = assembly().build();
    let layer = doc_new(&mut e, 1, "transparent");
    select_rect(&mut e, 2, 16, 16, 32, 32);
    fill_rgb(&mut e, 3, layer, [1.0, 0.0, 0.0]);
    deselect(&mut e, 4);
    e.apply_json(&env_t(
        5,
        "filter.apply",
        Some(&layer.to_string()),
        gaussian(9.0),
    ))
    .unwrap();
    assert!(
        pixel(&e, layer, 14, 32)[3] > 0.0,
        "unlocked blur writes alpha outside the rect"
    );

    // TRANSPARENCY-locked: alpha is preserved pixel-for-pixel (§40.6
    // "constrains filters that write alpha", lane-log pin).
    let mut e = assembly().build();
    let layer = doc_new(&mut e, 1, "transparent");
    select_rect(&mut e, 2, 16, 16, 32, 32);
    fill_rgb(&mut e, 3, layer, [1.0, 0.0, 0.0]);
    deselect(&mut e, 4);
    e.apply_json(&env_t(
        5,
        "layer.set_props",
        Some(&layer.to_string()),
        json!({"locks": {"transparency": true}}),
    ))
    .unwrap();
    e.apply_json(&env_t(
        6,
        "filter.apply",
        Some(&layer.to_string()),
        gaussian(9.0),
    ))
    .unwrap();
    assert_eq!(pixel(&e, layer, 14, 32)[3], 0.0, "outside alpha stays 0");
    assert_eq!(pixel(&e, layer, 32, 32)[3], 1.0, "inside alpha stays 1");
}

#[test]
fn filter_apply_rejects_match_the_contract_taxonomy() {
    let mut e = assembly().build();
    let layer = doc_new(&mut e, 1, "white");
    let l = layer.to_string();

    // §40.6 addition: unregistered filterId.
    let err = e
        .apply_json(&env_t(
            2,
            "filter.apply",
            Some(&l),
            json!({"filterId": "filter.does_not_exist", "params": {}}),
        ))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::UnknownOpKind);

    // Stage-3 nested param re-validation (contract invalid case 3).
    let err = e
        .apply_json(&env_t(3, "filter.apply", Some(&l), gaussian(-3.0)))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::OutOfRange);

    // Missing target.
    let err = e
        .apply_json(&env_t(4, "filter.apply", None, gaussian(4.0)))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::BadTarget);

    // Parametric target → mismatch with the §12.4 rasterize-first detail.
    e.apply_json(&env_t(
        5,
        "adjustment.add",
        None,
        json!({"opKind": "adjustment.invert", "params": {},
                "newLayerId": "f1ad0000-0000-4000-8000-000000000001",
                "newOpId": "f1ad0000-0000-4000-8000-000000000002"}),
    ))
    .unwrap();
    let err = e
        .apply_json(&env_t(
            6,
            "filter.apply",
            Some("f1ad0000-0000-4000-8000-000000000001"),
            gaussian(4.0),
        ))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::TargetKindMismatch);

    // Non-normal fade blend: typed refusal naming the compositor row.
    let err = e
        .apply_json(&env_t(
            7,
            "filter.apply",
            Some(&l),
            json!({"filterId": "filter.gaussian_blur", "params": {"radius": 4.0},
                    "blend": "multiply", "opacity": 0.5}),
        ))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotImplemented);

    // §11.1: PIXELS lock blocks (the section names filter.apply).
    e.apply_json(&env_t(
        8,
        "layer.set_props",
        Some(&l),
        json!({"locks": {"pixels": true}}),
    ))
    .unwrap();
    let err = e
        .apply_json(&env_t(9, "filter.apply", Some(&l), gaussian(4.0)))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Locked);
}

#[test]
fn filter_smart_add_reject_arms_stay_green_and_mutate_nothing() {
    let mut e = assembly().build();
    let layer = doc_new(&mut e, 1, "white");
    let l = layer.to_string();

    // Convert the raster layer to a smart object (§11.5).
    e.apply_json(&env_t(
        2,
        "smartobject.convert",
        None,
        json!({"layerIds": [l],
                "newLayerId": "f15a0000-0000-4000-8000-00000000aaa1"}),
    ))
    .unwrap();
    let smart = e
        .document()
        .unwrap()
        .z_order_walk()
        .into_iter()
        .find(|lid| {
            e.document()
                .unwrap()
                .layer(*lid)
                .map(|n| n.smart_object().is_some())
                .unwrap_or(false)
        })
        .expect("converted smart object");
    let s = smart.to_string();

    // Every reject arm is state-hash-pinned: a refusal mutates NOTHING
    // (the §40.1 discipline the door-blocked round pinned on the typed
    // tail carries forward to every remaining refusal arm now that the
    // valid tail executes).
    let pin = |e: &mut Engine, n: u32, target: &str, params: Value, want: ErrorCode| {
        let before = e.state_hash();
        let err = e
            .apply_json(&env_t(n, "filter.smart_add", Some(target), params))
            .unwrap_err();
        assert_eq!(err.code, want, "arm {n}");
        assert_eq!(e.state_hash(), before, "arm {n}: refusal mutated nothing");
    };

    // Unregistered filterId (§40.6 addition).
    pin(
        &mut e,
        3,
        &s,
        json!({"filterId": "filter.does_not_exist", "params": {},
                "newOpId": "f15a0000-0000-4000-8000-000000000001"}),
        ErrorCode::UnknownOpKind,
    );

    // Nested param re-validation (§16.1 stage 3).
    pin(
        &mut e,
        4,
        &s,
        json!({"filterId": "filter.gaussian_blur", "params": {"radius": -3.0},
                "newOpId": "f15a0000-0000-4000-8000-000000000002"}),
        ErrorCode::OutOfRange,
    );

    // newOpId collision with an existing arena op (§9.6).
    e.apply_json(&env_t(
        5,
        "adjustment.add",
        None,
        json!({"opKind": "adjustment.invert", "params": {},
                "newLayerId": "f15a0000-0000-4000-8000-000000000011",
                "newOpId": "f15a0000-0000-4000-8000-000000000012"}),
    ))
    .unwrap();
    let dup = json!({"filterId": "filter.gaussian_blur", "params": {"radius": 4.0},
            "newOpId": "f15a0000-0000-4000-8000-000000000012"});
    pin(&mut e, 6, &s, dup, ErrorCode::DuplicateId);

    // Target not a SmartObject → mismatch suggesting smartobject.convert.
    let plain = doc_layer_add(&mut e, 7);
    pin(
        &mut e,
        8,
        &plain,
        json!({"filterId": "filter.gaussian_blur", "params": {"radius": 4.0},
                "newOpId": "f15a0000-0000-4000-8000-000000000021"}),
        ErrorCode::TargetKindMismatch,
    );

    // §40.6 lock table: ALL blocks on the target (post-flip the refusal
    // must fire BEFORE the door — non-destructive PIXELS stays open by
    // design, ALL does not).
    e.apply_json(&env_t(
        9,
        "layer.set_props",
        Some(&s),
        json!({"locks": {"all": true}}),
    ))
    .unwrap();
    pin(
        &mut e,
        10,
        &s,
        json!({"filterId": "filter.gaussian_blur", "params": {"radius": 4.0},
                "index": 99, "newOpId": "f15a0000-0000-4000-8000-000000000031"}),
        ErrorCode::Locked,
    );
}

#[test]
fn filter_smart_add_grows_the_chain_through_the_door_and_round_trips() {
    let mut e = assembly().build();
    let layer = doc_new(&mut e, 1, "white");
    e.apply_json(&env_t(
        2,
        "smartobject.convert",
        None,
        json!({"layerIds": [layer.to_string()],
                "newLayerId": "f15b0000-0000-4000-8000-00000000aaa1"}),
    ))
    .unwrap();
    let so = appname_core::LayerId::from_uuid(
        uuid::Uuid::parse_str("f15b0000-0000-4000-8000-00000000aaa1").unwrap(),
    );
    let s = so.to_string();
    let chain = |e: &Engine| -> Vec<String> {
        e.document()
            .unwrap()
            .layer(so)
            .unwrap()
            .smart_object()
            .unwrap()
            .smart_filters()
            .iter()
            .map(|o| o.as_uuid().to_string())
            .collect()
    };
    let op_a = "f15b0000-0000-4000-8000-000000000001";
    let op_b = "f15b0000-0000-4000-8000-000000000002";
    let op_c = "f15b0000-0000-4000-8000-000000000003";
    let pre = e.state_hash();

    // Append onto the empty chain: index 99 CLAMPS to len == 0 (§40.6 —
    // never an error) and the receipt carries the created OpId.
    let receipt = e
        .apply_json(&env_t(
            3,
            "filter.smart_add",
            Some(&s),
            json!({"filterId": "filter.gaussian_blur", "params": {"radius": 4.0},
                    "index": 99, "newOpId": op_a}),
        ))
        .unwrap();
    assert_eq!(
        receipt.created_ids,
        vec![uuid::Uuid::parse_str(op_a).unwrap()],
        "receipt carries the OpId (§9.6 client-generated)"
    );
    assert_eq!(chain(&e), vec![op_a], "clamped append landed");

    // Head splice (index 0) then a mid splice (index 1).
    e.apply_json(&env_t(
        4,
        "filter.smart_add",
        Some(&s),
        json!({"filterId": "filter.median", "params": {}, "index": 0, "newOpId": op_b}),
    ))
    .unwrap();
    assert_eq!(chain(&e), vec![op_b, op_a]);
    e.apply_json(&env_t(
        5,
        "filter.smart_add",
        Some(&s),
        json!({"filterId": "filter.box_blur", "params": {}, "index": 1, "newOpId": op_c}),
    ))
    .unwrap();
    assert_eq!(chain(&e), vec![op_b, op_c, op_a], "mid splice at index 1");
    // The arena nodes resolve and carry the registered §12.1 op ids.
    let doc = e.document().unwrap();
    for (id, op) in [
        (op_a, "filter.gaussian_blur"),
        (op_b, "filter.median"),
        (op_c, "filter.box_blur"),
    ] {
        let oid = appname_core::graph::id::OpId::from_uuid(uuid::Uuid::parse_str(id).unwrap());
        assert_eq!(doc.op(oid).unwrap().op(), op, "arena node op id");
    }
    let post = e.state_hash();

    // §10.4 exact-inverse undo/redo: byte-identity through the door.
    e.undo().unwrap();
    e.undo().unwrap();
    assert_eq!(chain(&e), vec![op_a], "two splices reverted in order");
    e.undo().unwrap();
    assert_eq!(chain(&e), Vec::<String>::new());
    assert_eq!(e.state_hash(), pre, "undo ×3 restores byte identity");
    e.redo().unwrap();
    e.redo().unwrap();
    e.redo().unwrap();
    assert_eq!(chain(&e), vec![op_b, op_c, op_a]);
    assert_eq!(e.state_hash(), post, "redo ×3 restores byte identity");

    // D-48 replay determinism through the assembled engine.
    let replayed = e.replay_assembled(e.journal()).unwrap();
    assert_eq!(replayed.state_hash(), post, "replay determinism");
}

#[test]
fn filter_smart_add_enabled_chain_renders_at_export_disabled_exports_identically() {
    // §12.1 PARAMETRIC RE-RENDER (reopened-CP-0.7 core slice — this leg
    // FLIPPED BY DESIGN from the typed-refusal tripwire, flagged for
    // O-QA ratification): a smart_add'ed ENABLED filter now EVALUATES at
    // the reference export door — blurred pixels, deterministic across
    // re-renders — and a DISABLED chain still exports BYTE-IDENTICALLY
    // to the pre-add composite (§12.4 disabled == never-applied; the
    // §11.5 proxy serves as the captured SOURCE under the chain).
    let mut e = assembly().build();
    let layer = doc_new(&mut e, 1, "white");
    e.apply_json(&env_t(
        2,
        "smartobject.convert",
        None,
        json!({"layerIds": [layer.to_string()],
                "newLayerId": "f15c0000-0000-4000-8000-00000000aaa1"}),
    ))
    .unwrap();
    let s = "f15c0000-0000-4000-8000-00000000aaa1";
    let before_bytes = appname_core::export_png_bytes(&e).expect("empty chain exports");

    let op = "f15c0000-0000-4000-8000-000000000001";
    e.apply_json(&env_t(
        3,
        "filter.smart_add",
        Some(s),
        json!({"filterId": "filter.gaussian_blur", "params": {"radius": 4.0}, "newOpId": op}),
    ))
    .unwrap();
    let blurred = appname_core::export_png_bytes(&e).expect("enabled chain SERVES (§12.1)");
    assert_ne!(
        blurred, before_bytes,
        "the enabled gaussian chain changes the export (blur visible)"
    );
    assert_eq!(
        blurred,
        appname_core::export_png_bytes(&e).unwrap(),
        "§12.1 evaluation is a pure re-render — two runs byte-identical"
    );

    e.apply_json(&env_t(
        4,
        "op.toggle",
        None,
        json!({"opId": op, "enabled": false}),
    ))
    .unwrap();
    let after_bytes = appname_core::export_png_bytes(&e).expect("disabled chain exports");
    assert_eq!(
        before_bytes, after_bytes,
        "disabled == never-applied at the composite (§12.4 bit-identity)"
    );
}

/// Add a plain raster layer, returning its id string.
fn doc_layer_add(e: &mut Engine, n: u32) -> String {
    let id = format!("f1aa0000-0000-4000-8000-0000000{n:05x}");
    e.apply_json(&env_t(
        n,
        "layer.add",
        None,
        json!({"kind": "raster", "newLayerId": id}),
    ))
    .unwrap();
    id
}

#[test]
fn single_truth_registration_through_the_full_assembly() {
    let a = assembly();
    let kinds: Vec<&str> = a.kinds().collect();
    assert!(kinds.contains(&"filter.apply") && kinds.contains(&"filter.smart_add"));
    assert!(kinds.contains(&"image.auto_tone") && kinds.contains(&"image.auto_color"));
    // §12.1 registry: 8 filters + 8 adjustments, ids sorted and stable
    // (CP-1.0 warps row: filter.liquify joined the corpus).
    let ops: Vec<&str> = a.ops().ids().collect();
    assert_eq!(ops.len(), 16, "eight filters + eight adjustments: {ops:?}");
    assert!(ops.contains(&"filter.liquify"));
    assert!(ops.contains(&"filter.gaussian_blur") && ops.contains(&"adjustment.levels"));
    // Ledger evidence (rule-9 entry): this slice adds 5 assembled kinds
    // over the pre-slice composition (printed with --nocapture).
    let pre: std::collections::BTreeSet<&str> = appname_tools::kind_impls::with_tools_fill(
        appname_core::engine::kinds::register(appname_tools::kind_impls::assembly()),
    )
    .kinds()
    .collect();
    let post: std::collections::BTreeSet<&str> = a.kinds().collect();
    let added: Vec<&&str> = post.difference(&pre).collect();
    println!(
        "assembled kinds: pre {} -> post {} (+{added:?})",
        pre.len(),
        post.len()
    );
    assert_eq!(post.len(), pre.len() + 5, "exactly the five CP-0.7 kinds");
}

// ---------------------------------------------------------------------------
// CP-0.7 wave-2 FIX ROUND (reviewer defects D1–D3).
// ---------------------------------------------------------------------------

#[test]
fn d1_median_border_pin_canvas_clamp_keeps_solid_corners_solid() {
    // Reviewer repro verbatim: full-red 16×16, filter.median at the
    // default radius (2). §16.3 clamp anchors at the DOCUMENT data
    // bounds (R1; == canvas here, content ⊆ canvas): taps beyond them
    // re-read the border pixel, so every window on a uniform canvas is
    // uniform and the corners STAY red (pre-fix: the apron materialized
    // transparent beyond the canvas and the corners eroded to
    // (0,0,0,0)).
    let mut e = assembly().build();
    // The repro geometry directly: a 16×16 canvas.
    e.apply_json(&env_t(
        1,
        "doc.new",
        None,
        json!({"width": 16, "height": 16, "background": "white"}),
    ))
    .unwrap();
    let layer = e.document().unwrap().z_order_walk()[0];
    fill_rgb(&mut e, 3, layer, [1.0, 0.0, 0.0]);
    e.apply_json(&env_t(
        4,
        "filter.apply",
        Some(&layer.to_string()),
        json!({"filterId": "filter.median", "params": {}}),
    ))
    .unwrap();
    for (x, y) in [(0, 0), (15, 0), (0, 15), (15, 15), (8, 8)] {
        assert_eq!(
            pixel(&e, layer, x, y),
            [1.0, 0.0, 0.0, 1.0],
            "uniform canvas is a median fixed point at ({x},{y})"
        );
    }
    // D1-consequential (pin-5 refinement): a Clamp-policy bake writes
    // nothing beyond the canvas — no edge-replication ring content.
    let bounds = e
        .document()
        .unwrap()
        .layer(layer)
        .unwrap()
        .pixels()
        .unwrap()
        .bounds();
    assert!(
        appname_core::IRect::new(0, 0, 16, 16).contains_rect(&bounds),
        "no beyond-canvas replication ring, got {bounds:?}"
    );
}

#[test]
fn d2_transparency_lock_keeps_dst_when_filtered_alpha_is_zero() {
    // A red square on a transparent layer, TRANSPARENCY-locked, median
    // r4: at the square's corner the 9×9 window is majority-transparent,
    // so the filtered source has α = 0 — it contributes NOTHING and dst
    // must survive UNCHANGED IN COLOR (pre-fix: unpremul(0,0,0,0) → the
    // corner painted black at the original alpha).
    let mut e = assembly().build();
    let layer = doc_new(&mut e, 1, "transparent");
    select_rect(&mut e, 2, 8, 8, 16, 16);
    fill_rgb(&mut e, 3, layer, [1.0, 0.0, 0.0]);
    deselect(&mut e, 4);
    e.apply_json(&env_t(
        5,
        "layer.set_props",
        Some(&layer.to_string()),
        json!({"locks": {"transparency": true}}),
    ))
    .unwrap();
    e.apply_json(&env_t(
        6,
        "filter.apply",
        Some(&layer.to_string()),
        json!({"filterId": "filter.median", "params": {"radius": 4}}),
    ))
    .unwrap();
    assert_eq!(
        pixel(&e, layer, 8, 8),
        [1.0, 0.0, 0.0, 1.0],
        "zero-α filtered source keeps dst — color asserted, not just alpha"
    );
    assert_eq!(
        pixel(&e, layer, 6, 6)[3],
        0.0,
        "outside the square alpha stays 0 under the lock"
    );
}

#[test]
fn d3_unknown_fade_blend_is_a_stage3_reject_not_ni() {
    // decode_blend precedent: an unknown wire name is a schema violation
    // (error.schema.out_of_range); only valid-but-unimplemented §4.5
    // modes answer not_implemented (covered by the "multiply" case in
    // the reject battery).
    let mut e = assembly().build();
    let layer = doc_new(&mut e, 1, "white");
    let err = e
        .apply_json(&env_t(
            2,
            "filter.apply",
            Some(&layer.to_string()),
            json!({"filterId": "filter.gaussian_blur", "params": {"radius": 4.0},
                    "blend": "glow", "opacity": 0.5}),
        ))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::OutOfRange, "unknown blend name");
}

#[test]
fn r1_off_canvas_content_bakes_with_a_ramp_not_a_cliff() {
    // Reviewer probe shape (R1, ES:2617): content pushed partly
    // off-canvas, blurred DESTRUCTIVELY, pulled back. The read substrate
    // is the DOCUMENT data bounds (canvas ∪ §2.5 content bounds), so the
    // off-canvas slab must blur exactly like the in-canvas part — a
    // proper alpha ramp at its content edge, not a hard 1→0 cliff, and
    // no half-applied seam (pre-R1: the write region clamped to the
    // canvas, leaving the slab unfiltered, and canvas-clamped reads
    // masked it behind edge replication).
    let mut e = assembly().build();
    // A 32×32 canvas so the pushed slab genuinely leaves it.
    e.apply_json(&env_t(
        1,
        "doc.new",
        None,
        json!({"width": 32, "height": 32, "background": "transparent"}),
    ))
    .unwrap();
    let layer = e.document().unwrap().z_order_walk()[0];
    // A 16×16 red square: content [8,24)².
    select_rect(&mut e, 2, 8, 8, 16, 16);
    fill_rgb(&mut e, 3, layer, [1.0, 0.0, 0.0]);
    deselect(&mut e, 4);
    // Push 16 px right: content [24,40)×[8,24) — 8 px hang off-canvas.
    let translate = |tx: f64| {
        json!({"matrix": [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, tx, 0.0, 1.0],
                "reference": [0.0, 0.0]})
    };
    let t = translate(16.0);
    e.apply_json(&env_t(5, "transform.free", Some(&layer.to_string()), t))
        .unwrap();
    // Destructive gaussian, radius 3 (σ = 1).
    e.apply_json(&env_t(
        6,
        "filter.apply",
        Some(&layer.to_string()),
        gaussian(3.0),
    ))
    .unwrap();
    // Pull back.
    let t = translate(-16.0);
    e.apply_json(&env_t(7, "transform.free", Some(&layer.to_string()), t))
        .unwrap();
    // Column x = 20 came from x = 36 during the blur — OFF-canvas.
    // Column x = 12 stayed in-canvas throughout. Their top-edge ramps
    // must MATCH: the filter applied uniformly across the data bounds.
    let off = |y: i32| pixel(&e, layer, 20, y)[3];
    let on = |y: i32| pixel(&e, layer, 12, y)[3];
    // A ramp, not a cliff: the edge rows are strictly intermediate.
    assert!(
        off(7) > 0.02 && off(7) < 0.98 && off(8) > 0.02 && off(8) < 0.98,
        "off-canvas slab got a blur ramp (pre-R1: α(7)=0, α(8)=1 cliff), got {} / {}",
        off(7),
        off(8)
    );
    assert!(off(6) < off(7) && off(7) < off(9), "monotone ramp");
    for y in [6, 7, 8, 9, 10] {
        assert!(
            (off(y) - on(y)).abs() < 1e-3,
            "no half-applied seam: off-canvas column blurs identically \
             to the in-canvas column at y={y}: {} vs {}",
            off(y),
            on(y)
        );
    }
}

#[test]
fn filter_apply_under_pixels_locked_group_refuses_locked_transitively() {
    // §11.1 transitive letter through `Document::effective_locks` (the
    // O-CORE door-slice consolidation, adopted this round): a PIXELS
    // lock on an ancestor GROUP blocks filter.apply on the child —
    // own-node reads under-enforced this until the adoption.
    let mut e = assembly().build();
    let layer = doc_new(&mut e, 1, "white");
    let l = layer.to_string();
    e.apply_json(&env_t(
        2,
        "layer.group",
        None,
        json!({"layerIds": [l],
                "newGroupId": "f1704d00-0000-4000-8000-0000000000f1"}),
    ))
    .unwrap();
    e.apply_json(&env_t(
        3,
        "layer.set_props",
        Some("f1704d00-0000-4000-8000-0000000000f1"),
        json!({"locks": {"pixels": true}}),
    ))
    .unwrap();
    let before = e.state_hash();
    let err = e
        .apply_json(&env_t(4, "filter.apply", Some(&l), gaussian(2.0)))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Locked, "group PIXELS binds the child");
    assert_eq!(before, e.state_hash(), "refusal mutates nothing (§10.2)");
}

// ---------------------------------------------------------------------------
// CP-1.0 warps row: filter.liquify through the wire (§45.4 commit record).
// ---------------------------------------------------------------------------

/// A uniform (+8, 0) backward-map field over the 64×64 doc (5×5 nodes at
/// pitch 16): `out(p) = src(p + 8)`.
fn liquify_shift8() -> Value {
    json!({
        "filterId": "filter.liquify",
        "params": {
            "meshPitch": 16,
            "origin": [0, 0],
            "size": [5, 5],
            "disp": (0..25).flat_map(|_| [8.0, 0.0]).collect::<Vec<f64>>(),
        },
    })
}

#[test]
fn liquify_commit_bakes_undoes_and_replays() {
    // §45.4: the raster commit is ONE filter.apply{filter.liquify}
    // envelope; §45.8 commit-equals-recorded-params replay.
    let mut e = assembly().build();
    let layer = two_tone(&mut e);
    let before = e.state_hash();
    let receipt = e
        .apply_json(&env_t(
            5,
            "filter.apply",
            Some(&layer.to_string()),
            liquify_shift8(),
        ))
        .unwrap();
    assert!(!receipt.dirty.is_empty(), "liquify wrote tiles");
    let after = e.state_hash();
    // Backward map (+8, 0): the tone boundary at x = 32 moves LEFT 8 px —
    // x = 26 now reads the white field, x = 20 stays gray.
    let shifted = encoded_at(&e, layer, 26, 32);
    assert!(shifted[0] > 0.95, "boundary moved left: {shifted:?}");
    let gray = encoded_at(&e, layer, 20, 32);
    assert!((gray[0] - 0.2).abs() < 0.02, "left field stable: {gray:?}");
    // §10.4 tile-delta inverse + D-48 replay determinism.
    e.undo().unwrap();
    assert_eq!(e.state_hash(), before, "undo byte-identity");
    e.redo().unwrap();
    assert_eq!(e.state_hash(), after);
    let replayed = e.replay_assembled(e.journal()).unwrap();
    assert_eq!(replayed.state_hash(), after, "replay determinism");
}

#[test]
fn liquify_identity_field_is_the_empty_delta_push() {
    // §45.8 prop_liquify_identity through the wire: an all-zero field
    // commits bit-identically ⇒ no tile changes ⇒ §40.1 empty push.
    let mut e = assembly().build();
    let layer = two_tone(&mut e);
    let before = e.state_hash();
    let receipt = e
        .apply_json(&env_t(
            5,
            "filter.apply",
            Some(&layer.to_string()),
            json!({
                "filterId": "filter.liquify",
                "params": {
                    "origin": [0, 0],
                    "size": [5, 5],
                    "disp": vec![0.0; 50],
                },
            }),
        ))
        .unwrap();
    assert!(receipt.dirty.is_empty(), "identity field → empty delta");
    assert_eq!(e.state_hash(), before);
}

#[test]
fn liquify_wire_rejects_are_typed_and_mutation_free() {
    let mut e = assembly().build();
    let layer = two_tone(&mut e);
    let before = e.state_hash();
    let journal_len = e.journal().len();
    // disp length mismatch (§45.4 cross-field).
    let err = e
        .apply_json(&env_t(
            10,
            "filter.apply",
            Some(&layer.to_string()),
            json!({"filterId": "filter.liquify",
                    "params": {"origin": [0,0], "size": [5,5], "disp": [0.0, 0.0]}}),
        ))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::OutOfRange);
    // meshPitch outside {4, 8, 16, 32}.
    let err = e
        .apply_json(&env_t(
            11,
            "filter.apply",
            Some(&layer.to_string()),
            json!({"filterId": "filter.liquify",
                    "params": {"meshPitch": 12, "origin": [0,0], "size": [1,1], "disp": [0.0, 0.0]}}),
        ))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::OutOfRange);
    // Required field payload keys missing.
    let err = e
        .apply_json(&env_t(
            12,
            "filter.apply",
            Some(&layer.to_string()),
            json!({"filterId": "filter.liquify", "params": {}}),
        ))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::MissingField);
    assert_eq!(e.state_hash(), before, "rejects leave the doc untouched");
    assert_eq!(e.journal().len(), journal_len, "rejects journal nothing");
}

#[test]
fn liquify_smart_add_lands_as_a_re_editable_smart_filter() {
    // §45.4: on a smart-object target the same params land via
    // filter.smart_add (§12.2) — validation resolves through the same
    // registry entry.
    let mut e = assembly().build();
    let layer = two_tone(&mut e);
    e.apply_json(&env_t(
        5,
        "smartobject.convert",
        None,
        json!({"layerIds": [layer.to_string()],
                "newLayerId": "f15a0000-0000-4000-8000-00000000bbb1"}),
    ))
    .unwrap();
    let so_layer = e
        .document()
        .unwrap()
        .z_order_walk()
        .into_iter()
        .find(|lid| {
            e.document()
                .unwrap()
                .layer(*lid)
                .map(|n| n.smart_object().is_some())
                .unwrap_or(false)
        })
        .expect("converted smart object");
    let op_id = "a11ce000-0000-4000-8000-000000000001";
    let mut params = liquify_shift8();
    params["newOpId"] = json!(op_id);
    e.apply_json(&env_t(
        6,
        "filter.smart_add",
        Some(&so_layer.to_string()),
        params,
    ))
    .unwrap();
    let doc = e.document().unwrap();
    let chain = doc.layer(so_layer).unwrap().smart_object().unwrap();
    assert_eq!(chain.smart_filters().len(), 1, "one re-editable entry");
}

// ---------------------------------------------------------------------------
// Strictness ruling 2026-08-17 (O-UI probe filing @ 48eacd8): the wire
// probes that proved the drift, now typed end-to-end.
// ---------------------------------------------------------------------------

#[test]
fn unknown_filter_params_refuse_typed_on_the_wire() {
    let mut e = assembly().build();
    let layer = two_tone(&mut e);
    let before = e.state_hash();
    // THE probe: {bogus: 1} previously no-oped with ok=true.
    let err = e
        .apply_json(&env_t(
            5,
            "filter.apply",
            Some(&layer.to_string()),
            json!({"filterId": "filter.gaussian_blur", "params": {"bogus": 1}}),
        ))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::OutOfRange);
    assert!(err.message.contains("bogus"), "{}", err.message);
    // The probe's companion stays its own typed reject.
    let err = e
        .apply_json(&env_t(
            6,
            "filter.apply",
            Some(&layer.to_string()),
            json!({"filterId": "filter.gaussian_blur", "params": {"radius": 2000}}),
        ))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::OutOfRange);
    // Typo'd TYPE on a known key: same silent-no-op class, same refusal.
    let err = e
        .apply_json(&env_t(
            7,
            "filter.apply",
            Some(&layer.to_string()),
            json!({"filterId": "filter.add_noise", "params": {"monochromatic": "yes"}}),
        ))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::OutOfRange);
    // filter.smart_add rides the same resolve seam.
    let err = e
        .apply_json(&env_t(
            8,
            "filter.smart_add",
            Some(&layer.to_string()),
            json!({"filterId": "filter.gaussian_blur", "params": {"bogus": 1},
                    "newOpId": "a11ce000-0000-4000-8000-00000000c001"}),
        ))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::OutOfRange);
    assert_eq!(e.state_hash(), before, "every refusal mutation-free");
}

/// A-18 (B-06 / Q-D24, ruled ENGINE PROJECTION by the operator 2026-09-05):
/// the history row must name WHICH filter ran. Before this, every one of
/// the EIGHT filters registered in `slice1_registry` (§16.1) produced a row
/// bearing only `kind: "filter.apply"`, so the shell had nothing to speak
/// but the generic `history.filter.apply` — "Apply Filter" — and eight
/// distinct commands projected onto one label.
///
/// THE LOAD-BEARING ASSERTION IS DISCRIMINATION, NOT PRESENCE. A field that
/// exists but reads identically for gaussian and box blur would leave the
/// defect exactly where it was while turning this test green, so the pins
/// below compare the two filters against EACH OTHER as well as against
/// their expected ids.
#[test]
fn history_rows_name_which_filter_ran_and_discriminate_between_filters() {
    let mut e = assembly().build();
    let layer = two_tone(&mut e);
    e.apply_json(&env_t(
        5,
        "filter.apply",
        Some(&layer.to_string()),
        gaussian(12.0),
    ))
    .unwrap();
    e.apply_json(&env_t(
        6,
        "filter.apply",
        Some(&layer.to_string()),
        json!({"filterId": "filter.box_blur", "params": {"radius": 4}}),
    ))
    .unwrap();

    let rows = e.journal_rows();
    let filters: Vec<_> = rows.iter().filter(|r| r.kind == "filter.apply").collect();
    assert_eq!(filters.len(), 2, "two filter rows in the journal");

    // The kind STILL collapses them -- that is the projection -- and the
    // subject is what pulls them apart again.
    assert_eq!(
        filters[0].kind, filters[1].kind,
        "precondition: one kind covers both filters"
    );
    assert_eq!(
        filters[0].filter_id.as_deref(),
        Some("filter.gaussian_blur")
    );
    assert_eq!(filters[1].filter_id.as_deref(), Some("filter.box_blur"));
    assert_ne!(
        filters[0].filter_id, filters[1].filter_id,
        "two different filters must not project onto one label"
    );

    // CONTROL: kinds that need no subject carry none, so the field is not
    // being populated indiscriminately. The `any` guards a vacuous loop.
    assert!(
        rows.iter().any(|r| r.kind != "filter.apply"),
        "control set is non-empty"
    );
    for r in rows.iter().filter(|r| r.kind != "filter.apply") {
        assert!(r.filter_id.is_none(), "{} must carry no filterId", r.kind);
    }

    // `Engine::history` is the OTHER row builder (§10.4 in-process rows) and
    // must agree with the journal-derived one -- a fix applied to one site
    // and not the other is the shape this fleet keeps filing.
    let hist: Vec<_> = e
        .history()
        .into_iter()
        .filter(|r| r.kind == "filter.apply")
        .collect();
    assert_eq!(hist.len(), 2, "both row builders see both filter rows");
    assert_eq!(hist[0].filter_id.as_deref(), Some("filter.gaussian_blur"));
    assert_eq!(hist[1].filter_id.as_deref(), Some("filter.box_blur"));

    // WHAT THE SHELL ACTUALLY RECEIVES: the wire form the C ABI serializes
    // (`appname_doc_history_json` -> `journal_rows`). Asserting the struct
    // alone would not prove the shell can see it.
    let wire = serde_json::to_value(&rows).unwrap();
    let arr = wire.as_array().unwrap();
    let on_wire: Vec<_> = arr.iter().filter(|v| v["kind"] == "filter.apply").collect();
    assert_eq!(on_wire[0]["filterId"], "filter.gaussian_blur");
    assert_eq!(on_wire[1]["filterId"], "filter.box_blur");

    // ...and ABSENT rather than null for kinds without a subject, so the
    // other 101 kinds serialize exactly as they did before the field.
    let doc_new_row = arr
        .iter()
        .find(|v| v["kind"] == "doc.new")
        .expect("doc.new row present");
    assert!(
        doc_new_row.get("filterId").is_none(),
        "no filterId key at all for kinds that need no subject: {doc_new_row}"
    );
}
