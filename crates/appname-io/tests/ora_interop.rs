// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §8.4; interop truth `bogos/tests/interop/ora-fixtures.yaml`
//! (READ-ONLY — translated here into runnable tests; the covered subset
//! is tabulated in `reports/O-IO-log.md`). Fixture classes per the yaml:
//! (a)/(c)-style fixtures are script-built at test time; the (b)
//! checked-in golden corpus file does not exist in the spec repo yet, so
//! its three CHECKS run against a hand-built equivalent (same contents
//! the yaml describes: minimal, foreign, no `appname:` data).

mod common;

use appname_core::BlendMode;
use appname_io::codecs::ora::{
    blend_from_canonical, blend_from_ora_op, canonical_blend_string, ora_op_from_blend,
};
use appname_io::{
    Codec, CodecOptions, CodecRegistry, CompatAction, DecodeOpts, EncodeOpts, Metadata, OraOptions,
};
use common::{buffer_to_srgb_bytes, linear_plane, srgb_bytes};

// ------------------------------------------------------ fixture builders ----

/// Hand-built minimal FOREIGN ORA (the yaml's `ora_roundtrip.ora` class
/// (b) description: no `appname:` extension data). Entries Stored.
fn foreign_ora(
    stack_xml: &str,
    layer_pngs: &[(&str, Vec<u8>)],
    merged: Option<Vec<u8>>,
) -> Vec<u8> {
    use std::io::Write;
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut zw = zip::ZipWriter::new(&mut cursor);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zw.start_file("mimetype", opts).unwrap();
        zw.write_all(b"image/openraster").unwrap();
        zw.start_file("stack.xml", opts).unwrap();
        zw.write_all(stack_xml.as_bytes()).unwrap();
        for (name, png) in layer_pngs {
            zw.start_file(*name, opts).unwrap();
            zw.write_all(png).unwrap();
        }
        if let Some(m) = merged {
            zw.start_file("mergedimage.png", opts).unwrap();
            zw.write_all(&m).unwrap();
        }
        zw.finish().unwrap();
    }
    cursor.into_inner()
}

/// Same contents as [`foreign_ora`] but every entry AFTER `mimetype` is
/// DEFLATED — the shape other apps actually write (mimetype itself stays
/// Stored-first per the ORA container rule).
fn foreign_ora_deflated(
    stack_xml: &str,
    layer_pngs: &[(&str, Vec<u8>)],
    merged: Option<Vec<u8>>,
) -> Vec<u8> {
    use std::io::Write;
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut zw = zip::ZipWriter::new(&mut cursor);
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        let deflated = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zw.start_file("mimetype", stored).unwrap();
        zw.write_all(b"image/openraster").unwrap();
        zw.start_file("stack.xml", deflated).unwrap();
        zw.write_all(stack_xml.as_bytes()).unwrap();
        for (name, png) in layer_pngs {
            zw.start_file(*name, deflated).unwrap();
            zw.write_all(png).unwrap();
        }
        if let Some(m) = merged {
            zw.start_file("mergedimage.png", deflated).unwrap();
            zw.write_all(&m).unwrap();
        }
        zw.finish().unwrap();
    }
    cursor.into_inner()
}

/// A tiny PNG from straight sRGB bytes (through the crate's own plane).
fn tiny_png(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    appname_io::RgbaPlane::new8(w, h, rgba.to_vec())
        .unwrap()
        .to_png_bytes()
        .unwrap()
}

fn ora_codec(reg: &CodecRegistry) -> &dyn Codec {
    reg.by_id("ora").unwrap()
}

// ------------------------------------------- composite-op map (16 + 11) ----

