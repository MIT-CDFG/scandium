// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Lens profile data model + per-file schema construction.
//!
//! Modeled EXACTLY on what the vendored database corpus contains, fixed
//! by a first-hand survey of `lensfun-database.dtd` / `lensfun-database
//! .xsd` and the 56 profile files at the census pin (nothing invented):
//!
//! - `mount` — localized `name`+ plus a flat `compat` list;
//! - `camera` — `maker`+/`model`+ (localized variants), optional
//!   `variant`, one `mount`, required `cropfactor`, optional
//!   `aspect-ratio`;
//! - `lens` — `maker`+/`model`+, `mount`+, optional `focal`/`aperture`
//!   (min/max range or single value), optional `type` (five optical
//!   types per the XSD enum), required `cropfactor` (DTD-required;
//!   0 of 1561 corpus lenses omit it), optional `aspect-ratio`,
//!   optional `calibration`;
//! - `calibration` — `distortion`* (`ptlens` a/b/c | `poly3` k1 |
//!   `poly5` k1/k2, each at a `focal` breakpoint with optional
//!   `real-focal`), `tca`* (`linear` kr/kb | `poly3` vr/vb/cr/cb/br/bb),
//!   `vignetting`* (`pa` k1/k2/k3 at `focal`/`aperture`/`distance`).
//!
//! Optional-coefficient defaults are the IDENTITY-consistent values,
//! inferred from the corpus's observable omission pattern (entries such
//! as `<distortion model="ptlens" b="…"/>` with a/c absent, `<tca
//! model="poly3" vr="…" vb="…"/>` with c*/b* absent, and explicit
//! `kr="1"` linear entries): scale-like terms (`kr`, `kb`, `vr`, `vb`)
//! default to 1, additive polynomial coefficients (`a`, `b`, `c`, `k1`,
//! `k2`, `cr`, `cb`, `br`, `bb`) default to 0. No lens-correction
//! library was consulted for these semantics (clean-room rule; the LGPL
//! lensfun LIBRARY is out of bounds — only the CC-BY-SA-3.0 DATA and
//! its DTD/XSD were read).
//!
//! Strictness: unknown elements/attributes, missing required fields,
//! non-numeric or non-finite numbers, and out-of-range values (the
//! XSD's positiveFloat fields) produce a typed [`SchemaError`] — the
//! data dir is user-replaceable, so bad input is an `Err`, never a
//! panic. Calibration entry lists are sorted by breakpoint at
//! construction (stable sort, `f64::total_cmp`) so every downstream
//! consumer sees one canonical, deterministic order (D-48).

use super::xml::{Element, Node};

/// Schema-level error: message + byte offset into the source file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemaError {
    /// What is wrong, in plain words.
    pub msg: String,
    /// Byte offset of the offending element/attribute in the file.
    pub offset: usize,
}

impl SchemaError {
    fn new(msg: impl Into<String>, offset: usize) -> Self {
        Self {
            msg: msg.into(),
            offset,
        }
    }
}

/// A possibly-localized name (`lang` attribute per DTD/XSD `MultiName`).
#[derive(Clone, Debug, PartialEq)]
pub struct LocalName {
    /// Language tag, absent on the canonical entry.
    pub lang: Option<String>,
    /// The name text.
    pub text: String,
}

/// Canonical pick from a `MultiName` list: the first lang-less entry,
/// else the first entry (deterministic; corpus files list the canonical
/// lang-less form first).
pub fn canonical(names: &[LocalName]) -> &str {
    names
        .iter()
        .find(|n| n.lang.is_none())
        .or_else(|| names.first())
        .map_or("", |n| n.text.as_str())
}

/// A mount declaration.
#[derive(Clone, Debug, PartialEq)]
pub struct Mount {
    /// Localized names; `canonical` picks the identity name.
    pub names: Vec<LocalName>,
    /// Compatible mount names (flat list as recorded; no closure).
    pub compat: Vec<String>,
}

impl Mount {
    /// Identity name of the mount.
    pub fn name(&self) -> &str {
        canonical(&self.names)
    }
}

/// Aspect ratio, either `W:H` or a decimal (XSD union).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AspectRatio {
    /// `W:H` form (all 365 corpus occurrences).
    Ratio(u32, u32),
    /// Plain positive decimal (XSD-allowed alternative).
    Value(f64),
}

/// A camera body record.
#[derive(Clone, Debug, PartialEq)]
pub struct Camera {
    /// Maker names (localized variants included).
    pub maker_names: Vec<LocalName>,
    /// Model names (localized variants included).
    pub model_names: Vec<LocalName>,
    /// Firmware/regional variant, when recorded.
    pub variant: Option<String>,
    /// Mount name.
    pub mount: String,
    /// Sensor crop factor (positive).
    pub crop_factor: f64,
    /// Sensor aspect ratio, when recorded.
    pub aspect_ratio: Option<AspectRatio>,
}

impl Camera {
    /// Canonical maker name.
    pub fn maker(&self) -> &str {
        canonical(&self.maker_names)
    }
    /// Canonical model name.
    pub fn model(&self) -> &str {
        canonical(&self.model_names)
    }
}

/// `focal`/`aperture` descriptor: a min/max range or a single value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RangeOrValue {
    /// `min="…" max="…"` (attribute order in the file is irrelevant).
    Range {
        /// Lower bound.
        min: f64,
        /// Upper bound.
        max: f64,
    },
    /// `value="…"` (primes).
    Value(f64),
}

/// Lens optical type (XSD `LensOpticalType` enum; all five occur).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LensType {
    /// Ordinary rectilinear projection.
    Rectilinear,
    /// Fisheye projection.
    Fisheye,
    /// Panoramic (cylindrical) projection.
    Panoramic,
    /// Stereographic fisheye.
    Stereographic,
    /// Equisolid-angle fisheye.
    Equisolid,
}

