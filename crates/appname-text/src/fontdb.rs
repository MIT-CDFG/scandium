// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! FontDb + the §44.2 fallback-chain resolution order (CP-1.0 text
//! row, slice C-2 groundwork).
//!
//! Spec: ES §23.5 (`FontDb`: "system + document-embedded faces,
//! indexed; fallback chains per script"; duplicate PostScript names
//! collapse at index time, **user > system > bundled**), §44.2
//! [NORMATIVE] resolution order — per grapheme cluster, first face
//! whose cmap covers EVERY code point of the cluster wins:
//!
//! 1. the run's resolved face,
//! 2. the document's embedded faces in asset-index order,
//! 3. the per-script user preference chain,
//! 4. the bundled Noto set (a FACE-REGISTRATION concern: the bundled
//!    faces arrive with their own deps records and register with
//!    [`FaceSource::Bundled`]; no font ships in this crate, §3.10),
//! 5. system enumeration, deterministically ordered,
//! 6. tofu — the procedural hex-box `.notdef` ([`TOFU_FACE`] sentinel;
//!    its §23.5 hex-box rasterization is the raster side's, C-3).
//!
//! Cluster integrity (§44.2 [NORMATIVE], property-tested): a fallback
//! boundary never splits a grapheme cluster — eligibility is
//! whole-cluster coverage, so a mark can never come from a different
//! face than its base. Default-ignorable code points (ZWJ/ZWNJ,
//! variation selectors, directional controls) are coverage-EXEMPT:
//! they render zero-width when uncovered and never cause tofu.
//!
//! Honest limits (recorded in the lane log):
//! * §44.2 step 5's ordering key (b) "script coverage ratio
//!   descending" needs per-script exemplar sets (OPEN — closes when
//!   exemplar data lands, expected with the bundled-face records
//!   wave); until then the hook returns a constant and keys
//!   (a)/(c)/(d) decide. Deterministic either way; divergence from
//!   the final ordering is possible whenever MULTIPLE faces of the
//!   same source class cover one cluster — bundled as well as system
//!   (review correction: an earlier wording said "system only").
//! * §23.5's step-4 interior selection rule — CJK bundled face
//!   SC/TC/JP/KR chosen **by run language, else UI locale** — is
//!   IMPLEMENTED as of slice C-3: [`ResolveCtx`] carries
//!   `run_language`/`ui_locale`, bundled faces declare their
//!   [`CjkVariant`], and matching variants rank first among bundled
//!   candidates FOR CJK-SCRIPT CLUSTERS ONLY (Hani/Hira/Kana/Hang/
//!   Bopo — a Latin cluster under a CJK locale resolves per the Latin
//!   table row; review round). Tested with tagged subset faces. The
//!   bundled FACE DATA still arrives with its own records wave.

use std::collections::BTreeMap;

use appname_shape_sys::ShapeFont;

use crate::pipeline::{FaceId, FontProbe, ScriptClass, ScriptClassifier, StyleId};

/// The §44.2 step-6 sentinel: nothing covers the cluster; render the
/// procedural hex-box `.notdef` (never silently invisible).
pub const TOFU_FACE: FaceId = FaceId(u32::MAX);

/// Where a face came from (§23.5 index-time dedup priority:
/// user > system > bundled; document-embedded faces are
/// document-scoped and never deduped against installed ones).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FaceSource {
    DocEmbedded,
    User,
    System,
    Bundled,
}

/// §23.5 CJK bundled-face variant (Noto Sans CJK SC/TC/JP/KR class).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CjkVariant {
    Sc,
    Tc,
    Jp,
    Kr,
}

impl CjkVariant {
    /// §23.5: variant from a BCP 47 tag ("selected by run `language`,
    /// else UI locale"). Deterministic SUBTAG matching (review round:
    /// bare prefixes overmatched — `jav` is Javanese, not Japanese):
    /// the primary language subtag must be exactly ja/ko/zh; for zh an
    /// explicit Hans/Hant script subtag is the strongest signal and
    /// beats any region (zh-Hans-HK → SC); regions TW/HK/MO imply TC
    /// only when no script subtag says otherwise; bare zh → SC.
    pub fn from_language(tag: &str) -> Option<CjkVariant> {
        let t = tag.to_ascii_lowercase();
        let mut parts = t.split('-');
        let lang = parts.next().unwrap_or("");
        let rest: Vec<&str> = parts.collect();
        match lang {
            "ja" => Some(CjkVariant::Jp),
            "ko" => Some(CjkVariant::Kr),
            "zh" => {
                if rest.contains(&"hans") {
                    Some(CjkVariant::Sc)
                } else if rest.contains(&"hant")
                    || rest.iter().any(|&s| s == "tw" || s == "hk" || s == "mo")
                {
                    Some(CjkVariant::Tc)
                } else {
                    Some(CjkVariant::Sc)
                }
            }
            _ => None,
        }
    }
}

