// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! C-5 raster binding: compose a laid-out paragraph into ONE Gray f32
//! coverage page — the §23.2 tail ("FreeType rasterization → tile
//! upload"; the tile upload itself is the text-kind arm's, this module
//! produces the page the arm installs).
//!
//! Spec: ES §23.2 "Rasterization" [NORMATIVE] (Gray f32 coverage, the
//! §13.1/§20.2 plane convention; cache keyed per the §23.2 tuple —
//! [`crate::cache::RasterCache`] IS the cache, this module drives it);
//! §23.4 decoration/position fields (underline/strikethrough rects in
//! the run's coverage; sub/super scale+shift; faux italic
//! x′ = x + y·tan 12° about the BASELINE; §23.5 tofu for
//! [`crate::fontdb::TOFU_FACE`] — nothing silently invisible).
//!
//! Pinned realizations (fold-in register):
//! * Glyph coverage composites by MAX (coverage union — overlapping AA
//!   edges of adjacent glyphs must not sum past 1.0; §13.1 range).
//! * Faux-italic shear applies at composition as a per-row fractional
//!   x-offset with linear resampling (deterministic; outline-level
//!   shear arrives with the faux-bold outline work, C-6).
//! * Subpixel phases: pen x quantizes to ¼-px phases in the cache key
//!   (§23.2); the DEFAULT grayscale path renders phase 0 only — the
//!   `text.render.subpixel` preference arm is C-6, so every current
//!   raster is phase 0 and the quantization is plumbing, not behavior.
//! * Decoration rects span each contiguous decorated RUN's ADVANCE
//!   extent (first glyph's pen x to last glyph's pen x + advance —
//!   §23.4 "drawn as rects in the run fill"; the fill itself
//!   composites at the engine; coverage here). `DecorLine.position`
//!   is the line's CENTER (the style module's pinned reading) for
//!   BOTH underline and strikethrough.
//! * Faux italic combined with baseline shift shears about the
//!   SHIFTED baseline (the glyph's own raster frame — pinned; shearing
//!   about the run baseline would add a shift·tan12° x-displacement).
//!
//! NOT here (C-6+): caps mapping (pre-shaping text transform at
//! itemization), kerning Optical/Manual, h/v scale, faux bold, LCD
//! filtering (never in documents, §23.2).

use crate::cache::{HintMode, RasterCache, RasterKey};
use crate::fontdb::TOFU_FACE;
use crate::layout::{LaidLine, ParagraphLayout};
use crate::pipeline::FaceId;
use crate::raster::{FreeTypeRasterizer, GlyphCoverage, HINT_EM_MAX_PX};
use crate::style::{DecorLine, FAUX_ITALIC_SHEAR};
use crate::tofu::tofu_coverage;

/// One composed page: Gray f32 coverage over an integer pixel rect
/// (doc-space; `left`/`top` locate it).
#[derive(Debug, Clone, PartialEq)]
pub struct CoveragePage {
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
    /// Row-major, `width × height`, each value in [0, 1].
    pub data: Vec<f32>,
}

impl CoveragePage {
    pub fn total(&self) -> f64 {
        self.data.iter().map(|&v| f64::from(v)).sum()
    }

