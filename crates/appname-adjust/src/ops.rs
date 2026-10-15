// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §17.2 [NORMATIVE] per-adjustment blocks; DD ㉘ (LUT
//! compilation). The slice-1 adjustments, each from its block verbatim:
//! brightness/contrast (legacy ramp + the default endpoint-preserving
//! power-lift/quintic-S pair with Newton S⁻¹ at bake), levels (the
//! canonical `levels_lut`, per-channel-then-composite composition),
//! curves (monotone PCHIP — the shared Fritsch–Carlson machinery),
//! exposure (linear domain, signed power), hue/saturation ([NORMATIVE]
//! hexcone, ±15°/±45° feathered bands — partition of unity where bands
//! meet — and the colorize replace), invert, posterize, threshold.
//! Domains per block; display-referred ops route through the crate's one
//! §5 hook pair. Alpha untouched by all eight. Clamp on store, never
//! mid-pipeline (§17.2 header).

use serde_json::Value;
use std::sync::Arc;

use crate::kernel::{
    bake_display_lut, display_closed, display_decode, display_encode, AdjError, AdjKernel,
    Adjustment,
};

// ---------------------------------------------------------------------------
// Shared param plumbing.
// ---------------------------------------------------------------------------

fn f32_param(
    params: &Value,
    name: &'static str,
    default: f64,
    lo: f64,
    hi: f64,
) -> Result<f32, AdjError> {
    match params.get(name) {
        None => Ok(default as f32),
        Some(v) => {
            let x = v.as_f64().unwrap_or(f64::NAN);
            let x = if x.is_nan() {
                default
            } else if x == f64::INFINITY {
                hi
            } else if x == f64::NEG_INFINITY {
                lo
            } else {
                x
            };
            if !(lo..=hi).contains(&x) {
                return Err(AdjError::ParamOutOfRange {
                    param: name,
                    detail: format!("{x} outside {lo}..={hi} (§17.2)"),
                });
            }
            Ok(x as f32)
        }
    }
}

#[inline]
fn lum(rgb: [f32; 3]) -> f32 {
    0.3 * rgb[0] + 0.59 * rgb[1] + 0.11 * rgb[2] // §4.5 coefficients
}

// ---------------------------------------------------------------------------
// Brightness/Contrast — adjustment.brightness_contrast
// ---------------------------------------------------------------------------

pub struct BrightnessContrast;

/// The §17.2 quintic S-curve (mid slope 1.875).
#[inline]
fn quintic_s(t: f32) -> f32 {
    t * t * t * (10.0 + t * (-15.0 + 6.0 * t))
}

/// S⁻¹ by Newton iteration (the block's bake-time rule): monotone on
/// [0,1]; 24 fixed iterations from the identity seed reach f32 precision
/// everywhere (deterministic — no convergence-dependent exit).
fn quintic_s_inv(y: f32) -> f32 {
    // Safeguarded Newton (bracketed): S′ vanishes at the endpoints, where
    // a bare Newton step stalls or overshoots — the bisection fallback
    // keeps every step inside the bracket, so the sampled inverse stays
    // monotone to f32 precision. Fixed 48 iterations, deterministic.
    let y = y.clamp(0.0, 1.0);
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    let mut t = y;
    for _ in 0..48 {
        let f = quintic_s(t) - y;
        if f > 0.0 {
            hi = t;
        } else {
            lo = t;
        }
        let d = 30.0 * t * t * (t - 1.0) * (t - 1.0); // S′(t) = 30t²(t−1)²
        let newton = if d > 1e-9 { t - f / d } else { f32::NAN };
        t = if newton.is_finite() && newton > lo && newton < hi {
            newton
        } else {
            0.5 * (lo + hi) // bisect when Newton exits the bracket
        };
    }
    t
}

fn bc_display_form(brightness: f32, contrast: f32, legacy: bool) -> impl Fn(f32, usize) -> f32 {
    move |v, _c| {
        if legacy {
            // §17.2 table line exactly; clamp on store.
            ((v - 0.5) * (1.0 + contrast) + 0.5 + brightness).clamp(0.0, 1.0)
        } else {
            let v1 = v.clamp(0.0, 1.0).powf(2.0f32.powf(-1.25 * brightness));
            if contrast >= 0.0 {
                v1 + (quintic_s(v1) - v1) * contrast
            } else {
                v1 + (quintic_s_inv(v1) - v1) * (-contrast)
            }
        }
    }
}

impl Adjustment for BrightnessContrast {
    fn id(&self) -> &'static str {
        "adjustment.brightness_contrast"
    }

    fn param_schema(&self) -> &'static str {
        r#"{"$id":"https://appname.org/spec/adjustment/brightness_contrast.v1.json","type":"object","additionalProperties":false,"properties":{"brightness":{"type":"number","minimum":-1,"maximum":1,"default":0},"contrast":{"type":"number","minimum":-1,"maximum":1,"default":0},"legacy":{"type":"boolean","default":false}}}"#
    }

    fn validate(&self, params: &Value) -> Result<(), AdjError> {
        f32_param(params, "brightness", 0.0, -1.0, 1.0)?;
        f32_param(params, "contrast", 0.0, -1.0, 1.0)?;
        Ok(())
    }

    fn compile(&self, params: &Value) -> Result<AdjKernel, AdjError> {
        let b = f32_param(params, "brightness", 0.0, -1.0, 1.0)?;
        let c = f32_param(params, "contrast", 0.0, -1.0, 1.0)?;
        let legacy = params
            .get("legacy")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        Ok(AdjKernel::Lut1D {
            tables: bake_display_lut(bc_display_form(b, c, legacy)),
            closed: display_closed(bc_display_form(b, c, legacy)),
        })
    }
}

