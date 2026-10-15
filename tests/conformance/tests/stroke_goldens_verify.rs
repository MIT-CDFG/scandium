// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Stroke-golden verification — ACTIVE regression gate (CP-0.3 slice B).
//!
//! Spec: contracts brush.stroke.yaml / eraser.stroke.yaml; ES §15/§40.5;
//! plan §0.6 rule 6; goldens/README.md rules. Every `cargo test`
//! re-produces all four stroke goldens through the ASSEMBLED engine
//! (which re-proves the undo case's byte identities at production time)
//! and byte-compares the committed files; the runner samples fixtures are
//! pinned to their generator too.

use appname_conformance::paths;
use appname_conformance::stroke_goldens::{fixed_samples_basic, produce_all};
use appname_core::ErrorCode;

fn stroke_dir() -> std::path::PathBuf {
    paths::repo_root().join("goldens").join("stroke")
}

#[test]
fn stroke_goldens_match_fresh_assembled_production() {
    let produced = produce_all(); // re-proves the undo/redo evidence too
    assert_eq!(produced.len(), 4);
    for g in &produced {
        let pkg = std::fs::read(stroke_dir().join(format!("{}.appname", g.name)))
            .unwrap_or_else(|e| panic!("{}: package golden missing: {e}", g.name));
        assert_eq!(
            g.package, pkg,
            "{}: committed package golden != fresh assembled production — \
             route the change through O-QA review",
            g.name
        );
        let png = std::fs::read(stroke_dir().join(format!("{}.png", g.name)))
            .unwrap_or_else(|e| panic!("{}: export golden missing: {e}", g.name));
        assert_eq!(g.export, png, "{}: export golden drifted", g.name);
    }
}

#[test]
fn stroke_manifest_passes_shasum() {
    let out = appname_conformance::shasum256()
        .args(["-c", "MANIFEST.sha256"])
        .current_dir(stroke_dir())
        .output()
        .expect("shasum available");
    assert!(
        out.status.success(),
        "goldens/stroke/MANIFEST.sha256 check failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn runner_samples_fixtures_are_generator_pinned() {
    let dir = paths::repo_root().join("fixtures").join("samples");
    let committed =
        std::fs::read(dir.join("basic-5pt.samplesv1")).expect("runner samples fixture committed");
    assert_eq!(
        committed,
        fixed_samples_basic(),
        "fixtures/samples/basic-5pt.samplesv1 drifted from its generator"
    );
    let template = std::fs::read_to_string(dir.join("stroke-script.template.jsonl"))
        .expect("stroke script template committed");
    assert!(
        template.contains("<LAYER_ID>") && template.contains("brush.stroke"),
        "stroke script template lost its shape"
    );
    // the baked asset id must equal the content-derived id of the blob
    let mut engine = appname_conformance::assembly_point::assembled_engine();
    let id = engine.add_samples_asset(fixed_samples_basic());
    assert!(
        template.contains(&id.to_string()),
        "template asset id != content-derived id of basic-5pt.samplesv1"
    );
}

/// The recorded lock decision (lane log): no public surface sets §11.1
/// locks until layer.set_props, so there is no lock GOLDEN — the wire id
/// the stage map names for lock refusals is pinned here instead, so the
/// taxonomy cannot drift silently before the golden becomes producible.
#[test]
fn lock_wire_id_is_pinned() {
    assert_eq!(ErrorCode::Locked.as_str(), "error.command.locked");
    assert_eq!(
        ErrorCode::EraseToHistoryUnsupported.as_str(),
        "error.unsupported.erase_to_history"
    );
}
