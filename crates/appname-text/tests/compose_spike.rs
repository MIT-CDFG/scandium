// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! C-5 raster-binding acceptance: layout → coverage page through the
//! §23.2 cache, with tofu, decorations, position metrics, and the
//! faux-italic shear — exact oracles on the spike faces.

mod common;

use appname_text::cache::RasterCache;
use appname_text::compose::{compose_page, ItemRaster};
use appname_text::fontdb::TOFU_FACE;
use appname_text::layout::{layout_paragraph, LayoutBox, ParagraphStyle, StyledItem};
use appname_text::pipeline::{
    itemize, FaceId, FontProbe, GraphemeSegmenter, LineBreakProvider, RunSpan, Shaper, StyleId,
};
use appname_text::providers::{
    IcuGraphemeSegmenter, IcuLineBreakProvider, IcuScriptClassifier, UnicodeBidiProvider,
};
use appname_text::raster::FreeTypeRasterizer;
use appname_text::shape::HarfBuzzShaper;
use appname_text::style::DecorLine;
use appname_text::tofu::{tofu_coverage, tofu_shape};

const EM: f64 = 100.0; // > 32: unhinted, integer-exact rect rasters

fn stack(
    text: &str,
    probe_face: FaceId,
) -> (Vec<StyledItem>, Vec<usize>, Vec<usize>, FreeTypeRasterizer) {
    struct P(FaceId);
    impl FontProbe for P {
        fn resolve(&self, _s: StyleId, _c: &str) -> FaceId {
            self.0
        }
    }
    let items = itemize(
        &[RunSpan {
            text,
            style: StyleId(1),
        }],
        &IcuScriptClassifier::new(),
        &UnicodeBidiProvider,
        &IcuGraphemeSegmenter::new(),
        &P(probe_face),
    );
    let mut shaper = HarfBuzzShaper::new(EM);
    shaper.add_face(FaceId(0), &common::build()).unwrap();
    let mut rast = FreeTypeRasterizer::new().unwrap();
    rast.add_face(FaceId(0), common::build()).unwrap();
    let metrics = rast.metrics(FaceId(0), EM);
    let styled: Vec<StyledItem> = items
        .into_iter()
        .map(|item| {
            let glyphs = if item.face == TOFU_FACE {
                tofu_shape(&item, text, EM)
            } else {
                shaper.shape(&item, text)
            };
            StyledItem {
                item,
                glyphs,
                em_px: EM,
                metrics,
                tracking_px: 0.0,
            }
        })
        .collect();
    let bounds = IcuGraphemeSegmenter::new().cluster_boundaries(text);
    let opps = IcuLineBreakProvider::new().opportunities(text);
    (styled, bounds, opps, rast)
}

fn layout_of(
    text: &str,
    items: &[StyledItem],
    bounds: &[usize],
    opps: &[usize],
) -> appname_text::layout::ParagraphLayout {
    layout_paragraph(
        text,
        items,
        bounds,
        opps,
        &ParagraphStyle::default(),
        &LayoutBox {
            left: 0.0,
            top: 0.0,
            right: 1e6,
            bottom: 1e6,
        },
        0.0,
    )
}

fn no_decor(_f: FaceId, _e: f64) -> DecorLine {
    DecorLine {
        position: 0.0,
        thickness: 0.0,
    }
}

#[test]
fn plain_glyph_composes_with_the_exact_raster_mass() {
    let (items, bounds, opps, rast) = stack("A", FaceId(0));
    let layout = layout_of("A", &items, &bounds, &opps);
    let mut cache = RasterCache::new(1 << 20);
    let page = compose_page(
        &layout,
        &rast,
        &mut cache,
        |_, _| EM,
        |_, _| ItemRaster::plain(),
        no_decor,
        no_decor,
    );
    // 'A' rect at em 100: 40×70 px of full coverage (shaping_spike's
    // exact-oracle case) — composition must carry the mass verbatim.
    assert_eq!(page.total(), 2800.0);
    let (hits, misses, used) = cache.stats();
    assert_eq!((hits, misses), (0, 1), "one miss");
    assert!(used > 0, "cache accounts the raster bytes");
}

