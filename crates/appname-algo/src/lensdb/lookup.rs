// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Deterministic camera/lens matching + calibration interpolation.
//!
//! Feeds O-IO's RAW develop `lens_correction` stage (ES Part 43 §43.3
//! reserved sub-stage 4b `lensCorrection`; ALGORITHMS.md §21; contract
//! primary `brown_conrady_plus_division_model` L0). This module only
//! RESOLVES profile parameters; sampling/undistortion is the consumer's
//! (dossier `upright-lens`: single-pass inverse-map, Newton inverse).
//!
//! Matching (D-48 total order):
//! - names compare under [`normalize_name`] (Unicode lowercase +
//!   whitespace collapse — the corpus mixes case and spacing across
//!   maker/model variants);
//! - every localized name variant participates (the corpus records
//!   e.g. a canonical `Canon PowerShot Pro1` plus a `lang="en"`
//!   `PowerShot Pro1`), and a query model is additionally tried as
//!   `"maker model"` so maker-prefixed canonical strings match
//!   maker-less queries;
//! - multi-match results are returned in one documented total order
//!   (cameras: normalized maker, model, variant, crop-factor bits,
//!   mount; lenses: normalized maker, model, crop-factor bits, joined
//!   sorted mounts) — ties are impossible because each sort key
//!   contains the reader's dedupe identity as a sub-tuple (cameras:
//!   maker/model/variant; lenses: maker/model/mounts/crop bits);
//! - camera-constrained lens matching requires a shared mount (the
//!   camera's own mount or its declared one-level `compat` list — the
//!   corpus compat lists are flat, no transitive closure) and crop
//!   compatibility (lens calibrated for a sensor no smaller than the
//!   camera's: `lens.crop_factor ≤ camera.crop_factor + 1e-9`).
//!
//! Interpolation (documented own choice; the dossier fixes the
//! corrector math and the no-double-resample rule but not the profile
//! interpolation — flagged in the row log as an open dossier item):
//! LINEAR IN PARAMETER SPACE between adjacent focal breakpoints of the
//! SAME model; exact at breakpoints; clamped to the nearest endpoint
//! outside the calibrated range; when the two bracketing breakpoints
//! carry different models, the nearer breakpoint wins (ties → the
//! lower focal). Vignetting selects the nearest calibrated aperture,
//! then the nearest distance within it (ties → the smaller value;
//! absent query values default to the smallest calibrated aperture /
//! the largest calibrated distance), then interpolates over focal.
//! Non-finite (NaN/±Inf) query parameters are rejected up front with
//! a typed [`LookupError`] — see [`resolve_calibration`].

use std::fmt;

use super::model::{Calibration, Camera, DistortionModel, Lens, TcaModel, VignettingCal};
use super::reader::LensDatabase;

/// Crop-compatibility slack (pure float-noise guard).
const CROP_EPS: f64 = 1e-9;

/// Invalid-query error from calibration resolution — the module's
/// "typed errors, never panics" posture applied to the f64 query
/// parameters the O-IO adapter seam derives from untrusted EXIF (a
/// 0/0 rational yields NaN).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LookupError {
    /// A query parameter was NaN or ±infinity. DELIBERATE CONTRACT
    /// CHANGE from the first Row C round: ±Inf previously clamped to
    /// the nearest calibrated endpoint (and NaN could panic in the
    /// focal bracketing search); a non-finite QUERY is garbage input,
    /// so all three parameters now consistently return this error
    /// up front instead.
    NonFiniteQuery {
        /// Which parameter: `"focal"`, `"aperture"`, or `"distance"`.
        param: &'static str,
    },
}

impl fmt::Display for LookupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LookupError::NonFiniteQuery { param } => {
                write!(f, "non-finite {param} query parameter (NaN or ±inf)")
            }
        }
    }
}

/// Normalized comparison form: Unicode lowercase, whitespace runs
/// collapsed to single spaces, trimmed.
pub fn normalize_name(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for (i, word) in s.split_whitespace().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        for ch in word.chars() {
            out.extend(ch.to_lowercase());
        }
    }
    out
}

fn name_variants_match(names: &[super::model::LocalName], query_norm: &str) -> bool {
    names.iter().any(|n| normalize_name(&n.text) == query_norm)
}

/// True when the record's model names match the query model, where the
/// query may omit the maker prefix carried by canonical DB strings.
fn model_matches(
    maker_norm: &str,
    model_names: &[super::model::LocalName],
    model_norm: &str,
) -> bool {
    if name_variants_match(model_names, model_norm) {
        return true;
    }
    let prefixed = format!("{maker_norm} {model_norm}");
    name_variants_match(model_names, &prefixed)
}

fn camera_sort_key(c: &Camera) -> (String, String, String, u64, String) {
    (
        normalize_name(c.maker()),
        normalize_name(c.model()),
        // Variant is part of the reader's camera dedupe identity
        // (maker, model, variant): including it makes this key
        // tie-free by construction — two variants of one body would
        // otherwise tie on (maker, model, crop bits, mount).
        normalize_name(c.variant.as_deref().unwrap_or("")),
        c.crop_factor.to_bits(),
        c.mount.clone(),
    )
}

fn lens_sort_key(l: &Lens) -> (String, String, u64, String) {
    let mut mounts: Vec<String> = l.mounts.iter().map(|m| normalize_name(m)).collect();
    mounts.sort();
    (
        normalize_name(l.maker()),
        normalize_name(l.model()),
        l.crop_factor.to_bits(),
        mounts.join("|"),
    )
}

impl LensDatabase {
    /// All cameras matching (maker, model), in the documented total
    /// order.
    pub fn find_cameras(&self, maker: &str, model: &str) -> Vec<&Camera> {
        let maker_norm = normalize_name(maker);
        let model_norm = normalize_name(model);
        let mut hits: Vec<&Camera> = self
            .cameras
            .iter()
            .filter(|c| {
                name_variants_match(&c.maker_names, &maker_norm)
                    && model_matches(&maker_norm, &c.model_names, &model_norm)
            })
            .collect();
        hits.sort_by_key(|c| camera_sort_key(c));
        hits
    }

