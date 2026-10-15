// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! §23.3 layout acceptance (CP-1.0 slice C-1): exact-oracle tests over
//! the procedural spike face — at `em = 1000/64` px every advance is an
//! exact binary fraction (hmtx/64), so line breaks, justification
//! arithmetic, baselines, and on-path positions assert EXACTLY.
//! The §23.2 vertical-metrics rule is asserted on both arms (hhea for
//! the plain face, sTypo for the USE_TYPO_METRICS variant).

mod common;

use appname_text::layout::{
    layout_on_path, layout_paragraph, layout_point_text, Align, JustifyLast, LayoutBox, Leading,
    OnPathParams, ParagraphStyle, PointAlign, StyledItem,
};
use appname_text::pipeline::{
    itemize, FaceId, FontProbe, GraphemeSegmenter, LineBreakProvider, RunSpan, Shaper, StyleId,
};
use appname_text::providers::{
    IcuGraphemeSegmenter, IcuLineBreakProvider, IcuScriptClassifier, UnicodeBidiProvider,
};
use appname_text::raster::FreeTypeRasterizer;
use appname_text::shape::HarfBuzzShaper;
use appname_vector::flatten::ArcLengthTable;
use appname_vector::geom::Vec2;
use appname_vector::path::{CubicSeg, SubPath};

const EM: f64 = 1000.0 / 64.0; // 15.625 px — scale == upm, exact advances

struct OneFace;
impl FontProbe for OneFace {
    fn resolve(&self, _s: StyleId, _c: &str) -> FaceId {
        FaceId(0)
    }
}

/// Advance in px at EM for a spike-face char.
fn adv(c: char) -> f64 {
    f64::from(common::advance(c)) / 64.0
}

