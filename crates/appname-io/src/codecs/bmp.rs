// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §8.2 (BMP row: baseline read/write; reads 1/4/8-bit palette,
//! 16/24/32-bit, `BI_RGB`/`BI_RLE8`/`BI_RLE4`/`BI_BITFIELDS`; writes
//! `BITMAPINFOHEADER` for B24, `BITMAPV5HEADER` for B32 — correct alpha
//! semantics — and whenever an ICC profile embeds, `bV5ProfileData`);
//! DD ⑫. BMP carries no EXIF/XMP (carrier table).
//!
//! ELECTION (jpeg-codecs record, BMP-posture note: "the image crate …
//! or own"): decode via the `image` facade's BMP decoder (battle-tested
//! against the wild variants above); encode via an IN-CRATE writer,
//! because the elected facade writes no `BITMAPV5HEADER` and §8.2's
//! B32-alpha + ICC-embed semantics require one. The writer emits
//! deterministic fixed-layout headers: B24 = 40-byte info header,
//! `BI_RGB`, bottom-up, 4-byte row padding; B32 = 124-byte V5 header,
//! `BI_BITFIELDS` BGRA masks, `LCS_sRGB` color space (the interim color
//! assumption) or `PROFILE_EMBEDDED` + appended profile bytes when one
//! embeds.

use std::io::Cursor;

use image::codecs::bmp::BmpDecoder;
use image::{DynamicImage, ImageDecoder};

use crate::codecs::shared;
use crate::error::CodecError;
use crate::opts::{BmpDepth, CodecOptions, DecodeOpts, EncodeOpts, IccPolicy};
use crate::plane::{linear_plane_to_srgb8, linear_plane_to_srgb8_over_matte, RgbaPlane};
use crate::registry::{
    Codec, CodecCaps, CodecId, CompatReport, ExportImage, Imported, LicenseStatus,
};

pub struct BmpCodec;

const PROFILE_EMBEDDED: u32 = 0x4D42_4544; // 'MBED'
const LCS_SRGB: u32 = 0x7352_4742; // 'sRGB'
const V5_SIZE: u32 = 124;
const FILE_HEADER: u32 = 14;

/// The elected decode facade's per-axis capacity (review round 1 minor 2):
/// dimensions above it are §8.1-legal (the gate allows up to 2^20/axis)
/// but beyond the facade — typed `UnsupportedFeature`, never `Corrupt`.
const FACADE_AXIS_CAP: u32 = 65_535;

fn map_image_err(e: image::ImageError) -> CodecError {
    match e {
        image::ImageError::Unsupported(u) => CodecError::unsupported(format!("bmp: {u}")),
        image::ImageError::Limits(l) => {
            CodecError::unsupported(format!("bmp.decoder_capacity: {l}"))
        }
        image::ImageError::Decoding(d) => CodecError::corrupt(d),
        image::ImageError::IoError(e) => CodecError::io_read(e),
        other => CodecError::internal(other),
    }
}

/// Read the claimed dimensions straight from the raw headers (file header
/// 14 bytes + core/info header), so the §8.1 gate and the facade-capacity
/// check both run BEFORE facade construction (review round 1 minor 2).
/// Absolute values: BMP encodes top-down images as negative heights.
fn header_dimensions(bytes: &[u8]) -> Result<(u32, u32), CodecError> {
    let info_size = bytes
        .get(14..18)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .ok_or(CodecError::Truncated { offset: 14 })?;
    if info_size == 12 {
        // BITMAPCOREHEADER: u16 dimensions.
        let d16 = |o: usize| -> Result<u16, CodecError> {
            bytes
                .get(o..o + 2)
                .map(|b| u16::from_le_bytes(b.try_into().unwrap()))
                .ok_or(CodecError::Truncated { offset: o as u64 })
        };
        Ok((u32::from(d16(18)?), u32::from(d16(20)?)))
    } else {
        let d32 = |o: usize| -> Result<i32, CodecError> {
            bytes
                .get(o..o + 4)
                .map(|b| i32::from_le_bytes(b.try_into().unwrap()))
                .ok_or(CodecError::Truncated { offset: o as u64 })
        };
        Ok((d32(18)?.unsigned_abs(), d32(22)?.unsigned_abs()))
    }
}

