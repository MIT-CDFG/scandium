// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §8.5 + Part 42; corpus truth
//! `bogos/tests/interop/psd-corpus.yaml` (READ-ONLY — the executable
//! subset translated per the ORA pattern; the checked-in matrix files do
//! not exist in the spec repo, so fixtures come from the emitter in
//! `tests/psd_common`, the yaml's own byte-level-emitter class). Rows
//! needing unlanded engine constructs (§17 adjustments, §23 text, §11.5
//! smart objects, §11.2 MaskNode, §14 ChannelSet) are covered at their
//! REGISTERED fallback behavior — blocked-honest, with the fallback's
//! report contract pinned here.

mod common;
mod psd_common;

use appname_core::BlendMode;
use appname_io::codecs::psd::blend_from_psd_key;
use appname_io::{Codec, CodecRegistry, DecodeOpts};
use common::{buffer_to_srgb_bytes, srgb_bytes};
use psd_common::{LayerSpec, PsdBuilder};

fn psd_codec(reg: &CodecRegistry) -> &dyn Codec {
    reg.by_id("psd").unwrap()
}

fn items_of<'a>(imported: &'a appname_io::Imported, feature: &str) -> Vec<&'a serde_json::Value> {
    imported
        .compat_report
        .as_ref()
        .unwrap()
        .get("items")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i.get("feature").and_then(|f| f.as_str()) == Some(feature))
        .collect()
}

// ---------------------------------------------------------- H rows ---------

/// H-04 + RS-08-adjacent: flat 8-bit RGB file, raw merged data; pixel
/// round trip vs the emitted planes; report outcome/summary contract.
#[test]
fn flat_rgb8_imports_with_schema_valid_report() {
    let rgb: Vec<u8> = srgb_bytes(5, 4, true)
        .chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    let mut b = PsdBuilder::rgb8(5, 4);
    b.merged_from_rgb8(&rgb, 3);
    let reg = CodecRegistry::builtin();
    let imported = psd_codec(&reg)
        .decode(&b.build(), &DecodeOpts::default())
        .unwrap();
    assert_eq!(imported.size, (5, 4));
    assert!(imported.layers.is_none(), "flattened-only file");
    let round = buffer_to_srgb_bytes(&imported.buffer, 5, 4);
    for (i, px) in round.chunks_exact(4).enumerate() {
        assert_eq!(&px[..3], &rgb[i * 3..i * 3 + 3], "pixel {i}");
        assert_eq!(px[3], 255);
    }
    // §42.4 report contract.
    let rep = imported.compat_report.as_ref().unwrap();
    assert_eq!(rep["schema"], "appname.compatReport.v1");
    assert_eq!(rep["outcome"], "imported");
    assert_eq!(rep["source"]["format"], "psd");
    assert_eq!(rep["source"]["colorMode"], "rgb");
    assert_eq!(rep["source"]["depth"], 8);
    assert_eq!(rep["source"]["size"], serde_json::json!([5, 4]));
    let summary = rep["summary"].as_object().unwrap();
    let items = rep["items"].as_array().unwrap();
    let warnings = items.iter().filter(|i| i["severity"] == "warning").count() as u64;
    assert_eq!(
        summary["warnings"].as_u64().unwrap(),
        warnings,
        "summary tallies items"
    );
    // §42.1: severity derives from action.
    for i in items {
        let expect = if i["action"] == "imported" {
            "info"
        } else {
            "warning"
        };
        assert_eq!(i["severity"], expect, "{i}");
    }
}

/// H-01: Bitmap × 1 — rows unpacked MSB-first, 1 = black, composite only.
#[test]
fn bitmap_1_bit_unpacks_msb_first() {
    let mut b = PsdBuilder::rgb8(10, 2);
    b.mode = 0;
    b.depth = 1;
    b.channels = 1;
    // Row pattern 1100000001 → bytes 0b11000000, 0b01_000000.
    b.merged = vec![vec![0b1100_0000, 0b0100_0000, 0b1100_0000, 0b0100_0000]];
    let reg = CodecRegistry::builtin();
    let imported = psd_codec(&reg)
        .decode(&b.build(), &DecodeOpts::default())
        .unwrap();
    let round = buffer_to_srgb_bytes(&imported.buffer, 10, 2);
    let r = |x: usize, y: usize| round[(y * 10 + x) * 4];
    assert_eq!(r(0, 0), 0, "bit 1 = black");
    assert_eq!(r(1, 0), 0);
    assert_eq!(r(2, 0), 255);
    assert_eq!(r(9, 0), 0, "second byte MSB-first");
    let h01 = items_of(&imported, "header.bitmap");
    assert_eq!(h01.len(), 1);
    assert_eq!(h01[0]["action"], "imported");
    assert_eq!(h01[0]["matrixRow"], "H-01");
}