// ---------------------------------------------------------------------------
// Levels — adjustment.levels
// ---------------------------------------------------------------------------

pub struct Levels;

/// One §17.2 levels tuple.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LevelsTuple {
    pub in_black: f32,
    pub in_white: f32,
    pub gamma: f32,
    pub out_black: f32,
    pub out_white: f32,
}

impl Default for LevelsTuple {
    fn default() -> LevelsTuple {
        LevelsTuple {
            in_black: 0.0,
            in_white: 1.0,
            gamma: 1.0,
            out_black: 0.0,
            out_white: 1.0,
        }
    }
}

impl LevelsTuple {
    /// The §17.2 formula verbatim.
    #[inline]
    pub fn eval(&self, v: f32) -> f32 {
        let n = ((v - self.in_black) / (self.in_white - self.in_black)).clamp(0.0, 1.0);
        self.out_black + n.powf(1.0 / self.gamma) * (self.out_white - self.out_black)
    }

    fn parse(params: &Value, key: &'static str) -> Result<LevelsTuple, AdjError> {
        let Some(obj) = params.get(key) else {
            return Ok(LevelsTuple::default());
        };
        let g = |name: &'static str, d: f64, lo: f64, hi: f64| f32_param(obj, name, d, lo, hi);
        let t = LevelsTuple {
            in_black: g("inBlack", 0.0, 0.0, 1.0)?,
            in_white: g("inWhite", 1.0, 0.0, 1.0)?,
            gamma: g("gamma", 1.0, 0.1, 10.0)?,
            out_black: g("outBlack", 0.0, 0.0, 1.0)?,
            out_white: g("outWhite", 1.0, 0.0, 1.0)?,
        };
        if t.in_white - t.in_black < 1.0 / 255.0 {
            return Err(AdjError::ParamOutOfRange {
                param: "inWhite",
                detail: "inWhite − inBlack must be ≥ 1/255 (§17.2)".into(),
            });
        }
        Ok(t)
    }
}

/// The §17.2 canonical compile, verbatim (1024 entries).
pub fn levels_lut(t: &LevelsTuple) -> Vec<f32> {
    (0..1024).map(|i| t.eval(i as f32 / 1023.0)).collect()
}

impl Adjustment for Levels {
    fn id(&self) -> &'static str {
        "adjustment.levels"
    }

    fn param_schema(&self) -> &'static str {
        r##"{"$id":"https://appname.org/spec/adjustment/levels.v1.json","type":"object","additionalProperties":false,"properties":{"composite":{"$ref":"#/$defs/tuple"},"r":{"$ref":"#/$defs/tuple"},"g":{"$ref":"#/$defs/tuple"},"b":{"$ref":"#/$defs/tuple"}},"$defs":{"tuple":{"type":"object","additionalProperties":false,"properties":{"inBlack":{"type":"number","minimum":0,"maximum":1,"default":0},"inWhite":{"type":"number","minimum":0,"maximum":1,"default":1},"gamma":{"type":"number","minimum":0.1,"maximum":10,"default":1},"outBlack":{"type":"number","minimum":0,"maximum":1,"default":0},"outWhite":{"type":"number","minimum":0,"maximum":1,"default":1}}}}}"##
    }

    fn validate(&self, params: &Value) -> Result<(), AdjError> {
        for key in ["composite", "r", "g", "b"] {
            LevelsTuple::parse(params, key)?;
        }
        Ok(())
    }

    fn compile(&self, params: &Value) -> Result<AdjKernel, AdjError> {
        let composite = LevelsTuple::parse(params, "composite")?;
        let chans = [
            LevelsTuple::parse(params, "r")?,
            LevelsTuple::parse(params, "g")?,
            LevelsTuple::parse(params, "b")?,
        ];
        // [NORMATIVE] per-channel first, then composite; one LUT each.
        let form = move |v: f32, c: usize| composite.eval(chans[c].eval(v));
        Ok(AdjKernel::Lut1D {
            tables: bake_display_lut(form),
            closed: display_closed(form),
        })
    }
}

// ---------------------------------------------------------------------------
// Curves — adjustment.curves
// ---------------------------------------------------------------------------

pub struct Curves;

fn parse_curve(
    params: &Value,
    key: &'static str,
) -> Result<Option<appname_tools::brush::MonotoneCurve>, AdjError> {
    let Some(arr) = params.get(key) else {
        return Ok(None);
    };
    let pts = arr.as_array().ok_or(AdjError::ParamOutOfRange {
        param: "points",
        detail: "curve is an array of [x, y] points".into(),
    })?;
    if !(2..=16).contains(&pts.len()) {
        return Err(AdjError::ParamOutOfRange {
            param: "points",
            detail: "2–16 points (§17.2)".into(),
        });
    }
    let mut points = Vec::with_capacity(pts.len());
    let mut prev_x = -1.0f32;
    for p in pts {
        let pair = p
            .as_array()
            .filter(|a| a.len() == 2)
            .ok_or(AdjError::ParamOutOfRange {
                param: "points",
                detail: "each point is [x, y]".into(),
            })?;
        let x = pair[0].as_f64().unwrap_or(f64::NAN) as f32;
        let y = pair[1].as_f64().unwrap_or(f64::NAN) as f32;
        if !x.is_finite()
            || !y.is_finite()
            || !(0.0..=1.0).contains(&x)
            || !(0.0..=1.0).contains(&y)
        {
            return Err(AdjError::ParamOutOfRange {
                param: "points",
                detail: "coordinates in [0,1]".into(),
            });
        }
        if prev_x >= 0.0 && x - prev_x < 1.0 / 255.0 {
            return Err(AdjError::ParamOutOfRange {
                param: "points",
                detail: "x strictly increasing, ≥ 1/255 apart (§17.2)".into(),
            });
        }
        prev_x = x;
        points.push([x, y]);
    }
    appname_tools::brush::MonotoneCurve::build(&points)
        .map(Some)
        .map_err(|e| AdjError::ParamOutOfRange {
            param: "points",
            detail: format!("{e}"),
        })
}

