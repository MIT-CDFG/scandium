// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! [`crate::pipeline::Shaper`] over HarfBuzz, via the `appname-shape-sys`
//! wrapper (DD ①: every unsafe block lives there; this crate stays
//! `forbid(unsafe_code)`).
//!
//! Spec: ES §23.2 "HarfBuzz inputs and outputs" [NORMATIVE]: per item the
//! buffer receives the item text plus pre/post context (context crosses
//! item edges — the wrapper passes the WHOLE paragraph and marks the item
//! range), direction from the bidi level, OpenType script/language from
//! the item, features = engine defaults (`liga`, `calt`, `kern` on)
//! overlaid by the run's features, cluster level 1 (monotone character
//! clusters). Outputs are font units scaled by `em_px·64` (26.6 fixed
//! point), converted here to f64 pixels — deterministic: integer scale in,
//! exact /64.0 out.

use appname_shape_sys::{Feature, ShapeFont, ShapeParams, ShapeSysError};
use std::collections::HashMap;

use crate::pipeline::{FaceId, Item, ShapedGlyph, Shaper, StyleId};

/// §23.2 engine-default features, in overlay order.
pub const ENGINE_DEFAULT_FEATURES: [Feature; 3] = [
    Feature::global(*b"liga", 1),
    Feature::global(*b"calt", 1),
    Feature::global(*b"kern", 1),
];

/// §23.2 feature overlay: engine defaults first, the run's §23.4
/// features after. Later duplicate-tag entries win in HarfBuzz
/// (verified in the vendored 8.4.0 hb-ot-map.cc merge loop, C-4
/// review; behavioral coverage needs a feature-bearing face — C-5).
pub fn feature_overlay(style: &crate::style::CharStyle) -> Vec<Feature> {
    ENGINE_DEFAULT_FEATURES
        .iter()
        .copied()
        .chain(
            style
                .features
                .iter()
                .map(|&(tag, value)| Feature::global(tag, value)),
        )
        .collect()
}

/// Fixed-em HarfBuzz shaper for the CP-0.7 spike: faces are registered
/// as raw bytes and shaped at one `em_px` (the seam's `shape(item, text)`
/// carries no style payload yet; the CP-1.0 engine keys fonts by
/// (face, em, variation coords) in the FontDb/raster-cache instead).
/// C-4: per-StyleId language + feature overlays register via
/// [`HarfBuzzShaper::set_style`]; unset styles shape with the engine
/// defaults and the deterministic dflt language lookup.
pub struct HarfBuzzShaper {
    em_px: f64,
    fonts: HashMap<u32, ShapeFont>,
    styles: HashMap<u32, (Option<String>, Vec<Feature>)>,
}

impl HarfBuzzShaper {
    /// 26.6 fixed-point scale for an em size in pixels (§19.1
    /// `em_px = size_pt · PPI / 72`; the ×64 convention is shared with
    /// the FreeType char-size call so shaping and rastering agree).
    fn scale(em_px: f64) -> i32 {
        (em_px * 64.0).round() as i32
    }

    pub fn new(em_px: f64) -> Self {
        HarfBuzzShaper {
            em_px,
            fonts: HashMap::new(),
            styles: HashMap::new(),
        }
    }

    /// Register a §23.4 style's shaping inputs (language + feature
    /// overlay) under its StyleId.
    pub fn set_style(&mut self, id: StyleId, style: &crate::style::CharStyle) {
        self.styles
            .insert(id.0, (style.language.clone(), feature_overlay(style)));
    }

    pub fn em_px(&self) -> f64 {
        self.em_px
    }

    /// Register a face's bytes under a [`FaceId`]. Returns the wrapper's
    /// error when HarfBuzz rejects the bytes.
    pub fn add_face(&mut self, id: FaceId, bytes: &[u8]) -> Result<(), ShapeSysError> {
        let font = ShapeFont::from_bytes(bytes, 0, Self::scale(self.em_px))?;
        self.fonts.insert(id.0, font);
        Ok(())
    }

    /// cmap probe surface (the §44.2 fallback chain and tests use this).
    pub fn glyph_index(&self, id: FaceId, c: char) -> Option<u32> {
        self.fonts.get(&id.0).and_then(|f| f.glyph_index(c))
    }
}

impl Shaper for HarfBuzzShaper {
    fn shape(&self, item: &Item, text: &str) -> Vec<ShapedGlyph> {
        let Some(font) = self.fonts.get(&item.face.0) else {
            // An item can only carry a face the FontProbe resolved; an
            // unregistered face here is a caller bug, surfaced loudly in
            // debug and shaped as nothing (never a panic) in release.
            debug_assert!(
                false,
                "shape() called with unregistered FaceId({})",
                item.face.0
            );
            return Vec::new();
        };
        let (language, features) = match self.styles.get(&item.style.0) {
            Some((lang, feats)) => (lang.as_deref(), feats.as_slice()),
            None => (None, &ENGINE_DEFAULT_FEATURES[..]),
        };
        let params = ShapeParams {
            rtl: item.bidi_level % 2 == 1,
            script: item.script,
            language,
            features,
        };
        match font.shape(text, item.range.start, item.range.len(), &params) {
            Ok(raw) => raw
                .into_iter()
                .map(|g| ShapedGlyph {
                    glyph: g.glyph,
                    cluster: g.cluster as usize,
                    advance: f64::from(g.x_advance) / 64.0,
                    offset_x: f64::from(g.x_offset) / 64.0,
                    offset_y: f64::from(g.y_offset) / 64.0,
                    unsafe_to_break: g.unsafe_to_break,
                })
                .collect(),
            Err(e) => {
                debug_assert!(false, "HarfBuzz shape failed: {e}");
                Vec::new()
            }
        }
    }
}
