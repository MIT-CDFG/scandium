// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Export-subsystem integration surface (ES §29.1/§29.2/§29.3/§29.4):
//! the io-side W21 export-matrix and W15 slice legs — per-format
//! default presets decode back, profile embed/strip per `ProfilePolicy`
//! (W21 step 1), metadata policy verified by DECODE-BACK (W21 step 2's
//! shape), quick preset, slice matrix + manifest, print-profile PDF
//! (W11 steps 3–4's io-side arm). The full workflow harnesses live
//! with O-QA (`tests/workflows/`); these tests pin the crate surface
//! they will drive.

use appname_color::Cms;
use appname_core::IRect;
use appname_io::export::exif_edit::{emit, Entry, ExifBlock};
use appname_io::export::pdf::{write_raster_pdf, PdfExportOptions};
use appname_io::export::pipeline::encode_variant;
use appname_io::export::preset::{ExportPreset, ProfilePolicy};
use appname_io::export::slices::{export_slices, ManifestStamp, SliceDef, SliceJob};
use appname_io::export::ExportSource;
use appname_io::{CodecRegistry, DecodeOpts, Metadata, MetadataPolicy, SelectiveMeta};

fn gradient_plane(w: u32, h: u32) -> Vec<f32> {
    let mut plane = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            plane.extend_from_slice(&[
                x as f32 / w as f32,
                y as f32 / h as f32,
                ((x + y) % 16) as f32 / 16.0,
                1.0,
            ]);
        }
    }
    plane
}

fn exif_with_gps() -> Vec<u8> {
    emit(&ExifBlock {
        big_endian: false,
        ifd0: vec![Entry {
            tag: 271,
            typ: 2,
            count: 5,
            data: b"Acme\0".to_vec(),
        }],
        exif_ifd: vec![Entry {
            tag: 33434,
            typ: 5,
            count: 1,
            data: vec![1, 0, 0, 0, 250, 0, 0, 0],
        }],
        gps_ifd: vec![Entry {
            tag: 1,
            typ: 2,
            count: 2,
            data: b"N\0".to_vec(),
        }],
        interop_ifd: vec![],
        had_thumbnail: false,
    })
}

fn source<'a>(plane: &'a [f32], meta: &'a Metadata, w: u32, h: u32) -> ExportSource<'a> {
    ExportSource {
        width: w,
        height: h,
        plane,
        icc: None,
        meta,
        doc_name: "matrix",
        ppi: 72.0,
    }
}

/// W21 step 1 (io-side): every §29.1 table format encodes with its
/// default preset and decodes back at the right dimensions.
#[test]
fn w21_shape_per_format_defaults_round_trip() {
    let plane = gradient_plane(48, 32);
    let meta = Metadata::default();
    let src = source(&plane, &meta, 48, 32);
    let mut cms = Cms::new();
    let registry = CodecRegistry::builtin();
    for fmt in ["png", "jpeg", "webp", "tiff", "gif", "bmp"] {
        let preset = ExportPreset::default_for(fmt).unwrap();
        let v = encode_variant(&src, None, &preset, 1.0, &mut cms)
            .unwrap_or_else(|e| panic!("{fmt}: {e}"));
        let head = &v.bytes[..v.bytes.len().min(512)];
        let codec = registry.resolve(head, Some(preset.extension())).unwrap();
        assert_eq!(
            codec.id().0,
            preset.codec_id(),
            "{fmt}: sniff resolves the encoder"
        );
        let imported = codec
            .decode(&v.bytes, &DecodeOpts::default())
            .unwrap_or_else(|e| panic!("{fmt} decode-back: {e}"));
        assert_eq!(imported.size, (48, 32), "{fmt}: dimensions");
    }
}

/// W21 step 1's profile axis: `Embed` re-embeds the document profile,
/// `ConvertToSrgbStrip` omits it (§29.1 stage 3 / §29.4 ICC row).
#[test]
fn profile_embed_and_strip_per_policy() {
    let plane = gradient_plane(24, 24);
    let meta = Metadata::default();
    // a real parseable profile: the synthesized sRGB definition
    let icc = appname_io::export::icc_synth::builtin_icc_bytes(appname_color::BuiltinSpace::Srgb);
    let src = ExportSource {
        icc: Some(&icc),
        ..source(&plane, &meta, 24, 24)
    };
    let mut cms = Cms::new();
    let registry = CodecRegistry::builtin();
    for fmt in ["png", "jpeg"] {
        let mut preset = ExportPreset::default_for(fmt).unwrap();
        preset.profile_policy = ProfilePolicy::Embed;
        let v = encode_variant(&src, None, &preset, 1.0, &mut cms).unwrap();
        let codec = registry.by_id(preset.codec_id()).unwrap();
        let imported = codec.decode(&v.bytes, &DecodeOpts::default()).unwrap();
        assert_eq!(
            imported.profile.as_deref(),
            Some(&icc[..]),
            "{fmt}: Embed carries the document profile byte-wise"
        );

        preset.profile_policy = ProfilePolicy::ConvertToSrgbStrip;
        let v = encode_variant(&src, None, &preset, 1.0, &mut cms).unwrap();
        let imported = codec.decode(&v.bytes, &DecodeOpts::default()).unwrap();
        assert!(
            imported.profile.is_none(),
            "{fmt}: ConvertToSrgbStrip omits the profile (§29.1)"
        );
    }
}

