// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §8.1 (error taxonomy; limits checked BEFORE pixel allocation);
//! plan §2.3 test bar — malformed-input typed rejects, NO panics (the
//! D-series bar; the fuzzing harness proper is CP-0.7).

mod common;

use appname_io::{
    BmpOptions, CodecOptions, CodecRegistry, DecodeOpts, EncodeOpts, GifOptions, JpegOptions,
    Metadata, TiffOptions, WebpOptions,
};
use common::{linear_plane, srgb_bytes};

fn all_formats() -> Vec<(&'static str, CodecOptions)> {
    vec![
        ("jpeg", CodecOptions::Jpeg(JpegOptions::default())),
        ("tiff", CodecOptions::Tiff(TiffOptions::default())),
        ("webp", CodecOptions::Webp(WebpOptions::default())),
        ("gif", CodecOptions::Gif(GifOptions::default())),
        ("bmp", CodecOptions::Bmp(BmpOptions::default())),
    ]
}

/// Valid encodes of a small fixture, one per format (the mutation bases).
fn encoded_fixtures() -> Vec<(&'static str, Vec<u8>)> {
    let bytes = srgb_bytes(8, 8, true);
    let plane = linear_plane(&bytes);
    let meta = Metadata::default();
    let registry = CodecRegistry::builtin();
    all_formats()
        .into_iter()
        .map(|(id, format)| {
            let img = appname_io::ExportImage {
                width: 8,
                height: 8,
                plane: &plane,
                layers: None,
                profile: None,
                meta: &meta,
            };
            let encoded = registry
                .by_id(id)
                .unwrap()
                .encode(&img, &EncodeOpts::new(format))
                .unwrap();
            (id, encoded)
        })
        .collect()
}

#[test]
fn garbage_after_magic_rejects_typed_never_panics() {
    let registry = CodecRegistry::builtin();
    let heads: Vec<(&str, Vec<u8>)> = vec![
        ("jpeg", vec![0xFF, 0xD8, 0xFF, 0xE0]),
        ("tiff", b"II\x2A\x00".to_vec()),
        ("webp", b"RIFF\x10\x00\x00\x00WEBP".to_vec()),
        ("gif", b"GIF89a".to_vec()),
        ("bmp", b"BM".to_vec()),
    ];
    for (id, magic) in heads {
        let mut junk = magic.clone();
        junk.extend_from_slice(&[0xAB; 64]);
        let err = registry
            .by_id(id)
            .unwrap()
            .decode(&junk, &DecodeOpts::default())
            .err()
            .unwrap_or_else(|| panic!("{id}: garbage decoded"));
        // Any mapped taxonomy id is acceptable; the bar is TYPED, not
        // which arm — but it must be a codec/io arm, not internal.
        let mid = err.message_id();
        assert!(
            mid.starts_with("error.codec.") || mid.starts_with("error.io."),
            "{id}: unexpected mapping {mid}"
        );
    }
}

#[test]
fn truncation_at_every_quarter_rejects_typed_never_panics() {
    let registry = CodecRegistry::builtin();
    for (id, encoded) in encoded_fixtures() {
        for cut in [encoded.len() / 4, encoded.len() / 2, encoded.len() * 3 / 4] {
            let result = registry
                .by_id(id)
                .unwrap()
                .decode(&encoded[..cut], &DecodeOpts::default());
            if let Err(e) = result {
                let mid = e.message_id();
                assert!(
                    mid.starts_with("error.codec.") || mid.starts_with("error.io."),
                    "{id}@{cut}: unexpected mapping {mid}"
                );
            }
            // Some prefixes still parse as a smaller valid stream (e.g. a
            // multi-frame GIF cut between frames) — Ok is acceptable; the
            // bar is NO PANIC + typed errors when they occur.
        }
    }
}

