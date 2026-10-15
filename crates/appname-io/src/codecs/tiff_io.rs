// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §8.2 (TIFF row: read/write, 8/16/32-bit, alpha, multi-page →
//! `page:{n}` layers, LZW/ZIP/none; ICC tag 34675, EXIF IFD via 34665, XMP
//! tag 700 per the carrier table); §8.6 (orientation tag 274 lives in
//! IFD0 directly); DD ⑫.
//!
//! ELECTION: the pure-Rust `tiff` crate per its deps record (0.11 line),
//! feature-gated to deflate+lzw (drops the record-external fax/JPEG-in-TIFF
//! transitive surface; those streams answer typed `UnsupportedFeature`
//! from the decoder). Slice-1 depth posture under the interim color
//! assumption ([`appname_core::io::srgb`] module doc): 8-bit imports ride
//! the 8-bit ingestion hop, 16-bit the 16-bit hop (precision preserved
//! into linear f32); 32-bit float and CMYK/Lab answer typed
//! (`DepthUnsupported`/`ModelUnsupported`) until the §5/§6 CMM + mode
//! pipeline lands — flagged handoffs, never silent. Export writes 8-bit
//! RGBA strips (chunky); 16/32-bit export joins the depth pipeline;
//! `planar: true` and `tile_size: Some(_)` answer typed.

use std::io::Cursor;

use tiff::decoder::{Decoder, DecodingResult, Limits};
use tiff::encoder::{colortype, Compression, DeflateLevel, TiffEncoder};
use tiff::tags::Tag;

use crate::codecs::shared::drop_bad_metadata;
use crate::error::CodecError;
use crate::meta::Metadata;
use crate::opts::{BigTiff, CodecOptions, DecodeOpts, EncodeOpts, IccPolicy, TiffCompression};
use crate::plane::{linear_plane_to_srgb8, RgbaPlane};
use crate::registry::{
    Codec, CodecCaps, CodecId, CompatAction, CompatReport, ExportImage, Imported, ImportedLayer,
    LicenseStatus,
};

pub struct TiffCodec;

const TAG_ICC: u16 = 34675;
const TAG_XMP: u16 = 700;

fn map_tiff_err(e: tiff::TiffError) -> CodecError {
    match e {
        tiff::TiffError::FormatError(d) => CodecError::corrupt(d),
        tiff::TiffError::UnsupportedError(d) => CodecError::unsupported(format!("tiff: {d}")),
        tiff::TiffError::IoError(e) => CodecError::io_read(e),
        tiff::TiffError::LimitsExceeded => {
            CodecError::corrupt("decoder limits exceeded before the §8.1 gate")
        }
        other => CodecError::corrupt(other),
    }
}

/// One decoded page as an interchange plane (+ per-IFD orientation).
fn read_page(
    dec: &mut Decoder<Cursor<&[u8]>>,
    opts: &DecodeOpts,
    report: &mut CompatReport,
) -> Result<(RgbaPlane, Option<u16>), CodecError> {
    let (w, h) = dec.dimensions().map_err(map_tiff_err)?;
    opts.check_limits(w, h)?;
    let color = dec.colortype().map_err(map_tiff_err)?;
    use tiff::ColorType as Ct;
    // Model/depth gates first (typed; interim posture in the module doc).
    match color {
        Ct::Gray(8 | 16) | Ct::GrayA(8 | 16) | Ct::RGB(8 | 16) | Ct::RGBA(8 | 16) => {}
        Ct::Gray(32) | Ct::GrayA(32) | Ct::RGB(32) | Ct::RGBA(32) => {
            return Err(CodecError::DepthUnsupported {
                kind: appname_core::ChannelKind::F32,
            })
        }
        Ct::CMYK(_) | Ct::CMYKA(_) => {
            return Err(CodecError::ModelUnsupported {
                model: appname_core::ColorModelKind::Cmyk,
            })
        }
        other => {
            return Err(CodecError::unsupported(format!(
                "tiff color type {other:?}"
            )))
        }
    }
    let channels = match color {
        Ct::Gray(_) => 1usize,
        Ct::GrayA(_) => 2,
        Ct::RGB(_) => 3,
        Ct::RGBA(_) => 4,
        _ => unreachable!(),
    };
    let expand8 = |data: &[u8]| -> Vec<u8> {
        let mut out = Vec::with_capacity(w as usize * h as usize * 4);
        for px in data.chunks_exact(channels) {
            let (r, g, b, a) = match channels {
                1 => (px[0], px[0], px[0], u8::MAX),
                2 => (px[0], px[0], px[0], px[1]),
                3 => (px[0], px[1], px[2], u8::MAX),
                _ => (px[0], px[1], px[2], px[3]),
            };
            out.extend_from_slice(&[r, g, b, a]);
        }
        out
    };
    let expand16 = |data: &[u16]| -> Vec<u16> {
        let mut out = Vec::with_capacity(w as usize * h as usize * 4);
        for px in data.chunks_exact(channels) {
            let (r, g, b, a) = match channels {
                1 => (px[0], px[0], px[0], u16::MAX),
                2 => (px[0], px[0], px[0], px[1]),
                3 => (px[0], px[1], px[2], u16::MAX),
                _ => (px[0], px[1], px[2], px[3]),
            };
            out.extend_from_slice(&[r, g, b, a]);
        }
        out
    };
    let plane = match dec.read_image().map_err(map_tiff_err)? {
        DecodingResult::U8(data) => {
            if data.len() != w as usize * h as usize * channels {
                return Err(CodecError::corrupt("tiff payload length mismatch"));
            }
            RgbaPlane::new8(w, h, expand8(&data))?
        }
        DecodingResult::U16(data) => {
            if data.len() != w as usize * h as usize * channels {
                return Err(CodecError::corrupt("tiff payload length mismatch"));
            }
            RgbaPlane::new16(w, h, expand16(&data))?
        }
        _ => {
            return Err(CodecError::DepthUnsupported {
                kind: appname_core::ChannelKind::F32,
            })
        }
    };
    // §8.6: orientation tag 274 sits in the page's own IFD.
    let orientation = match dec.find_tag(Tag::Orientation) {
        Ok(Some(v)) => match v.into_u16() {
            Ok(o) => Some(o),
            Err(e) => drop_bad_metadata(report, "exif.orientation", e),
        },
        Ok(None) => None,
        Err(e) => drop_bad_metadata(report, "exif.orientation", map_tiff_err(e)),
    };
    Ok((plane, orientation))
}

