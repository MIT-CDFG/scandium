// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §8.2 (WebP row: read/write, 8-bit, alpha, lossy+lossless read,
//! animation via the GIF frame↔layer convention with disposal/blend
//! applied on decode so every layer is a full frame; ICC/EXIF/XMP chunks
//! per the carrier table); §8.6; DD ⑫.
//!
//! ELECTION: pure-Rust `image-webp` per its deps record (the two-layer
//! libwebp alternative needs a fresh record and was not taken — record
//! notes). Elected-encoder facts, documented not hidden: **encode is VP8L
//! lossless only** — `lossless: false` answers typed
//! `UnsupportedFeature("webp.lossy_encode")` and our `WebpOptions` default
//! flips to lossless (deviation note in [`crate::opts`]); `quality`'s
//! lossless effort-weighting and `effort` are range-validated then have no
//! effect — the elected encoder exposes no equivalent dial, so encodes use
//! its fixed deterministic parameters (the Export-As report surfaces this
//! when that subsystem lands, §29.1); animation export joins a later slice
//! (typed). ICC/EXIF embed via the encoder's native VP8X chunk support.

use std::io::Cursor;

use image_webp::{ColorType as WebpColor, WebPDecoder, WebPEncoder};

use crate::codecs::shared::{self, drop_bad_metadata};
use crate::error::CodecError;
use crate::meta::Metadata;
use crate::opts::{CodecOptions, DecodeOpts, EncodeOpts, IccPolicy, MetadataPolicy};
use crate::plane::{linear_plane_to_srgb8, RgbaPlane};
use crate::registry::{
    Codec, CodecCaps, CodecId, CompatAction, CompatReport, ExportImage, Imported, ImportedLayer,
    LicenseStatus,
};

pub struct WebpCodec;

fn map_webp_err(e: image_webp::DecodingError) -> CodecError {
    use image_webp::DecodingError as E;
    match e {
        E::IoError(e) => CodecError::io_read(e),
        other => CodecError::corrupt(other),
    }
}

impl Codec for WebpCodec {
    fn id(&self) -> CodecId {
        CodecId("webp")
    }

    fn capabilities(&self) -> CodecCaps {
        CodecCaps {
            read: true,
            write: true,
            layered: false,
            max_depth: appname_core::ChannelKind::U8,
            alpha: true,
            animation: true,
            license: LicenseStatus::Builtin,
        }
    }

    fn sniff(&self, head: &[u8]) -> bool {
        head.len() >= 12 && &head[0..4] == b"RIFF" && &head[8..12] == b"WEBP"
    }

    fn magic_len(&self) -> u32 {
        12
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["webp"]
    }

