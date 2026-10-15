// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Portions derived from libmypaint, Copyright (C) 2008-2011 Martin Renold
// and contributors. ISC license. (The `.myb` setting identifiers, curve
// semantics, and mapping formulas below are ported per the port-spec memo
// MEMO-libmypaint-brush-dynamics.md §2/§3/§5 from the ISC tree at commit
// d5a88fbe6649d5ec776bc42ec8c1f4bb29d7fd7f; ISC permission notice carried
// in THIRD-PARTY-NOTICES.md.)

//! Spec: SPEC §28 (presets) / Part 15 (brush engine, §15.3–§15.7);
//! ALGORITHMS.md §23.B stroke rendering; INTEGRATION-NOTES §6 order 3c
//! (`.myb` importer); port scope fixed by
//! `appname-proto/research/memos/MEMO-libmypaint-brush-dynamics.md` §5.1.
//!
//! `.myb` (MyPaint brush v3 JSON) → neutral imported-preset description.
//!
//! libmypaint's engine is ADDITIVE (setting = base + Σ per-input curves);
//! Appname's SPEC §15.4 engine is multiplicative with monotone-cubic
//! curves. Per memo §5.1 the conversion happens HERE, at import time only:
//! curves are re-sampled into multiplier knots around the base value
//! (exact for the radius family in the log domain), the opacity triple
//! (`opaque`, `opaque_multiply`, `opaque_linearize`) folds into the
//! (opacity, flow) pair, spacing converts from dabs-per-radius, and
//! everything unmappable is reported in `warnings` (memo: "import warning
//! list per preset").
//!
//! ADAPTER SEAM (brokered): the O-TOOLS preset system maps
//! [`ImportedBrush`] onto `appname_tools::brush::BrushPreset` (`.bbrush`)
//! — a handoff via O-INT, not a direct edit; imported presets are ordinary
//! preset files afterwards (nothing model-dependent, D-48-neutral).
//! Numeric hardness equivalence is ratified there against the §15.5
//! smoothstep tip (this module documents its half-opacity-radius match
//! assumption inline).

use serde_json::Value;

/// A libmypaint dynamic input relevant to the import mapping (memo §3.1's
/// 18-input table, reduced to what Appname can consume + what needs
/// warnings).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MybInput {
    /// Stylus pressure (0..1 soft).
    Pressure,
    /// Fresh uniform draw per dab.
    Random,
    /// Fine speed EMA (log-mapped, memo §3.4).
    Speed1,
    /// Gross speed EMA (log-mapped, memo §3.4).
    Speed2,
    /// 180°-symmetric stroke direction.
    Direction,
    /// Stylus tilt declination.
    TiltDeclination,
    /// Stylus tilt ascension.
    TiltAscension,
    /// Stroke progress 0..1 (memo §3.6 — flagged Appname gap).
    Stroke,
    /// Anything else (warned, dropped).
    Unsupported,
}

impl MybInput {
    fn parse(name: &str) -> MybInput {
        match name {
            "pressure" => MybInput::Pressure,
            "random" => MybInput::Random,
            "speed1" => MybInput::Speed1,
            "speed2" => MybInput::Speed2,
            "direction" => MybInput::Direction,
            "tilt_declination" => MybInput::TiltDeclination,
            "tilt_ascension" => MybInput::TiltAscension,
            "stroke" => MybInput::Stroke,
            _ => MybInput::Unsupported,
        }
    }
}

/// Which Appname preset quantity a converted curve modulates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MappedOutput {
    /// Tip size multiplier.
    Size,
    /// Per-dab flow multiplier.
    Flow,
    /// Stroke opacity multiplier.
    Opacity,
    /// Scatter amount multiplier.
    Scatter,
    /// Tip angle offset (degrees, additive by contract).
    AngleDeg,
    /// Smudge pickup ratio multiplier.
    Smudge,
}