    fn blit_max(&mut self, cov: &GlyphCoverage, x: i32, y_baseline: i32, shear: f64) {
        // Two-stage composition (pinned): the glyph's OWN sheared
        // coverage ACCUMULATES (clamped) — the shear's linear split is
        // an integral of one glyph's coverage, so adjacent split
        // contributions of the same glyph sum back to full interior
        // coverage — then the accumulated glyph blits into the page by
        // MAX (inter-glyph coverage union, §13.1 range).
        if cov.width == 0 || cov.height == 0 {
            return;
        }
        // Review T-1 (+ re-review residual): margins must be SYMMETRIC
        // — rows BELOW the baseline shear LEFT — and sized from the
        // TRUE row extent max(|top|, |top−height+1|), not height: a
        // glyph floating entirely above the baseline (apostrophes,
        // degree signs: top > height) shears by up to shear·top and
        // would clip under a height-keyed margin.
        let span = f64::from(cov.top)
            .abs()
            .max(f64::from(cov.top - cov.height as i32 + 1).abs());
        let extra = (shear * span).abs().ceil() as i32 + 1;
        let tw = cov.width as i32 + 2 * extra;
        let mut temp = vec![0.0f32; (tw * cov.height as i32) as usize];
        for row in 0..cov.height as i32 {
            // Faux italic: rows above the baseline shift right by
            // height·tan12° (y-up positive height above baseline).
            let above = f64::from(cov.top - row);
            let dx = shear * above;
            let dx_floor = dx.floor();
            let frac = (dx - dx_floor) as f32;
            for col in 0..cov.width as i32 {
                let v = cov.data[(row * cov.width as i32 + col) as usize];
                if v <= 0.0 {
                    continue;
                }
                for (off, w) in [(0i32, 1.0 - frac), (1, frac)] {
                    if w <= 0.0 {
                        continue;
                    }
                    let tx = col + extra + dx_floor as i32 + off;
                    if tx < 0 || tx >= tw {
                        continue;
                    }
                    let idx = (row * tw + tx) as usize;
                    temp[idx] = (temp[idx] + v * w).min(1.0);
                }
            }
        }
        // Stage 2: MAX the accumulated glyph into the page.
        let gy0 = y_baseline - cov.top;
        for row in 0..cov.height as i32 {
            let py = gy0 + row - self.top;
            if py < 0 || py >= self.height as i32 {
                continue;
            }
            for tx in 0..tw {
                let v = temp[(row * tw + tx) as usize];
                if v <= 0.0 {
                    continue;
                }
                let px = x + cov.left + (tx - extra) - self.left;
                if px < 0 || px >= self.width as i32 {
                    continue;
                }
                let idx = (py * self.width as i32 + px) as usize;
                if v > self.data[idx] {
                    self.data[idx] = v;
                }
            }
        }
    }

    fn fill_rect_max(&mut self, x0: i32, x1: i32, y0: i32, y1: i32) {
        for py in y0.max(self.top)..y1.min(self.top + self.height as i32) {
            for px in x0.max(self.left)..x1.min(self.left + self.width as i32) {
                let idx = ((py - self.top) * self.width as i32 + (px - self.left)) as usize;
                self.data[idx] = 1.0;
            }
        }
    }
}

/// Per-item composition inputs the layout does not carry (C-5 style
/// consumption; resolved by the engine glue from the §23.4 model).
#[derive(Debug, Clone, Copy, Default)]
pub struct ItemRaster {
    /// §23.4 position metrics already resolved (scale folds into the
    /// raster em; shift into the baseline, y-up positive).
    pub em_scale: f64,
    pub baseline_shift: f64,
    pub faux_italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
}

impl ItemRaster {
    pub fn plain() -> ItemRaster {
        ItemRaster {
            em_scale: 1.0,
            ..Default::default()
        }
    }
}

/// §23.2 hint rule at composition time (em half; the layout transform
/// is identity here — integer translation — so the em decides).
fn hint_for(em_px: f64) -> HintMode {
    if em_px <= HINT_EM_MAX_PX {
        HintMode::AutohintLight
    } else {
        HintMode::Unhinted
    }
}

