// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §8.1–§8.2, §8.6; plan §2.3 "Codecs v2" test bar — per-format
//! round-trips (fixed fixtures, byte-stable where the format permits;
//! lossy formats pin decode-of-encode hashes), profile carriage, frame ↔
//! layer mapping, encoder determinism.
//!
//! NOTE on interop corpora: `bogos/tests/interop/*.yaml` carries ORA/PSD/
//! RAW/migration fixtures only — none cover JPEG/TIFF/WebP/GIF/BMP, so
//! slice-1 fixtures are self-authored deterministic builders
//! (`tests/common`).

mod common;

use appname_io::{
    BmpOptions, CodecOptions, CodecRegistry, DecodeOpts, EncodeOpts, ExportImage, ExportLayer,
    GifOptions, JpegOptions, Metadata, TiffOptions, WebpOptions,
};
use common::{assert_bytes_close, buffer_to_srgb_bytes, linear_plane, srgb_bytes};

fn export_image<'a>(
    w: u32,
    h: u32,
    plane: &'a [f32],
    profile: Option<&'a [u8]>,
    meta: &'a Metadata,
) -> ExportImage<'a> {
    ExportImage {
        width: w,
        height: h,
        plane,
        layers: None,
        profile,
        meta,
    }
}

fn reg() -> CodecRegistry {
    CodecRegistry::builtin()
}

// ---------------------------------------------------------------- JPEG ----

#[test]
fn jpeg_encode_is_deterministic_and_decode_of_encode_is_pinned() {
    let bytes = srgb_bytes(16, 8, true);
    let plane = linear_plane(&bytes);
    let meta = Metadata::default();
    let img = export_image(16, 8, &plane, None, &meta);
    let opts = EncodeOpts::new(CodecOptions::Jpeg(JpegOptions::default()));
    let codec = reg();
    let codec = codec.by_id("jpeg").unwrap();
    let a = codec.encode(&img, &opts).unwrap();
    let b = codec.encode(&img, &opts).unwrap();
    assert_eq!(a, b, "identical input -> identical bytes");
    // Lossy format: pin the decode-of-encode content hash (plan §2.3).
    let imported = codec.decode(&a, &DecodeOpts::default()).unwrap();
    assert_eq!(imported.size, (16, 8));
    let round = buffer_to_srgb_bytes(&imported.buffer, 16, 8);
    let hash = appname_core::ContentHash::of_bytes(&round).to_hex();
    assert_eq!(
        hash,
        // Pinned from the first recorded run (lane log, wave 1b); a drift
        // means the elected decoder/encoder pair changed behavior.
        "f55eea7ba978bd1f6de9b8fe2ebc23c6e6b3963775e682da9c1b80704cda1225",
        "decode-of-encode drifted: pinned bytes changed"
    );
}

#[test]
fn jpeg_carries_icc_profile_both_ways() {
    let bytes = srgb_bytes(8, 8, true);
    let plane = linear_plane(&bytes);
    let profile = b"appname-test-icc-payload".to_vec();
    let meta = Metadata::default();
    let img = export_image(8, 8, &plane, Some(&profile), &meta);
    let opts = EncodeOpts::new(CodecOptions::Jpeg(JpegOptions::default()));
    let codec = reg();
    let codec = codec.by_id("jpeg").unwrap();
    let encoded = codec.encode(&img, &opts).unwrap();
    let imported = codec.decode(&encoded, &DecodeOpts::default()).unwrap();
    assert_eq!(
        imported.profile.as_deref(),
        Some(profile.as_slice()),
        "profile CARRIED"
    );
    assert_eq!(imported.meta.icc.as_deref(), Some(profile.as_slice()));
}

#[test]
fn jpeg_composites_alpha_over_the_matte() {
    // A fully transparent plane over the default white matte encodes white.
    let plane = vec![0.0f32; 4 * 4 * 4];
    let meta = Metadata::default();
    let img = export_image(4, 4, &plane, None, &meta);
    let opts = EncodeOpts::new(CodecOptions::Jpeg(JpegOptions {
        quality: 100,
        ..JpegOptions::default()
    }));
    let codec = reg();
    let codec = codec.by_id("jpeg").unwrap();
    let encoded = codec.encode(&img, &opts).unwrap();
    let imported = codec.decode(&encoded, &DecodeOpts::default()).unwrap();
    let round = buffer_to_srgb_bytes(&imported.buffer, 4, 4);
    for px in round.chunks_exact(4) {
        assert!(
            px[0] >= 250 && px[1] >= 250 && px[2] >= 250,
            "matte white, got {px:?}"
        );
        assert_eq!(px[3], 255);
    }
}