/// yaml `compositeOpMapSpotChecks`: all 16 pairs, BOTH directions, with
/// the §4.5 discriminants.
#[test]
fn composite_op_map_spot_checks_both_directions() {
    let pairs: [(&str, BlendMode, u32); 16] = [
        ("svg:src-over", BlendMode::Normal, 0),
        ("svg:multiply", BlendMode::Multiply, 3),
        ("svg:screen", BlendMode::Screen, 8),
        ("svg:overlay", BlendMode::Overlay, 12),
        ("svg:darken", BlendMode::Darken, 2),
        ("svg:lighten", BlendMode::Lighten, 7),
        ("svg:color-dodge", BlendMode::ColorDodge, 9),
        ("svg:plus", BlendMode::LinearDodge, 10),
        ("svg:color-burn", BlendMode::ColorBurn, 4),
        ("svg:hard-light", BlendMode::HardLight, 14),
        ("svg:soft-light", BlendMode::SoftLight, 13),
        ("svg:difference", BlendMode::Difference, 19),
        ("svg:hue", BlendMode::Hue, 23),
        ("svg:saturation", BlendMode::Saturation, 24),
        ("svg:color", BlendMode::Color, 25),
        ("svg:luminosity", BlendMode::Luminosity, 26),
    ];
    for (op, mode, disc) in pairs {
        assert_eq!(blend_from_ora_op(op), Some(mode), "{op} import");
        assert_eq!(
            ora_op_from_blend(mode),
            (op, None),
            "{op} export is the exact inverse"
        );
        assert_eq!(mode as u32, disc, "{op} discriminant per §4.5/Appendix A");
    }
}

/// yaml `nearestOpFallbacks`: the 11 Appname-only modes export as the
/// pinned nearest op + canonical `appname:blend`; re-import restores the
/// exact mode via precedence.
#[test]
fn nearest_op_fallbacks_all_eleven() {
    let rows: [(BlendMode, &str, &str); 11] = [
        (BlendMode::Dissolve, "svg:src-over", "dissolve"),
        (BlendMode::LinearBurn, "svg:color-burn", "linearBurn"),
        (BlendMode::DarkerColor, "svg:darken", "darkerColor"),
        (BlendMode::LighterColor, "svg:lighten", "lighterColor"),
        (BlendMode::VividLight, "svg:hard-light", "vividLight"),
        (BlendMode::LinearLight, "svg:hard-light", "linearLight"),
        (BlendMode::PinLight, "svg:hard-light", "pinLight"),
        (BlendMode::HardMix, "svg:hard-light", "hardMix"),
        (BlendMode::Exclusion, "svg:difference", "exclusion"),
        (BlendMode::Subtract, "svg:difference", "subtract"),
        (BlendMode::Divide, "svg:color-dodge", "divide"),
    ];
    for (mode, nearest, canonical) in rows {
        let (op, blend) = ora_op_from_blend(mode);
        assert_eq!(op, nearest, "{canonical} nearest op");
        assert_eq!(
            blend.as_deref(),
            Some(canonical),
            "{canonical} canonical string"
        );
        assert_eq!(
            blend_from_canonical(canonical),
            Some(mode),
            "{canonical} precedence parse"
        );
        assert_eq!(canonical_blend_string(mode), canonical);
    }
    // Every one of the 27 modes maps somewhere: 16 exact + 11 nearest.
    for mode in BlendMode::ALL {
        let _ = ora_op_from_blend(mode);
    }
}

// -------------------------------------------------- foreign-ORA imports ----

