// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §8.1 (registry slot 1), §8.2 (PNG row); DD ⑫.
//!
//! OWNERSHIP NOTE (plan: "PNG lives in appname-core at CP-0.1 interim;
//! migrating codec ownership to appname-io is part of the O-IO arc —
//! coordinate, don't grab"): the pixel paths still ride core's public
//! PNG boundary (`decode_png_to_buffer` / `encode_composite_png`) so
//! the CP-0.1 byte-stability bar stands untouched — a fully-default,
//! metadata-less encode is core's bytes, verbatim. The CP-1.0 export
//! wave lands the ENCODE-SIDE half of the recorded ownership handoff
//! in-crate: explicit 8/16-bit depth, compression levels, `iCCP` and
//! `eXIf` carriage (§29.1 option table + the §29.4 embed surface),
//! written by this codec through the same elected `png` crate. Decode
//! grows `iCCP` extraction (profile carriage for the §5 seam). Still
//! with the migration's later half: interlaced encode (the elected
//! encoder cannot write Adam7 — typed), `sRGB`/`gAMA` precedence and
//! `eXIf`-orientation honoring on DECODE (the ingress carrier is the
//! original file bytes; baking would decouple them — recorded
//! deferral, lane log).

use std::io::Write as _;

use appname_core::io::srgb::{srgb_encode, unpremul};
use appname_core::{decode_png_to_buffer, encode_composite_png};

use crate::error::CodecError;
use crate::meta::Metadata;
use crate::opts::{CodecOptions, DecodeOpts, EncodeOpts, IccPolicy, PngDepth};
use crate::registry::{
    Codec, CodecCaps, CodecId, CompatAction, CompatReport, ExportImage, Imported, LicenseStatus,
};

pub struct PngCodec;

impl Codec for PngCodec {
    fn id(&self) -> CodecId {
        CodecId("png")
    }

    fn capabilities(&self) -> CodecCaps {
        CodecCaps {
            read: true,
            write: true,
            layered: false,
            max_depth: appname_core::ChannelKind::U16,
            alpha: true,
            animation: false,
            license: LicenseStatus::Builtin,
        }
    }