// ---------------------------------------------------------------- TIFF ----

#[test]
fn tiff_8_bit_round_trip_is_pixel_exact_for_opaque_and_close_for_alpha() {
    let codec = reg();
    let codec = codec.by_id("tiff").unwrap();
    for (opaque, tol) in [(true, 0u8), (false, 1u8)] {
        let bytes = srgb_bytes(9, 5, opaque);
        let plane = linear_plane(&bytes);
        let meta = Metadata::default();
        let img = export_image(9, 5, &plane, None, &meta);
        let opts = EncodeOpts::new(CodecOptions::Tiff(TiffOptions::default()));
        let encoded = codec.encode(&img, &opts).unwrap();
        let imported = codec.decode(&encoded, &DecodeOpts::default()).unwrap();
        assert_eq!(imported.size, (9, 5));
        let round = buffer_to_srgb_bytes(&imported.buffer, 9, 5);
        assert_bytes_close(&round, &bytes, tol, &format!("tiff opaque={opaque}"));
    }
}

#[test]
fn tiff_compression_variants_round_trip_and_are_deterministic() {
    let bytes = srgb_bytes(33, 7, true);
    let plane = linear_plane(&bytes);
    let meta = Metadata::default();
    let codec = reg();
    let codec = codec.by_id("tiff").unwrap();
    for compression in [
        appname_io::TiffCompression::None,
        appname_io::TiffCompression::Lzw,
        appname_io::TiffCompression::Zip,
    ] {
        let img = export_image(33, 7, &plane, None, &meta);
        let opts = EncodeOpts::new(CodecOptions::Tiff(TiffOptions {
            compression,
            ..TiffOptions::default()
        }));
        let a = codec.encode(&img, &opts).unwrap();
        let b = codec.encode(&img, &opts).unwrap();
        assert_eq!(a, b, "{compression:?} deterministic");
        let imported = codec.decode(&a, &DecodeOpts::default()).unwrap();
        let round = buffer_to_srgb_bytes(&imported.buffer, 33, 7);
        assert_bytes_close(&round, &bytes, 0, &format!("tiff {compression:?}"));
    }
}

#[test]
fn tiff_carries_icc_profile_both_ways() {
    let bytes = srgb_bytes(6, 6, true);
    let plane = linear_plane(&bytes);
    let profile = b"tiff-icc-carriage-fixture".to_vec();
    let meta = Metadata::default();
    let img = export_image(6, 6, &plane, Some(&profile), &meta);
    let opts = EncodeOpts::new(CodecOptions::Tiff(TiffOptions::default()));
    let codec = reg();
    let codec = codec.by_id("tiff").unwrap();
    let encoded = codec.encode(&img, &opts).unwrap();
    assert_eq!(
        appname_io::codecs::tiff_io::tiff_icc(&encoded).as_deref(),
        Some(profile.as_slice()),
        "tag 34675 written"
    );
    let imported = codec.decode(&encoded, &DecodeOpts::default()).unwrap();
    assert_eq!(imported.profile.as_deref(), Some(profile.as_slice()));
}

#[test]
fn tiff_16_bit_import_preserves_precision_through_the_door() {
    use tiff::encoder::{colortype, TiffEncoder};
    // 1x1 RGB16 with a value an 8-bit hop would flatten.
    let mut out = std::io::Cursor::new(Vec::new());
    {
        let mut enc = TiffEncoder::new(&mut out).unwrap();
        enc.write_image::<colortype::RGB16>(1, 1, &[0x8081u16, 0, 0xFFFF])
            .unwrap();
    }
    let codec = reg();
    let codec = codec.by_id("tiff").unwrap();
    let imported = codec
        .decode(&out.into_inner(), &DecodeOpts::default())
        .unwrap();
    let px = imported
        .buffer
        .read_pixel_f32(appname_core::IVec2::new(0, 0));
    let expect = appname_core::io::srgb::srgb_decode(0x8081 as f32 / 65535.0);
    assert_eq!(
        px[0].to_bits(),
        expect.to_bits(),
        "16-bit precision survived"
    );
}