/// Distortion model + coefficients, as recorded in the database.
///
/// Multiplier convention (documented candidate mapping for the engine's
/// corrector; basis: the PanoTools/Brown radial polynomial lineage cited
/// by the census `paper_keys` \[brown66\], NOT any lens-correction
/// library source): each model gives `r_d = r_u * scale(r_u)` with
/// radii normalized per the database's own convention.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DistortionModel {
    /// PanoTools-style cubic: `scale(r) = a·r³ + b·r² + c·r + d`,
    /// `d = 1 − a − b − c` (the published PanoTools normalization).
    Ptlens {
        /// Cubic coefficient.
        a: f64,
        /// Quadratic coefficient.
        b: f64,
        /// Linear coefficient.
        c: f64,
    },
    /// One-coefficient odd cubic: `scale(r) = 1 − k1 + k1·r²` (the
    /// PanoTools single-coefficient case, i.e. `ptlens` with a=c=0,
    /// b=k1).
    Poly3 {
        /// The single coefficient.
        k1: f64,
    },
    /// Brown–Conrady truncation: `scale(r) = 1 + k1·r² + k2·r⁴`
    /// (\[brown66\]; maps 1:1 onto the engine's primary corrector).
    Poly5 {
        /// r² coefficient.
        k1: f64,
        /// r⁴ coefficient.
        k2: f64,
    },
}

/// One distortion calibration breakpoint.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DistortionCal {
    /// Focal length (mm) of the breakpoint.
    pub focal: f64,
    /// Measured real focal length, when recorded.
    pub real_focal: Option<f64>,
    /// Model + coefficients at this breakpoint.
    pub model: DistortionModel,
}

/// Lateral chromatic aberration model (per-channel radial scale, L0 —
/// ALGORITHMS.md §21).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TcaModel {
    /// Constant per-channel scale: `r_red = kr·r`, `r_blue = kb·r`.
    Linear {
        /// Red-channel radial scale.
        kr: f64,
        /// Blue-channel radial scale.
        kb: f64,
    },
    /// Cubic per-channel scale: `r_ch = v·r + c·r² + b·r³` (degree
    /// assignment inferred from the corpus's observable magnitudes —
    /// `v*` clusters at ≈1 (linear scale), `b*` at ≈±1e-5 — and the
    /// DTD attribute ordering; flagged for dossier ratification).
    Poly3 {
        /// Red linear coefficient (default 1).
        vr: f64,
        /// Blue linear coefficient (default 1).
        vb: f64,
        /// Red quadratic coefficient (default 0).
        cr: f64,
        /// Blue quadratic coefficient (default 0).
        cb: f64,
        /// Red cubic coefficient (default 0).
        br: f64,
        /// Blue cubic coefficient (default 0).
        bb: f64,
    },
}

/// One TCA calibration breakpoint.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TcaCal {
    /// Focal length (mm) of the breakpoint.
    pub focal: f64,
    /// Model + coefficients at this breakpoint.
    pub model: TcaModel,
}

/// One vignetting calibration breakpoint (`pa` model — even radial
/// polynomial, cos⁴-lineage per ALGORITHMS.md §21:
/// `falloff(r) = 1 + k1·r² + k2·r⁴ + k3·r⁶`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VignettingCal {
    /// Focal length (mm).
    pub focal: f64,
    /// Aperture (f-number).
    pub aperture: f64,
    /// Subject distance (m).
    pub distance: f64,
    /// r² coefficient.
    pub k1: f64,
    /// r⁴ coefficient.
    pub k2: f64,
    /// r⁶ coefficient.
    pub k3: f64,
}

/// A lens's calibration block, each list sorted by breakpoint.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Calibration {
    /// Distortion breakpoints, sorted by focal (stable).
    pub distortion: Vec<DistortionCal>,
    /// TCA breakpoints, sorted by focal (stable).
    pub tca: Vec<TcaCal>,
    /// Vignetting breakpoints, sorted by (focal, aperture, distance).
    pub vignetting: Vec<VignettingCal>,
}

/// A lens record.
#[derive(Clone, Debug, PartialEq)]
pub struct Lens {
    /// Maker names (localized variants included).
    pub maker_names: Vec<LocalName>,
    /// Model names (localized variants included).
    pub model_names: Vec<LocalName>,
    /// Mount names this lens fits (one or more).
    pub mounts: Vec<String>,
    /// Focal range/value, when recorded.
    pub focal: Option<RangeOrValue>,
    /// Aperture range/value, when recorded.
    pub aperture: Option<RangeOrValue>,
    /// Optical type, when recorded (absent ⇒ rectilinear by corpus
    /// convention — 40 of 1561 lenses carry the element).
    pub lens_type: Option<LensType>,
    /// Crop factor of the calibration sensor (positive).
    pub crop_factor: f64,
    /// Aspect ratio, when recorded.
    pub aspect_ratio: Option<AspectRatio>,
    /// Calibration block, when recorded.
    pub calibration: Option<Calibration>,
}

impl Lens {
    /// Canonical maker name.
    pub fn maker(&self) -> &str {
        canonical(&self.maker_names)
    }
    /// Canonical model name.
    pub fn model(&self) -> &str {
        canonical(&self.model_names)
    }
}

/// One parsed database file (pre-merge; the reader merges files).
#[derive(Clone, Debug, PartialEq)]
pub struct DbFile {
    /// Root `version` attribute (corpus: 2 everywhere).
    pub version: u32,
    /// Mount declarations in file order.
    pub mounts: Vec<Mount>,
    /// Camera records in file order.
    pub cameras: Vec<Camera>,
    /// Lens records in file order.
    pub lenses: Vec<Lens>,
}

// ---------------------------------------------------------------------
// Schema construction helpers
// ---------------------------------------------------------------------