/// W21 step 2's shape: `--metadata strip` ⇒ the output carries no
/// EXIF/XMP; Selective strips GPS while camera EXIF survives — all
/// verified by DECODE-BACK, not by trusting the writer.
#[test]
fn metadata_policy_verified_by_decode_back() {
    let plane = gradient_plane(16, 16);
    let meta = Metadata {
        exif: Some(exif_with_gps()),
        xmp: Some(b"<x:xmpmeta><rdf:RDF></rdf:RDF></x:xmpmeta>".to_vec()),
        ..Metadata::default()
    };
    let src = source(&plane, &meta, 16, 16);
    let mut cms = Cms::new();
    let registry = CodecRegistry::builtin();
    let codec = registry.by_id("jpeg").unwrap();

    // StripAll
    let mut preset = ExportPreset::default_for("jpeg").unwrap();
    preset.metadata_policy = MetadataPolicy::StripAll;
    let v = encode_variant(&src, None, &preset, 1.0, &mut cms).unwrap();
    let imported = codec.decode(&v.bytes, &DecodeOpts::default()).unwrap();
    assert!(imported.meta.exif.is_none(), "StripAll: no EXIF (§29.4)");
    assert!(imported.meta.xmp.is_none(), "StripAll: no XMP (§29.4)");

    // Preserve
    preset.metadata_policy = MetadataPolicy::Preserve;
    let v = encode_variant(&src, None, &preset, 1.0, &mut cms).unwrap();
    let imported = codec.decode(&v.bytes, &DecodeOpts::default()).unwrap();
    let exif = imported.meta.exif.expect("Preserve carries EXIF");
    let parsed = appname_io::export::exif_edit::parse(&exif).unwrap();
    assert!(!parsed.gps_ifd.is_empty(), "Preserve keeps GPS");

    // Selective (dialog defaults: strip_gps on, keep_camera on)
    preset.metadata_policy = MetadataPolicy::Selective(SelectiveMeta::default());
    let v = encode_variant(&src, None, &preset, 1.0, &mut cms).unwrap();
    let imported = codec.decode(&v.bytes, &DecodeOpts::default()).unwrap();
    let exif = imported.meta.exif.expect("Selective keeps the block");
    let parsed = appname_io::export::exif_edit::parse(&exif).unwrap();
    assert!(parsed.gps_ifd.is_empty(), "Selective default: GPS gone");
    assert!(
        parsed.ifd0.iter().any(|e| e.tag == 271),
        "Selective default: camera Make survives"
    );
}

/// The reserved quick preset writes `{name}.png`, sRGB-stripped,
/// metadata-stripped (§29.1 Quick export).
#[test]
fn quick_export_writes_the_normative_defaults() {
    let plane = gradient_plane(20, 10);
    let meta = Metadata {
        exif: Some(exif_with_gps()),
        ..Metadata::default()
    };
    let src = source(&plane, &meta, 20, 10);
    let mut cms = Cms::new();
    let dest = std::env::temp_dir().join(format!("appname-quick-{}", std::process::id()));
    std::fs::create_dir_all(&dest).unwrap();
    let out = appname_io::export::pipeline::export_file(
        &src,
        &ExportPreset::quick(),
        &dest,
        true,
        &mut cms,
    )
    .unwrap();
    assert_eq!(out.len(), 1);
    assert!(out[0].path.ends_with("matrix.png"));
    let registry = CodecRegistry::builtin();
    let codec = registry.by_id("png").unwrap();
    let imported = codec
        .decode(
            &std::fs::read(&out[0].path).unwrap(),
            &DecodeOpts::default(),
        )
        .unwrap();
    assert!(imported.profile.is_none(), "quick: ConvertToSrgbStrip");
    assert!(imported.meta.exif.is_none(), "quick: StripAll");
    std::fs::remove_dir_all(&dest).ok();
}

