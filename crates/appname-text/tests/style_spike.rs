// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! §23.4 style-model acceptance (CP-1.0 slice C-4): decoration and
//! position metrics on BOTH spike-face arms (post/OS/2 present vs the
//! normative fallbacks), tracking as inter-cluster gaps with exact
//! oracles, and the shaping-input plumbing.

mod common;

use appname_text::layout::{layout_paragraph, LayoutBox, ParagraphStyle, StyledItem};
use appname_text::pipeline::{
    itemize, FaceId, FontProbe, GraphemeSegmenter, LineBreakProvider, RunSpan, Shaper, StyleId,
};
use appname_text::providers::{
    IcuGraphemeSegmenter, IcuLineBreakProvider, IcuScriptClassifier, UnicodeBidiProvider,
};
use appname_text::raster::FreeTypeRasterizer;
use appname_text::shape::{feature_overlay, HarfBuzzShaper, ENGINE_DEFAULT_FEATURES};
use appname_text::style::{
    CharStyle, DecorLine, Position, DECOR_FALLBACK_THICKNESS_EM, POSITION_FALLBACK_SCALE,
    POSITION_FALLBACK_SHIFT_EM, STRIKEOUT_FALLBACK_POS_EM,
};

const EM: f64 = 1000.0 / 64.0;

struct OneFace;
impl FontProbe for OneFace {
    fn resolve(&self, _s: StyleId, _c: &str) -> FaceId {
        FaceId(0)
    }
}

fn adv(c: char) -> f64 {
    f64::from(common::advance(c)) / 64.0
}

// ---------------------------------------------------------------------
// Decoration + position metrics: font arm vs fallback arm
// ---------------------------------------------------------------------

#[test]
fn underline_uses_post_metrics_when_present() {
    let mut r = FreeTypeRasterizer::new().unwrap();
    r.add_face(FaceId(0), common::build()).unwrap();
    // Spike post: underlinePosition −75, thickness 50, upm 1000.
    let em = 16.0;
    let got = r.underline_line(FaceId(0), em);
    assert_eq!(
        got,
        DecorLine {
            position: -75.0 * em / 1000.0,
            thickness: 50.0 * em / 1000.0
        },
        "post-table arm exact"
    );
}

#[test]
fn strikeout_and_position_fall_back_when_os2_metrics_are_absent_or_zero() {
    let mut r = FreeTypeRasterizer::new().unwrap();
    r.add_face(FaceId(0), common::build()).unwrap(); // no OS/2 at all
    r.add_face(FaceId(1), common::build_with_typo_os2())
        .unwrap(); // OS/2, zeroed decor fields
    let em = 16.0;
    for face in [FaceId(0), FaceId(1)] {
        let s = r.strikeout_line(face, em);
        assert_eq!(s.position, STRIKEOUT_FALLBACK_POS_EM * em);
        assert_eq!(s.thickness, DECOR_FALLBACK_THICKNESS_EM * em);
        let sup = r.position_metrics(face, em, Position::Super);
        assert_eq!(sup.scale, POSITION_FALLBACK_SCALE);
        assert_eq!(sup.shift, POSITION_FALLBACK_SHIFT_EM * em);
        let sub = r.position_metrics(face, em, Position::Sub);
        assert_eq!(sub.shift, -POSITION_FALLBACK_SHIFT_EM * em);
        let norm = r.position_metrics(face, em, Position::Normal);
        assert_eq!((norm.scale, norm.shift), (1.0, 0.0));
    }
    // Underline on the no-post arm? Both spike faces carry post →
    // exercise the fallback constant only via the zero-thickness rule:
    // (spike post thickness is 50 ≠ 0, so the font arm is used — the
    // fallback arm for underline is covered by the constants test).
    let u = r.underline_line(FaceId(1), em);
    assert_eq!(
        u.thickness,
        50.0 * em / 1000.0,
        "post arm on the OS/2 face too"
    );
}

// ---------------------------------------------------------------------
// Tracking: inter-cluster gaps, suppressed line-final by construction
// ---------------------------------------------------------------------