/// The log-speed input mapping parameters (memo §3.4): input =
/// `log(gamma + speed) * m + q`, anchored at physical speed 45 -> 0.5 with
/// slope 0.015. The brokered adapter uses these to place speed-curve knots
/// onto Appname's velocity axis.
#[derive(Clone, Copy, Debug)]
pub struct SpeedMap {
    /// `exp(speed?_gamma setting)`.
    pub gamma: f64,
    /// Slope `0.015 * (45 + gamma)`.
    pub m: f64,
    /// Offset `0.5 - m * ln(45 + gamma)`.
    pub q: f64,
}

impl SpeedMap {
    fn new(gamma_setting: f64) -> SpeedMap {
        let gamma = gamma_setting.exp();
        let m = 0.015 * (45.0 + gamma);
        let q = 0.5 - m * (45.0f64 + gamma).ln();
        SpeedMap { gamma, m, q }
    }

    /// Physical speed (px/s, view-scaled) for a libmypaint speed-input value.
    pub fn input_to_speed(&self, input: f64) -> f64 {
        ((input - self.q) / self.m).exp() - self.gamma
    }
}

/// One converted dynamics curve: multiplier (or additive angle) knots over
/// the input's native domain.
#[derive(Clone, Debug)]
pub struct ImportedCurve {
    /// Source input.
    pub input: MybInput,
    /// Target quantity.
    pub output: MappedOutput,
    /// (input value, multiplier) knots, ascending input; the adapter fits
    /// Appname's monotone-cubic `DynamicCurve` through them.
    pub knots: Vec<(f64, f64)>,
    /// For Speed1/Speed2 curves: the log-map so knots (in libmypaint
    /// speed-input units) can be re-axed to physical speed.
    pub speed_map: Option<SpeedMap>,
}

/// Neutral imported preset (memo §5.1's mapping table output).
#[derive(Clone, Debug, Default)]
pub struct ImportedBrush {
    /// Tip diameter in pixels (`2 * exp(radius_logarithmic)`).
    pub diameter_px: f64,
    /// Dab spacing as a fraction of diameter
    /// (`1 / (2 * dabs_per_actual_radius)` per memo §5.1).
    pub spacing: f64,
    /// Stroke opacity in [0, 1] (from `opaque`).
    pub opacity: f64,
    /// Per-dab flow in [0, 1] (from the `opaque_linearize` fold, §3.3).
    pub flow: f64,
    /// Appname hardness in [0, 1] (half-opacity-radius match, see module
    /// docs).
    pub hardness: f64,
    /// Tip roundness = `1 / elliptical_dab_ratio` in (0, 1].
    pub roundness: f64,
    /// Tip angle in degrees.
    pub angle_deg: f64,
    /// Scatter as a fraction of diameter (from `offset_by_random`, which is
    /// in base-radius units: fraction = value / 2).
    pub scatter: f64,
    /// Eraser preset (`eraser` setting > 0.5).
    pub eraser: bool,
    /// Paint only where alpha exists (`lock_alpha` > 0.5).
    pub lock_alpha: bool,
    /// Pigment/spectral mixing requested (`paint_mode` > 0 per memo §5.3).
    pub pigment_mode: bool,
    /// Smudge ratio [0, 1].
    pub smudge: f64,
    /// Smudge length (pickup persistence) [0, 1].
    pub smudge_length: f64,
    /// Converted dynamics curves.
    pub curves: Vec<ImportedCurve>,
    /// Unmappable settings/inputs, one line each (memo §5.1 warning list).
    pub warnings: Vec<String>,
}

/// One parsed dynamics input: (input id, raw input name, control points).
type RawInputCurve = (MybInput, String, Vec<(f64, f64)>);

/// A raw parsed setting: base value + per-input control points.
struct RawSetting {
    base: f64,
    inputs: Vec<RawInputCurve>,
}

/// Evaluate a libmypaint piecewise-linear curve (memo §3.1 mapping
/// semantics): linear between points, linear extrapolation along the end
/// segments, degenerate segments evaluate to their y.
fn eval_curve(points: &[(f64, f64)], x: f64) -> f64 {
    match points.len() {
        0 => 0.0,
        1 => points[0].1,
        _ => {
            // Find the segment: before first -> first segment, after last
            // -> last segment (linear extrapolation).
            let mut i = 0usize;
            while i + 2 < points.len() && x > points[i + 1].0 {
                i += 1;
            }
            let (x0, y0) = points[i];
            let (x1, y1) = points[i + 1];
            if (x1 - x0).abs() < 1e-12 || (y1 - y0).abs() < 1e-12 {
                y1
            } else {
                y0 + (x - x0) * (y1 - y0) / (x1 - x0)
            }
        }
    }
}

