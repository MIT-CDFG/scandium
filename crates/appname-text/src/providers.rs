// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Real Unicode providers behind the [`crate::pipeline`] seams.
//!
//! Spec: ES §23.2 (pipeline stages 1 and 3 — itemization properties and
//! break boundaries); Part 44; DD ㉒. Dependency authorization:
//! deps/icu.yaml (ICU4X 2.2 `compiled_data` — Unicode data baked and
//! version-pinned into the binary, no runtime data files, no system ICU;
//! unicode-bidi 0.3.18 for UAX #9, since ICU4X ships no bidi engine —
//! the election is recorded in the deps record).
//!
//! Everything here is pure and deterministic: identical strings produce
//! identical classifications on every OS, because the data ships in the
//! binary and no locale, environment, or platform API is consulted.

use icu_properties::props::{JoiningType, Script};
use icu_properties::{CodePointMapData, CodePointMapDataBorrowed, PropertyNamesShortBorrowed};
use icu_segmenter::options::LineBreakOptions;
use icu_segmenter::{
    GraphemeClusterSegmenter, GraphemeClusterSegmenterBorrowed, LineSegmenter,
    LineSegmenterBorrowed,
};
use unicode_bidi::BidiInfo;

use crate::pipeline::{
    BidiProvider, GraphemeSegmenter, LineBreakProvider, ScriptClass, ScriptClassifier,
};

/// UAX #24 script classification via ICU4X compiled data.
///
/// Explicit scripts are reported by their ISO 15924 short tag (the same
/// 4-byte form `Item.script` carries and HarfBuzz consumes); `Common`/
/// `Inherited` are surfaced as classes so the itemizer applies the §23.2
/// resolution rule in exactly one place.
pub struct IcuScriptClassifier {
    map: CodePointMapDataBorrowed<'static, Script>,
    names: PropertyNamesShortBorrowed<'static, Script>,
}

impl IcuScriptClassifier {
    pub fn new() -> Self {
        IcuScriptClassifier {
            map: CodePointMapData::<Script>::new(),
            names: PropertyNamesShortBorrowed::<Script>::new(),
        }
    }
}

