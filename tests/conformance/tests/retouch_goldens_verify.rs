// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Retouch-suite stroke-golden verification — ACTIVE regression gate
//! (O-QA CP-1.0 golden round; the retouch row merged d660d3b staged its
//! goldens for this freeze).
//!
//! The suite re-produces every retouch golden from a fresh assembled
//! production and byte-compares against the committed files; production
//! itself re-proves the undo/redo/replay D-48 evidence per station, so a
//! green run here is also a determinism re-proof. A drifted byte is an
//! INVESTIGATION, never a silent re-pin (freeze discipline; route through
//! O-QA review).

use appname_conformance::paths;
use appname_conformance::retouch_goldens::produce_all;

fn retouch_dir() -> std::path::PathBuf {
    paths::repo_root().join("goldens").join("retouch")
}

const STATIONS: [&str; 6] = [
    "retouch-clone",
    "retouch-heal-sampled",
    "retouch-heal-spot",
    "retouch-dodge-burn",
    "retouch-smudge",
    "retouch-blur-sharpen",
];

#[test]
fn retouch_goldens_match_fresh_assembled_production() {
    let produced = produce_all(); // re-proves undo/redo/replay evidence
    assert_eq!(produced.len(), STATIONS.len());
    for g in &produced {
        assert!(
            STATIONS.contains(&g.name),
            "unexpected station name: {}",
            g.name
        );
        let pkg = std::fs::read(retouch_dir().join(format!("{}.appname", g.name)))
            .unwrap_or_else(|e| panic!("{}: package golden missing: {e}", g.name));
        assert_eq!(
            g.package, pkg,
            "{}: committed package golden != fresh production — an \
             INVESTIGATION, never a re-pin (freeze discipline); route \
             through O-QA review",
            g.name
        );
        let png = std::fs::read(retouch_dir().join(format!("{}.png", g.name)))
            .unwrap_or_else(|e| panic!("{}: export golden missing: {e}", g.name));
        assert_eq!(g.export, png, "{}: export golden drifted", g.name);
    }
}

#[test]
fn retouch_manifest_passes_shasum() {
    let out = appname_conformance::shasum256()
        .args(["-c", "MANIFEST.sha256"])
        .current_dir(retouch_dir())
        .output()
        .expect("shasum available");
    assert!(
        out.status.success(),
        "goldens/retouch/MANIFEST.sha256 check failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Each station's `.meta` records its seed rationale + oracle (the D-48
/// evidence) — the freeze discipline's per-golden requirement.
#[test]
fn retouch_meta_carries_seed_rationale_and_evidence() {
    for name in STATIONS {
        let meta = std::fs::read_to_string(retouch_dir().join(format!("{name}.meta")))
            .unwrap_or_else(|e| panic!("{name}.meta missing: {e}"));
        assert!(
            meta.contains("seed-rationale:"),
            "{name}.meta: missing the seed rationale"
        );
        assert!(
            meta.contains("evidence: undo/redo byte-identity"),
            "{name}.meta: missing the D-48 evidence oracle"
        );
    }
}
