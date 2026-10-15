// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Deterministic corpus-seed emitter for the CP-1.0 fuzz targets (house
//! pattern: committed seeds from REAL fixtures, emitted mechanically —
//! the sibling of `w07_psd_migration seeds` for the PSD corpus).
//!
//! Every seed is produced by the production emitters themselves, never
//! hand-typed wire text:
//!
//! * `fuzz_vector_sidecar/` — real `VectorPath`s built from
//!   `appname_vector::primitives`, serialized by the strict writer
//!   `path_to_sidecar_json` (§7.5 sidecar spelling).
//! * `fuzz_action_v1/` — one action RECORDED from a live engine journal
//!   (`Recorder`, fixed envelope ids → byte-stable) and one constructed
//!   `Action` exercising the stop/parameterized/assets branches,
//!   serialized by `Action::to_json`.
//! * `fuzz_pack_manifest/` — the staged registry `packs/packs.json`
//!   verbatim (the real O-ML fixture) plus a single-pack reduction of it
//!   re-serialized after a real `PackRegistry::parse`.
//!
//! Usage: `cargo run --release --bin seed_gen` (from `fuzz/`). Idempotent
//! and deterministic; re-run any time to regenerate the committed seeds.

use std::fs;
use std::path::{Path, PathBuf};

use appname_core::engine::assets::{asset_id_for, AssetKind};
use appname_core::{EngineAssembly, PathId};
use appname_script::{Action, ActionAsset, CommandStep, RecordedTarget, Recorder, Step, StopStep};
use appname_vector::path::{FillRule, SubPath, VectorPath};
use appname_vector::{path_to_sidecar_json, primitives};
use libraw_sys::stub::{stub_bytes, with_prefix, StubSpec};

fn corpus(target: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("corpus")
        .join(target);
    fs::create_dir_all(&dir).expect("corpus dir");
    dir
}

fn write(dir: &Path, name: &str, bytes: &[u8]) {
    let p = dir.join(name);
    fs::write(&p, bytes).expect("seed writes");
    println!("wrote {} ({} bytes)", p.display(), bytes.len());
}

fn vector_path(rule: FillRule, subs: Vec<SubPath>) -> VectorPath {
    let mut p = VectorPath::new(PathId::nil(), rule);
    p.subpaths = subs;
    p
}

fn vector_seeds() {
    let dir = corpus("fuzz_vector_sidecar");

    let rect = vector_path(
        FillRule::NonZero,
        vec![primitives::rectangle(2.0, 3.0, 40.0, 24.0, [0.0; 4]).expect("rect subpath")],
    );
    write(
        &dir,
        "rect.json",
        &path_to_sidecar_json(&rect).expect("rect emits"),
    );

    let rounded = vector_path(
        FillRule::NonZero,
        vec![
            primitives::rectangle(0.5, 0.5, 31.0, 17.0, [4.0, 4.0, 4.0, 4.0])
                .expect("rounded-rect subpath"),
        ],
    );
    write(
        &dir,
        "rounded-rect.json",
        &path_to_sidecar_json(&rounded).expect("rounded emits"),
    );

    let multi = vector_path(
        FillRule::EvenOdd,
        vec![
            primitives::star(16.0, 16.0, 12.0, 5, 50.0, 0.0).expect("star subpath"),
            primitives::ellipse(16.0, 16.0, 6.0, 4.0).expect("ellipse subpath"),
        ],
    );
    write(
        &dir,
        "star-ellipse-evenodd.json",
        &path_to_sidecar_json(&multi).expect("multi emits"),
    );

    let line = vector_path(
        FillRule::NonZero,
        primitives::line(
            appname_vector::Vec2::new(1.0, 1.0),
            appname_vector::Vec2::new(30.0, 9.0),
            2.0,
            None,
            None,
        ),
    );
    write(
        &dir,
        "line-weighted.json",
        &path_to_sidecar_json(&line).expect("line emits"),
    );
}

