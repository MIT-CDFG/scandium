// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Golden verification — ACTIVE regression gate (wave 4).
//!
//! Spec: plan §0.6 rule 6 (goldens as cross-lane contract; changes only
//! via O-QA review) + §2.1 "Test harness v1"; goldens/README.md rules;
//! rule 7 (expected bytes come from the engine, mechanically).
//!
//! The strongest possible check runs on every `cargo test`: the committed
//! golden bytes are compared against a FRESH in-memory engine production
//! of the same fixed inputs — a lane whose change shifts any package or
//! export byte turns this red and routes through O-QA review. Manifest
//! integrity is additionally checked with `shasum -c`.

use appname_conformance::goldens::{
    doc_new_matrix, produce, selftest_reference_package, SELFTEST_REFERENCE_BYTES,
    SELFTEST_REFERENCE_SHA256, SELFTEST_STATE_HASH_PREFIX,
};
use appname_conformance::paths;

#[test]
fn doc_new_goldens_match_fresh_engine_production() {
    let dir = paths::repo_root().join("goldens").join("doc_new");
    let mut exports = 0;
    let mut deferred = 0;
    for combo in doc_new_matrix() {
        let (pkg, png) = produce(&combo.envelope);
        let committed_pkg = std::fs::read(dir.join(format!("{}.appname", combo.name)))
            .unwrap_or_else(|e| panic!("{}: package golden missing: {e}", combo.name));
        assert_eq!(
            pkg, committed_pkg,
            "{}: committed package golden != fresh engine production — \
             route the change through O-QA review",
            combo.name
        );
        match png {
            Ok(png) => {
                let committed_png = std::fs::read(dir.join(format!("{}.png", combo.name)))
                    .unwrap_or_else(|e| panic!("{}: png golden missing: {e}", combo.name));
                assert_eq!(
                    png, committed_png,
                    "{}: committed export golden != fresh engine production",
                    combo.name
                );
                exports += 1;
            }
            Err(reason) => {
                // the deferral must stay declared in the sidecar
                let meta =
                    std::fs::read_to_string(dir.join(format!("{}.meta", combo.name))).unwrap();
                assert!(
                    meta.contains("export: DEFERRED"),
                    "{}: engine defers the export ({reason}) but the meta \
                     does not declare it",
                    combo.name
                );
                deferred += 1;
            }
        }
    }
    // the CP-0.1 boundary, pinned: rgba exports live, non-rgba deferred
    assert_eq!(
        (exports, deferred),
        (6, 12),
        "export-availability boundary moved (conversion pipeline landed?) \
         — regenerate goldens via O-QA review"
    );
}

#[test]
fn golden_manifests_pass_shasum() {
    for sub in ["doc_new", "selftest"] {
        let dir = paths::repo_root().join("goldens").join(sub);
        let out = appname_conformance::shasum256()
            .args(["-c", "MANIFEST.sha256"])
            .current_dir(&dir)
            .output()
            .expect("shasum available");
        assert!(
            out.status.success(),
            "goldens/{sub}/MANIFEST.sha256 check failed:\n{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn selftest_reference_package_matches_the_pinned_record() {
    // Pin provenance: reports/O-CORE-log.md stroke-wiring entry — the
    // CP-0.3 reference (doc.new + brush leg; the pre-stroke 527133-byte
    // reference is retired). Reproduced by mirroring the selftest fixed
    // script verbatim on the assembled engine.
    let (pkg, state_hash) = selftest_reference_package();
    assert_eq!(pkg.len(), SELFTEST_REFERENCE_BYTES);
    assert!(
        state_hash.starts_with(SELFTEST_STATE_HASH_PREFIX),
        "state_hash drifted: {state_hash}"
    );
    let committed = std::fs::read(
        paths::repo_root()
            .join("goldens")
            .join("selftest")
            .join("reference.appname"),
    )
    .expect("selftest reference golden committed");
    assert_eq!(pkg, committed, "selftest reference drifted");

    // and the committed bytes carry the O-CORE-log sha (via shasum)
    // W-19 (O-WIN): hash the bare filename from its own dir — GNU sha256sum
    // prefixes its output line with '' when the path holds a backslash.
    let out = appname_conformance::shasum256()
        .arg("reference.appname")
        .current_dir(paths::repo_root().join("goldens").join("selftest"))
        .output()
        .expect("shasum available");
    let sha = String::from_utf8(out.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();
    assert_eq!(sha, SELFTEST_REFERENCE_SHA256);
}
