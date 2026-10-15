// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §6.2 (color-mode conversions — "The complete mode-conversion
//! matrix [NORMATIVE]", every ordered pair with path, lossiness, and
//! warning MessageIds; the every-row rules: alpha never color-managed,
//! masks/channels never converted, tile-parallel through one
//! `cmsFLAGS_NOCACHE` transform, COW undo), §5.5 ("The CMYK fallback
//! formula" — the last-resort textbook separation on ENCODED working
//! RGB, with its mandatory persistent warning and untagged output), §5.3
//! (assign ≠ convert), §10.3 `doc.mode_convert` row (`to`, `profile?`,
//! `intent`, `flatten`; class C, snapshot inverse), contract
//! `contracts/commands/doc.mode_convert.yaml`; DD ⑦㉗.
//!
//! The color-side `doc.mode_convert` machinery: the mode vocabulary, the
//! §6.2 planning matrix (which CMS chain, what is lossy, which warnings
//! surface BEFORE executing), and the §5.5 naive-separation fallback.
//! Core's kind arm (its fence; door filed in the lane log) resolves
//! profiles, walks tiles through [`crate::Cms::transform`] with the
//! planned endpoints, re-tags, and captures results in COW tile deltas —
//! the conversion is a value transform at apply time (D-48), never a
//! render-time reinterpretation.
//!
//! Alpha and coverage, restated from the §6.2 every-row rules because
//! they gate core's marshalling: the alpha channel transfers bit-exact
//! (`COPY_ALPHA`, `icc.rs`); the CMS sees straight (un-premultiplied)
//! color only; masks, the selection mask, saved alpha channels and spot
//! channels are coverage data and are NEVER converted by a mode change.

use crate::error::ColorError;
use crate::types::{ColorModel, Depth};

/// Warning/notice MessageIds the §6.2 matrix and §5.5 fallback surface.
/// Machine ids only (V39: code-span field names); the catalog rows are
/// O-I18N's fence — ids not yet in `message_ids.yaml` ride the standing
/// mint-and-log protocol (flagged in the lane log).
pub mod warn {
    /// Gamut compression into a CMYK separation (§6.2).
    pub const TO_CMYK_GAMUT: &str = "color.mode.to_cmyk_gamut";
    /// Chroma (and separation) discarded going to Gray (§6.2).
    pub const TO_GRAY_LOSSY: &str = "color.mode.to_gray_lossy";
    /// No CMYK profile configured for a to-CMYK conversion (§6.2 RGB→CMYK
    /// row: "if no profile", surfaced before the §5.5 fallback engages).
    pub const NO_CMYK_PROFILE: &str = "color.mode.no_cmyk_profile";
    /// Leaving CMYK folds ink/K information; re-separation won't match
    /// (§6.2).
    pub const CMYK_ROUNDTRIP_LOSSY: &str = "color.mode.cmyk_roundtrip_lossy";
    /// RGB→Lab at u8 storage depth (§6.2: precision caveat).
    pub const LAB_U8_PRECISION: &str = "color.mode.lab_u8_precision";
    /// Lab→RGB clips out-of-working-gamut colors per intent (§6.2).
    pub const LAB_CLIP: &str = "color.mode.lab_clip";
    /// §5.5 [NORMATIVE]: the mathematical separation executed — colors
    /// are not print-accurate; the export is written untagged. MUST be a
    /// persistent warning whenever the fallback path runs, forward or
    /// inverse.
    pub const CMYK_FALLBACK: &str = "color.convert.cmyk_fallback_warning";
}

/// Document color mode (the `doc.mode_convert` / `doc.new` `to`/`mode`
/// vocabulary; document buffers are always alpha-bearing, §7).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum DocMode {
    Rgba,
    GrayA,
    CmykA,
    LabA,
}