#[test]
fn cache_is_hit_on_repeated_glyphs_across_composition() {
    let (items, bounds, opps, rast) = stack("AAA", FaceId(0));
    let layout = layout_of("AAA", &items, &bounds, &opps);
    let mut cache = RasterCache::new(1 << 20);
    let page = compose_page(
        &layout,
        &rast,
        &mut cache,
        |_, _| EM,
        |_, _| ItemRaster::plain(),
        no_decor,
        no_decor,
    );
    assert_eq!(page.total(), 3.0 * 2800.0, "disjoint glyph boxes sum");
    let (hits, misses, _) = cache.stats();
    assert_eq!((hits, misses), (2, 1), "one raster, two hits");
}

#[test]
fn tofu_items_render_the_hex_box_never_invisible() {
    // Greek is in no spike face: the probe resolves TOFU_FACE and the
    // page must carry the procedural box (§23.5).
    let text = "\u{03B1}";
    let (items, bounds, opps, rast) = stack(text, TOFU_FACE);
    let layout = layout_of(text, &items, &bounds, &opps);
    let mut cache = RasterCache::new(1 << 20);
    let page = compose_page(
        &layout,
        &rast,
        &mut cache,
        |_, _| EM,
        |_, _| ItemRaster::plain(),
        no_decor,
        no_decor,
    );
    let want = tofu_coverage('\u{03B1}', EM).total();
    assert!(want > 0.0);
    assert_eq!(page.total(), want, "tofu mass verbatim");
    assert_eq!(cache.stats().1, 0, "tofu never touches the glyph cache");
}

#[test]
fn underline_and_strikethrough_rects_land_at_the_metric_positions() {
    let (items, bounds, opps, mut rast_mut) = stack("A", FaceId(0));
    let layout = layout_of("A", &items, &bounds, &opps);
    let u = rast_mut.underline_line(FaceId(0), EM); // post: −7.5, 5.0 at em 100
    assert_eq!(u.position, -7.5);
    assert_eq!(u.thickness, 5.0);
    let rast = rast_mut;
    let mut cache = RasterCache::new(1 << 20);
    let deco = ItemRaster {
        underline: true,
        ..ItemRaster::plain()
    };
    let page = compose_page(
        &layout,
        &rast,
        &mut cache,
        |_, _| EM,
        move |_, _| deco,
        |_, _| DecorLine {
            position: -7.5,
            thickness: 5.0,
        },
        no_decor,
    );
    // EXACT oracle (re-review T-2/T-3): the rect spans the run's
    // ADVANCE extent (0..60 for 'A', advance 600 units at em 100),
    // CENTERED at baseline − position: baseline 80, center 87.5,
    // half 2.5 → rows 85..90 → mass 60 × 5 = 300 exactly.
    let plain = {
        let mut c2 = RasterCache::new(1 << 20);
        compose_page(
            &layout,
            &rast,
            &mut c2,
            |_, _| EM,
            |_, _| ItemRaster::plain(),
            no_decor,
            no_decor,
        )
    };
    let added = page.total() - plain.total();
    assert_eq!(added, 60.0 * 5.0, "advance-span × thickness, exact");
    // Position probes: inside the rect, above it, past the run end.
    let at = |px: i32, py: i32| -> f32 {
        if px < page.left
            || py < page.top
            || px >= page.left + page.width as i32
            || py >= page.top + page.height as i32
        {
            return 0.0;
        }
        page.data[((py - page.top) * page.width as i32 + (px - page.left)) as usize]
    };
    let baseline = layout.lines[0].baseline as i32; // 80
    assert_eq!(at(30, baseline + 7), 1.0, "inside the centered rect");
    assert_eq!(at(30, baseline + 4), 0.0, "above the rect");
    assert_eq!(at(61, baseline + 7), 0.0, "past the advance span");
}

