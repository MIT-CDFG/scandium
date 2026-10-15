// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! fill/paste golden verification + paste-family live-state EVIDENCE —
//! ACTIVE regression gate (CP-0.5 wave 2).
//!
//! Spec: contracts fill.yaml / edit.paste.yaml / edit.paste_into.yaml;
//! O-CORE CP-0.5 wave-1 stage maps; plan §0.6 rule 6.

use appname_conformance::assembly_point;
use appname_conformance::edit_goldens::{paste_source_png, produce_all};
use appname_conformance::paths;

fn edit_dir() -> std::path::PathBuf {
    paths::repo_root().join("goldens").join("edit")
}

#[test]
fn edit_goldens_match_fresh_assembled_production() {
    let produced = produce_all(); // re-proves the paste undo/redo evidence
    assert_eq!(produced.len(), 4);
    for g in &produced {
        let pkg = std::fs::read(edit_dir().join(format!("{}.appname", g.name)))
            .unwrap_or_else(|e| panic!("{}: package golden missing: {e}", g.name));
        assert_eq!(
            g.package, pkg,
            "{}: committed package golden != fresh production — route \
             through O-QA review",
            g.name
        );
        let png = std::fs::read(edit_dir().join(format!("{}.png", g.name)))
            .unwrap_or_else(|e| panic!("{}: export golden missing: {e}", g.name));
        assert_eq!(g.export, png, "{}: export golden drifted", g.name);
    }
}

#[test]
fn edit_manifest_passes_shasum() {
    let out = appname_conformance::shasum256()
        .args(["-c", "MANIFEST.sha256"])
        .current_dir(edit_dir())
        .output()
        .expect("shasum available");
    assert!(
        out.status.success(),
        "goldens/edit/MANIFEST.sha256 check failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The recorded deferral, held honest: edit.paste_into WITHOUT a live
/// selection answers its §40.5 live-state gate
/// (`error.command.selection_required`) — the accept golden joins when
/// selection.set lands (its drift alarm is armed). Also pins that a
/// paste of an UNINGESTED asset id answers bad_target (asset-ingress
/// precondition, stage-4-analog).
#[test]
fn paste_family_live_state_gates_hold() {
    let mut engine = assembly_point::assembled_engine();
    engine
        .apply_bytes(
            br#"{"schema":"appname.command.v1","id":"ed17e000-0000-4000-8000-000000000050","kind":"doc.new","params":{"width":64,"height":48,"background":"white"}}"#,
        )
        .expect("doc.new");
    let asset = engine.add_raster_asset(paste_source_png()).expect("ingest");

    // paste_into without a selection -> selection_required (all three
    // required params present so stage 5 passes and the §40.5 live-state
    // gate answers: assetId + newLayerId + newMaskId per the contract)
    let env = format!(
        r#"{{"schema":"appname.command.v1","id":"ed17e000-0000-4000-8000-000000000051","kind":"edit.paste_into","params":{{"assetId":"{asset}","newLayerId":"ed17e000-0000-4000-8000-000000000052","newMaskId":"ed17e000-0000-4000-8000-000000000055"}}}}"#
    );
    let err = engine.apply_bytes(env.as_bytes()).unwrap_err();
    assert_eq!(
        err.code.as_str(),
        "error.command.selection_required",
        "paste_into live-state gate"
    );

    // paste of an uningested asset id -> bad_target (ingress precondition)
    let env = r#"{"schema":"appname.command.v1","id":"ed17e000-0000-4000-8000-000000000053","kind":"edit.paste","params":{"assetId":"ed17e000-0000-4000-8000-0000000000ff","at":[0,0],"newLayerId":"ed17e000-0000-4000-8000-000000000054"}}"#;
    let err = engine.apply_bytes(env.as_bytes()).unwrap_err();
    assert_eq!(
        err.code.as_str(),
        "error.command.bad_target",
        "uningested asset must not resolve"
    );
}