/// H-03 + RS-05: Indexed × 8 with the 768-byte palette and resource 1047.
#[test]
fn indexed_palette_and_transparency_index() {
    let mut b = PsdBuilder::rgb8(3, 1);
    b.mode = 2;
    b.channels = 1;
    let mut palette = vec![0u8; 768];
    palette[0] = 200; // idx0 R
    palette[256] = 100; // idx0 G
    palette[512] = 50; // idx0 B
    palette[1] = 10;
    palette[257] = 20;
    palette[513] = 30;
    b.color_mode_data = palette;
    b.resources.push((1047, 1u16.to_be_bytes().to_vec()));
    b.merged = vec![vec![0, 1, 0]];
    let reg = CodecRegistry::builtin();
    let imported = psd_codec(&reg)
        .decode(&b.build(), &DecodeOpts::default())
        .unwrap();
    let round = buffer_to_srgb_bytes(&imported.buffer, 3, 1);
    assert_eq!(&round[0..4], &[200, 100, 50, 255]);
    assert_eq!(round[7], 0, "transparency index 1 → alpha 0");
    assert_eq!(&round[8..12], &[200, 100, 50, 255]);
    let item = items_of(&imported, "header.indexed");
    assert_eq!(item[0]["action"], "approximated");
    assert_eq!(item[0]["matrixRow"], "H-03");
}

/// H-05: CMYK × 8 — stored inverted; v → 1 − v then the naive seam
/// conversion (pure C ink at half K → checkable value).
#[test]
fn cmyk_inverts_then_converts() {
    let mut b = PsdBuilder::rgb8(1, 1);
    b.mode = 4;
    b.channels = 4;
    // Stored values (inverted): C=0 (full ink), M=255, Y=255, K=255 (no K).
    b.merged = vec![vec![0], vec![255], vec![255], vec![255]];
    let reg = CodecRegistry::builtin();
    let imported = psd_codec(&reg)
        .decode(&b.build(), &DecodeOpts::default())
        .unwrap();
    let round = buffer_to_srgb_bytes(&imported.buffer, 1, 1);
    assert_eq!(round[0], 0, "full cyan ink kills red");
    assert_eq!(round[1], 255);
    assert_eq!(round[2], 255);
    let item = items_of(&imported, "header.cmyk");
    assert_eq!(
        item[0]["action"], "approximated",
        "CMM-seam conversion is reported"
    );
}

/// H-09 (psd-corpus.yaml expectedReport pin): invalid mode × depth pair
/// rejects `CodecError::Corrupt` carrying the PSD-scoped registry id
/// `error.codec.psd.header_invalid`; the §42.4 headless rejected-report
/// is emittable beside the error. CR-05 precision: OTHER header failures
/// keep their generic §8.1 arms.
#[test]
fn invalid_header_pair_rejects_with_rejected_report() {
    let mut b = PsdBuilder::rgb8(2, 2);
    b.depth = 1; // RGB × 1 is no valid pair
    let bytes = b.build();
    let reg = CodecRegistry::builtin();
    let err = psd_codec(&reg)
        .decode(&bytes, &DecodeOpts::default())
        .err()
        .unwrap();
    assert_eq!(
        err.message_id(),
        "error.codec.psd.header_invalid",
        "H-09 pin"
    );
    let rep = appname_io::rejected_report(&bytes, &err);
    assert_eq!(rep["outcome"], "rejected");
    assert_eq!(rep["error"]["messageId"], "error.codec.psd.header_invalid");

    // Unknown mode VALUE is a pairing violation too (outside the §42.2-A
    // table entirely).
    let mut b = PsdBuilder::rgb8(2, 2);
    b.mode = 5;
    let err = psd_codec(&reg)
        .decode(&b.build(), &DecodeOpts::default())
        .err()
        .unwrap();
    assert_eq!(err.message_id(), "error.codec.psd.header_invalid");

    // CR-05 generic arms STAY generic: bad version → error.codec.corrupt;
    // missing signature → error.codec.unrecognized; gate violations →
    // error.codec.limit_exceeded.
    let mut bytes = PsdBuilder::rgb8(2, 2).build();
    bytes[5] = 9; // version 9
    let err = psd_codec(&reg)
        .decode(&bytes, &DecodeOpts::default())
        .err()
        .unwrap();
    assert_eq!(
        err.message_id(),
        "error.codec.corrupt",
        "version stays generic"
    );
    let mut bytes = PsdBuilder::rgb8(2, 2).build();
    bytes[0] = b'X';
    let err = psd_codec(&reg)
        .decode(&bytes, &DecodeOpts::default())
        .err()
        .unwrap();
    assert_eq!(
        err.message_id(),
        "error.codec.unrecognized",
        "signature stays generic"
    );
    let big = PsdBuilder::rgb8(64, 64).build();
    let tight = DecodeOpts {
        max_pixels: 16,
        ..DecodeOpts::default()
    };
    let err = psd_codec(&reg).decode(&big, &tight).err().unwrap();
    assert_eq!(
        err.message_id(),
        "error.codec.limit_exceeded",
        "gates stay generic"
    );
}

