// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! [`crate::pipeline::GlyphRasterizer`] over FreeType (freetype-rs safe
//! wrapper; the vendored C compiles at freetype-sys — deps/freetype.yaml,
//! FTL elected; DD ① keeps this crate `forbid(unsafe_code)`).
//!
//! Spec: ES §23.2 "Rasterization" [NORMATIVE]:
//! * Hinting: FreeType AUTOHINTER, light target, when em ≤ 32 doc px (the
//!   integer-translation half of the rule is the layout's to assert —
//!   CP-1.0; the spike rasters at glyph origin, which is the identity
//!   translation). Otherwise unhinted. Native bytecode hinting NEVER
//!   (face/version-sensitive — breaks cross-OS pixel identity).
//! * Output: Gray f32 coverage in [0,1] — the SAME plane convention as
//!   `appname-vector`'s `CoveragePlane` (§20.2) and §13.1 masks; DD ㉒/㉕
//!   meet at this representation.
//! * Cache: (face, em, variation coords, hint mode, phase, glyph id) LRU
//!   under the §30.1 budget — CP-1.0 (the spike rasters uncached).

use freetype::face::LoadFlag;
use freetype::{Face, Library, RenderMode};
use std::collections::HashMap;

use crate::pipeline::{FaceId, GlyphRasterizer};

/// The §23.2 hinting threshold: autohinter-light at or below this em.
pub const HINT_EM_MAX_PX: f64 = 32.0;

/// One glyph's Gray f32 coverage raster. `left` is the horizontal
/// bearing from the pen position to the first column; `top` the vertical
/// bearing from the baseline UP to the first row (FreeType convention).
/// `data` is row-major, `width × height`, each value in [0,1].
#[derive(Debug, Clone, PartialEq)]
pub struct GlyphCoverage {
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
    pub data: Vec<f32>,
}

impl GlyphCoverage {
    pub const EMPTY: GlyphCoverage = GlyphCoverage {
        left: 0,
        top: 0,
        width: 0,
        height: 0,
        data: Vec::new(),
    };

    /// Σ coverage (f64 accumulation) — the test oracle surface.
    pub fn total(&self) -> f64 {
        self.data.iter().map(|&v| f64::from(v)).sum()
    }
}

/// FreeType-backed rasterizer holding registered memory faces.
pub struct FreeTypeRasterizer {
    faces: HashMap<u32, Face>,
    // Library must outlive faces; kept for that reason (and future
    // properties calls), not otherwise read.
    _library: Library,
}

impl FreeTypeRasterizer {
    pub fn new() -> Result<Self, freetype::Error> {
        Ok(FreeTypeRasterizer {
            faces: HashMap::new(),
            _library: Library::init()?,
        })
    }

    /// Register a face's bytes under a [`FaceId`].
    pub fn add_face(&mut self, id: FaceId, bytes: Vec<u8>) -> Result<(), freetype::Error> {
        let face = self._library.new_memory_face(bytes, 0)?;
        self.faces.insert(id.0, face);
        Ok(())
    }

    fn raster_inner(
        &self,
        face: FaceId,
        glyph: u32,
        em_px: f64,
    ) -> Result<GlyphCoverage, freetype::Error> {
        let Some(f) = self.faces.get(&face.0) else {
            debug_assert!(
                false,
                "raster() called with unregistered FaceId({})",
                face.0
            );
            return Ok(GlyphCoverage::EMPTY);
        };
        // 26.6 fixed-point char size at 72 dpi (1 pt = 1 px) — the same
        // ×64 integer convention as the shaper's scale, so shaping and
        // rastering can never disagree about the em.
        let size_26_6 = (em_px * 64.0).round() as isize;
        f.set_char_size(0, size_26_6, 72, 72)?;
        // §23.2 hinting rule (em half; translation half is the layout's).
        let flags = if em_px <= HINT_EM_MAX_PX {
            LoadFlag::FORCE_AUTOHINT | LoadFlag::TARGET_LIGHT
        } else {
            LoadFlag::NO_HINTING
        };
        f.load_glyph(glyph, flags)?;
        let slot = f.glyph();
        slot.render_glyph(RenderMode::Normal)?;
        let bitmap = slot.bitmap();
        let (w, h) = (bitmap.width().max(0) as u32, bitmap.rows().max(0) as u32);
        let pitch = bitmap.pitch();
        // FT_RENDER_MODE_NORMAL always emits positive-pitch (top-down)
        // 8-bit gray. A negative pitch means the data begins BEFORE the
        // buffer pointer (upward flow) — freetype-rs's safe `buffer()`
        // slice cannot represent that, so it is refused honestly rather
        // than "handled" with rows that could never be read (review
        // finding, slice-2 re-review round).
        if pitch < 0 {
            debug_assert!(
                false,
                "negative-pitch glyph bitmap (mode Normal never emits one)"
            );
            return Ok(GlyphCoverage::EMPTY);
        }
        let buffer = bitmap.buffer();
        let mut data = Vec::with_capacity((w * h) as usize);
        for y in 0..h as usize {
            let row_start = y * pitch as usize;
            for x in 0..w as usize {
                let v = buffer.get(row_start + x).copied().unwrap_or(0);
                data.push(f32::from(v) / 255.0);
            }
        }
        Ok(GlyphCoverage {
            left: slot.bitmap_left(),
            top: slot.bitmap_top(),
            width: w,
            height: h,
            data,
        })
    }
}

