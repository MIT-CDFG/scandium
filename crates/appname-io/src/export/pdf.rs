// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §29.3 "PDF export [NORMATIVE]" — target PDF 1.7; raster
//! images FlateDecode (lossless, PNG predictor class — v1 writes plain
//! Flate, predictor 1) by default or DCTDecode with JPEG quality ≥ 90
//! as the explicit smaller-file option (the dialog NEVER offers lower);
//! the destination profile embedded as an `/OutputIntents` entry with
//! per-image `ICCBased` color spaces. Acceptance: W11 steps 3–4, W14
//! step 8 (the raster arms).
//!
//! ## v1 scope (the CP-1.0 honest cut, recorded in the lane log)
//!
//! * **Raster PDF**: the flattened composite as one full-page image —
//!   real, deterministic, structurally complete.
//! * **Text arm**: §29.3 native PDF text needs shaped glyph runs +
//!   subset CID font embedding — SERVICELESS until O-TEXTVEC's
//!   TextLayout service wires into the assembly ([`text_arm`] answers
//!   typed naming exactly that trigger).
//! * **Layered OCG arm**: needs per-top-level-layer composite planes
//!   from the flatten stage ([`layered_arm`], typed; joins when the
//!   flatten door grows per-layer readback).
//! * **Alpha**: v1 composites over the §8.1 matte (SMask streams are a
//!   recorded deferral); print-oriented PDF is opaque anyway.
//!
//! ## Own emitter — the recorded DEPS decision
//!
//! No PDF crate is consumed: the §29.3 v1 surface (header, 8-object
//! body, xref, one image stream, OutputIntent) is ~300 lines against a
//! stable public file format, deterministic by construction (ordered
//! objects, fixed flate level, zeroed ids), while every surveyed Rust
//! PDF crate is record-less and brings a dependency tree for features
//! (fonts, forms, encryption) v1 must not claim. Census-style
//! reasoning + the revisit trigger (the text arm may justify a real
//! dependency) live in the lane log; drafted record routes to O-BUILD
//! only if that trigger fires.
//!
//! Determinism: object order fixed, offsets computed, flate level
//! pinned (6, same as the package writer), no timestamps, no /ID.

use appname_color::{BuiltinSpace, Cms};

use crate::error::CodecError;
use crate::export::icc_synth;
use crate::export::pipeline;
use crate::export::preset::ProfilePolicy;
use crate::export::ExportSource;
use crate::opts::{CodecOptions, JpegOptions};
use crate::plane::linear_plane_to_srgb8_over_matte;

/// §29.3 raster stream election.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PdfImageEncoding {
    /// FlateDecode — lossless, the default.
    Flate,
    /// DCTDecode with quality ≥ 90 (the dialog never offers lower).
    Dct { quality: u8 },
}

/// PDF export options (the §29.3 export half; print composition —
/// strips, marks, scaling — is the print wave's arm).
#[derive(Debug, Clone)]
pub struct PdfExportOptions {
    pub encoding: PdfImageEncoding,
    /// §29.1 stage-3 policy for the image pixels; the destination
    /// profile (converted-to space, or the source/interim profile under
    /// `Embed`) becomes the `/OutputIntents` entry.
    pub profile_policy: ProfilePolicy,
    /// Print chain (§29.3/W11): destination printer/paper profile BYTES;
    /// when set, pixels convert working → this profile through the Cms
    /// and THESE bytes ride `/OutputIntents` (overrides
    /// `profile_policy`).
    pub print_profile: Option<Vec<u8>>,
    /// Matte for alpha flattening (linear RGB; §8.1 convention).
    pub matte: [f32; 3],
}

impl Default for PdfExportOptions {
    fn default() -> PdfExportOptions {
        PdfExportOptions {
            encoding: PdfImageEncoding::Flate,
            profile_policy: ProfilePolicy::Embed,
            print_profile: None,
            matte: [1.0, 1.0, 1.0],
        }
    }
}

/// The §29.3 text arm — typed until O-TEXTVEC's TextLayout service
/// lands in the assembly (native PDF text needs shaped runs + §23.5
/// subset CID embedding).
pub fn text_arm() -> CodecError {
    CodecError::unsupported(
        "pdf.text (native PDF text arrives with the O-TEXTVEC TextLayout service wiring; \
         v1 rasterizes text with the composite)",
    )
}