/// Compose a paragraph layout into one coverage page.
///
/// `em_of`/`raster_of` resolve per-item facts from the glyph's face and
/// paragraph position: em in px and the [`ItemRaster`] style facts.
/// Decoration geometry (`underline_of`/`strikeout_of`) comes from the
/// §23.4 metrics arms on the SAME rasterizer. Tofu glyphs
/// ([`TOFU_FACE`]) render procedurally (§23.5).
#[allow(clippy::too_many_arguments)]
pub fn compose_page(
    layout: &ParagraphLayout,
    rasterizer: &FreeTypeRasterizer,
    cache: &mut RasterCache,
    em_of: impl Fn(&LaidLine, usize) -> f64,
    raster_of: impl Fn(&LaidLine, usize) -> ItemRaster,
    underline_of: impl Fn(FaceId, f64) -> DecorLine,
    strikeout_of: impl Fn(FaceId, f64) -> DecorLine,
) -> CoveragePage {
    // Page bounds: union of glyph boxes + decorations, integer hull.
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    let mut any = false;
    for line in &layout.lines {
        for (gi, g) in line.glyphs.iter().enumerate() {
            let em = em_of(line, gi);
            any = true;
            let approx = em.ceil() as i32 + 2;
            min_x = min_x.min(g.x.floor() as i32 - approx);
            max_x = max_x.max(g.x.ceil() as i32 + 2 * approx);
            min_y = min_y.min(g.y.floor() as i32 - 2 * approx);
            max_y = max_y.max(g.y.ceil() as i32 + approx);
        }
    }
    if !any {
        return CoveragePage {
            left: 0,
            top: 0,
            width: 0,
            height: 0,
            data: Vec::new(),
        };
    }
    let mut page = CoveragePage {
        left: min_x,
        top: min_y,
        width: (max_x - min_x).max(0) as u32,
        height: (max_y - min_y).max(0) as u32,
        data: vec![0.0; ((max_x - min_x).max(0) * (max_y - min_y).max(0)) as usize],
    };

    for line in &layout.lines {
        // Contiguous decorated runs: (face, em_eff, flags, x0, x1,
        // baseline) — x1 tracks pen x + ADVANCE (review T-2). Merging
        // keys on GLYPH-SEQUENCE ADJACENCY (re-review residual: an
        // x-distance heuristic bridged an undecorated glyph sitting
        // between two decorated ones — the discriminator is that the
        // previous decorated glyph is the IMMEDIATELY preceding one).
        let mut decor: Vec<(FaceId, f64, ItemRaster, f64, f64, f64)> = Vec::new();
        let mut last_decor_gi: Option<usize> = None;
        for (gi, g) in line.glyphs.iter().enumerate() {
            let em = em_of(line, gi);
            let ir = raster_of(line, gi);
            let em_eff = em * if ir.em_scale > 0.0 { ir.em_scale } else { 1.0 };
            let baseline = g.y - ir.baseline_shift; // y-up shift
            let shear = if ir.faux_italic {
                FAUX_ITALIC_SHEAR
            } else {
                0.0
            };
            // ¼-px phase plumbing (§23.2): quantized from pen x; the
            // grayscale default renders phase 0 (pinned above).
            let _phase_quantized = ((g.x.fract() * 4.0).round() as u8) & 3;
            let phase = 0u8;
            let cov = if g.face == TOFU_FACE {
                std::rc::Rc::new(tofu_coverage(
                    char::from_u32(g.glyph).unwrap_or('\u{FFFD}'),
                    em_eff,
                ))
            } else {
                cache.get_or_raster(
                    rasterizer,
                    &RasterKey {
                        face: g.face,
                        em_26_6: (em_eff * 64.0).round() as u32,
                        variations: Vec::new(),
                        hint: hint_for(em_eff),
                        phase,
                        glyph: g.glyph,
                    },
                )
            };
            page.blit_max(&cov, g.x.round() as i32, baseline.round() as i32, shear);
            if ir.underline || ir.strikethrough {
                // Extend the previous run when the previous DECORATED
                // glyph is the immediately preceding one in sequence
                // and the style matches; else open a new run.
                let end = g.x + g.advance;
                let adjacent = gi > 0 && last_decor_gi == Some(gi - 1);
                match decor.last_mut() {
                    Some((f, e, pir, _, x1, b))
                        if adjacent
                            && *f == g.face
                            && *e == em_eff
                            && pir.underline == ir.underline
                            && pir.strikethrough == ir.strikethrough
                            && *b == baseline =>
                    {
                        *x1 = end.max(*x1);
                    }
                    _ => decor.push((g.face, em_eff, ir, g.x, end, baseline)),
                }
                last_decor_gi = Some(gi);
            }
        }
        // Decoration rects over each decorated glyph's advance span.
        for (face, em_eff, ir, x0, x1, baseline) in decor {
            // Review T-3: DecorLine.position is the line's CENTER for
            // BOTH decorations (the style module's pinned reading).
            let mut line_rect = |d: DecorLine| {
                let center = baseline - d.position; // y-up → page y-down
                let half = d.thickness.max(1.0) / 2.0;
                page.fill_rect_max(
                    x0.floor() as i32,
                    x1.ceil() as i32,
                    (center - half).floor() as i32,
                    (center + half).ceil() as i32,
                );
            };
            if ir.underline {
                line_rect(underline_of(face, em_eff));
            }
            if ir.strikethrough {
                line_rect(strikeout_of(face, em_eff));
            }
        }
    }
    page
}
