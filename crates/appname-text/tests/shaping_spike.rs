// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! CP-0.7 SHAPING SPIKE (plan §2.4 O-TEXTVEC row, item 3): the real
//! text stack — ICU4X + unicode-bidi itemization properties, HarfBuzz
//! shaping through `appname-shape-sys`, FreeType rasterization — driven
//! end-to-end through the `pipeline` seams with EXACT oracles.
//!
//! Spec: ES §23.2 [NORMATIVE] ("Identical rasterization across OSes for
//! a fixed font+string; a bidi/CJK sample renders correctly (tested)").
//! The fixture face is procedural (tests/common — authored bytes, no
//! GSUB/GPOS), so expected glyph ids and advances are knowable a priori:
//! shaping must echo cmap+hmtx verbatim. These fixed inputs/outputs are
//! the §34.1 golden-text-corpus seed; the cross-OS half of the claim
//! lands when a second CI leg exists (recorded honestly in the lane log).

mod common;

use appname_text::pipeline::{
    itemize, visual_order, FaceId, FontProbe, Item, RunSpan, ScriptClass, ScriptClassifier, Shaper,
    StyleId,
};
use appname_text::pipeline::{BidiProvider, GlyphRasterizer, GraphemeSegmenter, LineBreakProvider};
use appname_text::providers::{
    IcuGraphemeSegmenter, IcuLineBreakProvider, IcuScriptClassifier, UnicodeBidiProvider,
};
use appname_text::raster::{FreeTypeRasterizer, GlyphCoverage};
use appname_text::shape::HarfBuzzShaper;

/// Spike probe: one face covers every sample char (the §44.2 fallback
/// chain is the CP-1.0 row).
struct SpikeProbe;
impl FontProbe for SpikeProbe {
    fn resolve(&self, _style: StyleId, _cluster: &str) -> FaceId {
        FaceId(0)
    }
}

/// An em size whose 26.6 scale is exactly the 1000 upm — advances come
/// back in unscaled font units, so oracles are integer-exact.
const EM_EXACT: f64 = 1000.0 / 64.0; // scale = 1000

fn items_of(text: &str) -> Vec<Item> {
    itemize(
        &[RunSpan {
            text,
            style: StyleId(1),
        }],
        &IcuScriptClassifier::new(),
        &UnicodeBidiProvider,
        &IcuGraphemeSegmenter::new(),
        &SpikeProbe,
    )
}

fn shaper_at(em_px: f64) -> HarfBuzzShaper {
    let mut s = HarfBuzzShaper::new(em_px);
    s.add_face(FaceId(0), &common::build())
        .expect("spike face must parse");
    s
}

// ---------------------------------------------------------------------
// Face + cmap sanity
// ---------------------------------------------------------------------

#[test]
fn spike_face_parses_and_cmap_matches_the_builder() {
    let s = shaper_at(EM_EXACT);
    for c in [' ', '!', 'A', 'F', '\u{05D0}', '\u{05D1}', '\u{6C38}'] {
        assert_eq!(
            s.glyph_index(FaceId(0), c),
            Some(common::gid(c)),
            "cmap disagrees with the builder for {c:?}"
        );
    }
    assert_eq!(
        s.glyph_index(FaceId(0), 'z'),
        None,
        "unmapped char must miss"
    );
}

// ---------------------------------------------------------------------
// ASCII shaping: exact ids, advances, clusters
// ---------------------------------------------------------------------

#[test]
fn ascii_shape_is_cmap_and_hmtx_verbatim() {
    let text = "AF!";
    let items = items_of(text);
    assert_eq!(
        items.len(),
        1,
        "one Latin item ('!' resolves to Latn by carry)"
    );
    assert_eq!(items[0].script, *b"Latn");
    assert_eq!(items[0].bidi_level, 0);

    let s = shaper_at(EM_EXACT);
    let glyphs = s.shape(&items[0], text);
    let ids: Vec<u32> = glyphs.iter().map(|g| g.glyph).collect();
    assert_eq!(
        ids,
        vec![common::gid('A'), common::gid('F'), common::gid('!')]
    );
    let clusters: Vec<usize> = glyphs.iter().map(|g| g.cluster).collect();
    assert_eq!(clusters, vec![0, 1, 2], "cluster level 1, monotone");
    // scale == upm: raw advance units == hmtx; px = units/64 exactly.
    let adv: Vec<f64> = glyphs.iter().map(|g| g.advance).collect();
    assert_eq!(
        adv,
        vec![
            f64::from(common::advance('A')) / 64.0,
            f64::from(common::advance('F')) / 64.0,
            f64::from(common::advance('!')) / 64.0,
        ]
    );
    assert!(glyphs
        .iter()
        .all(|g| g.offset_x == 0.0 && g.offset_y == 0.0));
}

