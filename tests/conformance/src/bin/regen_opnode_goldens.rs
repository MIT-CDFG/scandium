// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! regen_opnode_goldens — mechanical (re)generator for the CP-0.7 OpNode
//! re-render correctness goldens (O-QA; module doc in
//! `appname_conformance::opnode_goldens` carries the scope history).
//!
//! COMPOSITE-LEVEL revision (reopened CP-0.7, the fired composite-door
//! tripwire's own instructions, O-QA goldens round): writes
//! goldens/opnode/ with
//!   * the two SURFACE content states of the smart layer (postConvert /
//!     postEdit, little-endian f32 RGBA over bounds) — UNCHANGED from
//!     the surface-level revision (verified via `git status` at the
//!     goldens round: only the meta/manifest and the new composite
//!     files moved);
//!   * four full-canvas COMPOSITE states through the render door
//!     (postConvert / postEdit / postChainAdd / postAdjustAdd,
//!     little-endian f32 RGBA over the canvas rect);
//!   * stations.meta with every station's state_hash, per-instance
//!     surface hashes, AND composite hash;
//!   * MANIFEST.sha256.
//!
//! Produced TWICE through independent engines and byte-compared before
//! writing (goldens/README.md rule 5).
//!
//! Run:  cargo run -p appname-conformance --bin regen_opnode_goldens

use appname_conformance::opnode_goldens::{produce, CONVERT_ASSET};
use appname_conformance::paths;
use std::path::Path;
use std::process::Command;

fn sha256(path: &Path) -> String {
    let out = Command::new("shasum")
        .args(["-a", "256"])
        .arg(path)
        .output()
        .expect("shasum available");
    assert!(out.status.success());
    String::from_utf8(out.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string()
}

fn main() {
    let dir = paths::repo_root().join("goldens").join("opnode");
    std::fs::create_dir_all(&dir).unwrap();
    let a = produce();
    let b = produce();
    assert_eq!(
        a.post_convert_surface, b.post_convert_surface,
        "postConvert surface nondeterminism"
    );
    assert_eq!(
        a.post_edit_surface, b.post_edit_surface,
        "postEdit surface nondeterminism"
    );
    for (name, ca, cb) in [
        (
            "postConvert",
            &a.post_convert_composite,
            &b.post_convert_composite,
        ),
        ("postEdit", &a.post_edit_composite, &b.post_edit_composite),
        (
            "postChainAdd",
            &a.post_chain_composite,
            &b.post_chain_composite,
        ),
        (
            "postAdjustAdd",
            &a.post_adjust_composite,
            &b.post_adjust_composite,
        ),
    ] {
        assert_eq!(ca, cb, "{name} composite nondeterminism");
    }
    for (sa, sb) in a.stations.iter().zip(&b.stations) {
        assert_eq!(
            sa.state_hash, sb.state_hash,
            "{}: state nondeterminism",
            sa.name
        );
        assert_eq!(
            sa.composite, sb.composite,
            "{}: composite nondeterminism",
            sa.name
        );
    }
    assert_ne!(
        a.post_convert_surface, a.post_edit_surface,
        "the content swap must CHANGE the surface (re-render truth)"
    );
    assert_ne!(
        a.post_convert_composite, a.post_edit_composite,
        "the content swap must CHANGE the composite (the QA-F8 truth, \
         now visible end-to-end)"
    );
    assert_ne!(
        a.post_chain_composite, a.post_edit_composite,
        "the enabled chain must BITE at the composite door (premise-real \
         station: a non-biting golden guards nothing)"
    );
    assert_ne!(
        a.post_adjust_composite, a.post_chain_composite,
        "the adjustment must BITE over the backdrop"
    );

    let mut manifest = Vec::new();
    for (name, bytes) in [
        ("postConvert.surface.bin", &a.post_convert_surface),
        ("postEdit.surface.bin", &a.post_edit_surface),
        ("postConvert.composite.bin", &a.post_convert_composite),
        ("postEdit.composite.bin", &a.post_edit_composite),
        ("postChainAdd.composite.bin", &a.post_chain_composite),
        ("postAdjustAdd.composite.bin", &a.post_adjust_composite),
    ] {
        let p = dir.join(name);
        std::fs::write(&p, bytes).unwrap();
        manifest.push(format!("{}  {name}", sha256(&p)));
        println!("golden opnode/{name}: {}", sha256(&p));
    }

    let mut meta = String::from(
        "goldens: opnode re-render (CP-0.7 exit-gate row; composite-level\n\
         revision per the fired composite-door tripwire, O-QA goldens round)\n\
         producer: appname_conformance::opnode_goldens::produce — fixed\n\
         envelope ids; doc.new 64x64 white -> layer.add -> fill ->\n\
         smartobject.convert -> layer.duplicate -> edit_contents (blue\n\
         fixture fixtures/w16/blue-64.png) -> undo -> redo ->\n\
         filter.smart_add (filter.custom dyadic single-tap (+2,0) shift\n\
         kernel on the TOPMOST instance DUP — chosen premise-real: a\n\
         chain on covered SOL, or a blur on the uniform proxy, would\n\
         composite to nothing) ->\n\
         adjustment.add (unmasked invert, top of stack); surfaces are\n\
         the smart layers' cached composites (LE f32 RGBA over bounds);\n\
         composites are the full-canvas render-door output\n\
         (composite_document_with over the engine's own op registry,\n\
         LE f32 RGBA over the canvas rect)\n\
         scope: COMPOSITE level (the former QA-F8 surface-level scope\n\
         pin is LIFTED — finding closed in reports/O-QA-log.md,\n\
         goldens-round entry; surface goldens unchanged from the\n\
         surface-level revision)\n",
    );
    meta.push_str(&format!(
        "convertAsset: {CONVERT_ASSET}\nblueAsset: {}\n",
        a.blue_asset
    ));
    for s in &a.stations {
        meta.push_str(&format!(
            "station {}: state_hash {}\n",
            s.name, s.state_hash
        ));
        for (layer, h) in &s.surfaces {
            meta.push_str(&format!("  surface {layer}: {h}\n"));
        }
        meta.push_str(&format!("  composite: {}\n", s.composite));
    }
    meta.push_str("rule: goldens change only via O-QA review (plan section 0.6 rule 6)\n");
    let p = dir.join("stations.meta");
    std::fs::write(&p, meta).unwrap();
    manifest.push(format!("{}  stations.meta", sha256(&p)));

    manifest.sort();
    std::fs::write(dir.join("MANIFEST.sha256"), manifest.join("\n") + "\n").unwrap();
    println!("opnode goldens written under {}", dir.display());
}