    /// First camera in [`Self::find_cameras`] order.
    pub fn find_camera(&self, maker: &str, model: &str) -> Option<&Camera> {
        self.find_cameras(maker, model).into_iter().next()
    }

    /// Compatible mounts of a camera: its own mount plus the mount
    /// declaration's one-level `compat` list (normalized).
    pub fn compatible_mounts(&self, camera: &Camera) -> Vec<String> {
        let own = normalize_name(&camera.mount);
        let mut out = vec![own.clone()];
        for m in &self.mounts {
            if normalize_name(m.name()) == own {
                for c in &m.compat {
                    let n = normalize_name(c);
                    if !out.contains(&n) {
                        out.push(n);
                    }
                }
            }
        }
        out
    }

    /// All lenses matching (maker, model), optionally constrained to a
    /// camera (mount + crop compatibility), in the documented total
    /// order.
    pub fn find_lenses(&self, maker: &str, model: &str, camera: Option<&Camera>) -> Vec<&Lens> {
        let maker_norm = normalize_name(maker);
        let model_norm = normalize_name(model);
        let cam_mounts = camera.map(|c| self.compatible_mounts(c));
        let mut hits: Vec<&Lens> = self
            .lenses
            .iter()
            .filter(|l| {
                if !name_variants_match(&l.maker_names, &maker_norm)
                    || !model_matches(&maker_norm, &l.model_names, &model_norm)
                {
                    return false;
                }
                match (camera, &cam_mounts) {
                    (Some(cam), Some(mounts)) => {
                        let mount_ok = l.mounts.iter().any(|m| mounts.contains(&normalize_name(m)));
                        mount_ok && l.crop_factor <= cam.crop_factor + CROP_EPS
                    }
                    _ => true,
                }
            })
            .collect();
        hits.sort_by_key(|l| lens_sort_key(l));
        hits
    }

    /// First lens in [`Self::find_lenses`] order.
    pub fn find_lens(&self, maker: &str, model: &str, camera: Option<&Camera>) -> Option<&Lens> {
        self.find_lenses(maker, model, camera).into_iter().next()
    }
}

/// Radial part of the engine's Brown–Conrady corrector
/// (`r_d = pre_scale · r_u · (1 + k1·r_u² + k2·r_u⁴ + k3·r_u⁶)` —
/// \[brown66\]; ALGORITHMS.md §21 primary, division model
/// \[fitzgibbon01\] is the corrector's second parameterization and has
/// no direct database source model).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BrownConradyRadial {
    /// Global scale folded out of the polynomial (1 when none).
    pub pre_scale: f64,
    /// r² coefficient.
    pub k1: f64,
    /// r⁴ coefficient.
    pub k2: f64,
    /// r⁶ coefficient.
    pub k3: f64,
}

impl DistortionModel {
    /// Radial multiplier `scale(r)` with `r_d = r_u · scale(r_u)`
    /// (model conventions documented on the enum).
    pub fn radial_scale(&self, r: f64) -> f64 {
        match *self {
            DistortionModel::Ptlens { a, b, c } => {
                let d = 1.0 - a - b - c;
                ((a * r + b) * r + c) * r + d
            }
            DistortionModel::Poly3 { k1 } => 1.0 - k1 + k1 * r * r,
            DistortionModel::Poly5 { k1, k2 } => {
                let r2 = r * r;
                1.0 + k1 * r2 + k2 * r2 * r2
            }
        }
    }

    /// Exact Brown–Conrady radial form, where one exists:
    /// - `poly5` is already the \[brown66\] truncation → `{1, k1, k2, 0}`;
    /// - `poly3` factors as `r_d = (1−k1)·r·(1 + (k1/(1−k1))·r²)` →
    ///   `{1−k1, k1/(1−k1), 0, 0}` (`None` if `1−k1` is numerically 0);
    /// - `ptlens` carries odd multiplier terms (r¹, r², r³) that a pure
    ///   even Brown–Conrady series cannot express → `None`; the
    ///   consumer corrects through [`Self::radial_scale`] with the
    ///   dossier's Newton inverse instead.
    pub fn to_brown_conrady(&self) -> Option<BrownConradyRadial> {
        match *self {
            DistortionModel::Poly5 { k1, k2 } => Some(BrownConradyRadial {
                pre_scale: 1.0,
                k1,
                k2,
                k3: 0.0,
            }),
            DistortionModel::Poly3 { k1 } => {
                let s = 1.0 - k1;
                if s.abs() < 1e-12 {
                    return None;
                }
                Some(BrownConradyRadial {
                    pre_scale: s,
                    k1: k1 / s,
                    k2: 0.0,
                    k3: 0.0,
                })
            }
            DistortionModel::Ptlens { .. } => None,
        }
    }
}

impl TcaModel {
    /// Red-channel radial multiplier (`r_red = r · scale_red(r)`).
    pub fn scale_red(&self, r: f64) -> f64 {
        match *self {
            TcaModel::Linear { kr, .. } => kr,
            TcaModel::Poly3 { vr, cr, br, .. } => vr + cr * r + br * r * r,
        }
    }

    /// Blue-channel radial multiplier (`r_blue = r · scale_blue(r)`).
    pub fn scale_blue(&self, r: f64) -> f64 {
        match *self {
            TcaModel::Linear { kb, .. } => kb,
            TcaModel::Poly3 { vb, cb, bb, .. } => vb + cb * r + bb * r * r,
        }
    }
}

/// Vignetting falloff resolved at a query point:
/// `falloff(r) = 1 + k1·r² + k2·r⁴ + k3·r⁶` (`pa` model — even radial
/// polynomial, cos⁴ lineage, L0 per ALGORITHMS.md §21). Correction
/// divides the image by the falloff.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedVignetting {
    /// r² coefficient.
    pub k1: f64,
    /// r⁴ coefficient.
    pub k2: f64,
    /// r⁶ coefficient.
    pub k3: f64,
    /// Calibrated aperture group actually used.
    pub aperture: f64,
    /// Calibrated distance group actually used.
    pub distance: f64,
}

impl ResolvedVignetting {
    /// Falloff multiplier at normalized radius `r`.
    pub fn falloff(&self, r: f64) -> f64 {
        let r2 = r * r;
        1.0 + self.k1 * r2 + self.k2 * r2 * r2 + self.k3 * r2 * r2 * r2
    }
}