    fn sniff(&self, head: &[u8]) -> bool {
        head.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A])
    }

    fn magic_len(&self) -> u32 {
        8
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["png"]
    }

    fn decode(&self, bytes: &[u8], opts: &DecodeOpts) -> Result<Imported, CodecError> {
        opts.check_interim_unsupported()?;
        let mut report = CompatReport::default();
        // §8.1 bomb gate BEFORE pixel allocation: read the IHDR dimensions
        // straight from the fixed layout (signature 8 + length 4 + "IHDR").
        let dims = |o: usize| -> Option<u32> {
            Some(u32::from_be_bytes(bytes.get(o..o + 4)?.try_into().ok()?))
        };
        match (bytes.get(12..16), dims(16), dims(20)) {
            (Some(b"IHDR"), Some(w), Some(h)) => opts.check_limits(w, h)?,
            _ => return Err(CodecError::corrupt("missing IHDR")),
        }
        let (buffer, w, h) =
            decode_png_to_buffer(bytes, opts.working_profile).map_err(CodecError::corrupt)?;
        // PNG input IS its own journaled-import carrier: the original
        // bytes go to the asset ingress verbatim (best fidelity + content
        // dedupe; core's asset re-decode is this same boundary).
        let ingress_png = bytes.to_vec();
        // Metadata pass (export-wave half of the ownership handoff):
        // a header-only `png` read (stops at the first IDAT) surfaces
        // iCCP. eXIf-orientation HONORING stays with the migration
        // (module doc) — the block is carried, not baked.
        let mut icc = None;
        let mut exif = None;
        if opts.keep_unknown_metadata {
            if let Ok(reader) = png::Decoder::new(std::io::Cursor::new(bytes)).read_info() {
                let info = reader.info();
                icc = info.icc_profile.as_ref().map(|c| c.to_vec());
                exif = info.exif_metadata.as_ref().map(|c| c.to_vec());
                if exif.is_some() {
                    report.push(
                        "metadata.png.exif",
                        CompatAction::Info,
                        "eXIf carried; orientation honoring lands with the decode-side migration",
                    );
                }
            }
        }
        Ok(Imported {
            buffer,
            ingress_png,
            size: (w, h),
            profile: icc.clone(),
            meta: Metadata {
                icc,
                exif,
                ..Metadata::default()
            },
            layers: None,
            orientation_applied: None,
            report,
            compat_report: None,
        })
    }

    fn encode(&self, img: &ExportImage<'_>, opts: &EncodeOpts) -> Result<Vec<u8>, CodecError> {
        let CodecOptions::Png(o) = &opts.format else {
            return Err(CodecError::internal("png codec handed non-png options"));
        };
        opts.check_interim_unsupported()?;
        if o.compression > 9 {
            return Err(CodecError::OptionOutOfRange {
                field: "png.compression",
                detail: format!("{} not in 0..=9", o.compression),
            });
        }
        if o.interlace {
            // the elected encoder cannot WRITE Adam7 (module doc)
            return Err(CodecError::unsupported(
                "png.interlace encode (the elected encoder has no Adam7 writer; migration deferral)",
            ));
        }
        let embed_icc = match opts.icc {
            IccPolicy::Embed => img.profile,
            IccPolicy::Omit => None,
            _ => unreachable!("check_interim_unsupported rejected the CMM arms"),
        };
        let embed_exif = if opts.metadata == crate::opts::MetadataPolicy::Preserve {
            img.meta.exif.as_deref()
        } else {
            None
        };
        // Fully-default, metadata-less encode = core's CP-0.1 boundary,
        // byte-for-byte (the goldens bar; ownership note).
        if o.compression == 6
            && matches!(o.bit_depth, PngDepth::Auto | PngDepth::B8)
            && embed_icc.is_none()
            && embed_exif.is_none()
        {
            return encode_composite_png(img.width, img.height, img.plane)
                .map_err(CodecError::internal);
        }
        // The export-wave encode path (§29.1 table: depth 8|16,
        // compression 0–9, iCCP/eXIf carriage).
        if img.plane.len() != img.width as usize * img.height as usize * 4 {
            return Err(CodecError::internal("plane length must be w*h*4"));
        }
        let sixteen = o.bit_depth == PngDepth::B16;
        let mut data = Vec::with_capacity(img.plane.len() * if sixteen { 2 } else { 1 });
        for px in img.plane.chunks_exact(4) {
            let s = unpremul([px[0], px[1], px[2], px[3]]);
            let enc = [
                srgb_encode(s[0].clamp(0.0, 1.0)),
                srgb_encode(s[1].clamp(0.0, 1.0)),
                srgb_encode(s[2].clamp(0.0, 1.0)),
                s[3].clamp(0.0, 1.0),
            ];
            if sixteen {
                for c in enc {
                    data.extend_from_slice(&(((c * 65535.0).round()) as u16).to_be_bytes());
                }
            } else {
                for c in enc {
                    data.push((c * 255.0).round() as u8);
                }
            }
        }
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(std::io::Cursor::new(&mut out), img.width, img.height);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(if sixteen {
                png::BitDepth::Sixteen
            } else {
                png::BitDepth::Eight
            });
            if o.compression != 6 {
                enc.set_deflate_compression(png::DeflateCompression::Level(o.compression));
            }
            let mut writer = enc
                .write_header()
                .map_err(|e| CodecError::internal(format!("png header: {e}")))?;
            if let Some(profile) = embed_icc {
                // iCCP: name NUL method(0) zlib(profile) — fixed name,
                // pinned flate level (determinism)
                let mut payload = b"ICC profile\0\0".to_vec();
                let mut z =
                    flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(6));
                z.write_all(profile)
                    .and_then(|_| z.finish())
                    .map(|zb| payload.extend_from_slice(&zb))
                    .map_err(|e| CodecError::internal(format!("png iCCP flate: {e}")))?;
                writer
                    .write_chunk(png::chunk::iCCP, &payload)
                    .map_err(|e| CodecError::internal(format!("png iCCP: {e}")))?;
            }
            if let Some(exif) = embed_exif {
                writer
                    .write_chunk(png::chunk::eXIf, exif)
                    .map_err(|e| CodecError::internal(format!("png eXIf: {e}")))?;
            }
            writer
                .write_image_data(&data)
                .map_err(|e| CodecError::internal(format!("png data: {e}")))?;
        }
        Ok(out)
    }
}