#[test]
fn tiff_multi_page_imports_page_n_layers() {
    use tiff::encoder::{colortype, TiffEncoder};
    let mut out = std::io::Cursor::new(Vec::new());
    {
        let mut enc = TiffEncoder::new(&mut out).unwrap();
        enc.write_image::<colortype::RGBA8>(2, 2, &srgb_bytes(2, 2, true))
            .unwrap();
        enc.write_image::<colortype::RGBA8>(3, 1, &srgb_bytes(3, 1, true))
            .unwrap();
    }
    let bytes = out.into_inner();
    let codec = reg();
    let codec = codec.by_id("tiff").unwrap();
    let imported = codec.decode(&bytes, &DecodeOpts::default()).unwrap();
    let layers = imported.layers.expect("multi-page -> layers");
    assert_eq!(layers.len(), 2);
    assert_eq!(layers[0].name, "page:0");
    assert_eq!(layers[1].name, "page:1");
    assert_eq!(imported.size, (2, 2), "primary is page 0");
    // frames_as_layers = false -> first page only.
    let first_only = codec
        .decode(
            &bytes,
            &DecodeOpts {
                frames_as_layers: false,
                ..DecodeOpts::default()
            },
        )
        .unwrap();
    assert!(first_only.layers.is_none());
}

#[test]
fn tiff_cmyk_answers_model_unsupported() {
    use tiff::encoder::{colortype, TiffEncoder};
    let mut out = std::io::Cursor::new(Vec::new());
    {
        let mut enc = TiffEncoder::new(&mut out).unwrap();
        enc.write_image::<colortype::CMYK8>(1, 1, &[0u8, 0, 0, 255])
            .unwrap();
    }
    let codec = reg();
    let codec = codec.by_id("tiff").unwrap();
    let err = codec
        .decode(&out.into_inner(), &DecodeOpts::default())
        .err()
        .unwrap();
    assert_eq!(err.message_id(), "error.codec.model");
}

// ---------------------------------------------------------------- WebP ----

#[test]
fn webp_lossless_round_trip_is_pixel_exact_for_opaque_and_close_for_alpha() {
    let codec = reg();
    let codec = codec.by_id("webp").unwrap();
    for (opaque, tol) in [(true, 0u8), (false, 1u8)] {
        let bytes = srgb_bytes(11, 4, opaque);
        let plane = linear_plane(&bytes);
        let meta = Metadata::default();
        let img = export_image(11, 4, &plane, None, &meta);
        let opts = EncodeOpts::new(CodecOptions::Webp(WebpOptions::default()));
        let a = codec.encode(&img, &opts).unwrap();
        let b = codec.encode(&img, &opts).unwrap();
        assert_eq!(a, b, "deterministic");
        let imported = codec.decode(&a, &DecodeOpts::default()).unwrap();
        assert_eq!(imported.size, (11, 4));
        let round = buffer_to_srgb_bytes(&imported.buffer, 11, 4);
        assert_bytes_close(&round, &bytes, tol, &format!("webp opaque={opaque}"));
    }
}

#[test]
fn webp_lossy_encode_answers_typed_unsupported() {
    let plane = vec![0.0f32; 4];
    let meta = Metadata::default();
    let img = export_image(1, 1, &plane, None, &meta);
    let opts = EncodeOpts::new(CodecOptions::Webp(WebpOptions {
        lossless: false,
        ..WebpOptions::default()
    }));
    let codec = reg();
    let err = codec
        .by_id("webp")
        .unwrap()
        .encode(&img, &opts)
        .err()
        .unwrap();
    assert_eq!(err.message_id(), "error.codec.unsupported_feature");
}

