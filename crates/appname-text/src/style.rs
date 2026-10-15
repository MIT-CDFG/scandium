// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! §23.4 character/paragraph style model (CP-1.0 slice C-4).
//!
//! Spec: ES §23.4 [tables] — every field with its EXACT range and
//! default; §23.1 (styles live on runs; paragraph fields are layout
//! inputs); §7.7 (reserved fields exist from day one so documents
//! round-trip). Serialization into `appname.text.v1` rides the text
//! sidecar work (C-5); this module is the typed model + validation +
//! the §23.4 normative constants.
//!
//! INTEGRATED into the pipeline this slice: `language` (→ HarfBuzz +
//! the §23.5 CJK selection), `features` (overlay on the §23.2 engine
//! defaults), `tracking` (inter-cluster gaps — the gap model makes the
//! §23.4 "suppressed on line-final clusters" rule hold by construction;
//! cursive-join suppression needs shaper join info, C-5), `no_break`
//! (→ [`crate::providers::NoBreakRanges`]), and the metrics-driven
//! decoration geometry below (sub/super, underline, strikethrough —
//! font metrics when present, [NORMATIVE fallback] constants
//! otherwise). NOT yet consumed (C-5, raster/render binding): fill,
//! caps mapping, h/v scale, faux bold dilation (needs outline ops),
//! faux italic shear at raster positions, kerning Optical/Manual arms.
//!
//! Pinned elections (fold-in register; C-4 review round):
//! * ZERO-valued table thickness/size metrics are treated as ABSENT
//!   (the spec says "absent metrics"; zero ≠ absent is an
//!   interpretation — a face declaring zero thickness gets the
//!   fallback, never an invisible line).
//! * `post.underlinePosition` is read as the line's CENTER (FreeType's
//!   documented stem-center reading; the OT post table nominally says
//!   top — center is the mainstream reading, and it moves the drawn
//!   rect by thickness/2, hence pinned).
//! * Baseline-shift's model range is ± current size (§23.4 gives it as
//!   the "UI range"). CONSEQUENCE for the envelope layer (§5.4
//!   text.set_style / TX-03 import): a size change that strands an
//!   existing |shift| beyond the new size must CLAMP, not error —
//!   validate() here is the writer-side check, not the ingest rule.
//! * [`StyleRangeError`] pointers use the Rust model field names; the
//!   envelope layer translates to wire names (`size`,
//!   `baselineShift`, …) at the boundary per the §5.4 convention.

use crate::pipeline::FaceId;
use crate::raster::FreeTypeRasterizer;

/// §23.4 kerning modes (Metric = the font's GPOS/kern via shaping —
/// already what the shaper does; Optical/Manual are C-5 arms).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kerning {
    Metric,
    Optical,
    /// Per-pair manual value at the caret, −1000…1000 ‰ em.
    Manual(f64),
}

/// §23.4 caps field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Caps {
    None,
    All,
    Small,
}

/// §23.4 position field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Position {
    Normal,
    Super,
    Sub,
}

/// §23.4 character fields (the panel table, typed). Fill is DEFERRED —
/// no field yet; it arrives with §22's FillContent (O-TOOLS surface)
/// at the raster binding (C-5).
#[derive(Clone, Debug, PartialEq)]
pub struct CharStyle {
    /// 0.1–1296 pt (12).
    pub size_pt: f64,
    /// `None` = auto leading (§23.3); `Some` 0.1–5000 pt.
    pub leading_pt: Option<f64>,
    pub kerning: Kerning,
    /// −1000…10000 ‰ em (0); applied to inter-cluster gaps.
    pub tracking_permille: f64,
    /// 1–1000 % (100).
    pub h_scale_pct: f64,
    /// 1–1000 % (100).
    pub v_scale_pct: f64,
    /// pt; UI range ± current size (0).
    pub baseline_shift_pt: f64,
    pub faux_bold: bool,
    pub faux_italic: bool,
    pub caps: Caps,
    pub position: Position,
    pub underline: bool,
    pub strikethrough: bool,
    /// §23.4 No-break: the marked range exposes no UAX #14 opportunities.
    pub no_break: bool,
    /// BCP 47 (drives case mapping, `locl`, and §23.5 CJK selection).
    pub language: Option<String>,
    /// OpenType feature overlay (§23.4 popover), applied AFTER the
    /// §23.2 engine defaults.
    pub features: Vec<([u8; 4], u32)>,
    /// §44.3 variable-font axis coords (user-space values).
    pub variations: Vec<([u8; 4], f32)>,
}

impl Default for CharStyle {
    fn default() -> Self {
        CharStyle {
            size_pt: 12.0,
            leading_pt: None,
            kerning: Kerning::Metric,
            tracking_permille: 0.0,
            h_scale_pct: 100.0,
            v_scale_pct: 100.0,
            baseline_shift_pt: 0.0,
            faux_bold: false,
            faux_italic: false,
            caps: Caps::None,
            position: Position::Normal,
            underline: false,
            strikethrough: false,
            no_break: false,
            language: None,
            features: Vec::new(),
            variations: Vec::new(),
        }
    }
}