fn styled_with_tracking(text: &str, tracking_px: f64) -> (Vec<StyledItem>, Vec<usize>, Vec<usize>) {
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
    let styled = items
        .into_iter()
        .map(|item| {
            let glyphs = shaper.shape(&item, text);
            StyledItem {
                item,
                glyphs,
                em_px: EM,
                metrics,
                tracking_px,
            }
        })
        .collect();
    let bounds = IcuGraphemeSegmenter::new().cluster_boundaries(text);
    let opps = IcuLineBreakProvider::new().opportunities(text);
    (styled, bounds, opps)
}

#[test]
fn os2_font_arm_metrics_are_exact_including_the_subscript_sign() {
    // The DECOR face carries NONZERO OS/2 decor metrics — this is the
    // font arm the fallback faces cannot reach (C-4 review MINOR),
    // including the sign normalization: ySuperscriptYOffset is
    // positive-UP (passthrough), ySubscriptYOffset positive-DOWN
    // (negated into the y-up convention).
    let mut r = FreeTypeRasterizer::new().unwrap();
    r.add_face(FaceId(0), common::build_with_decor_os2())
        .unwrap();
    let em = 16.0;
    let upm = 1000.0;
    let s = r.strikeout_line(FaceId(0), em);
    assert_eq!(s.position, f64::from(common::DECOR_STRIKEOUT.0) * em / upm);
    assert_eq!(s.thickness, f64::from(common::DECOR_STRIKEOUT.1) * em / upm);
    let sup = r.position_metrics(FaceId(0), em, Position::Super);
    assert_eq!(sup.scale, f64::from(common::DECOR_SUPER.0) / upm);
    assert_eq!(
        sup.shift,
        f64::from(common::DECOR_SUPER.1) * em / upm,
        "super: positive-UP passthrough"
    );
    let sub = r.position_metrics(FaceId(0), em, Position::Sub);
    assert_eq!(sub.scale, f64::from(common::DECOR_SUB.0) / upm);
    assert_eq!(
        sub.shift,
        -f64::from(common::DECOR_SUB.1) * em / upm,
        "sub: positive-DOWN negated into y-up"
    );
}

#[test]
fn rtl_tracking_gaps_are_interior_with_no_phantom_trailing_gap() {
    // C-4 review MAJOR regression (the reviewer's probe): three Hebrew
    // letters with tracking 2.0 must show BOTH interior gaps widened,
    // no phantom gap past the visually-rightmost glyph, and
    // LaidLine.width equal to the ink extent.
    let track = 2.0;
    let text = "\u{05D0}\u{05D1}\u{05D0}";
    let (items, bounds, opps) = styled_with_tracking(text, track);
    let box_ = LayoutBox {
        left: 0.0,
        top: 0.0,
        right: 1e6,
        bottom: 1e6,
    };
    let style = ParagraphStyle {
        rtl_base: true,
        ..Default::default()
    };
    let out = layout_paragraph(text, &items, &bounds, &opps, &style, &box_, 0.0);
    let l = &out.lines[0];
    assert_eq!(l.glyphs.len(), 3);
    let mut xs: Vec<f64> = l.glyphs.iter().map(|g| g.x).collect();
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let (a1, a2) = (adv('\u{05D0}'), adv('\u{05D1}'));
    // Visual pen order (RTL): logical last א, then ב, then first א.
    assert_eq!(xs[1] - xs[0], a1 + track, "left interior gap widened");
    assert_eq!(xs[2] - xs[1], a2 + track, "right interior gap widened");
    // Width equals the ink extent: rightmost glyph end − line origin.
    let ink = xs[2] + a1 - xs[0];
    assert_eq!(l.width, ink, "no phantom trailing gap in the width");
    assert_eq!(l.width, 2.0 * a1 + a2 + 2.0 * track);
}

