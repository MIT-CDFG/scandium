// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! C-5 slice 2 acceptance: the unified gap mechanism — ligature
//! absorption + join suppression (§23.4 "suppressed inside cursive
//! joins") and the bidi-level-aware gap ownership that closes the C-4
//! pinned residual (its recorded probe is the oracle).

mod common;

use appname_text::layout::{layout_paragraph, LayoutBox, ParagraphStyle, StyledItem};
use appname_text::pipeline::{
    itemize, FaceId, FontProbe, GraphemeSegmenter, LineBreakProvider, RunSpan, Shaper, StyleId,
};
use appname_text::providers::{
    IcuGraphemeSegmenter, IcuLineBreakProvider, IcuScriptClassifier, UnicodeBidiProvider,
};
use appname_text::raster::FreeTypeRasterizer;
use appname_text::shape::HarfBuzzShaper;

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

#[test]
fn ligature_interior_boundaries_take_no_tracking() {
    // LIGA face: "ABAB" forms two A+B ligatures (the 永 glyph). The
    // grapheme boundaries INSIDE each ligature are absorbed — tracking
    // a boundary inside a ligature would visually split the glyph —
    // so with tracking 2 the line has exactly ONE gap (between the
    // ligatures): width = 2·adv(永) + 2.
    let track = 2.0;
    let text = "ABAB";
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
    shaper
        .add_face(FaceId(0), &common::build_with_liga())
        .unwrap();
    let mut rast = FreeTypeRasterizer::new().unwrap();
    rast.add_face(FaceId(0), common::build_with_liga()).unwrap();
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
                tracking_px: track,
            }
        })
        .collect();
    assert_eq!(
        styled[0].glyphs.len(),
        2,
        "two ligature glyphs (the shaping precondition)"
    );
    let bounds = IcuGraphemeSegmenter::new().cluster_boundaries(text);
    let opps = IcuLineBreakProvider::new().opportunities(text);
    let out = layout_paragraph(
        text,
        &styled,
        &bounds,
        &opps,
        &ParagraphStyle::default(),
        &LayoutBox {
            left: 0.0,
            top: 0.0,
            right: 1e6,
            bottom: 1e6,
        },
        0.0,
    );
    let l = &out.lines[0];
    let lig = adv('\u{6C38}');
    assert_eq!(l.width, 2.0 * lig + track, "ONE gap between the ligatures");
    assert_eq!(l.glyphs.len(), 2);
    assert_eq!(l.glyphs[0].x, 0.0);
    assert_eq!(
        l.glyphs[1].x,
        lig + track,
        "gap lands between, never inside"
    );
}

