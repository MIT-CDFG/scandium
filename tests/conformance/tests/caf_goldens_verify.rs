// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! CAF §45.3 behavioral-golden verification — ACTIVE regression gate
//! (O-QA CP-1.0 ratification batch; the freeze the ledger trigger
//! staged).
//!
//! The freeze citations ride in every goldens/caf/*.meta (ratification
//! entry + the live `caf_stream_conformance` test); this suite holds
//! the frozen bytes against a fresh assembled production — production
//! itself re-proves the undo/redo/replay evidence, so a green run here
//! is also a D-48 re-proof.

use appname_conformance::caf_goldens::produce_all;
use appname_conformance::paths;

fn caf_dir() -> std::path::PathBuf {
    paths::repo_root().join("goldens").join("caf")
}

#[test]
fn caf_goldens_match_fresh_assembled_production() {
    let produced = produce_all(); // re-proves undo/redo/replay evidence
    assert_eq!(produced.len(), 4);
    for g in &produced {
        let pkg = std::fs::read(caf_dir().join(format!("{}.appname", g.name)))
            .unwrap_or_else(|e| panic!("{}: package golden missing: {e}", g.name));
        assert_eq!(
            g.package, pkg,
            "{}: committed package golden != fresh production — an \
             INVESTIGATION, never a re-pin (freeze discipline); route \
             through O-QA review",
            g.name
        );
        let png = std::fs::read(caf_dir().join(format!("{}.png", g.name)))
            .unwrap_or_else(|e| panic!("{}: export golden missing: {e}", g.name));
        assert_eq!(g.export, png, "{}: export golden drifted", g.name);
    }
}

#[test]
fn caf_manifest_passes_shasum() {
    let out = appname_conformance::shasum256()
        .args(["-c", "MANIFEST.sha256"])
        .current_dir(caf_dir())
        .output()
        .expect("shasum available");
    assert!(
        out.status.success(),
        "goldens/caf/MANIFEST.sha256 check failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The freeze citations are load-bearing: every meta must carry BOTH
/// (ratification entry + live conformance test) — the ledger trigger's
/// letter.
#[test]
fn caf_meta_carries_both_freeze_citations() {
    for name in [
        "caf-heal-rect",
        "caf-heal-seed1",
        "caf-params-rect-draft",
        "caf-starved-fallback",
    ] {
        let meta = std::fs::read_to_string(caf_dir().join(format!("{name}.meta")))
            .unwrap_or_else(|e| panic!("{name}.meta missing: {e}"));
        assert!(
            meta.contains("CAF §45.3 SEEDED-STREAM MICRO-LAYOUT RATIFICATION"),
            "{name}.meta: missing the ratification citation"
        );
        assert!(
            meta.contains("caf_stream_conformance"),
            "{name}.meta: missing the live-conformance-test citation"
        );
    }
}