impl Adjustment for Curves {
    fn id(&self) -> &'static str {
        "adjustment.curves"
    }

    fn param_schema(&self) -> &'static str {
        r##"{"$id":"https://appname.org/spec/adjustment/curves.v1.json","type":"object","additionalProperties":false,"properties":{"master":{"$ref":"#/$defs/pts"},"r":{"$ref":"#/$defs/pts"},"g":{"$ref":"#/$defs/pts"},"b":{"$ref":"#/$defs/pts"}},"$defs":{"pts":{"type":"array","minItems":2,"maxItems":16,"items":{"type":"array","minItems":2,"maxItems":2,"items":{"type":"number","minimum":0,"maximum":1}}}}}"##
    }

    fn validate(&self, params: &Value) -> Result<(), AdjError> {
        for key in ["master", "r", "g", "b"] {
            parse_curve(params, key)?;
        }
        Ok(())
    }

    fn compile(&self, params: &Value) -> Result<AdjKernel, AdjError> {
        let master = parse_curve(params, "master")?;
        let chans = [
            parse_curve(params, "r")?,
            parse_curve(params, "g")?,
            parse_curve(params, "b")?,
        ];
        // [NORMATIVE] out_c = S_comp(S_c(in_c)); PCHIP never overshoots.
        let form: Arc<dyn Fn(f32, usize) -> f32 + Send + Sync> =
            Arc::new(move |v: f32, c: usize| {
                let v1 = chans[c].as_ref().map_or(v, |s| s.eval(v));
                master.as_ref().map_or(v1, |s| s.eval(v1))
            });
        let baked = bake_display_lut(|v, c| form(v, c));
        let f2 = form.clone();
        Ok(AdjKernel::Lut1D {
            tables: baked,
            closed: display_closed(move |v, c| f2(v, c)),
        })
    }
}

// ---------------------------------------------------------------------------
// Exposure — adjustment.exposure (linear domain)
// ---------------------------------------------------------------------------

pub struct Exposure;

impl Adjustment for Exposure {
    fn id(&self) -> &'static str {
        "adjustment.exposure"
    }

    fn param_schema(&self) -> &'static str {
        r#"{"$id":"https://appname.org/spec/adjustment/exposure.v1.json","type":"object","additionalProperties":false,"properties":{"exposure":{"type":"number","minimum":-20,"maximum":20,"default":0},"offset":{"type":"number","minimum":-0.5,"maximum":0.5,"default":0},"gamma":{"type":"number","minimum":0.01,"maximum":9.99,"default":1}}}"#
    }

    fn validate(&self, params: &Value) -> Result<(), AdjError> {
        f32_param(params, "exposure", 0.0, -20.0, 20.0)?;
        f32_param(params, "offset", 0.0, -0.5, 0.5)?;
        f32_param(params, "gamma", 1.0, 0.01, 9.99)?;
        Ok(())
    }

    fn compile(&self, params: &Value) -> Result<AdjKernel, AdjError> {
        let ev = f32_param(params, "exposure", 0.0, -20.0, 20.0)?;
        let offset = f32_param(params, "offset", 0.0, -0.5, 0.5)?;
        let gamma = f32_param(params, "gamma", 1.0, 0.01, 9.99)?;
        let scale = 2.0f32.powf(ev);
        // Linear domain (§17.2 block): scale → offset → SIGNED power.
        let form = move |v: f32, _c: usize| {
            let x = v * scale + offset;
            x.signum() * x.abs().powf(1.0 / gamma)
        };
        Ok(AdjKernel::Lut1D {
            tables: {
                // Linear-axis tables of the LINEAR form (no display wrap).
                let mut t: [Vec<f32>; 3] = [Vec::new(), Vec::new(), Vec::new()];
                for (c, tab) in t.iter_mut().enumerate() {
                    *tab = (0..1024).map(|i| form(i as f32 / 1023.0, c)).collect();
                }
                t
            },
            closed: Arc::new(form),
        })
    }
}

// ---------------------------------------------------------------------------
// Hue/Saturation — adjustment.hue_saturation (PerPixel, hexcone)
// ---------------------------------------------------------------------------

pub struct HueSaturation;

/// [NORMATIVE §17.2] hexcone RGB→HSL. H in degrees [0, 360).
pub fn rgb_to_hsl(rgb: [f32; 3]) -> (f32, f32, f32) {
    let (r, g, b) = (rgb[0], rgb[1], rgb[2]);
    let mx = r.max(g).max(b);
    let mn = r.min(g).min(b);
    let delta = mx - mn;
    let l = (mx + mn) / 2.0;
    let s = if delta == 0.0 {
        0.0
    } else {
        delta / (1.0 - (2.0 * l - 1.0).abs())
    };
    let h = if delta == 0.0 {
        0.0
    } else if mx == r {
        60.0 * (((g - b) / delta).rem_euclid(6.0))
    } else if mx == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    (h, s, l)
}

/// The exact algebraic inverse (§17.2: chroma + sector table).
pub fn hsl_to_rgb(h: f32, s: f32, l: f32) -> [f32; 3] {
    let h = h.rem_euclid(360.0);
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = l - c / 2.0;
    let (r1, g1, b1) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [r1 + m, g1 + m, b1 + m]
}

