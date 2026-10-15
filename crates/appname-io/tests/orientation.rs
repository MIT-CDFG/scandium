// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §8.6 [NORMATIVE] — orientation applied on import per the
//! transform table, stored tag reset to 1, values outside 1–8 treated as 1
//! with a CompatItem; plan §2.3 "EXIF orientation … value-captured (the
//! applied orientation recorded so replay is deterministic); orientation
//! test vectors". End-to-end through real carriers: JPEG APP1 and the TIFF
//! IFD tag (the plane-level 8-value matrix is pinned in
//! `src/plane.rs::tests`).

mod common;

use appname_io::codecs::jpeg::splice_app1_exif;
use appname_io::codecs::tiff_io::test_fixture_tiff_with_orientation;
use appname_io::meta::exif_block_with_orientation;
use appname_io::{
    CodecOptions, CodecRegistry, CompatAction, DecodeOpts, EncodeOpts, JpegOptions, Metadata,
};
use common::{buffer_to_srgb_bytes, linear_plane, srgb_bytes};

/// A 4x2 asymmetric JPEG (quality 100 keeps corners distinguishable).
fn base_jpeg() -> (Vec<u8>, Vec<u8>) {
    let bytes = srgb_bytes(4, 2, true);
    let plane = linear_plane(&bytes);
    let meta = Metadata::default();
    let img = appname_io::ExportImage {
        width: 4,
        height: 2,
        plane: &plane,
        layers: None,
        profile: None,
        meta: &meta,
    };
    let opts = EncodeOpts::new(CodecOptions::Jpeg(JpegOptions {
        quality: 100,
        ..JpegOptions::default()
    }));
    let registry = CodecRegistry::builtin();
    let encoded = registry.by_id("jpeg").unwrap().encode(&img, &opts).unwrap();
    (encoded, bytes)
}

#[test]
fn jpeg_exif_orientation_matrix_bakes_and_value_captures() {
    let (jpeg, _) = base_jpeg();
    let registry = CodecRegistry::builtin();
    let codec = registry.by_id("jpeg").unwrap();
    // Reference upright pixels (decode without any EXIF).
    let upright = codec.decode(&jpeg, &DecodeOpts::default()).unwrap();
    let reference = buffer_to_srgb_bytes(&upright.buffer, 4, 2);
    for v in 1..=8u16 {
        let with_exif =
            splice_app1_exif(&jpeg, &exif_block_with_orientation(v, v % 2 == 0)).unwrap();
        let imported = codec.decode(&with_exif, &DecodeOpts::default()).unwrap();
        // Value capture (D-48: replay determinism for the future
        // journaled-import envelope).
        assert_eq!(imported.orientation_applied, Some(v), "v={v}");
        // Dimensions per the table: 5–8 swap.
        let expect_size = if v >= 5 { (2, 4) } else { (4, 2) };
        assert_eq!(imported.size, expect_size, "v={v}");
        // The §8.6 normalization: the CARRIED block re-reads as 1.
        let carried = imported.meta.exif.as_ref().expect("block carried");
        assert_eq!(
            appname_io::meta::orientation_from_exif(carried),
            Some(1),
            "v={v}"
        );
        // Baked pixels equal the plane-level transform of the upright
        // reference (the matrix itself is pinned at the unit level).
        let plane = appname_io::RgbaPlane::new8(4, 2, reference.clone()).unwrap();
        let expected = plane.apply_orientation(v).unwrap();
        let got = buffer_to_srgb_bytes(&imported.buffer, imported.size.0, imported.size.1);
        let expected_bytes = match expected.pixels {
            appname_io::plane::PlanePixels::B8(b) => b,
            _ => unreachable!(),
        };
        assert_eq!(got, expected_bytes, "v={v} baked pixels");
    }
}

#[test]
fn jpeg_orientation_bake_can_be_disabled() {
    let (jpeg, _) = base_jpeg();
    let with_exif = splice_app1_exif(&jpeg, &exif_block_with_orientation(6, false)).unwrap();
    let registry = CodecRegistry::builtin();
    let imported = registry
        .by_id("jpeg")
        .unwrap()
        .decode(
            &with_exif,
            &DecodeOpts {
                apply_orientation: false,
                ..DecodeOpts::default()
            },
        )
        .unwrap();
    assert_eq!(imported.size, (4, 2), "not baked");
    assert_eq!(imported.orientation_applied, None);
    // The carried block keeps its original value when nothing was baked.
    let carried = imported.meta.exif.as_ref().unwrap();
    assert_eq!(appname_io::meta::orientation_from_exif(carried), Some(6));
}

#[test]
fn out_of_range_orientation_is_treated_as_1_with_a_compat_item() {
    let (jpeg, _) = base_jpeg();
    let with_exif = splice_app1_exif(&jpeg, &exif_block_with_orientation(9, false)).unwrap();
    let registry = CodecRegistry::builtin();
    let imported = registry
        .by_id("jpeg")
        .unwrap()
        .decode(&with_exif, &DecodeOpts::default())
        .unwrap();
    assert_eq!(imported.size, (4, 2));
    assert_eq!(
        imported.orientation_applied,
        Some(1),
        "treated as identity, value-captured"
    );
    assert!(
        imported
            .report
            .items
            .iter()
            .any(|i| i.feature == "metadata.exif.orientation"
                && matches!(i.action, CompatAction::Approximated)),
        "CompatItem recorded: {:?}",
        imported.report.items
    );
}

#[test]
fn tiff_ifd_orientation_bakes_and_value_captures() {
    let rgba = srgb_bytes(4, 2, true);
    let bytes = test_fixture_tiff_with_orientation(4, 2, &rgba, 6);
    let registry = CodecRegistry::builtin();
    let imported = registry
        .by_id("tiff")
        .unwrap()
        .decode(&bytes, &DecodeOpts::default())
        .unwrap();
    assert_eq!(imported.size, (2, 4), "rot90 swaps");
    assert_eq!(imported.orientation_applied, Some(6));
    // Pixel spot check: source (0,0) lands at (H-1-0, 0) = (1, 0).
    let got = buffer_to_srgb_bytes(&imported.buffer, 2, 4);
    assert_eq!(&got[4..8], &rgba[0..4]);
}

#[test]
fn exported_jpeg_reembeds_the_normalized_block() {
    // decode(spliced v=6) -> encode -> decode: the re-embedded block must
    // read orientation 1 and the pixels must stay upright (no double
    // rotation).
    let (jpeg, _) = base_jpeg();
    let with_exif = splice_app1_exif(&jpeg, &exif_block_with_orientation(6, false)).unwrap();
    let registry = CodecRegistry::builtin();
    let codec = registry.by_id("jpeg").unwrap();
    let first = codec.decode(&with_exif, &DecodeOpts::default()).unwrap();
    assert_eq!(first.size, (2, 4));
    let plane: Vec<f32> = {
        // Rebuild the linear plane from the baked buffer.
        let srgb = buffer_to_srgb_bytes(&first.buffer, 2, 4);
        linear_plane(&srgb)
    };
    let img = appname_io::ExportImage {
        width: 2,
        height: 4,
        plane: &plane,
        layers: None,
        profile: None,
        meta: &first.meta,
    };
    let opts = EncodeOpts::new(CodecOptions::Jpeg(JpegOptions::default()));
    let reencoded = codec.encode(&img, &opts).unwrap();
    let second = codec.decode(&reencoded, &DecodeOpts::default()).unwrap();
    assert_eq!(second.size, (2, 4), "no double rotation");
    assert_eq!(
        second.orientation_applied,
        Some(1),
        "normalized tag round-tripped"
    );
}