/// Extract an embedded ICC profile from a V5-headed BMP (the elected
/// decode facade does not surface it). Absent/foreign color spaces → None;
/// malformed offsets → None (carriage is best-effort metadata, §8.1
/// non-fatal rule).
fn bmp_icc(bytes: &[u8]) -> Option<Vec<u8>> {
    let u32le = |o: usize| -> Option<u32> {
        Some(u32::from_le_bytes(bytes.get(o..o + 4)?.try_into().ok()?))
    };
    if bytes.len() < 14 + 124 || &bytes[0..2] != b"BM" {
        return None;
    }
    let info_size = u32le(14)?;
    if info_size < V5_SIZE {
        return None;
    }
    let cstype = u32le(14 + 56)?;
    if cstype != PROFILE_EMBEDDED {
        return None;
    }
    let profile_offset = u32le(14 + 112)? as usize; // from the info header start
    let profile_size = u32le(14 + 116)? as usize;
    let start = 14usize.checked_add(profile_offset)?;
    let end = start.checked_add(profile_size)?;
    bytes.get(start..end).map(<[u8]>::to_vec)
}

impl Codec for BmpCodec {
    fn id(&self) -> CodecId {
        CodecId("bmp")
    }

    fn capabilities(&self) -> CodecCaps {
        CodecCaps {
            read: true,
            write: true,
            layered: false,
            max_depth: appname_core::ChannelKind::U8,
            alpha: true,
            animation: false,
            license: LicenseStatus::Builtin,
        }
    }

    fn sniff(&self, head: &[u8]) -> bool {
        head.starts_with(b"BM")
    }

    fn magic_len(&self) -> u32 {
        2
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["bmp", "dib"]
    }

    fn decode(&self, bytes: &[u8], opts: &DecodeOpts) -> Result<Imported, CodecError> {
        opts.check_interim_unsupported()?;
        let report = CompatReport::default();
        // §8.1 gate + facade-capacity check from the RAW headers, before
        // the facade allocates anything (review round 1 minor 2).
        let (hw, hh) = header_dimensions(bytes)?;
        opts.check_limits(hw, hh)?;
        if hw > FACADE_AXIS_CAP || hh > FACADE_AXIS_CAP {
            return Err(CodecError::unsupported(format!(
                "bmp.dimension {hw}x{hh} beyond the elected decoder's {FACADE_AXIS_CAP}/axis capacity"
            )));
        }
        let dec = BmpDecoder::new(Cursor::new(bytes)).map_err(map_image_err)?;
        let (w, h) = dec.dimensions();
        opts.check_limits(w, h)?;
        let icc = bmp_icc(bytes);
        let img = DynamicImage::from_decoder(dec).map_err(map_image_err)?;
        let rgba = img.into_rgba8();
        let plane = RgbaPlane::new8(w, h, rgba.into_raw())?;
        // BMP carries no EXIF (§8.2 carrier table) — no orientation.
        shared::finish_single(plane, None, icc, None, opts, report)
    }