/// The PSD-scoped unsupported id (Appendix H) stays pinned at the
/// taxonomy level: a `psd.`-prefixed `UnsupportedFeature` carries
/// `error.codec.psd.unsupported_feature`; other codecs' arms are
/// untouched. (The ZIP merged-data arm that used to emit it decodes for
/// real since the deflate election — merged code 2 over a NON-zlib
/// payload is now a plain corrupt reject, also pinned here.)
#[test]
fn psd_unsupported_arms_carry_the_scoped_id() {
    let scoped = appname_io::CodecError::UnsupportedFeature {
        feature: "psd.future_feature".into(),
    };
    assert_eq!(scoped.message_id(), "error.codec.psd.unsupported_feature");
    // Precision: a generic (non-PSD) unsupported arm is unchanged.
    let generic = appname_io::CodecError::UnsupportedFeature {
        feature: "webp.lossy_encode".into(),
    };
    assert_eq!(generic.message_id(), "error.codec.unsupported_feature");
    // The un-gated negative arm: merged code 2 whose payload is not a
    // zlib stream rejects as CORRUPT (generic id), not unsupported.
    let rgb: Vec<u8> = srgb_bytes(2, 2, true)
        .chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    let mut b = PsdBuilder::rgb8(2, 2);
    b.merged_from_rgb8(&rgb, 3);
    let mut bytes = b.build();
    // The merged-data compression code is the file's last section: locate
    // the final u16 code (immediately after the empty layer section) and
    // set it to 2 (ZIP) over the raw (non-zlib) planes.
    let code_at = bytes.len() - (2 + 3 * 2 * 2); // code + 3 planes of 2x2
    assert_eq!(
        &bytes[code_at..code_at + 2],
        &[0, 0],
        "emitter wrote raw code"
    );
    bytes[code_at + 1] = 2;
    let reg = CodecRegistry::builtin();
    let err = psd_codec(&reg)
        .decode(&bytes, &DecodeOpts::default())
        .err()
        .unwrap();
    assert_eq!(err.message_id(), "error.codec.corrupt");
    // A genuinely unknown merged compression code stays corrupt too.
    bytes[code_at + 1] = 4;
    let err = psd_codec(&reg)
        .decode(&bytes, &DecodeOpts::default())
        .err()
        .unwrap();
    assert_eq!(err.message_id(), "error.codec.corrupt");
}

// ---------------------------------------------------------- B/L rows -------

/// B-01: all 27 recognized keys map in `BlendMode::ALL` order.
#[test]
fn all_27_blend_keys_map_in_discriminant_order() {
    let keys: [&[u8; 4]; 27] = [
        b"norm", b"diss", b"dark", b"mul ", b"idiv", b"lbrn", b"dkCl", b"lite", b"scrn", b"div ",
        b"lddg", b"lgCl", b"over", b"sLit", b"hLit", b"vLit", b"lLit", b"pLit", b"hMix", b"diff",
        b"smud", b"fsub", b"fdiv", b"hue ", b"sat ", b"colr", b"lum ",
    ];
    for (key, mode) in keys.iter().zip(BlendMode::ALL) {
        assert_eq!(
            blend_from_psd_key(key),
            Some(mode),
            "{}",
            String::from_utf8_lossy(*key)
        );
    }
    assert_eq!(blend_from_psd_key(b"zzzz"), None);
}

/// L-01..L-05 + B-01/B-04 through a real two-layer file.
#[test]
fn layer_records_carry_props_names_and_blends() {
    let rgba = srgb_bytes(3, 2, false);
    let mut bottom = LayerSpec::basic((0, 0, 2, 3), "bottom", &rgba);
    bottom.blend = *b"mul ";
    bottom.opacity = 128;
    bottom
        .keys
        .push((*b"lspf", 0b101u32.to_be_bytes().to_vec())); // transparency+position locks
    bottom.keys.push((*b"iOpa", vec![128]));
    bottom.keys.push((*b"lyid", 42u32.to_be_bytes().to_vec()));
    let mut top = LayerSpec::basic((0, 0, 2, 3), "top", &rgba);
    top.blend = *b"zzzz"; // B-04
    top.flags = 0b10; // hidden
    top.clipping = 1;
    // luni name supersedes the Pascal one.
    let mut luni = 3u32.to_be_bytes().to_vec();
    for u in "Töp".encode_utf16() {
        luni.extend_from_slice(&u.to_be_bytes());
    }
    top.keys.push((*b"luni", luni));

    let mut b = PsdBuilder::rgb8(3, 2);
    b.layers = vec![bottom, top];
    let rgb: Vec<u8> = rgba
        .chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    b.merged_from_rgb8(&rgb, 3);
    let reg = CodecRegistry::builtin();
    let imported = psd_codec(&reg)
        .decode(&b.build(), &DecodeOpts::default())
        .unwrap();
    let layers = imported.layers.as_ref().unwrap();
    assert_eq!(layers.len(), 2, "bottom-to-top");
    let l0 = &layers[0];
    assert_eq!(l0.name, "bottom");
    assert_eq!(l0.blend, BlendMode::Multiply);
    assert!((l0.opacity - 128.0 / 255.0).abs() < 1e-6);
    assert!((l0.fill_opacity - 128.0 / 255.0).abs() < 1e-6);
    assert!(l0.locks.transparency && l0.locks.position && !l0.locks.pixels);
    let l1 = &layers[1];
    assert_eq!(l1.name, "Töp", "luni wins (L-07)");
    assert_eq!(l1.blend, BlendMode::Normal, "unknown key → Normal");
    assert!(!l1.visible, "flags bit 1 (L-04)");
    assert!(l1.clip_to_below, "clipping byte (L-03)");
    let b04 = items_of(&imported, "blend.unknown_key");
    assert_eq!(b04.len(), 1);
    assert_eq!(b04[0]["matrixRow"], "B-04");
    assert_eq!(
        b04[0]["sourceLayerId"],
        serde_json::json!(null),
        "top layer has no lyid"
    );
    // Layer pixels round-trip (alpha included, ±1 for fractional alphas).
    let round = buffer_to_srgb_bytes(&l0.buffer, 3, 2);
    common::assert_bytes_close(&round, &rgba, 1, "layer pixels");
}