/// The golden-fixture checks (yaml `ora_roundtrip.ora`): missing
/// isolation reads as isolate / auto reads as pass-through; unknown
/// composite-op → Normal + CompatItem(Approximated); layer iCCP handling.
#[test]
fn foreign_ora_fixture_checks() {
    let l0 = tiny_png(4, 3, &srgb_bytes(4, 3, true));
    let l1 = tiny_png(2, 2, &srgb_bytes(2, 2, false));
    let merged = tiny_png(4, 3, &srgb_bytes(4, 3, true));
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<image w="4" h="3">
  <stack>
    <stack name="G">
      <layer name="top" src="data/l1.png" x="1" y="1" opacity="0.5" composite-op="krita:weird-op"/>
    </stack>
    <layer name="bottom" src="data/l0.png" visibility="hidden" composite-op="svg:multiply"/>
  </stack>
</image>"#;
    let bytes = foreign_ora(
        xml,
        &[("data/l0.png", l0), ("data/l1.png", l1)],
        Some(merged),
    );
    let reg = CodecRegistry::builtin();
    let imported = ora_codec(&reg)
        .decode(&bytes, &DecodeOpts::default())
        .unwrap();
    assert_eq!(imported.size, (4, 3));
    let layers = imported.layers.as_ref().unwrap();
    // Bottom-to-top: document order in stack.xml is topmost-first.
    assert_eq!(layers.len(), 2);
    assert_eq!(layers[0].name, "bottom");
    assert!(!layers[0].visible);
    assert_eq!(layers[0].blend, BlendMode::Multiply);
    assert_eq!(layers[1].name, "top");
    assert_eq!(
        layers[1].group_path,
        vec!["G".to_string()],
        "nested stack carried"
    );
    assert_eq!(layers[1].offset, appname_core::IVec2::new(1, 1));
    assert!((layers[1].opacity - 0.5).abs() < 1e-6);
    // Unknown composite-op → Normal + CompatItem (Approximated).
    assert_eq!(layers[1].blend, BlendMode::Normal);
    assert!(
        imported
            .report
            .items
            .iter()
            .any(|i| i.feature == "ora.composite_op"
                && matches!(i.action, CompatAction::Approximated))
    );
    // Missing isolation attribute reads as isolate: NO pass-through item.
    assert!(!imported
        .report
        .items
        .iter()
        .any(|i| i.feature == "ora.stack_isolation"));
    // No iCCP in any layer PNG: assumed sRGB, no carriage item.
    assert!(layers.iter().all(|l| l.icc.is_none()));
    assert!(!imported
        .report
        .items
        .iter()
        .any(|i| i.feature == "ora.layer_iccp"));
}

/// Post-election: a foreign ORA whose entries are DEFLATED (the common
/// foreign shape) imports — and imports IDENTICALLY to its Stored twin:
/// same size, same layer pixels, same composite, no extra report items.
#[test]
fn foreign_deflated_entries_import_identically() {
    let l0 = tiny_png(4, 3, &srgb_bytes(4, 3, true));
    let merged = tiny_png(4, 3, &srgb_bytes(4, 3, true));
    let xml = r#"<image w="4" h="3"><stack>
      <layer name="only" src="data/l0.png" composite-op="svg:multiply"/>
    </stack></image>"#;
    let stored = foreign_ora(xml, &[("data/l0.png", l0.clone())], Some(merged.clone()));
    let deflated = foreign_ora_deflated(xml, &[("data/l0.png", l0)], Some(merged));
    assert_ne!(stored, deflated, "twins differ on the wire");
    let reg = CodecRegistry::builtin();
    let a = ora_codec(&reg)
        .decode(&stored, &DecodeOpts::default())
        .unwrap();
    let b = ora_codec(&reg)
        .decode(&deflated, &DecodeOpts::default())
        .unwrap();
    assert_eq!(b.size, (4, 3));
    let (la, lb) = (a.layers.as_ref().unwrap(), b.layers.as_ref().unwrap());
    assert_eq!(lb.len(), 1);
    assert_eq!(lb[0].name, "only");
    assert_eq!(lb[0].blend, BlendMode::Multiply);
    assert_eq!(
        buffer_to_srgb_bytes(&la[0].buffer, 4, 3),
        buffer_to_srgb_bytes(&lb[0].buffer, 4, 3),
        "layer pixels identical across entry compression"
    );
    assert_eq!(
        buffer_to_srgb_bytes(&a.buffer, 4, 3),
        buffer_to_srgb_bytes(&b.buffer, 4, 3),
        "merged surface identical across entry compression"
    );
    assert_eq!(
        a.report.items.len(),
        b.report.items.len(),
        "no compression-induced report drift"
    );
}