#[test]
fn webp_carries_icc_and_exif_both_ways() {
    let bytes = srgb_bytes(5, 5, true);
    let plane = linear_plane(&bytes);
    let profile = b"webp-icc-fixture".to_vec();
    let exif = appname_io::meta::exif_block_with_orientation(1, false);
    let meta = Metadata {
        icc: None,
        exif: Some(exif.clone()),
        xmp: None,
        iptc: None,
        animation_loop: None,
    };
    let img = export_image(5, 5, &plane, Some(&profile), &meta);
    let opts = EncodeOpts::new(CodecOptions::Webp(WebpOptions::default()));
    let codec = reg();
    let codec = codec.by_id("webp").unwrap();
    let encoded = codec.encode(&img, &opts).unwrap();
    let imported = codec.decode(&encoded, &DecodeOpts::default()).unwrap();
    assert_eq!(imported.profile.as_deref(), Some(profile.as_slice()));
    assert_eq!(imported.meta.exif.as_deref(), Some(exif.as_slice()));
}

/// Hand-assembled animated WebP (VP8X + ANIM + two ANMF frames whose
/// payloads reuse the elected encoder's VP8L output) — §8.2 frame↔layer
/// import contract.
#[test]
fn webp_animation_imports_frame_layers() {
    // Encode two 4x3 lossless stills and lift their VP8L chunks.
    let vp8l = |seed: u8| -> Vec<u8> {
        let mut rgba = Vec::new();
        for i in 0..12u32 {
            rgba.extend_from_slice(&[seed.wrapping_mul(i as u8), 10, 20, 255]);
        }
        let mut out = Vec::new();
        image_webp::WebPEncoder::new(&mut out)
            .encode(&rgba, 4, 3, image_webp::ColorType::Rgba8)
            .unwrap();
        // Simple container: RIFF(12) + "VP8L" + size + payload.
        let size = u32::from_le_bytes(out[16..20].try_into().unwrap()) as usize;
        out[12..20 + size + size % 2].to_vec() // whole VP8L chunk, padded
    };
    let f0 = vp8l(3);
    let f1 = vp8l(7);
    let chunk = |tag: &[u8; 4], payload: &[u8]| -> Vec<u8> {
        let mut c = Vec::new();
        c.extend_from_slice(tag);
        c.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        c.extend_from_slice(payload);
        if payload.len() % 2 == 1 {
            c.push(0);
        }
        c
    };
    let mut vp8x = Vec::new();
    vp8x.push(0x02); // ANIMATION flag
    vp8x.extend_from_slice(&[0, 0, 0]); // reserved
    vp8x.extend_from_slice(&3u32.to_le_bytes()[0..3]); // canvas w-1
    vp8x.extend_from_slice(&2u32.to_le_bytes()[0..3]); // canvas h-1
    let mut anim = Vec::new();
    anim.extend_from_slice(&0u32.to_le_bytes()); // background
    anim.extend_from_slice(&0u16.to_le_bytes()); // loop forever
    let anmf = |frame: &[u8], dur_ms: u32| -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&[0, 0, 0]); // x/2
        p.extend_from_slice(&[0, 0, 0]); // y/2
        p.extend_from_slice(&3u32.to_le_bytes()[0..3]); // w-1
        p.extend_from_slice(&2u32.to_le_bytes()[0..3]); // h-1
        p.extend_from_slice(&dur_ms.to_le_bytes()[0..3]);
        p.push(0); // blend over, dispose none
        p.extend_from_slice(frame);
        chunk(b"ANMF", &p)
    };
    let mut body = Vec::new();
    body.extend_from_slice(b"WEBP");
    body.extend_from_slice(&chunk(b"VP8X", &vp8x));
    body.extend_from_slice(&chunk(b"ANIM", &anim));
    body.extend_from_slice(&anmf(&f0, 40));
    body.extend_from_slice(&anmf(&f1, 60));
    let mut file = Vec::new();
    file.extend_from_slice(b"RIFF");
    file.extend_from_slice(&(body.len() as u32).to_le_bytes());
    file.extend_from_slice(&body);

    let codec = reg();
    let codec = codec.by_id("webp").unwrap();
    let imported = codec.decode(&file, &DecodeOpts::default()).unwrap();
    let layers = imported.layers.expect("animated -> layers");
    assert_eq!(layers.len(), 2);
    assert_eq!(layers[0].name, "frame:0 delay:40");
    assert_eq!(layers[1].name, "frame:1 delay:60");
    assert_eq!(imported.meta.animation_loop, Some(0));
}

// ----------------------------------------------------------------- GIF ----

