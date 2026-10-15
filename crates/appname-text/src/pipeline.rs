// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Shaping-pipeline SEAMS (CP-0.7 groundwork, plan §2.4 row 3 — the spike
//! DESIGN; full engine at CP-1.0).
//!
//! Spec: ES §23.2 [NORMATIVE] itemize → HarfBuzz shape → ICU breaks →
//! layout → FreeType raster, pixel-identical on every OS; §23.1 run-model
//! invariants; Part 44 (§44.2 fallback chain — per-grapheme-cluster,
//! never splits a cluster; cluster-count invariant property-tested per
//! §44.8); DD ㉒.
//!
//! WHY TRAITS: HarfBuzz/FreeType/ICU carry NO deps records in `deps/` yet
//! (rule 6 — records land via O-BUILD first; REQUEST drafts in
//! reports/O-TEXTVEC-log.md). This module therefore defines the exact
//! BOUNDARY each external will plug into, so the spike is a drop-in the
//! moment records merge:
//!
//! * [`ScriptClassifier`]  ← ICU/ICU4X (UAX #24 script property)
//! * [`BidiProvider`]      ← ICU/unicode-bidi (UAX #9 levels)
//! * [`GraphemeSegmenter`] ← ICU/ICU4X (UAX #29 cluster boundaries)
//! * [`LineBreakProvider`] ← ICU/ICU4X (UAX #14 opportunities)
//! * [`FontProbe`]         ← FontDb + §44.2 six-step fallback chain
//! * [`Shaper`]            ← HarfBuzz (cluster level 1, monotone clusters)
//! * [`GlyphRasterizer`]   ← FreeType (Gray f32 coverage, §23.2 — the
//!   SAME representation as `appname-vector`'s `CoveragePlane` / §13.1
//!   masks; DD ㉕/㉒ meet here)
//!
//! What IS implemented dep-free (all deterministic, all tested): the
//! four-partition itemizer (§23.2 — style × script × bidi level × resolved
//! face, maximal uniform runs = the intersection of the four partitions)
//! and the UAX #9 L2 visual reordering over provider-supplied levels.
//! What is NOT implemented here: any Unicode property data, any shaping,
//! any rasterization — providers are seams, and the test doubles say so
//! in their names. No system text API will ever stand behind these traits
//! (§23.2 determinism rule: pinned vendored libs only).

use core::ops::Range;

/// Opaque style identity (the §23.1 `StyledRun` boundary — full style
/// payload arrives with the CP-1.0 text model; itemization only needs
/// equality).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StyleId(pub u32);

/// Opaque resolved face identity (§44.2 step outcome; FontDb key).
/// Ordered/hashable so cache and index keys can carry it directly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FaceId(pub u32);

/// UAX #24 script class as the itemizer consumes it. `Common`/`Inherited`
/// resolve to the surrounding script (§23.2 itemization rule 2); providers
/// report the RAW class and the itemizer does the resolution, so the rule
/// lives in exactly one place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptClass {
    /// A concrete script, tagged by its four-byte OpenType tag.
    Explicit([u8; 4]),
    /// UAX #24 Common (punctuation, digits, spaces…).
    Common,
    /// UAX #24 Inherited (combining marks).
    Inherited,
}

/// §23.2 itemization input: one logical-order styled run of the paragraph.
#[derive(Clone, Debug)]
pub struct RunSpan<'a> {
    pub text: &'a str,
    pub style: StyleId,
}

/// §23.2 itemization output: a maximal run uniform in style, script, bidi
/// level, and resolved face. `range` is in paragraph scalar (byte)
/// indices; items are produced in LOGICAL order.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub range: Range<usize>,
    pub style: StyleId,
    pub script: [u8; 4],
    pub bidi_level: u8,
    pub face: FaceId,
}

/// Seam ← ICU/ICU4X: UAX #24 script property per character.
pub trait ScriptClassifier {
    fn script(&self, c: char) -> ScriptClass;
}

/// Seam ← ICU: UAX #9 resolved embedding level per character of the
/// paragraph (base direction handling per §23.4 `ParagraphStyle.direction`
/// is the caller's, CP-1.0).
pub trait BidiProvider {
    fn levels(&self, paragraph: &str) -> Vec<u8>;
}

/// Seam ← ICU/ICU4X: UAX #29 grapheme-cluster boundaries (byte offsets,
/// ascending, first = 0, last = text.len()).
pub trait GraphemeSegmenter {
    fn cluster_boundaries(&self, text: &str) -> Vec<usize>;
}