/// Non-whitespace direct text content is an error in structural
/// elements (DTD mixed-content allowance is unused in the corpus).
fn reject_stray_text(e: &Element<'_>) -> Result<(), SchemaError> {
    for n in &e.children {
        if let Node::Text(t) = n {
            if !t.trim().is_empty() {
                return Err(SchemaError::new(
                    format!("unexpected text content in <{}>", e.name),
                    e.offset,
                ));
            }
        }
    }
    Ok(())
}

fn reject_unknown_attrs(e: &Element<'_>, allowed: &[&str]) -> Result<(), SchemaError> {
    for a in &e.attrs {
        if !allowed.contains(&a.name) {
            return Err(SchemaError::new(
                format!("unknown attribute {:?} on <{}>", a.name, e.name),
                a.offset,
            ));
        }
    }
    Ok(())
}

fn parse_f64(e: &Element<'_>, what: &str, raw: &str, offset: usize) -> Result<f64, SchemaError> {
    let v: f64 = raw.trim().parse().map_err(|_| {
        SchemaError::new(
            format!("{what} on <{}> is not a number: {raw:?}", e.name),
            offset,
        )
    })?;
    if !v.is_finite() {
        return Err(SchemaError::new(
            format!("{what} on <{}> is not finite: {raw:?}", e.name),
            offset,
        ));
    }
    Ok(v)
}

/// Required positive float attribute (XSD positiveFloat).
fn req_pos_attr(e: &Element<'_>, name: &str) -> Result<f64, SchemaError> {
    let raw = e.attr(name).ok_or_else(|| {
        SchemaError::new(
            format!("<{}> is missing required attribute {name:?}", e.name),
            e.offset,
        )
    })?;
    let v = parse_f64(e, name, raw, e.offset)?;
    if v <= 0.0 {
        return Err(SchemaError::new(
            format!("{name} on <{}> must be positive, got {raw:?}", e.name),
            e.offset,
        ));
    }
    Ok(v)
}

/// Optional float attribute (any finite value) with a default.
fn opt_attr_or(e: &Element<'_>, name: &str, default: f64) -> Result<f64, SchemaError> {
    match e.attr(name) {
        Some(raw) => parse_f64(e, name, raw, e.offset),
        None => Ok(default),
    }
}

/// Required float attribute (any finite value).
fn req_attr(e: &Element<'_>, name: &str) -> Result<f64, SchemaError> {
    let raw = e.attr(name).ok_or_else(|| {
        SchemaError::new(
            format!("<{}> is missing required attribute {name:?}", e.name),
            e.offset,
        )
    })?;
    parse_f64(e, name, raw, e.offset)
}

/// Text-only child (e.g. `<compat>`, `<mount>` inside camera/lens).
fn text_only(e: &Element<'_>) -> Result<String, SchemaError> {
    if e.child_elements().next().is_some() {
        return Err(SchemaError::new(
            format!("<{}> must contain text only", e.name),
            e.offset,
        ));
    }
    let t = e.text();
    if t.is_empty() {
        return Err(SchemaError::new(
            format!("<{}> must not be empty", e.name),
            e.offset,
        ));
    }
    Ok(t)
}

fn local_name(e: &Element<'_>) -> Result<LocalName, SchemaError> {
    reject_unknown_attrs(e, &["lang"])?;
    Ok(LocalName {
        lang: e.attr("lang").map(str::to_owned),
        text: text_only(e)?,
    })
}

fn aspect_ratio(e: &Element<'_>) -> Result<AspectRatio, SchemaError> {
    reject_unknown_attrs(e, &[])?;
    let t = text_only(e)?;
    if let Some((w, h)) = t.split_once(':') {
        let w: u32 = w
            .trim()
            .parse()
            .map_err(|_| SchemaError::new(format!("bad aspect-ratio {t:?}"), e.offset))?;
        let h: u32 = h
            .trim()
            .parse()
            .map_err(|_| SchemaError::new(format!("bad aspect-ratio {t:?}"), e.offset))?;
        if w == 0 || h == 0 {
            return Err(SchemaError::new(
                format!("aspect-ratio terms must be positive: {t:?}"),
                e.offset,
            ));
        }
        return Ok(AspectRatio::Ratio(w, h));
    }
    let v = parse_f64(e, "aspect-ratio", &t, e.offset)?;
    if v <= 0.0 {
        return Err(SchemaError::new(
            format!("aspect-ratio must be positive: {t:?}"),
            e.offset,
        ));
    }
    Ok(AspectRatio::Value(v))
}

fn range_or_value(e: &Element<'_>) -> Result<RangeOrValue, SchemaError> {
    reject_unknown_attrs(e, &["min", "max", "value"])?;
    reject_stray_text(e)?;
    if e.child_elements().next().is_some() {
        return Err(SchemaError::new(
            format!("<{}> must be empty", e.name),
            e.offset,
        ));
    }
    match (e.attr("min"), e.attr("max"), e.attr("value")) {
        (Some(_), Some(_), None) => Ok(RangeOrValue::Range {
            min: req_pos_attr(e, "min")?,
            max: req_pos_attr(e, "max")?,
        }),
        (None, None, Some(_)) => Ok(RangeOrValue::Value(req_pos_attr(e, "value")?)),
        _ => Err(SchemaError::new(
            format!("<{}> needs either min+max or value (corpus forms)", e.name),
            e.offset,
        )),
    }
}

fn mount_decl(e: &Element<'_>) -> Result<Mount, SchemaError> {
    reject_unknown_attrs(e, &[])?;
    reject_stray_text(e)?;
    let mut names = Vec::new();
    let mut compat = Vec::new();
    for c in e.child_elements() {
        match c.name {
            "name" => names.push(local_name(c)?),
            "compat" => {
                reject_unknown_attrs(c, &[])?;
                compat.push(text_only(c)?);
            }
            other => {
                return Err(SchemaError::new(
                    format!("unknown element <{other}> in <mount>"),
                    c.offset,
                ))
            }
        }
    }
    if names.is_empty() {
        return Err(SchemaError::new("<mount> has no <name>", e.offset));
    }
    Ok(Mount { names, compat })
}