#[test]
fn gif_two_color_still_round_trips_exactly() {
    // Two exactly-representable colors -> quantization is the identity.
    let mut bytes = Vec::new();
    for i in 0..48u32 {
        let c: [u8; 4] = if i % 2 == 0 {
            [0, 0, 0, 255]
        } else {
            [255, 255, 255, 255]
        };
        bytes.extend_from_slice(&c);
    }
    let plane = linear_plane(&bytes);
    let meta = Metadata::default();
    let img = export_image(12, 4, &plane, None, &meta);
    let opts = EncodeOpts::new(CodecOptions::Gif(GifOptions::default()));
    let codec = reg();
    let codec = codec.by_id("gif").unwrap();
    let a = codec.encode(&img, &opts).unwrap();
    let b = codec.encode(&img, &opts).unwrap();
    assert_eq!(a, b, "deterministic");
    let imported = codec.decode(&a, &DecodeOpts::default()).unwrap();
    assert_eq!(imported.size, (12, 4));
    let round = buffer_to_srgb_bytes(&imported.buffer, 12, 4);
    assert_bytes_close(&round, &bytes, 0, "gif two-color");
    assert!(imported.layers.is_none(), "a still is not an animation");
    assert_eq!(
        imported.profile, None,
        "GIF carries no ICC (§8.2 carrier table)"
    );
}

#[test]
fn gif_animation_exports_frame_layers_and_reimports_them() {
    let f0 = srgb_bytes(6, 6, true);
    let f1: Vec<u8> = f0.iter().map(|b| b.wrapping_add(64)).collect();
    let p0 = linear_plane(&f0);
    let p1 = linear_plane(&f1);
    let composite = p0.clone();
    let meta = Metadata::default();
    let img = ExportImage {
        width: 6,
        height: 6,
        plane: &composite,
        layers: Some(vec![
            ExportLayer {
                name: "frame:0 delay:30",
                plane: &p0,
            },
            ExportLayer {
                name: "not a frame layer",
                plane: &p1,
            },
            ExportLayer {
                name: "frame:1",
                plane: &p1,
            },
        ]),
        profile: None,
        meta: &meta,
    };
    let opts = EncodeOpts::new(CodecOptions::Gif(GifOptions {
        loop_count: 3,
        default_delay_ms: 100,
        ..GifOptions::default()
    }));
    let codec = reg();
    let codec = codec.by_id("gif").unwrap();
    let encoded = codec.encode(&img, &opts).unwrap();
    let imported = codec.decode(&encoded, &DecodeOpts::default()).unwrap();
    let layers = imported.layers.expect("two frames -> layers");
    assert_eq!(
        layers.len(),
        2,
        "non-matching layer skipped, never mixed in"
    );
    assert_eq!(layers[0].name, "frame:0 delay:30");
    assert_eq!(
        layers[1].name, "frame:1 delay:100",
        "missing delay: uses default_delay_ms"
    );
    assert_eq!(imported.meta.animation_loop, Some(3));
}

#[test]
fn gif_duplicate_frame_index_is_a_frame_mapping_error() {
    let plane = vec![0.0f32; 4];
    let meta = Metadata::default();
    let img = ExportImage {
        width: 1,
        height: 1,
        plane: &plane,
        layers: Some(vec![
            ExportLayer {
                name: "frame:2",
                plane: &plane,
            },
            ExportLayer {
                name: "frame:2 delay:50",
                plane: &plane,
            },
        ]),
        profile: None,
        meta: &meta,
    };
    let opts = EncodeOpts::new(CodecOptions::Gif(GifOptions::default()));
    let codec = reg();
    let err = codec
        .by_id("gif")
        .unwrap()
        .encode(&img, &opts)
        .err()
        .unwrap();
    assert_eq!(err.message_id(), "error.codec.frame_mapping");
}

#[test]
fn gif_frame_name_grammar_is_exact() {
    use appname_io::codecs::gif_io::parse_frame_name;
    assert_eq!(parse_frame_name("frame:0"), Some((0, None)));
    assert_eq!(
        parse_frame_name("frame:12 delay:250"),
        Some((12, Some(250)))
    );
    assert_eq!(parse_frame_name("frame:01"), None, "leading zero forbidden");
    assert_eq!(parse_frame_name("frame:"), None);
    assert_eq!(parse_frame_name("frame:1 delay:"), None);
    assert_eq!(parse_frame_name("frame:1 wait:5"), None);
    assert_eq!(parse_frame_name("Frame:1"), None, "case-sensitive contract");
    assert_eq!(
        parse_frame_name("frame:1  delay:5"),
        None,
        "single space only"
    );
}