/// Seam ← ICU/ICU4X: UAX #14 line-break opportunities (byte offsets;
/// §23.3 restricts them to cluster boundaries at layout time).
pub trait LineBreakProvider {
    fn opportunities(&self, text: &str) -> Vec<usize>;
}

/// Seam ← FontDb + the §44.2 six-step fallback chain: resolve the face
/// for one grapheme cluster under a run's requested style. Whole-cluster
/// cmap coverage is the provider's contract (§44.2 "a fallback boundary
/// never splits a grapheme cluster").
pub trait FontProbe {
    fn resolve(&self, style: StyleId, cluster: &str) -> FaceId;
}

/// One shaped glyph (§23.2 HarfBuzz outputs; font units already scaled by
/// `em_px = size_pt · PPI / 72`, §19.1). `cluster` is a paragraph byte
/// index (cluster level 1 — monotone character clusters).
#[derive(Clone, Debug, PartialEq)]
pub struct ShapedGlyph {
    pub glyph: u32,
    pub cluster: usize,
    pub advance: f64,
    pub offset_x: f64,
    pub offset_y: f64,
    /// Breaking at this glyph's cluster start would change shaping
    /// (HarfBuzz UNSAFE_TO_BREAK) — layout suppresses tracking at such
    /// boundaries (§23.4 "suppressed inside cursive joins").
    pub unsafe_to_break: bool,
}

/// Seam ← HarfBuzz. Direction/script/language/features derive from the
/// item + run style; pre/post context crosses item edges (§23.2).
pub trait Shaper {
    fn shape(&self, item: &Item, text: &str) -> Vec<ShapedGlyph>;
}

/// Seam ← FreeType: one glyph's Gray f32 coverage raster (§23.2
/// rasterization — same representation as §20.2 fill coverage and §13.1
/// masks). Groundwork keeps the payload opaque; the CP-1.0 engine binds
/// it to the tile pipeline.
pub trait GlyphRasterizer {
    type Coverage;
    fn raster(&self, face: FaceId, glyph: u32, em_px: f64) -> Self::Coverage;
}

