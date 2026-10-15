// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §8.2 (GIF row + "GIF frame ↔ layer mapping" [NORMATIVE] +
//! "Palette quantization" [NORMATIVE]); §13.5 (the 0.5 alpha iso-level);
//! DD ⑫.
//!
//! ELECTION: the `gif` crate per its deps record, default-features OFF —
//! drops the optional NeuQuant (`color_quant`) path because §8.2 names
//! modified median-cut + Floyd–Steinberg, implemented in
//! [`crate::quant`]. GIF's LZW-era rights posture is era-expired per the
//! record note.
//!
//! Import: each frame decodes to a FULL-canvas RGBA image — disposal
//! (none / restore-to-background / restore-to-previous) and transparency
//! applied first, so every layer is the complete visible frame, not the
//! stored delta. Frame *k* (delay *d* cs) becomes `frame:{k} delay:{d·10}`,
//! bottom-to-top. The loop count is surfaced for the consumer's
//! `meta.animation.loopCount`. XMP Application Extension is read-only per
//! the carrier table; GIF carries no ICC and no EXIF.
//!
//! Export: top-level layers matching the §8.2 pattern become frames; no
//! match → the flattened composite as a single frame; a partial match
//! skips non-matching layers with a report... (encode carries no report
//! channel at this slice — the skip is deterministic and documented);
//! duplicate `n` → `FrameMapping`. Alpha binarizes at 0.5 (§13.5).
//! Disposal on write: Background when the frame carries transparency,
//! else Keep — a deterministic rule for correct playback of full-canvas
//! frames.

use std::io::Cursor;

use gif::{ColorOutput, DecodeOptions, DisposalMethod, Encoder, Frame, Repeat};

use crate::error::CodecError;
use crate::meta::Metadata;
use crate::opts::{CodecOptions, DecodeOpts, EncodeOpts};
use crate::plane::{linear_plane_to_srgb8, RgbaPlane};
use crate::quant::{index_plane, median_cut_palette};
use crate::registry::{
    Codec, CodecCaps, CodecId, CompatReport, ExportImage, Imported, ImportedLayer, LicenseStatus,
};

pub struct GifCodec;

fn map_gif_err(e: gif::DecodingError) -> CodecError {
    match e {
        gif::DecodingError::Io(e) => CodecError::io_read(e),
        other => CodecError::corrupt(other),
    }
}

/// Parse a §8.2 frame-layer name: `frame:{n}[ delay:{ms}]` (the normative
/// grammar `^frame:(0|[1-9][0-9]*)( delay:([0-9]+))?$`, matched exactly,
/// no regex crate).
pub fn parse_frame_name(name: &str) -> Option<(u32, Option<u32>)> {
    let rest = name.strip_prefix("frame:")?;
    let (n_str, delay) = match rest.split_once(' ') {
        None => (rest, None),
        Some((n, tail)) => {
            let d = tail.strip_prefix("delay:")?;
            if d.is_empty() || !d.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            (n, Some(d.parse::<u32>().ok()?))
        }
    };
    if n_str.is_empty() || !n_str.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if n_str.len() > 1 && n_str.starts_with('0') {
        return None; // the grammar forbids leading zeros
    }
    Some((n_str.parse::<u32>().ok()?, delay))
}

impl Codec for GifCodec {
    fn id(&self) -> CodecId {
        CodecId("gif")
    }

    fn capabilities(&self) -> CodecCaps {
        CodecCaps {
            read: true,
            write: true,
            layered: false,
            max_depth: appname_core::ChannelKind::U8,
            alpha: true, // 1-bit
            animation: true,
            license: LicenseStatus::Builtin,
        }
    }

    fn sniff(&self, head: &[u8]) -> bool {
        head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a")
    }

    fn magic_len(&self) -> u32 {
        6
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["gif"]
    }