#[test]
fn isolation_auto_is_pass_through_carried() {
    let l0 = tiny_png(2, 2, &srgb_bytes(2, 2, true));
    let xml = r#"<image w="2" h="2"><stack><stack name="G" isolation="auto">
      <layer name="in" src="data/l0.png"/></stack></stack></image>"#;
    let bytes = foreign_ora(xml, &[("data/l0.png", l0)], None);
    let reg = CodecRegistry::builtin();
    let imported = ora_codec(&reg)
        .decode(&bytes, &DecodeOpts::default())
        .unwrap();
    assert!(imported
        .report
        .items
        .iter()
        .any(|i| i.feature == "ora.stack_isolation" && matches!(i.action, CompatAction::Info)));
    // mergedimage absent: bottom layer stands in, reported.
    assert!(imported
        .report
        .items
        .iter()
        .any(|i| i.feature == "ora.mergedimage"));
}

/// yaml check: "import honors an iCCP chunk inside a layer PNG" — v1
/// posture: DETECTED + CARRIED raw (registered deviation; honored at the
/// CMM seam).
#[test]
fn layer_png_iccp_is_detected_and_carried() {
    // Splice a fake iCCP chunk (name + method + payload; CRC unchecked by
    // our scanner) after IHDR.
    let base = tiny_png(2, 1, &srgb_bytes(2, 1, true));
    let mut with_iccp = Vec::new();
    let ihdr_end = 8 + 4 + 4 + 13 + 4; // sig + len + "IHDR" + data + crc
    with_iccp.extend_from_slice(&base[..ihdr_end]);
    let payload = b"test-profile\0\0fake-zlib-icc-bytes";
    with_iccp.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    with_iccp.extend_from_slice(b"iCCP");
    with_iccp.extend_from_slice(payload);
    with_iccp.extend_from_slice(&[0u8; 4]); // scanner ignores the CRC
    with_iccp.extend_from_slice(&base[ihdr_end..]);

    let xml = r#"<image w="2" h="1"><stack>
      <layer name="tagged" src="data/l0.png"/></stack></image>"#;
    let bytes = foreign_ora(xml, &[("data/l0.png", with_iccp)], None);
    let reg = CodecRegistry::builtin();
    let imported = ora_codec(&reg)
        .decode(&bytes, &DecodeOpts::default())
        .unwrap();
    let layers = imported.layers.as_ref().unwrap();
    assert_eq!(
        layers[0].icc.as_deref(),
        Some(&payload[..]),
        "raw iCCP payload carried"
    );
    assert!(imported
        .report
        .items
        .iter()
        .any(|i| i.feature == "ora.layer_iccp"));
}

/// yaml `orphaned-extras-dropped` + the matched-entry carriage note.
#[test]
fn appname_extras_orphans_dropped_matches_carried() {
    use std::io::Write;
    let l0 = tiny_png(2, 2, &srgb_bytes(2, 2, true));
    let xml = r#"<image w="2" h="2" xmlns:appname="https://appname.org/ns/ora/1"><stack>
      <layer name="a" src="data/l0.png" appname:layerId="11111111-1111-4111-8111-111111111111"/>
    </stack></image>"#;
    let extra = r#"{ "schema": "appname.oraExtra.v1", "producer": {"app":"Appname","version":"x"},
      "layers": {
        "11111111-1111-4111-8111-111111111111": { "fillOpacity": 0.5 },
        "22222222-2222-4222-8222-222222222222": { "clipToBelow": true }
      } }"#;
    // Build with the extras entry appended.
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut zw = zip::ZipWriter::new(&mut cursor);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zw.start_file("mimetype", opts).unwrap();
        zw.write_all(b"image/openraster").unwrap();
        zw.start_file("stack.xml", opts).unwrap();
        zw.write_all(xml.as_bytes()).unwrap();
        zw.start_file("data/l0.png", opts).unwrap();
        zw.write_all(&l0).unwrap();
        zw.start_file("appname-extra.json", opts).unwrap();
        zw.write_all(extra.as_bytes()).unwrap();
        zw.finish().unwrap();
    }
    let reg = CodecRegistry::builtin();
    let imported = ora_codec(&reg)
        .decode(&cursor.into_inner(), &DecodeOpts::default())
        .unwrap();
    let items: Vec<_> = imported
        .report
        .items
        .iter()
        .filter(|i| i.feature == "ora.appname_extra")
        .collect();
    assert!(
        items
            .iter()
            .any(|i| matches!(i.action, CompatAction::Dropped) && i.detail.contains("22222222")),
        "orphan dropped: {items:?}"
    );
    assert!(
        items
            .iter()
            .any(|i| matches!(i.action, CompatAction::Info) && i.detail.contains("11111111")),
        "match carried: {items:?}"
    );
}