/// §23.2 itemization: split logical-order styled runs into maximal items
/// uniform in all four of style, script, bidi level, resolved face — the
/// intersection of the four partitions. Deterministic: single forward
/// pass, cluster-aligned (face resolution is per cluster, §44.2).
pub fn itemize<S, B, G, F>(
    runs: &[RunSpan<'_>],
    scripts: &S,
    bidi: &B,
    graphemes: &G,
    fonts: &F,
) -> Vec<Item>
where
    S: ScriptClassifier,
    B: BidiProvider,
    G: GraphemeSegmenter,
    F: FontProbe,
{
    // Paragraph string (the §23.1 invariant: concatenation of run texts
    // IS the layer string — no second backing store).
    let paragraph: String = runs.iter().map(|r| r.text).collect();
    if paragraph.is_empty() {
        return Vec::new();
    }
    let levels = bidi.levels(&paragraph);
    debug_assert_eq!(levels.len(), paragraph.chars().count());

    // Style at each byte range boundary.
    let mut style_at = Vec::new(); // (start_byte, StyleId)
    let mut off = 0;
    for r in runs {
        style_at.push((off, r.style));
        off += r.text.len();
    }
    let style_of = |byte: usize| -> StyleId {
        let mut cur = style_at[0].1;
        for &(s, id) in &style_at {
            if s <= byte {
                cur = id;
            } else {
                break;
            }
        }
        cur
    };

    // Walk grapheme clusters; resolve script (with Common/Inherited
    // carry-over) and face per cluster; emit a new item at any change in
    // the four-tuple.
    let bounds = graphemes.cluster_boundaries(&paragraph);
    debug_assert!(bounds.first() == Some(&0) && bounds.last() == Some(&paragraph.len()));

    let mut items: Vec<Item> = Vec::new();
    let mut char_index = 0usize; // running char position for bidi levels
    let mut last_explicit: [u8; 4] = *b"DFLT"; // script carry (resolution)
    for w in bounds.windows(2) {
        let (start, end) = (w[0], w[1]);
        let cluster = &paragraph[start..end];
        // Cluster script: first non-Common/Inherited char wins; else the
        // carried surrounding script (§23.2 rule 2 — paired-bracket
        // resolution is a CP-1.0 refinement, noted in the design).
        let mut script = None;
        for c in cluster.chars() {
            if let ScriptClass::Explicit(tag) = scripts.script(c) {
                script = Some(tag);
                break;
            }
        }
        let script = script.unwrap_or(last_explicit);
        last_explicit = script;

        let n_chars = cluster.chars().count();
        let level = levels[char_index]; // cluster level = its first char's
        char_index += n_chars;

        let style = style_of(start);
        let face = fonts.resolve(style, cluster);

        match items.last_mut() {
            Some(it)
                if it.style == style
                    && it.script == script
                    && it.bidi_level == level
                    && it.face == face
                    && it.range.end == start =>
            {
                it.range.end = end;
            }
            _ => items.push(Item {
                range: start..end,
                style,
                script,
                bidi_level: level,
                face,
            }),
        }
    }
    items
}

/// UAX #9 rule L2 over item bidi levels (§23.2 "items … reassemble per
/// line in visual order by bidi level"): from the highest level down to
/// the lowest odd level, reverse every maximal run of items at or above
/// that level. Pure and deterministic; levels come from the [`BidiProvider`].
pub fn visual_order(items: &[Item]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..items.len()).collect();
    if items.is_empty() {
        return order;
    }
    let max = items.iter().map(|i| i.bidi_level).max().unwrap_or(0);
    let min_odd = items
        .iter()
        .map(|i| i.bidi_level)
        .filter(|l| l % 2 == 1)
        .min()
        .unwrap_or(max.saturating_add(1));
    let mut level = max;
    while level >= min_odd && level > 0 {
        let mut i = 0;
        while i < order.len() {
            if items[order[i]].bidi_level >= level {
                let mut j = i;
                while j < order.len() && items[order[j]].bidi_level >= level {
                    j += 1;
                }
                order[i..j].reverse();
                i = j;
            } else {
                i += 1;
            }
        }
        level -= 1;
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- test doubles (named as such: these are SEAM exercisers, not
    // Unicode implementations; the real providers arrive with the deps
    // records) ----

    /// ASCII letters → "Latn"; a designated char range → "Grek"; digits/
    /// space/punct → Common; combining-diacritic block → Inherited.
    struct DoubleScript;
    impl ScriptClassifier for DoubleScript {
        fn script(&self, c: char) -> ScriptClass {
            if c.is_ascii_alphabetic() {
                ScriptClass::Explicit(*b"Latn")
            } else if ('\u{0370}'..='\u{03FF}').contains(&c) {
                ScriptClass::Explicit(*b"Grek")
            } else if ('\u{0300}'..='\u{036F}').contains(&c) {
                ScriptClass::Inherited
            } else {
                ScriptClass::Common
            }
        }
    }

    /// Greek chars get level 1 (stand-in for an RTL segment), rest 0.
    struct DoubleBidi;
    impl BidiProvider for DoubleBidi {
        fn levels(&self, paragraph: &str) -> Vec<u8> {
            paragraph
                .chars()
                .map(|c| u8::from(('\u{0370}'..='\u{03FF}').contains(&c)))
                .collect()
        }
    }

    /// Char boundaries, except combining marks join their base (a crude
    /// stand-in for UAX #29 — enough to exercise cluster alignment).
    struct DoubleGraphemes;
    impl GraphemeSegmenter for DoubleGraphemes {
        fn cluster_boundaries(&self, text: &str) -> Vec<usize> {
            let mut out = vec![0];
            for (i, c) in text.char_indices() {
                if i == 0 {
                    continue;
                }
                if !('\u{0300}'..='\u{036F}').contains(&c) {
                    out.push(i);
                }
            }
            out.push(text.len());
            out.dedup();
            out
        }
    }

    /// One face per style except Greek clusters fall to face 99 (a
    /// stand-in for the §44.2 chain switching faces mid-run).
    struct DoubleFonts;
    impl FontProbe for DoubleFonts {
        fn resolve(&self, style: StyleId, cluster: &str) -> FaceId {
            if cluster
                .chars()
                .any(|c| ('\u{0370}'..='\u{03FF}').contains(&c))
            {
                FaceId(99)
            } else {
                FaceId(style.0)
            }
        }
    }

    /// One glyph per cluster-leading char, advance 0.5 em — monotone
    /// clusters by construction.
    struct DoubleShaper;
    impl Shaper for DoubleShaper {
        fn shape(&self, item: &Item, text: &str) -> Vec<ShapedGlyph> {
            text[item.range.clone()]
                .char_indices()
                .map(|(i, c)| ShapedGlyph {
                    glyph: c as u32,
                    cluster: item.range.start + i,
                    advance: 0.5,
                    offset_x: 0.0,
                    offset_y: 0.0,
                    unsafe_to_break: false,
                })
                .collect()
        }
    }

    fn items_for(text: &str) -> Vec<Item> {
        itemize(
            &[RunSpan {
                text,
                style: StyleId(1),
            }],
            &DoubleScript,
            &DoubleBidi,
            &DoubleGraphemes,
            &DoubleFonts,
        )
    }

    #[test]
    fn single_script_single_style_is_one_item() {
        let items = items_for("hello world");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].range, 0..11);
        assert_eq!(items[0].script, *b"Latn");
        assert_eq!(items[0].bidi_level, 0);
    }

    #[test]
    fn items_are_the_intersection_of_the_four_partitions() {
        // Latin + Greek (script AND level AND face all switch): 2 items
        // around the boundary; Common-class space resolves to the
        // surrounding script (carry rule).
        let items = items_for("ab \u{03B1}\u{03B2}");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].script, *b"Latn");
        assert_eq!(items[0].range, 0..3); // space carried into Latin
        assert_eq!(items[1].script, *b"Grek");
        assert_eq!(items[1].bidi_level, 1);
        assert_eq!(items[1].face, FaceId(99));

        // Style boundary alone also splits (same script/level/face).
        let runs = [
            RunSpan {
                text: "ab",
                style: StyleId(1),
            },
            RunSpan {
                text: "cd",
                style: StyleId(2),
            },
        ];
        let items = itemize(
            &runs,
            &DoubleScript,
            &DoubleBidi,
            &DoubleGraphemes,
            &DoubleFonts,
        );
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].range, 0..2);
        assert_eq!(items[1].range, 2..4);
    }

    #[test]
    fn combining_mark_never_splits_from_its_base() {
        // "e" + COMBINING ACUTE forms one cluster; the mark is Inherited
        // so the cluster resolves to the base's script and stays whole.
        let items = items_for("e\u{0301}f");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].script, *b"Latn");
    }

    #[test]
    fn cluster_count_invariant_through_shaping() {
        // §44.2/§44.8 invariant: laid-out cluster count == source cluster
        // count, whatever faces resolved.
        let text = "ab \u{03B1}\u{03B2} e\u{0301}!";
        let source_clusters = DoubleGraphemes.cluster_boundaries(text).len() - 1;
        let items = items_for(text);
        let glyphs: Vec<ShapedGlyph> = items
            .iter()
            .flat_map(|it| DoubleShaper.shape(it, text))
            .collect();
        // Distinct cluster starts among glyphs, restricted to cluster
        // boundaries (level-1 monotone clusters).
        let bounds = DoubleGraphemes.cluster_boundaries(text);
        let mut starts: Vec<usize> = glyphs
            .iter()
            .map(|g| *bounds.iter().rfind(|&&b| b <= g.cluster).unwrap())
            .collect();
        starts.dedup();
        assert_eq!(starts.len(), source_clusters);
        // Monotonicity (cluster level 1).
        assert!(glyphs.windows(2).all(|w| w[0].cluster <= w[1].cluster));
    }

    #[test]
    fn visual_order_reverses_rtl_runs() {
        // L0 R1 R1 L0 → visual: 0, (2 1), 3.
        let mk = |lvl: u8, a: usize, b: usize| Item {
            range: a..b,
            style: StyleId(1),
            script: *b"Latn",
            bidi_level: lvl,
            face: FaceId(1),
        };
        let items = vec![mk(0, 0, 1), mk(1, 1, 2), mk(1, 2, 3), mk(0, 3, 4)];
        assert_eq!(visual_order(&items), vec![0, 2, 1, 3]);
        // All-LTR is identity.
        let ltr = vec![mk(0, 0, 1), mk(0, 1, 2)];
        assert_eq!(visual_order(&ltr), vec![0, 1]);
        // Nested levels: L1 L1 L2 L2 L1 — level 2 is an EVEN (LTR)
        // embedding inside the RTL run: the L2 pass reverses (2,3), the
        // L1 pass reverses everything, so the pair reads in LOGICAL order
        // within the reversed surroundings (UAX #9 L2 worked by hand:
        // 01234 → 01324 → 42310).
        let nest = vec![
            mk(1, 0, 1),
            mk(1, 1, 2),
            mk(2, 2, 3),
            mk(2, 3, 4),
            mk(1, 4, 5),
        ];
        assert_eq!(visual_order(&nest), vec![4, 2, 3, 1, 0]);
    }

    #[test]
    fn itemize_is_deterministic() {
        let text = "mixed \u{03B1}\u{03B2} text e\u{0301} 123";
        assert_eq!(items_for(text), items_for(text));
    }
}
