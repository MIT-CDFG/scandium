// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Composite SERVE-golden verification — ACTIVE regression gate (O-QA
//! CP-1.0 golden round; the composite walls fell @ 3b21d8f).
//!
//! Re-produces the text-layer and shape-layer composite goldens from a
//! fresh assembled production and byte-compares. `produce_all` itself
//! re-asserts the export==readback one-truth invariant and the D-48
//! undo/redo/replay evidence, so a green run re-proves the wall.

use appname_conformance::composite_goldens::produce_all;
use appname_conformance::paths;

fn composite_dir() -> std::path::PathBuf {
    paths::repo_root().join("goldens").join("composite")
}

const STATIONS: [&str; 2] = ["composite-shape", "composite-text"];

#[test]
fn composite_goldens_match_fresh_assembled_production() {
    let produced = produce_all(); // re-asserts export==readback + D-48
    assert_eq!(produced.len(), STATIONS.len());
    for g in &produced {
        assert!(
            STATIONS.contains(&g.name),
            "unexpected station name: {}",
            g.name
        );
        let pkg = std::fs::read(composite_dir().join(format!("{}.appname", g.name)))
            .unwrap_or_else(|e| panic!("{}: package golden missing: {e}", g.name));
        assert_eq!(
            g.package, pkg,
            "{}: committed package golden != fresh production — an \
             INVESTIGATION, never a re-pin (freeze discipline); route \
             through O-QA review",
            g.name
        );
        let png = std::fs::read(composite_dir().join(format!("{}.png", g.name)))
            .unwrap_or_else(|e| panic!("{}: export golden missing: {e}", g.name));
        assert_eq!(g.export, png, "{}: export golden drifted", g.name);
    }
}

#[test]
fn composite_manifest_passes_shasum() {
    let out = appname_conformance::shasum256()
        .args(["-c", "MANIFEST.sha256"])
        .current_dir(composite_dir())
        .output()
        .expect("shasum available");
    assert!(
        out.status.success(),
        "goldens/composite/MANIFEST.sha256 check failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The export==readback invariant is the golden's own contract — every
/// meta must record it (the wall that fell).
#[test]
fn composite_meta_records_the_one_truth_invariant() {
    for name in STATIONS {
        let meta = std::fs::read_to_string(composite_dir().join(format!("{name}.meta")))
            .unwrap_or_else(|e| panic!("{name}.meta missing: {e}"));
        assert!(
            meta.contains("invariant: export_png_bytes == encode(composite_plane_f32)"),
            "{name}.meta: missing the export==readback one-truth invariant"
        );
    }
}