impl Default for IcuScriptClassifier {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptClassifier for IcuScriptClassifier {
    fn script(&self, c: char) -> ScriptClass {
        let s = self.map.get(c);
        if s == Script::Common {
            return ScriptClass::Common;
        }
        if s == Script::Inherited {
            return ScriptClass::Inherited;
        }
        match self.names.get(s) {
            Some(tag) if tag.len() == 4 && tag.is_ascii() => {
                let b = tag.as_bytes();
                ScriptClass::Explicit([b[0], b[1], b[2], b[3]])
            }
            // A script the property data can't name in ISO 15924 form
            // degrades to Common (resolves to the surrounding script) —
            // deterministic and safe; Unknown (Zzzz) lands here too.
            _ => ScriptClass::Common,
        }
    }
}

/// UAX #9 resolved embedding levels via unicode-bidi.
///
/// Base direction: first strong character of the paragraph (the §23.2
/// default; the `ParagraphStyle.direction` override arrives with the
/// CP-1.0 text model and will pass an explicit base level here).
pub struct UnicodeBidiProvider;

impl BidiProvider for UnicodeBidiProvider {
    fn levels(&self, paragraph: &str) -> Vec<u8> {
        let info = BidiInfo::new(paragraph, None);
        // unicode-bidi reports levels per BYTE; the seam contract is per
        // CHARACTER (the itemizer walks chars). Sample at char starts.
        paragraph
            .char_indices()
            .map(|(i, _)| info.levels[i].number())
            .collect()
    }
}

/// UAX #29 extended grapheme clusters via ICU4X compiled data.
pub struct IcuGraphemeSegmenter {
    seg: GraphemeClusterSegmenterBorrowed<'static>,
}

impl IcuGraphemeSegmenter {
    pub fn new() -> Self {
        IcuGraphemeSegmenter {
            seg: GraphemeClusterSegmenter::new(),
        }
    }
}

impl Default for IcuGraphemeSegmenter {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphemeSegmenter for IcuGraphemeSegmenter {
    fn cluster_boundaries(&self, text: &str) -> Vec<usize> {
        // ICU4X yields every boundary including 0 and text.len(); the
        // seam contract wants exactly that (first = 0, last = len).
        let mut out: Vec<usize> = self.seg.segment_str(text).collect();
        if out.first() != Some(&0) {
            out.insert(0, 0);
        }
        if out.last() != Some(&text.len()) {
            out.push(text.len());
        }
        out
    }
}

/// UAX #14 line-break opportunities via ICU4X compiled data.
///
/// §23.3 [NORMATIVE v1] layout is greedy first-fit over these
/// opportunities restricted to cluster boundaries; this provider reports
/// the raw UAX #14 positions (interior + end of text). The leading
/// 0 boundary ICU4X emits is dropped — "break before any text" is not an
/// opportunity the layout consumes.
pub struct IcuLineBreakProvider {
    seg: LineSegmenterBorrowed<'static>,
}

impl IcuLineBreakProvider {
    pub fn new() -> Self {
        IcuLineBreakProvider {
            // Auto = the UAX #14 rule set with the best available
            // dictionary/LSTM data for SE-Asian scripts, from compiled
            // data — deterministic (pinned data, no runtime selection).
            seg: LineSegmenter::new_auto(LineBreakOptions::default()),
        }
    }
}

impl Default for IcuLineBreakProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl LineBreakProvider for IcuLineBreakProvider {
    fn opportunities(&self, text: &str) -> Vec<usize> {
        self.seg.segment_str(text).filter(|&b| b != 0).collect()
    }
}

/// §23.4 No-break: "the marked range exposes no UAX #14 opportunities"
/// — a wrapper suppressing every opportunity STRICTLY INSIDE a marked
/// range. Boundary semantics (pinned, tested): an opportunity exactly
/// at `range.end` splits AFTER the range, leaving it whole, so it
/// survives; one at `range.start` splits before it — also whole — and
/// survives. Ranges are paragraph byte ranges from the style model
/// (§23.1 run marks; the CP-1.0 kind wiring feeds them through).
pub struct NoBreakRanges<'a, P: LineBreakProvider> {
    pub inner: &'a P,
    pub ranges: &'a [core::ops::Range<usize>],
}

/// §23.4 "suppressed inside cursive joins" — the REAL discriminator
/// (C-5 slice-2 review D1: HarfBuzz's UNSAFE_TO_BREAK is a superset,
/// set by kerning, mark attachment, and contextual substitution too;
/// kerned Latin must NOT lose tracking). A boundary is a CURSIVE JOIN
/// iff the adjacent characters actually join per Unicode Joining_Type
/// (UAX #44 / ArabicShaping): the earlier side joins leftward-onward
/// (D, C, or L) and the later side joins rightward-backward (D, C, or
/// R), with Transparent (marks) skipped on both sides. Data: the
/// recorded icu_properties compiled_data (no new dependency).
pub fn is_cursive_join_boundary(before: &str, after: &str) -> bool {
    let jt = CodePointMapData::<JoiningType>::new();
    let prev = before
        .chars()
        .rev()
        .map(|c| jt.get(c))
        .find(|&t| t != JoiningType::Transparent);
    let next = after
        .chars()
        .map(|c| jt.get(c))
        .find(|&t| t != JoiningType::Transparent);
    let joins_onward = matches!(
        prev,
        Some(JoiningType::DualJoining)
            | Some(JoiningType::JoinCausing)
            | Some(JoiningType::LeftJoining)
    );
    let joins_backward = matches!(
        next,
        Some(JoiningType::DualJoining)
            | Some(JoiningType::JoinCausing)
            | Some(JoiningType::RightJoining)
    );
    joins_onward && joins_backward
}

impl<P: LineBreakProvider> LineBreakProvider for NoBreakRanges<'_, P> {
    fn opportunities(&self, text: &str) -> Vec<usize> {
        self.inner
            .opportunities(text)
            .into_iter()
            .filter(|&p| !self.ranges.iter().any(|r| r.start < p && p < r.end))
            .collect()
    }
}