/// The scripts the §23.5 CJK table row serves. The variant boost is
/// GATED on the cluster's script being one of these — a Latin cluster
/// under a Japanese UI locale must still resolve per the Latin table
/// row, never to a CJK face by locale (review round: the unggated
/// boost gave CJK-flavored Latin to every CJK-locale user).
fn is_cjk_script(tag: [u8; 4]) -> bool {
    matches!(&tag, b"Hani" | b"Hira" | b"Kana" | b"Hang" | b"Bopo")
}

/// §23.5 `FontRef`-shaped face metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FaceInfo {
    pub family: String,
    pub style: String,
    pub postscript_name: String,
    pub source: FaceSource,
    /// Set on bundled CJK faces only (the §23.5 SC/TC/JP/KR table);
    /// `None` everywhere else.
    pub cjk_variant: Option<CjkVariant>,
}

struct Entry {
    info: FaceInfo,
    /// cmap probe (coverage only; scale is irrelevant to coverage).
    probe: ShapeFont,
    /// Shadowed by a same-PostScript-name face of higher priority
    /// (§23.5: "a shadowed face can never win by enumeration order").
    shadowed: bool,
}

/// The face index + §44.2 resolver.
pub struct FontDb {
    faces: BTreeMap<u32, Entry>,
}

/// Resolution context: the run's face plus the document- and
/// preference-level chains (§44.2 steps 1–3). Step 4 (bundled) and
/// step 5 (system) come from the registered faces' sources.
#[derive(Clone, Debug, Default)]
pub struct ResolveCtx {
    /// Step 1: the run's resolved `FontRef` (§44.1 view-time mappings
    /// already applied by the caller).
    pub run_face: Option<FaceId>,
    /// Step 2: document-embedded faces in asset-index order (§7.5).
    pub embedded: Vec<FaceId>,
    /// Step 3: per-script user preference chains, keyed by the
    /// cluster's UAX #24 script tag.
    pub script_prefs: BTreeMap<[u8; 4], Vec<FaceId>>,
    /// §23.5 CJK selection input: the run's BCP 47 `language`
    /// (from the §23.4 style model; `None` until the run carries one).
    pub run_language: Option<String>,
    /// §23.5 CJK selection fallback: the UI locale.
    pub ui_locale: String,
}

/// Default-ignorable code points are coverage-exempt (§44.2: they
/// render zero-width when uncovered — never tofu). The set here covers
/// the joiners, variation selectors, and directional controls the rule
/// names; the full DI property arrives with the ICU property data if a
/// gap ever matters in practice.
fn is_default_ignorable(c: char) -> bool {
    matches!(c,
        '\u{200C}' | '\u{200D}' // ZWNJ, ZWJ
        | '\u{200B}' | '\u{2060}' | '\u{FEFF}' // ZWSP, WJ, BOM
        | '\u{200E}' | '\u{200F}' | '\u{061C}' // LRM, RLM, ALM
        | '\u{202A}'..='\u{202E}' // embedding/override controls
        | '\u{2066}'..='\u{2069}' // isolate controls
        | '\u{FE00}'..='\u{FE0F}' // variation selectors
        | '\u{E0100}'..='\u{E01EF}' // VS supplement
    )
}

impl FontDb {
    pub fn new() -> Self {
        FontDb {
            faces: BTreeMap::new(),
        }
    }

    /// Register a face's bytes under `id`. Returns `Err` when HarfBuzz
    /// rejects the bytes. Re-runs the §23.5 PostScript-name dedup
    /// (user > system > bundled) over the index.
    pub fn register(
        &mut self,
        id: FaceId,
        bytes: &[u8],
        info: FaceInfo,
    ) -> Result<(), appname_shape_sys::ShapeSysError> {
        let probe = ShapeFont::from_bytes(bytes, 0, 1000)?;
        self.faces.insert(
            id.0,
            Entry {
                info,
                probe,
                shadowed: false,
            },
        );
        self.redup();
        Ok(())
    }

