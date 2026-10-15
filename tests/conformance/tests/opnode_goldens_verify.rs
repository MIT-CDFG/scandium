// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! OpNode re-render golden verification — ACTIVE regression gate
//! (CP-0.7; COMPOSITE-LEVEL revision, reopened-CP-0.7 goldens round).
//! Scope history + rationale in `appname_conformance::opnode_goldens`.
//!
//! This suite's `composite_door_tripwire` used to be an ARMED SENTINEL
//! (finding QA-F8: the composite doors refused/omitted smart content;
//! the goldens were scope-pinned to surface level and the sentinel
//! failed the workspace the moment `composite_document` started
//! answering, printing the extension instructions). The §12.1 row
//! landed, the sentinel FIRED as designed, and this revision is its
//! designed closure: the goldens now pin the composite level, and the
//! sentinel is CONVERTED to the live serve-and-match verify below —
//! its job is done; the goldens it demanded exist and guard the
//! behavior. QA-F8 is CLOSED (reports/O-QA-log.md goldens-round entry).

use appname_conformance::opnode_goldens::{produce, surface_bytes, SOL};
use appname_conformance::paths;

fn dir() -> std::path::PathBuf {
    paths::repo_root().join("goldens").join("opnode")
}

/// Committed surface AND composite goldens == fresh production;
/// stations reproduce (state hashes + per-instance surface hashes +
/// composite hashes from stations.meta); the undo station byte-restores
/// the pre-edit state on BOTH levels; the content swap is a REAL pixel
/// change at BOTH levels (the composite delta is the QA-F8 truth,
/// verifiable end-to-end since the §12.1 row).
#[test]
fn opnode_goldens_match_committed() {
    let flow = produce();
    let conv = std::fs::read(dir().join("postConvert.surface.bin")).expect("committed");
    let edit = std::fs::read(dir().join("postEdit.surface.bin")).expect("committed");
    assert_eq!(
        flow.post_convert_surface, conv,
        "postConvert surface drifted"
    );
    assert_eq!(flow.post_edit_surface, edit, "postEdit surface drifted");
    assert_ne!(conv, edit, "content swap must change the surface");

    // Composite level (the fired tripwire's demanded extension).
    for (name, fresh) in [
        ("postConvert.composite.bin", &flow.post_convert_composite),
        ("postEdit.composite.bin", &flow.post_edit_composite),
        ("postChainAdd.composite.bin", &flow.post_chain_composite),
        ("postAdjustAdd.composite.bin", &flow.post_adjust_composite),
    ] {
        let committed = std::fs::read(dir().join(name)).expect(name);
        assert_eq!(fresh, &committed, "{name} drifted");
    }
    assert_ne!(
        flow.post_convert_composite, flow.post_edit_composite,
        "content swap must change the COMPOSITE (QA-F8 truth, end-to-end)"
    );
    assert_ne!(
        flow.post_chain_composite, flow.post_edit_composite,
        "the enabled gaussian chain must bite at the composite door"
    );
    assert_ne!(
        flow.post_adjust_composite, flow.post_chain_composite,
        "the invert adjustment must bite over the backdrop"
    );

    let meta = std::fs::read_to_string(dir().join("stations.meta")).expect("meta");
    for s in &flow.stations {
        assert!(
            meta.contains(&format!("station {}: state_hash {}", s.name, s.state_hash)),
            "{}: state_hash drifted from stations.meta ({})",
            s.name,
            s.state_hash
        );
        for (layer, h) in &s.surfaces {
            assert!(
                meta.contains(h.as_str()),
                "{}: surface hash for {layer} drifted",
                s.name
            );
        }
        assert!(
            meta.contains(&format!("composite: {}", s.composite)),
            "{}: composite hash drifted from stations.meta ({})",
            s.name,
            s.composite
        );
    }
    // undo == pre-edit state on every instance AND at the composite
    // door; redo == edit-state on both levels.
    let by_name = |n: &str| flow.stations.iter().find(|s| s.name == n).unwrap();
    assert_eq!(
        by_name("postUndo").surfaces[0].1,
        by_name("postConvert").surfaces[0].1,
        "undo restores the convert-state surface"
    );
    assert_eq!(
        by_name("postRedo").surfaces,
        by_name("postEdit").surfaces,
        "redo restores the edit-state surfaces on both instances"
    );
    assert_eq!(
        by_name("postUndo").composite,
        by_name("postDuplicate").composite,
        "undo restores the pre-edit COMPOSITE (duplicate-state, byte-hash)"
    );
    assert_eq!(
        by_name("postRedo").composite,
        by_name("postEdit").composite,
        "redo restores the edit-state COMPOSITE"
    );
}