// ------------------------------------------------- export + round trips ----

/// Appname→ORA→Appname round trip through the rich writer: engine doc
/// (journaled imports) → `export_ora_document` → decode → same pixels,
/// names, offsets; entry order per the yaml `exportContract`; mandatory
/// mergedimage; double-encode byte equality (determinism bar).
#[test]
fn document_export_round_trips_and_is_deterministic() {
    // Build a doc with one pasted layer at [0,0] (the journaled path).
    let mut engine =
        appname_core::engine::kinds::register(appname_core::EngineAssembly::new()).build();
    engine
        .apply_json(
            "{\"schema\":\"appname.command.v1\",\"id\":\"c0000000-0000-4000-8000-000000000201\",\
             \"kind\":\"doc.new\",\"params\":{\"width\":5,\"height\":4,\
             \"background\":\"transparent\"}}",
        )
        .unwrap();
    let content = srgb_bytes(5, 4, false);
    let png = appname_io::RgbaPlane::new8(5, 4, content.clone())
        .unwrap()
        .to_png_bytes()
        .unwrap();
    let asset = engine.add_raster_asset(png).unwrap();
    engine
        .apply_json(&format!(
            "{{\"schema\":\"appname.command.v1\",\"id\":\"c0000000-0000-4000-8000-000000000202\",\
             \"kind\":\"edit.paste\",\"params\":{{\"assetId\":\"{asset}\",\"at\":[0,0],\
             \"newLayerId\":\"33333333-3333-4333-8333-333333333333\"}}}}"
        ))
        .unwrap();
    let doc = engine.document().unwrap();

    // The established-path composite: core's reference export math over
    // the same document (single visible raster layer over transparent).
    let composite: Vec<f32> = {
        let ref_png = appname_core::export_png_bytes(&engine).unwrap();
        let (buf, w, h) =
            appname_core::decode_png_to_buffer(&ref_png, appname_core::ProfileId::nil()).unwrap();
        assert_eq!((w, h), (5, 4));
        linear_plane(&buffer_to_srgb_bytes(&buf, 5, 4))
    };

    let a = appname_io::export_ora_document(doc, &composite).unwrap();
    let b = appname_io::export_ora_document(doc, &composite).unwrap();
    assert_eq!(a, b, "double-encode byte equality");

    // Entry order per §8.4 / yaml exportContract (structural: mimetype
    // first, then stack.xml, then ALL data/*.png, then mergedimage,
    // thumbnail, extras).
    let mut za = zip::ZipArchive::new(std::io::Cursor::new(a.as_slice())).unwrap();
    let names: Vec<String> = (0..za.len())
        .map(|i| za.by_index(i).unwrap().name().to_string())
        .collect();
    assert_eq!(names[0], "mimetype", "first entry");
    assert_eq!(names[1], "stack.xml");
    let n = names.len();
    assert!(names[2..n - 3]
        .iter()
        .all(|e| e.starts_with("data/") && e.ends_with(".png")));
    assert_eq!(
        names[n - 3],
        "mergedimage.png",
        "mandatory (yaml mergedimage-mandatory)"
    );
    assert_eq!(names[n - 2], "Thumbnails/thumbnail.png");
    assert_eq!(names[n - 1], "appname-extra.json");

    // Re-import: the pasted layer round-trips by name with its geometry.
    let reg = CodecRegistry::builtin();
    let imported = ora_codec(&reg).decode(&a, &DecodeOpts::default()).unwrap();
    assert_eq!(imported.size, (5, 4));
    let layers = imported.layers.as_ref().unwrap();
    let pasted = layers
        .iter()
        .find(|l| l.name == "Pasted")
        .expect("name round-tripped through stack.xml");
    assert_eq!(pasted.blend, BlendMode::Normal);
    let (lw, lh) = (
        pasted.buffer.bounds().width(),
        pasted.buffer.bounds().height(),
    );
    assert!(lw <= 5 && lh <= 4 && lw > 0 && lh > 0);

    // §8.4 depth rule (yaml sixteen-bit-depth): the working buffer is f32
    // (>8-bit storage), so the layer PNG must be 16-bit.
    let mut layer_png = Vec::new();
    {
        use std::io::Read;
        za.by_name(&names[2])
            .unwrap()
            .read_to_end(&mut layer_png)
            .unwrap();
    }
    assert_eq!(layer_png[24], 16, "IHDR bit depth = 16");
}

