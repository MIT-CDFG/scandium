// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §8.2 (JPEG row: read/write, 8-bit, no alpha; APP2 ICC; EXIF
//! orientation; quality/chroma/progressive options; carriers per "What
//! each format embeds"); §8.6; DD ⑫.
//!
//! ELECTIONS (jpeg-codecs deps record; reasoning in `reports/O-IO-log.md`):
//! decode = **zune-jpeg** consumed via the `image` facade (the record's
//! viable set; the mozjpeg/IJG arm is non-viable under policy); encode =
//! **`image`'s pure-Rust baseline encoder** (default-features off,
//! feature-gated to jpeg+bmp). Encoder facts, documented not hidden:
//! baseline sequential only (progressive → typed `UnsupportedFeature`),
//! fixed 4:4:4 subsampling — a REGISTERED deviation: §8.2's [NORMATIVE]
//! auto-chroma rule (S420 below q90) resolves to S444 at every quality
//! here, and no per-encode report channel exists yet to say so (see the
//! [`crate::opts`] deviation register; explicit S422/S420 → typed) — no
//! restart markers. ICC
//! embeds as spec-conformant multi-segment APP2; EXIF as APP1 (both via
//! the encoder's setters). 12-bit and arithmetic-coded streams surface as
//! typed decode errors from the elected decoder.

use std::io::Cursor;

use image::codecs::jpeg::{JpegDecoder, JpegEncoder};
use image::{DynamicImage, ExtendedColorType, ImageDecoder, ImageEncoder};

use crate::codecs::shared::{self, drop_bad_metadata};
use crate::error::CodecError;
use crate::opts::{
    ChromaSubsampling, CodecOptions, DecodeOpts, EncodeOpts, IccPolicy, MetadataPolicy,
};
use crate::plane::{linear_plane_to_srgb8_over_matte, RgbaPlane};
use crate::registry::{
    Codec, CodecCaps, CodecId, CompatReport, ExportImage, Imported, LicenseStatus,
};

pub struct JpegCodec;

fn map_image_err(e: image::ImageError) -> CodecError {
    match e {
        image::ImageError::Unsupported(u) => CodecError::unsupported(format!("jpeg: {u}")),
        // Facade/decoder capacity is an elected-decoder limitation, not a
        // malformed stream (review round 1 minor 2): §8.1 taxonomy arm =
        // UnsupportedFeature. (JPEG's own format ceiling is 65535/axis —
        // SOF dimensions are u16 — so the §8.1-legal band above that is
        // unrepresentable in this format to begin with.)
        image::ImageError::Limits(l) => {
            CodecError::unsupported(format!("jpeg.decoder_capacity: {l}"))
        }
        image::ImageError::Decoding(d) => CodecError::corrupt(d),
        image::ImageError::Encoding(d) => CodecError::internal(d),
        image::ImageError::Parameter(d) => CodecError::internal(d),
        image::ImageError::IoError(e) => CodecError::io_read(e),
    }
}

impl Codec for JpegCodec {
    fn id(&self) -> CodecId {
        CodecId("jpeg")
    }

    fn capabilities(&self) -> CodecCaps {
        CodecCaps {
            read: true,
            write: true,
            layered: false,
            max_depth: appname_core::ChannelKind::U8,
            alpha: false,
            animation: false,
            license: LicenseStatus::Builtin,
        }
    }

    fn sniff(&self, head: &[u8]) -> bool {
        head.starts_with(&[0xFF, 0xD8, 0xFF])
    }

    fn magic_len(&self) -> u32 {
        3
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["jpg", "jpeg", "jpe"]
    }

