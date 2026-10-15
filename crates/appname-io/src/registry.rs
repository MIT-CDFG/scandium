// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §8.1 "The `Codec` trait" + "Registration order and sniff
//! resolution" [NORMATIVE]; DD ⑫. All non-native formats implement one
//! uniform trait; new codecs register without core edits.
//!
//! Registration order of built-ins is FIXED: **PNG, JPEG, TIFF, WebP, GIF,
//! BMP** (PSD, ORA, RAW join at their §2 rows). Sniff resolution: the
//! winner among claimants is the longest effective magic; ties break by
//! case-insensitive extension match against the tied codecs' extension
//! lists; still tied → earliest registration. The extension NEVER selects a
//! codec whose sniff said no — content beats name; no claimant →
//! `UnrecognizedFormat`. The native `.appname` short-circuit (§8.1 step 1)
//! is the package opener's, upstream of this registry.
//!
//! Slice-1 shape notes vs the frozen trait (each a documented seam, lane
//! log "handoffs"): `decode` returns pixel data as a linear premultiplied
//! f32 [`Buffer`] built through core's public boundary plus the
//! byte-coupled `ingress_png` journaled-import carrier ([`crate::plane`]
//! module doc), `Imported` additionally records the value-captured
//! `orientation_applied` (D-48: the bake is already materialized in the
//! carrier bytes), and `CodecCaps::license` is the built-in constant
//! until the license-gated module row (§8.3) lands.

use appname_core::{BlendMode, Buffer, IVec2};

use crate::error::CodecError;
use crate::meta::Metadata;
use crate::opts::{DecodeOpts, EncodeOpts};

/// Stable codec identity (§8.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodecId(pub &'static str);

/// §8.1 capability surface.
#[derive(Debug, Clone, Copy)]
pub struct CodecCaps {
    pub read: bool,
    pub write: bool,
    pub layered: bool,
    pub max_depth: appname_core::ChannelKind,
    pub alpha: bool,
    pub animation: bool,
    pub license: LicenseStatus,
}

/// §8.1 `LicenseStatus` — built-ins are unconditional; RAW is the
/// license-gated module row (§8.3): compiled only behind feature `raw`
/// (§33.1 matrix), its decoder linking dynamically per the LibRaw deps
/// record posture (LGPL-2.1-elected, module-not-core-link, §48.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LicenseStatus {
    Builtin,
    /// Present only when its feature is compiled; ships as a replaceable
    /// dynamic module, never in the core link graph.
    LicenseGatedModule,
}

/// §8.1 compat-report action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompatAction {
    Dropped,
    Approximated,
    Info,
}

/// One §8.1 compat item (e.g. `metadata.exif` dropped as malformed).
#[derive(Debug, Clone)]
pub struct CompatItem {
    pub feature: String,
    pub action: CompatAction,
    pub detail: String,
}

/// The honest-decode ledger: what was dropped/approximated, never silent.
#[derive(Debug, Clone, Default)]
pub struct CompatReport {
    pub items: Vec<CompatItem>,
}

impl CompatReport {
    pub fn push(
        &mut self,
        feature: impl Into<String>,
        action: CompatAction,
        detail: impl Into<String>,
    ) {
        self.items.push(CompatItem {
            feature: feature.into(),
            action,
            detail: detail.into(),
        });
    }
}

/// §11.1 lock flags carried through import (applied via the journaled
/// `layer.set_props` at engine landing).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LayerLocks {
    pub transparency: bool,
    pub pixels: bool,
    pub position: bool,
}

/// §8.1 `ImportedLayer` (layered/animated decodes; bottom-to-top order).
#[derive(Debug)]
pub struct ImportedLayer {
    pub name: String,
    pub buffer: Buffer,
    /// The journaled-import asset carrier: the deterministic PNG transcode
    /// this layer's `buffer` was decoded from (byte-coupled — see
    /// [`crate::plane`] module doc). `Engine::add_raster_asset` ingests it;
    /// the `edit.paste` envelope references the resulting asset id.
    pub ingress_png: Vec<u8>,
    /// Per-layer embedded ICC where the container carries one (ORA layer
    /// PNG `iCCP`, raw as stored) — CARRIED for the CMM seam, like
    /// `Imported::profile`.
    pub icc: Option<Vec<u8>>,
    pub offset: IVec2,
    pub opacity: f32,
    /// §11.1 fill opacity (PSD `iOpa`); 1.0 default.
    pub fill_opacity: f32,
    pub blend: BlendMode,
    pub visible: bool,
    pub clip_to_below: bool,
    /// §11.1 locks (PSD flags/`lspf`).
    pub locks: LayerLocks,
    pub group_path: Vec<String>,
}