    fn decode(&self, bytes: &[u8], opts: &DecodeOpts) -> Result<Imported, CodecError> {
        opts.check_interim_unsupported()?;
        let mut report = CompatReport::default();
        let mut dec = WebPDecoder::new(Cursor::new(bytes)).map_err(map_webp_err)?;
        let (w, h) = dec.dimensions();
        opts.check_limits(w, h)?;
        let icc = match dec.icc_profile() {
            Ok(v) => v.filter(|v| !v.is_empty()),
            Err(e) => drop_bad_metadata(&mut report, "icc", map_webp_err(e)),
        };
        let exif = match dec.exif_metadata() {
            Ok(v) => v.filter(|v| !v.is_empty()),
            Err(e) => drop_bad_metadata(&mut report, "exif", map_webp_err(e)),
        };
        let xmp = match dec.xmp_metadata() {
            Ok(v) => v.filter(|v| !v.is_empty()),
            Err(e) => drop_bad_metadata(&mut report, "xmp", map_webp_err(e)),
        };
        let has_alpha = dec.has_alpha();
        let buf_len = dec
            .output_buffer_size()
            .ok_or_else(|| CodecError::corrupt("output size overflow"))?;
        let to_plane = |data: Vec<u8>| -> Result<RgbaPlane, CodecError> {
            if has_alpha {
                RgbaPlane::new8(w, h, data)
            } else {
                let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
                for px in data.chunks_exact(3) {
                    rgba.extend_from_slice(&[px[0], px[1], px[2], u8::MAX]);
                }
                RgbaPlane::new8(w, h, rgba)
            }
        };

        if dec.is_animated() && opts.frames_as_layers {
            // §8.2: every layer is the complete visible frame — the
            // decoder hands back fully-composed canvases per frame.
            let n = dec.num_frames();
            let loop_count = match dec.loop_count() {
                image_webp::LoopCount::Forever => 0u16,
                image_webp::LoopCount::Times(t) => u16::from(t),
            };
            let mut layers = Vec::new();
            let mut first_plane: Option<RgbaPlane> = None;
            for k in 0..n {
                let mut data = vec![0u8; buf_len];
                let delay_ms = dec.read_frame(&mut data).map_err(map_webp_err)?;
                let plane = to_plane(data)?;
                if k == 0 {
                    first_plane = Some(plane.clone());
                }
                let (frame_png, frame_buffer) = plane.to_png_and_buffer(opts.working_profile)?;
                layers.push(ImportedLayer {
                    name: format!("frame:{k} delay:{delay_ms}"),
                    buffer: frame_buffer,
                    ingress_png: frame_png,
                    offset: appname_core::IVec2::new(0, 0),
                    opacity: 1.0,
                    fill_opacity: 1.0,
                    blend: appname_core::BlendMode::Normal,
                    visible: true,
                    clip_to_below: false,
                    locks: crate::registry::LayerLocks::default(),
                    group_path: Vec::new(),
                    icc: None,
                });
            }
            let first =
                first_plane.ok_or_else(|| CodecError::corrupt("animation with zero frames"))?;
            // Animated WebP: EXIF orientation bake would have to permute
            // every frame AND the canvas — carried unbaked with a report
            // note instead (honest, typed surface; §8.6 bake applies to
            // stills).
            if exif.is_some() {
                report.push(
                    "metadata.exif.orientation",
                    CompatAction::Info,
                    "orientation not baked for animated decodes at this slice",
                );
            }
            let meta = Metadata {
                icc: icc.clone(),
                exif: if opts.keep_unknown_metadata {
                    exif
                } else {
                    None
                },
                xmp: if opts.keep_unknown_metadata {
                    xmp
                } else {
                    None
                },
                iptc: None,
                animation_loop: Some(loop_count),
            };
            let (first_png, first_buffer) = first.to_png_and_buffer(opts.working_profile)?;
            return Ok(Imported {
                buffer: first_buffer,
                ingress_png: first_png,
                size: (w, h),
                profile: meta.icc.clone(),
                meta,
                layers: Some(layers),
                orientation_applied: None,
                report,
                compat_report: None,
            });
        }

        let mut data = vec![0u8; buf_len];
        dec.read_image(&mut data).map_err(map_webp_err)?;
        let plane = to_plane(data)?;
        let mut imported = shared::finish_single(plane, exif, icc, xmp, opts, report)?;
        if !opts.keep_unknown_metadata {
            imported.meta.exif = None;
            imported.meta.xmp = None;
        }
        Ok(imported)
    }

    fn encode(&self, img: &ExportImage<'_>, opts: &EncodeOpts) -> Result<Vec<u8>, CodecError> {
        let CodecOptions::Webp(o) = &opts.format else {
            return Err(CodecError::internal("webp codec handed non-webp options"));
        };
        o.validate()?;
        opts.check_interim_unsupported()?;
        if !o.lossless {
            return Err(CodecError::unsupported(
                "webp.lossy_encode (elected pure-Rust encoder is VP8L lossless only)",
            ));
        }
        let rgba = linear_plane_to_srgb8(img.width, img.height, img.plane)?;
        let mut out = Vec::new();
        let mut enc = WebPEncoder::new(&mut out);
        if opts.icc == IccPolicy::Embed {
            if let Some(profile) = img.profile {
                enc.set_icc_profile(profile.to_vec());
            }
        }
        if opts.metadata == MetadataPolicy::Preserve {
            if let Some(exif) = &img.meta.exif {
                enc.set_exif_metadata(exif.clone());
            }
            if let Some(xmp) = &img.meta.xmp {
                enc.set_xmp_metadata(xmp.clone());
            }
        }
        enc.encode(&rgba, img.width, img.height, WebpColor::Rgba8)
            .map_err(|e| CodecError::internal(format!("webp encode: {e}")))?;
        Ok(out)
    }
}