/// §23.4 range-validation error (RFC-6901-style field pointer, matching
/// the envelope error convention).
#[derive(Debug, Clone, PartialEq)]
pub struct StyleRangeError {
    pub field: &'static str,
    pub value: f64,
}

impl CharStyle {
    /// Validate every numeric field against its §23.4 table range.
    pub fn validate(&self) -> Result<(), StyleRangeError> {
        let checks: [(&'static str, f64, f64, f64); 5] = [
            ("/size_pt", self.size_pt, 0.1, 1296.0),
            (
                "/tracking_permille",
                self.tracking_permille,
                -1000.0,
                10000.0,
            ),
            ("/h_scale_pct", self.h_scale_pct, 1.0, 1000.0),
            ("/v_scale_pct", self.v_scale_pct, 1.0, 1000.0),
            // Baseline shift: ± current size (§23.4 "UI range" — pinned
            // as the model range too; fold-in note).
            (
                "/baseline_shift_pt",
                self.baseline_shift_pt,
                -self.size_pt,
                self.size_pt,
            ),
        ];
        for (field, v, lo, hi) in checks {
            if !v.is_finite() || v < lo || v > hi {
                return Err(StyleRangeError { field, value: v });
            }
        }
        if let Some(l) = self.leading_pt {
            if !l.is_finite() || !(0.1..=5000.0).contains(&l) {
                return Err(StyleRangeError {
                    field: "/leading_pt",
                    value: l,
                });
            }
        }
        if let Kerning::Manual(k) = self.kerning {
            if !k.is_finite() || !(-1000.0..=1000.0).contains(&k) {
                return Err(StyleRangeError {
                    field: "/kerning",
                    value: k,
                });
            }
        }
        Ok(())
    }

    /// em in doc px per §19.1: `size · PPI / 72`.
    pub fn em_px(&self, ppi: f64) -> f64 {
        self.size_pt * ppi / 72.0
    }

    /// Tracking in px at an em size (‰ of em, §23.4).
    pub fn tracking_px(&self, em_px: f64) -> f64 {
        self.tracking_permille / 1000.0 * em_px
    }
}

/// §23.4 reserved paragraph fields — parsed, preserved, INERT in v1
/// (§7.7 round-trip; §23.3 hyphenation reserved note).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReservedParagraphFields {
    pub hyphenation: bool,
    pub hanging_punctuation: bool,
    /// Opaque §23.4 "CJK options" payload (kinsoku etc. land with
    /// vertical text; carried verbatim).
    pub cjk_options: Option<String>,
    /// Drop caps lines 2–10 (off = None).
    pub drop_caps_lines: Option<u8>,
}

// ---------------------------------------------------------------------
// §23.4 normative decoration constants + metrics-driven geometry
// ---------------------------------------------------------------------

/// Faux italic [NORMATIVE]: shear about the baseline, x′ = x + y·tan 12°.
pub const FAUX_ITALIC_SHEAR: f64 = 0.212_556_561_670_022_13; // tan(12°)

/// Faux bold [NORMATIVE]: outline dilation of +2 % em (the dilation
/// itself needs outline offsetting — C-5; the constant is the contract).
pub const FAUX_BOLD_DILATE_EM: f64 = 0.02;

/// Sub/superscript [NORMATIVE fallback] when the face has no OS/2
/// metrics: 58 % size, baseline shift ±33 % em.
pub const POSITION_FALLBACK_SCALE: f64 = 0.58;
pub const POSITION_FALLBACK_SHIFT_EM: f64 = 0.33;

/// Underline/strikethrough absent-metrics fallback (§23.4): thickness
/// 5 % em at −10 % (underline, below baseline) / +30 % (strikeout,
/// above) em.
pub const DECOR_FALLBACK_THICKNESS_EM: f64 = 0.05;
pub const UNDERLINE_FALLBACK_POS_EM: f64 = -0.10;
pub const STRIKEOUT_FALLBACK_POS_EM: f64 = 0.30;

/// One decoration line: `position` is the SIGNED offset of the line's
/// center from the baseline in px, y-up positive (§23.2 bearing
/// convention: positive = above baseline); `thickness` in px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DecorLine {
    pub position: f64,
    pub thickness: f64,
}

/// Sub/super metrics: scale factor and SIGNED baseline shift (px,
/// y-up positive).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PositionMetrics {
    pub scale: f64,
    pub shift: f64,
}