#[test]
fn gif_transparency_binarizes_at_half_and_survives() {
    // Left pixel transparent (a<128), right opaque red.
    let bytes = vec![9, 9, 9, 100, 200, 10, 10, 255];
    let plane = linear_plane(&bytes);
    let meta = Metadata::default();
    let img = export_image(2, 1, &plane, None, &meta);
    let opts = EncodeOpts::new(CodecOptions::Gif(GifOptions::default()));
    let codec = reg();
    let codec = codec.by_id("gif").unwrap();
    let encoded = codec.encode(&img, &opts).unwrap();
    let imported = codec.decode(&encoded, &DecodeOpts::default()).unwrap();
    let round = buffer_to_srgb_bytes(&imported.buffer, 2, 1);
    assert_eq!(round[3], 0, "binarized transparent");
    assert_eq!(round[7], 255, "opaque survived");
    assert_eq!(&round[4..7], &[200, 10, 10], "exact palette color");
}

// ----------------------------------------------------------------- BMP ----

#[test]
fn bmp_round_trips_b24_exact_and_b32_close() {
    let codec = reg();
    let codec = codec.by_id("bmp").unwrap();
    for (opaque, tol) in [(true, 0u8), (false, 1u8)] {
        let bytes = srgb_bytes(7, 3, opaque);
        let plane = linear_plane(&bytes);
        let meta = Metadata::default();
        let img = export_image(7, 3, &plane, None, &meta);
        let opts = EncodeOpts::new(CodecOptions::Bmp(BmpOptions::default()));
        let a = codec.encode(&img, &opts).unwrap();
        let b = codec.encode(&img, &opts).unwrap();
        assert_eq!(a, b, "deterministic");
        let imported = codec.decode(&a, &DecodeOpts::default()).unwrap();
        assert_eq!(imported.size, (7, 3));
        let round = buffer_to_srgb_bytes(&imported.buffer, 7, 3);
        if opaque {
            // Auto -> B24: alpha-less; the decode restores opaque alpha.
            assert_bytes_close(&round, &bytes, 0, "bmp b24");
        } else {
            assert_bytes_close(&round, &bytes, tol, "bmp b32 alpha");
        }
    }
}

#[test]
fn bmp_carries_icc_through_decode() {
    let bytes = srgb_bytes(3, 3, true);
    let plane = linear_plane(&bytes);
    let profile = b"bmp-icc-carriage".to_vec();
    let meta = Metadata::default();
    let img = export_image(3, 3, &plane, Some(&profile), &meta);
    let opts = EncodeOpts::new(CodecOptions::Bmp(BmpOptions::default()));
    let codec = reg();
    let codec = codec.by_id("bmp").unwrap();
    let encoded = codec.encode(&img, &opts).unwrap();
    let imported = codec.decode(&encoded, &DecodeOpts::default()).unwrap();
    assert_eq!(imported.profile.as_deref(), Some(profile.as_slice()));
}

// ------------------------------------------------------------- registry ----

#[test]
fn registry_resolves_every_encoded_format_by_content() {
    let bytes = srgb_bytes(4, 4, true);
    let plane = linear_plane(&bytes);
    let meta = Metadata::default();
    let registry = reg();
    let cases: Vec<(&str, CodecOptions)> = vec![
        ("jpeg", CodecOptions::Jpeg(JpegOptions::default())),
        ("tiff", CodecOptions::Tiff(TiffOptions::default())),
        ("webp", CodecOptions::Webp(WebpOptions::default())),
        ("gif", CodecOptions::Gif(GifOptions::default())),
        ("bmp", CodecOptions::Bmp(BmpOptions::default())),
    ];
    for (id, format) in cases {
        let img = export_image(4, 4, &plane, None, &meta);
        let encoded = registry
            .by_id(id)
            .unwrap()
            .encode(&img, &EncodeOpts::new(format))
            .unwrap();
        let head = &encoded[..encoded.len().min(512)];
        // A wrong extension never overrides content.
        let resolved = registry.resolve(head, Some("png")).unwrap();
        assert_eq!(resolved.id().0, id, "content resolution for {id}");
    }
}