/// G-01/B-02: lsct group walk → group_path; pass-through noted. G-02:
/// unbalanced dividers flatten with a report item.
#[test]
fn group_dividers_build_paths_and_malformed_flattens() {
    let rgba = srgb_bytes(2, 2, true);
    let rgb: Vec<u8> = rgba
        .chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    // Bottom-to-top: divider(3) opens, member, folder(1) closes+names.
    let mut opener = LayerSpec::basic((0, 0, 0, 0), "</Layer group>", &[]);
    opener.channels.clear();
    opener.keys.push((*b"lsct", 3u32.to_be_bytes().to_vec()));
    let member = LayerSpec::basic((0, 0, 2, 2), "inside", &rgba);
    let mut folder = LayerSpec::basic((0, 0, 0, 0), "G", &[]);
    folder.channels.clear();
    let mut lsct_pass = 1u32.to_be_bytes().to_vec();
    lsct_pass.extend_from_slice(b"8BIM");
    lsct_pass.extend_from_slice(b"pass");
    folder.keys.push((*b"lsct", lsct_pass));

    let mut b = PsdBuilder::rgb8(2, 2);
    b.layers = vec![opener, member, folder];
    b.merged_from_rgb8(&rgb, 3);
    let reg = CodecRegistry::builtin();
    let imported = psd_codec(&reg)
        .decode(&b.build(), &DecodeOpts::default())
        .unwrap();
    let layers = imported.layers.as_ref().unwrap();
    assert_eq!(layers.len(), 1, "dividers are not pixel layers");
    assert_eq!(layers[0].name, "inside");
    let group_items = items_of(&imported, "group");
    assert_eq!(group_items.len(), 1);
    assert_eq!(
        group_items[0]["matrixRow"], "B-02",
        "pass-through group noted"
    );

    // Unbalanced: opener without folder → G-02 flatten.
    let mut opener2 = LayerSpec::basic((0, 0, 0, 0), "x", &[]);
    opener2.channels.clear();
    opener2.keys.push((*b"lsct", 3u32.to_be_bytes().to_vec()));
    let mut b2 = PsdBuilder::rgb8(2, 2);
    b2.layers = vec![opener2, LayerSpec::basic((0, 0, 2, 2), "flat", &rgba)];
    b2.merged_from_rgb8(&rgb, 3);
    let imported2 = psd_codec(&reg)
        .decode(&b2.build(), &DecodeOpts::default())
        .unwrap();
    let g02 = items_of(&imported2, "layers.groups");
    assert_eq!(g02.len(), 1);
    assert_eq!(g02[0]["matrixRow"], "G-02");
    assert!(
        imported2.layers.as_ref().unwrap()[0].group_path.is_empty(),
        "flattened"
    );
}

// ------------------------------------------------- depth / PSB / RLE -------

/// PS-03: 16-bit records ride the global `Lr16` key; precision survives
/// the 16-bit ingestion door. PS-01: the PSB variant with identical
/// logical content decodes identically.
#[test]
fn lr16_records_and_psb_variant_agree() {
    // 1x1 16-bit gray layer with a precision-bearing value.
    let v: u16 = 0x8081;
    let make = |psb: bool| -> Vec<u8> {
        let mut layer = LayerSpec::basic((0, 0, 1, 1), "deep", &[0, 0, 0, 255]);
        layer.channels = vec![(0, v.to_be_bytes().to_vec(), false)];
        let mut b = PsdBuilder::rgb8(1, 1);
        b.psb = psb;
        b.mode = 1; // gray
        b.depth = 16;
        b.channels = 1;
        b.via_lr16 = true;
        b.layers = vec![layer];
        b.merged = vec![v.to_be_bytes().to_vec()];
        b.build()
    };
    let reg = CodecRegistry::builtin();
    let a = psd_codec(&reg)
        .decode(&make(false), &DecodeOpts::default())
        .unwrap();
    let b = psd_codec(&reg)
        .decode(&make(true), &DecodeOpts::default())
        .unwrap();
    assert_eq!(b.compat_report.as_ref().unwrap()["source"]["format"], "psb");
    let expect = appname_core::io::srgb::srgb_decode(f32::from(v) / 65535.0);
    for imported in [&a, &b] {
        let layer = &imported.layers.as_ref().unwrap()[0];
        let px = layer.buffer.read_pixel_f32(appname_core::IVec2::new(0, 0));
        assert_eq!(
            px[0].to_bits(),
            expect.to_bits(),
            "16-bit precision through Lr16"
        );
    }
    assert_eq!(
        a.layers.as_ref().unwrap()[0].buffer.region_hash(),
        b.layers.as_ref().unwrap()[0].buffer.region_hash(),
        "PS-01: PSD and PSB variants decode identically"
    );
}

