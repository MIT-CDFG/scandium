// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! `RawDevelopParams` — the §8.3 parameter struct under the Part 43 §43.3
//! [NORMATIVE] contract: camelCase JSON keys under `raw.develop.params`,
//! the five v1 rows, the five Post-1.0 rows RESERVED (v1 writes them as
//! `null`, validates them if present, rejects non-null with a
//! "reserved until Post-1.0" detail), and staged validation BEFORE any
//! pixel work — out-of-range REJECTS rather than clamps (§43.3: scripted
//! pipelines must not drift silently), failures are
//! `CodecError::OptionOutOfRange` naming the field.
//!
//! Compiled UNCONDITIONALLY (not behind feature `raw`): `DecodeOpts::raw`
//! is §8.1 contract surface, and a v1 reader validates `raw.develop`
//! blocks in documents whether or not the decoder module is present
//! (§43.3 reserved-key rule + §7.7 unknown-field rules).

use appname_core::ProfileId;
use serde_json::{Map, Value};

use crate::error::CodecError;

/// §8.3 `WhiteBalance` (stage 3).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WhiteBalance {
    /// Camera multipliers from the file (default).
    AsShot,
    /// Gray-world over non-clipped pixels, center-weighted (§8.3).
    Auto,
    /// CCT + tint through the standard CCT→xy relation (§8.3 stage 3).
    Custom {
        /// Kelvin, `2000..=50000` (§43.3).
        temp_k: f32,
        /// `-150..=150` (§43.3).
        tint: f32,
    },
}

/// §8.3 `Demosaic` (stage 4); default `Ahd` (§43.3 selection table).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Demosaic {
    /// Bilinear CFA interpolation — preview/proxy path, never a silent
    /// substitute at commit (§43.3).
    Linear,
    /// Variable number of gradients.
    Vng,
    /// Patterned pixel grouping.
    Ppg,
    /// Adaptive homogeneity-directed — THE default at commit.
    Ahd,
}

impl Demosaic {
    /// The §43.3 JSON enum string.
    pub fn as_str(self) -> &'static str {
        match self {
            Demosaic::Linear => "linear",
            Demosaic::Vng => "vng",
            Demosaic::Ppg => "ppg",
            Demosaic::Ahd => "ahd",
        }
    }

    fn from_str(s: &str) -> Option<Demosaic> {
        match s {
            "linear" => Some(Demosaic::Linear),
            "vng" => Some(Demosaic::Vng),
            "ppg" => Some(Demosaic::Ppg),
            "ahd" => Some(Demosaic::Ahd),
            _ => None,
        }
    }
}

/// §8.3 `RawDevelopParams` — the five v1 rows. The five Post-1.0 rows
/// (contrast, shadows, noiseReduction, sharpening, lensCorrection) are
/// deliberately NOT fields: v1 code cannot hold a non-neutral value for a
/// reserved parameter (§43.3 "explicitly not v1 features"); the JSON layer
/// writes them as `null` and rejects anything else.
#[derive(Debug, Clone, PartialEq)]
pub struct RawDevelopParams {
    pub white_balance: WhiteBalance,
    pub demosaic: Demosaic,
    /// EV, `-10..=10` (§43.3; dialog slider ±5, field the full range).
    pub exposure_ev: f32,
    /// `0..=1` (§8.3 semantics: 0 = clip, 1 = full reconstruct).
    pub highlight_recovery: f32,
    /// Profile override: ASSIGN this instead of generating (§43.2). Store
    /// resolvability is checked by the caller holding the profile store;
    /// this layer validates shape only.
    pub camera_profile: Option<ProfileId>,
}

impl Default for RawDevelopParams {
    /// The §43.3 defaults column: asShot / ahd / 0 / 0 / null.
    fn default() -> RawDevelopParams {
        RawDevelopParams {
            white_balance: WhiteBalance::AsShot,
            demosaic: Demosaic::Ahd,
            exposure_ev: 0.0,
            highlight_recovery: 0.0,
            camera_profile: None,
        }
    }
}

/// The §43.3 reserved Post-1.0 JSON keys, in table order. v1 writes them
/// as `null` and rejects non-null input ("reserved until Post-1.0").
pub const RESERVED_POST_1_0_KEYS: &[&str] = &[
    "contrast",
    "shadows",
    "noiseReduction",
    "sharpening",
    "lensCorrection",
];