fn camera(e: &Element<'_>) -> Result<Camera, SchemaError> {
    reject_unknown_attrs(e, &[])?;
    reject_stray_text(e)?;
    let mut maker_names = Vec::new();
    let mut model_names = Vec::new();
    let mut variant = None;
    let mut mount = None;
    let mut crop_factor = None;
    let mut ar = None;
    for c in e.child_elements() {
        match c.name {
            "maker" => maker_names.push(local_name(c)?),
            "model" => model_names.push(local_name(c)?),
            "variant" => {
                reject_unknown_attrs(c, &[])?;
                variant = Some(text_only(c)?);
            }
            "mount" => {
                reject_unknown_attrs(c, &[])?;
                mount = Some(text_only(c)?);
            }
            "cropfactor" => {
                reject_unknown_attrs(c, &[])?;
                let t = text_only(c)?;
                let v = parse_f64(c, "cropfactor", &t, c.offset)?;
                if v <= 0.0 {
                    return Err(SchemaError::new("cropfactor must be positive", c.offset));
                }
                crop_factor = Some(v);
            }
            "aspect-ratio" => ar = Some(aspect_ratio(c)?),
            other => {
                return Err(SchemaError::new(
                    format!("unknown element <{other}> in <camera>"),
                    c.offset,
                ))
            }
        }
    }
    if maker_names.is_empty() {
        return Err(SchemaError::new("<camera> has no <maker>", e.offset));
    }
    if model_names.is_empty() {
        return Err(SchemaError::new("<camera> has no <model>", e.offset));
    }
    Ok(Camera {
        maker_names,
        model_names,
        variant,
        mount: mount.ok_or_else(|| SchemaError::new("<camera> has no <mount>", e.offset))?,
        crop_factor: crop_factor
            .ok_or_else(|| SchemaError::new("<camera> has no <cropfactor>", e.offset))?,
        aspect_ratio: ar,
    })
}

fn distortion(e: &Element<'_>) -> Result<DistortionCal, SchemaError> {
    reject_stray_text(e)?;
    let model_name = e
        .attr("model")
        .ok_or_else(|| SchemaError::new("<distortion> is missing \"model\"", e.offset))?;
    let model = match model_name {
        "ptlens" => {
            reject_unknown_attrs(e, &["model", "focal", "real-focal", "a", "b", "c"])?;
            DistortionModel::Ptlens {
                a: opt_attr_or(e, "a", 0.0)?,
                b: opt_attr_or(e, "b", 0.0)?,
                c: opt_attr_or(e, "c", 0.0)?,
            }
        }
        "poly3" => {
            reject_unknown_attrs(e, &["model", "focal", "real-focal", "k1"])?;
            DistortionModel::Poly3 {
                k1: opt_attr_or(e, "k1", 0.0)?,
            }
        }
        "poly5" => {
            reject_unknown_attrs(e, &["model", "focal", "real-focal", "k1", "k2"])?;
            DistortionModel::Poly5 {
                k1: opt_attr_or(e, "k1", 0.0)?,
                k2: opt_attr_or(e, "k2", 0.0)?,
            }
        }
        other => {
            return Err(SchemaError::new(
                format!("unknown distortion model {other:?}"),
                e.offset,
            ))
        }
    };
    let real_focal = match e.attr("real-focal") {
        Some(_) => Some(req_pos_attr(e, "real-focal")?),
        None => None,
    };
    Ok(DistortionCal {
        focal: req_pos_attr(e, "focal")?,
        real_focal,
        model,
    })
}

fn tca(e: &Element<'_>) -> Result<TcaCal, SchemaError> {
    reject_stray_text(e)?;
    let model_name = e
        .attr("model")
        .ok_or_else(|| SchemaError::new("<tca> is missing \"model\"", e.offset))?;
    let model = match model_name {
        "linear" => {
            reject_unknown_attrs(e, &["model", "focal", "kr", "kb"])?;
            TcaModel::Linear {
                kr: opt_attr_or(e, "kr", 1.0)?,
                kb: opt_attr_or(e, "kb", 1.0)?,
            }
        }
        "poly3" => {
            reject_unknown_attrs(e, &["model", "focal", "vr", "vb", "cr", "cb", "br", "bb"])?;
            TcaModel::Poly3 {
                vr: opt_attr_or(e, "vr", 1.0)?,
                vb: opt_attr_or(e, "vb", 1.0)?,
                cr: opt_attr_or(e, "cr", 0.0)?,
                cb: opt_attr_or(e, "cb", 0.0)?,
                br: opt_attr_or(e, "br", 0.0)?,
                bb: opt_attr_or(e, "bb", 0.0)?,
            }
        }
        other => {
            return Err(SchemaError::new(
                format!("unknown tca model {other:?}"),
                e.offset,
            ))
        }
    };
    Ok(TcaCal {
        focal: req_pos_attr(e, "focal")?,
        model,
    })
}

fn vignetting(e: &Element<'_>) -> Result<VignettingCal, SchemaError> {
    reject_stray_text(e)?;
    let model_name = e
        .attr("model")
        .ok_or_else(|| SchemaError::new("<vignetting> is missing \"model\"", e.offset))?;
    if model_name != "pa" {
        return Err(SchemaError::new(
            format!("unknown vignetting model {model_name:?}"),
            e.offset,
        ));
    }
    reject_unknown_attrs(
        e,
        &["model", "focal", "aperture", "distance", "k1", "k2", "k3"],
    )?;
    Ok(VignettingCal {
        focal: req_pos_attr(e, "focal")?,
        aperture: req_pos_attr(e, "aperture")?,
        distance: req_pos_attr(e, "distance")?,
        k1: req_attr(e, "k1")?,
        k2: req_attr(e, "k2")?,
        k3: req_attr(e, "k3")?,
    })
}