/// RLE (PackBits) channel and merged data round-trip exactly.
#[test]
fn rle_channels_and_merged_round_trip() {
    let rgba = srgb_bytes(9, 3, true);
    let rgb: Vec<u8> = rgba
        .chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    let mut layer = LayerSpec::basic((0, 0, 3, 9), "rle", &rgba);
    for ch in &mut layer.channels {
        ch.2 = true; // RLE-compress every channel
    }
    let mut b = PsdBuilder::rgb8(9, 3);
    b.layers = vec![layer];
    b.merged_from_rgb8(&rgb, 3);
    b.merged_rle = true;
    let reg = CodecRegistry::builtin();
    let imported = psd_codec(&reg)
        .decode(&b.build(), &DecodeOpts::default())
        .unwrap();
    let round = buffer_to_srgb_bytes(&imported.layers.as_ref().unwrap()[0].buffer, 9, 3);
    common::assert_bytes_close(&round, &rgba, 0, "RLE layer pixels (opaque = exact)");
    let comp = buffer_to_srgb_bytes(&imported.buffer, 9, 3);
    for (i, px) in comp.chunks_exact(4).enumerate() {
        assert_eq!(&px[..3], &rgb[i * 3..i * 3 + 3], "RLE merged pixel {i}");
    }
}

// ----------------------------------------- fallbacks / CR / keys -----------

/// CR-01: a corrupt channel falls back to the merged composite for the
/// layer rect — fallback layer on top, `bounds` recorded, import still Ok.
#[test]
fn corrupt_channel_falls_back_to_composite() {
    let rgba = srgb_bytes(4, 4, true);
    let rgb: Vec<u8> = rgba
        .chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    let mut bad = LayerSpec::basic((1, 1, 3, 3), "hurt", &srgb_bytes(2, 2, true));
    // Overrunning PackBits row: declare RLE with a run longer than the row.
    bad.channels[1] = (0, Vec::new(), false);
    // Manually corrupt: replace channel 1's raw bytes with a bad RLE block.
    // (Emitter writes code 0 + empty payload → decode_channel hits
    // truncation → typed corrupt → CR-01.)
    let good = LayerSpec::basic((0, 0, 4, 4), "ok", &rgba);
    let mut b = PsdBuilder::rgb8(4, 4);
    b.layers = vec![good, bad];
    b.merged_from_rgb8(&rgb, 3);
    let reg = CodecRegistry::builtin();
    let imported = psd_codec(&reg)
        .decode(&b.build(), &DecodeOpts::default())
        .unwrap();
    let layers = imported.layers.as_ref().unwrap();
    // Live "ok" layer + fallback layer (top).
    assert_eq!(layers.len(), 2);
    assert_eq!(layers[0].name, "ok");
    assert_eq!(layers[1].name, "hurt (composite fallback)");
    assert_eq!(layers[1].offset, appname_core::IVec2::new(1, 1));
    let cr = items_of(&imported, "channel.corrupt");
    assert_eq!(cr.len(), 1);
    assert_eq!(cr[0]["matrixRow"], "CR-01");
    assert_eq!(
        cr[0]["bounds"],
        serde_json::json!({"x": 1, "y": 1, "w": 2, "h": 2})
    );
    // Fallback pixels == composite crop.
    let fb = buffer_to_srgb_bytes(&layers[1].buffer, 2, 2);
    for y in 0..2usize {
        for x in 0..2usize {
            let ci = ((y + 1) * 4 + x + 1) * 3;
            assert_eq!(&fb[(y * 2 + x) * 4..(y * 2 + x) * 4 + 3], &rgb[ci..ci + 3]);
        }
    }
}

/// CR-adjacent (post-election): ZIP channels (code 2) decode for real —
/// pixels land live, byte-equal to the raw-encoding baseline, and NO
/// fallback item is emitted.
#[test]
fn zip_channels_decode_live() {
    let rgba = srgb_bytes(3, 2, true);
    let rgb: Vec<u8> = rgba
        .chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    let mut layer = LayerSpec::basic((0, 0, 2, 3), "zipped", &rgba);
    layer.zip = Some(2);
    let mut b = PsdBuilder::rgb8(3, 2);
    b.layers = vec![layer];
    b.merged_from_rgb8(&rgb, 3);
    let reg = CodecRegistry::builtin();
    let imported = psd_codec(&reg)
        .decode(&b.build(), &DecodeOpts::default())
        .unwrap();
    let layers = imported.layers.as_ref().unwrap();
    assert_eq!(layers.len(), 1, "live layer, no composite fallback");
    assert_eq!(layers[0].name, "zipped");
    let px = buffer_to_srgb_bytes(&layers[0].buffer, 3, 2);
    assert_eq!(px, rgba, "ZIP channel pixels round exact");
    assert!(items_of(&imported, "channel.zip").is_empty());
    assert!(items_of(&imported, "channel.corrupt").is_empty());
}