#[test]
fn mixed_tracking_rtl_item_boundary_width_equals_ink() {
    // C-4 re-review F1 regression: two RTL items with DIFFERENT
    // tracking (2.0 / 6.0) on one line. The boundary gap must take the
    // LOGICALLY-earlier endpoint's tracking, so placement matches
    // measure_width's logical n−1 sum and LaidLine.width equals the
    // ink extent (visually-earlier attribution diverged by tb−ta).
    let (ta, tb) = (2.0, 6.0);
    let text = "\u{05D0}\u{05D1}\u{05D0}\u{05D1}";
    let runs = [
        RunSpan {
            text: &text[0..4],
            style: StyleId(1),
        },
        RunSpan {
            text: &text[4..8],
            style: StyleId(2),
        },
    ];
    let items = itemize(
        &runs,
        &IcuScriptClassifier::new(),
        &UnicodeBidiProvider,
        &IcuGraphemeSegmenter::new(),
        &OneFace,
    );
    assert_eq!(items.len(), 2, "style boundary splits the RTL run");
    let mut shaper = HarfBuzzShaper::new(EM);
    shaper.add_face(FaceId(0), &common::build()).unwrap();
    let mut rast = FreeTypeRasterizer::new().unwrap();
    rast.add_face(FaceId(0), common::build()).unwrap();
    let metrics = rast.metrics(FaceId(0), EM);
    let styled: Vec<StyledItem> = items
        .into_iter()
        .map(|item| {
            let track = if item.style == StyleId(1) { ta } else { tb };
            let glyphs = shaper.shape(&item, text);
            StyledItem {
                item,
                glyphs,
                em_px: EM,
                metrics,
                tracking_px: track,
            }
        })
        .collect();
    let bounds = IcuGraphemeSegmenter::new().cluster_boundaries(text);
    let opps = IcuLineBreakProvider::new().opportunities(text);
    let style = ParagraphStyle {
        rtl_base: true,
        ..Default::default()
    };
    let box_ = LayoutBox {
        left: 0.0,
        top: 0.0,
        right: 1e6,
        bottom: 1e6,
    };
    let out = layout_paragraph(text, &styled, &bounds, &opps, &style, &box_, 0.0);
    let l = &out.lines[0];
    // Logical gaps: after c0 (ta), c1 (ta), c2 (tb) → Σadv + 2ta + tb.
    let (a1, a2) = (adv('\u{05D0}'), adv('\u{05D1}'));
    let want = 2.0 * (a1 + a2) + 2.0 * ta + tb;
    assert_eq!(l.width, want, "width = logical n−1 gap sum");
    // Ink extent equals the width (the F1 divergence was tb − ta).
    let min_x = l.glyphs.iter().map(|g| g.x).fold(f64::INFINITY, f64::min);
    let max_end = l
        .glyphs
        .iter()
        .map(|g| {
            let c = if g.cluster % 4 == 0 { a1 } else { a2 };
            g.x + c
        })
        .fold(f64::NEG_INFINITY, f64::max);
    assert_eq!(max_end - min_x, want, "ink extent == width");
}

#[test]
fn tracking_widens_gaps_but_never_after_the_line_final_cluster() {
    let track = 2.0;
    let (items, bounds, opps) = styled_with_tracking("AAA", track);
    let box_ = LayoutBox {
        left: 0.0,
        top: 0.0,
        right: 1e6,
        bottom: 1e6,
    };
    let out = layout_paragraph(
        "AAA",
        &items,
        &bounds,
        &opps,
        &ParagraphStyle::default(),
        &box_,
        0.0,
    );
    let l = &out.lines[0];
    // Pen: A, gap+track, A, gap+track, A — two gaps, no trailing one.
    assert_eq!(l.glyphs[0].x, 0.0);
    assert_eq!(l.glyphs[1].x, adv('A') + track);
    assert_eq!(l.glyphs[2].x, 2.0 * (adv('A') + track));
    assert_eq!(
        l.width,
        3.0 * adv('A') + 2.0 * track,
        "width counts 2 gaps only"
    );
}