#[test]
fn undecorated_glyph_between_decorated_runs_is_never_bridged() {
    // Re-review residual regression (the reviewer's probe): "AAA" with
    // glyphs 0 and 2 underlined must NOT draw underline coverage under
    // the plain middle glyph — merging keys on glyph-sequence
    // adjacency, not x-distance.
    let (items, bounds, opps, mut rast_mut) = stack("AAA", FaceId(0));
    let layout = layout_of("AAA", &items, &bounds, &opps);
    let _ = rast_mut.underline_line(FaceId(0), EM);
    let rast = rast_mut;
    let mut cache = RasterCache::new(1 << 20);
    let page = compose_page(
        &layout,
        &rast,
        &mut cache,
        |_, _| EM,
        |_, gi| ItemRaster {
            underline: gi != 1, // middle glyph plain
            ..ItemRaster::plain()
        },
        |_, _| DecorLine {
            position: -7.5,
            thickness: 5.0,
        },
        no_decor,
    );
    let at = |px: i32, py: i32| -> f32 {
        if px < page.left
            || py < page.top
            || px >= page.left + page.width as i32
            || py >= page.top + page.height as i32
        {
            return 0.0;
        }
        page.data[((py - page.top) * page.width as i32 + (px - page.left)) as usize]
    };
    let baseline = layout.lines[0].baseline as i32;
    let adv = 60; // 'A' advance at em 100
    assert_eq!(at(30, baseline + 7), 1.0, "first run decorated");
    assert_eq!(at(adv + 30, baseline + 7), 0.0, "middle glyph NOT bridged");
    assert_eq!(at(2 * adv + 30, baseline + 7), 1.0, "second run decorated");
    // Contiguous decorated glyphs still merge into one run: glyphs 0-1
    // decorated together must cover the boundary between them.
    let mut c2 = RasterCache::new(1 << 20);
    let merged = compose_page(
        &layout,
        &rast,
        &mut c2,
        |_, _| EM,
        |_, gi| ItemRaster {
            underline: gi <= 1,
            ..ItemRaster::plain()
        },
        |_, _| DecorLine {
            position: -7.5,
            thickness: 5.0,
        },
        no_decor,
    );
    let at2 = |px: i32, py: i32| -> f32 {
        merged.data[((py - merged.top) * merged.width as i32 + (px - merged.left)) as usize]
    };
    assert_eq!(
        at2(adv, baseline + 7),
        1.0,
        "adjacent decorated glyphs bridge"
    );
}

#[test]
fn descender_shear_conserves_mass_and_shifts_left_below_baseline() {
    // Re-review T-1 regression: 'g' has ink below the baseline (y
    // −200..500 units); rows below the baseline shear LEFT and the
    // symmetric temp margins must keep every contribution.
    let (items, bounds, opps, rast) = stack("g", FaceId(0));
    let layout = layout_of("g", &items, &bounds, &opps);
    let mk = |italic: bool, cache: &mut RasterCache| {
        let ir = ItemRaster {
            faux_italic: italic,
            ..ItemRaster::plain()
        };
        compose_page(
            &layout,
            &rast,
            cache,
            |_, _| EM,
            move |_, _| ir,
            no_decor,
            no_decor,
        )
    };
    let mut c1 = RasterCache::new(1 << 20);
    let mut c2 = RasterCache::new(1 << 20);
    let plain = mk(false, &mut c1);
    let italic = mk(true, &mut c2);
    // Mass conservation over BOTH sides of the baseline (the pre-fix
    // right-only margin lost the below-baseline left shear).
    assert!(
        (plain.total() - italic.total()).abs() < 0.01,
        "descender mass conserved"
    );
    // Bottom rows sit LEFT of top rows by ≈ rise·tan12° across the
    // glyph's 70-px span (50 above + 20 below baseline at em 100).
    let w = italic.width as usize;
    let rows: Vec<usize> = (0..italic.height as usize)
        .filter(|r| italic.data[r * w..(r + 1) * w].iter().any(|&v| v > 0.5))
        .collect();
    let leftmost = |row: usize| -> i32 {
        italic.data[row * w..(row + 1) * w]
            .iter()
            .position(|&v| v > 0.5)
            .map(|c| c as i32)
            .unwrap_or(-1)
    };
    let (top_row, bottom_row) = (rows[0], *rows.last().unwrap());
    let shift = leftmost(top_row) - leftmost(bottom_row);
    let want = (70.0 * appname_text::style::FAUX_ITALIC_SHEAR).round() as i32;
    assert!(
        (shift - want).abs() <= 1,
        "total shear ≈ {want}px over 70px, got {shift}"
    );
}