/// ZIP-with-prediction channels (code 3), 16-bit via Lr16: the §8.5
/// per-row `u16` delta un-predicts bit-exact through the door — every
/// pixel bit-equal to the raw-encoded baseline of the same file.
#[test]
fn zip_prediction_channels_decode_16bit_exact() {
    // Deterministic 16-bit gradient planes (values exercise wrapping
    // deltas across rows).
    let (w, h) = (4usize, 3usize);
    let plane16 = |seed: u16| -> Vec<u8> {
        (0..w * h)
            .flat_map(|i| (seed.wrapping_mul(257).wrapping_add(i as u16 * 5731)).to_be_bytes())
            .collect()
    };
    let make = |zip: Option<u16>| {
        let mut layer = LayerSpec::basic((0, 0, h as i32, w as i32), "deep", &vec![0u8; w * h * 4]);
        layer.channels = vec![
            (-1, plane16(9), false),
            (0, plane16(1), false),
            (1, plane16(2), false),
            (2, plane16(3), false),
        ];
        layer.zip = zip;
        let mut b = PsdBuilder::rgb8(w as u32, h as u32);
        b.depth = 16;
        b.via_lr16 = true;
        b.layers = vec![layer];
        b.merged = (0..3).map(|c| plane16(c as u16 + 1)).collect();
        b.build()
    };
    let reg = CodecRegistry::builtin();
    let raw = psd_codec(&reg)
        .decode(&make(None), &DecodeOpts::default())
        .unwrap();
    let zipped = psd_codec(&reg)
        .decode(&make(Some(3)), &DecodeOpts::default())
        .unwrap();
    let layers = zipped.layers.as_ref().unwrap();
    assert_eq!(layers.len(), 1, "live layer, no composite fallback");
    assert!(items_of(&zipped, "channel.corrupt").is_empty());
    let base = &raw.layers.as_ref().unwrap()[0];
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let at = appname_core::IVec2::new(x, y);
            let a = base.buffer.read_pixel_f32(at);
            let b = layers[0].buffer.read_pixel_f32(at);
            for c in 0..4 {
                assert_eq!(
                    a[c].to_bits(),
                    b[c].to_bits(),
                    "code-3 16-bit bit-exact at ({x},{y}) ch {c}"
                );
            }
        }
    }
}

/// A ZIP channel whose payload is NOT a valid zlib stream follows the
/// CR-01 composite-fallback rule as `channel.corrupt` (the arm the
/// pre-election typed refusal collapsed into).
#[test]
fn zip_channel_with_bad_stream_falls_back_corrupt() {
    let rgba = srgb_bytes(2, 2, true);
    let rgb: Vec<u8> = rgba
        .chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    let mut layer = LayerSpec::basic((0, 0, 2, 2), "zipped", &rgba);
    layer.code_override = Some(2); // code 2 + RAW payload = not a zlib stream
    let mut b = PsdBuilder::rgb8(2, 2);
    b.layers = vec![layer];
    b.merged_from_rgb8(&rgb, 3);
    let reg = CodecRegistry::builtin();
    let imported = psd_codec(&reg)
        .decode(&b.build(), &DecodeOpts::default())
        .unwrap();
    assert!(items_of(&imported, "channel.zip").is_empty());
    let bad = items_of(&imported, "channel.corrupt");
    assert_eq!(bad.len(), 1);
    assert_eq!(bad[0]["action"], "fellBackToComposite");
    assert!(
        bad[0]["note"].as_str().unwrap().contains("zlib"),
        "{}",
        bad[0]
    );
}

/// ZIP merged data (codes 2 and 3): the flattened composite decodes to
/// the same pixels as the raw-encoded baseline.
#[test]
fn zip_merged_data_decodes_both_codes() {
    let rgb: Vec<u8> = srgb_bytes(5, 4, true)
        .chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    let baseline = {
        let mut b = PsdBuilder::rgb8(5, 4);
        b.merged_from_rgb8(&rgb, 3);
        let reg = CodecRegistry::builtin();
        let imported = psd_codec(&reg)
            .decode(&b.build(), &DecodeOpts::default())
            .unwrap();
        buffer_to_srgb_bytes(&imported.buffer, 5, 4)
    };
    for code in [2u16, 3] {
        let mut b = PsdBuilder::rgb8(5, 4);
        b.merged_from_rgb8(&rgb, 3);
        b.merged_zip = Some(code);
        let reg = CodecRegistry::builtin();
        let imported = psd_codec(&reg)
            .decode(&b.build(), &DecodeOpts::default())
            .unwrap();
        assert_eq!(
            imported.compat_report.as_ref().unwrap()["outcome"],
            "imported"
        );
        assert_eq!(
            buffer_to_srgb_bytes(&imported.buffer, 5, 4),
            baseline,
            "merged ZIP code {code} == raw baseline"
        );
    }
}

/// 32-bit merged data under ZIP-with-prediction: the delta-then-deshuffle
/// path reassembles the exact f32 planes (quantized at the 16-bit door
/// like the raw 32-bit path — both routes must agree byte-for-byte).
#[test]
fn zip_prediction_merged_32bit_deshuffles_exact() {
    let (w, h) = (3u32, 2u32);
    // f32 big-endian planes with distinct byte patterns per sample.
    let plane32 = |seed: u32| -> Vec<u8> {
        (0..w * h)
            .flat_map(|i| (((seed + i) as f32) / 8.0).to_be_bytes())
            .collect()
    };
    let build = |zip: Option<u16>| {
        let mut b = PsdBuilder::rgb8(w, h);
        b.depth = 32;
        b.merged = (0..3).map(|c| plane32(c as u32)).collect();
        b.merged_zip = zip;
        b.build()
    };
    let reg = CodecRegistry::builtin();
    let raw = psd_codec(&reg)
        .decode(&build(None), &DecodeOpts::default())
        .unwrap();
    let zipped = psd_codec(&reg)
        .decode(&build(Some(3)), &DecodeOpts::default())
        .unwrap();
    assert_eq!(
        buffer_to_srgb_bytes(&raw.buffer, w, h),
        buffer_to_srgb_bytes(&zipped.buffer, w, h),
        "code-3 32-bit merged == raw 32-bit merged"
    );
}