    fn decode(&self, bytes: &[u8], opts: &DecodeOpts) -> Result<Imported, CodecError> {
        opts.check_interim_unsupported()?;
        let mut report = CompatReport::default();
        let mut dec = JpegDecoder::new(Cursor::new(bytes)).map_err(map_image_err)?;
        let (w, h) = dec.dimensions();
        opts.check_limits(w, h)?;
        // Metadata extraction is never fatal (§8.1 MetadataInvalid rule).
        let icc = match dec.icc_profile() {
            Ok(v) => v,
            Err(e) => drop_bad_metadata(&mut report, "icc", e),
        };
        let exif = match dec.exif_metadata() {
            Ok(v) => v,
            Err(e) => drop_bad_metadata(&mut report, "exif", e),
        };
        let img = DynamicImage::from_decoder(dec).map_err(map_image_err)?;
        let rgba = img.into_rgba8();
        let plane = RgbaPlane::new8(w, h, rgba.into_raw())?;
        shared::finish_single(plane, exif, icc, None, opts, report)
    }

    fn encode(&self, img: &ExportImage<'_>, opts: &EncodeOpts) -> Result<Vec<u8>, CodecError> {
        let CodecOptions::Jpeg(o) = &opts.format else {
            return Err(CodecError::internal("jpeg codec handed non-jpeg options"));
        };
        o.validate()?;
        opts.check_interim_unsupported()?;
        if o.progressive {
            return Err(CodecError::unsupported(
                "jpeg.progressive (elected encoder is baseline)",
            ));
        }
        if o.restart_interval.is_some() {
            return Err(CodecError::unsupported(
                "jpeg.restart_interval (elected encoder)",
            ));
        }
        match o.chroma {
            None | Some(ChromaSubsampling::S444) => {}
            Some(_) => {
                return Err(CodecError::unsupported(
                    "jpeg.chroma subsampling other than 4:4:4 (elected encoder is fixed 4:4:4)",
                ))
            }
        }
        // No alpha (§8.2): composite over the matte in linear first.
        let rgb = linear_plane_to_srgb8_over_matte(img.width, img.height, img.plane, opts.matte)?;
        let mut out = Vec::new();
        // Encoder quality domain is 1..=100; the §8.2 contract range is
        // 0..=100 — 0 clamps to the encoder floor.
        let mut enc = JpegEncoder::new_with_quality(&mut out, o.quality.max(1));
        if opts.icc == IccPolicy::Embed {
            if let Some(profile) = img.profile {
                enc.set_icc_profile(profile.to_vec())
                    .map_err(|e| CodecError::internal(format!("icc embed: {e}")))?;
            }
        }
        if opts.metadata == MetadataPolicy::Preserve {
            if let Some(exif) = &img.meta.exif {
                // The carried block is orientation-normalized at decode
                // (§8.6) — re-embed byte-wise.
                enc.set_exif_metadata(exif.clone())
                    .map_err(|e| CodecError::internal(format!("exif embed: {e}")))?;
            }
        }
        enc.write_image(&rgb, img.width, img.height, ExtendedColorType::Rgb8)
            .map_err(map_image_err)?;
        Ok(out)
    }
}

/// Splice one APP1 EXIF segment (with the `Exif\0\0` header) into a JPEG
/// byte stream right after SOI — the test-fixture builder for the §8.6
/// orientation matrix (real files put EXIF before the frame; so does
/// this).
pub fn splice_app1_exif(jpeg: &[u8], exif_block: &[u8]) -> Result<Vec<u8>, CodecError> {
    if !jpeg.starts_with(&[0xFF, 0xD8]) {
        return Err(CodecError::corrupt("not a JPEG (no SOI)"));
    }
    let payload_len = exif_block.len() + 6 + 2; // "Exif\0\0" + the length word
    if payload_len > u16::MAX as usize {
        return Err(CodecError::internal(
            "EXIF block too large for one APP1 segment",
        ));
    }
    let mut out = Vec::with_capacity(jpeg.len() + payload_len + 2);
    out.extend_from_slice(&jpeg[..2]);
    out.extend_from_slice(&[0xFF, 0xE1]);
    out.extend_from_slice(&(payload_len as u16).to_be_bytes());
    out.extend_from_slice(b"Exif\0\0");
    out.extend_from_slice(exif_block);
    out.extend_from_slice(&jpeg[2..]);
    Ok(out)
}