fn action_seeds() {
    let dir = corpus("fuzz_action_v1");

    // Seed 1: a REAL recording — live engine, journal tap, fixed envelope
    // ids so regeneration is byte-stable.
    let mut engine = appname_core::engine::kinds::register(EngineAssembly::new()).build();
    let apply = |engine: &mut appname_core::Engine, env: serde_json::Value| {
        engine
            .apply_json(&env.to_string())
            .expect("seed envelope applies");
    };
    apply(
        &mut engine,
        serde_json::json!({
            "schema": "appname.command.v1",
            "id": "0f7a5e11-2bce-4d10-9b6f-3a4d5e6f7090",
            "kind": "doc.new",
            "params": {"width": 32, "height": 24, "background": "white"},
        }),
    );
    apply(
        &mut engine,
        serde_json::json!({
            "schema": "appname.command.v1",
            "id": "0f7a5e11-2bce-4d10-9b6f-3a4d5e6f7091",
            "kind": "layer.add",
            "params": {"kind": "raster", "name": "Retouch",
                        "newLayerId": "0f7a5e11-2bce-4d10-9b6f-3a4d5e6f7092"},
        }),
    );
    let recorder = Recorder::begin(&engine, "Tint Retouch");
    apply(
        &mut engine,
        serde_json::json!({
            "schema": "appname.command.v1",
            "id": "0f7a5e11-2bce-4d10-9b6f-3a4d5e6f7093",
            "kind": "layer.set_props",
            "target": {"layerId": "0f7a5e11-2bce-4d10-9b6f-3a4d5e6f7092"},
            "params": {"opacity": 0.5},
        }),
    );
    let mut recorded = recorder.finish(&engine).expect("recording finishes");
    // The recorder mints a fresh v4 action id; pin it so regeneration is
    // byte-stable (ids are identity, not captured values — same posture
    // as replay's fresh command ids).
    recorded.id =
        Some(uuid::Uuid::parse_str("0f7a5e11-2bce-4d10-9b6f-3a4d5e6f7096").expect("uuid"));
    write(
        &dir,
        "recorded-set-props.json",
        recorded.to_json().as_bytes(),
    );

    // Seed 2: the branches a plain recording never hits — stop step,
    // parameterized pointer, recordedTarget, a content-addressed asset —
    // built as the typed model and serialized by the production emitter.
    let asset_bytes = b"appname-fuzz-seed-sample-blob-v1".to_vec();
    let constructed = Action {
        id: Some(uuid::Uuid::parse_str("0f7a5e11-2bce-4d10-9b6f-3a4d5e6f7094").expect("uuid")),
        name: "Fuzz seed: stops, pointers, assets".to_string(),
        steps: vec![
            Step::Command(CommandStep {
                command: serde_json::json!({
                    "schema": "appname.command.v1",
                    "id": "0f7a5e11-2bce-4d10-9b6f-3a4d5e6f7095",
                    "kind": "layer.set_props",
                    "target": {"layerId": "0f7a5e11-2bce-4d10-9b6f-3a4d5e6f7092"},
                    "params": {"opacity": 0.25},
                }),
                enabled: true,
                parameterized: vec!["/params/opacity".to_string()],
                recorded_target: Some(RecordedTarget {
                    layer_name: Some("Retouch".to_string()),
                    tree_index: Some(0),
                }),
            }),
            Step::Stop(StopStep {
                message: "Seed stop: check the tint before continuing.".to_string(),
                allow_continue: true,
                enabled: false,
            }),
        ],
        assets: vec![ActionAsset {
            id: asset_id_for(&asset_bytes),
            kind: AssetKind::Samples,
            bytes: asset_bytes,
        }],
    };
    write(
        &dir,
        "stop-parameterized-asset.json",
        constructed.to_json().as_bytes(),
    );
}

fn manifest_seeds() {
    let dir = corpus("fuzz_pack_manifest");
    let staged = Path::new(env!("CARGO_MANIFEST_DIR")).join("../packs/packs.json");
    let text = fs::read_to_string(&staged).expect("packs/packs.json reads");
    // Verbatim copy of the real staged registry (must parse — seed
    // emission doubles as a fixture check).
    let reg = appname_ml::manifest::PackRegistry::parse(&text).expect("staged registry parses");
    write(&dir, "packs-registry.json", text.as_bytes());
    // Single-pack reduction, re-serialized mechanically.
    let mut v: serde_json::Value = serde_json::from_str(&text).expect("registry json");
    let first = v["packs"]
        .as_array()
        .and_then(|a| a.first())
        .expect("registry has packs")
        .clone();
    v["packs"] = serde_json::Value::Array(vec![first]);
    let minimal = serde_json::to_string_pretty(&v).expect("minimal serializes");
    appname_ml::manifest::PackRegistry::parse(&minimal).expect("minimal registry parses");
    write(&dir, "single-pack.json", minimal.as_bytes());
    drop(reg);
}

fn raw_probe_seeds() {
    let dir = corpus("fuzz_raw_probe");

    // The floor's pinned seed set (appname-io/tests/raw_fuzz_floor.rs,
    // the O-IO handoff's "seeds = the floor's pinned corpus builders"):
    // three §43.7 truncated_*-shaped vendor-magic heads + two stub-spec
    // payloads emitted by the PRODUCTION stub emitters
    // (`libraw_sys::stub` — mechanical, never hand-typed wire bytes;
    // the magic heads are the floor's pinned literals carried verbatim).
    let cr2 = {
        let mut v = vec![0x49, 0x49, 0x2A, 0x00, 0x10, 0x00, 0x00, 0x00, 0x43, 0x52];
        v.resize(24, 0x11);
        v
    };
    let raf = b"FUJIFILMCCD-RAW 0201FF129502".to_vec();
    let cr3 = {
        let mut v = vec![0, 0, 0, 0x18];
        v.extend_from_slice(b"ftypcrx ");
        v.resize(32, 0);
        v
    };
    write(&dir, "cr2-head.bin", &cr2);
    write(&dir, "raf-head.bin", &raf);
    write(&dir, "cr3-head.bin", &cr3);
    // The floor composes with_prefix(&cr2, &stub_bytes(..)) — mirror it
    // exactly (magic head first, stub payload after).
    write(
        &dir,
        "stub-ok.bin",
        &with_prefix(&cr2, &stub_bytes(&StubSpec::bayer_rggb(16, 12))),
    );
    write(
        &dir,
        "stub-degenerate-maximum0.bin",
        &with_prefix(
            &cr2,
            &stub_bytes(&StubSpec {
                color_maximum: 0,
                ..StubSpec::bayer_rggb(8, 8)
            }),
        ),
    );
}

fn main() {
    vector_seeds();
    action_seeds();
    manifest_seeds();
    raw_probe_seeds();
    println!("seed_gen: done (deterministic; re-run any time)");
}