/// Calibration parameters resolved at one (focal, aperture, distance)
/// query — what the develop stage consumes per shot.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedCalibration {
    /// Distortion model at the query focal, when calibrated.
    pub distortion: Option<DistortionModel>,
    /// TCA model at the query focal, when calibrated.
    pub tca: Option<TcaModel>,
    /// Vignetting at the query point, when calibrated.
    pub vignetting: Option<ResolvedVignetting>,
}

/// Bracket + interpolate over a focal-sorted slice. `same_model` gates
/// cross-model interpolation; `lerp` blends two entries of the same
/// model shape.
fn interp_focal<T: Copy>(
    entries: &[T],
    focal_of: impl Fn(&T) -> f64,
    focal: f64,
    same_model: impl Fn(&T, &T) -> bool,
    lerp: impl Fn(&T, &T, f64) -> T,
) -> Option<T> {
    let (first, last) = (entries.first()?, entries.last()?);
    if focal <= focal_of(first) {
        return Some(*first);
    }
    if focal >= focal_of(last) {
        return Some(*last);
    }
    // First index whose focal is >= query (entries sorted by focal).
    let hi = entries
        .iter()
        .position(|e| focal_of(e) >= focal)
        .expect("bracketing endpoint exists: query < last focal");
    let lo = hi - 1; // hi >= 1: query > first focal.
    let (a, b) = (&entries[lo], &entries[hi]);
    let (fa, fb) = (focal_of(a), focal_of(b));
    if fb <= fa {
        // Duplicate breakpoint focals: deterministic pick of the lower
        // index.
        return Some(*a);
    }
    if focal_of(b) == focal {
        return Some(*b); // exact breakpoint hit
    }
    if !same_model(a, b) {
        // No cross-model interpolation: nearest breakpoint, ties → the
        // lower focal.
        let t = (focal - fa) / (fb - fa);
        return Some(if t <= 0.5 { *a } else { *b });
    }
    let t = (focal - fa) / (fb - fa);
    Some(lerp(a, b, t))
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

fn interp_distortion(cal: &Calibration, focal: f64) -> Option<DistortionModel> {
    interp_focal(
        &cal.distortion,
        |d| d.focal,
        focal,
        |a, b| std::mem::discriminant(&a.model) == std::mem::discriminant(&b.model),
        |a, b, t| {
            let model = match (a.model, b.model) {
                (
                    DistortionModel::Ptlens {
                        a: aa,
                        b: ab,
                        c: ac,
                    },
                    DistortionModel::Ptlens {
                        a: ba,
                        b: bb,
                        c: bc,
                    },
                ) => DistortionModel::Ptlens {
                    a: lerp(aa, ba, t),
                    b: lerp(ab, bb, t),
                    c: lerp(ac, bc, t),
                },
                (DistortionModel::Poly3 { k1: x }, DistortionModel::Poly3 { k1: y }) => {
                    DistortionModel::Poly3 { k1: lerp(x, y, t) }
                }
                (
                    DistortionModel::Poly5 { k1: x1, k2: x2 },
                    DistortionModel::Poly5 { k1: y1, k2: y2 },
                ) => DistortionModel::Poly5 {
                    k1: lerp(x1, y1, t),
                    k2: lerp(x2, y2, t),
                },
                // same_model gate makes mixed arms unreachable.
                (m, _) => m,
            };
            super::model::DistortionCal {
                focal,
                real_focal: None,
                model,
            }
        },
    )
    .map(|d| d.model)
}

fn interp_tca(cal: &Calibration, focal: f64) -> Option<TcaModel> {
    interp_focal(
        &cal.tca,
        |t| t.focal,
        focal,
        |a, b| std::mem::discriminant(&a.model) == std::mem::discriminant(&b.model),
        |a, b, t| {
            let model = match (a.model, b.model) {
                (TcaModel::Linear { kr: x1, kb: x2 }, TcaModel::Linear { kr: y1, kb: y2 }) => {
                    TcaModel::Linear {
                        kr: lerp(x1, y1, t),
                        kb: lerp(x2, y2, t),
                    }
                }
                (
                    TcaModel::Poly3 {
                        vr: xvr,
                        vb: xvb,
                        cr: xcr,
                        cb: xcb,
                        br: xbr,
                        bb: xbb,
                    },
                    TcaModel::Poly3 {
                        vr: yvr,
                        vb: yvb,
                        cr: ycr,
                        cb: ycb,
                        br: ybr,
                        bb: ybb,
                    },
                ) => TcaModel::Poly3 {
                    vr: lerp(xvr, yvr, t),
                    vb: lerp(xvb, yvb, t),
                    cr: lerp(xcr, ycr, t),
                    cb: lerp(xcb, ycb, t),
                    br: lerp(xbr, ybr, t),
                    bb: lerp(xbb, ybb, t),
                },
                (m, _) => m,
            };
            super::model::TcaCal { focal, model }
        },
    )
    .map(|t| t.model)
}

/// Nearest value in a sorted, deduplicated list; ties → the smaller.
fn nearest(sorted: &[f64], query: f64) -> Option<f64> {
    let mut best: Option<f64> = None;
    let mut best_d = f64::INFINITY;
    for &v in sorted {
        let d = (v - query).abs();
        // Strict `<` keeps the FIRST (smaller) value on exact ties.
        if d < best_d {
            best_d = d;
            best = Some(v);
        }
    }
    best
}

fn interp_vignetting(
    cal: &Calibration,
    focal: f64,
    aperture: Option<f64>,
    distance: Option<f64>,
) -> Option<ResolvedVignetting> {
    if cal.vignetting.is_empty() {
        return None;
    }
    // Distinct calibrated apertures (sorted set).
    let mut apertures: Vec<f64> = cal.vignetting.iter().map(|v| v.aperture).collect();
    apertures.sort_by(f64::total_cmp);
    apertures.dedup();
    // Absent aperture → the smallest calibrated one (documented choice).
    let ap = match aperture {
        Some(q) => nearest(&apertures, q)?,
        None => *apertures.first()?,
    };
    let group: Vec<VignettingCal> = cal
        .vignetting
        .iter()
        .filter(|v| v.aperture == ap)
        .copied()
        .collect();
    let mut distances: Vec<f64> = group.iter().map(|v| v.distance).collect();
    distances.sort_by(f64::total_cmp);
    distances.dedup();
    // Absent distance → the largest calibrated one (infinity-style
    // default; documented choice).
    let dist = match distance {
        Some(q) => nearest(&distances, q)?,
        None => *distances.last()?,
    };
    let series: Vec<VignettingCal> = group
        .iter()
        .filter(|v| v.distance == dist)
        .copied()
        .collect();
    interp_focal(
        &series,
        |v| v.focal,
        focal,
        |_, _| true,
        |a, b, t| VignettingCal {
            focal,
            aperture: ap,
            distance: dist,
            k1: lerp(a.k1, b.k1, t),
            k2: lerp(a.k2, b.k2, t),
            k3: lerp(a.k3, b.k3, t),
        },
    )
    .map(|v| ResolvedVignetting {
        k1: v.k1,
        k2: v.k2,
        k3: v.k3,
        aperture: ap,
        distance: dist,
    })
}

/// Resolve a lens's calibration at a query point. `Ok(None)` when the
/// lens carries no calibration block at all.
///
/// Query validation (up front, before any lookup): a NaN or ±Inf
/// focal, aperture, or distance returns
/// [`LookupError::NonFiniteQuery`]. This is a deliberate change from
/// the first Row C round, where ±Inf clamped to the nearest
/// calibrated endpoint and a NaN focal could panic in the bracketing
/// search — a non-finite query is garbage input (EXIF 0/0 rationals
/// at the O-IO seam), and the honest contract is a typed `Err` for
/// all three parameters consistently.
pub fn resolve_calibration(
    lens: &Lens,
    focal: f64,
    aperture: Option<f64>,
    distance: Option<f64>,
) -> Result<Option<ResolvedCalibration>, LookupError> {
    if !focal.is_finite() {
        return Err(LookupError::NonFiniteQuery { param: "focal" });
    }
    if aperture.is_some_and(|a| !a.is_finite()) {
        return Err(LookupError::NonFiniteQuery { param: "aperture" });
    }
    if distance.is_some_and(|d| !d.is_finite()) {
        return Err(LookupError::NonFiniteQuery { param: "distance" });
    }
    let Some(cal) = lens.calibration.as_ref() else {
        return Ok(None);
    };
    Ok(Some(ResolvedCalibration {
        distortion: interp_distortion(cal, focal),
        tca: interp_tca(cal, focal),
        vignetting: interp_vignetting(cal, focal, aperture, distance),
    }))
}

#[cfg(test)]
mod tests {
    use super::super::model::FULL_FIXTURE;
    use super::super::reader::from_named_documents;
    use super::*;

    fn db() -> LensDatabase {
        from_named_documents(&[("a.xml".to_owned(), FULL_FIXTURE.to_owned())]).expect("db")
    }

    #[test]
    fn normalize_folds_case_and_whitespace() {
        assert_eq!(normalize_name("  Canon   EF-S  "), "canon ef-s");
        assert_eq!(normalize_name("EXAMPLECORP"), "examplecorp");
        assert_eq!(normalize_name("Байонет K"), "байонет k");
        assert_eq!(normalize_name("a\tb\nc"), "a b c");
    }

    #[test]
    fn camera_exact_match() {
        let d = db();
        let c = d
            .find_camera("Examplecorp", "Examplecorp Snapper 5D")
            .expect("hit");
        assert_eq!(c.crop_factor, 1.6);
    }

    #[test]
    fn camera_matches_case_and_whitespace_insensitively() {
        let d = db();
        assert!(d
            .find_camera("EXAMPLECORP", "examplecorp   snapper 5d")
            .is_some());
    }

    #[test]
    fn camera_matches_localized_variant_and_maker_prefix_form() {
        let d = db();
        // lang="en" model variant "Snapper 5D".
        assert!(d.find_camera("Examplecorp", "Snapper 5D").is_some());
        // lang="en" maker variant.
        assert!(d.find_camera("Example", "Examplecorp Snapper 5D").is_some());
        // Maker-less query model against the maker-prefixed canonical:
        // "examplecorp" + "snapper 5d" → "examplecorp snapper 5d".
        assert!(d.find_camera("Examplecorp", "snapper 5d").is_some());
    }

    #[test]
    fn unknown_camera_is_none() {
        assert!(db().find_camera("Nobody", "Nothing 3000").is_none());
    }

    #[test]
    fn lens_match_unconstrained_and_via_camera() {
        let d = db();
        let l = d
            .find_lens("Examplecorp", "Examplecorp Wide 10-22mm f/3.5-4.5", None)
            .expect("hit");
        assert_eq!(l.crop_factor, 1.6);
        let cam = d
            .find_camera("Examplecorp", "Examplecorp Snapper 5D")
            .expect("cam");
        assert!(d
            .find_lens(
                "Examplecorp",
                "Examplecorp Wide 10-22mm f/3.5-4.5",
                Some(cam)
            )
            .is_some());
    }

    #[test]
    fn lens_mount_compat_list_is_honored() {
        let d = db();
        let cam = d
            .find_camera("Examplecorp", "Examplecorp Snapper 5D")
            .expect("cam");
        // The prime is mounted "Example EF" and "M42"; the camera mount
        // "Example EF" also lists M42 in compat. Either way it matches.
        assert!(d
            .find_lens("Examplecorp", "Examplecorp Prime 50mm f/1.8", Some(cam))
            .is_some());
        // compatible_mounts = own + declared compat, normalized (the
        // reader canonicalizes compat lists to sorted order).
        assert_eq!(
            d.compatible_mounts(cam),
            vec!["example ef", "generic", "m42"]
        );
    }

    #[test]
    fn crop_incompatible_lens_is_excluded() {
        let d = db();
        // The full-frame camera (crop 1.0) is NOT in the fixture; build
        // the check the other way: the fisheye lens (crop 1.6) must not
        // match a hypothetical crop-1.0 camera.
        let mut cam = d
            .find_camera("Examplecorp", "Examplecorp Snapper 5D")
            .expect("cam")
            .clone();
        cam.crop_factor = 1.0;
        assert!(
            d.find_lens("Examplecorp", "Examplecorp Fish 8mm f/3.5", Some(&cam))
                .is_none(),
            "lens calibrated on a 1.6-crop sensor must not match a 1.0-crop body"
        );
        // The crop-1.0 prime still matches.
        assert!(d
            .find_lens("Examplecorp", "Examplecorp Prime 50mm f/1.8", Some(&cam))
            .is_some());
    }

    #[test]
    fn multi_match_order_is_total_and_deterministic() {
        let extra = "<lensdatabase version=\"2\">\
                     <lens><maker>Examplecorp</maker>\
                     <model>Examplecorp Prime 50mm f/1.8</model>\
                     <mount>Other</mount><cropfactor>2.0</cropfactor>\
                     </lens></lensdatabase>";
        let d = from_named_documents(&[
            ("a.xml".to_owned(), FULL_FIXTURE.to_owned()),
            ("b.xml".to_owned(), extra.to_owned()),
        ])
        .expect("db");
        let hits = d.find_lenses("Examplecorp", "Examplecorp Prime 50mm f/1.8", None);
        assert_eq!(hits.len(), 2);
        // Total order: crop bits ascending (1.0 < 2.0).
        assert_eq!(hits[0].crop_factor, 1.0);
        assert_eq!(hits[1].crop_factor, 2.0);
        let again = d.find_lenses("Examplecorp", "Examplecorp Prime 50mm f/1.8", None);
        assert_eq!(hits, again);
    }

    fn wide(d: &LensDatabase) -> Lens {
        d.find_lens("Examplecorp", "Examplecorp Wide 10-22mm f/3.5-4.5", None)
            .expect("lens")
            .clone()
    }

    /// Finite-query resolution helper: unwraps both the query-
    /// validation layer and the has-calibration layer.
    fn resolved(
        l: &Lens,
        focal: f64,
        aperture: Option<f64>,
        distance: Option<f64>,
    ) -> ResolvedCalibration {
        resolve_calibration(l, focal, aperture, distance)
            .expect("finite query")
            .expect("calibrated lens")
    }

    #[test]
    fn interpolation_exact_at_breakpoints() {
        let d = db();
        let l = wide(&d);
        let at10 = resolved(&l, 10.0, Some(3.5), Some(10.0));
        assert_eq!(
            at10.distortion,
            Some(DistortionModel::Ptlens {
                a: 0.02,
                b: -0.068,
                c: 0.051
            })
        );
        let at22 = resolved(&l, 22.0, None, None);
        assert_eq!(
            at22.distortion,
            Some(DistortionModel::Ptlens {
                a: 0.0,
                b: -0.004,
                c: 0.0
            })
        );
    }

    #[test]
    fn interpolation_linear_at_midpoint() {
        let d = db();
        let l = wide(&d);
        let mid = resolved(&l, 16.0, None, None);
        // t = (16-10)/(22-10) = 0.5 → parameter-space midpoint.
        match mid.distortion.expect("distortion") {
            DistortionModel::Ptlens { a, b, c } => {
                assert!((a - 0.01).abs() < 1e-15, "a={a}");
                assert!((b - (-0.036)).abs() < 1e-15, "b={b}");
                assert!((c - 0.0255).abs() < 1e-15, "c={c}");
            }
            other => panic!("wrong model: {other:?}"),
        }
        match mid.tca.expect("tca") {
            TcaModel::Poly3 { vr, br, .. } => {
                assert!((vr - 1.0002).abs() < 1e-12, "vr={vr}");
                assert!((br - (-0.00000125)).abs() < 1e-15, "br={br}");
            }
            other => panic!("wrong model: {other:?}"),
        }
    }

    #[test]
    fn interpolation_clamps_outside_calibrated_range() {
        let d = db();
        let l = wide(&d);
        let below = resolved(&l, 5.0, None, None);
        let at10 = resolved(&l, 10.0, None, None);
        assert_eq!(below.distortion, at10.distortion);
        let above = resolved(&l, 400.0, None, None);
        let at22 = resolved(&l, 22.0, None, None);
        assert_eq!(above.distortion, at22.distortion);
    }

    #[test]
    fn cross_model_bracket_takes_nearest_breakpoint() {
        let d = db();
        let l = d
            .find_lens("Examplecorp", "Examplecorp Prime 50mm f/1.8", None)
            .expect("lens")
            .clone();
        // Breakpoints: poly3 @ 50, poly5 @ 55 — different models.
        let near50 = resolved(&l, 51.0, None, None);
        assert_eq!(
            near50.distortion,
            Some(DistortionModel::Poly3 { k1: -0.01 })
        );
        let near55 = resolved(&l, 54.0, None, None);
        assert_eq!(
            near55.distortion,
            Some(DistortionModel::Poly5 {
                k1: -0.01,
                k2: 0.002
            })
        );
        // Exact midpoint 52.5: tie → the lower focal.
        let mid = resolved(&l, 52.5, None, None);
        assert_eq!(mid.distortion, Some(DistortionModel::Poly3 { k1: -0.01 }));
    }

    #[test]
    fn vignetting_group_selection_and_defaults() {
        let d = db();
        let l = wide(&d);
        // Exact group hit.
        let v = resolved(&l, 10.0, Some(3.5), Some(10.0))
            .vignetting
            .expect("vignetting");
        assert_eq!(v.aperture, 3.5);
        assert_eq!(v.distance, 10.0);
        assert_eq!(v.k1, -2.1);
        // Absent aperture → smallest calibrated; absent distance →
        // largest calibrated (documented defaults).
        let v2 = resolved(&l, 10.0, None, None)
            .vignetting
            .expect("vignetting");
        assert_eq!(v2.aperture, 3.5);
        assert_eq!(v2.distance, 1000.0);
        // Nearest aperture with deterministic tie handling.
        let v3 = resolved(&l, 10.0, Some(100.0), Some(20.0))
            .vignetting
            .expect("vignetting");
        assert_eq!(v3.aperture, 3.5, "single calibrated aperture");
        assert_eq!(v3.distance, 10.0, "20 is nearer to 10 than to 1000");
    }

    #[test]
    fn nearest_tie_prefers_smaller_value() {
        assert_eq!(nearest(&[1.0, 3.0], 2.0), Some(1.0));
        assert_eq!(nearest(&[1.0, 3.0], 2.1), Some(3.0));
        assert_eq!(nearest(&[], 2.0), None);
    }

    #[test]
    fn lens_without_calibration_resolves_none() {
        let extra = "<lensdatabase version=\"2\">\
                     <lens><maker>X</maker><model>X Bare 30mm</model>\
                     <mount>M</mount><cropfactor>1.0</cropfactor>\
                     </lens></lensdatabase>";
        let d = from_named_documents(&[("a.xml".to_owned(), extra.to_owned())]).expect("db");
        let l = d.find_lens("X", "X Bare 30mm", None).expect("lens").clone();
        assert!(resolve_calibration(&l, 30.0, None, None)
            .expect("finite query")
            .is_none());
    }

    #[test]
    fn non_finite_focal_query_is_typed_err_not_panic() {
        let d = db();
        let l = wide(&d);
        // The reviewer's empirical repro: a NaN focal on a
        // multi-breakpoint calibration previously failed both clamp
        // guards and every `>=` bracket comparison, panicking on the
        // bracketing-endpoint `expect`. Must now be a typed Err.
        assert_eq!(
            resolve_calibration(&l, f64::NAN, None, None),
            Err(LookupError::NonFiniteQuery { param: "focal" })
        );
        // ±Inf previously clamped to an endpoint (degrade path);
        // now deliberately Err — garbage query, consistent contract.
        assert_eq!(
            resolve_calibration(&l, f64::INFINITY, None, None),
            Err(LookupError::NonFiniteQuery { param: "focal" })
        );
        assert_eq!(
            resolve_calibration(&l, f64::NEG_INFINITY, None, None),
            Err(LookupError::NonFiniteQuery { param: "focal" })
        );
    }

    #[test]
    fn non_finite_aperture_and_distance_queries_are_typed_err() {
        let d = db();
        let l = wide(&d);
        // Degrade path before the fix: a NaN aperture/distance made
        // `nearest` return None and vignetting silently vanish.
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(
                resolve_calibration(&l, 10.0, Some(bad), None),
                Err(LookupError::NonFiniteQuery { param: "aperture" })
            );
            assert_eq!(
                resolve_calibration(&l, 10.0, None, Some(bad)),
                Err(LookupError::NonFiniteQuery { param: "distance" })
            );
        }
    }

    #[test]
    fn non_finite_query_rejected_before_missing_calibration_check() {
        // Validation is up front: even a lens WITHOUT calibration
        // reports the bad query, not Ok(None).
        let extra = "<lensdatabase version=\"2\">\
                     <lens><maker>X</maker><model>X Bare 30mm</model>\
                     <mount>M</mount><cropfactor>1.0</cropfactor>\
                     </lens></lensdatabase>";
        let d = from_named_documents(&[("a.xml".to_owned(), extra.to_owned())]).expect("db");
        let l = d.find_lens("X", "X Bare 30mm", None).expect("lens").clone();
        assert_eq!(
            resolve_calibration(&l, f64::NAN, None, None),
            Err(LookupError::NonFiniteQuery { param: "focal" })
        );
    }

    #[test]
    fn two_variant_camera_tie_is_broken_by_variant_in_sort_key() {
        // Two variants of ONE body: identical maker, model, crop and
        // mount — under the pre-fix sort key (maker, model, crop bits,
        // mount) these tied; the key now carries the variant, so the
        // documented order is tie-free by construction.
        let extra = "<lensdatabase version=\"2\">\
                     <camera><maker>Examplecorp</maker>\
                     <model>Examplecorp Snapper 5D</model>\
                     <variant>mk III</variant>\
                     <mount>Example EF</mount><cropfactor>1.6</cropfactor>\
                     </camera></lensdatabase>";
        let d = from_named_documents(&[
            ("a.xml".to_owned(), FULL_FIXTURE.to_owned()),
            ("b.xml".to_owned(), extra.to_owned()),
        ])
        .expect("db");
        let hits = d.find_cameras("Examplecorp", "Examplecorp Snapper 5D");
        assert_eq!(hits.len(), 2, "distinct variants are distinct records");
        // Normalized variant order: "mk ii" < "mk iii".
        assert_eq!(hits[0].variant.as_deref(), Some("mk II"));
        assert_eq!(hits[1].variant.as_deref(), Some("mk III"));
        let again = d.find_cameras("Examplecorp", "Examplecorp Snapper 5D");
        assert_eq!(hits, again);
    }

    #[test]
    fn radial_scale_conventions() {
        // Identity coefficients give the identity multiplier.
        assert_eq!(
            DistortionModel::Ptlens {
                a: 0.0,
                b: 0.0,
                c: 0.0
            }
            .radial_scale(0.7),
            1.0
        );
        assert_eq!(DistortionModel::Poly3 { k1: 0.0 }.radial_scale(0.7), 1.0);
        assert_eq!(
            DistortionModel::Poly5 { k1: 0.0, k2: 0.0 }.radial_scale(0.7),
            1.0
        );
        // ptlens normalization: scale(1) = a + b + c + (1−a−b−c) = 1.
        let p = DistortionModel::Ptlens {
            a: 0.02,
            b: -0.068,
            c: 0.051,
        };
        assert!((p.radial_scale(1.0) - 1.0).abs() < 1e-15);
        // poly3 normalization: scale(1) = 1 − k1 + k1 = 1.
        let q = DistortionModel::Poly3 { k1: -0.019 };
        assert!((q.radial_scale(1.0) - 1.0).abs() < 1e-15);
    }

    #[test]
    fn brown_conrady_mapping_is_exact_where_defined() {
        // poly5 maps 1:1.
        let p5 = DistortionModel::Poly5 {
            k1: -0.02,
            k2: 0.003,
        };
        let bc = p5.to_brown_conrady().expect("exact");
        assert_eq!(bc.pre_scale, 1.0);
        assert_eq!((bc.k1, bc.k2, bc.k3), (-0.02, 0.003, 0.0));
        // poly3 algebra: pre_scale·(1 + k1'·r²) == radial_scale(r).
        let p3 = DistortionModel::Poly3 { k1: -0.019 };
        let bc3 = p3.to_brown_conrady().expect("exact");
        for r in [0.0, 0.25, 0.5, 0.75, 1.0, 1.3] {
            let via_bc = bc3.pre_scale * (1.0 + bc3.k1 * r * r);
            assert!(
                (via_bc - p3.radial_scale(r)).abs() < 1e-15,
                "r={r}: {via_bc} vs {}",
                p3.radial_scale(r)
            );
        }
        // ptlens has no pure even-series form.
        assert!(DistortionModel::Ptlens {
            a: 0.01,
            b: 0.0,
            c: 0.0
        }
        .to_brown_conrady()
        .is_none());
        // Degenerate poly3 (k1 = 1) declines instead of dividing by 0.
        assert!(DistortionModel::Poly3 { k1: 1.0 }
            .to_brown_conrady()
            .is_none());
    }

    #[test]
    fn tca_scales_default_to_identity() {
        let lin = TcaModel::Linear { kr: 1.0, kb: 1.0 };
        assert_eq!(lin.scale_red(0.9), 1.0);
        assert_eq!(lin.scale_blue(0.9), 1.0);
        let p3 = TcaModel::Poly3 {
            vr: 1.0,
            vb: 1.0,
            cr: 0.0,
            cb: 0.0,
            br: 0.0,
            bb: 0.0,
        };
        assert_eq!(p3.scale_red(0.9), 1.0);
        assert_eq!(p3.scale_blue(0.9), 1.0);
    }

    // ------------------------------------------------------------------
    // Tester round (2026-08-17): degenerate-calibration + matching-edge
    // pins.
    // ------------------------------------------------------------------

    fn lens_from(doc: &str, maker: &str, model: &str) -> Lens {
        let d = from_named_documents(&[("t.xml".to_owned(), doc.to_owned())]).expect("db");
        d.find_lens(maker, model, None).expect("lens").clone()
    }

    /// Duplicate focal breakpoints (two entries at the SAME focal):
    /// every query resolves deterministically — pinned picks, not UB.
    #[test]
    fn duplicate_focal_breakpoints_resolve_deterministically() {
        let doc = "<lensdatabase version=\"2\">\
                   <lens><maker>X</maker><model>X Dup 10-20mm</model>\
                   <mount>M</mount><cropfactor>1.0</cropfactor><calibration>\
                   <distortion model=\"poly3\" focal=\"10\" k1=\"-0.01\"/>\
                   <distortion model=\"poly3\" focal=\"10\" k1=\"-0.05\"/>\
                   <distortion model=\"poly3\" focal=\"20\" k1=\"-0.09\"/>\
                   </calibration></lens></lensdatabase>";
        let l = lens_from(doc, "X", "X Dup 10-20mm");
        // At/below the duplicated breakpoint: the FIRST duplicate (the
        // stable sort preserves file order) wins via the clamp path.
        for f in [5.0, 10.0] {
            assert_eq!(
                resolved(&l, f, None, None).distortion,
                Some(DistortionModel::Poly3 { k1: -0.01 }),
                "focal {f}: first duplicate wins at/below the breakpoint"
            );
        }
        // Between 10 and 20 the bracketing low endpoint is the LAST
        // duplicate (position() finds the first focal >= query, i.e.
        // the 20 entry; lo = the second 10 entry): t = 0.5 from
        // k1=-0.05 to k1=-0.09.
        match resolved(&l, 15.0, None, None)
            .distortion
            .expect("distortion")
        {
            DistortionModel::Poly3 { k1 } => {
                assert!((k1 - (-0.07)).abs() < 1e-15, "k1={k1}");
            }
            other => panic!("wrong model: {other:?}"),
        }
        // Two runs agree (determinism oracle on the degenerate shape).
        assert_eq!(
            resolve_calibration(&l, 15.0, None, None),
            resolve_calibration(&l, 15.0, None, None)
        );
    }

    /// ONLY duplicate breakpoints (no other focal): clamp paths pick
    /// first below/at, last above — both deterministic; pinned.
    #[test]
    fn all_duplicate_breakpoints_clamp_first_below_last_above() {
        let doc = "<lensdatabase version=\"2\">\
                   <lens><maker>X</maker><model>X TwoAtTen</model>\
                   <mount>M</mount><cropfactor>1.0</cropfactor><calibration>\
                   <distortion model=\"poly3\" focal=\"10\" k1=\"-0.01\"/>\
                   <distortion model=\"poly3\" focal=\"10\" k1=\"-0.05\"/>\
                   </calibration></lens></lensdatabase>";
        let l = lens_from(doc, "X", "X TwoAtTen");
        assert_eq!(
            resolved(&l, 10.0, None, None).distortion,
            Some(DistortionModel::Poly3 { k1: -0.01 }),
            "at the breakpoint: first entry (clamp-low path)"
        );
        assert_eq!(
            resolved(&l, 15.0, None, None).distortion,
            Some(DistortionModel::Poly3 { k1: -0.05 }),
            "above the breakpoint: last entry (clamp-high path)"
        );
    }

    /// A single-breakpoint calibration degenerates to a constant:
    /// exactly the calibrated entry at every query focal.
    #[test]
    fn single_breakpoint_calibration_is_constant_everywhere() {
        let doc = "<lensdatabase version=\"2\">\
                   <lens><maker>X</maker><model>X Prime 50</model>\
                   <mount>M</mount><cropfactor>1.0</cropfactor><calibration>\
                   <distortion model=\"ptlens\" focal=\"50\" a=\"0.01\" b=\"-0.03\" c=\"0.02\"/>\
                   <tca model=\"linear\" focal=\"50\" kr=\"1.0002\" kb=\"0.9998\"/>\
                   <vignetting model=\"pa\" focal=\"50\" aperture=\"2\" distance=\"10\" \
                   k1=\"-0.4\" k2=\"0.1\" k3=\"0\"/>\
                   </calibration></lens></lensdatabase>";
        let l = lens_from(doc, "X", "X Prime 50");
        let expected = resolved(&l, 50.0, Some(2.0), Some(10.0));
        for f in [0.001, 49.0, 50.0, 51.0, 10_000.0] {
            let r = resolved(&l, f, Some(2.0), Some(10.0));
            assert_eq!(r.distortion, expected.distortion, "focal {f}");
            assert_eq!(r.tca, expected.tca, "focal {f}");
            assert_eq!(r.vignetting, expected.vignetting, "focal {f}");
        }
    }

    /// Sparse vignetting dimensions: breakpoints living in DIFFERENT
    /// aperture groups never interpolate across focal — the aperture
    /// group is chosen first (nearest), then focal clamps within it.
    #[test]
    fn sparse_vignetting_stays_inside_the_chosen_aperture_group() {
        let doc = "<lensdatabase version=\"2\">\
                   <lens><maker>X</maker><model>X Zoom 10-22</model>\
                   <mount>M</mount><cropfactor>1.0</cropfactor><calibration>\
                   <vignetting model=\"pa\" focal=\"10\" aperture=\"2.8\" distance=\"10\" \
                   k1=\"-1\" k2=\"0\" k3=\"0\"/>\
                   <vignetting model=\"pa\" focal=\"22\" aperture=\"5.6\" distance=\"10\" \
                   k1=\"-2\" k2=\"0\" k3=\"0\"/>\
                   </calibration></lens></lensdatabase>";
        let l = lens_from(doc, "X", "X Zoom 10-22");
        // Query at focal 16 (between the two breakpoints) with the
        // focal-10 group's aperture: only that group participates —
        // k1 = -1 (clamped), NOT a -1/-2 blend.
        let v = resolved(&l, 16.0, Some(2.8), None)
            .vignetting
            .expect("vignetting");
        assert_eq!((v.aperture, v.k1), (2.8, -1.0));
        // The other group's aperture: clamped to ITS sole entry even
        // though the query focal 10 exactly matches the other group's
        // breakpoint.
        let v = resolved(&l, 10.0, Some(5.6), None)
            .vignetting
            .expect("vignetting");
        assert_eq!((v.aperture, v.k1), (5.6, -2.0));
        // Absent aperture defaults to the smallest calibrated (2.8).
        let v = resolved(&l, 22.0, None, None)
            .vignetting
            .expect("vignetting");
        assert_eq!((v.aperture, v.k1), (2.8, -1.0));
    }

    /// Unicode maker names (real corpus shape: maker "Voigtländer"
    /// with an ASCII-prefixed canonical model): Unicode case folding
    /// matches; diacritics are NOT folded — pinned contract.
    #[test]
    fn unicode_maker_name_case_folds_but_diacritics_do_not_fold() {
        let doc = "<lensdatabase version=\"2\">\
                   <lens><maker>Voigtländer</maker>\
                   <model>Voigtlander Ultron 40mm f/2 SLII Aspherical</model>\
                   <model lang=\"en\">Ultron 40mm f/2 SL-II Aspherical</model>\
                   <mount>Nikon F AI-S</mount><cropfactor>1.0</cropfactor>\
                   </lens></lensdatabase>";
        let d = from_named_documents(&[("v.xml".to_owned(), doc.to_owned())]).expect("db");
        // Unicode uppercase Ä folds to ä.
        assert!(
            d.find_lens(
                "VOIGTLÄNDER",
                "voigtlander ultron 40mm f/2 slii aspherical",
                None
            )
            .is_some(),
            "Unicode case folding must match"
        );
        // Localized model variant participates.
        assert!(
            d.find_lens("Voigtländer", "Ultron 40mm f/2 SL-II Aspherical", None)
                .is_some(),
            "lang variant must match"
        );
        // PINNED: normalization is lowercase + whitespace only — the
        // ASCII spelling "Voigtlander" does NOT match the maker
        // "Voigtländer" (no diacritic folding).
        assert!(
            d.find_lens(
                "Voigtlander",
                "Voigtlander Ultron 40mm f/2 SLII Aspherical",
                None
            )
            .is_none(),
            "diacritics are not folded (pinned contract)"
        );
    }

    /// Crop-compatibility boundary: exact equality matches, float noise
    /// inside CROP_EPS matches, a real difference above it is excluded.
    #[test]
    fn crop_factor_boundary_values_around_the_epsilon() {
        let doc = "<lensdatabase version=\"2\">\
                   <camera><maker>X</maker><model>X Body</model>\
                   <mount>M</mount><cropfactor>1.6</cropfactor></camera>\
                   <lens><maker>X</maker><model>X Equal</model>\
                   <mount>M</mount><cropfactor>1.6</cropfactor></lens>\
                   <lens><maker>X</maker><model>X Noise</model>\
                   <mount>M</mount><cropfactor>1.6000000001</cropfactor></lens>\
                   <lens><maker>X</maker><model>X Over</model>\
                   <mount>M</mount><cropfactor>1.60000002</cropfactor></lens>\
                   </lensdatabase>";
        let d = from_named_documents(&[("c.xml".to_owned(), doc.to_owned())]).expect("db");
        let cam = d.find_camera("X", "X Body").expect("camera");
        assert!(
            d.find_lens("X", "X Equal", Some(cam)).is_some(),
            "exact crop equality is compatible"
        );
        assert!(
            d.find_lens("X", "X Noise", Some(cam)).is_some(),
            "1e-10 above is float noise inside CROP_EPS"
        );
        assert!(
            d.find_lens("X", "X Over", Some(cam)).is_none(),
            "2e-8 above is a real difference outside CROP_EPS"
        );
    }

    #[test]
    fn vignetting_falloff_evaluates_even_polynomial() {
        let v = ResolvedVignetting {
            k1: -0.5,
            k2: 0.25,
            k3: -0.125,
            aperture: 4.0,
            distance: 1000.0,
        };
        assert_eq!(v.falloff(0.0), 1.0);
        let r: f64 = 0.8;
        let r2 = r * r;
        let expect = 1.0 - 0.5 * r2 + 0.25 * r2 * r2 - 0.125 * r2 * r2 * r2;
        assert!((v.falloff(r) - expect).abs() < 1e-15);
    }
}