/// The flattened trait-writer arm + registry resolution by content.
#[test]
fn flattened_trait_encode_and_sniff_resolution() {
    let bytes = srgb_bytes(4, 4, true);
    let plane = linear_plane(&bytes);
    let meta = Metadata::default();
    let img = appname_io::ExportImage {
        width: 4,
        height: 4,
        plane: &plane,
        layers: None,
        profile: None,
        meta: &meta,
    };
    let reg = CodecRegistry::builtin();
    let opts = EncodeOpts::new(CodecOptions::Ora(OraOptions::default()));
    let a = ora_codec(&reg).encode(&img, &opts).unwrap();
    let b = ora_codec(&reg).encode(&img, &opts).unwrap();
    assert_eq!(a, b, "deterministic");
    // §8.1 magic: 54-byte effective sniff wins over every other claimant.
    let head = &a[..a.len().min(512)];
    assert_eq!(reg.resolve(head, Some("zip")).unwrap().id().0, "ora");
    // Round trip: single "Composite" layer, exact pixels (lossless chain).
    let imported = ora_codec(&reg).decode(&a, &DecodeOpts::default()).unwrap();
    let layers = imported.layers.as_ref().unwrap();
    assert_eq!(layers.len(), 1);
    assert_eq!(layers[0].name, "Composite");
    let round = buffer_to_srgb_bytes(&imported.buffer, 4, 4);
    assert_eq!(round, bytes, "mergedimage carries the exact composite");
}

/// Journaled landing of a layered ORA: offsets ride `edit.paste at`.
#[test]
fn ora_layers_land_journaled_with_offsets() {
    let l0 = tiny_png(2, 2, &srgb_bytes(2, 2, true));
    let l1 = tiny_png(1, 1, &[10, 20, 30, 255]);
    let xml = r#"<image w="4" h="4"><stack>
      <layer name="tiny" src="data/l1.png" x="2" y="3"/>
      <layer name="base" src="data/l0.png"/>
    </stack></image>"#;
    let bytes = foreign_ora(xml, &[("data/l0.png", l0), ("data/l1.png", l1)], None);
    let reg = CodecRegistry::builtin();
    let imported = ora_codec(&reg)
        .decode(&bytes, &DecodeOpts::default())
        .unwrap();

    let mut engine =
        appname_core::engine::kinds::register(appname_core::EngineAssembly::new()).build();
    engine
        .apply_json(
            "{\"schema\":\"appname.command.v1\",\"id\":\"c0000000-0000-4000-8000-000000000301\",\
             \"kind\":\"doc.new\",\"params\":{\"width\":4,\"height\":4}}",
        )
        .unwrap();
    let ids = appname_io::import_into_engine(&mut engine, "ora", imported).unwrap();
    assert_eq!(ids.len(), 2);
    let doc = engine.document().unwrap();
    // stack.xml fidelity through the journaled doors (wave-4 kinds).
    assert_eq!(doc.layer(ids[0]).unwrap().name(), "base");
    assert_eq!(doc.layer(ids[1]).unwrap().name(), "tiny");
    // The offset layer's pixel landed at [2,3].
    let top = doc.layer(ids[1]).unwrap().pixels().unwrap();
    let px = top.read_pixel_f32(appname_core::IVec2::new(2, 3));
    assert!(px[3] > 0.99, "offset paste landed at (2,3): {px:?}");
    // Replay identity (journaled property) holds for ORA imports too.
    let saved = appname_core::save_package_bytes(&engine).unwrap();
    let loaded = appname_core::load_package_bytes_with(engine.assembly(), &saved).unwrap();
    let replayed = loaded
        .engine
        .replay_assembled(loaded.engine.journal())
        .unwrap();
    assert_eq!(replayed.state_hash(), engine.state_hash());
}