/// Build StyledItems for `text` through the REAL pipeline (itemize +
/// HarfBuzz + FreeType metrics).
fn styled(text: &str) -> (Vec<StyledItem>, Vec<usize>, Vec<usize>) {
    let items = itemize(
        &[RunSpan {
            text,
            style: StyleId(1),
        }],
        &IcuScriptClassifier::new(),
        &UnicodeBidiProvider,
        &IcuGraphemeSegmenter::new(),
        &OneFace,
    );
    let mut shaper = HarfBuzzShaper::new(EM);
    shaper.add_face(FaceId(0), &common::build()).unwrap();
    let mut rast = FreeTypeRasterizer::new().unwrap();
    rast.add_face(FaceId(0), common::build()).unwrap();
    let metrics = rast.metrics(FaceId(0), EM);
    let styled: Vec<StyledItem> = items
        .into_iter()
        .map(|item| {
            let glyphs = shaper.shape(&item, text);
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
    (styled, bounds, opps)
}

fn wide_box(measure: f64) -> LayoutBox {
    LayoutBox {
        left: 0.0,
        top: 0.0,
        right: measure,
        bottom: 1e6,
    }
}

// ---------------------------------------------------------------------
// Metrics rule (§23.2, both arms)
// ---------------------------------------------------------------------

#[test]
fn metrics_rule_prefers_typo_when_flagged_else_hhea() {
    let em = 16.0;
    let mut r = FreeTypeRasterizer::new().unwrap();
    r.add_face(FaceId(0), common::build()).unwrap();
    r.add_face(FaceId(1), common::build_with_typo_os2())
        .unwrap();
    let hhea = r.metrics(FaceId(0), em);
    // hhea: 800 / −200 / 90 at upm 1000, em 16.
    assert_eq!(hhea.ascent, 12.8);
    assert_eq!(hhea.descent, 3.2);
    assert_eq!(hhea.line_gap, 1.44);
    let typo = r.metrics(FaceId(1), em);
    // sTypo: 760 / −240 / 100 — the USE_TYPO_METRICS arm must win.
    assert_eq!(typo.ascent, f64::from(common::TYPO.0) * 16.0 / 1000.0);
    assert_eq!(typo.descent, -f64::from(common::TYPO.1) * 16.0 / 1000.0);
    assert_eq!(typo.line_gap, f64::from(common::TYPO.2) * 16.0 / 1000.0);
}

// ---------------------------------------------------------------------
// Greedy first-fit + leading/baselines
// ---------------------------------------------------------------------

#[test]
fn greedy_first_fit_breaks_and_baselines_are_exact() {
    let text = "AA AA AA";
    let (items, bounds, opps) = styled(text);
    let style = ParagraphStyle::default();
    let out = layout_paragraph(text, &items, &bounds, &opps, &style, &wide_box(30.0), 0.0);
    // "AA AA" (45.3125 px hanging-excluded) exceeds 30 → break after
    // each "AA " (hanging space free): three lines.
    assert_eq!(out.lines.len(), 3);
    assert_eq!(out.lines[0].range, 0..3);
    assert_eq!(out.lines[1].range, 3..6);
    assert_eq!(out.lines[2].range, 6..8);
    assert_eq!(out.lines[0].width, 2.0 * adv('A'));
    // Leading: auto = max(1.2·15.625 = 18.75, 12.5+3.125+1.40625) = 18.75;
    // first baseline at box.top + ascent 12.5.
    assert_eq!(out.lines[0].baseline, 12.5);
    assert_eq!(out.lines[1].baseline, 12.5 + 18.75);
    assert_eq!(out.lines[2].baseline, 12.5 + 37.5);
    assert!(!out.overflowed);
    // Pen positions on line 1: A at 0, A at adv(A).
    assert_eq!(out.lines[0].glyphs[0].x, 0.0);
    assert_eq!(out.lines[0].glyphs[1].x, adv('A'));
    assert_eq!(out.lines[0].glyphs[0].y, 12.5);
}

#[test]
fn fixed_leading_overrides_auto() {
    let text = "AA AA";
    let (items, bounds, opps) = styled(text);
    let style = ParagraphStyle {
        leading: Leading::Fixed(25.0),
        ..Default::default()
    };
    let out = layout_paragraph(text, &items, &bounds, &opps, &style, &wide_box(20.0), 0.0);
    assert_eq!(out.lines.len(), 2);
    assert_eq!(out.lines[1].baseline - out.lines[0].baseline, 25.0);
}

#[test]
fn single_cluster_wider_than_measure_overflows_alone() {
    let text = "AA";
    let (items, bounds, opps) = styled(text);
    let style = ParagraphStyle::default();
    let out = layout_paragraph(text, &items, &bounds, &opps, &style, &wide_box(5.0), 0.0);
    assert_eq!(out.lines.len(), 2, "each cluster overflows alone");
    assert_eq!(out.lines[0].range, 0..1);
    assert_eq!(out.lines[1].range, 1..2);
}

#[test]
fn emergency_break_fills_the_measure_before_overflowing() {
    // "AAAAAA": no interior UAX #14 opportunity; three A's fit in 30 px.
    // The emergency break must carry the LAST FITTING cluster count —
    // AAA | AAA — never one letter per line (C-1 review MAJOR).
    let text = "AAAAAA";
    let (items, bounds, opps) = styled(text);
    let style = ParagraphStyle::default();
    let out = layout_paragraph(text, &items, &bounds, &opps, &style, &wide_box(30.0), 0.0);
    assert_eq!(out.lines.len(), 2);
    assert_eq!(out.lines[0].range, 0..3);
    assert_eq!(out.lines[1].range, 3..6);
    assert_eq!(out.lines[0].width, 3.0 * adv('A'));
}

#[test]
fn hanging_indent_survives_a_zero_trailing_indent() {
    // The classic hang: indent_left 20, indent_first −20, indent_right
    // 0. The clamp is against the LEADING edge only (C-1 review MAJOR:
    // a direction-agnostic min() erased the hang entirely).
    let text = "AA AA";
    let (items, bounds, opps) = styled(text);
    let style = ParagraphStyle {
        indent_left: 20.0,
        indent_first: -20.0,
        ..Default::default()
    };
    let box_ = LayoutBox {
        left: 0.0,
        top: 0.0,
        right: 40.0,
        bottom: 1e6,
    };
    let out = layout_paragraph(text, &items, &bounds, &opps, &style, &box_, 0.0);
    assert_eq!(out.lines.len(), 2, "measure 20 splits the two words");
    assert_eq!(
        out.lines[0].glyphs[0].x, 0.0,
        "first line hangs to the box edge"
    );
    assert_eq!(
        out.lines[1].glyphs[0].x, 20.0,
        "later lines at the left indent"
    );
}

#[test]
fn mandatory_break_line_counts_as_paragraph_last_for_justification() {
    // Pinned interpretation (module header): the line ended by the LF
    // follows justifyLastLine (Start here) — its space must NOT expand.
    let text = "A A\nB";
    let (items, bounds, opps) = styled(text);
    let style = ParagraphStyle {
        align: Align::Justify,
        justify_last: JustifyLast::Start,
        ..Default::default()
    };
    let out = layout_paragraph(text, &items, &bounds, &opps, &style, &wide_box(40.0), 0.0);
    let l1 = &out.lines[0];
    let a2 = l1.glyphs.iter().find(|g| g.cluster == 2).unwrap();
    assert_eq!(a2.x, adv('A') + adv(' '), "no justification on the LF line");
}

#[test]
fn mandatory_lf_always_breaks() {
    let text = "A\nB";
    let (items, bounds, opps) = styled(text);
    let style = ParagraphStyle::default();
    let out = layout_paragraph(text, &items, &bounds, &opps, &style, &wide_box(100.0), 0.0);
    assert_eq!(out.lines.len(), 2);
    assert_eq!(out.lines[0].range, 0..2);
    assert_eq!(out.lines[1].range, 2..3);
}

// ---------------------------------------------------------------------
// Justification (§23.3 steps 1–3, exact)
// ---------------------------------------------------------------------

#[test]
fn word_space_expansion_fills_the_measure_exactly() {
    let text = "A A A A";
    let (items, bounds, opps) = styled(text);
    let style = ParagraphStyle {
        align: Align::Justify,
        justify_last: JustifyLast::Start,
        ..Default::default()
    };
    // measure 30: line 1 = "A A " (natural body "A A" = 26.5625);
    // deficit 3.4375 ≤ 2×space(7.8125) → the ONE body space absorbs it.
    let out = layout_paragraph(text, &items, &bounds, &opps, &style, &wide_box(30.0), 0.0);
    assert!(out.lines.len() >= 2);
    let l1 = &out.lines[0];
    assert_eq!(l1.range, 0..4);
    let a2 = l1.glyphs.iter().find(|g| g.cluster == 2).unwrap();
    assert_eq!(a2.x, adv('A') + adv(' ') + 3.4375);
    // Right edge lands exactly on the measure.
    assert_eq!(a2.x + adv('A'), 30.0);
}

#[test]
fn letter_spacing_caps_at_two_percent_em_and_line_stays_loose() {
    // The long second word forces line 1 to be exactly "A A " — greedy
    // packing would otherwise keep absorbing small words into the
    // spaces' +200 % headroom and step 2 would never trigger (the first
    // version of this test learned that the hard way: the engine's
    // longer line was CORRECT greedy behavior, the expectation wasn't).
    let text = "A A AAAA";
    let (items, bounds, opps) = styled(text);
    let style = ParagraphStyle {
        align: Align::Justify,
        justify_last: JustifyLast::Start,
        ..Default::default()
    };
    // measure 43: line 1 = "A A " (body "A A" = 26.5625): deficit
    // 16.4375 > space cap 15.625 → the one space takes the cap; the
    // remaining 0.8125 over 2 gaps wants 0.40625/gap but clamps at
    // 0.02·em = 0.3125 → loose line (§23.3 step 3).
    let out = layout_paragraph(text, &items, &bounds, &opps, &style, &wide_box(43.0), 0.0);
    let l1 = &out.lines[0];
    assert_eq!(l1.range, 0..4);
    let a2 = l1.glyphs.iter().find(|g| g.cluster == 2).unwrap();
    assert_eq!(a2.x, adv('A') + 0.3125 + adv(' ') + 15.625 + 0.3125);
    assert!(a2.x + adv('A') < 43.0, "loose line never overflows margins");
}

#[test]
fn single_word_line_falls_back_to_start_aligned() {
    let text = "AAAA AAAA";
    let (items, bounds, opps) = styled(text);
    let style = ParagraphStyle {
        align: Align::Justify,
        justify_last: JustifyLast::Start,
        ..Default::default()
    };
    // measure 40: "AAAA" (37.5) fits, "AAAA AAAA" doesn't → line 1 is
    // one word; deficit 2.5 over 3 gaps = 0.833/gap > 5%·em (0.78125)
    // → start-aligned, NO spread (pinned reading).
    let out = layout_paragraph(text, &items, &bounds, &opps, &style, &wide_box(40.0), 0.0);
    let l1 = &out.lines[0];
    assert_eq!(l1.glyphs[1].x, adv('A'), "no synthetic spread");
    assert_eq!(l1.glyphs[0].x, 0.0, "start-aligned");
}

// ---------------------------------------------------------------------
// RTL base direction
// ---------------------------------------------------------------------

#[test]
fn rtl_base_mirrors_start_alignment_and_visual_order() {
    let text = "\u{05D0}\u{05D1} \u{05D0}"; // "אב א"
    let (items, bounds, opps) = styled(text);
    let style = ParagraphStyle {
        rtl_base: true,
        ..Default::default()
    };
    let natural = adv('\u{05D0}') * 2.0 + adv('\u{05D1}') + adv(' ');
    let out = layout_paragraph(text, &items, &bounds, &opps, &style, &wide_box(60.0), 0.0);
    assert_eq!(out.lines.len(), 1);
    let l = &out.lines[0];
    assert_eq!(l.width, natural);
    // Start under RTL = right edge: the line occupies [60−natural, 60].
    let min_x = l.glyphs.iter().map(|g| g.x).fold(f64::INFINITY, f64::min);
    assert_eq!(min_x, 60.0 - natural);
    // Visual order: the LAST logical cluster (final א) is leftmost.
    let leftmost = l
        .glyphs
        .iter()
        .min_by(|a, b| a.x.partial_cmp(&b.x).unwrap())
        .unwrap();
    assert_eq!(leftmost.cluster, 5, "trailing א renders leftmost");
}

// ---------------------------------------------------------------------
// Point text + overflow
// ---------------------------------------------------------------------

#[test]
// NoBreakRanges takes a SLICE of ranges; one-element arrays are the
// test's point, not a botched range literal (lint suggests `[0; 5]`,
// which is exactly what this is not).
#[allow(clippy::single_range_in_vec_init)]
fn no_break_ranges_suppress_interior_opportunities_only() {
    use appname_text::providers::NoBreakRanges;
    let inner = IcuLineBreakProvider::new();
    let text = "aa bb cc"; // opportunities: 3, 6, 8
    assert_eq!(
        appname_text::pipeline::LineBreakProvider::opportunities(&inner, text),
        vec![3, 6, 8]
    );
    // Mark "aa bb" (0..5): the interior opportunity at 3 vanishes; the
    // one at 6 (splits AFTER the range) and the end survive — §23.4
    // No-break, boundary semantics pinned in the wrapper doc.
    let ranges = [0..5];
    let nb = NoBreakRanges {
        inner: &inner,
        ranges: &ranges,
    };
    assert_eq!(
        appname_text::pipeline::LineBreakProvider::opportunities(&nb, text),
        vec![6, 8]
    );
    // Marking everything leaves only the end-of-text opportunity: the
    // span then behaves like one long word (emergency rules apply).
    let all = [0..text.len()];
    let nb_all = NoBreakRanges {
        inner: &inner,
        ranges: &all,
    };
    assert_eq!(
        appname_text::pipeline::LineBreakProvider::opportunities(&nb_all, text),
        vec![8]
    );
}

#[test]
fn point_text_anchor_sits_on_first_baseline() {
    let text = "AB\nC";
    let (items, bounds, _) = styled(text);
    let style = ParagraphStyle::default();
    let out = layout_point_text(
        text,
        &items,
        &bounds,
        &style,
        (100.0, 50.0),
        PointAlign::Center,
    );
    assert_eq!(out.lines.len(), 2);
    let w1 = adv('A') + adv('B');
    assert_eq!(out.lines[0].baseline, 50.0, "anchor ON the first baseline");
    assert_eq!(out.lines[0].glyphs[0].x, 100.0 - w1 / 2.0);
    assert_eq!(out.lines[1].baseline, 50.0 + 18.75);
    assert_eq!(out.lines[1].glyphs[0].x, 100.0 - adv('C') / 2.0);
}

#[test]
fn paragraph_overflow_clips_after_last_fitting_line_and_preserves_text() {
    let text = "AA AA";
    let (items, bounds, opps) = styled(text);
    let style = ParagraphStyle::default();
    let tight = LayoutBox {
        left: 0.0,
        top: 0.0,
        right: 30.0,
        bottom: 20.0, // line 1 bottom 15.625 fits; line 2 (34.375) not
    };
    let out = layout_paragraph(text, &items, &bounds, &opps, &style, &tight, 0.0);
    assert_eq!(out.lines.len(), 1);
    assert!(out.overflowed);
    assert_eq!(
        out.first_hidden,
        Some(3),
        "hidden text preserved from byte 3"
    );
}

// ---------------------------------------------------------------------
// Text on a path (§23.3 pseudocode + stretch policy)
// ---------------------------------------------------------------------

fn line_table(from: Vec2, to: Vec2) -> ArcLengthTable {
    let sub = SubPath {
        closed: false,
        segments: vec![CubicSeg::line(from, to)],
    };
    ArcLengthTable::new(&sub)
}

#[test]
fn on_path_straight_line_positions_are_exact() {
    let text = "AB";
    let (items, bounds, _) = styled(text);
    let table = line_table(Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0));
    let params = OnPathParams {
        start: 0.0,
        end: 100.0,
        offset: 5.0,
        flip_side: false,
    };
    let out = layout_on_path(text, &items, &bounds, &table, &params);
    assert!(!out.overflowed);
    assert_eq!(out.glyphs.len(), 2);
    // Left normal of (1,0) in y-down space is (0,−1): offset +5 lifts
    // the baseline ABOVE the path (visually up).
    assert_eq!(out.glyphs[0].pos, Vec2::new(0.0, -5.0));
    assert_eq!(out.glyphs[0].angle, 0.0);
    assert_eq!(out.glyphs[1].pos, Vec2::new(adv('A'), -5.0));
}

#[test]
fn on_path_window_overflow_truncates_and_flags() {
    let text = "AB";
    let (items, bounds, _) = styled(text);
    let table = line_table(Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0));
    let params = OnPathParams {
        start: 0.0,
        end: 15.0, // A (9.375) fits; B would end at 19.0625
        offset: 0.0,
        flip_side: false,
    };
    let out = layout_on_path(text, &items, &bounds, &table, &params);
    assert!(out.overflowed);
    assert_eq!(out.glyphs.len(), 1);
}