impl DocMode {
    /// Contract wire spelling (`doc.mode_convert.yaml` `paramsSchema.to`).
    pub const fn as_wire(self) -> &'static str {
        match self {
            DocMode::Rgba => "rgba",
            DocMode::GrayA => "gray_a",
            DocMode::CmykA => "cmyk_a",
            DocMode::LabA => "lab_a",
        }
    }

    /// Parse the wire spelling (unknown literals are the caller's
    /// `error.schema.out_of_range`).
    pub fn from_wire(s: &str) -> Option<DocMode> {
        Some(match s {
            "rgba" => DocMode::Rgba,
            "gray_a" => DocMode::GrayA,
            "cmyk_a" => DocMode::CmykA,
            "lab_a" => DocMode::LabA,
            _ => return None,
        })
    }

    /// The CMS-boundary layout of this mode's color buffers.
    pub const fn color_model(self) -> ColorModel {
        match self {
            DocMode::Rgba => ColorModel::Rgba,
            DocMode::GrayA => ColorModel::GrayA,
            DocMode::CmykA => ColorModel::CmykA,
            DocMode::LabA => ColorModel::LabA,
        }
    }
}

/// A planned mode conversion: the §6.2 matrix row for one ordered pair,
/// resolved against the live profile situation. The plan is what the UI
/// warns from ("the UI warns when gamut/channels are lost") and what
/// core's kind arm executes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ModeConvertPlan {
    pub from: DocMode,
    pub to: DocMode,
    /// §6.2: "Same-mode is the identity" (depth changes are §6.3's
    /// separate command). Nothing to execute; core records the command
    /// with zero pixel change.
    pub identity: bool,
    /// The row's "Lossy?" column.
    pub lossy: bool,
    /// Warning MessageIds to surface BEFORE executing (§6.2 column +
    /// conditional rows), in row order.
    pub warnings: Vec<&'static str>,
    /// True when a to/from-CMYK leg has no CMYK profile available and the
    /// §5.5 last-resort separation must run ([`cmyk_fallback_separate`] /
    /// [`cmyk_fallback_compose`], through encoded working RGB). Never
    /// true when any valid CMYK profile is installed (§5.5: "never
    /// silently chosen").
    pub uses_cmyk_fallback: bool,
}

/// Build the §6.2 plan for `from → to` at storage `depth`.
/// `cmyk_profile_available` = does the document/preferences resolve ANY
/// valid CMYK profile (§30.1 `[color] cmyk_profile`, or the document's
/// own tag for CMYK sources)?
pub fn plan_mode_convert(
    from: DocMode,
    to: DocMode,
    depth: Depth,
    cmyk_profile_available: bool,
) -> ModeConvertPlan {
    use DocMode::{CmykA, GrayA, LabA, Rgba};
    let mut warnings: Vec<&'static str> = Vec::new();
    let mut lossy = false;
    let mut uses_cmyk_fallback = false;
    let identity = from == to;
    if !identity {
        match (from, to) {
            // Gray → RGB: luminance onto the neutral axis. Lossless
            // (appearance), no warning.
            (GrayA, Rgba) => {}
            // Gray → CMYK: separation (not a K-only plate, §14.3 note).
            (GrayA, CmykA) => {
                lossy = true;
                warnings.push(warn::TO_CMYK_GAMUT);
            }
            // Gray → Lab: no within precision.
            (GrayA, LabA) => {}
            // RGB → Gray: chroma discarded.
            (Rgba, GrayA) => {
                lossy = true;
                warnings.push(warn::TO_GRAY_LOSSY);
            }
            // RGB → CMYK: gamut compression, document intent + BPC.
            (Rgba, CmykA) => {
                lossy = true;
                warnings.push(warn::TO_CMYK_GAMUT);
            }
            // RGB → Lab: no within precision; u8 caveat.
            (Rgba, LabA) => {
                if depth == Depth::U8 {
                    warnings.push(warn::LAB_U8_PRECISION);
                }
            }
            // CMYK → Gray: chroma + separation lost.
            (CmykA, GrayA) => {
                lossy = true;
                warnings.push(warn::TO_GRAY_LOSSY);
            }
            // CMYK → RGB / CMYK → Lab: K/ink info folded.
            (CmykA, Rgba) | (CmykA, LabA) => {
                lossy = true;
                warnings.push(warn::CMYK_ROUNDTRIP_LOSSY);
            }
            // Lab → Gray: chroma discarded.
            (LabA, GrayA) => {
                lossy = true;
                warnings.push(warn::TO_GRAY_LOSSY);
            }
            // Lab → RGB: gamut clip per intent.
            (LabA, Rgba) => {
                lossy = true;
                warnings.push(warn::LAB_CLIP);
            }
            // Lab → CMYK: gamut compression.
            (LabA, CmykA) => {
                lossy = true;
                warnings.push(warn::TO_CMYK_GAMUT);
            }
            _ => unreachable!("same-mode pairs are the identity arm"),
        }
        // A CMYK endpoint without any CMYK profile: the §6.2 RGB→CMYK row's
        // conditional ("if no profile: color.mode.no_cmyk_profile, then
        // §5.5 fallback + color.convert.cmyk_fallback_warning") — applied
        // to every CMYK-touching pair, both directions (§5.5 gives the
        // naive inverse the same warning).
        if (to == CmykA || from == CmykA) && !cmyk_profile_available {
            warnings.push(warn::NO_CMYK_PROFILE);
            warnings.push(warn::CMYK_FALLBACK);
            uses_cmyk_fallback = true;
        }
    }
    ModeConvertPlan {
        from,
        to,
        identity,
        lossy,
        warnings,
        uses_cmyk_fallback,
    }
}