/// §23.2 "shaping context crosses item edges": a style-only split must
/// not change what comes out (the face has no contextual lookups, so the
/// concatenation must be glyph-for-glyph identical).
#[test]
fn item_splits_do_not_change_shaping_output() {
    let text = "AB";
    let s = shaper_at(EM_EXACT);
    let whole = items_of(text);
    assert_eq!(whole.len(), 1);
    let all = s.shape(&whole[0], text);

    let runs = [
        RunSpan {
            text: "A",
            style: StyleId(1),
        },
        RunSpan {
            text: "B",
            style: StyleId(2),
        },
    ];
    let split = itemize(
        &runs,
        &IcuScriptClassifier::new(),
        &UnicodeBidiProvider,
        &IcuGraphemeSegmenter::new(),
        &SpikeProbe,
    );
    assert_eq!(split.len(), 2, "style boundary splits the item");
    let joined: Vec<_> = split.iter().flat_map(|it| s.shape(it, text)).collect();
    assert_eq!(all, joined);
}

// ---------------------------------------------------------------------
// Bidi sample (§23.2 [NORMATIVE] "a bidi/CJK sample renders correctly")
// ---------------------------------------------------------------------

#[test]
fn bidi_sample_itemizes_levels_and_reverses_the_rtl_run() {
    let text = "AB \u{05D0}\u{05D1} C"; // "AB אב C"

    // Provider truth first: per-char UAX #9 levels.
    assert_eq!(
        UnicodeBidiProvider.levels(text),
        vec![0, 0, 0, 1, 1, 0, 0],
        "L L WS R R WS L under an LTR base paragraph"
    );

    let items = items_of(text);
    // "AB " Latn L0 (trailing WS carries Latn) · "אב" Hebr L1 ·
    // " " Hebr L0 (carry) · "C" Latn L0.
    let got: Vec<(std::ops::Range<usize>, [u8; 4], u8)> = items
        .iter()
        .map(|it| (it.range.clone(), it.script, it.bidi_level))
        .collect();
    assert_eq!(
        got,
        vec![
            (0..3, *b"Latn", 0),
            (3..7, *b"Hebr", 1),
            (7..8, *b"Hebr", 0),
            (8..9, *b"Latn", 0),
        ]
    );
    assert_eq!(visual_order(&items), vec![0, 1, 2, 3]);

    // The RTL item comes back in visual order: ב then א, clusters
    // monotone DECREASING, advances verbatim from hmtx.
    let s = shaper_at(EM_EXACT);
    let heb = s.shape(&items[1], text);
    assert_eq!(
        heb.iter().map(|g| g.glyph).collect::<Vec<_>>(),
        vec![common::gid('\u{05D1}'), common::gid('\u{05D0}')]
    );
    assert_eq!(
        heb.iter().map(|g| g.cluster).collect::<Vec<_>>(),
        vec![5, 3]
    );
    assert_eq!(
        heb.iter().map(|g| g.advance).collect::<Vec<_>>(),
        vec![560.0 / 64.0, 550.0 / 64.0]
    );
}

#[test]
fn cjk_sample_itemizes_and_shapes_exactly() {
    let text = "\u{6C38}A"; // "永A"
    let items = items_of(text);
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].script, *b"Hani");
    assert_eq!(items[0].range, 0..3);
    assert_eq!(items[1].script, *b"Latn");

    let s = shaper_at(EM_EXACT);
    let han = s.shape(&items[0], text);
    assert_eq!(han.len(), 1);
    assert_eq!(han[0].glyph, common::gid('\u{6C38}'));
    assert_eq!(han[0].advance, 1000.0 / 64.0);
    assert_eq!(han[0].cluster, 0);
}

// ---------------------------------------------------------------------
// Provider truths (ICU4X compiled data)
// ---------------------------------------------------------------------

#[test]
fn script_classifier_reports_iso15924_tags_and_classes() {
    let s = IcuScriptClassifier::new();
    assert_eq!(s.script('a'), ScriptClass::Explicit(*b"Latn"));
    assert_eq!(s.script('\u{05D0}'), ScriptClass::Explicit(*b"Hebr"));
    assert_eq!(s.script('\u{6C38}'), ScriptClass::Explicit(*b"Hani"));
    assert_eq!(s.script('\u{03B1}'), ScriptClass::Explicit(*b"Grek"));
    assert_eq!(s.script('3'), ScriptClass::Common);
    assert_eq!(s.script(' '), ScriptClass::Common);
    assert_eq!(s.script('\u{0301}'), ScriptClass::Inherited);
}

#[test]
fn grapheme_segmenter_keeps_clusters_whole() {
    let g = IcuGraphemeSegmenter::new();
    // Combining mark joins its base.
    assert_eq!(g.cluster_boundaries("e\u{0301}f"), vec![0, 3, 4]);
    // Emoji ZWJ family: ONE cluster.
    let fam = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    assert_eq!(g.cluster_boundaries(fam), vec![0, fam.len()]);
    // Empty text still satisfies the seam contract (first=0, last=len).
    assert_eq!(g.cluster_boundaries(""), vec![0]);
}