#[inline]
fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// §17.2 band membership: 1 inside ±15°, feathered to 0 at ±45°.
#[inline]
pub fn band_membership(h: f32, center: f32) -> f32 {
    let mut d = (h - center).rem_euclid(360.0);
    if d > 180.0 {
        d = 360.0 - d;
    }
    1.0 - smoothstep(15.0, 45.0, d)
}

#[derive(Clone, Copy, Default)]
struct BandAdj {
    hue: f32,
    sat: f32,
    light: f32,
}

fn parse_band(params: &Value, key: &str) -> Result<BandAdj, AdjError> {
    let Some(obj) = params.get(key) else {
        return Ok(BandAdj::default());
    };
    Ok(BandAdj {
        hue: f32_param(obj, "hue", 0.0, -180.0, 180.0)?,
        sat: f32_param(obj, "sat", 0.0, -1.0, 1.0)?,
        light: f32_param(obj, "lightness", 0.0, -1.0, 1.0)?,
    })
}

/// Band centers: R/Y/G/C/B/M at 0/60/120/180/240/300° (§17.2).
const BAND_KEYS: [(&str, f32); 6] = [
    ("r", 0.0),
    ("y", 60.0),
    ("g", 120.0),
    ("c", 180.0),
    ("b", 240.0),
    ("m", 300.0),
];

impl Adjustment for HueSaturation {
    fn id(&self) -> &'static str {
        "adjustment.hue_saturation"
    }

    fn param_schema(&self) -> &'static str {
        r##"{"$id":"https://appname.org/spec/adjustment/hue_saturation.v1.json","type":"object","additionalProperties":false,"properties":{"master":{"$ref":"#/$defs/band"},"r":{"$ref":"#/$defs/band"},"y":{"$ref":"#/$defs/band"},"g":{"$ref":"#/$defs/band"},"c":{"$ref":"#/$defs/band"},"b":{"$ref":"#/$defs/band"},"m":{"$ref":"#/$defs/band"},"colorize":{"type":"boolean","default":false},"colorizeHue":{"type":"number","minimum":0,"maximum":360,"default":0},"colorizeSat":{"type":"number","minimum":0,"maximum":1,"default":0.25},"colorizeLightness":{"type":"number","minimum":-1,"maximum":1,"default":0}},"$defs":{"band":{"type":"object","additionalProperties":false,"properties":{"hue":{"type":"number","minimum":-180,"maximum":180,"default":0},"sat":{"type":"number","minimum":-1,"maximum":1,"default":0},"lightness":{"type":"number","minimum":-1,"maximum":1,"default":0}}}}}"##
    }

    fn validate(&self, params: &Value) -> Result<(), AdjError> {
        parse_band(params, "master")?;
        for (k, _) in BAND_KEYS {
            parse_band(params, k)?;
        }
        f32_param(params, "colorizeHue", 0.0, 0.0, 360.0)?;
        f32_param(params, "colorizeSat", 0.25, 0.0, 1.0)?;
        f32_param(params, "colorizeLightness", 0.0, -1.0, 1.0)?;
        Ok(())
    }

    fn compile(&self, params: &Value) -> Result<AdjKernel, AdjError> {
        let master = parse_band(params, "master")?;
        let mut bands = [BandAdj::default(); 6];
        for (i, (k, _)) in BAND_KEYS.iter().enumerate() {
            bands[i] = parse_band(params, k)?;
        }
        let colorize = params
            .get("colorize")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let cz_hue = f32_param(params, "colorizeHue", 0.0, 0.0, 360.0)?;
        let cz_sat = f32_param(params, "colorizeSat", 0.25, 0.0, 1.0)?;
        let cz_light = f32_param(params, "colorizeLightness", 0.0, -1.0, 1.0)?;
        Ok(AdjKernel::PerPixel(Arc::new(move |s| {
            // Display-referred (block): wrap through the §5 hooks.
            let enc = [
                display_encode(s[0]),
                display_encode(s[1]),
                display_encode(s[2]),
            ];
            let (h, sat, l) = rgb_to_hsl([
                enc[0].clamp(0.0, 1.0),
                enc[1].clamp(0.0, 1.0),
                enc[2].clamp(0.0, 1.0),
            ]);
            let (h2, s2, l2) = if colorize {
                let ell = cz_light + master.light; // colorize replaces H/S; L law below
                let l2 = if ell >= 0.0 {
                    l + ell * (1.0 - l)
                } else {
                    l * (1.0 + ell)
                };
                (cz_hue, cz_sat, l2)
            } else {
                let ms: [f32; 6] = core::array::from_fn(|i| band_membership(h, BAND_KEYS[i].1));
                let mut h2 = h + master.hue;
                let mut s_gain = 1.0 + master.sat;
                let mut ell = master.light;
                for i in 0..6 {
                    h2 += ms[i] * bands[i].hue;
                    s_gain *= 1.0 + ms[i] * bands[i].sat;
                    ell += ms[i] * bands[i].light;
                }
                let s2 = (sat * s_gain).clamp(0.0, 1.0);
                let l2 = if ell >= 0.0 {
                    l + ell * (1.0 - l)
                } else {
                    l * (1.0 + ell)
                };
                (h2, s2, l2)
            };
            let rgb = hsl_to_rgb(h2, s2, l2.clamp(0.0, 1.0));
            [
                display_decode(rgb[0].clamp(0.0, 1.0)),
                display_decode(rgb[1].clamp(0.0, 1.0)),
                display_decode(rgb[2].clamp(0.0, 1.0)),
                s[3],
            ]
        })))
    }
}