// ---------------------------------------------------------------------
// §5.5 "The CMYK fallback formula" — pure public math, last resort only.
// ---------------------------------------------------------------------

/// §5.5 forward fallback, one pixel: encoded working RGB → CMYK.
/// `K = 1 − max(R′,G′,B′)`; `C,M,Y = (1 − X′ − K) / (1 − K)` guarding
/// `K = 1`. Fixed-order f32 math.
#[inline]
pub fn cmyk_fallback_separate_px(rgb_encoded: [f32; 3]) -> [f32; 4] {
    let [r, g, b] = rgb_encoded;
    let k = 1.0 - r.max(g).max(b);
    if k < 1.0 {
        let d = 1.0 - k;
        [(1.0 - r - k) / d, (1.0 - g - k) / d, (1.0 - b - k) / d, k]
    } else {
        [0.0, 0.0, 0.0, 1.0]
    }
}

/// §5.5 naive inverse, one pixel: CMYK → encoded working RGB
/// (`R′ = (1−C)(1−K)` etc.). Same-warning territory as the forward
/// direction.
#[inline]
pub fn cmyk_fallback_compose_px(cmyk: [f32; 4]) -> [f32; 3] {
    let [c, m, y, k] = cmyk;
    let d = 1.0 - k;
    [(1.0 - c) * d, (1.0 - m) * d, (1.0 - y) * d]
}

/// Plane-level fallback separation: interleaved encoded-working `RgbA`
/// f32 → interleaved `CmykA` f32; alpha transfers bit-exact (§6.2 rule).
/// Whenever this executes, [`warn::CMYK_FALLBACK`] MUST already be on
/// the surfaced plan and the export written untagged (§5.5 [NORMATIVE]
/// — enforced at core's arm; restated here because it is normative).
pub fn cmyk_fallback_separate(rgba: &[f32], cmyka_out: &mut [f32]) -> Result<(), ColorError> {
    if !rgba.len().is_multiple_of(4) || rgba.len() / 4 != cmyka_out.len() / 5 {
        return Err(ColorError::ParamOutOfRange(
            "fallback separation buffer lengths",
        ));
    }
    for (s, d) in rgba.chunks_exact(4).zip(cmyka_out.chunks_exact_mut(5)) {
        let cmyk = cmyk_fallback_separate_px([s[0], s[1], s[2]]);
        d[..4].copy_from_slice(&cmyk);
        d[4] = s[3]; // alpha bit-exact
    }
    Ok(())
}

