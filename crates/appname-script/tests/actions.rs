// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! End-to-end actions/macros suite: record (journal tap) → action file
//! (contract shape) → replay (rule-3 rewriting, stops, parameterization)
//! → batch (§26.2 report + template + on-error policy).
//!
//! Spec: ES §26.1/§26.2; contracts/schemas/action.v1.json; W20
//! groundwork (plan §2.4).

use serde_json::{json, Value};
use uuid::Uuid;

use appname_core::engine::kind_impl::EngineAssembly;
use appname_core::Engine;
use appname_script::batch::{run_batch, BatchJob, OnError, PngExporter};
use appname_script::{
    replay, Action, EngineDoor, Recorder, ReplayOptions, ReplayOutcome, Step, StepStatus,
};

// The SHIPPED assembly composition — THE production constructor (core
// door slice 2, item D; O-INT MAJOR-1 recorded the drift class this
// retires: this file was a hand-synced COPY of the bin's chain). A copy
// can no longer drift — there is nothing left to copy.
fn full_assembly() -> EngineAssembly {
    appname_assembly::production_assembly()
}

fn apply(engine: &mut Engine, kind: &str, target: Option<Value>, params: Value) -> Value {
    let mut env = json!({
        "schema": "appname.command.v1",
        "id": Uuid::new_v4().to_string(),
        "kind": kind,
        "params": params,
    });
    if let Some(t) = target {
        env["target"] = t;
    }
    let receipt = engine
        .apply_json(&env.to_string())
        .unwrap_or_else(|e| panic!("{kind} applies: {e:?}"));
    serde_json::to_value(&receipt).expect("receipt serializes")
}

fn new_doc(w: u32, h: u32) -> Engine {
    let mut engine = full_assembly().build();
    apply(
        &mut engine,
        "doc.new",
        None,
        json!({ "width": w, "height": h, "background": "white" }),
    );
    engine
}