// ---------------------------------------------------------------------------
// Invert / Posterize / Threshold
// ---------------------------------------------------------------------------

pub struct Invert;

impl Adjustment for Invert {
    fn id(&self) -> &'static str {
        "adjustment.invert"
    }

    fn param_schema(&self) -> &'static str {
        r#"{"$id":"https://appname.org/spec/adjustment/invert.v1.json","type":"object","additionalProperties":false,"properties":{}}"#
    }

    fn validate(&self, _params: &Value) -> Result<(), AdjError> {
        Ok(())
    }

    fn compile(&self, _params: &Value) -> Result<AdjKernel, AdjError> {
        // Display-referred (§17.2: "inverting linear is not the
        // photographic negative users expect").
        let form = |v: f32, _c: usize| 1.0 - v.clamp(0.0, 1.0);
        Ok(AdjKernel::Lut1D {
            tables: bake_display_lut(form),
            closed: display_closed(form),
        })
    }
}

pub struct Posterize;

impl Adjustment for Posterize {
    fn id(&self) -> &'static str {
        "adjustment.posterize"
    }

    fn param_schema(&self) -> &'static str {
        r#"{"$id":"https://appname.org/spec/adjustment/posterize.v1.json","type":"object","additionalProperties":false,"properties":{"levels":{"type":"integer","minimum":2,"maximum":255,"default":4}}}"#
    }

    fn validate(&self, params: &Value) -> Result<(), AdjError> {
        match params.get("levels") {
            None => Ok(()),
            Some(v) => {
                let l = v.as_i64().unwrap_or(0);
                if !(2..=255).contains(&l) {
                    return Err(AdjError::ParamOutOfRange {
                        param: "levels",
                        detail: format!("{v} outside 2..=255 (§17.2)"),
                    });
                }
                Ok(())
            }
        }
    }

    fn compile(&self, params: &Value) -> Result<AdjKernel, AdjError> {
        self.validate(params)?;
        let levels = params.get("levels").and_then(Value::as_i64).unwrap_or(4) as f32;
        let form =
            move |v: f32, _c: usize| (v.clamp(0.0, 1.0) * (levels - 1.0)).round() / (levels - 1.0);
        Ok(AdjKernel::Lut1D {
            tables: bake_display_lut(form),
            closed: display_closed(form),
        })
    }
}

pub struct Threshold;

impl Adjustment for Threshold {
    fn id(&self) -> &'static str {
        "adjustment.threshold"
    }

    fn param_schema(&self) -> &'static str {
        r#"{"$id":"https://appname.org/spec/adjustment/threshold.v1.json","type":"object","additionalProperties":false,"properties":{"level":{"type":"number","minimum":0,"maximum":1,"default":0.5}}}"#
    }

    fn validate(&self, params: &Value) -> Result<(), AdjError> {
        f32_param(params, "level", 0.5, 0.0, 1.0).map(|_| ())
    }

    fn compile(&self, params: &Value) -> Result<AdjKernel, AdjError> {
        let level = f32_param(params, "level", 0.5, 0.0, 1.0)?;
        Ok(AdjKernel::PerPixel(Arc::new(move |s| {
            // Display-referred luma coupling (§17.2 block).
            let enc = [
                display_encode(s[0]),
                display_encode(s[1]),
                display_encode(s[2]),
            ];
            let v = if lum(enc) >= level { 1.0 } else { 0.0 };
            let lin = display_decode(v);
            [lin, lin, lin, s[3]]
        })))
    }
}