impl GlyphRasterizer for FreeTypeRasterizer {
    type Coverage = GlyphCoverage;

    fn raster(&self, face: FaceId, glyph: u32, em_px: f64) -> GlyphCoverage {
        match self.raster_inner(face, glyph, em_px) {
            Ok(c) => c,
            Err(e) => {
                debug_assert!(false, "FreeType raster failed: {e}");
                GlyphCoverage::EMPTY
            }
        }
    }
}

/// One outline command in px, glyph-origin-relative, y-UP (FreeType's
/// coordinate convention; Door 9 flips into doc space). Quadratics stay
/// quadratic here — the consumer elevates to the §20.1 cubic model.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OutlineCmd {
    MoveTo([f64; 2]),
    LineTo([f64; 2]),
    QuadTo([f64; 2], [f64; 2]),
    CubicTo([f64; 2], [f64; 2], [f64; 2]),
    Close,
}

impl FreeTypeRasterizer {
    /// OS/2 `fsType` (§23.5 embedding policy input); `None` when the
    /// face carries no OS/2 table (installable per the §23.5 default).
    pub fn os2_fs_type(&mut self, face: FaceId) -> Option<u16> {
        let f = self.faces.get_mut(&face.0)?;
        let os2 = freetype::tt_os2::TrueTypeOS2Table::from_face(f)?;
        Some(os2.fs_type())
    }

    /// Door 9 (`text.to_shape`): one glyph's outline at `em_px`,
    /// UNHINTED — outlines are GEOMETRY (§20.1 paths); hinting is a
    /// raster-space concern (§23.2) and must not deform the bake.
    /// Same 26.6 char-size convention as [`Self::raster`] (÷ 64 to px).
    /// Unregistered faces and bitmap-only glyphs yield an empty list.
    pub fn glyph_outline_px(&mut self, face: FaceId, glyph: u32, em_px: f64) -> Vec<OutlineCmd> {
        let Some(f) = self.faces.get(&face.0) else {
            debug_assert!(
                false,
                "glyph_outline_px with unregistered FaceId({})",
                face.0
            );
            return Vec::new();
        };
        let size_26_6 = (em_px * 64.0).round() as isize;
        if f.set_char_size(0, size_26_6, 72, 72).is_err()
            || f.load_glyph(glyph, LoadFlag::NO_HINTING | LoadFlag::NO_BITMAP)
                .is_err()
        {
            return Vec::new();
        }
        let slot = f.glyph();
        let Some(outline) = slot.outline() else {
            return Vec::new();
        };
        // FT_Pos is i64; 26.6 glyph coordinates are far inside f64's
        // exact-integer range, so the cast is lossless here.
        #[allow(clippy::cast_precision_loss)]
        let px = |v: &freetype::Vector| [v.x as f64 / 64.0, v.y as f64 / 64.0];
        let mut cmds = Vec::new();
        for contour in outline.contours_iter() {
            cmds.push(OutlineCmd::MoveTo(px(contour.start())));
            for curve in contour {
                cmds.push(match curve {
                    freetype::outline::Curve::Line(p) => OutlineCmd::LineTo(px(&p)),
                    freetype::outline::Curve::Bezier2(c, p) => OutlineCmd::QuadTo(px(&c), px(&p)),
                    freetype::outline::Curve::Bezier3(c1, c2, p) => {
                        OutlineCmd::CubicTo(px(&c1), px(&c2), px(&p))
                    }
                });
            }
            cmds.push(OutlineCmd::Close);
        }
        cmds
    }
}