fn calibration(e: &Element<'_>) -> Result<Calibration, SchemaError> {
    reject_unknown_attrs(e, &[])?;
    reject_stray_text(e)?;
    let mut cal = Calibration::default();
    for c in e.child_elements() {
        match c.name {
            "distortion" => cal.distortion.push(distortion(c)?),
            "tca" => cal.tca.push(tca(c)?),
            "vignetting" => cal.vignetting.push(vignetting(c)?),
            other => {
                return Err(SchemaError::new(
                    format!("unknown element <{other}> in <calibration>"),
                    c.offset,
                ))
            }
        }
    }
    // Canonical deterministic order: stable sort by breakpoint keys.
    cal.distortion.sort_by(|x, y| x.focal.total_cmp(&y.focal));
    cal.tca.sort_by(|x, y| x.focal.total_cmp(&y.focal));
    cal.vignetting.sort_by(|x, y| {
        x.focal
            .total_cmp(&y.focal)
            .then(x.aperture.total_cmp(&y.aperture))
            .then(x.distance.total_cmp(&y.distance))
    });
    Ok(cal)
}

fn lens(e: &Element<'_>) -> Result<Lens, SchemaError> {
    reject_unknown_attrs(e, &[])?;
    reject_stray_text(e)?;
    let mut maker_names = Vec::new();
    let mut model_names = Vec::new();
    let mut mounts = Vec::new();
    let mut focal = None;
    let mut aperture = None;
    let mut lens_type = None;
    let mut crop_factor = None;
    let mut ar = None;
    let mut cal = None;
    for c in e.child_elements() {
        match c.name {
            "maker" => maker_names.push(local_name(c)?),
            "model" => model_names.push(local_name(c)?),
            "mount" => {
                reject_unknown_attrs(c, &[])?;
                mounts.push(text_only(c)?);
            }
            "focal" => focal = Some(range_or_value(c)?),
            "aperture" => aperture = Some(range_or_value(c)?),
            "type" => {
                reject_unknown_attrs(c, &[])?;
                lens_type = Some(match text_only(c)?.as_str() {
                    "rectilinear" => LensType::Rectilinear,
                    "fisheye" => LensType::Fisheye,
                    "panoramic" => LensType::Panoramic,
                    "stereographic" => LensType::Stereographic,
                    "equisolid" => LensType::Equisolid,
                    other => {
                        return Err(SchemaError::new(
                            format!("unknown lens type {other:?}"),
                            c.offset,
                        ))
                    }
                });
            }
            "cropfactor" => {
                reject_unknown_attrs(c, &[])?;
                let t = text_only(c)?;
                let v = parse_f64(c, "cropfactor", &t, c.offset)?;
                if v <= 0.0 {
                    return Err(SchemaError::new("cropfactor must be positive", c.offset));
                }
                crop_factor = Some(v);
            }
            "aspect-ratio" => ar = Some(aspect_ratio(c)?),
            "calibration" => cal = Some(calibration(c)?),
            other => {
                return Err(SchemaError::new(
                    format!("unknown element <{other}> in <lens>"),
                    c.offset,
                ))
            }
        }
    }
    if maker_names.is_empty() {
        return Err(SchemaError::new("<lens> has no <maker>", e.offset));
    }
    if model_names.is_empty() {
        return Err(SchemaError::new("<lens> has no <model>", e.offset));
    }
    if mounts.is_empty() {
        return Err(SchemaError::new("<lens> has no <mount>", e.offset));
    }
    Ok(Lens {
        maker_names,
        model_names,
        mounts,
        focal,
        aperture,
        lens_type,
        crop_factor: crop_factor
            .ok_or_else(|| SchemaError::new("<lens> has no <cropfactor>", e.offset))?,
        aspect_ratio: ar,
        calibration: cal,
    })
}

/// Build a [`DbFile`] from a parsed document root.
pub fn db_file_from_root(root: &Element<'_>) -> Result<DbFile, SchemaError> {
    if root.name != "lensdatabase" {
        return Err(SchemaError::new(
            format!("root element is <{}>, expected <lensdatabase>", root.name),
            root.offset,
        ));
    }
    reject_unknown_attrs(root, &["version"])?;
    reject_stray_text(root)?;
    let vraw = root
        .attr("version")
        .ok_or_else(|| SchemaError::new("<lensdatabase> is missing \"version\"", root.offset))?;
    let version: u32 = vraw
        .trim()
        .parse()
        .map_err(|_| SchemaError::new(format!("bad database version {vraw:?}"), root.offset))?;
    let mut out = DbFile {
        version,
        mounts: Vec::new(),
        cameras: Vec::new(),
        lenses: Vec::new(),
    };
    for c in root.child_elements() {
        match c.name {
            "mount" => out.mounts.push(mount_decl(c)?),
            "camera" => out.cameras.push(camera(c)?),
            "lens" => out.lenses.push(lens(c)?),
            other => {
                return Err(SchemaError::new(
                    format!("unknown element <{other}> in <lensdatabase>"),
                    c.offset,
                ))
            }
        }
    }
    Ok(out)
}