/// The slice-1 adjustments corpus as one deterministic list (sorted by
/// id) — the §12.1 registration set the OpImpl bridge walks (op_bridge).
pub fn slice1_adjustments() -> Vec<Arc<dyn Adjustment>> {
    vec![
        Arc::new(BrightnessContrast),
        Arc::new(Curves),
        Arc::new(Exposure),
        Arc::new(HueSaturation),
        Arc::new(Invert),
        Arc::new(Levels),
        Arc::new(Posterize),
        Arc::new(Threshold),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::{apply_kernel, AdjKernel};
    use appname_core::IRect;
    use appname_tools::resample::{EdgePolicy, Plane};
    use serde_json::json;

    fn one_px(v: [f32; 4]) -> Plane {
        let mut p = Plane::new(IRect::new(0, 0, 1, 1));
        p.set(0, 0, v);
        p
    }

    fn run1(adj: &dyn Adjustment, params: &Value, v: [f32; 4]) -> [f32; 4] {
        let k = adj.compile(params).unwrap();
        let (out, _) = apply_kernel(&one_px(v), &k);
        out.get(0, 0, EdgePolicy::Transparent)
    }

    fn closed_of(k: &AdjKernel) -> impl Fn(f32, usize) -> f32 + '_ {
        move |v, c| match k {
            AdjKernel::Lut1D { closed, .. } => closed(v, c),
            AdjKernel::PerPixel(_) => unreachable!(),
        }
    }

    #[test]
    fn levels_lut_matches_the_canonical_compile() {
        // The spec's own closed form at sampled entries.
        let t = LevelsTuple {
            in_black: 0.1,
            in_white: 0.9,
            gamma: 2.0,
            out_black: 0.05,
            out_white: 0.95,
        };
        let lut = levels_lut(&t);
        assert_eq!(lut.len(), 1024);
        for i in [0usize, 100, 511, 1023] {
            let v = i as f32 / 1023.0;
            let n = ((v - 0.1) / 0.8).clamp(0.0, 1.0);
            let want = 0.05 + n.powf(0.5) * 0.9;
            assert!((lut[i] - want).abs() < 1e-6, "i={i}");
        }
        // Identity params = identity table.
        let id = levels_lut(&LevelsTuple::default());
        for i in (0..1024).step_by(93) {
            assert!((id[i] - i as f32 / 1023.0).abs() < 1e-6);
        }
        // Per-channel-then-composite composition (§17.2 NORMATIVE order).
        let adj = Levels;
        let params = json!({
            "r": {"gamma": 2.0},
            "composite": {"inBlack": 0.5},
        });
        let k = adj.compile(&params).unwrap();
        let f = closed_of(&k);
        let enc = display_encode(0.5);
        let want_enc = {
            let after_r = LevelsTuple {
                gamma: 2.0,
                ..LevelsTuple::default()
            }
            .eval(enc);
            LevelsTuple {
                in_black: 0.5,
                ..LevelsTuple::default()
            }
            .eval(after_r)
        };
        let got = f(0.5, 0);
        assert!(
            (got - display_decode(want_enc)).abs() < 1e-5,
            "composition order"
        );
        // Gap gate.
        assert!(adj
            .validate(&json!({"r": {"inBlack": 0.5, "inWhite": 0.502}}))
            .is_err());
    }

    #[test]
    fn brightness_contrast_legacy_and_default_forms() {
        let adj = BrightnessContrast;
        // Legacy: exact table line in display space.
        let k = adj
            .compile(&json!({"brightness": 0.1, "contrast": 0.5, "legacy": true}))
            .unwrap();
        let f = closed_of(&k);
        let enc = display_encode(0.25);
        let want = ((enc - 0.5) * 1.5 + 0.5 + 0.1).clamp(0.0, 1.0);
        assert!((f(0.25, 0) - display_decode(want)).abs() < 1e-5);
        // Non-legacy: identity at zero params; endpoints fixed for any b/c.
        let out = run1(&adj, &json!({}), [0.3, 0.3, 0.3, 1.0]);
        assert!((out[0] - 0.3).abs() < 2e-3, "identity default {out:?}");
        for (b, c) in [(0.7, 0.9), (-0.8, -0.6), (1.0, 1.0), (-1.0, -1.0)] {
            let k = adj
                .compile(&json!({"brightness": b, "contrast": c}))
                .unwrap();
            let f = closed_of(&k);
            assert!(f(0.0, 0).abs() < 1e-4, "0 fixed at b={b} c={c}");
            assert!((f(1.0, 0) - 1.0).abs() < 1e-4, "1 fixed at b={b} c={c}");
            // Monotone over samples.
            let mut prev = -1.0f32;
            for i in 0..=64 {
                let v = f(i as f32 / 64.0, 0);
                assert!(v >= prev - 1e-4, "monotone at b={b} c={c}");
                prev = v;
            }
        }
        // S and S⁻¹ are inverses to f32 precision.
        for i in 0..=100 {
            let y = i as f32 / 100.0;
            assert!((quintic_s(quintic_s_inv(y)) - y).abs() < 1e-5, "y={y}");
        }
        // Positive contrast raises the upper mids and lowers the lower
        // mids — in DISPLAY space, where the S pivot lives (probe at
        // display 0.75 / 0.25, i.e. linear decode of those).
        let k = adj.compile(&json!({"contrast": 1.0})).unwrap();
        let f = closed_of(&k);
        let hi = display_decode(0.75);
        let lo = display_decode(0.25);
        assert!(f(hi, 0) > hi + 1e-3, "upper mid raised");
        assert!(f(lo, 0) < lo - 1e-3, "lower mid lowered");
    }

    #[test]
    fn curves_compose_and_interpolate_knots() {
        let adj = Curves;
        // Identity default.
        let out = run1(&adj, &json!({}), [0.42, 0.42, 0.42, 1.0]);
        assert!((out[0] - 0.42).abs() < 2e-3);
        // Knot interpolation on the master curve (display domain).
        let k = adj
            .compile(&json!({"master": [[0.0, 0.0], [0.25, 0.21], [0.75, 0.83], [1.0, 1.0]]}))
            .unwrap();
        let f = closed_of(&k);
        assert!((f(display_decode(0.25), 0) - display_decode(0.21)).abs() < 1e-4);
        assert!((f(display_decode(0.75), 1) - display_decode(0.83)).abs() < 1e-4);
        // Channel-then-master order.
        let k = adj
            .compile(&json!({
                "r": [[0.0, 0.0], [0.5, 0.25], [1.0, 1.0]],
                "master": [[0.0, 0.5], [1.0, 1.0]],
            }))
            .unwrap();
        let f = closed_of(&k);
        // enc in 0.5 → r-curve 0.25 → master 0.5 + 0.25·0.5 = 0.625.
        let got = f(display_decode(0.5), 0);
        assert!((got - display_decode(0.625)).abs() < 1e-4, "order: {got}");
        // Spacing gate.
        assert!(adj
            .validate(&json!({"master": [[0.0, 0.0], [0.001, 1.0]]}))
            .is_err());
    }

    #[test]
    fn exposure_is_linear_domain_signed_power() {
        let adj = Exposure;
        // +1 EV doubles linear values (γ = 1, offset 0).
        let out = run1(&adj, &json!({"exposure": 1.0}), [0.2, 0.1, 0.05, 1.0]);
        assert!(
            (out[0] - 0.4).abs() < 2e-3 && (out[1] - 0.2).abs() < 2e-3,
            "{out:?}"
        );
        // Negative drive stays continuous (signed power).
        let k = adj.compile(&json!({"offset": -0.5, "gamma": 2.0})).unwrap();
        let f = closed_of(&k);
        let neg = f(0.25, 0); // v = −0.25 → sign·|v|^(1/2) = −0.5
        assert!((neg + 0.5).abs() < 1e-5, "{neg}");
        // HDR input takes the closed form (no clamp-by-LUT).
        let (out, stats) = apply_kernel(
            &one_px([4.0, 4.0, 4.0, 1.0]),
            &adj.compile(&json!({"exposure": 1.0})).unwrap(),
        );
        assert!((out.get(0, 0, EdgePolicy::Transparent)[0] - 8.0).abs() < 1e-4);
        assert_eq!(stats.fallback_pixels, 1);
    }

    #[test]
    fn hexcone_round_trips_and_bands_partition() {
        // Exact primaries.
        assert_eq!(rgb_to_hsl([1.0, 0.0, 0.0]).0, 0.0);
        assert_eq!(rgb_to_hsl([0.0, 1.0, 0.0]).0, 120.0);
        assert_eq!(rgb_to_hsl([0.0, 0.0, 1.0]).0, 240.0);
        for rgb in [[0.8, 0.2, 0.4], [0.1, 0.9, 0.5], [0.33, 0.33, 0.34]] {
            let (h, s, l) = rgb_to_hsl(rgb);
            let back = hsl_to_rgb(h, s, l);
            for c in 0..3 {
                assert!((back[c] - rgb[c]).abs() < 1e-5, "{rgb:?} → {back:?}");
            }
        }
        // Adjacent memberships sum to 1 where bands meet (30° midpoints).
        for mid in [30.0f32, 90.0, 150.0, 210.0, 270.0, 330.0] {
            let total: f32 = BAND_KEYS
                .iter()
                .map(|(_, c)| band_membership(mid, *c))
                .sum();
            assert!((total - 1.0).abs() < 1e-5, "partition at {mid}°: {total}");
        }
        assert_eq!(band_membership(10.0, 0.0), 1.0, "core ±15°");
        assert_eq!(band_membership(50.0, 0.0), 0.0, "beyond ±45°");
        // Wrapped distance: 350° is 10° from the red center.
        assert_eq!(band_membership(350.0, 0.0), 1.0);
    }

    #[test]
    fn hue_saturation_behaviors() {
        let adj = HueSaturation;
        // Master hue +120° turns red into green (display-space primaries
        // survive the encode/decode wrap exactly at the endpoints).
        let out = run1(
            &adj,
            &json!({"master": {"hue": 120.0}}),
            [1.0, 0.0, 0.0, 1.0],
        );
        assert!(
            out[1] > 0.99 && out[0] < 1e-4 && out[2] < 1e-4,
            "red→green: {out:?}"
        );
        // sat −1 desaturates to the luma axis (S′ = 0).
        let out = run1(
            &adj,
            &json!({"master": {"sat": -1.0}}),
            [0.8, 0.1, 0.3, 1.0],
        );
        assert!(
            (out[0] - out[1]).abs() < 1e-5 && (out[1] - out[2]).abs() < 1e-5,
            "gray: {out:?}"
        );
        // Band-scoped: shifting the BLUE band leaves red untouched.
        let out = run1(&adj, &json!({"b": {"hue": 60.0}}), [1.0, 0.0, 0.0, 1.0]);
        assert!(
            out[0] > 0.99 && out[1] < 1e-4,
            "red outside the blue band: {out:?}"
        );
        // Colorize replaces hue/sat globally.
        let out = run1(
            &adj,
            &json!({"colorize": true, "colorizeHue": 240.0, "colorizeSat": 1.0}),
            [0.5, 0.5, 0.5, 1.0],
        );
        let (h, s, _) = rgb_to_hsl([
            display_encode(out[0]).clamp(0.0, 1.0),
            display_encode(out[1]).clamp(0.0, 1.0),
            display_encode(out[2]).clamp(0.0, 1.0),
        ]);
        assert!(
            (h - 240.0).abs() < 1.0 && s > 0.9,
            "colorized to blue: h={h} s={s}"
        );
        // Lightness law: ℓ = 1 → white; ℓ = −1 → black.
        let out = run1(
            &adj,
            &json!({"master": {"lightness": 1.0}}),
            [0.2, 0.6, 0.4, 1.0],
        );
        assert!(out.iter().take(3).all(|v| *v > 0.99), "ℓ=1 → white");
        let out = run1(
            &adj,
            &json!({"master": {"lightness": -1.0}}),
            [0.2, 0.6, 0.4, 1.0],
        );
        assert!(out.iter().take(3).all(|v| *v < 1e-5), "ℓ=−1 → black");
    }

    #[test]
    fn invert_posterize_threshold() {
        // Invert is a display-space involution (closed form is exact).
        let inv = Invert.compile(&json!({})).unwrap();
        let f = closed_of(&inv);
        for i in 0..=20 {
            let v = i as f32 / 20.0;
            let twice = f(f(v, 0), 0);
            assert!((twice - v).abs() < 1e-5, "involution at {v}");
        }
        // White ↔ black exactly.
        let (out, _) = apply_kernel(&one_px([1.0, 1.0, 1.0, 1.0]), &inv);
        assert!(out.get(0, 0, EdgePolicy::Transparent)[0] < 1e-6);
        // Posterize levels = 2 → {0, 1} in display space; count distinct.
        let post = Posterize.compile(&json!({"levels": 5})).unwrap();
        let f = closed_of(&post);
        let mut seen = std::collections::BTreeSet::new();
        for i in 0..=1000 {
            let v = f(i as f32 / 1000.0, 0);
            seen.insert((display_encode(v) * 10000.0).round() as i64);
        }
        assert_eq!(seen.len(), 5, "5 quantization levels");
        assert!(Posterize.validate(&json!({"levels": 1})).is_err());
        assert!(Posterize.validate(&json!({"levels": 256})).is_err());
        // Threshold at the default 0.5 on encoded luma; all channels binary,
        // alpha untouched.
        let th = Threshold;
        // Inputs are PREMULTIPLIED (the §17.2 evaluation domain): straight
        // 0.1 / 0.8 at α = 0.5 arrive as 0.05 / 0.4.
        let dark = run1(&th, &json!({}), [0.05, 0.05, 0.05, 0.5]);
        assert!(dark[0] < 1e-6 && dark[3] == 0.5);
        let light = run1(&th, &json!({}), [0.4, 0.4, 0.4, 0.5]);
        // Output is re-premultiplied: straight 1.0 at α = 0.5 → 0.5.
        assert!((light[0] - 0.5).abs() < 1e-6 && light[3] == 0.5);
        // Luma coupling: a saturated blue (low luma weight 0.11) stays black
        // at level 0.5 even though B = 1.
        let blue = run1(&th, &json!({}), [0.0, 0.0, 1.0, 1.0]);
        assert!(blue[0] < 1e-6, "0.11 < 0.5: {blue:?}");
    }
}