    fn encode(&self, img: &ExportImage<'_>, opts: &EncodeOpts) -> Result<Vec<u8>, CodecError> {
        let CodecOptions::Bmp(o) = &opts.format else {
            return Err(CodecError::internal("bmp codec handed non-bmp options"));
        };
        opts.check_interim_unsupported()?;
        let profile = match opts.icc {
            IccPolicy::Embed => img.profile,
            _ => None,
        };
        let rgba = linear_plane_to_srgb8(img.width, img.height, img.plane)?;
        let any_alpha = rgba.chunks_exact(4).any(|px| px[3] != u8::MAX);
        let b32 = match o.depth {
            BmpDepth::B32 => true,
            BmpDepth::B24 => false,
            BmpDepth::Auto => any_alpha,
        } || profile.is_some(); // an ICC embed requires the V5 header
        let (wu, hu) = (img.width as usize, img.height as usize);
        let mut out = Vec::new();
        let w32 = |out: &mut Vec<u8>, v: u32| out.extend_from_slice(&v.to_le_bytes());
        let w16 = |out: &mut Vec<u8>, v: u16| out.extend_from_slice(&v.to_le_bytes());
        let wi32 = |out: &mut Vec<u8>, v: i32| out.extend_from_slice(&v.to_le_bytes());
        // Physical resolution: pixels-per-metre from ppi (0 = unspecified).
        let ppm = opts
            .ppi
            .map_or(0u32, |ppi| (ppi / 0.0254).round().max(0.0) as u32);
        if b32 {
            let row_bytes = wu * 4;
            let pixel_bytes = row_bytes * hu;
            let profile_len = profile.map_or(0, <[u8]>::len);
            let data_offset = FILE_HEADER + V5_SIZE;
            // BITMAPFILEHEADER
            out.extend_from_slice(b"BM");
            w32(
                &mut out,
                data_offset + pixel_bytes as u32 + profile_len as u32,
            );
            w32(&mut out, 0);
            w32(&mut out, data_offset);
            // BITMAPV5HEADER
            w32(&mut out, V5_SIZE);
            wi32(&mut out, img.width as i32);
            wi32(&mut out, img.height as i32); // positive = bottom-up
            w16(&mut out, 1); // planes
            w16(&mut out, 32); // bit count
            w32(&mut out, 3); // BI_BITFIELDS
            w32(&mut out, pixel_bytes as u32);
            w32(&mut out, ppm);
            w32(&mut out, ppm);
            w32(&mut out, 0); // colors used
            w32(&mut out, 0); // colors important
            w32(&mut out, 0x00FF_0000); // R mask
            w32(&mut out, 0x0000_FF00); // G mask
            w32(&mut out, 0x0000_00FF); // B mask
            w32(&mut out, 0xFF00_0000); // A mask
            w32(
                &mut out,
                if profile.is_some() {
                    PROFILE_EMBEDDED
                } else {
                    LCS_SRGB
                },
            );
            out.extend_from_slice(&[0u8; 36]); // CIEXYZTRIPLE endpoints (unused for sRGB/MBED)
            w32(&mut out, 0); // gamma red
            w32(&mut out, 0); // gamma green
            w32(&mut out, 0); // gamma blue
            w32(&mut out, 4); // LCS_GM_IMAGES intent
            w32(
                &mut out,
                if profile.is_some() {
                    V5_SIZE + pixel_bytes as u32
                } else {
                    0
                },
            );
            w32(&mut out, profile_len as u32);
            w32(&mut out, 0); // reserved
            debug_assert_eq!(out.len(), (FILE_HEADER + V5_SIZE) as usize);
            // Pixels, bottom-up BGRA.
            for y in (0..hu).rev() {
                for x in 0..wu {
                    let i = (y * wu + x) * 4;
                    out.extend_from_slice(&[rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]]);
                }
            }
            if let Some(p) = profile {
                out.extend_from_slice(p);
            }
        } else {
            let rgb =
                linear_plane_to_srgb8_over_matte(img.width, img.height, img.plane, opts.matte)?;
            let row_bytes = (wu * 3).div_ceil(4) * 4;
            let pixel_bytes = row_bytes * hu;
            let data_offset = FILE_HEADER + 40;
            out.extend_from_slice(b"BM");
            w32(&mut out, data_offset + pixel_bytes as u32);
            w32(&mut out, 0);
            w32(&mut out, data_offset);
            // BITMAPINFOHEADER
            w32(&mut out, 40);
            wi32(&mut out, img.width as i32);
            wi32(&mut out, img.height as i32);
            w16(&mut out, 1);
            w16(&mut out, 24);
            w32(&mut out, 0); // BI_RGB
            w32(&mut out, pixel_bytes as u32);
            w32(&mut out, ppm);
            w32(&mut out, ppm);
            w32(&mut out, 0);
            w32(&mut out, 0);
            for y in (0..hu).rev() {
                let mut row = Vec::with_capacity(row_bytes);
                for x in 0..wu {
                    let i = (y * wu + x) * 3;
                    row.extend_from_slice(&[rgb[i + 2], rgb[i + 1], rgb[i]]);
                }
                row.resize(row_bytes, 0);
                out.extend_from_slice(&row);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v5_icc_extraction_round_trips_with_the_writer() {
        let meta = crate::meta::Metadata::default();
        let plane = [0.5f32, 0.25, 0.0, 1.0];
        let profile = b"fake-icc-bytes-for-carriage".to_vec();
        let img = ExportImage {
            width: 1,
            height: 1,
            plane: &plane,
            layers: None,
            profile: Some(&profile),
            meta: &meta,
        };
        let opts = EncodeOpts::new(CodecOptions::Bmp(crate::opts::BmpOptions::default()));
        let bytes = BmpCodec.encode(&img, &opts).unwrap();
        assert_eq!(bmp_icc(&bytes), Some(profile));
    }
}