/// Settings that map cleanly; everything else warns (memo §5.1).
const MAPPED: &[&str] = &[
    "radius_logarithmic",
    "opaque",
    "opaque_multiply",
    "opaque_linearize",
    "dabs_per_actual_radius",
    "dabs_per_basic_radius",
    "hardness",
    "softness",
    "elliptical_dab_ratio",
    "elliptical_dab_angle",
    "offset_by_random",
    "eraser",
    "lock_alpha",
    "paint_mode",
    "smudge",
    "smudge_length",
    "speed1_gamma",
    "speed2_gamma",
];

/// libmypaint half-opacity squared-radius for a given hardness (memo §3.7
/// two-segment falloff), used for the hardness match.
fn myb_half_opacity_rr(h: f64) -> f64 {
    let h = h.clamp(1e-4, 1.0);
    // Segment 1 (rr in [0, h]): v = 1 - rr * (1/h - 1); reaches v = h at
    // rr = h. Segment 2 (rr in [h, 1]): v = h * (1 - rr) / (1 - h).
    if h >= 0.5 {
        // 0.5 falls inside segment 1 (since v(h) = h >= 0.5) unless h = 1.
        if h >= 1.0 - 1e-9 {
            return 1.0; // hard disc: half-opacity at the rim
        }
        (1.0 - 0.5) / (1.0 / h - 1.0)
    } else {
        // 0.5 falls inside segment 2.
        1.0 - 0.5 * (1.0 - h) / h.max(1e-4)
    }
}

/// Map libmypaint hardness onto the §15.5 smoothstep tip by matching the
/// half-opacity radius: the smoothstep tip is flat to `h_app * R` and falls
/// to 0 at `R`, crossing 0.5 at `(1 + h_app) / 2 * R`.
fn map_hardness(myb_hardness: f64) -> f64 {
    let rr_half = myb_half_opacity_rr(myb_hardness);
    let t_half = rr_half.max(0.0).sqrt(); // radius fraction
    (2.0 * t_half - 1.0).clamp(0.0, 1.0)
}