#[test]
fn line_break_opportunities_are_uax14() {
    let lb = IcuLineBreakProvider::new();
    // After the space (before 'b'), after the mandatory LF, at end.
    assert_eq!(lb.opportunities("aa bb\ncc"), vec![3, 6, 8]);
    // No interior opportunity in a single word.
    assert_eq!(lb.opportunities("word"), vec![4]);
}

// ---------------------------------------------------------------------
// FreeType rasterization (§23.2: Gray f32 coverage; hint rule)
// ---------------------------------------------------------------------

fn rasterizer() -> FreeTypeRasterizer {
    let mut r = FreeTypeRasterizer::new().expect("FreeType init");
    r.add_face(FaceId(0), common::build()).expect("face parses");
    r
}

/// em 100 > 32 ⇒ unhinted; the 'A' rect (x 100–500, y 0–700 in 1000 upm)
/// lands on EXACT pixel edges at em 100 (×0.1) — every covered pixel is
/// full, so dimensions, bearings, and Σcoverage are all integer-exact.
#[test]
fn unhinted_raster_of_an_integer_aligned_rect_is_exact() {
    let r = rasterizer();
    let cov = r.raster(FaceId(0), common::gid('A'), 100.0);
    // Expectations derive from the builder's own outline (0.1 px per
    // font unit at em 100 over 1000 upm).
    let (x0, x1, y0, y1) = common::rect('A');
    let (w, h) = ((x1 - x0) as u32 / 10, (y1 - y0) as u32 / 10);
    assert_eq!((cov.width, cov.height), (w, h));
    assert_eq!(
        (cov.left, cov.top),
        (i32::from(x0) / 10, i32::from(y1) / 10)
    );
    assert!(
        cov.data.iter().all(|&v| v == 1.0),
        "integer-aligned rect has no partial pixels"
    );
    assert_eq!(cov.total(), f64::from(w * h));
}

/// em 16 ≤ 32 ⇒ autohinter light. Hinting may snap edges (that is its
/// job) but must do so DETERMINISTICALLY, and the area stays near the
/// unhinted truth.
#[test]
fn hinted_raster_is_deterministic_and_plausible() {
    let r1 = rasterizer();
    let r2 = rasterizer();
    let a = r1.raster(FaceId(0), common::gid('A'), 16.0);
    let b = r2.raster(FaceId(0), common::gid('A'), 16.0);
    assert_eq!(a, b, "two fresh stacks, bit-identical rasters");
    let unhinted_area = (0.4 * 16.0) * (0.7 * 16.0); // 71.68 px²
    assert!(
        (a.total() - unhinted_area).abs() < 25.0,
        "hinted Σ {} strays too far from {unhinted_area}",
        a.total()
    );
    assert!(a.width > 0 && a.height > 0);
}

/// Empty glyphs (space) raster to an empty coverage, not an error.
#[test]
fn space_rasters_empty() {
    let r = rasterizer();
    let cov = r.raster(FaceId(0), common::gid(' '), 16.0);
    assert_eq!((cov.width, cov.height), (0, 0));
    assert_eq!(cov.total(), 0.0);
}

// ---------------------------------------------------------------------
// End-to-end determinism (the §34.1 golden seed)
// ---------------------------------------------------------------------

/// The whole spike — itemize → shape → raster on the bidi/CJK sample —
/// serialized and run twice through FRESHLY constructed stacks. This is
/// the same-machine half of DD ㉒; the cross-OS half is CI's when a
/// second leg exists.
#[test]
fn full_pipeline_double_run_is_byte_identical() {
    fn run_once() -> String {
        let text = "AB \u{05D0}\u{05D1} C \u{6C38}!";
        let items = items_of(text);
        let s = shaper_at(16.0);
        let r = rasterizer();
        let mut out = String::new();
        for (i, it) in items.iter().enumerate() {
            out.push_str(&format!(
                "item{} {:?} {} lvl{} |",
                i,
                it.range,
                std::str::from_utf8(&it.script).unwrap(),
                it.bidi_level
            ));
            for g in s.shape(it, text) {
                out.push_str(&format!(
                    " g{} c{} a{:.6} o{:.6},{:.6};",
                    g.glyph, g.cluster, g.advance, g.offset_x, g.offset_y
                ));
                let cov: GlyphCoverage = r.raster(FaceId(0), g.glyph, 16.0);
                out.push_str(&format!(
                    " r{}x{}@{},{}s{:.6}|",
                    cov.width,
                    cov.height,
                    cov.left,
                    cov.top,
                    cov.total()
                ));
            }
        }
        out.push_str(&format!(" order{:?}", visual_order(&items)));
        out
    }
    let a = run_once();
    let b = run_once();
    assert_eq!(a, b);
    assert!(!a.is_empty());
}