#[test]
fn faux_italic_shears_deterministically_and_conserves_mass() {
    let (items, bounds, opps, rast) = stack("A", FaceId(0));
    let layout = layout_of("A", &items, &bounds, &opps);
    let mk = |italic: bool, cache: &mut RasterCache| {
        let ir = ItemRaster {
            faux_italic: italic,
            ..ItemRaster::plain()
        };
        compose_page(
            &layout,
            &rast,
            cache,
            |_, _| EM,
            move |_, _| ir,
            no_decor,
            no_decor,
        )
    };
    let mut c1 = RasterCache::new(1 << 20);
    let mut c2 = RasterCache::new(1 << 20);
    let plain = mk(false, &mut c1);
    let italic = mk(true, &mut c2);
    assert_ne!(plain.data, italic.data, "shear moved coverage");
    // Accumulate-then-MAX conserves the glyph's mass (f32 rounding
    // only; no clipping at this size).
    assert!((plain.total() - italic.total()).abs() < 0.01);
    // Determinism.
    let mut c3 = RasterCache::new(1 << 20);
    let italic2 = mk(true, &mut c3);
    assert_eq!(italic.data, italic2.data);
    // Top rows shift right ≈ height·tan12°: compare leftmost covered
    // column of the top vs bottom glyph row.
    let leftmost = |page: &appname_text::compose::CoveragePage, row_from_top: usize| -> i32 {
        let w = page.width as usize;
        let row = &page.data[row_from_top * w..(row_from_top + 1) * w];
        row.iter()
            .position(|&v| v > 0.5)
            .map(|c| c as i32)
            .unwrap_or(-1)
    };
    // Find first and last rows with coverage in the italic page.
    let w = italic.width as usize;
    let rows: Vec<usize> = (0..italic.height as usize)
        .filter(|r| italic.data[r * w..(r + 1) * w].iter().any(|&v| v > 0.5))
        .collect();
    let (top_row, bottom_row) = (rows[0], *rows.last().unwrap());
    let shift = leftmost(&italic, top_row) - leftmost(&italic, bottom_row);
    let want = (70.0 * appname_text::style::FAUX_ITALIC_SHEAR).round() as i32;
    assert!(
        (shift - want).abs() <= 1,
        "shear ≈ {want}px over 70px rise, got {shift}"
    );
}

#[test]
fn position_metrics_scale_and_shift_the_glyph() {
    let (items, bounds, opps, rast) = stack("A", FaceId(0));
    let layout = layout_of("A", &items, &bounds, &opps);
    let mut cache = RasterCache::new(1 << 20);
    // Super at the normative fallback: 58 % size, +33 % em shift.
    let ir = ItemRaster {
        em_scale: 0.58,
        baseline_shift: 33.0,
        ..ItemRaster::plain()
    };
    let page = compose_page(
        &layout,
        &rast,
        &mut cache,
        |_, _| EM,
        move |_, _| ir,
        no_decor,
        no_decor,
    );
    // Scaled em 58: 'A' rect (0.4·58)×(0.7·58) — integer-aligned no
    // longer guaranteed; assert mass within AA slack of the scaled area
    // and that ink sits ABOVE the plain baseline position.
    let want = (0.4 * 58.0) * (0.7 * 58.0);
    assert!(
        (page.total() - want).abs() < 60.0,
        "scaled mass ≈ {want}, got {}",
        page.total()
    );
    // Ink's lowest row must be higher (smaller y) than baseline 12.5+…
    let w = page.width as usize;
    let lowest = (0..page.height as usize)
        .rev()
        .find(|r| page.data[r * w..(r + 1) * w].iter().any(|&v| v > 0.5))
        .unwrap() as i32
        + page.top;
    assert!(
        lowest < layout.lines[0].baseline as i32,
        "super sits above the baseline"
    );
}