/// Parse + convert a `.myb` v3 JSON document (memo §5.1).
pub fn import_myb(json_text: &str) -> Result<ImportedBrush, String> {
    let doc: Value =
        serde_json::from_str(json_text).map_err(|e| format!("myb: invalid JSON: {e}"))?;
    let version = doc.get("version").and_then(Value::as_i64).unwrap_or(0);
    if version < 2 {
        return Err(format!("myb: unsupported version {version} (need >= 2)"));
    }
    let settings_obj = doc
        .get("settings")
        .and_then(Value::as_object)
        .ok_or("myb: missing settings object")?;

    let mut warnings = Vec::new();
    let mut settings: Vec<(String, RawSetting)> = Vec::new();
    for (name, sv) in settings_obj {
        let base = sv.get("base_value").and_then(Value::as_f64).unwrap_or(0.0);
        let mut inputs = Vec::new();
        if let Some(io) = sv.get("inputs").and_then(Value::as_object) {
            for (iname, pts) in io {
                let mut points = Vec::new();
                if let Some(arr) = pts.as_array() {
                    for p in arr {
                        if let Some(pair) = p.as_array() {
                            if pair.len() == 2 {
                                if let (Some(x), Some(y)) = (pair[0].as_f64(), pair[1].as_f64()) {
                                    points.push((x, y));
                                }
                            }
                        }
                    }
                }
                inputs.push((MybInput::parse(iname), iname.clone(), points));
            }
        }
        if !MAPPED.contains(&name.as_str()) && (base != 0.0 || !inputs.is_empty()) {
            warnings.push(format!("setting not mapped: {name}"));
        }
        settings.push((name.clone(), RawSetting { base, inputs }));
    }
    // Deterministic order regardless of JSON object order.
    settings.sort_by(|a, b| a.0.cmp(&b.0));

    let get = |name: &str| -> Option<&RawSetting> {
        settings.iter().find(|(n, _)| n == name).map(|(_, s)| s)
    };
    let base_of = |name: &str, default: f64| get(name).map(|s| s.base).unwrap_or(default);

    let radius_log = base_of("radius_logarithmic", 2.0);
    let diameter_px = 2.0 * radius_log.exp();
    let dpar = base_of("dabs_per_actual_radius", 2.0).max(0.01);
    let dpbr = base_of("dabs_per_basic_radius", 0.0).max(0.0);
    let spacing = 1.0 / (2.0 * dpar);

    // Opacity triple -> (opacity, flow), memo §3.3: dabs_per_pixel =
    // (dpar + dpbr) * 2 (min 1), blended toward 1 by the linearize factor;
    // flow = 1 - (1 - opaque)^(1 / dp_eff).
    let opaque = base_of("opaque", 1.0).clamp(0.0, 1.0);
    let linearize = base_of("opaque_linearize", 0.9).clamp(0.0, 1.0);
    let dp = ((dpar + dpbr) * 2.0).max(1.0);
    let dp_eff = 1.0 + linearize * (dp - 1.0);
    let flow = if opaque >= 1.0 {
        1.0
    } else {
        1.0 - (1.0 - opaque).powf(1.0 / dp_eff)
    };

    let ratio = base_of("elliptical_dab_ratio", 1.0).max(1.0);
    let mut brush = ImportedBrush {
        diameter_px,
        spacing,
        opacity: opaque,
        flow,
        hardness: map_hardness(base_of("hardness", 0.8)),
        roundness: 1.0 / ratio,
        angle_deg: base_of("elliptical_dab_angle", 90.0),
        scatter: base_of("offset_by_random", 0.0).max(0.0) / 2.0,
        eraser: base_of("eraser", 0.0) > 0.5,
        lock_alpha: base_of("lock_alpha", 0.0) > 0.5,
        pigment_mode: base_of("paint_mode", 0.0) > 0.0,
        smudge: base_of("smudge", 0.0).clamp(0.0, 1.0),
        smudge_length: base_of("smudge_length", 0.5).clamp(0.0, 1.0),
        curves: Vec::new(),
        warnings,
    };

    // Curve conversion (memo §5.1): sample each per-input additive curve at
    // up to 8 knots; multiplier = exp(c(x)) for the radius family (exact in
    // log domain), (base + c(x)) / base for multiplicative bases, and the
    // raw curve value where the setting IS the multiplier (opaque_multiply).
    let speed1_map = SpeedMap::new(base_of("speed1_gamma", 4.0));
    let speed2_map = SpeedMap::new(base_of("speed2_gamma", 4.0));
    for (name, setting) in &settings {
        for (input, iname, points) in &setting.inputs {
            if points.is_empty() {
                continue;
            }
            if *input == MybInput::Unsupported {
                brush
                    .warnings
                    .push(format!("input not mapped: {name} <- {iname}"));
                continue;
            }
            if *input == MybInput::Stroke {
                // Memo §3.6: Appname has no stroke-progress input yet
                // (REPORT-B4 open question); surface, don't silently drop.
                brush.warnings.push(format!(
                    "input needs DynInput::StrokeProgress (engine gap): {name} <- {iname}"
                ));
                continue;
            }
            let (output, style) = match name.as_str() {
                "radius_logarithmic" => (MappedOutput::Size, CurveStyle::ExpOfC),
                "opaque_multiply" => (MappedOutput::Flow, CurveStyle::RawValue),
                "opaque" => (MappedOutput::Opacity, CurveStyle::AroundBase(setting.base)),
                "offset_by_random" => (MappedOutput::Scatter, CurveStyle::AroundBase(setting.base)),
                "elliptical_dab_angle" => (MappedOutput::AngleDeg, CurveStyle::Additive),
                "smudge" => (MappedOutput::Smudge, CurveStyle::AroundBase(setting.base)),
                _ => {
                    brush
                        .warnings
                        .push(format!("dynamic not mapped: {name} <- {iname}"));
                    continue;
                }
            };
            let x_min = points.first().unwrap().0;
            let x_max = points.last().unwrap().0;
            let n_knots = 8.min(points.len().max(2) * 2);
            let mut knots = Vec::with_capacity(n_knots);
            for k in 0..n_knots {
                let x = x_min + (x_max - x_min) * k as f64 / (n_knots - 1) as f64;
                let c = eval_curve(points, x);
                let v = match style {
                    CurveStyle::ExpOfC => c.exp(),
                    CurveStyle::RawValue => c.clamp(0.0, 2.0),
                    CurveStyle::Additive => c,
                    CurveStyle::AroundBase(b) => {
                        if b.abs() > 1e-9 {
                            (b + c) / b
                        } else {
                            // Zero base with a curve: the curve IS the value.
                            c
                        }
                    }
                };
                knots.push((x, v));
            }
            let speed_map = match input {
                MybInput::Speed1 => Some(speed1_map),
                MybInput::Speed2 => Some(speed2_map),
                _ => None,
            };
            brush.curves.push(ImportedCurve {
                input: *input,
                output,
                knots,
                speed_map,
            });
        }
    }
    Ok(brush)
}