#[test]
fn bidi_direction_edge_gap_ownership_matches_the_recorded_probe() {
    // The C-4 pinned residual's recorded probe, now the closing
    // oracle: base-LTR line A + א(t=2) ב(t=6) + F (A/F untracked).
    // Logical gaps: (A,א)←0, (א,ב)←2, (ב,F)←6. The plan places the
    // deferred direction-edge gaps in order, so width == ink for this
    // heterogeneous mixed-direction line — the divergence class
    // (tb−ta) is dead.
    let text = "A\u{05D0}\u{05D1}F";
    let runs = [
        RunSpan {
            text: &text[0..1],
            style: StyleId(1),
        },
        RunSpan {
            text: &text[1..3],
            style: StyleId(2),
        },
        RunSpan {
            text: &text[3..5],
            style: StyleId(3),
        },
        RunSpan {
            text: &text[5..6],
            style: StyleId(4),
        },
    ];
    let items = itemize(
        &runs,
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
            let track = match item.style {
                StyleId(2) => 2.0,
                StyleId(3) => 6.0,
                _ => 0.0,
            };
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
    let out = layout_paragraph(
        text,
        &styled,
        &bounds,
        &opps,
        &ParagraphStyle::default(),
        &LayoutBox {
            left: 0.0,
            top: 0.0,
            right: 1e6,
            bottom: 1e6,
        },
        0.0,
    );
    let l = &out.lines[0];
    let want = adv('A') + adv('\u{05D0}') + adv('\u{05D1}') + adv('F') + 0.0 + 2.0 + 6.0;
    assert_eq!(l.width, want, "width = the logical gap sum");
    // Ink extent == width: no phantom, no double-charge at either
    // direction edge.
    let min_x = l.glyphs.iter().map(|g| g.x).fold(f64::INFINITY, f64::min);
    let max_end = l
        .glyphs
        .iter()
        .map(|g| g.x + g.advance)
        .fold(f64::NEG_INFINITY, f64::max);
    assert_eq!(
        max_end - min_x,
        want,
        "ink extent == width (residual class dead)"
    );
    // Determinism.
    let out2 = layout_paragraph(
        text,
        &styled,
        &bounds,
        &opps,
        &ParagraphStyle::default(),
        &LayoutBox {
            left: 0.0,
            top: 0.0,
            right: 1e6,
            bottom: 1e6,
        },
        0.0,
    );
    assert_eq!(out.lines, out2.lines);
}

#[test]
fn kerned_latin_keeps_its_tracking() {
    // Review D1 regression (the reviewer's GPOS probe, now a fixture):
    // GPOS pair kerning sets UNSAFE_TO_BREAK, but a kerned LATIN pair
    // is NOT a cursive join — the Joining_Type gate must keep the
    // tracking gap. Oracle exact: kern applies to A's advance
    // (600−80)/64, the flag is observed on B (proving the gate is
    // actually exercised, not vacuously passed), and layout width =
    // kerned advances + BOTH tracking gaps.
    let track = 2.0;
    let text = "ABF";
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
    shaper
        .add_face(FaceId(0), &common::build_with_kern())
        .unwrap();
    let glyphs = shaper.shape(&items[0], text);
    let kerned_a = (f64::from(common::advance('A')) + f64::from(common::KERN_AB)) / 64.0;
    assert_eq!(glyphs[0].advance, kerned_a, "kern applied to A");
    assert!(
        glyphs.iter().any(|g| g.unsafe_to_break),
        "the kern DID set UNSAFE_TO_BREAK — the gate is live, not vacuous"
    );
    let mut rast = FreeTypeRasterizer::new().unwrap();
    rast.add_face(FaceId(0), common::build_with_kern()).unwrap();
    let metrics = rast.metrics(FaceId(0), EM);
    let styled = vec![StyledItem {
        item: items[0].clone(),
        glyphs,
        em_px: EM,
        metrics,
        tracking_px: track,
    }];
    let bounds = IcuGraphemeSegmenter::new().cluster_boundaries(text);
    let opps = IcuLineBreakProvider::new().opportunities(text);
    let out = layout_paragraph(
        text,
        &styled,
        &bounds,
        &opps,
        &ParagraphStyle::default(),
        &LayoutBox {
            left: 0.0,
            top: 0.0,
            right: 1e6,
            bottom: 1e6,
        },
        0.0,
    );
    let l = &out.lines[0];
    let want = kerned_a + adv('B') + adv('F') + 2.0 * track;
    assert_eq!(l.width, want, "kerned Latin keeps BOTH tracking gaps (D1)");
}

#[test]
fn arabic_joining_pair_suppresses_tracking_even_on_the_plain_face() {
    // Reviewer-discovered (re-review round): HB's arabic shaper marks
    // join-forming boundaries from CHARACTER data (the tatweel-fallback
    // path sets UNSAFE_TO_BREAK) even when the face has no Arabic
    // glyphs — so the SUPPRESS arm is behaviorally testable today.
    // BEH+BEH (DualJoining meets DualJoining): flag fires AND the
    // Joining_Type gate agrees → tracking suppressed. ALEF+BEH
    // (RightJoining then Dual: ALEF does not join ONWARD): no
    // suppression → tracking kept. Only font-behavioral joining (real
    // init/medi/fina forms) still needs the C-6 corpus.
    let track = 4.0;
    let mk = |text: &str| {
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
                    tracking_px: track,
                }
            })
            .collect();
        let bounds = IcuGraphemeSegmenter::new().cluster_boundaries(text);
        let opps = IcuLineBreakProvider::new().opportunities(text);
        let out = layout_paragraph(
            text,
            &styled,
            &bounds,
            &opps,
            &ParagraphStyle {
                rtl_base: true,
                ..Default::default()
            },
            &LayoutBox {
                left: 0.0,
                top: 0.0,
                right: 1e6,
                bottom: 1e6,
            },
            0.0,
        );
        let glyph_sum: f64 = out.lines[0].glyphs.iter().map(|g| g.advance).sum();
        out.lines[0].width - glyph_sum // the tracking actually applied
    };
    // BEH ب U+0628 is DualJoining; ALEF ا U+0627 is RightJoining.
    assert_eq!(mk("\u{0628}\u{0628}"), 0.0, "D+D joining pair: suppressed");
    assert_eq!(
        mk("\u{0627}\u{0628}"),
        track,
        "ALEF joins nothing onward: kept"
    );
}

#[test]
fn plain_face_glyphs_carry_no_join_flags() {
    // Plumbing floor: the featureless face has no join/ligature
    // contexts — every flag false (behavioral join-flag coverage needs
    // a joining-script face; C-6 corpus, recorded limit).
    let text = "AF!";
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
    let glyphs = shaper.shape(&items[0], text);
    assert!(glyphs.iter().all(|g| !g.unsafe_to_break));
}