/// κ-cubic circle of radius r about the origin (same construction the
/// vector crate's boolean tests pin).
fn circle_sub(r: f64) -> SubPath {
    let k = 0.552_284_749_830_793_4 * r;
    let p0 = Vec2::new(r, 0.0);
    let p1 = Vec2::new(0.0, r);
    let p2 = Vec2::new(-r, 0.0);
    let p3 = Vec2::new(0.0, -r);
    SubPath {
        closed: true,
        segments: vec![
            CubicSeg {
                p0,
                c1: Vec2::new(r, k),
                c2: Vec2::new(k, r),
                p1,
            },
            CubicSeg {
                p0: p1,
                c1: Vec2::new(-k, r),
                c2: Vec2::new(-r, k),
                p1: p2,
            },
            CubicSeg {
                p0: p2,
                c1: Vec2::new(-r, -k),
                c2: Vec2::new(-k, -r),
                p1: p3,
            },
            CubicSeg {
                p0: p3,
                c1: Vec2::new(k, -r),
                c2: Vec2::new(r, -k),
                p1: p0,
            },
        ],
    }
}

#[test]
fn tight_curvature_stretches_outer_side_only_within_the_band() {
    let text = "AA";
    let (items, bounds, _) = styled(text);
    let r = 6.0; // radius < em (15.625) → tight
    let table = ArcLengthTable::new(&circle_sub(r));
    // Baseline on the path (offset 0 counts as outer): stretch caps at
    // 1.25, so cluster midpoints sit advance·1.25 apart along the arc.
    let out = layout_on_path(
        text,
        &items,
        &bounds,
        &table,
        &OnPathParams {
            start: 0.0,
            end: table.total(),
            offset: 0.0,
            flip_side: false,
        },
    );
    assert_eq!(out.glyphs.len(), 2);
    let (g1, g2) = (&out.glyphs[0], &out.glyphs[1]);
    // Angle differences read the consumed arc directly.
    let dtheta = (g2.angle - g1.angle).rem_euclid(std::f64::consts::TAU);
    let arc = dtheta * r;
    assert!(
        (arc - adv('A') * 1.25).abs() < 0.05,
        "outer-side stretch at the 1.25 cap (arc {arc})"
    );
    // Inner side (offset toward the curvature center): NO stretch.
    let inner = layout_on_path(
        text,
        &items,
        &bounds,
        &table,
        &OnPathParams {
            start: 0.0,
            end: table.total(),
            offset: -2.0,
            flip_side: false,
        },
    );
    let (h1, h2) = (&inner.glyphs[0], &inner.glyphs[1]);
    let dtheta_i = (h2.angle - h1.angle).rem_euclid(std::f64::consts::TAU);
    let arc_i = dtheta_i * r;
    assert!(
        (arc_i - adv('A')).abs() < 0.05,
        "inner side advances never stretch (arc {arc_i})"
    );
}

// ---------------------------------------------------------------------
// Determinism
// ---------------------------------------------------------------------

#[test]
fn layout_is_deterministic_end_to_end() {
    let run = || {
        let text = "AA \u{05D0}\u{05D1} A\nAAA";
        let (items, bounds, opps) = styled(text);
        let style = ParagraphStyle {
            align: Align::Justify,
            ..Default::default()
        };
        let para = layout_paragraph(text, &items, &bounds, &opps, &style, &wide_box(40.0), 0.0);
        let table = line_table(Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0));
        let path = layout_on_path(
            text,
            &items,
            &bounds,
            &table,
            &OnPathParams {
                start: 0.0,
                end: 200.0,
                offset: 3.0,
                flip_side: false,
            },
        );
        format!("{para:?}::{path:?}")
    };
    assert_eq!(run(), run());
}