fn layer_named(engine: &Engine, name: &str) -> Option<Uuid> {
    let doc = engine.document()?;
    doc.z_order_walk()
        .iter()
        .find(|id| doc.layer(**id).map(|l| l.name() == name).unwrap_or(false))
        .map(|id| id.as_uuid())
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("appname-script-suite-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Record an edit against a PRE-EXISTING layer, replay it onto a
/// DIFFERENT document whose same-named layer has a different id —
/// rule-3 name-first resolution.
#[test]
fn record_then_replay_rewrites_targets_by_name() {
    let mut rec_engine = new_doc(32, 24);
    apply(
        &mut rec_engine,
        "layer.add",
        None,
        json!({ "kind": "raster", "name": "Retouch",
                "newLayerId": Uuid::new_v4().to_string() }),
    );
    let retouch = layer_named(&rec_engine, "Retouch").expect("added layer");

    let recorder = Recorder::begin(&rec_engine, "Tint Retouch");
    apply(
        &mut rec_engine,
        "layer.set_props",
        Some(json!({ "layerId": retouch.to_string() })),
        json!({ "opacity": 0.5 }),
    );
    let action = recorder.finish(&rec_engine).expect("recording finishes");
    assert_eq!(
        action.steps.len(),
        1,
        "journal delta = exactly the recorded edit"
    );
    // Rule-3 capture rode along.
    let Step::Command(set_props) = &action.steps[0] else {
        panic!("step 0 is a command")
    };
    let rt = set_props.recorded_target.as_ref().expect("captured target");
    assert_eq!(rt.layer_name.as_deref(), Some("Retouch"));
    assert!(rt.tree_index.is_some());

    // The wire form round-trips.
    let action = Action::from_json(&action.to_json()).expect("action re-parses");

    // Fresh document: same layer NAME, necessarily different id.
    let mut replay_engine = new_doc(32, 24);
    apply(
        &mut replay_engine,
        "layer.add",
        None,
        json!({ "kind": "raster", "name": "Retouch",
                "newLayerId": Uuid::new_v4().to_string() }),
    );
    let by_name = layer_named(&replay_engine, "Retouch").expect("layer exists");
    assert_ne!(by_name, retouch, "ids differ across documents");

    let report = replay(
        &action,
        &mut EngineDoor(&mut replay_engine),
        &ReplayOptions::default(),
    );
    assert!(
        matches!(report.outcome, ReplayOutcome::Completed),
        "{:?}",
        report.to_value()
    );
    assert!(report.steps.iter().all(|s| s.status == StepStatus::Ok));
    let doc = replay_engine.document().expect("doc");
    let l = doc
        .layer(appname_core::graph::id::LayerId::from_uuid(by_name))
        .expect("resolved layer");
    assert!(
        (l.opacity() - 0.5).abs() < 1e-6,
        "opacity landed on the NAME-resolved layer (stale id rewritten)"
    );
}

/// Intra-action created-id coherence: an action that ADDS a layer and
/// then edits it re-runs cleanly anywhere — replay mints fresh `new*Id`s
/// and remaps the later reference (no duplicate-object collision, no
/// accidental rule-3 detour onto a pre-existing layer).
#[test]
fn created_ids_remint_and_remap_across_steps() {
    let mut rec_engine = new_doc(16, 16);
    let recorder = Recorder::begin(&rec_engine, "add+tune");
    apply(
        &mut rec_engine,
        "layer.add",
        None,
        json!({ "kind": "raster", "name": "Wash",
                "newLayerId": Uuid::new_v4().to_string() }),
    );
    let wash = layer_named(&rec_engine, "Wash").expect("added");
    apply(
        &mut rec_engine,
        "layer.set_props",
        Some(json!({ "layerId": wash.to_string() })),
        json!({ "opacity": 0.7 }),
    );
    let action = recorder.finish(&rec_engine).expect("finishes");

    // Re-run on the SAME document: the recorded newLayerId exists here —
    // replay must mint a fresh one and steer the set_props to it.
    let report = replay(
        &action,
        &mut EngineDoor(&mut rec_engine),
        &ReplayOptions::default(),
    );
    assert!(
        matches!(report.outcome, ReplayOutcome::Completed),
        "{:?}",
        report.to_value()
    );
    let doc = rec_engine.document().expect("doc");
    let washes: Vec<(Uuid, f32)> = doc
        .z_order_walk()
        .iter()
        .filter_map(|id| doc.layer(*id).map(|l| (id.as_uuid(), l.opacity())))
        .filter(|_| true)
        .collect();
    let wash_layers: Vec<&(Uuid, f32)> = washes
        .iter()
        .filter(|(id, _)| {
            doc.layer(appname_core::graph::id::LayerId::from_uuid(*id))
                .map(|l| l.name() == "Wash")
                .unwrap_or(false)
        })
        .collect();
    assert_eq!(wash_layers.len(), 2, "recorded add + replayed add");
    let replayed = wash_layers
        .iter()
        .find(|(id, _)| *id != wash)
        .expect("fresh-minted layer id");
    assert!(
        (replayed.1 - 0.7).abs() < 1e-6,
        "the remapped set_props landed on the REPLAYED layer: {wash_layers:?}"
    );
}

#[test]
fn replay_semantics_stops_disabled_parameterized() {
    let action = Action::from_json(
        &json!({
            "schema": "appname.action.v1",
            "name": "semantics",
            "steps": [
                { "command": { "kind": "layer.add",
                               "params": { "kind": "raster", "name": "A", "newLayerId": Uuid::new_v4().to_string() } } },
                { "command": { "kind": "layer.add",
                               "params": { "kind": "raster", "name": "Disabled", "newLayerId": Uuid::new_v4().to_string() } },
                  "enabled": false },
                { "stop": { "message": "verbatim author text", "allowContinue": true } },
                { "command": { "kind": "layer.add",
                               "params": { "kind": "raster", "name": "Param", "newLayerId": Uuid::new_v4().to_string() } },
                  "parameterized": ["/params/name"] },
                { "stop": { "message": "hard stop", "allowContinue": false } },
                { "command": { "kind": "layer.add",
                               "params": { "kind": "raster", "name": "Never", "newLayerId": Uuid::new_v4().to_string() } } }
            ]
        })
        .to_string(),
    )
    .expect("parses");

    let mut engine = new_doc(16, 16);
    let mut vars = serde_json::Map::new();
    vars.insert("/params/name".to_string(), json!("Substituted"));
    let report = replay(
        &action,
        &mut EngineDoor(&mut engine),
        &ReplayOptions {
            vars,
            active_layer: None,
        },
    );

    // Halted at the allowContinue:false stop; the trailing step never ran.
    assert!(matches!(report.outcome, ReplayOutcome::Halted { at: 4 }));
    let statuses: Vec<_> = report.steps.iter().map(|s| s.status.clone()).collect();
    assert_eq!(
        statuses,
        vec![
            StepStatus::Ok,
            StepStatus::Skipped,
            StepStatus::Stop,
            StepStatus::Ok,
            StepStatus::Stop
        ]
    );
    assert_eq!(
        report.steps[2].message.as_deref(),
        Some("verbatim author text"),
        "stop text is author content, shown verbatim"
    );
    // The parameterized step used the batch variable, not the recorded value.
    assert!(layer_named(&engine, "Substituted").is_some());
    assert!(layer_named(&engine, "Param").is_none());
    assert!(
        layer_named(&engine, "Disabled").is_none(),
        "disabled = skipped"
    );
    assert!(
        layer_named(&engine, "Never").is_none(),
        "post-halt step unattempted"
    );

    // Wire report shape.
    let v = report.to_value();
    assert_eq!(v["status"], "halted");
    assert_eq!(v["haltedAt"], 4);
    assert_eq!(v["steps"][3]["kind"], "layer.add");
}

#[test]
fn unresolved_target_is_a_step_error_with_the_w20_detail() {
    // A set_props against a layer that does not exist in the replay doc,
    // recorded under a name the doc also lacks — the W20 "Grain" case.
    let action = Action::from_json(
        &json!({
            "schema": "appname.action.v1",
            "name": "grain",
            "steps": [
                { "command": { "kind": "layer.set_props",
                               "target": { "layerId": Uuid::new_v4().to_string() },
                               "params": { "opacity": 0.4 } },
                  "recordedTarget": { "layerName": "Grain", "treeIndex": 30 } }
            ]
        })
        .to_string(),
    )
    .expect("parses");

    let mut engine = new_doc(8, 8);
    let report = replay(
        &action,
        &mut EngineDoor(&mut engine),
        &ReplayOptions::default(),
    );
    let ReplayOutcome::Failed { at, error } = &report.outcome else {
        panic!("unresolvable target must fail: {:?}", report.to_value())
    };
    assert_eq!(*at, 0);
    assert!(
        error
            .message
            .contains("\"Grain\" not found by name or index"),
        "detail names the unresolved target: {}",
        error.message
    );
    let v = report.to_value();
    assert_eq!(v["error"]["messageId"], "error.batch.step_failed");
    assert_eq!(v["steps"][0]["status"], "error");
}

#[test]
fn null_layer_target_needs_an_active_layer() {
    let action = Action::from_json(
        &json!({
            "schema": "appname.action.v1",
            "name": "active",
            "steps": [
                { "command": { "kind": "layer.set_props",
                               "target": { "layerId": null },
                               "params": { "opacity": 0.25 } } }
            ]
        })
        .to_string(),
    )
    .expect("parses");

    // Headless with no active layer: typed step error.
    let mut engine = new_doc(8, 8);
    let report = replay(
        &action,
        &mut EngineDoor(&mut engine),
        &ReplayOptions::default(),
    );
    let ReplayOutcome::Failed { error, .. } = &report.outcome else {
        panic!("null target without an active layer must fail")
    };
    assert!(error.message.contains("active layer"), "{}", error.message);

    // With one: resolves and applies.
    let mut engine = new_doc(8, 8);
    let bg = layer_named(&engine, "Background").expect("background layer");
    let report = replay(
        &action,
        &mut EngineDoor(&mut engine),
        &ReplayOptions {
            vars: serde_json::Map::new(),
            active_layer: Some(bg),
        },
    );
    assert!(matches!(report.outcome, ReplayOutcome::Completed));
    let doc = engine.document().unwrap();
    let l = doc
        .layer(appname_core::graph::id::LayerId::from_uuid(bg))
        .unwrap();
    assert!((l.opacity() - 0.25).abs() < 1e-6);
}

#[test]
fn replay_is_rerunnable_on_the_recording_document() {
    // Fresh command ids at replay: the journal's duplicate-id reject must
    // not fire when an action replays on the doc that recorded it.
    let mut engine = new_doc(16, 16);
    let recorder = Recorder::begin(&engine, "again");
    apply(
        &mut engine,
        "layer.add",
        None,
        json!({ "kind": "raster", "name": "Twice",
                "newLayerId": Uuid::new_v4().to_string() }),
    );
    let action = recorder.finish(&engine).expect("finishes");
    let report = replay(
        &action,
        &mut EngineDoor(&mut engine),
        &ReplayOptions::default(),
    );
    assert!(
        matches!(report.outcome, ReplayOutcome::Completed),
        "{:?}",
        report.to_value()
    );
    let doc = engine.document().unwrap();
    let twice = doc
        .z_order_walk()
        .iter()
        .filter_map(|id| doc.layer(*id))
        .filter(|l| l.name() == "Twice")
        .count();
    assert_eq!(twice, 2, "recorded once + replayed once");
}

#[test]
fn batch_runs_the_action_over_png_inputs_with_report() {
    let dir = scratch("batch");
    // Two small PNG inputs from the engine's own reference exporter.
    for (name, w, h) in [("a", 20, 10), ("b", 12, 16)] {
        let engine = new_doc(w, h);
        let bytes = appname_core::io::png_io::export_png_bytes(&engine).expect("png");
        std::fs::write(dir.join(format!("{name}.png")), bytes).expect("write input");
    }

    let action = Action::from_json(
        &json!({
            "schema": "appname.action.v1",
            "id": Uuid::new_v4().to_string(),
            "name": "Batch fill",
            "steps": [
                { "command": { "kind": "layer.add",
                               "params": { "kind": "raster", "name": "Wash", "newLayerId": Uuid::new_v4().to_string() } } }
            ]
        })
        .to_string(),
    )
    .expect("parses");

    let out_dir = dir.join("out");
    let job = BatchJob {
        action: &action,
        inputs: vec![dir.join("a.png"), dir.join("b.png")],
        output_template: format!(
            "{}/{{name}}-{{date:yyyy-MM-dd}}-{{seq:2}}.{{ext}}",
            out_dir.display()
        ),
        format: "png".to_string(),
        on_error: OnError::Continue,
        overwrite: false,
        replay: ReplayOptions::default(),
        started: (2026, 8, 13, 9, 0, 0),
        clock: Some(|| (2026, 8, 13, 9, 0, 42)),
    };
    let report = run_batch(&job, &full_assembly(), &mut PngExporter).expect("batch runs");
    assert!(report.all_ok(), "{}", report.to_value());

    // Outputs per the template.
    for f in ["a-2026-08-13-01.png", "b-2026-08-13-02.png"] {
        let p = out_dir.join(f);
        let bytes = std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        assert!(
            bytes.starts_with(&[0x89, b'P', b'N', b'G']),
            "PNG magic in {f}"
        );
    }
    // batch-report.json next to the outputs, batchreport.v1 shape.
    let rep: Value = serde_json::from_str(
        &std::fs::read_to_string(out_dir.join("batch-report.json")).expect("report written"),
    )
    .expect("report is JSON");
    assert_eq!(rep["schema"], "appname.batchreport.v1");
    assert_eq!(rep["action"], "Batch fill");
    assert_eq!(rep["started"], "2026-08-13T09:00:00Z");
    // MINOR-4: `finished` is the injected clock's real reading, never a
    // restatement of `started`.
    assert_eq!(rep["finished"], "2026-08-13T09:00:42Z");
    assert_eq!(
        rep["summary"],
        json!({ "total": 2, "ok": 2, "failed": 0, "skipped": 0 })
    );
    assert_eq!(rep["results"][0]["steps"][0]["status"], "ok");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn batch_per_file_failure_continues_and_reports() {
    let dir = scratch("batch-fail");
    // One input that HAS layer "Grain" (an .appname package) and one PNG
    // that does not — the W20 resolution matrix in miniature.
    let with_grain = dir.join("with-grain.appname");
    {
        let mut engine = new_doc(10, 10);
        apply(
            &mut engine,
            "layer.add",
            None,
            json!({ "kind": "raster", "name": "Grain",
                    "newLayerId": Uuid::new_v4().to_string() }),
        );
        appname_core::io::package::save_package(&engine, &with_grain).expect("save");
    }
    let plain = dir.join("plain.png");
    {
        let engine = new_doc(10, 10);
        std::fs::write(
            &plain,
            appname_core::io::png_io::export_png_bytes(&engine).expect("png"),
        )
        .expect("write");
    }

    let action = Action::from_json(
        &json!({
            "schema": "appname.action.v1",
            "name": "grain-only",
            "steps": [
                { "command": { "kind": "layer.set_props",
                               "target": { "layerId": Uuid::new_v4().to_string() },
                               "params": { "opacity": 0.4 } },
                  "recordedTarget": { "layerName": "Grain" } }
            ]
        })
        .to_string(),
    )
    .expect("parses");

    let out_dir = dir.join("out");
    let job = BatchJob {
        action: &action,
        inputs: vec![with_grain.clone(), plain.clone()],
        output_template: format!("{}/{{name}}.{{ext}}", out_dir.display()),
        format: "png".to_string(),
        on_error: OnError::Continue,
        overwrite: false,
        replay: ReplayOptions::default(),
        started: (2026, 8, 13, 9, 0, 0),
        clock: Some(|| (2026, 8, 13, 9, 0, 42)),
    };
    let report = run_batch(&job, &full_assembly(), &mut PngExporter).expect("batch runs");
    assert!(!report.all_ok());
    let v = report.to_value();
    assert_eq!(v["summary"]["ok"], 1);
    assert_eq!(v["summary"]["failed"], 1);
    // The failing row: §26.2 shape — error.batch.step_failed + a detail
    // naming the unresolved target.
    assert_eq!(v["results"][1]["status"], "error");
    assert_eq!(v["results"][1]["output"], Value::Null);
    assert_eq!(
        v["results"][1]["error"]["messageId"],
        "error.batch.step_failed"
    );
    assert!(
        v["results"][1]["error"]["detail"]
            .as_str()
            .unwrap()
            .contains("Grain"),
        "{}",
        v["results"][1]["error"]
    );
    // The succeeding row exported.
    assert_eq!(v["results"][0]["status"], "ok");
    assert!(out_dir.join("with-grain.png").exists());

    // on-error halt: the second file is never attempted.
    let job = BatchJob {
        inputs: vec![plain.clone(), with_grain.clone()],
        on_error: OnError::Halt,
        overwrite: true,
        ..BatchJob {
            action: &action,
            inputs: vec![],
            output_template: format!("{}/{{name}}.{{ext}}", out_dir.display()),
            format: "png".to_string(),
            on_error: OnError::Continue,
            overwrite: false,
            replay: ReplayOptions::default(),
            started: (2026, 8, 13, 9, 0, 0),
            clock: Some(|| (2026, 8, 13, 9, 0, 42)),
        }
    };
    let report = run_batch(&job, &full_assembly(), &mut PngExporter).expect("batch runs");
    let v = report.to_value();
    assert_eq!(v["summary"]["failed"], 1);
    assert_eq!(v["summary"]["skipped"], 1);
    std::fs::remove_dir_all(&dir).ok();
}

/// Rule-4/asset carriage: a recorded `edit.paste` travels with its raster
/// asset and replays onto a document that never saw the blob.
#[test]
fn recorded_assets_travel_and_reingest_content_addressed() {
    let png_bytes = {
        let engine = new_doc(6, 6);
        appname_core::io::png_io::export_png_bytes(&engine).expect("png")
    };

    let mut rec = new_doc(24, 24);
    let recorder = Recorder::begin(&rec, "paste");
    let asset_id = rec.add_raster_asset(png_bytes.clone()).expect("ingest");
    apply(
        &mut rec,
        "edit.paste",
        None,
        json!({ "assetId": asset_id.to_string(), "at": [2, 2],
                "newLayerId": Uuid::new_v4().to_string() }),
    );
    let action = recorder.finish(&rec).expect("finishes");
    assert_eq!(action.assets.len(), 1, "the referenced blob was harvested");
    assert_eq!(action.assets[0].id, asset_id);

    // Round-trip through the wire form (base64 carriage).
    let action = Action::from_json(&action.to_json()).expect("re-parses");

    let mut fresh = new_doc(24, 24);
    assert!(fresh.assets().is_empty(), "fresh doc has no assets");
    let report = replay(
        &action,
        &mut EngineDoor(&mut fresh),
        &ReplayOptions::default(),
    );
    assert!(
        matches!(report.outcome, ReplayOutcome::Completed),
        "{:?}",
        report.to_value()
    );
    assert!(
        fresh.assets().contains(asset_id),
        "content-addressed re-entry"
    );
    let doc = fresh.document().unwrap();
    assert!(
        doc.z_order_walk()
            .iter()
            .filter_map(|id| doc.layer(*id))
            .any(|l| l.name() == "Pasted"),
        "the paste replayed"
    );
}

/// MAJOR-1 regression: the reviewer's exact composed-traversal breach.
/// An input FILE named `..png` (`Path::file_stem` = `.`) + template
/// `comp/{name}./esc.{ext}` used to resolve `comp/../esc.png` — written
/// OUTSIDE `comp/` with exit 0 and an "ok" report row. The §35.2
/// containment pass must keep every output under the template's literal
/// directory (or fail the row typed) — never an outside write.
#[test]
fn batch_composed_traversal_breach_stays_contained() {
    let dir = scratch("batch-breach");
    let hostile = dir.join("..png"); // stem "."
    {
        let engine = new_doc(8, 8);
        std::fs::write(
            &hostile,
            appname_core::io::png_io::export_png_bytes(&engine).expect("png"),
        )
        .expect("write hostile-named input");
    }
    let action = Action::from_json(
        &json!({ "schema": "appname.action.v1", "name": "noop", "steps": [] }).to_string(),
    )
    .expect("parses");

    let comp = dir.join("comp");
    let job = BatchJob {
        action: &action,
        inputs: vec![hostile.clone()],
        output_template: format!("{}/{{name}}./esc.{{ext}}", comp.display()),
        format: "png".to_string(),
        on_error: OnError::Continue,
        overwrite: false,
        replay: ReplayOptions::default(),
        started: (2026, 8, 13, 9, 0, 0),
        clock: Some(|| (2026, 8, 13, 9, 0, 42)),
    };
    let report = run_batch(&job, &full_assembly(), &mut PngExporter).expect("batch runs");

    // The breach artifact: dir/esc.png (i.e. comp/../esc.png) must not
    // exist under ANY outcome.
    assert!(
        !dir.join("esc.png").exists(),
        "output escaped the template's literal directory"
    );
    // The ruled containment: the `..`-composing component rewrites to
    // `_`, the row succeeds INSIDE comp/.
    let row = &report.results[0];
    assert!(row.ok(), "{}", report.to_value());
    let out = row.output.as_ref().expect("contained output path");
    assert!(
        out.starts_with(&comp),
        "output {} must stay under {}",
        out.display(),
        comp.display()
    );
    assert_eq!(out, &comp.join("_").join("esc.png"));
    assert!(out.exists(), "the contained output was written");
    assert!(
        !out.components()
            .any(|c| c == std::path::Component::ParentDir),
        "no `..` component may survive: {}",
        out.display()
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// MINOR-6 (O-INT interim ruling): NON-step failure rows — open/export/
/// unsupported-input — are DETAIL-ONLY: typed `code` + `detail`, no
/// `messageId`. `error.batch.step_failed` names STEP failures only; the
/// proper second id is an O-I18N fold-in candidate, not minted here.
#[test]
fn non_step_failure_rows_are_detail_only() {
    let dir = scratch("batch-detail-only");
    let bad = dir.join("not-an-image.txt");
    std::fs::write(&bad, b"nope").expect("write");
    let action = Action::from_json(
        &json!({ "schema": "appname.action.v1", "name": "noop", "steps": [] }).to_string(),
    )
    .expect("parses");

    let job = BatchJob {
        action: &action,
        inputs: vec![bad.clone()],
        output_template: format!("{}/out/{{name}}.{{ext}}", dir.display()),
        format: "png".to_string(),
        on_error: OnError::Continue,
        overwrite: false,
        replay: ReplayOptions::default(),
        started: (2026, 8, 13, 9, 0, 0),
        clock: Some(|| (2026, 8, 13, 9, 0, 42)),
    };
    let report = run_batch(&job, &full_assembly(), &mut PngExporter).expect("batch runs");
    assert!(!report.results[0].step_failure());
    let v = report.to_value();
    assert_eq!(v["results"][0]["status"], "error");
    let err = v["results"][0]["error"].as_object().expect("error object");
    assert_eq!(err["code"], "error.api.invalid_arg");
    assert!(
        err["detail"]
            .as_str()
            .unwrap()
            .contains("unsupported batch input"),
        "{err:?}"
    );
    assert!(
        !err.contains_key("messageId"),
        "non-step rows carry no messageId (detail-only, interim ruling): {err:?}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// O-INT consolidated fix round, MAJOR-1 + MAJOR-2 red-to-green pin: an
/// ADJUSTMENT-CARRYING action records over the wrapped assembly, replays
/// through `run_batch`, and exports a multi-raster + adjustment document
/// — byte-equal to the C ABI export door driven over the SAME input with
/// the SAME action.
///
/// Pre-fix failure modes this test pins away, in order:
/// * MAJOR-1 alone unfixed: the `adjustment.add` step refuses
///   `error.schema.unknown_op_kind` at replay stage 3 (the batch stack's
///   assembly lacked the adjust/filters/vector wraps cli/cabi carry);
/// * MAJOR-1 fixed but MAJOR-2 unfixed: the replay serves and the row
///   then fails at EXPORT — the PngExporter general arm carried a BARE
///   op registry, so the ≥2-raster + adjustment composite refused typed.
///
/// Only the full class fix makes the row "ok" with bytes on disk.
#[test]
fn batch_adjustment_action_exports_byte_equal_to_the_abi_door() {
    use appname_cabi::actions::appname_doc_apply_action;
    use appname_cabi::doc::{appname_doc_close, appname_doc_open};
    use appname_cabi::export::appname_doc_export;
    use appname_cabi::AppnameStatus;

    let dir = scratch("batch-adjustment-parity");
    // Input package: a 16x8 white document.
    let input_path = dir.join("in.appname");
    appname_core::io::package::save_package(&new_doc(16, 8), &input_path).expect("save input");

    // RECORD over the wrapped assembly (MAJOR-1's other face: pre-fix,
    // the recording engine itself refused adjustment.add at stage 3).
    let mut rec = new_doc(16, 8);
    let recorder = Recorder::begin(&rec, "wash and invert");
    apply(
        &mut rec,
        "layer.add",
        None,
        json!({ "kind": "raster", "name": "Wash",
                "newLayerId": Uuid::new_v4().to_string() }),
    );
    let wash = layer_named(&rec, "Wash").expect("added layer");
    apply(
        &mut rec,
        "fill",
        Some(json!({ "layerId": wash.to_string() })),
        json!({ "content": "color",
                "color": { "model": "rgb", "comps": [0.0, 0.0, 1.0, 0.0] } }),
    );
    apply(
        &mut rec,
        "adjustment.add",
        None,
        json!({ "opKind": "adjustment.invert",
                "newLayerId": Uuid::new_v4().to_string(),
                "newOpId": Uuid::new_v4().to_string(),
                "params": {} }),
    );
    let action = recorder.finish(&rec).expect("recording finishes");
    assert_eq!(action.steps.len(), 3, "add + fill + adjustment recorded");
    let action_json = action.to_json();
    let action = Action::from_json(&action_json).expect("wire round-trip");

    // REPLAY + EXPORT through the batch stack: 2 rasters + adjustment
    // forces the exporter's general (compositor) arm.
    let out_dir = dir.join("out");
    let job = BatchJob {
        action: &action,
        inputs: vec![input_path.clone()],
        output_template: format!("{}/{{name}}.{{ext}}", out_dir.display()),
        format: "png".to_string(),
        on_error: OnError::Halt,
        overwrite: false,
        replay: ReplayOptions::default(),
        started: (2026, 8, 16, 9, 0, 0),
        clock: Some(|| (2026, 8, 16, 9, 0, 42)),
    };
    let report = run_batch(&job, &full_assembly(), &mut PngExporter).expect("batch runs");
    assert!(
        report.all_ok(),
        "adjustment-carrying batch row serves: {}",
        report.to_value()
    );
    let batch_png = std::fs::read(out_dir.join("in.png")).expect("batch output exists");

    // CROSS-DOOR TRUTH: the same input + the same action through the C
    // ABI (open -> apply_action -> export), byte-compared. Both doors
    // composite through composite_document_with over the engine's §12.1
    // op registry and encode through core's §6.4 PNG encoder — assembly
    // parity (batch == cli == cabi) makes the bytes EQUAL, asserted from
    // outside the implementation.
    let mut doc: *mut appname_cabi::doc::AppnameDoc = std::ptr::null_mut();
    let cpath = std::ffi::CString::new(input_path.to_str().unwrap()).unwrap();
    // SAFETY (test): valid NUL-terminated path; writable out slot.
    let st = unsafe { appname_doc_open(cpath.as_ptr(), &mut doc, std::ptr::null_mut()) };
    assert_eq!(st, AppnameStatus::AppnameOk, "ABI opens the same input");
    let bytes = action_json.as_bytes();
    // SAFETY (test): live handle; bytes cover len; NULL options/report.
    let st = unsafe {
        appname_doc_apply_action(
            doc,
            bytes.as_ptr(),
            bytes.len(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(st, AppnameStatus::AppnameOk, "ABI replays the same action");
    let abi_out = dir.join("abi.png");
    let cout = std::ffi::CString::new(abi_out.to_str().unwrap()).unwrap();
    let cfmt = std::ffi::CString::new("png").unwrap();
    // SAFETY (test): live handle; valid NUL-terminated strings; NULL
    // options and err per the documented contract.
    let st = unsafe {
        appname_doc_export(
            doc,
            cout.as_ptr(),
            cfmt.as_ptr(),
            std::ptr::null(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(st, AppnameStatus::AppnameOk, "ABI exports the same doc");
    // SAFETY (test): live handle, closed exactly once.
    unsafe { appname_doc_close(doc) };
    let abi_png = std::fs::read(&abi_out).expect("ABI output exists");
    assert_eq!(
        batch_png, abi_png,
        "batch export bytes == ABI export bytes (one compositing truth, one encoder)"
    );
    // And the pixels are the inverted wash, not the raw one (the
    // adjustment really evaluated): blue [0,0,1] inverts to yellow.
    let (decoded, w, h) =
        appname_core::decode_png_to_buffer(&batch_png, appname_core::ProfileId::nil())
            .expect("batch png decodes");
    assert_eq!((w, h), (16, 8));
    assert_eq!(
        decoded.read_pixel_f32(appname_core::IVec2::new(5, 4)),
        [1.0, 1.0, 0.0, 1.0],
        "inverted blue wash == yellow (adjustment served)"
    );
}

// ---------------------------------------------------------------------
// COMPOSITE-LEGS SLICE — O-INT MAJOR-1 pin: the batch PNG exporter's
// fast-path guard. A ≤1-raster doc carrying a VISIBLE parametric shape
// with NON-DEFAULT compositing props (multiply, opacity < 1) must NOT hit
// core's §4.4-source-over-only `export_png_bytes` (which refuses typed) —
// it routes through render (which owns the §4.5 kernels) and SERVES,
// byte-identical to the render path cli/cabi take. Without the
// `!has_visible_parametric_content` guard the export would REFUSE here
// while cli/cabi serve — the exact serve↔refuse split Option A collapses.
// ---------------------------------------------------------------------
#[test]
fn batch_export_serves_one_raster_nondefault_blend_parametric_and_pins_to_render() {
    use appname_script::batch::{Exporter, PngExporter};

    let mut engine = new_doc(64, 48); // one white raster
    let shape = Uuid::new_v4();
    apply(
        &mut engine,
        "vector.shape_add",
        None,
        json!({
            "path": {
                "fillRule": "nonzero",
                "subpaths": [{"closed": true, "segments": [
                    {"p0": [8.0, 8.0],  "c1": [16.0, 8.0],  "c2": [24.0, 8.0],  "p1": [32.0, 8.0]},
                    {"p0": [32.0, 8.0], "c1": [32.0, 13.0], "c2": [32.0, 19.0], "p1": [32.0, 24.0]},
                    {"p0": [32.0, 24.0],"c1": [24.0, 24.0], "c2": [16.0, 24.0], "p1": [8.0, 24.0]},
                    {"p0": [8.0, 24.0], "c1": [8.0, 19.0],  "c2": [8.0, 13.0],  "p1": [8.0, 8.0]},
                ]}],
            },
            "fill": {"type": "solid", "color": {"model": "rgba", "comps": [0.0, 0.0, 0.0, 1.0]}},
            "stroke": {"widthPx": 0.0},
            "newLayerId": shape.to_string(),
        }),
    );
    // NON-DEFAULT compositing props on the shape layer — core's §4.4-only
    // reference arm cannot represent these; render can.
    apply(
        &mut engine,
        "layer.set_props",
        Some(json!({ "layerId": shape.to_string() })),
        json!({ "blend": "multiply", "opacity": 0.5 }),
    );

    let doc = engine.document().unwrap();
    // The fast-path escape conditions: parametric present AND ≤1 raster
    // (only the white background bears pixels) — exactly the cell the guard
    // must send to render.
    assert!(
        appname_core::has_visible_parametric_content(doc),
        "the visible shape layer is parametric content"
    );
    let rasters = doc
        .z_order_walk()
        .iter()
        .filter_map(|id| doc.layer(*id))
        .filter(|n| n.pixels().is_some())
        .count();
    assert_eq!(rasters, 1, "one raster (background) — the fast-path domain");

    // (a) batch SERVES (the guard routes it to render, not the refusing core
    // arm): the PngExporter writes real bytes.
    let dir = scratch("legs-batch-pin");
    let out = dir.join("batch-parametric.png");
    PngExporter
        .export(&engine, &out, "png")
        .expect("batch export SERVES the non-default-blend parametric shape (guard → render)");
    let batch_bytes = std::fs::read(&out).expect("batch wrote a PNG");
    assert!(!batch_bytes.is_empty());

    // (b) BYTE-EQUALITY to the render path (the same call cli/cabi make):
    // batch export == encode(render composite).
    let canvas = doc.geometry().canvas_rect();
    let composite =
        appname_render::composite_document_with(doc, engine.assembly(), canvas).expect("render");
    let plane = composite.region_rgba_f32(canvas);
    let render_bytes = appname_core::io::png_io::encode_composite_png(
        canvas.width() as u32,
        canvas.height() as u32,
        &plane,
    )
    .expect("encode render composite");
    assert_eq!(
        batch_bytes, render_bytes,
        "batch PNG export is byte-identical to the render path (one compositing truth at the 4th door)"
    );

    let _ = std::fs::remove_file(&out);
}