impl FreeTypeRasterizer {
    /// §23.4 underline geometry: `post.underlinePosition/Thickness`
    /// when present and non-zero, else the normative fallback.
    pub fn underline_line(&mut self, face: FaceId, em_px: f64) -> DecorLine {
        let upm = self.units_per_em(face);
        match self.post_underline(face) {
            Some((pos, thick)) if thick != 0 => DecorLine {
                position: f64::from(pos) * em_px / upm,
                thickness: f64::from(thick) * em_px / upm,
            },
            _ => DecorLine {
                position: UNDERLINE_FALLBACK_POS_EM * em_px,
                thickness: DECOR_FALLBACK_THICKNESS_EM * em_px,
            },
        }
    }

    /// §23.4 strikethrough geometry: OS/2 `yStrikeoutPosition/Size`
    /// when present and non-zero, else the normative fallback.
    pub fn strikeout_line(&mut self, face: FaceId, em_px: f64) -> DecorLine {
        let upm = self.units_per_em(face);
        match self.os2_strikeout(face) {
            Some((pos, size)) if size != 0 => DecorLine {
                position: f64::from(pos) * em_px / upm,
                thickness: f64::from(size) * em_px / upm,
            },
            _ => DecorLine {
                position: STRIKEOUT_FALLBACK_POS_EM * em_px,
                thickness: DECOR_FALLBACK_THICKNESS_EM * em_px,
            },
        }
    }

    /// §23.4 sub/superscript metrics: the face's OS/2 size+offset
    /// pairs when present and non-zero, else the [NORMATIVE fallback]
    /// (58 % size, ±33 % em). `Position::Normal` is identity.
    pub fn position_metrics(&mut self, face: FaceId, em_px: f64, pos: Position) -> PositionMetrics {
        let upm = self.units_per_em(face);
        let fallback = |sign: f64| PositionMetrics {
            scale: POSITION_FALLBACK_SCALE,
            shift: sign * POSITION_FALLBACK_SHIFT_EM * em_px,
        };
        match pos {
            Position::Normal => PositionMetrics {
                scale: 1.0,
                shift: 0.0,
            },
            Position::Super => match self.os2_superscript(face) {
                Some((size, off)) if size != 0 => PositionMetrics {
                    scale: f64::from(size) / upm,
                    // OS/2 offsets are positive-DOWN for subscript and
                    // positive-UP for superscript per the OT spec; both
                    // normalize to signed y-up here.
                    shift: f64::from(off) * em_px / upm,
                },
                _ => fallback(1.0),
            },
            Position::Sub => match self.os2_subscript(face) {
                Some((size, off)) if size != 0 => PositionMetrics {
                    scale: f64::from(size) / upm,
                    shift: -f64::from(off) * em_px / upm,
                },
                _ => fallback(-1.0),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_table_and_validate() {
        let s = CharStyle::default();
        assert_eq!(s.size_pt, 12.0);
        assert_eq!(s.kerning, Kerning::Metric);
        assert_eq!(s.h_scale_pct, 100.0);
        assert!(s.leading_pt.is_none(), "auto leading default");
        assert!(s.validate().is_ok());
    }

    #[test]
    fn ranges_are_the_table_ranges() {
        let mut s = CharStyle {
            size_pt: 0.05, // below 0.1
            ..Default::default()
        };
        assert_eq!(s.validate().unwrap_err().field, "/size_pt");
        s.size_pt = 1296.0;
        assert!(s.validate().is_ok());
        s.tracking_permille = 10_001.0;
        assert_eq!(s.validate().unwrap_err().field, "/tracking_permille");
        s.tracking_permille = -1000.0;
        assert!(s.validate().is_ok());
        s.baseline_shift_pt = 2000.0; // beyond ± size
        assert_eq!(s.validate().unwrap_err().field, "/baseline_shift_pt");
        s.baseline_shift_pt = 0.0;
        s.kerning = Kerning::Manual(1000.5);
        assert_eq!(s.validate().unwrap_err().field, "/kerning");
        s.kerning = Kerning::Manual(-1000.0);
        assert!(s.validate().is_ok());
        s.leading_pt = Some(5000.5);
        assert_eq!(s.validate().unwrap_err().field, "/leading_pt");
    }

    #[test]
    fn em_and_tracking_conversions_are_exact() {
        let s = CharStyle {
            size_pt: 12.0,
            tracking_permille: 100.0,
            ..Default::default()
        };
        assert_eq!(s.em_px(72.0), 12.0);
        assert_eq!(s.em_px(144.0), 24.0);
        assert_eq!(s.tracking_px(16.0), 1.6);
    }

    #[test]
    fn normative_constants_are_the_spec_values() {
        assert!((FAUX_ITALIC_SHEAR - (12.0f64.to_radians()).tan()).abs() < 1e-15);
        assert_eq!(FAUX_BOLD_DILATE_EM, 0.02);
        assert_eq!(POSITION_FALLBACK_SCALE, 0.58);
        assert_eq!(POSITION_FALLBACK_SHIFT_EM, 0.33);
        assert_eq!(DECOR_FALLBACK_THICKNESS_EM, 0.05);
    }
}