/// FX-15-shape: an `lfx2` key produces a composite-fallback layer over
/// the inflated rect; TySh reports Rasterized with live pixels kept.
#[test]
fn lfx2_and_tysh_fallback_contract() {
    let rgba = srgb_bytes(2, 2, true);
    let rgb4: Vec<u8> = srgb_bytes(6, 6, true)
        .chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    let mut fx = LayerSpec::basic((2, 2, 4, 4), "styled", &rgba);
    fx.keys.push((*b"lfx2", vec![0u8; 8]));
    let mut text = LayerSpec::basic((0, 0, 2, 2), "caption", &rgba);
    text.keys.push((*b"TySh", vec![0u8; 8]));
    let mut b = PsdBuilder::rgb8(6, 6);
    b.layers = vec![fx, text];
    b.merged_from_rgb8(&rgb4, 3);
    let reg = CodecRegistry::builtin();
    let imported = psd_codec(&reg)
        .decode(&b.build(), &DecodeOpts::default())
        .unwrap();
    let layers = imported.layers.as_ref().unwrap();
    // 2 live layers + 1 fallback on top.
    assert_eq!(layers.len(), 3);
    assert!(layers[2].name.ends_with("(composite fallback)"));
    let fx15 = items_of(&imported, "effect.lfx2");
    assert_eq!(fx15.len(), 1);
    assert_eq!(fx15[0]["action"], "fellBackToComposite");
    let bounds = fx15[0]["bounds"].as_object().unwrap();
    assert!(bounds["w"].as_u64().unwrap() > 2, "inflated rect");
    let tx = items_of(&imported, "text");
    assert_eq!(tx[0]["action"], "rasterized");
    assert_eq!(
        layers[1].name, "caption",
        "type layer keeps its stored raster"
    );
}

/// AD-17-shape: an adjustment key with no pixels yields the canvas-scope
/// fallback and no live layer.
#[test]
fn adjustment_key_falls_back_over_canvas() {
    let rgb: Vec<u8> = srgb_bytes(3, 3, true)
        .chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    let mut adj = LayerSpec::basic((0, 0, 0, 0), "Curves 1", &[]);
    adj.channels.clear();
    adj.keys.push((*b"curv", vec![0u8; 4]));
    let mut b = PsdBuilder::rgb8(3, 3);
    b.layers = vec![
        LayerSpec::basic((0, 0, 3, 3), "base", &srgb_bytes(3, 3, true)),
        adj,
    ];
    b.merged_from_rgb8(&rgb, 3);
    let reg = CodecRegistry::builtin();
    let imported = psd_codec(&reg)
        .decode(&b.build(), &DecodeOpts::default())
        .unwrap();
    let layers = imported.layers.as_ref().unwrap();
    assert_eq!(
        layers.len(),
        2,
        "base + canvas fallback (no live adjustment node)"
    );
    assert_eq!(layers[1].name, "Curves 1 (composite fallback)");
    let ad = items_of(&imported, "adjustment");
    assert_eq!(ad.len(), 1);
    assert_eq!(ad[0]["matrixRow"], "AD-17");
    assert_eq!(
        ad[0]["bounds"],
        serde_json::json!({"x": 0, "y": 0, "w": 3, "h": 3})
    );
}

/// MK-01 at the registered v1 fallback: the −2 raster mask multiplies
/// into alpha, reported Rasterized.
#[test]
fn raster_mask_rasterizes_into_alpha() {
    let rgba = srgb_bytes(2, 1, true);
    let mut layer = LayerSpec::basic((0, 0, 1, 2), "masked", &rgba);
    // Mask rect covers only x=0; default color 0 hides the rest.
    layer.mask = Some(((0, 0, 1, 1), 0, 0));
    layer.channels.push((-2, vec![255], false));
    let mut b = PsdBuilder::rgb8(2, 1);
    b.layers = vec![layer];
    let rgb: Vec<u8> = rgba
        .chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    b.merged_from_rgb8(&rgb, 3);
    let reg = CodecRegistry::builtin();
    let imported = psd_codec(&reg)
        .decode(&b.build(), &DecodeOpts::default())
        .unwrap();
    let layer = &imported.layers.as_ref().unwrap()[0];
    let round = buffer_to_srgb_bytes(&layer.buffer, 2, 1);
    assert_eq!(round[3], 255, "inside the mask rect: opaque");
    assert_eq!(round[7], 0, "outside: default color 0 hides");
    let mk = items_of(&imported, "mask.raster");
    assert_eq!(mk[0]["action"], "rasterized");
    assert_eq!(mk[0]["matrixRow"], "MK-01");
}

// ------------------------------------------------- resources / meta --------