/// How an additive libmypaint curve folds into a multiplier.
#[derive(Clone, Copy)]
enum CurveStyle {
    /// `exp(c)` — radius family (log-domain, exact).
    ExpOfC,
    /// The curve value used directly (settings whose base is a no-op
    /// multiplier slot, e.g. `opaque_multiply`).
    RawValue,
    /// `(base + c) / base` around a non-zero base.
    AroundBase(f64),
    /// Additive passthrough (angles).
    Additive,
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_MYB: &str = r#"{
        "version": 3,
        "group": "",
        "parent_brush_name": "test/roundbrush",
        "settings": {
            "radius_logarithmic": {
                "base_value": 2.0,
                "inputs": {
                    "pressure": [[0.0, -0.6], [1.0, 0.6]]
                }
            },
            "opaque": { "base_value": 0.8, "inputs": {} },
            "opaque_multiply": {
                "base_value": 0.0,
                "inputs": { "pressure": [[0.0, 0.0], [1.0, 1.0]] }
            },
            "opaque_linearize": { "base_value": 0.9, "inputs": {} },
            "dabs_per_actual_radius": { "base_value": 4.0, "inputs": {} },
            "hardness": { "base_value": 0.8, "inputs": {} },
            "elliptical_dab_ratio": { "base_value": 2.0, "inputs": {} },
            "elliptical_dab_angle": { "base_value": 45.0, "inputs": {} },
            "offset_by_random": { "base_value": 1.0, "inputs": {} },
            "smudge": { "base_value": 0.0, "inputs": {} },
            "paint_mode": { "base_value": 1.0, "inputs": {} },
            "speed1_gamma": { "base_value": 4.0, "inputs": {} },
            "radius_by_random": { "base_value": 0.3, "inputs": {} },
            "gridmap_scale": {
                "base_value": 1.0,
                "inputs": { "custom": [[0.0, 0.0], [1.0, 1.0]] }
            },
            "stroke_threshold": { "base_value": 0.02, "inputs": {} },
            "opaque2": { "base_value": 0.0, "inputs": { "stroke": [[0.0, 0.0], [1.0, -1.0]] } }
        }
    }"#;

    #[test]
    fn imports_geometry_and_opacity() {
        let b = import_myb(SAMPLE_MYB).unwrap();
        // diameter = 2 * exp(2.0)
        assert!((b.diameter_px - 2.0 * 2.0f64.exp()).abs() < 1e-9);
        // spacing = 1 / (2 * 4)
        assert!((b.spacing - 0.125).abs() < 1e-12);
        assert!((b.opacity - 0.8).abs() < 1e-12);
        // flow folds opaque through the linearize formula: dp = 8,
        // dp_eff = 1 + 0.9 * 7 = 7.3, flow = 1 - 0.2^(1/7.3).
        let expect_flow = 1.0 - 0.2f64.powf(1.0 / 7.3);
        assert!((b.flow - expect_flow).abs() < 1e-12, "flow {}", b.flow);
        assert!((b.roundness - 0.5).abs() < 1e-12);
        assert!((b.angle_deg - 45.0).abs() < 1e-12);
        assert!((b.scatter - 0.5).abs() < 1e-12);
        assert!(b.pigment_mode, "paint_mode > 0 must set pigment mode");
    }

    #[test]
    fn radius_pressure_curve_is_exact_in_log_domain() {
        let b = import_myb(SAMPLE_MYB).unwrap();
        let size = b
            .curves
            .iter()
            .find(|c| c.output == MappedOutput::Size)
            .expect("size curve imported");
        assert_eq!(size.input, MybInput::Pressure);
        // Additive log-radius curve c(x) = -0.6 + 1.2 x -> multiplier
        // exp(c): endpoints exp(-0.6), exp(0.6).
        let first = size.knots.first().unwrap();
        let last = size.knots.last().unwrap();
        assert!((first.1 - (-0.6f64).exp()).abs() < 1e-9);
        assert!((last.1 - 0.6f64.exp()).abs() < 1e-9);
    }

    #[test]
    fn unmappable_settings_are_warned_not_dropped_silently() {
        let b = import_myb(SAMPLE_MYB).unwrap();
        assert!(
            b.warnings.iter().any(|w| w.contains("radius_by_random")),
            "radius_by_random must warn: {:?}",
            b.warnings
        );
        assert!(
            b.warnings.iter().any(|w| w.contains("gridmap_scale")),
            "gridmap must warn"
        );
        assert!(
            b.warnings.iter().any(|w| w.contains("StrokeProgress")),
            "stroke input must surface the engine gap"
        );
    }

    #[test]
    fn speed_map_reproduces_the_documented_anchor() {
        // Memo §3.4: at physical speed 45 the input reports 0.5.
        let m = SpeedMap::new(4.0);
        let speed = m.input_to_speed(0.5);
        assert!((speed - 45.0).abs() < 1e-6, "anchor speed {speed}");
    }

    #[test]
    fn curve_evaluation_semantics() {
        let pts = [(0.0, 0.0), (0.5, 1.0), (1.0, 1.0)];
        assert!((eval_curve(&pts, 0.25) - 0.5).abs() < 1e-12);
        // Degenerate final segment (equal y) evaluates to its y.
        assert!((eval_curve(&pts, 0.75) - 1.0).abs() < 1e-12);
        // Extrapolation along the first segment.
        assert!((eval_curve(&pts, -0.25) + 0.5).abs() < 1e-12);
    }

    #[test]
    fn rejects_garbage_and_old_versions() {
        assert!(import_myb("not json").is_err());
        assert!(import_myb(r#"{"version": 1, "settings": {}}"#).is_err());
        assert!(import_myb(r#"{"version": 3}"#).is_err());
    }

    /// The same preset as `SAMPLE_MYB` with every JSON object's keys
    /// genuinely permuted: top-level keys rotated (`settings` first),
    /// settings listed in reverse order, and each setting's inner object
    /// flipped (`inputs` before `base_value`). Values are byte-identical
    /// to `SAMPLE_MYB`'s.
    const SAMPLE_MYB_REORDERED: &str = r#"{
        "settings": {
            "opaque2": { "inputs": { "stroke": [[0.0, 0.0], [1.0, -1.0]] }, "base_value": 0.0 },
            "stroke_threshold": { "inputs": {}, "base_value": 0.02 },
            "gridmap_scale": {
                "inputs": { "custom": [[0.0, 0.0], [1.0, 1.0]] },
                "base_value": 1.0
            },
            "radius_by_random": { "inputs": {}, "base_value": 0.3 },
            "speed1_gamma": { "inputs": {}, "base_value": 4.0 },
            "paint_mode": { "inputs": {}, "base_value": 1.0 },
            "smudge": { "inputs": {}, "base_value": 0.0 },
            "offset_by_random": { "inputs": {}, "base_value": 1.0 },
            "elliptical_dab_angle": { "inputs": {}, "base_value": 45.0 },
            "elliptical_dab_ratio": { "inputs": {}, "base_value": 2.0 },
            "hardness": { "inputs": {}, "base_value": 0.8 },
            "dabs_per_actual_radius": { "inputs": {}, "base_value": 4.0 },
            "opaque_linearize": { "inputs": {}, "base_value": 0.9 },
            "opaque_multiply": {
                "inputs": { "pressure": [[0.0, 0.0], [1.0, 1.0]] },
                "base_value": 0.0
            },
            "opaque": { "inputs": {}, "base_value": 0.8 },
            "radius_logarithmic": {
                "inputs": {
                    "pressure": [[0.0, -0.6], [1.0, 0.6]]
                },
                "base_value": 2.0
            }
        },
        "parent_brush_name": "test/roundbrush",
        "group": "",
        "version": 3
    }"#;

    /// Bitwise f64 equality (`to_bits`) — the determinism contract is
    /// bit-identity, not approximate equality.
    fn assert_f64_bits(a: f64, b: f64, what: &str) {
        assert_eq!(a.to_bits(), b.to_bits(), "{what}: {a} vs {b}");
    }

    #[test]
    fn import_is_deterministic_across_key_order() {
        // Two documents with identical values but permuted JSON key order
        // must import bit-identically: the importer sorts settings by name
        // (the sort in `import_myb`) so document order never leaks into
        // curve order, warning order, or any derived scalar.
        let a = import_myb(SAMPLE_MYB).unwrap();
        let b = import_myb(SAMPLE_MYB_REORDERED).unwrap();

        // Guard against the permutation silently degenerating (the old
        // version of this test self-replaced a key, comparing a document
        // with itself).
        assert_ne!(
            SAMPLE_MYB, SAMPLE_MYB_REORDERED,
            "reordered fixture must actually differ textually"
        );

        assert_f64_bits(a.diameter_px, b.diameter_px, "diameter_px");
        assert_f64_bits(a.spacing, b.spacing, "spacing");
        assert_f64_bits(a.opacity, b.opacity, "opacity");
        assert_f64_bits(a.flow, b.flow, "flow");
        assert_f64_bits(a.hardness, b.hardness, "hardness");
        assert_f64_bits(a.roundness, b.roundness, "roundness");
        assert_f64_bits(a.angle_deg, b.angle_deg, "angle_deg");
        assert_f64_bits(a.scatter, b.scatter, "scatter");
        assert_eq!(a.eraser, b.eraser);
        assert_eq!(a.lock_alpha, b.lock_alpha);
        assert_eq!(a.pigment_mode, b.pigment_mode);
        assert_f64_bits(a.smudge, b.smudge, "smudge");
        assert_f64_bits(a.smudge_length, b.smudge_length, "smudge_length");

        // Warnings: identical content AND identical order.
        assert_eq!(a.warnings, b.warnings);

        // Curves: identical order, mapping, and bit-identical knots.
        assert_eq!(a.curves.len(), b.curves.len(), "curve count");
        for (i, (ca, cb)) in a.curves.iter().zip(&b.curves).enumerate() {
            assert_eq!(ca.input, cb.input, "curve {i} input");
            assert_eq!(ca.output, cb.output, "curve {i} output");
            assert_eq!(ca.knots.len(), cb.knots.len(), "curve {i} knot count");
            for (j, (ka, kb)) in ca.knots.iter().zip(&cb.knots).enumerate() {
                assert_f64_bits(ka.0, kb.0, &format!("curve {i} knot {j} x"));
                assert_f64_bits(ka.1, kb.1, &format!("curve {i} knot {j} y"));
            }
            match (&ca.speed_map, &cb.speed_map) {
                (None, None) => {}
                (Some(ma), Some(mb)) => {
                    assert_f64_bits(ma.gamma, mb.gamma, &format!("curve {i} speed_map.gamma"));
                    assert_f64_bits(ma.m, mb.m, &format!("curve {i} speed_map.m"));
                    assert_f64_bits(ma.q, mb.q, &format!("curve {i} speed_map.q"));
                }
                _ => panic!("curve {i}: speed_map presence differs"),
            }
        }

        // Belt-and-braces: the full Debug renderings agree (Rust's f64
        // Debug is shortest-round-trip, so this catches any field added
        // to ImportedBrush later without a comparison above).
        assert_eq!(format!("{a:?}"), format!("{b:?}"));
    }
}
