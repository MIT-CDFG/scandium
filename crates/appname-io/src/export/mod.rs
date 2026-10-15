// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES Part 29 — the output subsystem (CP-1.0 §2.5 "Export
//! subsystem" row): §29.1 Export-As presets + the [NORMATIVE] 7-stage
//! export pipeline + the per-format option table, §29.2 web slices +
//! manifest, §29.3 (the EXPORT arm: profile policy + PDF), §29.4
//! metadata privacy; §26.2 naming grammar + collision policy +
//! path-traversal guard; §8.6 `MetadataPolicy`. Acceptance surface:
//! W15 (multi-asset slice export), W21 (headless export matrix), W11
//! (print-PDF profile chain). DD ⑩⑫㉞.
//!
//! ## The seam this module sits on
//!
//! One output subsystem, color-managed end to end (§29.1): every export
//! path — Export-As, quick export, batch, CLI `--export`, slices, PDF —
//! runs the same [NORMATIVE] stage order implemented in [`pipeline`]:
//! flatten → resize → color convert → depth/dither → encode → metadata
//! policy → atomic write. The FLATTEN stage is upstream of this crate
//! (core `export_png_bytes` math / render `region_rgba_f32` — the
//! established composite paths): callers hand a flattened
//! [`ExportSource`] (linear premultiplied f32 working plane, the
//! [`crate::registry::ExportImage`] convention) and this module owns
//! stages 2–7. CLI/ABI wiring of the source construction is the
//! recorded coordination row in the lane log.
//!
//! ## Engines consumed, not re-invented
//!
//! * Stage 2 resize: O-TOOLS' §18.2 kernels (`appname_tools::resample`)
//!   — Lanczos-3 down, bicubic (Catmull-Rom) up, premultiplied, per the
//!   §29.1 stage-2 letter; every `scale_variants` entry re-runs from
//!   the flattened source, never from another variant.
//! * Stage 3 color: O-COLOR's convert machinery (`appname_color::Cms`);
//!   the marshalling (straight-alpha f32, unpremul → transform →
//!   re-premul, a == 0 stays exactly transparent) MIRRORS core's
//!   `doc.convert_profile` implementation so the two converge on one
//!   convention (§5.2/§6.4; coordination row in the lane log).
//! * Stage 4 dithering: §29.1 pins blue-noise (the §22.4 table) at
//!   8-bit demotion; the shared `BN64` table lives in `appname-render`
//!   today, which this crate cannot link — the demotion rides the
//!   codecs' established round-to-nearest until the table's relocation
//!   (recorded door ask, lane log; same interim as core's
//!   `doc.depth_convert` NoDither note).
//! * Stage 5 encode: the §8.1/§8.2 codec registry already in-crate.
//!
//! ## Determinism (D-48)
//!
//! Identical source plane + preset → identical output bytes: the
//! resample taps are f64-derived constants, the color chain is
//! O-COLOR's fixed f64 path, the codecs are the pinned single-threaded
//! elections, and the PDF emitter ([`pdf`]) writes a fully ordered
//! object stream with a fixed flate level. The slice manifest's
//! `id`/`generated` fields are caller-injected ([`slices::ManifestStamp`])
//! so tests pin bytes while production stamps real UUIDs/clock (W15's
//! masked compare).

pub mod exif_edit;
pub mod icc_synth;
pub mod naming;
pub mod pdf;
pub mod pipeline;
pub mod policy;
pub mod preset;
pub mod slices;

use crate::meta::Metadata;

/// The flattened export source — §29.1 stage-1 output, handed in by the
/// caller (the established composite paths; module doc). One source
/// serves every format, slice rect, and scale variant of a job.
pub struct ExportSource<'a> {
    /// Full-canvas pixel extent.
    pub width: u32,
    pub height: u32,
    /// Row-major `width*height*4` LINEAR PREMULTIPLIED RGBA f32 in the
    /// working space (the [`crate::registry::ExportImage`] convention).
    pub plane: &'a [f32],
    /// The document/working ICC profile bytes (§5.1 carriage). `None` =
    /// the documented interim sRGB assumption (crate module doc): Embed
    /// then has nothing to write and convert-to-sRGB is the identity.
    pub icc: Option<&'a [u8]>,
    /// Document metadata (EXIF/IPTC/XMP carriage) — filtered per §29.4
    /// by [`policy`] before any encode.
    pub meta: &'a Metadata,
    /// Document name stem — the `{name}` token (§26.2).
    pub doc_name: &'a str,
    /// Document resolution (§19.1; PDF page geometry + TIFF/JPEG DPI).
    pub ppi: f64,
}

impl ExportSource<'_> {
    /// Validate the plane length against the declared extent (every
    /// entry door calls this first; a mismatch is a caller bug surfaced
    /// typed, never UB).
    pub(crate) fn check(&self) -> Result<(), crate::error::CodecError> {
        let need = self.width as u64 * self.height as u64 * 4;
        if self.plane.len() as u64 != need {
            return Err(crate::error::CodecError::internal(format!(
                "ExportSource plane length {} != {need} (declared {}x{})",
                self.plane.len(),
                self.width,
                self.height
            )));
        }
        if self.width == 0 || self.height == 0 {
            return Err(crate::error::CodecError::corrupt(
                "zero-sized export source",
            ));
        }
        Ok(())
    }
}
