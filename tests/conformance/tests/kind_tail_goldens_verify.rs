// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Kind-tail golden verification (layer family, merge trio watched
//! divergence, furniture) — ACTIVE regression gate (CP-0.5
//! wave 3). Re-produces all four goldens (which re-asserts the mask
//! round-trip and the assign-vs-convert pixel truth on every run) and
//! byte-compares the committed files; pins the DisplayP3 literal the
//! W04 runner bakes.
//!
//! Spec: contracts mask.add.yaml / doc.assign_profile.yaml /
//! doc.convert_profile.yaml; ES §5/§13; plan §0.6 rule 6.

use appname_conformance::kind_tail_goldens::produce_all;
use appname_conformance::paths;

fn dir() -> std::path::PathBuf {
    paths::repo_root().join("goldens").join("kind_tail")
}

#[test]
fn kind_tail_goldens_match_fresh_assembled_production() {
    let produced = produce_all();
    assert_eq!(produced.len(), 5);
    for g in &produced {
        let pkg = std::fs::read(dir().join(format!("{}.appname", g.name)))
            .unwrap_or_else(|e| panic!("{}: package golden missing: {e}", g.name));
        assert_eq!(
            g.package, pkg,
            "{}: committed golden != fresh production — route through \
             O-QA review",
            g.name
        );
        let committed = std::fs::read(dir().join(format!("{}.png", g.name)))
            .unwrap_or_else(|e| panic!("{}: export golden missing: {e}", g.name));
        assert_eq!(g.export, committed, "{}: export drifted", g.name);
    }
}

#[test]
fn kind_tail_manifest_passes_shasum() {
    let out = appname_conformance::shasum256()
        .args(["-c", "MANIFEST.sha256"])
        .current_dir(dir())
        .output()
        .expect("shasum available");
    assert!(
        out.status.success(),
        "goldens/kind_tail/MANIFEST.sha256 check failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