impl Codec for TiffCodec {
    fn id(&self) -> CodecId {
        CodecId("tiff")
    }

    fn capabilities(&self) -> CodecCaps {
        CodecCaps {
            read: true,
            write: true,
            layered: true,
            max_depth: appname_core::ChannelKind::F32,
            alpha: true,
            animation: false,
            license: LicenseStatus::Builtin,
        }
    }

    fn sniff(&self, head: &[u8]) -> bool {
        head.starts_with(b"II\x2A\x00") || head.starts_with(b"MM\x00\x2A")
    }

    fn magic_len(&self) -> u32 {
        4
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["tif", "tiff"]
    }

    fn decode(&self, bytes: &[u8], opts: &DecodeOpts) -> Result<Imported, CodecError> {
        opts.check_interim_unsupported()?;
        let mut report = CompatReport::default();
        let mut dec = Decoder::new(Cursor::new(bytes))
            .map_err(map_tiff_err)?
            .with_limits(Limits::unlimited()); // the §8.1 gate below governs
                                               // Metadata from the FIRST IFD (the primary image's).
        let icc = match dec.find_tag_unsigned_vec::<u8>(Tag::Unknown(TAG_ICC)) {
            Ok(v) => v,
            Err(e) => drop_bad_metadata(&mut report, "icc", map_tiff_err(e)),
        };
        let xmp = match dec.find_tag_unsigned_vec::<u8>(Tag::Unknown(TAG_XMP)) {
            Ok(v) => v,
            Err(e) => drop_bad_metadata(&mut report, "xmp", map_tiff_err(e)),
        };

        // Page loop (multi-page files import each IFD as a layer named
        // `page:{n}`, n = 0-based IFD index, §8.2).
        let mut pages: Vec<(RgbaPlane, Option<u16>)> = Vec::new();
        loop {
            pages.push(read_page(&mut dec, opts, &mut report)?);
            if !opts.frames_as_layers || !dec.more_images() {
                break;
            }
            dec.next_image().map_err(map_tiff_err)?;
        }

        // Orientation bake per page (§8.6; value capture from page 0).
        let mut orientation_applied = None;
        let mut baked: Vec<RgbaPlane> = Vec::with_capacity(pages.len());
        for (i, (plane, orient)) in pages.into_iter().enumerate() {
            let mut plane = plane;
            if let Some(v) = orient {
                if (1..=8).contains(&v) {
                    if opts.apply_orientation {
                        plane = plane.apply_orientation(v)?;
                        if i == 0 {
                            orientation_applied = Some(v);
                        }
                    }
                } else {
                    report.push(
                        "metadata.exif.orientation",
                        CompatAction::Approximated,
                        format!("orientation {v} outside 1..=8 treated as 1 (page {i})"),
                    );
                    if i == 0 {
                        orientation_applied = Some(1);
                    }
                }
            }
            baked.push(plane);
        }

        let meta = Metadata {
            icc: icc.clone(),
            exif: None, // TIFF: the file itself is the TIFF structure; baseline tags re-derive on export
            xmp: if opts.keep_unknown_metadata {
                xmp
            } else {
                None
            },
            iptc: None,
            animation_loop: None,
        };
        let size = (baked[0].width, baked[0].height);
        let (ingress_png, buffer) = baked[0].to_png_and_buffer(opts.working_profile)?;
        let layers = if baked.len() > 1 {
            let mut layers = Vec::with_capacity(baked.len());
            for (n, plane) in baked.iter().enumerate() {
                let (page_png, page_buffer) = plane.to_png_and_buffer(opts.working_profile)?;
                layers.push(ImportedLayer {
                    name: format!("page:{n}"),
                    buffer: page_buffer,
                    ingress_png: page_png,
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
        Ok(Imported {
            buffer,
            ingress_png,
            size,
            profile: meta.icc.clone(),
            meta,
            layers,
            orientation_applied,
            report,
            compat_report: None,
        })
    }

    fn encode(&self, img: &ExportImage<'_>, opts: &EncodeOpts) -> Result<Vec<u8>, CodecError> {
        let CodecOptions::Tiff(o) = &opts.format else {
            return Err(CodecError::internal("tiff codec handed non-tiff options"));
        };
        opts.check_interim_unsupported()?;
        if o.planar {
            return Err(CodecError::unsupported(
                "tiff.planar (chunky only at this slice)",
            ));
        }
        if o.tile_size.is_some() {
            return Err(CodecError::unsupported(
                "tiff.tile_size (strips only at this slice)",
            ));
        }
        match o.bigtiff {
            BigTiff::Auto | BigTiff::Never => {}
            BigTiff::Force => {
                return Err(CodecError::unsupported(
                    "tiff.bigtiff Force (joins the >4GiB export path)",
                ))
            }
        }
        let compression = match o.compression {
            TiffCompression::None => Compression::Uncompressed,
            TiffCompression::Lzw => Compression::Lzw,
            TiffCompression::Zip => Compression::Deflate(DeflateLevel::Balanced),
        };
        let rgba = linear_plane_to_srgb8(img.width, img.height, img.plane)?;
        let mut out = Cursor::new(Vec::new());
        {
            let mut enc = TiffEncoder::new(&mut out)
                .map_err(map_tiff_err)?
                .with_compression(compression);
            let mut image = enc
                .new_image::<colortype::RGBA8>(img.width, img.height)
                .map_err(map_tiff_err)?;
            if opts.icc == IccPolicy::Embed {
                if let Some(profile) = img.profile {
                    image
                        .encoder()
                        .write_tag(Tag::Unknown(TAG_ICC), profile)
                        .map_err(map_tiff_err)?;
                }
            }
            if opts.metadata == crate::opts::MetadataPolicy::Preserve {
                if let Some(xmp) = &img.meta.xmp {
                    image
                        .encoder()
                        .write_tag(Tag::Unknown(TAG_XMP), xmp.as_slice())
                        .map_err(map_tiff_err)?;
                }
            }
            if let Some(ppi) = opts.ppi {
                use tiff::encoder::Rational;
                use tiff::tags::ResolutionUnit;
                image.resolution(
                    ResolutionUnit::Inch,
                    Rational {
                        n: ppi.round().max(1.0) as u32,
                        d: 1,
                    },
                );
            }
            image.write_data(&rgba).map_err(map_tiff_err)?;
        }
        Ok(out.into_inner())
    }
}

/// Test/fixture helper: does this TIFF byte stream carry an ICC tag equal
/// to `profile`? (Round-trip carriage checks without re-entering decode.)
pub fn tiff_icc(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut dec = Decoder::new(Cursor::new(bytes)).ok()?;
    dec.find_tag_unsigned_vec::<u8>(Tag::Unknown(TAG_ICC))
        .ok()?
}

// Re-exported for the orientation fixtures: build a tiny TIFF with an
// orientation tag via the encoder (tests write through the same elected
// crate).
#[doc(hidden)]
pub fn test_fixture_tiff_with_orientation(
    w: u32,
    h: u32,
    rgba: &[u8],
    orientation: u16,
) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    {
        let mut enc = TiffEncoder::new(&mut out).unwrap();
        let mut image = enc.new_image::<colortype::RGBA8>(w, h).unwrap();
        image
            .encoder()
            .write_tag(Tag::Orientation, orientation)
            .unwrap();
        image.write_data(rgba).unwrap();
    }
    out.into_inner()
}