/// §8.1 `Imported` (slice-1 shape — module doc).
#[derive(Debug)]
pub struct Imported {
    /// The primary surface (single-image decodes; page/frame 0 for layered
    /// ones): linear premultiplied f32, decoded through core's public
    /// boundary.
    pub buffer: Buffer,
    /// The journaled-import asset carrier for the primary surface (see
    /// [`ImportedLayer::ingress_png`]).
    pub ingress_png: Vec<u8>,
    /// Full pixel extent (w, h) of the upright image — `Buffer::bounds` is
    /// content-tight and excludes transparent margins.
    pub size: (u32, u32),
    /// Embedded ICC profile bytes, CARRIED (§5 seam; also in `meta.icc`).
    pub profile: Option<Vec<u8>>,
    pub meta: Metadata,
    /// Bottom-to-top; `None` for single-image decodes.
    pub layers: Option<Vec<ImportedLayer>>,
    /// Value-captured §8.6 orientation that was BAKED into the pixels
    /// (D-48 replay determinism; `None` = the format carries none).
    pub orientation_applied: Option<u16>,
    pub report: CompatReport,
    /// The §42.4 `appname.compatReport.v1` document, for importers that
    /// emit one (PSD; ORA/SVG join per the shared-schema note). `None`
    /// elsewhere. Persists under `meta["compatReport"]` at the
    /// document-meta wave.
    pub compat_report: Option<serde_json::Value>,
}

/// §8.1 `ExportImage` (slice-1 shape): the composite comes from the
/// established path (core `export_png_bytes` math / render
/// `region_rgba_f32`) as one LINEAR PREMULTIPLIED f32 RGBA plane.
pub struct ExportImage<'a> {
    pub width: u32,
    pub height: u32,
    /// Row-major `w*h*4` linear premultiplied RGBA.
    pub plane: &'a [f32],
    /// Named full-canvas layer planes, bottom-to-top — consulted only by
    /// the §8.2 frame↔layer animation contract (GIF).
    pub layers: Option<Vec<ExportLayer<'a>>>,
    /// ICC profile bytes to embed under `IccPolicy::Embed`.
    pub profile: Option<&'a [u8]>,
    pub meta: &'a Metadata,
}

/// One named export layer (full canvas, linear premultiplied f32 RGBA).
pub struct ExportLayer<'a> {
    pub name: &'a str,
    pub plane: &'a [f32],
}

/// §8.1 codec trait (slice-1 shape — module doc).
pub trait Codec: Send + Sync {
    fn id(&self) -> CodecId;
    fn capabilities(&self) -> CodecCaps;
    /// Magic-byte detection over the §8.1 sniff head (first 512 bytes).
    fn sniff(&self, head: &[u8]) -> bool;
    /// The §8.1 effective-magic length used for sniff-tie resolution.
    fn magic_len(&self) -> u32;
    /// Content-dependent effective magic length for codecs whose §8.1
    /// table row varies by vendor container (RAW: CR2 = 10, NEF/ARW/DNG
    /// = 4). Default: the codec's fixed `magic_len` — only RAW overrides.
    /// Called only on claimants (`sniff(head) == true`).
    fn effective_magic_len(&self, _head: &[u8]) -> u32 {
        self.magic_len()
    }
    /// Registered extension list (lowercase), the §8.1 tiebreak.
    fn extensions(&self) -> &'static [&'static str];
    /// Decode bytes → pixels + ICC profile + metadata (+ layers).
    fn decode(&self, bytes: &[u8], opts: &DecodeOpts) -> Result<Imported, CodecError>;
    /// Encode an image (+ profile + metadata) to bytes.
    fn encode(&self, img: &ExportImage<'_>, opts: &EncodeOpts) -> Result<Vec<u8>, CodecError>;
}

/// §8.1 registry: fixed built-in order, then plugins in load order.
pub struct CodecRegistry {
    codecs: Vec<Box<dyn Codec>>,
}

