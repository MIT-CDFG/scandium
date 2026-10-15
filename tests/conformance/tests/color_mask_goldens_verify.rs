// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Mask + color golden verification — ACTIVE regression gate (CP-0.5
//! wave 3). Re-produces all five goldens (which re-asserts the §11.2
//! mask SERVE — mask-add-roundtrip + masks_stack — and the
//! assign-vs-convert pixel truth on every run) and byte-compares the
//! committed files; pins the DisplayP3 literal the W04 runner bakes.
//!
//! Spec: contracts mask.add.yaml / doc.assign_profile.yaml /
//! doc.convert_profile.yaml; ES §5/§13; plan §0.6 rule 6.

use appname_conformance::color_mask_goldens::{display_p3_id, produce_all};
use appname_conformance::paths;

fn dir() -> std::path::PathBuf {
    paths::repo_root().join("goldens").join("color_mask")
}

#[test]
fn color_mask_goldens_match_fresh_assembled_production() {
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
        if let Some(png) = &g.export {
            let committed = std::fs::read(dir().join(format!("{}.png", g.name)))
                .unwrap_or_else(|e| panic!("{}: export golden missing: {e}", g.name));
            assert_eq!(png, &committed, "{}: export drifted", g.name);
        }
    }
}

#[test]
fn color_mask_manifest_passes_shasum() {
    let out = appname_conformance::shasum256()
        .args(["-c", "MANIFEST.sha256"])
        .current_dir(dir())
        .output()
        .expect("shasum available");
    assert!(
        out.status.success(),
        "goldens/color_mask/MANIFEST.sha256 check failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The W04 runner bakes the DisplayP3 uuid literal — pin it to the
/// deriving fn so the literal cannot drift silently.
#[test]
fn w04_baked_profile_literal_is_the_derived_id() {
    assert_eq!(
        display_p3_id().0.to_string(),
        "ee12374b-5f25-51fd-a1c8-f45790a61d92",
        "the DisplayP3 builtin id moved — update the W04 runner literal \
         via O-QA review"
    );
}

/// `builtin:lab` pinned the way `builtin:srgb` is pinned (O-QA queued
/// observation from the door-slice-3 cabi review, added at the
/// ratification sweep now that `doc.mode_convert{to:"lab_a"}` is LIVE
/// on main and the CIELAB working profile is reachable). Two legs:
///
/// 1. Literal pin — `builtin_lab_profile_id()` against its uuid
///    literal, so id drift breaks HERE, not in the C-ABI leg that
///    renders `"builtin:lab"` from it. Rule 8: the literal was
///    extracted from the deriving fn on the shipped composition
///    2026-08-17 and cross-derived independently (uuid5 over
///    `APPNAME_NS` with seed `profile/builtin/lab`, both agree).
/// 2. Live reachability — doc.new → doc.mode_convert{to: lab_a} on the
///    assembled engine retags the working profile to exactly that id
///    with the `BuiltinLab` source (the CMM-built connection space,
///    §6.2/§5.3 — nothing to embed, hence the pinned-id discipline).
#[test]
fn builtin_lab_literal_is_the_derived_id_and_mode_convert_reaches_it() {
    use appname_core::engine::color_glue::builtin_lab_profile_id;
    use appname_core::graph::document::DocProfileSource;

    assert_eq!(
        builtin_lab_profile_id().0.to_string(),
        "af85c78e-40c5-548f-b4b1-5e6cad340c42",
        "the builtin Lab profile id moved — update via O-QA review \
         (the C-ABI info leg renders builtin:lab from this id)"
    );

    let mut e = appname_conformance::assembly_point::assembled_engine();
    e.apply_json(
        r#"{"schema":"appname.command.v1","id":"c0104000-0000-4000-8000-00000000ab01","kind":"doc.new","params":{"width":64,"height":64,"background":"white"}}"#,
    )
    .expect("doc.new");
    e.apply_json(
        r#"{"schema":"appname.command.v1","id":"c0104000-0000-4000-8000-00000000ab02","kind":"doc.mode_convert","params":{"to":"lab_a"}}"#,
    )
    .expect("doc.mode_convert to lab_a");
    let d = e.document().expect("document");
    let working = d.color().working;
    assert_eq!(
        working,
        builtin_lab_profile_id(),
        "lab_a conversion must retag the working profile to the derived \
         builtin Lab id"
    );
    assert!(
        matches!(
            d.profile(working).map(|p| &p.source),
            Some(DocProfileSource::BuiltinLab)
        ),
        "working profile source must be BuiltinLab after lab_a \
         conversion"
    );
}