#[cfg(test)]
mod strictness_tests {
    use super::*;
    use serde_json::{json, Value};

    /// The 2026-08-17 strictness ruling over the LIVE §17.1 registry
    /// (mirror of the filters loop): every adjustment accepts `{}` (the
    /// corpus is optional-with-defaults throughout) and refuses unknown
    /// keys typed — at the TOP level and, where the schema nests strict
    /// objects (levels tuples, hue/sat bands), at DEPTH.
    #[test]
    fn every_registered_adjustment_refuses_unknown_keys() {
        let ops = slice1_adjustments();
        assert_eq!(ops.len(), 8, "the slice-1 corpus");
        for op in &ops {
            assert!(
                op.validate_strict(&json!({})).is_ok(),
                "{}: defaults must pass",
                op.id()
            );
            let err = op
                .validate_strict(&json!({"definitelyNotAParam": 1}))
                .unwrap_err();
            assert!(
                format!("{err}").contains("definitelyNotAParam"),
                "{}: unknown key must reject typed",
                op.id()
            );
        }
    }

    /// Nested strictness at the letter of the registered sub-schemas.
    #[test]
    fn nested_unknown_keys_refuse_at_depth() {
        let ops = slice1_adjustments();
        let by_id = |id: &str| {
            ops.iter()
                .find(|o| o.id() == id)
                .unwrap_or_else(|| panic!("{id} registered"))
        };
        // Levels tuple: unknown nested key.
        let err = by_id("adjustment.levels")
            .validate_strict(&json!({"composite": {"inBlack": 0.1, "bogus": 1}}))
            .unwrap_err();
        assert!(format!("{err}").contains("bogus"));
        // Hue/sat band: unknown nested key.
        let err = by_id("adjustment.hue_saturation")
            .validate_strict(&json!({"master": {"hue": 10, "spin": 1}}))
            .unwrap_err();
        assert!(format!("{err}").contains("spin"));
        // The valid forms still pass.
        assert!(by_id("adjustment.levels")
            .validate_strict(&json!({"composite": {"inBlack": 0.1}}))
            .is_ok());
        assert!(by_id("adjustment.hue_saturation")
            .validate_strict(&json!({"master": {"hue": 10}}))
            .is_ok());
        // Curves: array-of-points shape holds through the items walk.
        assert!(by_id("adjustment.curves")
            .validate_strict(&json!({"master": [[0, 0], [1, 1]]}))
            .is_ok());
        assert!(by_id("adjustment.curves")
            .validate_strict(&json!({"master": [[0, 0], ["x", 1]]}))
            .is_err());
    }