/// A synthetic fixture modeled on the real corpus shapes, covering
/// every calibration model and the localized-name pattern (shared with
/// the reader/lookup test suites).
#[cfg(test)]
pub(crate) const FULL_FIXTURE: &str = "\
<!DOCTYPE lensdatabase SYSTEM \"lensfun-database.dtd\">
<lensdatabase version=\"2\">
    <mount>
        <name>Example EF</name>
        <name lang=\"ru\">Пример EF</name>
        <compat>M42</compat>
        <compat>Generic</compat>
    </mount>
    <camera>
        <maker>Examplecorp</maker>
        <maker lang=\"en\">Example</maker>
        <model>Examplecorp Snapper 5D</model>
        <model lang=\"en\">Snapper 5D</model>
        <variant>mk II</variant>
        <mount>Example EF</mount>
        <cropfactor>1.6</cropfactor>
        <aspect-ratio>3:2</aspect-ratio>
    </camera>
    <lens>
        <maker>Examplecorp</maker>
        <model>Examplecorp Wide 10-22mm f/3.5-4.5</model>
        <mount>Example EF</mount>
        <focal min=\"10\" max=\"22\"/>
        <aperture min=\"3.5\" max=\"22\"/>
        <type>rectilinear</type>
        <cropfactor>1.6</cropfactor>
        <aspect-ratio>1.5</aspect-ratio>
        <calibration>
            <!-- breakpoints deliberately out of order: sorted on parse -->
            <distortion model=\"ptlens\" focal=\"22\" b=\"-0.004\"/>
            <distortion model=\"ptlens\" focal=\"10\" a=\"0.02\" b=\"-0.068\" c=\"0.051\"/>
            <tca model=\"poly3\" focal=\"10\" vr=\"1.0003\" vb=\"1.0001\" br=\"-0.0000025\" bb=\"0.0000125\"/>
            <tca model=\"poly3\" focal=\"22\" vr=\"1.0001\" vb=\"1.0002\"/>
            <vignetting model=\"pa\" focal=\"10\" aperture=\"3.5\" distance=\"1000\" k1=\"-2.1\" k2=\"2.6\" k3=\"-1.2\"/>
            <vignetting model=\"pa\" focal=\"10\" aperture=\"3.5\" distance=\"10\" k1=\"-2.1\" k2=\"2.6\" k3=\"-1.2\"/>
        </calibration>
    </lens>
    <lens>
        <maker>Examplecorp</maker>
        <model>Examplecorp Prime 50mm f/1.8</model>
        <mount>Example EF</mount>
        <mount>M42</mount>
        <focal value=\"50\"/>
        <aperture value=\"1.8\"/>
        <cropfactor>1.0</cropfactor>
        <calibration>
            <distortion model=\"poly3\" focal=\"50\" k1=\"-0.01\"/>
            <distortion model=\"poly5\" focal=\"55\" k1=\"-0.01\" k2=\"0.002\"/>
            <tca model=\"linear\" focal=\"50\" kr=\"1.0004\" kb=\"1.0002\"/>
        </calibration>
    </lens>
    <lens>
        <maker>Examplecorp</maker>
        <model>Examplecorp Fish 8mm f/3.5</model>
        <mount>Example EF</mount>
        <type>fisheye</type>
        <cropfactor>1.6</cropfactor>
        <calibration>
            <distortion model=\"poly3\" focal=\"8\" real-focal=\"7.8\"/>
        </calibration>
    </lens>
</lensdatabase>
";

#[cfg(test)]
mod tests {
    use super::super::xml::parse_document;
    use super::*;

    fn file(doc: &str) -> DbFile {
        db_file_from_root(&parse_document(doc).expect("xml")).expect("schema")
    }

    fn schema_err(doc: &str) -> SchemaError {
        db_file_from_root(&parse_document(doc).expect("xml")).expect_err("expected schema error")
    }

    #[test]
    fn full_fixture_round_trips_every_model() {
        let f = file(FULL_FIXTURE);
        assert_eq!(f.version, 2);
        assert_eq!(f.mounts.len(), 1);
        assert_eq!(f.cameras.len(), 1);
        assert_eq!(f.lenses.len(), 3);

        let m = &f.mounts[0];
        assert_eq!(m.name(), "Example EF");
        assert_eq!(m.names[1].lang.as_deref(), Some("ru"));
        assert_eq!(m.compat, vec!["M42", "Generic"]);

        let c = &f.cameras[0];
        assert_eq!(c.maker(), "Examplecorp");
        assert_eq!(c.model(), "Examplecorp Snapper 5D");
        assert_eq!(c.variant.as_deref(), Some("mk II"));
        assert_eq!(c.mount, "Example EF");
        assert_eq!(c.crop_factor, 1.6);
        assert_eq!(c.aspect_ratio, Some(AspectRatio::Ratio(3, 2)));

        let wide = &f.lenses[0];
        assert_eq!(wide.model(), "Examplecorp Wide 10-22mm f/3.5-4.5");
        assert_eq!(wide.mounts, vec!["Example EF"]);
        assert_eq!(
            wide.focal,
            Some(RangeOrValue::Range {
                min: 10.0,
                max: 22.0
            })
        );
        assert_eq!(wide.lens_type, Some(LensType::Rectilinear));
        assert_eq!(wide.aspect_ratio, Some(AspectRatio::Value(1.5)));
        let cal = wide.calibration.as_ref().expect("calibration");
        // Sorted by focal despite file order 22, 10.
        assert_eq!(cal.distortion[0].focal, 10.0);
        assert_eq!(
            cal.distortion[0].model,
            DistortionModel::Ptlens {
                a: 0.02,
                b: -0.068,
                c: 0.051
            }
        );
        // Omitted ptlens a/c default to 0.
        assert_eq!(
            cal.distortion[1].model,
            DistortionModel::Ptlens {
                a: 0.0,
                b: -0.004,
                c: 0.0
            }
        );
        // Omitted tca poly3 c*/b* default to 0 (second entry).
        assert_eq!(
            cal.tca[1].model,
            TcaModel::Poly3 {
                vr: 1.0001,
                vb: 1.0002,
                cr: 0.0,
                cb: 0.0,
                br: 0.0,
                bb: 0.0
            }
        );
        // Vignetting sorted by (focal, aperture, distance): 10 before 1000.
        assert_eq!(cal.vignetting[0].distance, 10.0);

        let prime = &f.lenses[1];
        assert_eq!(prime.mounts, vec!["Example EF", "M42"]);
        assert_eq!(prime.focal, Some(RangeOrValue::Value(50.0)));
        let pcal = prime.calibration.as_ref().expect("calibration");
        assert_eq!(
            pcal.distortion[0].model,
            DistortionModel::Poly3 { k1: -0.01 }
        );
        assert_eq!(
            pcal.distortion[1].model,
            DistortionModel::Poly5 {
                k1: -0.01,
                k2: 0.002
            }
        );
        assert_eq!(
            pcal.tca[0].model,
            TcaModel::Linear {
                kr: 1.0004,
                kb: 1.0002
            }
        );

        let fish = &f.lenses[2];
        assert_eq!(fish.lens_type, Some(LensType::Fisheye));
        let fcal = fish.calibration.as_ref().expect("calibration");
        assert_eq!(fcal.distortion[0].real_focal, Some(7.8));
        // Omitted poly3 k1 defaults to 0 (identity).
        assert_eq!(fcal.distortion[0].model, DistortionModel::Poly3 { k1: 0.0 });
    }