/// Per-face vertical metrics at an em size, px. `descent` is the
/// POSITIVE magnitude below the baseline (§23.3 arithmetic wants
/// ascent + descent + line_gap directly).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FaceMetricsPx {
    pub ascent: f64,
    pub descent: f64,
    pub line_gap: f64,
}

/// OS/2 fsSelection bit 7 (§23.2 "Vertical metrics").
const USE_TYPO_METRICS: u16 = 1 << 7;

impl FreeTypeRasterizer {
    /// unitsPerEm as f64 (≥ 1 guarded); table-access helper for the
    /// §23.4 decoration/position metrics in [`crate::style`].
    pub(crate) fn units_per_em(&mut self, face: FaceId) -> f64 {
        self.faces
            .get(&face.0)
            .map_or(1000.0, |f| f64::from(f.em_size().max(1)))
    }

    /// post table underline (position, thickness), font units.
    pub(crate) fn post_underline(&mut self, face: FaceId) -> Option<(i16, i16)> {
        let f = self.faces.get_mut(&face.0)?;
        let post = freetype::tt_postscript::TrueTypePostscriptTable::from_face(f)?;
        Some((post.underline_position(), post.underline_thickness()))
    }

    /// OS/2 strikeout (position, size), font units.
    pub(crate) fn os2_strikeout(&mut self, face: FaceId) -> Option<(i16, i16)> {
        let f = self.faces.get_mut(&face.0)?;
        let os2 = freetype::tt_os2::TrueTypeOS2Table::from_face(f)?;
        Some((os2.y_strikeout_position(), os2.y_strikeout_size()))
    }

    /// OS/2 superscript (ySize, yOffset), font units.
    pub(crate) fn os2_superscript(&mut self, face: FaceId) -> Option<(i16, i16)> {
        let f = self.faces.get_mut(&face.0)?;
        let os2 = freetype::tt_os2::TrueTypeOS2Table::from_face(f)?;
        Some((os2.y_superscript_y_size(), os2.y_superscript_y_offset()))
    }

    /// OS/2 subscript (ySize, yOffset), font units.
    pub(crate) fn os2_subscript(&mut self, face: FaceId) -> Option<(i16, i16)> {
        let f = self.faces.get_mut(&face.0)?;
        let os2 = freetype::tt_os2::TrueTypeOS2Table::from_face(f)?;
        Some((os2.y_subscript_y_size(), os2.y_subscript_y_offset()))
    }
}

impl FreeTypeRasterizer {
    /// §23.2 [NORMATIVE] vertical-metrics rule, one rule on every OS:
    /// if OS/2 exists and fsSelection bit 7 (USE_TYPO_METRICS) is set,
    /// ascent/descent/line-gap come from sTypoAscender/sTypoDescender/
    /// sTypoLineGap; otherwise from hhea. Platform text APIs are never
    /// consulted. Values scale by `em_px / unitsPerEm`.
    pub fn metrics(&mut self, face: FaceId, em_px: f64) -> FaceMetricsPx {
        let Some(f) = self.faces.get_mut(&face.0) else {
            debug_assert!(false, "metrics() with unregistered FaceId({})", face.0);
            return FaceMetricsPx {
                ascent: 0.0,
                descent: 0.0,
                line_gap: 0.0,
            };
        };
        let upm = f64::from(f.em_size().max(1));
        let scale = em_px / upm;
        let typo = freetype::tt_os2::TrueTypeOS2Table::from_face(f)
            .filter(|os2| os2.fs_selection() & USE_TYPO_METRICS != 0)
            .map(|os2| {
                (
                    f64::from(os2.s_typo_ascender()),
                    f64::from(os2.s_typo_descender()),
                    f64::from(os2.s_typo_line_gap()),
                )
            });
        // hhea arm: with no USE_TYPO_METRICS flag, the pinned
        // FreeType's Face accessors yield the hhea values (font units;
        // height = a − d + gap). (Precision note from review: FT ≥
        // 2.9.1 accessors themselves honor USE_TYPO_METRICS, but this
        // branch only runs when the flag is absent, so both arms match
        // the §23.2 rule either way.)
        let (asc, desc, gap) = typo.unwrap_or_else(|| {
            let a = f64::from(f.ascender());
            let d = f64::from(f.descender()); // negative-down in hhea
            let h = f64::from(f.height());
            (a, d, h - (a - d))
        });
        FaceMetricsPx {
            ascent: asc * scale,
            descent: -desc * scale, // both sources carry negative-down
            line_gap: gap * scale,
        }
    }
}