// ------------------------------------------------------ malformed rejects ----

#[test]
fn malformed_ora_rejects_typed_never_panics() {
    let reg = CodecRegistry::builtin();
    let codec = ora_codec(&reg);
    let l0 = tiny_png(2, 2, &srgb_bytes(2, 2, true));

    // Wrong mimetype content (sniff bypassed by calling decode directly).
    let wrong = {
        use std::io::Write;
        let mut cursor = std::io::Cursor::new(Vec::new());
        let mut zw = zip::ZipWriter::new(&mut cursor);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zw.start_file("mimetype", opts).unwrap();
        zw.write_all(b"image/zebra").unwrap();
        zw.finish().unwrap();
        cursor.into_inner()
    };
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("not a zip", b"PK\x03\x04 but truncated garbage".to_vec()),
        ("wrong mimetype", wrong),
        ("no stack.xml", foreign_ora("", &[], None).split_off(0)),
        (
            "stack.xml not xml",
            foreign_ora("<image w=\"2\" h=\"2\"><stack>", &[], None),
        ),
        (
            "root not image",
            foreign_ora("<picture w=\"2\" h=\"2\"/>", &[], None),
        ),
        (
            "no dimensions",
            foreign_ora("<image><stack/></image>", &[], None),
        ),
        (
            "no layers",
            foreign_ora("<image w=\"2\" h=\"2\"><stack/></image>", &[], None),
        ),
        (
            "missing layer entry",
            foreign_ora(
                "<image w=\"2\" h=\"2\"><stack><layer name=\"x\" src=\"data/absent.png\"/></stack></image>",
                &[],
                None,
            ),
        ),
        (
            "zip-slip src",
            foreign_ora(
                "<image w=\"2\" h=\"2\"><stack><layer name=\"x\" src=\"../../etc/evil\"/></stack></image>",
                &[],
                None,
            ),
        ),
        (
            "layer entry is not a png",
            foreign_ora(
                "<image w=\"2\" h=\"2\"><stack><layer name=\"x\" src=\"data/l0.png\"/></stack></image>",
                &[("data/l0.png", b"not a png".to_vec())],
                None,
            ),
        ),
        (
            "dtd bomb",
            foreign_ora(
                "<!DOCTYPE image [<!ENTITY a \"bbbb\">]><image w=\"2\" h=\"2\"><stack/></image>",
                &[],
                None,
            ),
        ),
    ];
    for (what, bytes) in cases {
        let err = codec
            .decode(&bytes, &DecodeOpts::default())
            .err()
            .unwrap_or_else(|| panic!("{what}: must reject"));
        let mid = err.message_id();
        assert!(
            mid.starts_with("error.codec.") || mid.starts_with("error.io."),
            "{what}: unexpected mapping {mid}"
        );
    }
    // Bomb gate from stack.xml dimensions, before any layer decode.
    let huge = foreign_ora(
        "<image w=\"999999\" h=\"999999\"><stack><layer name=\"x\" src=\"data/l0.png\"/></stack></image>",
        &[("data/l0.png", l0)],
        None,
    );
    let err = codec.decode(&huge, &DecodeOpts::default()).err().unwrap();
    assert_eq!(err.message_id(), "error.codec.limit_exceeded");
}