#[test]
fn tracking_participates_in_line_breaking() {
    // Without tracking three A's fit in 30 px (28.125); tracking 2 px
    // pushes the 3-cluster width to 32.125 → only two fit per line.
    let (items, bounds, opps) = styled_with_tracking("AAAAAA", 2.0);
    let box_ = LayoutBox {
        left: 0.0,
        top: 0.0,
        right: 30.0,
        bottom: 1e6,
    };
    let out = layout_paragraph(
        "AAAAAA",
        &items,
        &bounds,
        &opps,
        &ParagraphStyle::default(),
        &box_,
        0.0,
    );
    assert_eq!(out.lines.len(), 3, "2+2+2 under tracking");
    assert_eq!(out.lines[0].range, 0..2);
}

// ---------------------------------------------------------------------
// Shaping-input plumbing
// ---------------------------------------------------------------------

#[test]
fn feature_overlay_is_defaults_then_run_features_in_order() {
    let style = CharStyle {
        features: vec![(*b"liga", 0), (*b"ss01", 1)],
        ..Default::default()
    };
    let ov = feature_overlay(&style);
    assert_eq!(ov.len(), ENGINE_DEFAULT_FEATURES.len() + 2);
    assert_eq!(&ov[..3], &ENGINE_DEFAULT_FEATURES[..], "defaults first");
    assert_eq!(ov[3].tag, *b"liga");
    // Claim scope: this pins the OVERLAY ORDER only; HarfBuzz's
    // duplicate-tag later-wins behavior was verified by the C-4
    // reviewer in the vendored 8.4.0 source (hb-ot-map.cc merge loop)
    // — a behavioral test needs a feature-bearing face (C-5).
    assert_eq!(
        ov[3].value, 0,
        "run feature can disable a default (order pinned)"
    );
    assert_eq!(ov[4].tag, *b"ss01");
}

#[test]
fn liga_face_proves_engine_default_and_later_wins_behaviorally() {
    // THE behavioral test the C-4/C-5 reviews called for, on the
    // GSUB-bearing LIGA face (A+B → the 永 glyph):
    // 1. engine default liga ON: "AB" shapes to ONE ligature glyph;
    // 2. run-feature overlay liga=0 (LATER duplicate-tag entry WINS in
    //    HarfBuzz): the ligature does NOT form — two component glyphs.
    let text = "AB";
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
    assert_eq!(items.len(), 1);
    let mut shaper = HarfBuzzShaper::new(EM);
    shaper
        .add_face(FaceId(0), &common::build_with_liga())
        .unwrap();
    // Default features: ligature forms.
    let lig = shaper.shape(&items[0], text);
    assert_eq!(
        lig.iter().map(|g| g.glyph).collect::<Vec<_>>(),
        vec![common::gid('\u{6C38}')],
        "engine-default liga substitutes A+B"
    );
    assert_eq!(lig[0].cluster, 0, "ligature carries the cluster start");
    // Run feature liga=0 overlays AFTER the default: later wins.
    let style = CharStyle {
        features: vec![(*b"liga", 0)],
        ..Default::default()
    };
    shaper.set_style(StyleId(1), &style);
    let plain = shaper.shape(&items[0], text);
    assert_eq!(
        plain.iter().map(|g| g.glyph).collect::<Vec<_>>(),
        vec![common::gid('A'), common::gid('B')],
        "run-feature liga=0 disables the default — later wins, BEHAVIORALLY"
    );
}

#[test]
fn per_style_language_and_features_still_shape_identically_on_the_plain_face() {
    // The spike face has no GSUB/locl, so language/features must not
    // CHANGE anything — this pins that the plumbing itself is neutral
    // (real effects need a feature-bearing face; C-5 corpus).
    let text = "AF!";
    let (plain, bounds, _) = styled_with_tracking(text, 0.0);
    let mut shaper = HarfBuzzShaper::new(EM);
    shaper.add_face(FaceId(0), &common::build()).unwrap();
    let style = CharStyle {
        language: Some("tr".into()),
        features: vec![(*b"ss01", 1)],
        ..Default::default()
    };
    shaper.set_style(StyleId(1), &style);
    let with_style: Vec<_> = plain
        .iter()
        .map(|si| shaper.shape(&si.item, text))
        .collect();
    let baseline: Vec<_> = plain.iter().map(|si| si.glyphs.clone()).collect();
    assert_eq!(with_style, baseline);
    let _ = bounds;
}