    /// §23.5 index-time dedup: among installed faces (user/system/
    /// bundled) sharing a PostScript name, only the highest-priority
    /// one stays visible. Document-embedded faces never shadow or get
    /// shadowed (document-scoped).
    fn redup(&mut self) {
        let mut best: BTreeMap<String, (FaceSource, u32)> = BTreeMap::new();
        for (&id, e) in &self.faces {
            if e.info.source == FaceSource::DocEmbedded {
                continue;
            }
            let cand = (e.info.source, id);
            best.entry(e.info.postscript_name.clone())
                .and_modify(|cur| {
                    // FaceSource orders User < System < Bundled after
                    // DocEmbedded; smaller = higher priority. Ties (same
                    // source) keep the lower id — deterministic.
                    if cand < *cur {
                        *cur = cand;
                    }
                })
                .or_insert(cand);
        }
        for (&id, e) in self.faces.iter_mut() {
            if e.info.source == FaceSource::DocEmbedded {
                e.shadowed = false;
                continue;
            }
            e.shadowed = best
                .get(&e.info.postscript_name)
                .is_some_and(|&(_, keep)| keep != id);
        }
    }

    pub fn info(&self, id: FaceId) -> Option<&FaceInfo> {
        self.faces.get(&id.0).map(|e| &e.info)
    }

    /// Whole-cluster cmap coverage (§44.2 eligibility), with the
    /// default-ignorable exemption.
    pub fn covers(&self, id: FaceId, cluster: &str) -> bool {
        let Some(e) = self.faces.get(&id.0) else {
            return false;
        };
        cluster
            .chars()
            .all(|c| is_default_ignorable(c) || e.probe.glyph_index(c).is_some())
    }

    /// §44.2 step-5 ordering hook (b): script coverage ratio. Constant
    /// until per-script exemplar sets land with the bundled-face wave
    /// (module-header honest limit).
    fn coverage_ratio(&self, _id: FaceId, _script: [u8; 4]) -> f64 {
        0.0
    }

    /// The §44.2 resolution order for one cluster. Returns
    /// [`TOFU_FACE`] when nothing covers it (step 6).
    pub fn resolve(&self, ctx: &ResolveCtx, script: [u8; 4], cluster: &str) -> FaceId {
        // A cluster of ONLY default-ignorables sticks with the run face
        // (zero-width; no probe can fail it).
        // Step 1: the run's resolved face.
        if let Some(f) = ctx.run_face {
            if self.covers(f, cluster) {
                return f;
            }
        }
        // Step 2: document-embedded faces, asset-index order.
        for &f in &ctx.embedded {
            if self.faces.get(&f.0).map(|e| e.info.source) == Some(FaceSource::DocEmbedded)
                && self.covers(f, cluster)
            {
                return f;
            }
        }
        // Step 3: per-script user preference chain.
        if let Some(chain) = ctx.script_prefs.get(&script) {
            for &f in chain {
                if self.covers(f, cluster) {
                    return f;
                }
            }
        }
        // Steps 4+5: bundled set, then system enumeration — both live
        // in the registered index; §44.2's step-4/step-5 split is the
        // source tag. Ordering: covers → (bundled: §23.5 CJK-variant
        // match by run language else UI locale) → ratio desc → family
        // byte order → PostScript byte order; shadowed faces never win
        // (§23.5).
        let wanted_variant = if is_cjk_script(script) {
            ctx.run_language
                .as_deref()
                .and_then(CjkVariant::from_language)
                .or_else(|| CjkVariant::from_language(&ctx.ui_locale))
        } else {
            None // non-CJK clusters never take the variant boost
        };
        let mut candidates: Vec<(&Entry, u32)> = self
            .faces
            .iter()
            .filter(|(_, e)| !e.shadowed && e.info.source != FaceSource::DocEmbedded)
            .map(|(&id, e)| (e, id))
            .collect();
        candidates.sort_by(|(ea, ia), (eb, ib)| {
            let ba = ea.info.source == FaceSource::Bundled;
            let bb = eb.info.source == FaceSource::Bundled;
            // Step 4 (bundled) probes before step 5 (everything else);
            // within bundled, a face whose declared CJK variant matches
            // the wanted one ranks first (§23.5 "selected by run
            // language, else UI locale").
            let va = ba && ea.info.cjk_variant.is_some() && ea.info.cjk_variant == wanted_variant;
            let vb = bb && eb.info.cjk_variant.is_some() && eb.info.cjk_variant == wanted_variant;
            bb.cmp(&ba)
                .then(vb.cmp(&va))
                .then_with(|| {
                    self.coverage_ratio(FaceId(*ib), script)
                        .partial_cmp(&self.coverage_ratio(FaceId(*ia), script))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| ea.info.family.as_bytes().cmp(eb.info.family.as_bytes()))
                .then_with(|| {
                    ea.info
                        .postscript_name
                        .as_bytes()
                        .cmp(eb.info.postscript_name.as_bytes())
                })
        });
        for (e, id) in candidates {
            let _ = e;
            if self.covers(FaceId(id), cluster) {
                return FaceId(id);
            }
        }
        // Step 6.
        TOFU_FACE
    }
}

impl Default for FontDb {
    fn default() -> Self {
        Self::new()
    }
}

/// [`FontProbe`] adapter: plugs the §44.2 chain into the §23.2
/// itemizer (fallback resolves per grapheme cluster during
/// itemization; items split wherever the resolved face changes).
/// The cluster's script is classified here with the same rule the
/// itemizer applies (first explicit char, else Common-carry is the
/// itemizer's own job — Common clusters resolve under the carried
/// script only for step 3; steps 1/2/4/5 are script-blind).
pub struct DbProbe<'a, S: ScriptClassifier> {
    pub db: &'a FontDb,
    pub ctx: &'a ResolveCtx,
    pub scripts: &'a S,
}