    #[test]
    fn wrong_root_rejected() {
        let e = schema_err("<notlensdb version=\"2\"/>");
        assert!(e.msg.contains("expected <lensdatabase>"), "{}", e.msg);
    }

    #[test]
    fn missing_version_rejected() {
        let e = schema_err("<lensdatabase/>");
        assert!(e.msg.contains("version"), "{}", e.msg);
    }

    #[test]
    fn unknown_element_rejected_with_position() {
        let doc = "<lensdatabase version=\"2\">\n  <gadget/>\n</lensdatabase>";
        let e = schema_err(doc);
        assert!(e.msg.contains("<gadget>"), "{}", e.msg);
        assert_eq!(&doc[e.offset..e.offset + 8], "<gadget/");
    }

    #[test]
    fn unknown_attribute_rejected() {
        let e = schema_err(
            "<lensdatabase version=\"2\"><mount weird=\"1\"><name>X</name></mount></lensdatabase>",
        );
        assert!(e.msg.contains("weird"), "{}", e.msg);
    }

    #[test]
    fn camera_missing_cropfactor_rejected() {
        let e = schema_err(
            "<lensdatabase version=\"2\"><camera><maker>A</maker><model>B</model>\
             <mount>M</mount></camera></lensdatabase>",
        );
        assert!(e.msg.contains("cropfactor"), "{}", e.msg);
    }

    #[test]
    fn lens_missing_cropfactor_rejected() {
        let e = schema_err(
            "<lensdatabase version=\"2\"><lens><maker>A</maker><model>B</model>\
             <mount>M</mount></lens></lensdatabase>",
        );
        assert!(e.msg.contains("cropfactor"), "{}", e.msg);
    }

    #[test]
    fn non_numeric_coefficient_rejected() {
        let e = schema_err(
            "<lensdatabase version=\"2\"><lens><maker>A</maker><model>B</model>\
             <mount>M</mount><cropfactor>1</cropfactor><calibration>\
             <distortion model=\"poly3\" focal=\"50\" k1=\"NaN-ish\"/>\
             </calibration></lens></lensdatabase>",
        );
        assert!(e.msg.contains("not a number"), "{}", e.msg);
    }

    #[test]
    fn non_finite_coefficient_rejected() {
        let e = schema_err(
            "<lensdatabase version=\"2\"><lens><maker>A</maker><model>B</model>\
             <mount>M</mount><cropfactor>1</cropfactor><calibration>\
             <distortion model=\"poly3\" focal=\"50\" k1=\"inf\"/>\
             </calibration></lens></lensdatabase>",
        );
        assert!(e.msg.contains("not finite"), "{}", e.msg);
    }

    #[test]
    fn negative_focal_rejected() {
        let e = schema_err(
            "<lensdatabase version=\"2\"><lens><maker>A</maker><model>B</model>\
             <mount>M</mount><cropfactor>1</cropfactor><calibration>\
             <tca model=\"linear\" focal=\"-3\"/>\
             </calibration></lens></lensdatabase>",
        );
        assert!(e.msg.contains("positive"), "{}", e.msg);
    }

    #[test]
    fn min_without_max_rejected() {
        let e = schema_err(
            "<lensdatabase version=\"2\"><lens><maker>A</maker><model>B</model>\
             <mount>M</mount><focal min=\"10\"/><cropfactor>1</cropfactor>\
             </lens></lensdatabase>",
        );
        assert!(e.msg.contains("min+max or value"), "{}", e.msg);
    }

    #[test]
    fn unknown_calibration_model_rejected() {
        let e = schema_err(
            "<lensdatabase version=\"2\"><lens><maker>A</maker><model>B</model>\
             <mount>M</mount><cropfactor>1</cropfactor><calibration>\
             <distortion model=\"warp9\" focal=\"50\"/>\
             </calibration></lens></lensdatabase>",
        );
        assert!(e.msg.contains("warp9"), "{}", e.msg);
    }

    #[test]
    fn unknown_lens_type_rejected() {
        let e = schema_err(
            "<lensdatabase version=\"2\"><lens><maker>A</maker><model>B</model>\
             <mount>M</mount><type>orthographic</type><cropfactor>1</cropfactor>\
             </lens></lensdatabase>",
        );
        assert!(e.msg.contains("orthographic"), "{}", e.msg);
    }

    #[test]
    fn stray_text_in_structural_element_rejected() {
        let e = schema_err("<lensdatabase version=\"2\">stray</lensdatabase>");
        assert!(e.msg.contains("unexpected text"), "{}", e.msg);
    }

    // ------------------------------------------------------------------
    // Tester round (2026-08-17): non-finite DATA-side probes + range
    // contract pins.
    // ------------------------------------------------------------------