#[test]
fn limits_gate_fires_before_pixel_allocation() {
    let registry = CodecRegistry::builtin();
    let tight = DecodeOpts {
        max_pixels: 16,
        ..DecodeOpts::default()
    };
    for (id, encoded) in encoded_fixtures() {
        // 8x8 = 64 pixels > 16.
        let err = registry
            .by_id(id)
            .unwrap()
            .decode(&encoded, &tight)
            .err()
            .unwrap_or_else(|| panic!("{id}: limit not enforced"));
        assert_eq!(err.message_id(), "error.codec.limit_exceeded", "{id}");
    }
    let narrow = DecodeOpts {
        max_dimension: 4,
        ..DecodeOpts::default()
    };
    for (id, encoded) in encoded_fixtures() {
        let err = registry
            .by_id(id)
            .unwrap()
            .decode(&encoded, &narrow)
            .err()
            .unwrap();
        assert_eq!(err.message_id(), "error.codec.limit_exceeded", "{id}");
    }
}

#[test]
fn interim_option_arms_reject_typed() {
    let registry = CodecRegistry::builtin();
    let fixtures = encoded_fixtures();
    let (_, jpeg) = &fixtures[0];
    // target_depth / icc_override wait for the CMM seam.
    for opts in [
        DecodeOpts {
            target_depth: Some(appname_core::ChannelKind::U16),
            ..DecodeOpts::default()
        },
        DecodeOpts {
            icc_override: Some(appname_core::ProfileId::nil()),
            ..DecodeOpts::default()
        },
    ] {
        let err = registry
            .by_id("jpeg")
            .unwrap()
            .decode(jpeg, &opts)
            .err()
            .unwrap();
        assert_eq!(err.message_id(), "error.codec.unsupported_feature");
    }
}

#[test]
fn out_of_range_options_fail_before_encoding() {
    let plane = vec![0.0f32; 4];
    let meta = Metadata::default();
    let registry = CodecRegistry::builtin();
    let img = || appname_io::ExportImage {
        width: 1,
        height: 1,
        plane: &plane,
        layers: None,
        profile: None,
        meta: &meta,
    };
    let cases: Vec<(&str, CodecOptions, &str)> = vec![
        (
            "jpeg",
            CodecOptions::Jpeg(JpegOptions {
                quality: 101,
                ..JpegOptions::default()
            }),
            "error.codec.bad_option",
        ),
        (
            "jpeg",
            CodecOptions::Jpeg(JpegOptions {
                progressive: true,
                ..JpegOptions::default()
            }),
            "error.codec.unsupported_feature",
        ),
        (
            "jpeg",
            CodecOptions::Jpeg(JpegOptions {
                restart_interval: Some(4),
                ..JpegOptions::default()
            }),
            "error.codec.unsupported_feature",
        ),
        (
            "webp",
            CodecOptions::Webp(WebpOptions {
                quality: 100.5,
                ..WebpOptions::default()
            }),
            "error.codec.bad_option",
        ),
        (
            "webp",
            CodecOptions::Webp(WebpOptions {
                effort: 7,
                ..WebpOptions::default()
            }),
            "error.codec.bad_option",
        ),
        (
            "gif",
            CodecOptions::Gif(GifOptions {
                max_colors: 1,
                ..GifOptions::default()
            }),
            "error.codec.bad_option",
        ),
        (
            "gif",
            CodecOptions::Gif(GifOptions {
                max_colors: 257,
                ..GifOptions::default()
            }),
            "error.codec.bad_option",
        ),
        (
            "tiff",
            CodecOptions::Tiff(TiffOptions {
                planar: true,
                ..TiffOptions::default()
            }),
            "error.codec.unsupported_feature",
        ),
        (
            "tiff",
            CodecOptions::Tiff(TiffOptions {
                tile_size: Some(256),
                ..TiffOptions::default()
            }),
            "error.codec.unsupported_feature",
        ),
    ];
    for (id, format, want) in cases {
        let err = registry
            .by_id(id)
            .unwrap()
            .encode(&img(), &EncodeOpts::new(format))
            .err()
            .unwrap_or_else(|| panic!("{id}: expected {want}"));
        assert_eq!(err.message_id(), want, "{id}");
    }
}