const V1_KEYS: &[&str] = &[
    "whiteBalance",
    "demosaic",
    "exposureEv",
    "highlightRecovery",
    "cameraProfile",
];

fn bad(field: &'static str, detail: impl Into<String>) -> CodecError {
    CodecError::OptionOutOfRange {
        field,
        detail: detail.into(),
    }
}

fn finite_in(field: &'static str, v: f32, lo: f32, hi: f32) -> Result<(), CodecError> {
    if !v.is_finite() {
        // §0.3 non-finite rejection, restated by the §43.3 validation table.
        return Err(bad(field, format!("non-finite value {v}")));
    }
    if v < lo || v > hi {
        // Out-of-range REJECTS rather than clamps (§43.3).
        return Err(bad(field, format!("{v} outside {lo}..={hi}")));
    }
    Ok(())
}

impl RawDevelopParams {
    /// The §43.3 [NORMATIVE] validation table, run BEFORE any pixel work
    /// (§10.2 staged-validation spirit). Failures name the field and map
    /// to `error.codec.bad_option` via the §8.1 taxonomy.
    pub fn validate(&self) -> Result<(), CodecError> {
        if let WhiteBalance::Custom { temp_k, tint } = self.white_balance {
            finite_in("raw.whiteBalance.tempK", temp_k, 2000.0, 50_000.0)?;
            finite_in("raw.whiteBalance.tint", tint, -150.0, 150.0)?;
        }
        finite_in("raw.exposureEv", self.exposure_ev, -10.0, 10.0)?;
        finite_in("raw.highlightRecovery", self.highlight_recovery, 0.0, 1.0)?;
        Ok(())
    }

    /// Serialize as the §43.6 `raw.develop.params` object: camelCase keys,
    /// reserved Post-1.0 keys written as `null` (§43.3 — reserved schema
    /// now, so v1 documents carry forward-compatible data).
    pub fn to_json(&self) -> Value {
        let wb = match self.white_balance {
            WhiteBalance::AsShot => serde_json::json!({ "mode": "asShot" }),
            WhiteBalance::Auto => serde_json::json!({ "mode": "auto" }),
            WhiteBalance::Custom { temp_k, tint } => serde_json::json!({
                "mode": "custom", "tempK": temp_k, "tint": tint,
            }),
        };
        let mut m = Map::new();
        m.insert("whiteBalance".into(), wb);
        m.insert("demosaic".into(), Value::from(self.demosaic.as_str()));
        m.insert("exposureEv".into(), Value::from(self.exposure_ev));
        m.insert(
            "highlightRecovery".into(),
            Value::from(self.highlight_recovery),
        );
        m.insert(
            "cameraProfile".into(),
            match self.camera_profile {
                Some(id) => Value::from(id.to_string()),
                None => Value::Null,
            },
        );
        for k in RESERVED_POST_1_0_KEYS {
            m.insert((*k).into(), Value::Null);
        }
        Value::Object(m)
    }

    /// Parse a §43.3 params object. Absent keys take the defaults column
    /// (the `--opt raw.*` grammar sets keys individually, §26.3/§43.3);
    /// unknown keys and non-null reserved keys REJECT (`error.codec
    /// .bad_option` — §43.3: "unknown or Post-1.0 keys fail"). The result
    /// is `validate()`d before it is returned.
    pub fn from_json(v: &Value) -> Result<RawDevelopParams, CodecError> {
        let obj = v
            .as_object()
            .ok_or_else(|| bad("raw.develop.params", "expected a JSON object"))?;
        let mut p = RawDevelopParams::default();
        for (k, val) in obj {
            match k.as_str() {
                "whiteBalance" => p.white_balance = parse_wb(val)?,
                "demosaic" => {
                    let s = val
                        .as_str()
                        .ok_or_else(|| bad("raw.demosaic", "expected a string"))?;
                    p.demosaic = Demosaic::from_str(s).ok_or_else(|| {
                        bad("raw.demosaic", format!("{s:?} not in linear|vng|ppg|ahd"))
                    })?;
                }
                "exposureEv" => p.exposure_ev = num(val, "raw.exposureEv")?,
                "highlightRecovery" => p.highlight_recovery = num(val, "raw.highlightRecovery")?,
                "cameraProfile" => {
                    p.camera_profile = match val {
                        Value::Null => None,
                        Value::String(_) => Some(
                            serde_json::from_value::<ProfileId>(val.clone()).map_err(|e| {
                                bad("raw.cameraProfile", format!("not a UUID: {e}"))
                            })?,
                        ),
                        _ => {
                            return Err(bad(
                                "raw.cameraProfile",
                                "expected a profile UUID string or null",
                            ))
                        }
                    };
                }
                reserved if RESERVED_POST_1_0_KEYS.contains(&reserved) => {
                    if !val.is_null() {
                        // §43.3: non-null → bad_option with a reserved detail.
                        return Err(bad(
                            "raw.develop.params",
                            format!("{reserved:?} is reserved until Post-1.0 (must be null in v1)"),
                        ));
                    }
                }
                unknown => {
                    debug_assert!(!V1_KEYS.contains(&unknown));
                    return Err(bad(
                        "raw.develop.params",
                        format!("unknown key {unknown:?}"),
                    ));
                }
            }
        }
        p.validate()?;
        Ok(p)
    }
}