/// The §29.3 layered-OCG arm — typed until the flatten door grows
/// per-top-level-layer composite readback.
pub fn layered_arm() -> CodecError {
    CodecError::unsupported(
        "pdf.layers (OCG per top-level layer needs per-layer composite planes from the \
         flatten stage; v1 writes the flattened page)",
    )
}

/// Write a single-page raster PDF 1.7 from the flattened source.
/// Page geometry: `MediaBox = px / ppi * 72` (§0.3 printed size).
pub fn write_raster_pdf(
    source: &ExportSource<'_>,
    opts: &PdfExportOptions,
    cms: &mut Cms,
) -> Result<Vec<u8>, CodecError> {
    source.check()?;
    if let PdfImageEncoding::Dct { quality } = opts.encoding {
        if quality < 90 {
            return Err(CodecError::OptionOutOfRange {
                field: "pdf.dct.quality",
                detail: format!("{quality} < 90 (§29.3: the dialog never offers lower for PDF)"),
            });
        }
    }

    // ---- stage 3: pixels + the destination profile ----
    // Ride the pipeline's convert machinery by encoding a variant?  No:
    // PDF needs the raw converted plane, so call the same policy arms
    // directly through a scratch preset when converting.
    let (plane_owned, intent_profile): (Option<Vec<f32>>, Vec<u8>) =
        if let Some(dest_bytes) = &opts.print_profile {
            let converted = convert_via(source, cms, dest_bytes)?;
            (Some(converted), dest_bytes.clone())
        } else {
            match &opts.profile_policy {
                ProfilePolicy::Embed => (
                    None,
                    source
                        .icc
                        .map(<[u8]>::to_vec)
                        // interim assumption documented at the crate seam:
                        // untagged working = sRGB; the intent entry states it
                        .unwrap_or_else(|| icc_synth::builtin_icc_bytes(BuiltinSpace::Srgb)),
                ),
                ProfilePolicy::ConvertToSrgbStrip | ProfilePolicy::ConvertTo(_) => {
                    let space = match opts.profile_policy {
                        ProfilePolicy::ConvertTo(s) => s,
                        _ => BuiltinSpace::Srgb,
                    };
                    let p = convert_builtin(source, cms, space)?;
                    // a PDF always carries its OutputIntent — "strip"
                    // affects file metadata, not the §29.3 intent entry
                    (Some(p), icc_synth::builtin_icc_bytes(space))
                }
            }
        };
    let plane: &[f32] = plane_owned.as_deref().unwrap_or(source.plane);

    // ---- image bytes ----
    let (w, h) = (source.width, source.height);
    let rgb = linear_plane_to_srgb8_over_matte(w, h, plane, opts.matte)?;
    let (img_bytes, filter): (Vec<u8>, &str) = match opts.encoding {
        PdfImageEncoding::Flate => {
            let mut enc = flate2::write::ZlibEncoder::new(
                Vec::new(),
                flate2::Compression::new(6), // pinned (module doc)
            );
            std::io::Write::write_all(&mut enc, &rgb)
                .and_then(|_| enc.finish())
                .map(|b| (b, "FlateDecode"))
                .map_err(|e| CodecError::internal(format!("pdf flate: {e}")))?
        }
        PdfImageEncoding::Dct { quality } => {
            // the elected §8.2 JPEG encoder at the explicit quality
            let registry = crate::registry::CodecRegistry::builtin();
            let codec = registry
                .by_id("jpeg")
                .ok_or_else(|| CodecError::internal("jpeg codec unregistered"))?;
            let meta = crate::meta::Metadata::default();
            let img = crate::registry::ExportImage {
                width: w,
                height: h,
                plane,
                layers: None,
                profile: None,
                meta: &meta,
            };
            let mut eopts = crate::opts::EncodeOpts::new(CodecOptions::Jpeg(JpegOptions {
                quality,
                ..JpegOptions::default()
            }));
            eopts.metadata = crate::opts::MetadataPolicy::StripAll;
            eopts.icc = crate::opts::IccPolicy::Omit;
            eopts.matte = opts.matte;
            (codec.encode(&img, &eopts)?, "DCTDecode")
        }
    };

    // ---- ICC stream (flate, pinned level) ----
    let icc_flate = {
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(6));
        std::io::Write::write_all(&mut enc, &intent_profile)
            .and_then(|_| enc.finish())
            .map_err(|e| CodecError::internal(format!("pdf icc flate: {e}")))?
    };

    // ---- assemble (fixed object order; offsets tracked for xref) ----
    let page_w = w as f64 / source.ppi * 72.0;
    let page_h = h as f64 / source.ppi * 72.0;
    let content = format!("q\n{page_w:.4} 0 0 {page_h:.4} 0 0 cm\n/Im0 Do\nQ\n");

    let mut pdf: Vec<u8> = Vec::with_capacity(img_bytes.len() + icc_flate.len() + 2048);
    pdf.extend_from_slice(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n");
    let mut offsets: Vec<usize> = Vec::new();
    let push_obj = |pdf: &mut Vec<u8>, offsets: &mut Vec<usize>, body: &[u8]| {
        offsets.push(pdf.len());
        let n = offsets.len();
        pdf.extend_from_slice(format!("{n} 0 obj\n").as_bytes());
        pdf.extend_from_slice(body);
        pdf.extend_from_slice(b"\nendobj\n");
    };

    // 1: catalog
    push_obj(
        &mut pdf,
        &mut offsets,
        b"<< /Type /Catalog /Pages 2 0 R /OutputIntents [5 0 R] >>",
    );
    // 2: pages
    push_obj(
        &mut pdf,
        &mut offsets,
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    );
    // 3: page
    push_obj(
        &mut pdf,
        &mut offsets,
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {page_w:.4} {page_h:.4}] \
             /Resources << /XObject << /Im0 6 0 R >> >> /Contents 4 0 R >>"
        )
        .as_bytes(),
    );
    // 4: content stream
    let mut body = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
    body.extend_from_slice(content.as_bytes());
    body.extend_from_slice(b"endstream");
    push_obj(&mut pdf, &mut offsets, &body);
    // 5: output intent (§29.3: the destination profile entry). Subtype:
    // a second-class name — the standard subtypes (GTS_PDFX/GTS_PDFA1)
    // assert standards conformance v1 does not attest; the print wave's
    // PDF/X election may upgrade it.
    push_obj(
        &mut pdf,
        &mut offsets,
        b"<< /Type /OutputIntent /S /Appname /OutputConditionIdentifier (Custom) \
          /Info (Appname export destination profile) /DestOutputProfile 7 0 R >>",
    );
    // 6: the image
    let mut body = format!(
        "<< /Type /XObject /Subtype /Image /Width {w} /Height {h} \
         /ColorSpace [/ICCBased 7 0 R] /BitsPerComponent 8 /Filter /{filter} \
         /Length {} >>\nstream\n",
        img_bytes.len()
    )
    .into_bytes();
    body.extend_from_slice(&img_bytes);
    body.extend_from_slice(b"\nendstream");
    push_obj(&mut pdf, &mut offsets, &body);
    // 7: the ICC stream (/N 3: RGB)
    let mut body = format!(
        "<< /N 3 /Filter /FlateDecode /Length {} >>\nstream\n",
        icc_flate.len()
    )
    .into_bytes();
    body.extend_from_slice(&icc_flate);
    body.extend_from_slice(b"\nendstream");
    push_obj(&mut pdf, &mut offsets, &body);

    // xref + trailer
    let xref_off = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n", offsets.len() + 1).as_bytes());
    pdf.extend_from_slice(b"0000000000 65535 f \n");
    for off in &offsets {
        pdf.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_off}\n%%EOF\n",
            offsets.len() + 1
        )
        .as_bytes(),
    );
    Ok(pdf)
}