/// A header-only BMP claiming the given dimensions (BITMAPINFOHEADER,
/// 24-bit, BI_RGB) — enough for every pre-pixel gate to fire.
fn bmp_header(w: i32, h: i32) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(b"BM");
    b.extend_from_slice(&[0u8; 8]); // size + reserved
    b.extend_from_slice(&54u32.to_le_bytes()); // data offset
    b.extend_from_slice(&40u32.to_le_bytes()); // info header size
    b.extend_from_slice(&w.to_le_bytes());
    b.extend_from_slice(&h.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // planes
    b.extend_from_slice(&24u16.to_le_bytes()); // bit count
    b.extend_from_slice(&[0u8; 24]); // compression..important
    b
}

/// Review round 1 minor 2: the §8.1-legal band ABOVE the elected facade's
/// 65535-per-axis capacity surfaces as typed `UnsupportedFeature`, never
/// `Corrupt`; over-limit dimensions keep firing OUR gate first, from the
/// raw header, before any facade construction.
#[test]
fn bmp_facade_capacity_band_is_typed_unsupported_not_corrupt() {
    let registry = CodecRegistry::builtin();
    let codec = registry.by_id("bmp").unwrap();
    // 70000 x 10: legal under the §8.1 defaults (max_dimension 2^20,
    // max_pixels 2^32) but beyond the facade cap.
    let err = codec
        .decode(&bmp_header(70_000, 10), &DecodeOpts::default())
        .err()
        .unwrap();
    assert_eq!(
        err.message_id(),
        "error.codec.unsupported_feature",
        "legal band: {err}"
    );
    // Top-down encoding (negative height) classifies by |h| the same way.
    let err = codec
        .decode(&bmp_header(10, -70_000), &DecodeOpts::default())
        .err()
        .unwrap();
    assert_eq!(
        err.message_id(),
        "error.codec.unsupported_feature",
        "negative-height band"
    );
    // 70000 x 70000 = 4.9e9 pixels: OUR §8.1 gate fires first (from the
    // raw header — no facade, no allocation).
    let err = codec
        .decode(&bmp_header(70_000, 70_000), &DecodeOpts::default())
        .err()
        .unwrap();
    assert_eq!(err.message_id(), "error.codec.limit_exceeded");
    // Axis over max_dimension: dimension gate, still ours, still typed.
    let err = codec
        .decode(&bmp_header(1_048_577, 1), &DecodeOpts::default())
        .err()
        .unwrap();
    assert_eq!(err.message_id(), "error.codec.limit_exceeded");
    // At-cap axis is NOT misrouted to the unsupported arm: 65535 x 1
    // passes both pre-checks and proceeds to real decoding (truncated
    // fixture -> a typed codec/io error, but never the capacity message).
    let err = codec
        .decode(&bmp_header(65_535, 1), &DecodeOpts::default())
        .err()
        .unwrap();
    assert_ne!(
        err.message_id(),
        "error.codec.unsupported_feature",
        "at-cap axis: {err}"
    );
}

#[test]
fn zero_and_absurd_headers_reject_typed() {
    let registry = CodecRegistry::builtin();
    // A GIF header claiming a 60000x60000 logical screen: the §8.1 gate
    // must fire from the HEADER, before any pixel allocation.
    let mut gif = Vec::new();
    gif.extend_from_slice(b"GIF89a");
    gif.extend_from_slice(&60000u16.to_le_bytes());
    gif.extend_from_slice(&60000u16.to_le_bytes());
    gif.extend_from_slice(&[0xF0, 0, 0]); // GCT flag + background + aspect
    gif.extend_from_slice(&[0; 12]); // a tiny global palette
    let opts = DecodeOpts {
        max_pixels: 1_000_000,
        ..DecodeOpts::default()
    };
    let err = registry
        .by_id("gif")
        .unwrap()
        .decode(&gif, &opts)
        .err()
        .unwrap();
    assert_eq!(err.message_id(), "error.codec.limit_exceeded");

    // A PNG with no IHDR is corrupt, typed.
    let png_magic = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0];
    let err = registry
        .by_id("png")
        .unwrap()
        .decode(&png_magic, &DecodeOpts::default())
        .err()
        .unwrap();
    assert_eq!(err.message_id(), "error.codec.corrupt");
}