    fn decode(&self, bytes: &[u8], opts: &DecodeOpts) -> Result<Imported, CodecError> {
        opts.check_interim_unsupported()?;
        let mut report = CompatReport::default();
        let mut options = DecodeOptions::new();
        options.set_color_output(ColorOutput::RGBA);
        let mut dec = options.read_info(Cursor::new(bytes)).map_err(map_gif_err)?;
        let (w, h) = (u32::from(dec.width()), u32::from(dec.height()));
        opts.check_limits(w, h)?;
        let loop_count = match dec.repeat() {
            Repeat::Infinite => 0u16,
            Repeat::Finite(n) => n,
        };
        let xmp = dec.xmp_metadata().map(<[u8]>::to_vec);

        // Full-canvas composition per §8.2: disposal + transparency
        // applied so each layer is the complete visible frame.
        let (wu, hu) = (w as usize, h as usize);
        let mut canvas = vec![0u8; wu * hu * 4];
        let mut frames: Vec<(RgbaPlane, u32)> = Vec::new(); // (plane, delay ms)
        while let Some(frame) = dec.read_next_frame().map_err(map_gif_err)? {
            let saved = if frame.dispose == DisposalMethod::Previous {
                Some(canvas.clone())
            } else {
                None
            };
            // Blit the frame rect (RGBA output; a=0 means keep underlying).
            let (fx, fy) = (usize::from(frame.left), usize::from(frame.top));
            let (fw, fh) = (usize::from(frame.width), usize::from(frame.height));
            if fx + fw > wu || fy + fh > hu {
                return Err(CodecError::corrupt("frame rect outside the logical screen"));
            }
            for y in 0..fh {
                for x in 0..fw {
                    let s = (y * fw + x) * 4;
                    let px = &frame.buffer[s..s + 4];
                    if px[3] != 0 {
                        let d = ((fy + y) * wu + fx + x) * 4;
                        canvas[d..d + 4].copy_from_slice(px);
                    }
                }
            }
            let delay_ms = u32::from(frame.delay) * 10; // centiseconds -> ms
            frames.push((RgbaPlane::new8(w, h, canvas.clone())?, delay_ms));
            if !opts.frames_as_layers {
                break; // first frame only
            }
            // Disposal for the NEXT frame.
            match frame.dispose {
                DisposalMethod::Any | DisposalMethod::Keep => {}
                DisposalMethod::Background => {
                    for y in 0..fh {
                        let d = ((fy + y) * wu + fx) * 4;
                        canvas[d..d + fw * 4].fill(0);
                    }
                }
                DisposalMethod::Previous => {
                    if let Some(saved) = saved {
                        canvas = saved;
                    }
                }
            }
        }
        if frames.is_empty() {
            return Err(CodecError::corrupt("gif with no image data"));
        }

        let meta = Metadata {
            icc: None, // GIF carries no ICC (§8.2 carrier table)
            exif: None,
            xmp: if opts.keep_unknown_metadata {
                xmp
            } else {
                None
            },
            iptc: None,
            animation_loop: Some(loop_count),
        };
        let (ingress_png, buffer) = frames[0].0.to_png_and_buffer(opts.working_profile)?;
        let layers = if frames.len() > 1 {
            let mut layers = Vec::with_capacity(frames.len());
            for (k, (plane, delay_ms)) in frames.iter().enumerate() {
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
            Some(layers)
        } else {
            None
        };
        let _ = &mut report;
        Ok(Imported {
            buffer,
            ingress_png,
            size: (w, h),
            profile: None,
            meta,
            layers,
            orientation_applied: None, // GIF carries no EXIF
            report,
            compat_report: None,
        })
    }

    fn encode(&self, img: &ExportImage<'_>, opts: &EncodeOpts) -> Result<Vec<u8>, CodecError> {
        let CodecOptions::Gif(o) = &opts.format else {
            return Err(CodecError::internal("gif codec handed non-gif options"));
        };
        o.validate()?;
        opts.check_interim_unsupported()?;

        // Frame set per the §8.2 mapping contract.
        let mut frames: Vec<(u32, Option<u32>, &[f32])> = Vec::new();
        if let Some(layers) = &img.layers {
            let mut seen = std::collections::BTreeMap::new();
            for l in layers {
                if let Some((n, delay)) = parse_frame_name(l.name) {
                    if seen.insert(n, ()).is_some() {
                        return Err(CodecError::FrameMapping {
                            detail: format!("duplicate frame index {n}"),
                        });
                    }
                    frames.push((n, delay, l.plane));
                }
                // Non-matching layers are skipped, never silently mixed in
                // (§8.2) — deterministic; the Export-As report surfaces the
                // warning when that subsystem lands.
            }
            frames.sort_by_key(|(n, _, _)| *n);
        }
        // No matching layer → the flattened composite as a single frame
        // (§8.2); it is a still, not an animation.
        let flattened_still = frames.is_empty();
        if flattened_still {
            frames.push((0u32, None, img.plane));
        }

        let mut out = Vec::new();
        {
            let wu =
                u16::try_from(img.width).map_err(|_| CodecError::internal("gif width over u16"))?;
            let hu = u16::try_from(img.height)
                .map_err(|_| CodecError::internal("gif height over u16"))?;
            let mut enc = Encoder::new(&mut out, wu, hu, &[]).map_err(CodecError::internal)?;
            enc.set_repeat(if o.loop_count == 0 {
                Repeat::Infinite
            } else {
                Repeat::Finite(o.loop_count)
            })
            .map_err(CodecError::internal)?;

            for (_, delay, plane) in &frames {
                let rgba = linear_plane_to_srgb8(img.width, img.height, plane)?;
                let any_transparent = rgba.chunks_exact(4).any(|px| px[3] < 128);
                // §8.2 [NORMATIVE]: modified median-cut, then dither;
                // transparency takes one palette slot when present.
                let color_budget = usize::from(o.max_colors) - usize::from(any_transparent);
                let palette = median_cut_palette(&rgba, color_budget.max(1));
                let transparent_idx = if any_transparent {
                    Some(palette.len() as u8)
                } else {
                    None
                };
                let indices = index_plane(
                    &rgba,
                    img.width as usize,
                    img.height as usize,
                    &palette,
                    o.dither,
                    transparent_idx,
                );
                let mut pal_bytes: Vec<u8> = palette.iter().flatten().copied().collect();
                if any_transparent {
                    pal_bytes.extend_from_slice(&[0, 0, 0]); // the transparent slot
                }
                let delay_ms = delay.unwrap_or(o.default_delay_ms);
                let mut frame = Frame {
                    delay: u16::try_from((delay_ms + 5) / 10)
                        .map_err(|_| CodecError::internal("gif delay over u16 centiseconds"))?,
                    dispose: if any_transparent {
                        DisposalMethod::Background
                    } else {
                        DisposalMethod::Keep
                    },
                    transparent: transparent_idx,
                    needs_user_input: false,
                    top: 0,
                    left: 0,
                    width: wu,
                    height: hu,
                    interlaced: false,
                    palette: Some(pal_bytes),
                    buffer: std::borrow::Cow::Owned(indices),
                };
                if flattened_still {
                    frame.delay = 0;
                    frame.dispose = DisposalMethod::Any;
                }
                enc.write_frame(&frame).map_err(CodecError::internal)?;
            }
        }
        Ok(out)
    }
}