fn num(v: &Value, field: &'static str) -> Result<f32, CodecError> {
    v.as_f64()
        .map(|f| f as f32)
        .ok_or_else(|| bad(field, "expected a number"))
}

fn parse_wb(v: &Value) -> Result<WhiteBalance, CodecError> {
    let obj = v
        .as_object()
        .ok_or_else(|| bad("raw.whiteBalance", "expected an object"))?;
    let mode = obj
        .get("mode")
        .and_then(Value::as_str)
        .ok_or_else(|| bad("raw.whiteBalance.mode", "missing mode string"))?;
    let has_pair = obj.contains_key("tempK") || obj.contains_key("tint");
    match mode {
        "asShot" | "auto" => {
            if has_pair {
                // §43.3: tempK/tint present IFF custom.
                return Err(bad(
                    "raw.whiteBalance",
                    format!("tempK/tint are only valid with mode custom (got {mode:?})"),
                ));
            }
            for k in obj.keys() {
                if k != "mode" {
                    return Err(bad("raw.whiteBalance", format!("unknown key {k:?}")));
                }
            }
            Ok(if mode == "asShot" {
                WhiteBalance::AsShot
            } else {
                WhiteBalance::Auto
            })
        }
        "custom" => {
            for k in obj.keys() {
                if !matches!(k.as_str(), "mode" | "tempK" | "tint") {
                    return Err(bad("raw.whiteBalance", format!("unknown key {k:?}")));
                }
            }
            let temp_k = num(
                obj.get("tempK")
                    .ok_or_else(|| bad("raw.whiteBalance.tempK", "required with mode custom"))?,
                "raw.whiteBalance.tempK",
            )?;
            let tint = num(
                obj.get("tint")
                    .ok_or_else(|| bad("raw.whiteBalance.tint", "required with mode custom"))?,
                "raw.whiteBalance.tint",
            )?;
            Ok(WhiteBalance::Custom { temp_k, tint })
        }
        other => Err(bad(
            "raw.whiteBalance.mode",
            format!("{other:?} not in asShot|auto|custom"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_table_and_validate() {
        let p = RawDevelopParams::default();
        assert_eq!(p.white_balance, WhiteBalance::AsShot);
        assert_eq!(p.demosaic, Demosaic::Ahd);
        assert_eq!(p.exposure_ev, 0.0);
        assert_eq!(p.highlight_recovery, 0.0);
        assert_eq!(p.camera_profile, None);
        p.validate().unwrap();
    }

    #[test]
    fn json_round_trip_writes_reserved_keys_as_null() {
        let p = RawDevelopParams {
            white_balance: WhiteBalance::Custom {
                temp_k: 5500.0,
                tint: 10.0,
            },
            demosaic: Demosaic::Ahd,
            exposure_ev: 0.5,
            highlight_recovery: 0.25,
            camera_profile: None,
        };
        let j = p.to_json();
        for k in RESERVED_POST_1_0_KEYS {
            assert!(j.get(*k).unwrap().is_null(), "{k} must serialize null");
        }
        assert_eq!(j["whiteBalance"]["mode"], "custom");
        assert_eq!(RawDevelopParams::from_json(&j).unwrap(), p);
    }

    #[test]
    fn out_of_range_rejects_rather_than_clamps() {
        for (params, field) in [
            (
                RawDevelopParams {
                    exposure_ev: 10.5,
                    ..Default::default()
                },
                "raw.exposureEv",
            ),
            (
                RawDevelopParams {
                    highlight_recovery: -0.01,
                    ..Default::default()
                },
                "raw.highlightRecovery",
            ),
            (
                RawDevelopParams {
                    white_balance: WhiteBalance::Custom {
                        temp_k: 1999.0,
                        tint: 0.0,
                    },
                    ..Default::default()
                },
                "raw.whiteBalance.tempK",
            ),
            (
                RawDevelopParams {
                    white_balance: WhiteBalance::Custom {
                        temp_k: 5000.0,
                        tint: 150.5,
                    },
                    ..Default::default()
                },
                "raw.whiteBalance.tint",
            ),
        ] {
            match params.validate() {
                Err(CodecError::OptionOutOfRange { field: f, .. }) => assert_eq!(f, field),
                other => panic!("expected OptionOutOfRange({field}), got {other:?}"),
            }
        }
    }

    #[test]
    fn non_finite_rejects() {
        let p = RawDevelopParams {
            exposure_ev: f32::NAN,
            ..Default::default()
        };
        assert!(matches!(
            p.validate(),
            Err(CodecError::OptionOutOfRange {
                field: "raw.exposureEv",
                ..
            })
        ));
    }

    #[test]
    fn temp_pair_present_iff_custom() {
        let j = serde_json::json!({ "whiteBalance": { "mode": "asShot", "tempK": 5000.0 } });
        assert!(RawDevelopParams::from_json(&j).is_err());
        let j = serde_json::json!({ "whiteBalance": { "mode": "custom" } });
        assert!(RawDevelopParams::from_json(&j).is_err());
        let j = serde_json::json!({
            "whiteBalance": { "mode": "custom", "tempK": 6500.0, "tint": 0.0 }
        });
        assert!(RawDevelopParams::from_json(&j).is_ok());
    }

    #[test]
    fn reserved_keys_null_ok_non_null_rejected() {
        let j = serde_json::json!({ "contrast": null, "sharpening": null });
        assert!(RawDevelopParams::from_json(&j).is_ok());
        let j = serde_json::json!({ "contrast": 0.2 });
        match RawDevelopParams::from_json(&j) {
            Err(CodecError::OptionOutOfRange { detail, .. }) => {
                assert!(detail.contains("reserved until Post-1.0"), "{detail}");
            }
            other => panic!("expected reserved rejection, got {other:?}"),
        }
        // Even the neutral-equivalent value is rejected: reserved means
        // reserved (§43.3), not "accepted when harmless".
        let j = serde_json::json!({ "shadows": 0.0 });
        assert!(RawDevelopParams::from_json(&j).is_err());
    }

    #[test]
    fn unknown_key_rejected() {
        let j = serde_json::json!({ "vibrance": 1.0 });
        match RawDevelopParams::from_json(&j) {
            Err(CodecError::OptionOutOfRange { detail, .. }) => {
                assert!(detail.contains("vibrance"), "{detail}");
            }
            other => panic!("expected unknown-key rejection, got {other:?}"),
        }
    }

    #[test]
    fn bad_option_message_id() {
        let err = RawDevelopParams {
            exposure_ev: 99.0,
            ..Default::default()
        }
        .validate()
        .unwrap_err();
        assert_eq!(err.message_id(), "error.codec.bad_option");
    }

    #[test]
    fn camera_profile_uuid_shape() {
        let j = serde_json::json!({
            "cameraProfile": "6fa459ea-ee8a-3ca4-894e-db77e160355e"
        });
        assert!(RawDevelopParams::from_json(&j)
            .unwrap()
            .camera_profile
            .is_some());
        let j = serde_json::json!({ "cameraProfile": "not-a-uuid" });
        assert!(RawDevelopParams::from_json(&j).is_err());
        let j = serde_json::json!({ "cameraProfile": 7 });
        assert!(RawDevelopParams::from_json(&j).is_err());
    }
}
