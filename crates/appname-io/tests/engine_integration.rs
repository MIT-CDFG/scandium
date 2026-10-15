// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: plan §2.3 "Import produces layer-compatible buffers via core's
//! PUBLIC surface; export composites via the established path." The
//! decode → [`appname_io::import_into_engine`] → core-export loop through
//! the JOURNALED doors (landed MAJOR-2 reconciliation:
//! `Engine::add_raster_asset` + `edit.paste` envelopes — `import_layer` is
//! retired). New property, asserted here because the old out-of-journal
//! door could not offer it: REPLAY IDENTITY holds through imports —
//! import → save → reopen → replay → `state_hash` equality.

mod common;

use appname_io::{
    CodecOptions, CodecRegistry, DecodeOpts, EncodeOpts, GifOptions, JpegOptions, Metadata,
};
use common::{linear_plane, srgb_bytes};

/// The reference-consumer assembly (CLI parity): core kinds registered so
/// `edit.paste` resolves.
fn engine_with_doc(w: u32, h: u32) -> appname_core::Engine {
    let mut engine =
        appname_core::engine::kinds::register(appname_core::EngineAssembly::new()).build();
    let envelope = format!(
        "{{\"schema\":\"appname.command.v1\",\"id\":\"c0000000-0000-4000-8000-000000000101\",\
         \"kind\":\"doc.new\",\"params\":{{\"width\":{w},\"height\":{h},\"background\":\"white\"}}}}"
    );
    engine.apply_json(&envelope).unwrap();
    engine
}

/// Save → reopen → replay → state-hash equality (the CLI's
/// replay-as-recovery gate, which REFUSED on the retired door's imports).
fn assert_replay_identity(engine: &appname_core::Engine, ctx: &str) {
    // In-memory package round trip (public doors).
    let bytes = appname_core::save_package_bytes(engine).unwrap();
    let loaded = appname_core::load_package_bytes_with(engine.assembly(), &bytes).unwrap();
    assert_eq!(
        loaded.engine.state_hash(),
        engine.state_hash(),
        "{ctx}: reopen reproduces the document"
    );
    // Journal replay from scratch through the same assembly.
    let replayed = loaded
        .engine
        .replay_assembled(loaded.engine.journal())
        .unwrap();
    assert_eq!(
        replayed.state_hash(),
        engine.state_hash(),
        "{ctx}: REPLAY IDENTITY — the journal alone rebuilds the imported document"
    );
}

#[test]
fn decoded_jpeg_lands_journaled_and_replay_identity_holds() {
    let bytes = srgb_bytes(6, 4, true);
    let plane = linear_plane(&bytes);
    let meta = Metadata::default();
    let img = appname_io::ExportImage {
        width: 6,
        height: 4,
        plane: &plane,
        layers: None,
        profile: None,
        meta: &meta,
    };
    let registry = CodecRegistry::builtin();
    let jpeg = registry
        .by_id("jpeg")
        .unwrap()
        .encode(
            &img,
            &EncodeOpts::new(CodecOptions::Jpeg(JpegOptions::default())),
        )
        .unwrap();

    let mut engine = engine_with_doc(6, 4);
    let journal_before = engine.journal().len();
    let before = engine.document().unwrap().layer_count();
    let imported = registry
        .by_id("jpeg")
        .unwrap()
        .decode(&jpeg, &DecodeOpts::default())
        .unwrap();
    let ids = appname_io::import_into_engine(&mut engine, "photo", imported).unwrap();
    assert_eq!(ids.len(), 1);
    assert_eq!(engine.document().unwrap().layer_count(), before + 1);
    // JOURNALED: the import added edit.paste + layer.rename envelopes
    // (no out-of-spine mutation; names apply since the wave-4 tail).
    assert_eq!(engine.journal().len(), journal_before + 2);
    let kinds: Vec<&str> = engine.journal()[journal_before..]
        .iter()
        .map(|e| e.kind.as_str())
        .collect();
    assert_eq!(kinds, ["edit.paste", "layer.rename"]);
    let doc = engine.document().unwrap();
    assert_eq!(doc.layer(ids[0]).unwrap().name(), "photo");

    // Export composites via the established §4.4 reference path.
    let png = appname_core::export_png_bytes(&engine).unwrap();
    assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));

    // THE new property (review round 2): replay identity holds through
    // the journaled import.
    assert_replay_identity(&engine, "jpeg import");

    // Determinism across repeat runs: same bytes in -> same envelopes,
    // same state hash, same exported bytes.
    let mut engine2 = engine_with_doc(6, 4);
    let imported2 = registry
        .by_id("jpeg")
        .unwrap()
        .decode(&jpeg, &DecodeOpts::default())
        .unwrap();
    appname_io::import_into_engine(&mut engine2, "photo", imported2).unwrap();
    assert_eq!(
        engine2.state_hash(),
        engine.state_hash(),
        "derived ids make runs identical"
    );
    assert_eq!(appname_core::export_png_bytes(&engine2).unwrap(), png);
}

#[test]
fn layered_gif_import_lands_every_frame_journaled() {
    // Build a 2-frame GIF through the export contract, then land it.
    let f0 = srgb_bytes(3, 3, true);
    let f1: Vec<u8> = f0.iter().map(|b| b.wrapping_add(80)).collect();
    let p0 = linear_plane(&f0);
    let p1 = linear_plane(&f1);
    let meta = Metadata::default();
    let img = appname_io::ExportImage {
        width: 3,
        height: 3,
        plane: &p0,
        layers: Some(vec![
            appname_io::ExportLayer {
                name: "frame:0 delay:20",
                plane: &p0,
            },
            appname_io::ExportLayer {
                name: "frame:1 delay:20",
                plane: &p1,
            },
        ]),
        profile: None,
        meta: &meta,
    };
    let registry = CodecRegistry::builtin();
    let gif = registry
        .by_id("gif")
        .unwrap()
        .encode(
            &img,
            &EncodeOpts::new(CodecOptions::Gif(GifOptions::default())),
        )
        .unwrap();
    let imported = registry
        .by_id("gif")
        .unwrap()
        .decode(&gif, &DecodeOpts::default())
        .unwrap();

    let mut engine = engine_with_doc(3, 3);
    let journal_before = engine.journal().len();
    let before = engine.document().unwrap().layer_count();
    let ids = appname_io::import_into_engine(&mut engine, "anim", imported).unwrap();
    assert_eq!(ids.len(), 2, "one paste per frame");
    let doc = engine.document().unwrap();
    assert_eq!(doc.layer_count(), before + 2);
    assert_eq!(
        engine.journal().len(),
        journal_before + 4,
        "paste + rename per frame (all-default props add no envelope)"
    );
    assert_eq!(doc.layer(ids[0]).unwrap().name(), "frame:0 delay:20");
    assert_eq!(doc.layer(ids[1]).unwrap().name(), "frame:1 delay:20");
    // Bottom-to-top: frame 0 pasted first sits below frame 1 (§8.2
    // stacking; each paste lands above the previous).
    let z: Vec<appname_core::LayerId> = doc
        .z_order_walk()
        .iter()
        .copied()
        .filter(|id| doc.layer(*id).is_some_and(|l| l.pixels().is_some()))
        .collect();
    let pos0 = z.iter().position(|id| *id == ids[0]).unwrap();
    let pos1 = z.iter().position(|id| *id == ids[1]).unwrap();
    assert!(pos0 < pos1, "frame 0 below frame 1");

    // Replay identity across a layered import too.
    assert_replay_identity(&engine, "gif layered import");
}