/// Manifest self-check (shasum -c, house pattern).
#[test]
fn opnode_manifest_passes_shasum() {
    let out = appname_conformance::shasum256()
        .args(["-c", "MANIFEST.sha256"])
        .current_dir(dir())
        .output()
        .expect("shasum available");
    assert!(
        out.status.success(),
        "shasum -c failed:\n{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// LIVE COMPOSITE-DOOR VERIFY — the converted tripwire (armed-sentinel
/// → live gate; kind-tail precedent, closure leg). The exact document
/// the sentinel used to build now SERVES at both public doors with the
/// smart content PRESENT — the QA-F8 serve-or-refuse discipline held
/// through the whole §12.1 row: no door ever silently omitted, and now
/// they answer with real content.
#[test]
fn composite_door_tripwire() {
    let mut engine = appname_conformance::assembly_point::assembled_engine();
    engine
        .apply_bytes(br#"{"schema":"appname.command.v1","id":"c07e0000-0000-4000-8000-000000000d0c","kind":"doc.new","params":{"width":64,"height":64,"background":"white"}}"#)
        .unwrap();
    engine
        .apply_bytes(br#"{"schema":"appname.command.v1","id":"c0000000-0000-4000-8000-00000000aaa1","kind":"layer.add","params":{"kind":"raster","newLayerId":"c0000000-0000-4000-8000-00000000bbb1","name":"art"}}"#)
        .unwrap();
    engine
        .apply_bytes(br#"{"schema":"appname.command.v1","id":"c0000000-0000-4000-8000-00000000aaa2","kind":"fill","target":{"layerId":"c0000000-0000-4000-8000-00000000bbb1"},"params":{"content":"foreground"}}"#)
        .unwrap();
    engine
        .apply_bytes(br#"{"schema":"appname.command.v1","id":"c0000000-0000-4000-8000-00000000aaa3","kind":"smartobject.convert","params":{"layerIds":["c0000000-0000-4000-8000-00000000bbb1"],"newLayerId":"c0000000-0000-4000-8000-00000000bbb2"}}"#)
        .unwrap();
    let _ = surface_bytes(&engine, SOL); // smart instance live
    let doc = engine.document().unwrap();
    let canvas = appname_core::geom::IRect::new(0, 0, 64, 64);

    // The bare-registry door (the sentinel's exact probe) ANSWERS.
    let bare = appname_render::composite_document(doc, canvas)
        .expect("composite_document serves the smart-object document (§12.1 row landed)");
    // The smart content is PRESENT, not omitted: the foreground fill
    // covers the white background — the composite is NOT white.
    let px = bare.pixel(appname_core::IVec2::new(8, 8));
    assert!(
        px[3] == 1.0 && (px[0] < 1.0 || px[1] < 1.0 || px[2] < 1.0),
        "smart content must be present in the composite (got {px:?})"
    );
    // Bare door == assembled door on this chainless document (one truth).
    let assembled =
        appname_render::composite_document_with(doc, engine.assembly(), canvas).unwrap();
    for ((k, t), (ka, ta)) in bare.tiles().zip(assembled.tiles()) {
        assert_eq!(k, ka);
        assert!(t == ta, "bare == assembled at tile {k:?}");
    }
    // Core's export door serves the same document (serve/refuse parity;
    // the silent-omit class is dead: QA-F8 closure citation).
    appname_core::export_png_bytes(&engine).expect("core export serves smart content");

    // The composite-level goldens the sentinel demanded exist and are
    // live: the full flow's chain/adjustment stations serve too (their
    // bytes are pinned against the committed goldens in
    // `opnode_goldens_match_committed`; here we pin the ids so a
    // station rename cannot silently drop coverage).
    let flow = produce();
    for want in ["postChainAdd", "postAdjustAdd"] {
        assert!(
            flow.stations.iter().any(|s| s.name == want),
            "composite extension station {want} present"
        );
    }
}