    /// Every f64 ingestion surface must reject non-finite DATA values
    /// (`inf`/`nan`/overflow-to-inf literals Rust's f64 parser accepts),
    /// so data-side non-finites can never re-introduce the sort/compare
    /// hazards the fix round closed on the QUERY side.
    #[test]
    fn non_finite_data_values_rejected_on_every_f64_surface() {
        let lens_wrap = |inner: &str| {
            format!(
                "<lensdatabase version=\"2\"><lens><maker>A</maker><model>B</model>\
                 <mount>M</mount><cropfactor>1</cropfactor>{inner}</lens></lensdatabase>"
            )
        };
        let cases: Vec<(String, &str)> =
            vec![
            // Breakpoint keys — these feed total_cmp sorts + bracketing.
            (
                lens_wrap("<calibration><distortion model=\"poly3\" focal=\"inf\"/></calibration>"),
                "distortion focal=inf",
            ),
            (
                lens_wrap("<calibration><distortion model=\"poly3\" focal=\"nan\"/></calibration>"),
                "distortion focal=nan",
            ),
            (
                lens_wrap("<calibration><tca model=\"linear\" focal=\"-inf\"/></calibration>"),
                "tca focal=-inf",
            ),
            (
                lens_wrap(
                    "<calibration><vignetting model=\"pa\" focal=\"NaN\" aperture=\"2\" \
                     distance=\"10\" k1=\"0\" k2=\"0\" k3=\"0\"/></calibration>",
                ),
                "vignetting focal=NaN",
            ),
            (
                lens_wrap(
                    "<calibration><vignetting model=\"pa\" focal=\"50\" aperture=\"inf\" \
                     distance=\"10\" k1=\"0\" k2=\"0\" k3=\"0\"/></calibration>",
                ),
                "vignetting aperture=inf",
            ),
            (
                lens_wrap(
                    "<calibration><vignetting model=\"pa\" focal=\"50\" aperture=\"2\" \
                     distance=\"nan\" k1=\"0\" k2=\"0\" k3=\"0\"/></calibration>",
                ),
                "vignetting distance=nan",
            ),
            // Overflow-to-inf literal: parses as +inf in Rust.
            (
                lens_wrap(
                    "<calibration><vignetting model=\"pa\" focal=\"50\" aperture=\"2\" \
                     distance=\"10\" k1=\"1e999\" k2=\"0\" k3=\"0\"/></calibration>",
                ),
                "vignetting k1=1e999 (overflow to inf)",
            ),
            // Optional coefficients with defaults still validate.
            (
                lens_wrap(
                    "<calibration><distortion model=\"ptlens\" focal=\"50\" a=\"nan\"/>\
                     </calibration>",
                ),
                "ptlens a=nan",
            ),
            (
                lens_wrap(
                    "<calibration><tca model=\"poly3\" focal=\"50\" vr=\"inf\"/></calibration>",
                ),
                "tca vr=inf",
            ),
            // real-focal (required-positive path).
            (
                lens_wrap(
                    "<calibration><distortion model=\"poly3\" focal=\"50\" real-focal=\"inf\"/>\
                     </calibration>",
                ),
                "real-focal=inf",
            ),
            // focal/aperture range descriptors.
            (lens_wrap("<focal min=\"inf\" max=\"22\"/>"), "focal min=inf"),
            (lens_wrap("<aperture value=\"nan\"/>"), "aperture value=nan"),
            // aspect-ratio decimal form.
            (
                lens_wrap("<aspect-ratio>inf</aspect-ratio>"),
                "aspect-ratio inf",
            ),
        ];
        for (doc, what) in &cases {
            let e = schema_err(doc);
            assert!(
                e.msg.contains("not finite"),
                "{what}: expected a not-finite rejection, got {:?}",
                e.msg
            );
        }
        // cropfactor: NaN must not slip through the `<= 0.0` positivity
        // check (NaN compares false) — the finiteness check catches it.
        let doc = "<lensdatabase version=\"2\"><camera><maker>A</maker><model>B</model>\
                   <mount>M</mount><cropfactor>NaN</cropfactor></camera></lensdatabase>";
        let e = schema_err(doc);
        assert!(
            e.msg.contains("not finite"),
            "camera cropfactor NaN: {}",
            e.msg
        );
        let doc = "<lensdatabase version=\"2\"><lens><maker>A</maker><model>B</model>\
                   <mount>M</mount><cropfactor>inf</cropfactor></lens></lensdatabase>";
        let e = schema_err(doc);
        assert!(
            e.msg.contains("not finite"),
            "lens cropfactor inf: {}",
            e.msg
        );
    }

    /// PINNED CONTRACT (observation): `min > max` on a focal/aperture
    /// range is ACCEPTED as-is — the schema checks each bound's
    /// positivity but not their ordering. Recorded so any future
    /// tightening is a conscious contract change.
    #[test]
    fn inverted_range_min_greater_than_max_is_accepted_as_is() {
        let f = file(
            "<lensdatabase version=\"2\"><lens><maker>A</maker><model>B</model>\
             <mount>M</mount><focal min=\"22\" max=\"10\"/><cropfactor>1</cropfactor>\
             </lens></lensdatabase>",
        );
        assert_eq!(
            f.lenses[0].focal,
            Some(RangeOrValue::Range {
                min: 22.0,
                max: 10.0
            })
        );
    }

    #[test]
    fn vignetting_requires_all_coefficients() {
        let e = schema_err(
            "<lensdatabase version=\"2\"><lens><maker>A</maker><model>B</model>\
             <mount>M</mount><cropfactor>1</cropfactor><calibration>\
             <vignetting model=\"pa\" focal=\"50\" aperture=\"2\" distance=\"10\" k1=\"-0.5\" k2=\"0.1\"/>\
             </calibration></lens></lensdatabase>",
        );
        assert!(e.msg.contains("k3"), "{}", e.msg);
    }
}