/// working → builtin destination (the pipeline's stage-3 machinery).
fn convert_builtin(
    source: &ExportSource<'_>,
    cms: &mut Cms,
    space: BuiltinSpace,
) -> Result<Vec<f32>, CodecError> {
    pipeline::convert_plane_for_pdf(source, cms, pipeline::ConvertDest::Builtin(space))
}

/// working → printer/paper profile bytes (the §29.3/W11 print chain).
fn convert_via(
    source: &ExportSource<'_>,
    cms: &mut Cms,
    dest_icc: &[u8],
) -> Result<Vec<f32>, CodecError> {
    pipeline::convert_plane_for_pdf(source, cms, pipeline::ConvertDest::IccBytes(dest_icc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::Metadata;

    fn source_plane(w: u32, h: u32) -> Vec<f32> {
        let mut p = Vec::new();
        for i in 0..(w * h) {
            let v = (i % 7) as f32 / 7.0;
            p.extend_from_slice(&[v, 1.0 - v, 0.5, 1.0]);
        }
        p
    }

    #[test]
    fn structural_asserts_header_intent_flate() {
        let plane = source_plane(32, 16);
        let meta = Metadata::default();
        let s = ExportSource {
            width: 32,
            height: 16,
            plane: &plane,
            icc: None,
            meta: &meta,
            doc_name: "d",
            ppi: 72.0,
        };
        let mut cms = Cms::new();
        let pdf = write_raster_pdf(&s, &PdfExportOptions::default(), &mut cms).unwrap();
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.starts_with("%PDF-1.7"), "W11 step 4: version 1.7");
        assert!(text.contains("/OutputIntents"), "W11 step 4: intent entry");
        assert!(
            text.contains("/DestOutputProfile"),
            "destination profile carried"
        );
        assert!(
            text.contains("/Filter /FlateDecode"),
            "W11 step 4: Flate streams"
        );
        assert!(text.contains("/ICCBased"), "§29.3 per-image ICCBased space");
        assert!(text.ends_with("%%EOF\n"));
        // page geometry: 32 px @ 72 ppi = 32 pt
        assert!(text.contains("/MediaBox [0 0 32.0000 16.0000]"));
    }

    #[test]
    fn deterministic_across_runs() {
        let plane = source_plane(16, 16);
        let meta = Metadata::default();
        let s = ExportSource {
            width: 16,
            height: 16,
            plane: &plane,
            icc: None,
            meta: &meta,
            doc_name: "d",
            ppi: 300.0,
        };
        let mut cms = Cms::new();
        let a = write_raster_pdf(&s, &PdfExportOptions::default(), &mut cms).unwrap();
        let b = write_raster_pdf(&s, &PdfExportOptions::default(), &mut cms).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn dct_below_90_refuses_typed() {
        let plane = source_plane(8, 8);
        let meta = Metadata::default();
        let s = ExportSource {
            width: 8,
            height: 8,
            plane: &plane,
            icc: None,
            meta: &meta,
            doc_name: "d",
            ppi: 72.0,
        };
        let mut cms = Cms::new();
        let e = write_raster_pdf(
            &s,
            &PdfExportOptions {
                encoding: PdfImageEncoding::Dct { quality: 82 },
                ..PdfExportOptions::default()
            },
            &mut cms,
        )
        .unwrap_err();
        assert!(matches!(e, CodecError::OptionOutOfRange { .. }), "{e}");
    }

    #[test]
    fn dct_at_90_writes_dctdecode() {
        let plane = source_plane(16, 8);
        let meta = Metadata::default();
        let s = ExportSource {
            width: 16,
            height: 8,
            plane: &plane,
            icc: None,
            meta: &meta,
            doc_name: "d",
            ppi: 72.0,
        };
        let mut cms = Cms::new();
        let pdf = write_raster_pdf(
            &s,
            &PdfExportOptions {
                encoding: PdfImageEncoding::Dct { quality: 92 },
                ..PdfExportOptions::default()
            },
            &mut cms,
        )
        .unwrap();
        assert!(String::from_utf8_lossy(&pdf).contains("/Filter /DCTDecode"));
    }

    #[test]
    fn xref_offsets_point_at_objects() {
        let plane = source_plane(8, 8);
        let meta = Metadata::default();
        let s = ExportSource {
            width: 8,
            height: 8,
            plane: &plane,
            icc: None,
            meta: &meta,
            doc_name: "d",
            ppi: 72.0,
        };
        let mut cms = Cms::new();
        let pdf = write_raster_pdf(&s, &PdfExportOptions::default(), &mut cms).unwrap();
        let text = String::from_utf8_lossy(&pdf);
        // every "N 0 obj" begins exactly at its xref offset
        let xref_at = text.rfind("xref\n").unwrap();
        let lines: Vec<&str> = text[xref_at..].lines().skip(3).take(7).collect();
        for (i, line) in lines.iter().enumerate() {
            let off: usize = line[..10].parse().unwrap();
            let expect = format!("{} 0 obj", i + 1);
            assert!(
                pdf[off..].starts_with(expect.as_bytes()),
                "object {} offset {off}",
                i + 1
            );
        }
    }

    #[test]
    fn typed_ni_arms_name_their_triggers() {
        assert!(text_arm().to_string().contains("TextLayout"));
        assert!(layered_arm().to_string().contains("per-layer"));
    }
}