impl<S: ScriptClassifier> FontProbe for DbProbe<'_, S> {
    fn resolve(&self, _style: StyleId, cluster: &str) -> FaceId {
        let mut script = *b"Zyyy";
        for c in cluster.chars() {
            if let ScriptClass::Explicit(tag) = self.scripts.script(c) {
                script = tag;
                break;
            }
        }
        self.db.resolve(self.ctx, script, cluster)
    }
}

#[cfg(test)]
mod tests {
    use super::CjkVariant::{Jp, Kr, Sc, Tc};
    use super::*;

    /// §23.5 language→variant matching (review round: subtag-exact,
    /// script-beats-region).
    #[test]
    fn cjk_variant_language_matching_is_subtag_exact() {
        assert_eq!(CjkVariant::from_language("ja"), Some(Jp));
        assert_eq!(CjkVariant::from_language("ja-JP"), Some(Jp));
        assert_eq!(
            CjkVariant::from_language("JA-jp"),
            Some(Jp),
            "case-insensitive"
        );
        assert_eq!(CjkVariant::from_language("ko"), Some(Kr));
        assert_eq!(CjkVariant::from_language("zh"), Some(Sc));
        assert_eq!(CjkVariant::from_language("zh-Hant"), Some(Tc));
        assert_eq!(CjkVariant::from_language("zh-TW"), Some(Tc));
        assert_eq!(CjkVariant::from_language("zh-HK"), Some(Tc));
        // Explicit script subtag BEATS region (real CLDR locales).
        assert_eq!(CjkVariant::from_language("zh-Hans-HK"), Some(Sc));
        assert_eq!(CjkVariant::from_language("zh-Hans-TW"), Some(Sc));
        assert_eq!(CjkVariant::from_language("zh-Hant-CN"), Some(Tc));
        // Bare-prefix overmatch is dead: these are NOT ja/ko/zh.
        assert_eq!(CjkVariant::from_language("jav"), None, "Javanese");
        assert_eq!(CjkVariant::from_language("kok"), None, "Konkani");
        assert_eq!(CjkVariant::from_language("zha"), None, "Zhuang");
        assert_eq!(CjkVariant::from_language("en-US"), None);
    }

    #[test]
    fn cjk_script_gate_is_exactly_the_table_row() {
        for tag in [b"Hani", b"Hira", b"Kana", b"Hang", b"Bopo"] {
            assert!(is_cjk_script(*tag));
        }
        for tag in [b"Latn", b"Cyrl", b"Grek", b"Hebr", b"Arab", b"Zyyy"] {
            assert!(!is_cjk_script(*tag));
        }
    }
}