/// Plane-level naive inverse: interleaved `CmykA` f32 → encoded-working
/// `RgbA` f32; alpha bit-exact. Same warning obligation as the forward
/// direction (§5.5).
pub fn cmyk_fallback_compose(cmyka: &[f32], rgba_out: &mut [f32]) -> Result<(), ColorError> {
    if !cmyka.len().is_multiple_of(5) || cmyka.len() / 5 != rgba_out.len() / 4 {
        return Err(ColorError::ParamOutOfRange(
            "fallback composition buffer lengths",
        ));
    }
    for (s, d) in cmyka.chunks_exact(5).zip(rgba_out.chunks_exact_mut(4)) {
        let rgb = cmyk_fallback_compose_px([s[0], s[1], s[2], s[3]]);
        d[..3].copy_from_slice(&rgb);
        d[3] = s[4]; // alpha bit-exact
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [DocMode; 4] = [DocMode::Rgba, DocMode::GrayA, DocMode::CmykA, DocMode::LabA];

    #[test]
    fn wire_spellings_round_trip() {
        for m in ALL {
            assert_eq!(DocMode::from_wire(m.as_wire()), Some(m));
        }
        // The contract's own invalid literal must not parse
        // (doc.mode_convert.yaml invalid case: to:'rgb').
        assert_eq!(DocMode::from_wire("rgb"), None);
    }

    /// The §6.2 matrix, row by row: lossiness + warning ids exactly as
    /// tabulated (CMYK profile present, depth f32 — the unconditional
    /// cells).
    #[test]
    fn matrix_rows_match_the_spec_table() {
        use warn::*;
        let row = |f, t| plan_mode_convert(f, t, Depth::F32, true);
        let cases: [(DocMode, DocMode, bool, &[&str]); 12] = [
            (DocMode::GrayA, DocMode::Rgba, false, &[]),
            (DocMode::GrayA, DocMode::CmykA, true, &[TO_CMYK_GAMUT]),
            (DocMode::GrayA, DocMode::LabA, false, &[]),
            (DocMode::Rgba, DocMode::GrayA, true, &[TO_GRAY_LOSSY]),
            (DocMode::Rgba, DocMode::CmykA, true, &[TO_CMYK_GAMUT]),
            (DocMode::Rgba, DocMode::LabA, false, &[]),
            (DocMode::CmykA, DocMode::GrayA, true, &[TO_GRAY_LOSSY]),
            (DocMode::CmykA, DocMode::Rgba, true, &[CMYK_ROUNDTRIP_LOSSY]),
            (DocMode::CmykA, DocMode::LabA, true, &[CMYK_ROUNDTRIP_LOSSY]),
            (DocMode::LabA, DocMode::GrayA, true, &[TO_GRAY_LOSSY]),
            (DocMode::LabA, DocMode::Rgba, true, &[LAB_CLIP]),
            (DocMode::LabA, DocMode::CmykA, true, &[TO_CMYK_GAMUT]),
        ];
        for (f, t, lossy, warns) in cases {
            let p = row(f, t);
            assert!(!p.identity);
            assert_eq!(p.lossy, lossy, "{f:?}->{t:?}");
            assert_eq!(p.warnings, warns, "{f:?}->{t:?}");
            assert!(!p.uses_cmyk_fallback);
        }
    }

    /// Same-mode is the identity (§6.2): no warnings, nothing lossy.
    #[test]
    fn same_mode_is_identity() {
        for m in ALL {
            let p = plan_mode_convert(m, m, Depth::U8, false);
            assert!(p.identity && !p.lossy && p.warnings.is_empty() && !p.uses_cmyk_fallback);
        }
    }

    /// Conditional cells: the u8 Lab caveat; the no-CMYK-profile chain
    /// (`no_cmyk_profile` then the §5.5 fallback warning) on every
    /// CMYK-touching pair, both directions.
    #[test]
    fn conditional_warnings() {
        let p = plan_mode_convert(DocMode::Rgba, DocMode::LabA, Depth::U8, true);
        assert_eq!(p.warnings, [warn::LAB_U8_PRECISION]);
        let p = plan_mode_convert(DocMode::Rgba, DocMode::LabA, Depth::U16, true);
        assert!(p.warnings.is_empty());

        let p = plan_mode_convert(DocMode::Rgba, DocMode::CmykA, Depth::F32, false);
        assert_eq!(
            p.warnings,
            [
                warn::TO_CMYK_GAMUT,
                warn::NO_CMYK_PROFILE,
                warn::CMYK_FALLBACK
            ]
        );
        assert!(p.uses_cmyk_fallback);
        let p = plan_mode_convert(DocMode::CmykA, DocMode::Rgba, Depth::F32, false);
        assert!(p.uses_cmyk_fallback);
        assert!(p.warnings.contains(&warn::CMYK_FALLBACK));
        // With a profile installed the fallback is never chosen (§5.5).
        let p = plan_mode_convert(DocMode::Rgba, DocMode::CmykA, Depth::F32, true);
        assert!(!p.uses_cmyk_fallback);
    }

    /// §5.5 formula oracles: white → 0/0/0/0; black → K=1; pure encoded
    /// red → C=0, M=1, Y=1, K=0; 50% gray → K=0.5, zero ink.
    #[test]
    fn fallback_formula_hand_oracles() {
        assert_eq!(cmyk_fallback_separate_px([1.0, 1.0, 1.0]), [0.0; 4]);
        assert_eq!(
            cmyk_fallback_separate_px([0.0, 0.0, 0.0]),
            [0.0, 0.0, 0.0, 1.0]
        );
        assert_eq!(
            cmyk_fallback_separate_px([1.0, 0.0, 0.0]),
            [0.0, 1.0, 1.0, 0.0]
        );
        let g = cmyk_fallback_separate_px([0.5, 0.5, 0.5]);
        assert_eq!(g[3], 0.5);
        assert_eq!(&g[..3], &[0.0, 0.0, 0.0]);
        // Naive inverse of the primaries lands back exactly.
        for px in [[1.0f32, 1.0, 1.0], [1.0, 0.0, 0.0], [0.25, 0.5, 0.75]] {
            let back = cmyk_fallback_compose_px(cmyk_fallback_separate_px(px));
            for (a, b) in px.iter().zip(back.iter()) {
                assert!((a - b).abs() < 1e-6, "{px:?}: {back:?}");
            }
        }
    }

    /// Plane forms: alpha transfers bit-exact (§6.2 rule), lengths gate.
    #[test]
    fn plane_fallback_preserves_alpha_bits() {
        let rgba = [0.2f32, 0.4, 0.6, 0.123_456_7, 1.0, 1.0, 1.0, 0.0];
        let mut cmyka = [0f32; 10];
        cmyk_fallback_separate(&rgba, &mut cmyka).unwrap();
        assert_eq!(cmyka[4].to_bits(), rgba[3].to_bits());
        assert_eq!(cmyka[9].to_bits(), rgba[7].to_bits());
        let mut back = [0f32; 8];
        cmyk_fallback_compose(&cmyka, &mut back).unwrap();
        assert_eq!(back[3].to_bits(), rgba[3].to_bits());
        for (a, b) in rgba.iter().zip(back.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
        let mut short = [0f32; 5];
        assert!(cmyk_fallback_separate(&rgba, &mut short).is_err());
        assert!(cmyk_fallback_compose(&[0.0; 7], &mut back).is_err());
    }
}