    /// Schema well-formedness self-check (mirror of the filters test):
    /// parses, strict root, every $ref resolves at the ROOT — the
    /// misplaced-$defs bug (levels/curves/hue_saturation carried $defs
    /// INSIDE properties until this slice) stays fixed.
    #[test]
    fn every_registered_schema_is_strict_and_wellformed() {
        fn collect_refs(v: &Value, out: &mut Vec<String>) {
            match v {
                Value::Object(m) => {
                    if let Some(r) = m.get("$ref").and_then(Value::as_str) {
                        out.push(r.to_string());
                    }
                    for x in m.values() {
                        collect_refs(x, out);
                    }
                }
                Value::Array(a) => {
                    for x in a {
                        collect_refs(x, out);
                    }
                }
                _ => {}
            }
        }
        for op in slice1_adjustments() {
            let schema: Value = serde_json::from_str(op.param_schema())
                .unwrap_or_else(|e| panic!("{}: schema unparsable: {e}", op.id()));
            assert_eq!(
                schema.get("additionalProperties"),
                Some(&Value::Bool(false)),
                "{}: strict root",
                op.id()
            );
            assert!(
                schema
                    .get("properties")
                    .and_then(|p| p.get("$defs"))
                    .is_none(),
                "{}: $defs must live at the schema ROOT, not inside properties",
                op.id()
            );
            let mut refs = Vec::new();
            collect_refs(&schema, &mut refs);
            for r in refs {
                let name = r
                    .strip_prefix("#/$defs/")
                    .unwrap_or_else(|| panic!("{}: unsupported $ref form {r}", op.id()));
                assert!(
                    schema.get("$defs").and_then(|d| d.get(name)).is_some(),
                    "{}: $ref {r} must resolve at the schema root",
                    op.id()
                );
            }
        }
    }
}