impl CodecRegistry {
    /// The built-ins in the NORMATIVE §8.1 order: PNG, JPEG, TIFF, WebP,
    /// GIF, BMP, PSD, ORA, RAW — RAW present only when its §33.1 feature
    /// is compiled (the license-gated module row; `crate::raw` module
    /// doc). Feature off ⇒ the slot does not exist and RAW opens take the
    /// §43.5 absent-module flow upstream of this registry.
    pub fn builtin() -> CodecRegistry {
        #[allow(unused_mut)] // mut is dead only in --no-default-features builds
        let mut codecs: Vec<Box<dyn Codec>> = vec![
            Box::new(crate::codecs::png_wrap::PngCodec),
            Box::new(crate::codecs::jpeg::JpegCodec),
            Box::new(crate::codecs::tiff_io::TiffCodec),
            Box::new(crate::codecs::webp::WebpCodec),
            Box::new(crate::codecs::gif_io::GifCodec),
            Box::new(crate::codecs::bmp::BmpCodec),
            Box::new(crate::codecs::psd::PsdCodec),
            Box::new(crate::codecs::ora::OraCodec),
        ];
        #[cfg(feature = "raw")]
        codecs.push(Box::new(crate::raw::codec::RawCodec::default()));
        CodecRegistry { codecs }
    }

    pub fn codecs(&self) -> impl Iterator<Item = &dyn Codec> {
        self.codecs.iter().map(Box::as_ref)
    }

    pub fn by_id(&self, id: &str) -> Option<&dyn Codec> {
        self.codecs().find(|c| c.id().0 == id)
    }

    /// §8.1 sniff resolution (module doc). `head` is the first 512 bytes
    /// (whole file if shorter); `ext` the file extension, if any.
    pub fn resolve(&self, head: &[u8], ext: Option<&str>) -> Result<&dyn Codec, CodecError> {
        let claimants: Vec<&dyn Codec> = self
            .codecs()
            .filter(|c| c.capabilities().read && c.sniff(head))
            .collect();
        if claimants.is_empty() {
            return Err(CodecError::UnrecognizedFormat);
        }
        let longest = claimants
            .iter()
            .map(|c| c.effective_magic_len(head))
            .max()
            .unwrap_or(0);
        let tied: Vec<&dyn Codec> = claimants
            .into_iter()
            .filter(|c| c.effective_magic_len(head) == longest)
            .collect();
        if tied.len() > 1 {
            if let Some(ext) = ext {
                let ext = ext.to_ascii_lowercase();
                let matches: Vec<&&dyn Codec> = tied
                    .iter()
                    .filter(|c| c.extensions().contains(&ext.as_str()))
                    .collect();
                if matches.len() == 1 {
                    return Ok(*matches[0]);
                }
            }
        }
        // Still tied (or no decisive extension): earliest registration.
        Ok(tied[0])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_order_is_normative() {
        let reg = CodecRegistry::builtin();
        let ids: Vec<&str> = reg.codecs().map(|c| c.id().0).collect();
        let mut expect = vec!["png", "jpeg", "tiff", "webp", "gif", "bmp", "psd", "ora"];
        if cfg!(feature = "raw") {
            expect.push("raw"); // the §8.1 row-9 slot, feature-gated
        }
        assert_eq!(ids, expect);
    }

    #[test]
    fn sniff_resolution_content_beats_name() {
        let reg = CodecRegistry::builtin();
        // PNG magic with a .bmp extension resolves PNG: the extension never
        // selects a codec whose sniff said no.
        let head = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0];
        assert_eq!(reg.resolve(&head, Some("bmp")).unwrap().id().0, "png");
    }

    #[test]
    fn no_claimant_is_unrecognized_regardless_of_extension() {
        let reg = CodecRegistry::builtin();
        let resolved = reg.resolve(b"plain text", Some("gif"));
        assert!(matches!(
            resolved.err(),
            Some(CodecError::UnrecognizedFormat)
        ));
    }

    #[test]
    fn each_builtin_claims_its_own_magic() {
        let reg = CodecRegistry::builtin();
        let cases: [(&str, &[u8]); 6] = [
            ("png", &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]),
            ("jpeg", &[0xFF, 0xD8, 0xFF, 0xE0]),
            ("tiff", b"II\x2A\x00"),
            ("webp", b"RIFF\x00\x00\x00\x00WEBP"),
            ("gif", b"GIF89a"),
            ("bmp", b"BM\x00\x00"),
        ];
        for (id, head) in cases {
            assert_eq!(reg.resolve(head, None).unwrap().id().0, id, "{id}");
        }
    }
}