/// RS rows: 1039 ICC → profile; 1058/1060 → meta; 1064 ≠ 1 → item;
/// unknown ids → skippedResources.
#[test]
fn resources_route_to_profile_meta_and_skips() {
    let rgb: Vec<u8> = srgb_bytes(2, 2, true)
        .chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    let mut b = PsdBuilder::rgb8(2, 2);
    b.merged_from_rgb8(&rgb, 3);
    b.resources.push((1039, b"fake-icc-profile-bytes".to_vec()));
    b.resources.push((
        1058,
        appname_io::meta::exif_block_with_orientation(6, false),
    ));
    b.resources.push((1060, b"<x:xmpmeta/>".to_vec()));
    let mut aspect = 1u32.to_be_bytes().to_vec();
    aspect.extend_from_slice(&2.0f64.to_be_bytes());
    b.resources.push((1064, aspect));
    b.resources.push((4242, vec![0u8; 10]));
    let reg = CodecRegistry::builtin();
    let imported = psd_codec(&reg)
        .decode(&b.build(), &DecodeOpts::default())
        .unwrap();
    assert_eq!(
        imported.profile.as_deref(),
        Some(&b"fake-icc-profile-bytes"[..])
    );
    assert!(imported.meta.exif.is_some(), "1058 carried");
    assert_eq!(imported.meta.xmp.as_deref(), Some(&b"<x:xmpmeta/>"[..]));
    assert_eq!(
        imported.orientation_applied, None,
        "PSD pixels are upright; EXIF carried, not baked"
    );
    let ar = items_of(&imported, "resource.pixel_aspect_ratio");
    assert_eq!(ar[0]["action"], "approximated");
    let skipped = imported.compat_report.as_ref().unwrap()["skippedResources"]
        .as_array()
        .unwrap()
        .clone();
    assert!(
        skipped.iter().any(|s| s["id"] == 4242 && s["bytes"] == 10),
        "unknown resource skipped by length: {skipped:?}"
    );
}

/// Negative layer count: the first extra merged channel is the composite
/// alpha.
#[test]
fn negative_layer_count_reads_merged_alpha() {
    let rgba = srgb_bytes(2, 1, true);
    let mut b = PsdBuilder::rgb8(2, 1);
    b.channels = 4;
    b.merged_has_alpha = true;
    b.layers = vec![LayerSpec::basic((0, 0, 1, 2), "l", &rgba)];
    let rgb: Vec<u8> = rgba
        .chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    b.merged_from_rgb8(&rgb, 3);
    b.merged.push(vec![255, 0]); // alpha plane: right pixel transparent
    let reg = CodecRegistry::builtin();
    let imported = psd_codec(&reg)
        .decode(&b.build(), &DecodeOpts::default())
        .unwrap();
    let round = buffer_to_srgb_bytes(&imported.buffer, 2, 1);
    assert_eq!(round[3], 255);
    assert_eq!(round[7], 0, "merged alpha honored");
}

// ------------------------------------------------- engine landing ----------

/// The journaled landing carries PSD fidelity end-to-end: names, blend,
/// opacity, fill opacity, locks, clip — and replay identity holds.
#[test]
fn psd_layers_land_journaled_with_full_props() {
    let rgba = srgb_bytes(3, 3, true);
    let mut styled = LayerSpec::basic((0, 0, 3, 3), "styled", &rgba);
    styled.blend = *b"scrn";
    styled.opacity = 191;
    styled.keys.push((*b"iOpa", vec![64]));
    styled
        .keys
        .push((*b"lspf", 0b010u32.to_be_bytes().to_vec()));
    let mut clipped = LayerSpec::basic((0, 0, 3, 3), "clipped", &rgba);
    clipped.clipping = 1;
    let mut b = PsdBuilder::rgb8(3, 3);
    b.layers = vec![styled, clipped];
    let rgb: Vec<u8> = rgba
        .chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    b.merged_from_rgb8(&rgb, 3);
    let reg = CodecRegistry::builtin();
    let imported = psd_codec(&reg)
        .decode(&b.build(), &DecodeOpts::default())
        .unwrap();

    let mut engine =
        appname_core::engine::kinds::register(appname_core::EngineAssembly::new()).build();
    engine
        .apply_json(
            "{\"schema\":\"appname.command.v1\",\"id\":\"c0000000-0000-4000-8000-000000000401\",\
             \"kind\":\"doc.new\",\"params\":{\"width\":3,\"height\":3,\
             \"background\":\"transparent\"}}",
        )
        .unwrap();
    let ids = appname_io::import_into_engine(&mut engine, "psd", imported).unwrap();
    assert_eq!(ids.len(), 2);
    let doc = engine.document().unwrap();
    let styled = doc.layer(ids[0]).unwrap();
    assert_eq!(styled.name(), "styled");
    assert_eq!(styled.blend(), BlendMode::Screen);
    assert!((styled.opacity() - 191.0 / 255.0).abs() < 1e-6);
    assert!((styled.fill_opacity() - 64.0 / 255.0).abs() < 1e-6);
    let clipped = doc.layer(ids[1]).unwrap();
    assert_eq!(clipped.name(), "clipped");
    assert!(clipped.clip_to_below());
    // Replay identity through the PSD path.
    let saved = appname_core::save_package_bytes(&engine).unwrap();
    let loaded = appname_core::load_package_bytes_with(engine.assembly(), &saved).unwrap();
    let replayed = loaded
        .engine
        .replay_assembled(loaded.engine.journal())
        .unwrap();
    assert_eq!(replayed.state_hash(), engine.state_hash());
}