/// W15's io-side leg at full width: 3 slices × 2 formats × 2 scales =
/// 12 files + a manifest listing every one; pixels equal the
/// flatten-then-crop reference (checked per-slice in the unit suite).
#[test]
fn w15_shape_slice_matrix_with_manifest() {
    let plane = gradient_plane(96, 64);
    let meta = Metadata::default();
    let src = source(&plane, &meta, 96, 64);
    let mut cms = Cms::new();
    let slices = vec![
        SliceDef {
            name: "a".into(),
            bounds: IRect::new(0, 0, 32, 64),
            preset: None,
        },
        SliceDef {
            name: "b".into(),
            bounds: IRect::new(32, 0, 64, 64),
            preset: None,
        },
        SliceDef {
            name: "c".into(),
            bounds: IRect::new(64, 0, 96, 64),
            preset: None,
        },
    ];
    let mut png = ExportPreset::default_for("png").unwrap();
    png.scale_variants = vec![1.0, 2.0];
    let mut webp = ExportPreset::default_for("webp").unwrap();
    webp.scale_variants = vec![1.0, 2.0];
    let dest = std::env::temp_dir().join(format!("appname-w15-{}", std::process::id()));
    std::fs::create_dir_all(&dest).unwrap();
    let stamp = ManifestStamp {
        id: "00000000-0000-4000-8000-00000000bb01".into(),
        generated: "2026-08-17T00:00:00Z".into(),
        document: "matrix.appname".into(),
    };
    let out = export_slices(
        &src,
        &slices,
        &[png, webp],
        &SliceJob {
            template: None,
            dest: &dest,
            overwrite: true,
            stamp,
        },
        &mut cms,
    )
    .unwrap();
    assert_eq!(out.written.len(), 12);
    assert!(out.failed.is_empty());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dest.join("export-manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["schema"], "appname.slicemanifest.v1");
    assert_eq!(manifest["entries"].as_array().unwrap().len(), 12);
    // every listed file exists on disk with the listed byte count
    for e in manifest["entries"].as_array().unwrap() {
        let p = dest.join(e["file"].as_str().unwrap());
        assert_eq!(
            std::fs::metadata(&p).unwrap().len(),
            e["bytes"].as_u64().unwrap(),
            "{}",
            p.display()
        );
    }
    std::fs::remove_dir_all(&dest).ok();
}

/// W11 steps 3–4's io-side arm: print-PDF with a destination profile —
/// pixels convert through the real CMM (lcms2 path: ICC-bytes
/// endpoints) and the PDF carries the printer profile in
/// `/OutputIntents`.
#[test]
fn w11_shape_print_pdf_with_destination_profile() {
    let plane = gradient_plane(40, 20);
    let meta = Metadata::default();
    let src = source(&plane, &meta, 40, 20);
    let mut cms = Cms::new();
    // the "printer/paper" profile: the synthesized ProPhoto definition
    // exercises a REAL profile-bytes destination end to end
    let printer =
        appname_io::export::icc_synth::builtin_icc_bytes(appname_color::BuiltinSpace::ProPhotoRgb);
    let opts = PdfExportOptions {
        print_profile: Some(printer.clone()),
        ..PdfExportOptions::default()
    };
    let pdf = write_raster_pdf(&src, &opts, &mut cms).unwrap();
    let text = String::from_utf8_lossy(&pdf);
    assert!(text.starts_with("%PDF-1.7"));
    assert!(text.contains("/OutputIntents"));
    // the flate-compressed printer profile decompresses byte-identical
    let marker = b"/N 3 /Filter /FlateDecode /Length ";
    let pos = pdf
        .windows(marker.len())
        .position(|w| w == marker)
        .expect("ICC stream dict");
    let stream_start = pdf[pos..]
        .windows(7)
        .position(|w| w == b"stream\n")
        .map(|o| pos + o + 7)
        .unwrap();
    let stream_end = pdf[stream_start..]
        .windows(10)
        .position(|w| w == b"\nendstream")
        .map(|o| stream_start + o)
        .unwrap();
    let mut dec = flate2::read::ZlibDecoder::new(&pdf[stream_start..stream_end]);
    let mut icc_out = Vec::new();
    std::io::Read::read_to_end(&mut dec, &mut icc_out).unwrap();
    assert_eq!(
        icc_out, printer,
        "W11: the destination profile rides the PDF"
    );
    // determinism across runs
    let pdf2 = write_raster_pdf(&src, &opts, &mut cms).unwrap();
    assert_eq!(pdf, pdf2);
}
