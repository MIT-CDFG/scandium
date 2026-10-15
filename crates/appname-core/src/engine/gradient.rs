// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! §22.4 gradient engine — the stop ramp, the interpolation spaces, and
//! the fill-LAYER gradient geometry.
//!
//! Spec: contract `spec/schemas/document.v1.json` `$defs/gradient` (the
//! frozen v5.5 wire shape, identical to `fill.yaml`'s `$defs/gradient`),
//! ENGINEERING-SPEC §22.4, §34.4 property 10 (gradient monotonicity).
//!
//! # SCOPE: this module is the fill-LAYER half, NOT the on-canvas tool
//!
//! Tier-1 row 3 was SPLIT by O-INT ruling (`f429bc5`) after both doors the
//! on-canvas gradient tool would need were verified shut:
//! `spec/commands/fill.yaml` is `additionalProperties: false` and contains
//! ZERO `from`/`to` keys, so the drag endpoints §39.10 commits are a
//! stage-3 schema violation on the served contract; and
//! `filter.gradient_fill` is unregistered, with the kind registry closed
//! terminal at 102. The on-canvas tool is therefore a v5.6 fold-in row and
//! **nothing here is that tool** — no drag state, no tool door, no
//! command arm. What remains is exactly what the fill-LAYER gradient arm
//! needs, which is served.
//!
//! # What the frozen contract carries
//!
//! `gradient = {stops[≥2]{pos, color, opacity, midpoint}, kind, space,
//! dither}` with `kind ∈ {linear, radial, angular, reflected, diamond,
//! noise}` and `space ∈ {linearRgb, perceptual}`.
//!
//! # What it does NOT carry (v5.6 fold-in, NOT invented here)
//!
//! * **Gradient GEOMETRY on the `fill` COMMAND.** `fill.yaml`'s
//!   `$defs/gradient` has no `from`/`to`, no angle, no scale — a
//!   `fill{content:gradient}` command cannot say WHERE the ramp runs, and
//!   `additionalProperties: false` forbids adding them. Only the
//!   fill-LAYER shape (`document.v1.json` `$defs/fill`, gradient arm)
//!   carries `angleDeg`/`scale`/`reverse`, which is why
//!   [`GradientGeometry::from_layer_box`] is the module's only geometry
//!   ENTRY POINT. ([`GradientGeometry::from_drag`] is its crate-internal
//!   two-point axis primitive, not a tool door — see its own note.)
//! * **`noise` gradient parameters.** The `kind` enum admits `noise`, but
//!   the contract carries no roughness, no per-channel color model, and no
//!   RNG seed — nothing from which a noise ramp could be generated
//!   deterministically. [`GradientKind::Noise`] is decoded (it is in the
//!   frozen enum) and REFUSED typed at evaluation, never silently aliased
//!   onto another kind.
//!
//! # Pinned math
//!
//! All of the following are DERIVED, and each is stated where it is used
//! so a reviewer can re-derive it without reading output pixels.
//!
//! * **Midpoint warp.** A stop's `midpoint` m says where, within the
//!   segment to the NEXT stop, the half-way colour lands. The exponent
//!   form `f' = f^k` with `k = ln(0.5)/ln(m)` satisfies that identity
//!   exactly: at `f = m`, `f' = exp(ln(m)·ln(0.5)/ln(m)) = exp(ln 0.5) =
//!   0.5`. For `m ∈ (0,1)` we have `k > 0`, so `f ↦ f^k` is strictly
//!   increasing on `[0,1]` — this is what makes §34.4 property 10
//!   (monotonicity) hold. `m ≤ 0` and `m ≥ 1` are the degenerate limits of
//!   that same form and become steps (see [`midpoint_warp`]).
//! * **Interpolation is JOINT and PREMULTIPLIED**, per §22.4 [NORMATIVE]:
//!   "Color and opacity interpolate jointly, premultiplied: each stop
//!   converts to the interpolation space, is multiplied by its `opacity`,
//!   components lerp by `s′`, then the result un-premultiplies (guards
//!   ÷0) — this is what prevents dark fringes through transparent stops."
//!   The consequence is worth stating because it is what a straight lerp
//!   gets wrong: the result is the ALPHA-WEIGHTED average of the two stop
//!   colours, `c = (αₐcₐ(1−w) + α_b c_b w) / (αₐ(1−w) + α_b w)`, so a
//!   fully transparent stop contributes NO colour at all. Red → transparent
//!   black therefore stays pure red at falling alpha, where a straight lerp
//!   would walk the midtones through half-strength red and darken every
//!   fade. Alpha itself is ALWAYS the plain linear lerp — `space` selects
//!   the RGB space only, because neither Oklab nor linear RGB says
//!   anything about alpha, and `(α·L, α·a, α·b, α)` in §22.4's perceptual
//!   line lerps its fourth component unweighted.
//! * **Endpoint exactness.** At `f' = 0` / `f' = 1` the segment returns the
//!   stop colour BIT-EXACTLY rather than routing it through the
//!   interpolation space, so the `perceptual` round trip can never perturb
//!   a colour the user literally typed.
//!
//! # Modeling choices the contract does not pin (FLAGGED for O-INT)
//!
//! 1. **`space: perceptual` is realized as Oklab.** The contract names
//!    "perceptual" and pins nothing further; the §22 dossier does not
//!    elect a space either. Oklab is chosen because it needs no white
//!    point and no profile (unlike CIE L\*a\*b\*), so it adds no
//!    determinism knobs — but the ELECTION is unpinned and wants a v5.6
//!    ruling. Constants are Ottosson's published matrices.
//! 2. **A stop's `midpoint` governs the segment to its RIGHT** (stop i's
//!    midpoint warps segment i→i+1); the LAST stop's midpoint is inert.
//!    The contract stores `midpoint` per stop without saying which segment
//!    it owns.
//! 3. **`angleDeg` is measured counter-clockwise from +x in a y-UP frame**
//!    (the convention the value 90 = "upward" default implies), so the
//!    raster-space axis is `u = (cos θ, −sin θ)` because document y grows
//!    DOWNWARD. The contract pins neither the origin nor the sign.
//! 4. **The ÷0 guard's VALUE.** §22.4 says the un-premultiply "guards ÷0"
//!    and stops there. See [`lerp_stop`] for the guard chosen and why it
//!    is the continuous extension rather than an invention.
//!
//! # Divergence this module INHERITS, flagged not hidden
//!
//! **Non-finite numerics are REFUSED here, not clamped.** §12680
//! [NORMATIVE] (restating §0.3, and re-inlined in `layer.add.yaml`'s
//! notes) says numeric params arriving non-finite "are never a validation
//! error — `NaN` clamps to the field's documented default, `±∞` to the
//! field's documented min/max, and every clamp is recorded in the
//! command-validation detail". This module (and
//! [`crate::engine::fill_reg`]) instead answers a typed
//! `error.schema.out_of_range`. That is the STANDING behaviour already on
//! main — `kinds_guides.rs` refuses a non-finite `position` under a
//! comment literally headed "Non-finite policy" — and there is no
//! clamp-recording channel in the engine for the detail the letter
//! demands. Matching the letter in this one module would put a THIRD
//! behaviour in the engine, so the existing one is carried deliberately
//! and the divergence is raised rather than papered over. O-INT ruling
//! wanted; §0.3 line 1965 ("command validation rejects non-finite params")
//! and §12680 do not agree with each other either.
//!
//! Nothing in this module reads or writes the document; it is pure math so
//! that the compositor leg, the export leg and the tool preview all share
//! one library and never re-derive a constant (the C4/kernel-seam
//! precedent).

use crate::engine::error::{EngineError, ErrorCode};

// ---------------------------------------------------------------------------
// Oklab (modeling choice 1)
// ---------------------------------------------------------------------------

/// Straight LINEAR RGB → Oklab. Ottosson's published matrices.
///
/// Total on the whole real line: `f64::cbrt` takes the real cube root of a
/// negative argument, so an out-of-gamut (negative) linear component maps
/// without a NaN.
pub fn linear_rgb_to_oklab(c: [f64; 3]) -> [f64; 3] {
    let [r, g, b] = c;
    let l = 0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b;
    let m = 0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b;
    let s = 0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b;
    let (l_, m_, s_) = (l.cbrt(), m.cbrt(), s.cbrt());
    [
        0.210_454_255_3 * l_ + 0.793_617_785_0 * m_ - 0.004_072_046_8 * s_,
        1.977_998_495_1 * l_ - 2.428_592_205_0 * m_ + 0.450_593_709_9 * s_,
        0.025_904_037_1 * l_ + 0.782_771_766_2 * m_ - 0.808_675_766_0 * s_,
    ]
}

/// Oklab → straight LINEAR RGB (the inverse of [`linear_rgb_to_oklab`]).
pub fn oklab_to_linear_rgb(c: [f64; 3]) -> [f64; 3] {
    let [ll, aa, bb] = c;
    let l_ = ll + 0.396_337_777_4 * aa + 0.215_803_757_3 * bb;
    let m_ = ll - 0.105_561_345_8 * aa - 0.063_854_172_8 * bb;
    let s_ = ll - 0.089_484_177_5 * aa - 1.291_485_548_0 * bb;
    let (l, m, s) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    [
        4.076_741_662_1 * l - 3.307_711_591_3 * m + 0.230_969_929_2 * s,
        -1.268_438_004_6 * l + 2.609_757_401_1 * m - 0.341_319_396_5 * s,
        -0.004_196_086_3 * l - 0.703_418_614_7 * m + 1.707_614_701_0 * s,
    ]
}

// ---------------------------------------------------------------------------
// The stop ramp
// ---------------------------------------------------------------------------

/// One decoded gradient stop. `color` is STRAIGHT working-space linear
/// RGB; `alpha` already folds the stop's `opacity` into the colour's own
/// alpha (they multiply — the colour says how opaque the ink is, the stop
/// says how much of it this position gets).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradientStop {
    pub pos: f64,
    pub color: [f64; 3],
    pub alpha: f64,
    pub midpoint: f64,
}

/// §22.4 `kind` — the frozen v5.5 enum, in full.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GradientKind {
    Linear,
    Radial,
    Angular,
    Reflected,
    Diamond,
    /// In the frozen enum, but the contract carries NO parameters from
    /// which to synthesize it — refused typed at evaluation. v5.6 fold-in.
    Noise,
}

impl GradientKind {
    pub fn wire(self) -> &'static str {
        match self {
            GradientKind::Linear => "linear",
            GradientKind::Radial => "radial",
            GradientKind::Angular => "angular",
            GradientKind::Reflected => "reflected",
            GradientKind::Diamond => "diamond",
            GradientKind::Noise => "noise",
        }
    }

    pub fn from_wire(s: &str) -> Option<GradientKind> {
        Some(match s {
            "linear" => GradientKind::Linear,
            "radial" => GradientKind::Radial,
            "angular" => GradientKind::Angular,
            "reflected" => GradientKind::Reflected,
            "diamond" => GradientKind::Diamond,
            "noise" => GradientKind::Noise,
            _ => return None,
        })
    }
}

/// §22.4 `space` — which space the RGB lerp runs in (alpha is always
/// linear; see the module header).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GradientSpace {
    #[default]
    LinearRgb,
    /// Realized as Oklab — modeling choice 1 in the module header.
    Perceptual,
}

impl GradientSpace {
    pub fn wire(self) -> &'static str {
        match self {
            GradientSpace::LinearRgb => "linearRgb",
            GradientSpace::Perceptual => "perceptual",
        }
    }

    pub fn from_wire(s: &str) -> Option<GradientSpace> {
        Some(match s {
            "linearRgb" => GradientSpace::LinearRgb,
            "perceptual" => GradientSpace::Perceptual,
            _ => return None,
        })
    }
}

/// A decoded §22.4 gradient: the ramp plus how to interpolate it.
#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    /// Sorted by `pos`, length ≥ 2 (both invariants established by
    /// [`Gradient::new`] and relied on by [`Gradient::eval`]).
    stops: Vec<GradientStop>,
    kind: GradientKind,
    space: GradientSpace,
    dither: bool,
}

impl Gradient {
    /// Build a gradient, establishing the two invariants `eval` needs:
    /// at least two stops, sorted by `pos`.
    ///
    /// The sort is STABLE, so stops that share a `pos` keep their wire
    /// order and the "later stop wins at a zero-width segment" rule below
    /// stays a function of the authored order, not of sort luck.
    pub fn new(
        mut stops: Vec<GradientStop>,
        kind: GradientKind,
        space: GradientSpace,
        dither: bool,
    ) -> Result<Gradient, EngineError> {
        if stops.len() < 2 {
            return Err(EngineError::range(
                "gradient.stops",
                "a gradient needs at least 2 stops (§22.4 minItems)",
            ));
        }
        for (i, s) in stops.iter().enumerate() {
            if !(s.pos.is_finite() && (0.0..=1.0).contains(&s.pos)) {
                return Err(EngineError::range(
                    &format!("gradient.stops/{i}/pos"),
                    "stop pos must be a finite number in 0..=1 (§22.4)",
                ));
            }
            if !(s.midpoint.is_finite() && (0.0..=1.0).contains(&s.midpoint)) {
                return Err(EngineError::range(
                    &format!("gradient.stops/{i}/midpoint"),
                    "stop midpoint must be a finite number in 0..=1 (§22.4)",
                ));
            }
            if !(s.alpha.is_finite() && (0.0..=1.0).contains(&s.alpha)) {
                return Err(EngineError::range(
                    &format!("gradient.stops/{i}/opacity"),
                    "stop opacity must be a finite number in 0..=1 (§22.4)",
                ));
            }
            if !s.color.iter().all(|v| v.is_finite()) {
                return Err(EngineError::range(
                    &format!("gradient.stops/{i}/color"),
                    "stop colour components must be finite",
                ));
            }
        }
        stops.sort_by(|a, b| a.pos.partial_cmp(&b.pos).expect("pos checked finite above"));
        Ok(Gradient {
            stops,
            kind,
            space,
            dither,
        })
    }

    pub fn stops(&self) -> &[GradientStop] {
        &self.stops
    }

    pub fn kind(&self) -> GradientKind {
        self.kind
    }

    pub fn space(&self) -> GradientSpace {
        self.space
    }

    pub fn dither(&self) -> bool {
        self.dither
    }

    /// Evaluate the ramp at `t`, returning STRAIGHT linear RGBA.
    ///
    /// `t` is clamped to `[0,1]`; below the first stop and above the last
    /// the ramp holds that stop's colour flat (the §22.4 "ends extend"
    /// reading, and the only reading that keeps a ramp whose stops do not
    /// reach 0/1 well-defined).
    pub fn eval(&self, t: f64) -> [f64; 4] {
        let t = if t.is_finite() {
            t.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let first = self.stops[0];
        let last = self.stops[self.stops.len() - 1];
        if t <= first.pos {
            return [first.color[0], first.color[1], first.color[2], first.alpha];
        }
        if t >= last.pos {
            return [last.color[0], last.color[1], last.color[2], last.alpha];
        }
        // Pick the LAST segment whose left stop is at or before t. The
        // `rposition` (rather than `position`) is what makes a HARD STOP —
        // two stops sharing one `pos` — RIGHT-CONTINUOUS: at exactly the
        // shared position the ramp has already jumped, so the later stop's
        // colour is the one returned. Left-biasing here would instead hold
        // the earlier colour through the seam, which reads as a one-sample
        // lag at every hard stop.
        //
        // `stops` is sorted and t lies strictly inside (first.pos,
        // last.pos), so such a segment exists.
        let i = self
            .stops
            .windows(2)
            .rposition(|w| w[0].pos <= t)
            .expect("t > first.pos, so the first window always qualifies");
        let (a, b) = (self.stops[i], self.stops[i + 1]);
        let span = b.pos - a.pos;
        if !(span > 0.0) {
            // UNREACHABLE BY CONSTRUCTION, kept as a division guard.
            // Proof: a zero-span window i has stops[i].pos ==
            // stops[i+1].pos. For `rposition` to select it, no later
            // window may have w[0].pos <= t, so i is the last window and
            // stops[i+1] is the LAST stop — giving last.pos ==
            // stops[i].pos <= t, which the `t >= last.pos` early return
            // above has already handled. Returning the left stop keeps
            // this consistent with the f = 0 result the code below would
            // produce, and never divides by zero.
            return [a.color[0], a.color[1], a.color[2], a.alpha];
        }
        let f = (t - a.pos) / span;
        let w = midpoint_warp(f, a.midpoint);
        lerp_stop(&a, &b, w, self.space)
    }
}

/// The §22.4 midpoint warp: reparameterize a segment so that the half-way
/// COLOUR lands at `f = m`.
///
/// Derivation (module header): `f' = f^k` with `k = ln(0.5)/ln(m)` gives
/// `f'(m) = 0.5` exactly, and `k > 0` for `m ∈ (0,1)` makes it strictly
/// increasing — which is what §34.4 property 10 asserts.
///
/// The two endpoints of the legal `midpoint` range are the LIMITS of that
/// same expression, not special cases invented here:
/// * as `m → 0⁺`, `k → 0⁺`, so `f^k → 1` for every `f > 0` — a step at 0;
/// * as `m → 1⁻`, `k → +∞`, so `f^k → 0` for every `f < 1` — a step at 1.
pub fn midpoint_warp(f: f64, m: f64) -> f64 {
    let f = if f.is_finite() {
        f.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if !m.is_finite() || m <= 0.0 {
        return if f > 0.0 { 1.0 } else { 0.0 };
    }
    if m >= 1.0 {
        return if f < 1.0 { 0.0 } else { 1.0 };
    }
    if m == 0.5 {
        // Exactly the identity warp; k = 1 and powf would only add
        // rounding. Keeping it exact means the overwhelmingly common
        // default costs nothing and is bit-clean.
        return f;
    }
    // k = ln(0.5) / ln(m); ln(0.5) = -ln(2).
    let k = -std::f64::consts::LN_2 / m.ln();
    f.powf(k)
}

/// Interpolate between two stops at already-warped fraction `w`,
/// returning STRAIGHT linear RGBA.
///
/// §22.4 [NORMATIVE], to the letter: each stop converts to the
/// interpolation space, is multiplied by its opacity, all components lerp
/// by `w`, and the result un-premultiplies. Alpha lerps linearly and is
/// returned straight.
pub(crate) fn lerp_stop(
    a: &GradientStop,
    b: &GradientStop,
    w: f64,
    space: GradientSpace,
) -> [f64; 4] {
    // Endpoint exactness (module header): never route a stop colour the
    // user typed through the interpolation space's round trip. This also
    // keeps the two ends well-defined when a stop is fully transparent,
    // where the premultiplied form below carries no colour information.
    if w <= 0.0 {
        return [a.color[0], a.color[1], a.color[2], a.alpha];
    }
    if w >= 1.0 {
        return [b.color[0], b.color[1], b.color[2], b.alpha];
    }
    let alpha = a.alpha + (b.alpha - a.alpha) * w;
    // "each stop converts to the interpolation space" — for `linearRgb`
    // that space IS the stop's own storage, so the conversion is identity.
    let (ca, cb) = match space {
        GradientSpace::LinearRgb => (a.color, b.color),
        GradientSpace::Perceptual => (linear_rgb_to_oklab(a.color), linear_rgb_to_oklab(b.color)),
    };
    let mut mixed = [0.0_f64; 3];
    if alpha > 0.0 {
        for i in 0..3 {
            // "…is multiplied by its opacity, components lerp by s′, then
            // the result un-premultiplies".
            let pa = ca[i] * a.alpha;
            let pb = cb[i] * b.alpha;
            mixed[i] = (pa + (pb - pa) * w) / alpha;
        }
    } else {
        // The ÷0 guard §22.4 calls for but does not value (modeling
        // choice 4). Alpha lerps linearly, so `alpha == 0` strictly
        // inside a segment forces BOTH stop alphas to 0, which zeroes
        // every premultiplied component: the ratio is a genuine 0/0, not
        // a large-over-small. The plain lerp of the two stop colours is
        // used, because that is exactly what the premultiplied form
        // returns for every EQUAL non-zero alpha pair — put α on both
        // stops and it cancels: `(αcₐ + α(c_b − cₐ)w)/α = cₐ + (c_b −
        // cₐ)w` — so this value is the limit the guarded case is missing,
        // not a new rule. Nothing is visible at α = 0 regardless; the
        // point is that the number is defined and stable.
        for i in 0..3 {
            mixed[i] = ca[i] + (cb[i] - ca[i]) * w;
        }
    }
    let rgb = match space {
        GradientSpace::LinearRgb => mixed,
        GradientSpace::Perceptual => oklab_to_linear_rgb(mixed),
    };
    [rgb[0], rgb[1], rgb[2], alpha]
}

// ---------------------------------------------------------------------------
// The gradient TOOL geometry
// ---------------------------------------------------------------------------

/// The gradient axis in DOCUMENT pixel space: a start point and an end
/// point. Every §22.4 geometric kind is a different scalar field over this
/// one axis, which is why the tool needs exactly this and nothing more.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradientGeometry {
    p0: [f64; 2],
    p1: [f64; 2],
    /// Unit vector along the axis (`d / |d|`), cached.
    u: [f64; 2],
    /// `|d|`, cached; strictly positive by construction.
    len: f64,
}

impl GradientGeometry {
    /// The two-point axis primitive: build the frame from an explicit
    /// start → end pair.
    ///
    /// NOT A TOOL DOOR, deliberately `pub(crate)`. The on-canvas gradient
    /// tool is a v5.6 fold-in with no wire home (module header), and this
    /// constructor must not read as its engine half — nothing outside the
    /// crate can reach it, so no shell surface can accidentally bind to
    /// it. It exists because [`GradientGeometry::from_layer_box`] derives
    /// an axis and then needs somewhere to put it, and because every
    /// `t(p)` derivation below is stated in terms of two points.
    ///
    /// A degenerate (zero-length) axis is REFUSED rather than defaulted:
    /// every kind below divides by `|d|`, and silently substituting a
    /// direction would invent geometry nobody specified.
    pub(crate) fn from_drag(p0: [f64; 2], p1: [f64; 2]) -> Result<GradientGeometry, EngineError> {
        if !p0.iter().chain(p1.iter()).all(|v| v.is_finite()) {
            return Err(EngineError::range(
                "gradient.geometry",
                "gradient endpoints must be finite",
            ));
        }
        let d = [p1[0] - p0[0], p1[1] - p0[1]];
        let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
        if !(len > 0.0) {
            return Err(EngineError::range(
                "gradient.geometry",
                "a zero-length gradient drag has no direction (§22.4)",
            ));
        }
        Ok(GradientGeometry {
            p0,
            p1,
            u: [d[0] / len, d[1] / len],
            len,
        })
    }

    /// The FILL-LAYER form: the served v5.5 shape. A gradient fill layer
    /// carries `angleDeg` + `scale` and no points, so the axis is derived
    /// from the box it fills.
    ///
    /// The axis is centred on the box and long enough that, at `scale =
    /// 1`, the ramp exactly spans the box along the chosen direction: the
    /// support of a `w × h` box projected onto direction `u` has half-width
    /// `(w·|uₓ| + h·|u_y|) / 2`, which is the box's support function in
    /// that direction. `scale` multiplies it.
    ///
    /// `angleDeg` follows modeling choice 3 (CCW from +x, y-UP), so the
    /// raster direction is `u = (cos θ, −sin θ)`.
    pub fn from_layer_box(
        box_min: [f64; 2],
        box_max: [f64; 2],
        angle_deg: f64,
        scale: f64,
    ) -> Result<GradientGeometry, EngineError> {
        if !box_min.iter().chain(box_max.iter()).all(|v| v.is_finite())
            || !angle_deg.is_finite()
            || !scale.is_finite()
        {
            return Err(EngineError::range(
                "fill.gradient",
                "gradient box, angleDeg and scale must be finite",
            ));
        }
        if !(scale > 0.0) {
            return Err(EngineError::range(
                "fill.scale",
                "scale must be > 0 (§7.3 exclusiveMinimum)",
            ));
        }
        let (w, h) = (box_max[0] - box_min[0], box_max[1] - box_min[1]);
        if !(w > 0.0 && h > 0.0) {
            return Err(EngineError::range(
                "fill.gradient",
                "a gradient fill needs a non-empty box",
            ));
        }
        let theta = angle_deg.to_radians();
        // y-UP angle → y-DOWN raster direction (modeling choice 3).
        let u = [theta.cos(), -theta.sin()];
        let half = scale * (w * u[0].abs() + h * u[1].abs()) / 2.0;
        if !(half > 0.0) {
            return Err(EngineError::range(
                "fill.gradient",
                "degenerate gradient axis for this box/angle/scale",
            ));
        }
        let c = [
            (box_min[0] + box_max[0]) / 2.0,
            (box_min[1] + box_max[1]) / 2.0,
        ];
        GradientGeometry::from_drag(
            [c[0] - half * u[0], c[1] - half * u[1]],
            [c[0] + half * u[0], c[1] + half * u[1]],
        )
    }

    pub fn start(&self) -> [f64; 2] {
        self.p0
    }

    pub fn end(&self) -> [f64; 2] {
        self.p1
    }

    pub fn length(&self) -> f64 {
        self.len
    }

    /// Axis-frame coordinates of `p`: `(a, b)` where `a` runs along the
    /// axis (0 at `p0`, `len` at `p1`) and `b` is the signed perpendicular
    /// distance. Every kind below is a formula in these two numbers.
    fn frame(&self, p: [f64; 2]) -> (f64, f64) {
        let q = [p[0] - self.p0[0], p[1] - self.p0[1]];
        // v = u rotated +90° in the raster frame.
        let a = q[0] * self.u[0] + q[1] * self.u[1];
        let b = -q[0] * self.u[1] + q[1] * self.u[0];
        (a, b)
    }

    /// The §22.4 ramp parameter at document-space point `p`.
    ///
    /// The §22.4 [NORMATIVE] `t(p)` table, TRANSCRIBED — not re-derived.
    /// The clause defines the gradient frame as the DIMENSIONLESS pair
    /// `u = dot(p − p₀, d̂)/|d|`, `v = dot(p − p₀, perp(d̂))/|d|` ("same
    /// scale both axes"); [`GradientGeometry::frame`] returns that pair in
    /// PIXELS as `(a, b)`, so `u = a/L` and `v = b/L` with `L = |d|`.
    /// Extend mode is clamp-to-edge in v1, which is where the `clamp`s
    /// come from.
    ///
    /// * **linear** — spec `t = dot(p − p₀, d)/|d|²`. Since
    ///   `dot(q, d) = dot(q, d̂)·|d| = a·L`, that is `a·L/L² = a/L = u`.
    /// * **radial** — spec `t = |p − p₀|/|d| = √(a²+b²)/L = √(u²+v²)`;
    ///   iso-lines are circles centred on `p0` reaching the end colour at
    ///   radius `L`.
    /// * **angular** — spec `t = ((atan2(p_y−p₀y, p_x−p₀x) − θ₀)/2π) mod 1`
    ///   with `θ₀ = atan2(d_y, d_x)`. `(a, b)` is `q` already rotated by
    ///   `−θ₀`, so `atan2(b, a)` IS that angle difference; the fold below
    ///   is the `mod 1`. This kind WRAPS rather than clamps, so it has an
    ///   inherent seam on the axis's negative side where `t` jumps 1 → 0.
    /// * **reflected** — spec `t = |2·t_lin − 1|` with `t_lin` the CLAMPED
    ///   linear `t`, glossed "mirror about the gesture midpoint". So the
    ///   FIRST stop's colour runs down the middle of the axis (`t_lin =
    ///   ½ ⇒ t = 0`) and the last stop's colour lands at BOTH ends
    ///   (`t_lin ∈ {0, 1} ⇒ t = 1`). Because `t_lin ∈ [0,1]`, `t ∈ [0,1]`
    ///   already and needs no further clamp.
    /// * **diamond** — spec `t = max(|u|, |v|)`, glossed "(Chebyshev
    ///   distance in the frame)". The L^∞ ball, NOT the L¹ one.
    ///
    /// # Two transcription defects this arm previously carried [FIXED]
    ///
    /// `reflected` was `|a|/L = |u|` (mirror about `p₀`) and `diamond` was
    /// `(|a|+|b|)/L = |u|+|v|` (L¹ / taxicab). Both were plausible
    /// re-derivations from the kind NAMES rather than transcriptions of
    /// the [NORMATIVE] table, and both are wrong against it. The
    /// `reflected` error was invisible while nothing rendered, and would
    /// have been invisible AFTER rendering too: on a fill layer the axis
    /// spans the box exactly, so `u ∈ [0,1]` over the whole box and
    /// `|u| ≡ u` — the reflected kind would have rendered PIXEL-IDENTICAL
    /// to linear, a silently dead enum arm rather than a visibly wrong
    /// one. `reflected_is_a_mirror_about_the_axis_midpoint_not_the_start`
    /// pins exactly that difference.
    ///
    /// NOTE for O-INT, not silently resolved here: `max(|u|,|v|)` renders
    /// an AXIS-ALIGNED square in the gradient frame, whereas the L¹ form
    /// renders a square rotated 45° to the axis — which is the shape the
    /// word "diamond" describes, and the shape the reference application
    /// produces. The spec states the Chebyshev form TWICE (formula and
    /// parenthetical gloss), so the letter is unambiguous and is what is
    /// implemented; whether the letter itself is the intent is a §22.4
    /// erratum question, and a parity row, that belongs to O-INT.
    /// * **angular** — `t = atan2(b, a) / 2π` folded into `[0,1)`: the
    ///   sweep angle about `p0` measured from the axis. This kind WRAPS
    ///   rather than clamps, so it has an inherent seam on the axis's
    ///   negative side where `t` jumps 1 → 0.
    /// * **noise** — refused; see [`GradientKind::Noise`].
    pub fn parameter_at(&self, kind: GradientKind, p: [f64; 2]) -> Result<f64, EngineError> {
        if !p.iter().all(|v| v.is_finite()) {
            return Err(EngineError::range(
                "gradient.point",
                "sample point must be finite",
            ));
        }
        let (a, b) = self.frame(p);
        Ok(match kind {
            GradientKind::Linear => (a / self.len).clamp(0.0, 1.0),
            GradientKind::Reflected => {
                // t = |2·t_lin − 1|, t_lin = CLAMPED linear t (§22.4).
                let t_lin = (a / self.len).clamp(0.0, 1.0);
                (2.0 * t_lin - 1.0).abs()
            }
            GradientKind::Radial => ((a * a + b * b).sqrt() / self.len).clamp(0.0, 1.0),
            GradientKind::Diamond => {
                // t = max(|u|, |v|) — Chebyshev in the frame (§22.4).
                let (u, v) = (a / self.len, b / self.len);
                u.abs().max(v.abs()).clamp(0.0, 1.0)
            }
            GradientKind::Angular => {
                let turn = b.atan2(a) / std::f64::consts::TAU;
                // atan2 ∈ (-π, π] ⇒ turn ∈ (-0.5, 0.5]; fold to [0,1).
                let t = if turn < 0.0 { turn + 1.0 } else { turn };
                // turn == 0.5 folds to exactly 1.0, which is the same
                // sweep position as 0.0 — normalize so the range is
                // half-open and the seam has ONE representation.
                if t >= 1.0 {
                    0.0
                } else {
                    t
                }
            }
            GradientKind::Noise => {
                return Err(EngineError::new(
                    ErrorCode::NotImplemented,
                    "gradient kind \"noise\" is in the frozen §22.4 enum but the contract \
                     carries no roughness / colour-model / seed from which to synthesize it \
                     — v5.6 fold-in, refused typed rather than aliased",
                ))
            }
        })
    }

    /// Sample the gradient at a document-space point: geometry → ramp.
    pub fn sample(
        &self,
        g: &Gradient,
        p: [f64; 2],
        reverse: bool,
    ) -> Result<[f64; 4], EngineError> {
        let t = self.parameter_at(g.kind(), p)?;
        Ok(g.eval(if reverse { 1.0 - t } else { t }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- tolerance derivations -------------------------------------------
    //
    // TOL_ULP = 1e-12. The identities asserted with it (midpoint(m) = 0.5,
    // linear t(p0) = 0, t(p1) = 1) are exact in real arithmetic; in f64
    // each is a composition of a handful of operations whose relative
    // error is a small multiple of eps = 2^-52 ≈ 2.2e-16. For the
    // midpoint identity the sharpest term is exp(k·ln f) with k·ln f =
    // ln 0.5, so the absolute error is bounded by ~0.5·|ln 0.5|·c·eps
    // with c a small constant — order 1e-15. 1e-12 sits ~3 decades above
    // that bound and ~9 decades below anything observable.
    const TOL_ULP: f64 = 1e-12;

    // TOL_OKLAB = 1e-6. Ottosson's forward and inverse matrices are
    // published to 10 decimal places, so their product differs from the
    // identity by ~1e-10 per entry; summed over a row with |coef| ≲ 4 the
    // round-trip error is order 1e-9. 1e-6 is three decades above that
    // bound and still three decades BELOW the 8-bit observability floor
    // below, so it separates "the matrices are inverses" from "a pixel
    // moved".
    const TOL_OKLAB: f64 = 1e-6;

    // An 8-bit channel code changes only when the ENCODED value moves by
    // half a code: 0.5/255 ≈ 1.96e-3. Anything below this cannot be seen
    // in an 8-bit export; it is quoted (not used as a pass tolerance) so
    // the tolerances above can be read against it.
    const U8_HALF_CODE: f64 = 0.5 / 255.0;

    fn stop(pos: f64, v: f64) -> GradientStop {
        GradientStop {
            pos,
            color: [v, v, v],
            alpha: 1.0,
            midpoint: 0.5,
        }
    }

    fn ramp(kind: GradientKind, space: GradientSpace) -> Gradient {
        Gradient::new(vec![stop(0.0, 0.0), stop(1.0, 1.0)], kind, space, true).unwrap()
    }

    // -- the midpoint identity -------------------------------------------

    #[test]
    fn midpoint_puts_the_half_way_colour_at_the_midpoint() {
        // THE defining identity of `midpoint`: warp(m, m) == 0.5.
        for m in [0.05, 0.1, 0.25, 0.4, 0.5, 0.6, 0.75, 0.9, 0.95] {
            let got = midpoint_warp(m, m);
            assert!(
                (got - 0.5).abs() < TOL_ULP,
                "midpoint {m}: warp(m,m) = {got}, want 0.5 (tol {TOL_ULP})"
            );
        }
    }

    #[test]
    fn midpoint_warp_is_strictly_increasing() {
        // §34.4 property 10's precondition: the warp cannot fold the ramp
        // back on itself, or a monotone stop pair would stop being
        // monotone in the output.
        for m in [0.05, 0.25, 0.5, 0.75, 0.95] {
            let mut prev = f64::NEG_INFINITY;
            for i in 0..=1000 {
                let f = f64::from(i) / 1000.0;
                let w = midpoint_warp(f, m);
                assert!(
                    w > prev || (i == 0 && w == 0.0),
                    "midpoint {m}: warp not increasing at f = {f} ({prev} -> {w})"
                );
                assert!(
                    (0.0..=1.0).contains(&w),
                    "midpoint {m}: warp {w} left [0,1]"
                );
                prev = w;
            }
        }
    }

    #[test]
    fn midpoint_warp_endpoints_are_exact() {
        for m in [0.0, 0.05, 0.5, 0.95, 1.0] {
            assert_eq!(
                midpoint_warp(0.0, m),
                0.0,
                "midpoint {m}: warp(0) must be 0"
            );
            assert_eq!(
                midpoint_warp(1.0, m),
                1.0,
                "midpoint {m}: warp(1) must be 1"
            );
        }
    }

    #[test]
    fn degenerate_midpoints_are_the_limits_of_the_same_form() {
        // m -> 0 is a step at 0: everything past the start is the far
        // colour. m -> 1 is a step at 1.
        assert_eq!(midpoint_warp(1e-9, 0.0), 1.0);
        assert_eq!(midpoint_warp(0.5, 0.0), 1.0);
        assert_eq!(midpoint_warp(0.0, 0.0), 0.0);
        assert_eq!(midpoint_warp(0.5, 1.0), 0.0);
        assert_eq!(midpoint_warp(1.0 - 1e-9, 1.0), 0.0);
        assert_eq!(midpoint_warp(1.0, 1.0), 1.0);
        // And the form APPROACHES those limits — but only LOGARITHMICALLY,
        // because k = ln(0.5)/ln(m) -> 0 like 1/|ln m|. Hand-derived
        // reference points (NOT read off the implementation):
        //   m = 0.01: k = ln0.5/ln0.01 = 0.693147/4.605170 = 0.1505150,
        //             warp(0.5) = exp(-0.1505150 * 0.693147) = 0.900929;
        //   m = 0.99: k = 0.693147/0.0100503 = 68.968,
        //             warp(0.5) = 2^-68.968 ~ 3e-21.
        let lo = midpoint_warp(0.5, 0.01);
        assert!(
            (lo - 0.900_929).abs() < 1e-5,
            "m=0.01: warp(0.5) = {lo}, derived 0.900929"
        );
        assert!(
            midpoint_warp(0.5, 0.99) < 1e-20,
            "m=0.99 is all but a step at 1"
        );
        // Monotone in m: a smaller midpoint pushes the half-way colour
        // earlier, so warp(0.5, m) must fall as m rises.
        let mut prev = f64::INFINITY;
        for m in [0.001, 0.01, 0.1, 0.3, 0.5, 0.7, 0.9, 0.99] {
            let w = midpoint_warp(0.5, m);
            assert!(
                w < prev,
                "warp(0.5, m) must fall as m rises (m={m}: {w} !< {prev})"
            );
            prev = w;
        }
    }

    #[test]
    fn default_midpoint_is_the_identity_warp() {
        for i in 0..=100 {
            let f = f64::from(i) / 100.0;
            assert_eq!(midpoint_warp(f, 0.5), f, "default midpoint must not warp");
        }
    }

    // -- the ramp ---------------------------------------------------------

    #[test]
    fn ramp_holds_the_end_stops_flat_outside_their_range() {
        let g = Gradient::new(
            vec![stop(0.25, 0.2), stop(0.75, 0.8)],
            GradientKind::Linear,
            GradientSpace::LinearRgb,
            true,
        )
        .unwrap();
        assert_eq!(g.eval(0.0)[0], 0.2, "below the first stop holds it flat");
        assert_eq!(g.eval(0.25)[0], 0.2);
        assert_eq!(g.eval(0.75)[0], 0.8);
        assert_eq!(g.eval(1.0)[0], 0.8, "above the last stop holds it flat");
    }

    #[test]
    fn ramp_stop_colours_are_returned_bit_exactly() {
        // Endpoint exactness, in the space where it is at risk: a
        // perceptual ramp must not round-trip a typed colour.
        let a = GradientStop {
            pos: 0.0,
            color: [0.1234, 0.5678, 0.9012],
            alpha: 0.75,
            midpoint: 0.5,
        };
        let b = GradientStop {
            pos: 1.0,
            color: [0.9, 0.2, 0.3],
            alpha: 0.25,
            midpoint: 0.5,
        };
        let g = Gradient::new(
            vec![a, b],
            GradientKind::Linear,
            GradientSpace::Perceptual,
            true,
        )
        .unwrap();
        assert_eq!(g.eval(0.0), [a.color[0], a.color[1], a.color[2], a.alpha]);
        assert_eq!(g.eval(1.0), [b.color[0], b.color[1], b.color[2], b.alpha]);
    }

    #[test]
    fn ramp_is_monotone_per_channel_property_10() {
        // §34.4 property 10 (gradient monotonicity), in both spaces and
        // across midpoints: a ramp between two monotone stops must not
        // reverse anywhere.
        for space in [GradientSpace::LinearRgb, GradientSpace::Perceptual] {
            // Monotonicity can only be asserted to the precision of the
            // space it is measured in. linearRgb is a plain lerp, so it is
            // monotone to a few ulp. `perceptual` round-trips through
            // Oklab, whose forward/inverse matrices are published to 10
            // decimals and therefore compose to the identity only within
            // ~1e-9 (see TOL_OKLAB) — a ramp there can wobble by that much
            // and still be monotone in exact arithmetic. Using TOL_ULP for
            // both would assert a precision the perceptual space does not
            // have; using TOL_OKLAB for both would blind the linear case.
            let tol = match space {
                GradientSpace::LinearRgb => TOL_ULP,
                GradientSpace::Perceptual => TOL_OKLAB,
            };
            for m in [0.05, 0.5, 0.95] {
                let g = Gradient::new(
                    vec![
                        GradientStop {
                            pos: 0.0,
                            color: [0.05, 0.1, 0.2],
                            alpha: 0.0,
                            midpoint: m,
                        },
                        GradientStop {
                            pos: 1.0,
                            color: [0.85, 0.7, 0.95],
                            alpha: 1.0,
                            midpoint: 0.5,
                        },
                    ],
                    GradientKind::Linear,
                    space,
                    true,
                )
                .unwrap();
                let mut prev = [f64::NEG_INFINITY; 4];
                let mut worst_dip = 0.0_f64;
                for i in 0..=2000 {
                    let t = f64::from(i) / 2000.0;
                    let c = g.eval(t);
                    for ch in 0..4 {
                        assert!(
                            c[ch] >= prev[ch] - tol,
                            "{:?} m={m}: channel {ch} fell at t={t} ({} -> {})",
                            space,
                            prev[ch],
                            c[ch]
                        );
                        if prev[ch].is_finite() {
                            worst_dip = worst_dip.max(prev[ch] - c[ch]);
                        }
                        prev[ch] = c[ch];
                    }
                }
                // The tolerance above is slack ONLY in the perceptual
                // space, and that slack must stay far below anything a
                // pixel could show. Pinning the ACTUAL worst dip here
                // stops the loose tolerance from hiding a real reversal:
                // if the ramp ever genuinely folded back, the dip would
                // blow past the 8-bit floor and this catches it even
                // though the per-sample assert above would not.
                assert!(
                    worst_dip < U8_HALF_CODE,
                    "{space:?} m={m}: worst dip {worst_dip} reached 8-bit visibility"
                );
            }
        }
    }

    #[test]
    fn alpha_interpolates_linearly_in_both_spaces() {
        // `space` selects the RGB space ONLY (module header). Alpha must
        // be identical either way.
        let mk = |space| {
            Gradient::new(
                vec![
                    GradientStop {
                        pos: 0.0,
                        color: [0.2, 0.4, 0.6],
                        alpha: 0.0,
                        midpoint: 0.5,
                    },
                    GradientStop {
                        pos: 1.0,
                        color: [0.8, 0.5, 0.1],
                        alpha: 1.0,
                        midpoint: 0.5,
                    },
                ],
                GradientKind::Linear,
                space,
                true,
            )
            .unwrap()
        };
        let lin = mk(GradientSpace::LinearRgb);
        let per = mk(GradientSpace::Perceptual);
        for i in 1..100 {
            let t = f64::from(i) / 100.0;
            assert!(
                (lin.eval(t)[3] - t).abs() < TOL_ULP,
                "alpha must be the linear lerp"
            );
            assert_eq!(
                lin.eval(t)[3],
                per.eval(t)[3],
                "space must not touch alpha at t={t}"
            );
        }
    }

    // -- §22.4 premultiplied joint interpolation --------------------------

    fn two(a: GradientStop, b: GradientStop, space: GradientSpace) -> Gradient {
        Gradient::new(vec![a, b], GradientKind::Linear, space, true).unwrap()
    }

    #[test]
    fn a_fully_transparent_stop_contributes_no_colour() {
        // §22.4's stated PURPOSE for premultiplied interpolation: "this is
        // what prevents dark fringes through transparent stops".
        //
        // Derivation, exact and independent of the implementation: with
        // alpha_a = 1 and alpha_b = 0 the lerped alpha is (1 - w), the
        // premultiplied b term is identically zero, so the un-premultiplied
        // result is c_a * (1 - w) / (1 - w) = c_a for EVERY w in (0,1). A
        // red-to-transparent fade stays pure red the whole way down.
        //
        // A straight lerp instead returns c_a + (c_b - c_a)*w, which at
        // w = 0.5 is half-way to the transparent stop's colour — the dark
        // fringe. The gap is asserted against U8_HALF_CODE so the test
        // fails on visibility, not on float noise.
        let opaque_red = GradientStop {
            pos: 0.0,
            color: [1.0, 0.0, 0.0],
            alpha: 1.0,
            midpoint: 0.5,
        };
        let transparent_blue = GradientStop {
            pos: 1.0,
            color: [0.0, 0.0, 1.0],
            alpha: 0.0,
            midpoint: 0.5,
        };
        for space in [GradientSpace::LinearRgb, GradientSpace::Perceptual] {
            // `perceptual` round-trips through Oklab, so it can only hold
            // the colour to the matrices' inverse precision (TOL_OKLAB).
            let tol = match space {
                GradientSpace::LinearRgb => TOL_ULP,
                GradientSpace::Perceptual => TOL_OKLAB,
            };
            let g = two(opaque_red, transparent_blue, space);
            let mut worst_straight_gap = 0.0_f64;
            for i in 1..100 {
                let t = f64::from(i) / 100.0;
                let c = g.eval(t);
                for ch in 0..3 {
                    assert!(
                        (c[ch] - opaque_red.color[ch]).abs() < tol,
                        "{space:?} t={t}: channel {ch} = {} drifted off the opaque \
                         stop's colour {} — the transparent stop leaked colour in",
                        c[ch],
                        opaque_red.color[ch]
                    );
                    // What a straight lerp would have produced here.
                    let straight = opaque_red.color[ch]
                        + (transparent_blue.color[ch] - opaque_red.color[ch]) * t;
                    worst_straight_gap = worst_straight_gap.max((straight - c[ch]).abs());
                }
                assert!(
                    (c[3] - (1.0 - t)).abs() < TOL_ULP,
                    "{space:?} t={t}: alpha must still be the plain linear lerp"
                );
            }
            assert!(
                worst_straight_gap > U8_HALF_CODE,
                "{space:?}: premultiplied and straight must differ VISIBLY here, \
                 else this test could not tell them apart (gap {worst_straight_gap})"
            );
        }
    }

    #[test]
    fn premultiplied_mix_matches_the_hand_derived_value() {
        // Hand-derived, NOT read off the implementation. Stops
        //   A = (0.8, 0.4, 0.2) at alpha 0.25,  B = (0.2, 0.6, 1.0) at alpha 0.75,
        // sampled at w = 0.5 with the default (identity) midpoint:
        //   premultiplied A = (0.20, 0.10, 0.05, 0.25)
        //   premultiplied B = (0.15, 0.45, 0.75, 0.75)
        //   lerp at 0.5     = (0.175, 0.275, 0.400, 0.50)
        //   un-premultiply  = (0.35, 0.55, 0.80) at alpha 0.50
        // A straight lerp would give (0.50, 0.50, 0.60) — every channel
        // differs by far more than an 8-bit code.
        let a = GradientStop {
            pos: 0.0,
            color: [0.8, 0.4, 0.2],
            alpha: 0.25,
            midpoint: 0.5,
        };
        let b = GradientStop {
            pos: 1.0,
            color: [0.2, 0.6, 1.0],
            alpha: 0.75,
            midpoint: 0.5,
        };
        let got = two(a, b, GradientSpace::LinearRgb).eval(0.5);
        for (ch, want) in [0.35, 0.55, 0.80, 0.50].iter().enumerate() {
            assert!(
                (got[ch] - want).abs() < TOL_ULP,
                "channel {ch}: got {}, hand-derived {want}",
                got[ch]
            );
        }
    }

    #[test]
    fn equal_alphas_reduce_to_the_plain_lerp() {
        // The premultiplied form is not a different curve when the two
        // stops share an alpha — alpha cancels: (a*c_a + a*(c_b - c_a)w)/a.
        // This is also the derivation the div-zero guard leans on, so it
        // is pinned rather than asserted in a comment.
        for alpha in [0.25, 0.5, 1.0] {
            let a = GradientStop {
                pos: 0.0,
                color: [0.1, 0.7, 0.3],
                alpha,
                midpoint: 0.5,
            };
            let b = GradientStop {
                pos: 1.0,
                color: [0.9, 0.2, 0.6],
                alpha,
                midpoint: 0.5,
            };
            let g = two(a, b, GradientSpace::LinearRgb);
            for i in 1..20 {
                let t = f64::from(i) / 20.0;
                let got = g.eval(t);
                for ch in 0..3 {
                    let want = a.color[ch] + (b.color[ch] - a.color[ch]) * t;
                    assert!(
                        (got[ch] - want).abs() < TOL_ULP,
                        "alpha {alpha} t={t} channel {ch}: {} != plain lerp {want}",
                        got[ch]
                    );
                }
            }
        }
    }

    #[test]
    fn a_fully_transparent_segment_takes_the_guarded_branch() {
        // Both stops at alpha 0 ⇒ the un-premultiply is 0/0. The guard
        // must return the plain lerp (modeling choice 4) and, above all,
        // must never produce NaN — a NaN here would poison every
        // downstream composite even though nothing is visible.
        let a = GradientStop {
            pos: 0.0,
            color: [0.2, 0.4, 0.6],
            alpha: 0.0,
            midpoint: 0.5,
        };
        let b = GradientStop {
            pos: 1.0,
            color: [0.8, 0.6, 0.1],
            alpha: 0.0,
            midpoint: 0.5,
        };
        for space in [GradientSpace::LinearRgb, GradientSpace::Perceptual] {
            let g = two(a, b, space);
            let tol = match space {
                GradientSpace::LinearRgb => TOL_ULP,
                GradientSpace::Perceptual => TOL_OKLAB,
            };
            for i in 1..20 {
                let t = f64::from(i) / 20.0;
                let got = g.eval(t);
                assert!(
                    got.iter().all(|v| v.is_finite()),
                    "{space:?} t={t}: guarded branch produced {got:?}"
                );
                assert_eq!(got[3], 0.0, "{space:?} t={t}: alpha stays 0");
                for ch in 0..3 {
                    // In `perceptual` the plain lerp happens in Oklab, so
                    // compare against that space's own straight lerp.
                    let (ca, cb) = match space {
                        GradientSpace::LinearRgb => (a.color, b.color),
                        GradientSpace::Perceptual => {
                            (linear_rgb_to_oklab(a.color), linear_rgb_to_oklab(b.color))
                        }
                    };
                    let mixed = [
                        ca[0] + (cb[0] - ca[0]) * t,
                        ca[1] + (cb[1] - ca[1]) * t,
                        ca[2] + (cb[2] - ca[2]) * t,
                    ];
                    let want = match space {
                        GradientSpace::LinearRgb => mixed,
                        GradientSpace::Perceptual => oklab_to_linear_rgb(mixed),
                    };
                    assert!(
                        (got[ch] - want[ch]).abs() < tol,
                        "{space:?} t={t} channel {ch}: {} != guarded plain lerp {}",
                        got[ch],
                        want[ch]
                    );
                }
            }
        }
    }

    #[test]
    fn perceptual_differs_from_linear_rgb_visibly() {
        // If `space` were silently ignored this test is the one that
        // notices: the two spaces must disagree by MORE than an 8-bit
        // code somewhere on a saturated ramp.
        let mk = |space| {
            Gradient::new(
                vec![
                    GradientStop {
                        pos: 0.0,
                        color: [1.0, 0.0, 0.0],
                        alpha: 1.0,
                        midpoint: 0.5,
                    },
                    GradientStop {
                        pos: 1.0,
                        color: [0.0, 0.0, 1.0],
                        alpha: 1.0,
                        midpoint: 0.5,
                    },
                ],
                GradientKind::Linear,
                space,
                true,
            )
            .unwrap()
        };
        let lin = mk(GradientSpace::LinearRgb);
        let per = mk(GradientSpace::Perceptual);
        let worst = (1..100)
            .map(|i| {
                let t = f64::from(i) / 100.0;
                let (a, b) = (lin.eval(t), per.eval(t));
                (0..3).map(|c| (a[c] - b[c]).abs()).fold(0.0, f64::max)
            })
            .fold(0.0, f64::max);
        assert!(
            worst > U8_HALF_CODE,
            "perceptual and linearRgb must differ observably; worst = {worst}"
        );
    }

    #[test]
    fn hard_stop_is_right_continuous() {
        let g = Gradient::new(
            vec![
                stop(0.0, 0.0),
                stop(0.5, 0.25),
                stop(0.5, 0.75),
                stop(1.0, 1.0),
            ],
            GradientKind::Linear,
            GradientSpace::LinearRgb,
            true,
        )
        .unwrap();
        assert_eq!(g.eval(0.5)[0], 0.75, "the later stop wins at the seam");
        assert!(
            g.eval(0.499)[0] < 0.26,
            "below the seam ramps to the earlier stop"
        );
        assert!(
            g.eval(0.501)[0] > 0.74,
            "above the seam ramps from the later stop"
        );
    }

    #[test]
    fn stops_are_sorted_and_under_two_is_refused() {
        let g = Gradient::new(
            vec![stop(1.0, 1.0), stop(0.0, 0.0), stop(0.5, 0.5)],
            GradientKind::Linear,
            GradientSpace::LinearRgb,
            true,
        )
        .unwrap();
        let positions: Vec<f64> = g.stops().iter().map(|s| s.pos).collect();
        assert_eq!(positions, vec![0.0, 0.5, 1.0], "stops must sort by pos");

        let e = Gradient::new(
            vec![stop(0.0, 0.0)],
            GradientKind::Linear,
            GradientSpace::LinearRgb,
            true,
        )
        .unwrap_err();
        assert_eq!(e.code.as_str(), "error.schema.out_of_range");
    }

    #[test]
    fn out_of_range_stop_fields_are_refused_typed() {
        let bad = |s: GradientStop| {
            Gradient::new(
                vec![s, stop(1.0, 1.0)],
                GradientKind::Linear,
                GradientSpace::LinearRgb,
                true,
            )
            .unwrap_err()
            .code
            .as_str()
            .to_string()
        };
        let base = stop(0.0, 0.0);
        assert_eq!(
            bad(GradientStop { pos: 1.5, ..base }),
            "error.schema.out_of_range"
        );
        assert_eq!(
            bad(GradientStop {
                midpoint: -0.1,
                ..base
            }),
            "error.schema.out_of_range"
        );
        assert_eq!(
            bad(GradientStop { alpha: 2.0, ..base }),
            "error.schema.out_of_range"
        );
        assert_eq!(
            bad(GradientStop {
                pos: f64::NAN,
                ..base
            }),
            "error.schema.out_of_range"
        );
        assert_eq!(
            bad(GradientStop {
                color: [f64::INFINITY, 0.0, 0.0],
                ..base
            }),
            "error.schema.out_of_range"
        );
    }

    // -- Oklab ------------------------------------------------------------

    #[test]
    fn oklab_round_trips_linear_rgb() {
        let samples = [
            [0.0, 0.0, 0.0],
            [1.0, 1.0, 1.0],
            [0.5, 0.5, 0.5],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.12, 0.34, 0.56],
            [0.9, 0.05, 0.4],
        ];
        for c in samples {
            let back = oklab_to_linear_rgb(linear_rgb_to_oklab(c));
            for i in 0..3 {
                assert!(
                    (back[i] - c[i]).abs() < TOL_OKLAB,
                    "oklab round trip {c:?} -> {back:?} channel {i} (tol {TOL_OKLAB})"
                );
            }
        }
    }

    #[test]
    fn oklab_is_total_on_negative_components() {
        // Out-of-gamut linear values reach this code; cbrt of a negative
        // must take the real root, not produce NaN.
        let out = linear_rgb_to_oklab([-0.2, 0.5, 1.4]);
        assert!(out.iter().all(|v| v.is_finite()), "oklab produced {out:?}");
        let back = oklab_to_linear_rgb(out);
        assert!(
            back.iter().all(|v| v.is_finite()),
            "inverse produced {back:?}"
        );
    }

    // -- geometry ---------------------------------------------------------

    #[test]
    fn linear_parameter_is_zero_at_start_and_one_at_end() {
        let g = GradientGeometry::from_drag([10.0, 20.0], [110.0, 220.0]).unwrap();
        let t0 = g.parameter_at(GradientKind::Linear, [10.0, 20.0]).unwrap();
        let t1 = g
            .parameter_at(GradientKind::Linear, [110.0, 220.0])
            .unwrap();
        assert!(t0.abs() < TOL_ULP, "t(p0) = {t0}, want 0");
        assert!((t1 - 1.0).abs() < TOL_ULP, "t(p1) = {t1}, want 1");
    }

    #[test]
    fn linear_is_constant_along_the_perpendicular() {
        // The defining property of a LINEAR ramp: iso-lines are straight
        // and perpendicular to the axis.
        let g = GradientGeometry::from_drag([0.0, 0.0], [100.0, 0.0]).unwrap();
        let mid = g.parameter_at(GradientKind::Linear, [50.0, 0.0]).unwrap();
        for y in [-500.0, -13.0, 7.5, 999.0] {
            let t = g.parameter_at(GradientKind::Linear, [50.0, y]).unwrap();
            assert!(
                (t - mid).abs() < TOL_ULP,
                "linear must not vary with perpendicular offset (y={y}: {t} vs {mid})"
            );
        }
    }

    #[test]
    fn linear_clamps_outside_the_axis() {
        let g = GradientGeometry::from_drag([0.0, 0.0], [100.0, 0.0]).unwrap();
        assert_eq!(
            g.parameter_at(GradientKind::Linear, [-50.0, 0.0]).unwrap(),
            0.0
        );
        assert_eq!(
            g.parameter_at(GradientKind::Linear, [900.0, 0.0]).unwrap(),
            1.0
        );
    }

    /// §22.4 [NORMATIVE]: `reflected  t = |2·t_lin − 1|`, "mirror about
    /// the gesture midpoint" — NOT about `p₀`.
    ///
    /// The third assertion is the one that matters, and is the reason this
    /// test was rewritten: over the axis span itself (`u ∈ [0,1]` — which
    /// is the WHOLE of a fill layer's box, since `from_layer_box` sizes the
    /// axis to span it) the previous `|u|` reading is identically equal to
    /// `u`, i.e. to LINEAR. A reflected fill layer would have rendered
    /// pixel-for-pixel as a linear one. Any implementation that mirrors
    /// about the start instead of the midpoint fails it.
    #[test]
    fn reflected_is_a_mirror_about_the_axis_midpoint_not_the_start() {
        let g = GradientGeometry::from_drag([0.0, 0.0], [100.0, 0.0]).unwrap();

        // 1. The normative identity, transcribed, at the three fixed points.
        for (x, want) in [(0.0, 1.0), (50.0, 0.0), (100.0, 1.0)] {
            let t = g.parameter_at(GradientKind::Reflected, [x, 0.0]).unwrap();
            assert!(
                (t - want).abs() < TOL_ULP,
                "reflected at u={}: {t} vs {want}",
                x / 100.0
            );
        }

        // 2. Symmetric about the axis MIDPOINT (u = ½), not about p₀.
        for d in [1.0, 17.0, 49.0] {
            let lo = g
                .parameter_at(GradientKind::Reflected, [50.0 - d, 0.0])
                .unwrap();
            let hi = g
                .parameter_at(GradientKind::Reflected, [50.0 + d, 0.0])
                .unwrap();
            assert!(
                (lo - hi).abs() < TOL_ULP,
                "reflected must mirror about the midpoint at 50±{d}: {lo} vs {hi}"
            );
            assert!(
                (lo - d / 50.0).abs() < TOL_ULP,
                "reflected magnitude at 50±{d}: {lo} vs {}",
                d / 50.0
            );
        }

        // 3. THE DISCRIMINATING CHECK. Inside the axis span, reflected must
        // NOT track linear — the failure mode the `|u|` reading had, which
        // is invisible everywhere else because both readings agree on
        // sign, on the endpoints' VALUES elsewhere, and behind p₀.
        let mut saw_a_difference = false;
        for k in 1..10 {
            let x = f64::from(k) * 10.0;
            let lin = g.parameter_at(GradientKind::Linear, [x, 0.0]).unwrap();
            let refl = g.parameter_at(GradientKind::Reflected, [x, 0.0]).unwrap();
            if (lin - refl).abs() > 0.05 {
                saw_a_difference = true;
            }
        }
        assert!(
            saw_a_difference,
            "reflected must differ from linear INSIDE the axis span — `|u|` \
             equals `u` there and would render a dead enum arm"
        );
    }

    #[test]
    fn radial_iso_lines_are_circles_about_the_start() {
        let g = GradientGeometry::from_drag([0.0, 0.0], [100.0, 0.0]).unwrap();
        let r = 60.0;
        let want = r / 100.0;
        for k in 0..16 {
            let a = f64::from(k) * std::f64::consts::TAU / 16.0;
            let t = g
                .parameter_at(GradientKind::Radial, [r * a.cos(), r * a.sin()])
                .unwrap();
            assert!(
                (t - want).abs() < TOL_ULP,
                "radial must be constant on a circle (angle {a}: {t} vs {want})"
            );
        }
        assert_eq!(
            g.parameter_at(GradientKind::Radial, [0.0, 0.0]).unwrap(),
            0.0,
            "radial is 0 at the centre"
        );
    }

    /// §22.4 [NORMATIVE]: `diamond  t = max(|u|, |v|)`, glossed
    /// "(Chebyshev distance in the frame)" — the L^∞ ball, not the L¹ one.
    /// See [`GradientGeometry::parameter_at`]'s note: the letter states
    /// this twice, and the "is the letter the intent?" question is
    /// O-INT's, not this module's.
    #[test]
    fn diamond_iso_lines_are_chebyshev_balls() {
        let g = GradientGeometry::from_drag([0.0, 0.0], [100.0, 0.0]).unwrap();
        // max(|a|, |b|) = 60 traces an AXIS-ALIGNED square in the frame:
        // its four edge midpoints and its four CORNERS all share one t.
        let want = 0.6;
        for (x, y) in [
            (60.0, 0.0),
            (0.0, 60.0),
            (-60.0, 0.0),
            (0.0, -60.0),
            (60.0, 60.0),   // corner
            (-60.0, 60.0),  // corner
            (60.0, -30.0),  // edge, off-centre
            (-15.0, -60.0), // edge, off-centre
        ] {
            let t = g.parameter_at(GradientKind::Diamond, [x, y]).unwrap();
            assert!(
                (t - want).abs() < TOL_ULP,
                "diamond must be constant on the Chebyshev ball at ({x},{y}): {t} vs {want}"
            );
        }
        // DISCRIMINATING: the L¹ reading is NOT constant on that square —
        // it gives 0.6 at the edge midpoints but 1.2 at the corners. This
        // point alone separates the two readings.
        let corner = g.parameter_at(GradientKind::Diamond, [60.0, 60.0]).unwrap();
        assert!(
            (corner - 0.6).abs() < TOL_ULP,
            "the corner (60,60) is Chebyshev-distance 60 (t=0.6); the L1 reading \
             would give 1.2 and clamp to 1.0 — got {corner}"
        );
        // And it is not radial: same Euclidean radius, different t.
        let axis = g.parameter_at(GradientKind::Diamond, [60.0, 0.0]).unwrap();
        let diag = g
            .parameter_at(
                GradientKind::Diamond,
                [60.0 * 0.5_f64.sqrt(), 60.0 * 0.5_f64.sqrt()],
            )
            .unwrap();
        assert!((axis - diag).abs() > 0.15, "diamond must not be radial");
    }

    #[test]
    fn angular_sweeps_one_full_turn_and_wraps() {
        let g = GradientGeometry::from_drag([0.0, 0.0], [100.0, 0.0]).unwrap();
        // On the axis itself the sweep starts at 0.
        let t0 = g.parameter_at(GradientKind::Angular, [50.0, 0.0]).unwrap();
        assert!(
            t0.abs() < TOL_ULP,
            "angular starts at 0 on the axis, got {t0}"
        );
        // A quarter turn in the +b direction is t = 0.25.
        let q = g.parameter_at(GradientKind::Angular, [0.0, 50.0]).unwrap();
        assert!((q - 0.25).abs() < TOL_ULP, "quarter turn: {q}");
        let h = g.parameter_at(GradientKind::Angular, [-50.0, 0.0]).unwrap();
        assert!(
            h.abs() < TOL_ULP || (h - 0.5).abs() < TOL_ULP,
            "half turn: {h}"
        );
        let three = g.parameter_at(GradientKind::Angular, [0.0, -50.0]).unwrap();
        assert!(
            (three - 0.75).abs() < TOL_ULP,
            "three-quarter turn: {three}"
        );
        // Range is half-open [0,1): every sample stays inside.
        for k in 0..360 {
            let a = f64::from(k) * std::f64::consts::TAU / 360.0;
            let t = g
                .parameter_at(GradientKind::Angular, [10.0 * a.cos(), 10.0 * a.sin()])
                .unwrap();
            assert!((0.0..1.0).contains(&t), "angular t = {t} left [0,1) at {a}");
        }
    }

    #[test]
    fn angular_is_scale_invariant() {
        // Angle depends on direction, never on distance.
        let g = GradientGeometry::from_drag([0.0, 0.0], [100.0, 0.0]).unwrap();
        let near = g.parameter_at(GradientKind::Angular, [1.0, 1.0]).unwrap();
        let far = g.parameter_at(GradientKind::Angular, [1e6, 1e6]).unwrap();
        assert!((near - far).abs() < TOL_ULP, "angular must be scale-free");
    }

    #[test]
    fn noise_is_refused_typed_not_aliased() {
        let g = GradientGeometry::from_drag([0.0, 0.0], [10.0, 0.0]).unwrap();
        let e = g.parameter_at(GradientKind::Noise, [1.0, 1.0]).unwrap_err();
        assert_eq!(e.code.as_str(), "error.command.not_implemented");
        assert!(
            e.to_string().contains("noise"),
            "the refusal must name the kind: {e}"
        );
    }

    #[test]
    fn zero_length_drag_is_refused() {
        let e = GradientGeometry::from_drag([5.0, 5.0], [5.0, 5.0]).unwrap_err();
        assert_eq!(e.code.as_str(), "error.schema.out_of_range");
        assert!(GradientGeometry::from_drag([5.0, 5.0], [5.0, 5.000_001]).is_ok());
    }

    #[test]
    fn non_finite_geometry_is_refused() {
        assert!(GradientGeometry::from_drag([f64::NAN, 0.0], [1.0, 1.0]).is_err());
        assert!(GradientGeometry::from_drag([0.0, 0.0], [f64::INFINITY, 1.0]).is_err());
        let g = GradientGeometry::from_drag([0.0, 0.0], [1.0, 0.0]).unwrap();
        assert!(g
            .parameter_at(GradientKind::Linear, [f64::NAN, 0.0])
            .is_err());
    }

    // -- the fill-layer axis ---------------------------------------------

    #[test]
    fn layer_box_axis_spans_the_box_at_scale_one() {
        // Default angle 90 with the y-UP convention runs BOTTOM to TOP in
        // raster space, and at scale 1 the axis exactly spans the height.
        let g = GradientGeometry::from_layer_box([0.0, 0.0], [200.0, 100.0], 90.0, 1.0).unwrap();
        assert!(
            (g.length() - 100.0).abs() < 1e-9,
            "90° axis must span the height, got {}",
            g.length()
        );
        assert!(
            g.start()[1] > g.end()[1],
            "90° must run upward in raster space (start y {} > end y {})",
            g.start()[1],
            g.end()[1]
        );
        // 0° spans the width.
        let h = GradientGeometry::from_layer_box([0.0, 0.0], [200.0, 100.0], 0.0, 1.0).unwrap();
        assert!(
            (h.length() - 200.0).abs() < 1e-9,
            "0° spans width, got {}",
            h.length()
        );
        // 45° spans the box's support width in that direction:
        // (w + h)/√2 = 300/√2 ≈ 212.132.
        let d = GradientGeometry::from_layer_box([0.0, 0.0], [200.0, 100.0], 45.0, 1.0).unwrap();
        let want = 300.0 / 2.0_f64.sqrt();
        assert!(
            (d.length() - want).abs() < 1e-9,
            "45° support width: got {}, want {want}",
            d.length()
        );
    }

    #[test]
    fn layer_box_axis_is_centred_and_scales() {
        let one = GradientGeometry::from_layer_box([0.0, 0.0], [200.0, 100.0], 0.0, 1.0).unwrap();
        let two = GradientGeometry::from_layer_box([0.0, 0.0], [200.0, 100.0], 0.0, 2.0).unwrap();
        assert!(
            (two.length() - 2.0 * one.length()).abs() < 1e-9,
            "scale multiplies length"
        );
        // Both stay centred on the box centre (100, 50).
        for g in [one, two] {
            let cx = (g.start()[0] + g.end()[0]) / 2.0;
            let cy = (g.start()[1] + g.end()[1]) / 2.0;
            assert!(
                (cx - 100.0).abs() < 1e-9 && (cy - 50.0).abs() < 1e-9,
                "axis must stay centred"
            );
        }
    }

    #[test]
    fn layer_box_refuses_degenerate_inputs() {
        let e = GradientGeometry::from_layer_box([0.0, 0.0], [10.0, 10.0], 0.0, 0.0).unwrap_err();
        assert_eq!(e.code.as_str(), "error.schema.out_of_range");
        assert!(GradientGeometry::from_layer_box([0.0, 0.0], [10.0, 10.0], 0.0, -1.0).is_err());
        assert!(
            GradientGeometry::from_layer_box([0.0, 0.0], [0.0, 10.0], 0.0, 1.0).is_err(),
            "an empty box has no axis"
        );
    }

    // -- sampling / reverse ----------------------------------------------

    #[test]
    fn reverse_flips_the_ramp() {
        let g = ramp(GradientKind::Linear, GradientSpace::LinearRgb);
        let geo = GradientGeometry::from_drag([0.0, 0.0], [100.0, 0.0]).unwrap();
        for x in [0.0, 25.0, 50.0, 75.0, 100.0] {
            let fwd = geo.sample(&g, [x, 0.0], false).unwrap();
            let rev = geo.sample(&g, [x, 0.0], true).unwrap();
            assert!(
                (fwd[0] - (1.0 - rev[0])).abs() < TOL_ULP,
                "reverse must mirror the ramp at x={x}: {} vs {}",
                fwd[0],
                rev[0]
            );
        }
        // And reverse must actually CHANGE something.
        let a = geo.sample(&g, [25.0, 0.0], false).unwrap();
        let b = geo.sample(&g, [25.0, 0.0], true).unwrap();
        assert!((a[0] - b[0]).abs() > 0.4, "reverse must not be a no-op");
    }

    #[test]
    fn every_geometric_kind_covers_the_full_ramp_over_its_domain() {
        // A blunt smoke net: each served kind must actually reach both
        // ends of the ramp somewhere in a box around the axis, so a kind
        // wired to a constant would be caught.
        let g = ramp(GradientKind::Linear, GradientSpace::LinearRgb);
        let geo = GradientGeometry::from_drag([0.0, 0.0], [50.0, 0.0]).unwrap();
        for kind in [
            GradientKind::Linear,
            GradientKind::Radial,
            GradientKind::Angular,
            GradientKind::Reflected,
            GradientKind::Diamond,
        ] {
            let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
            for y in -100..=100 {
                for x in -100..=100 {
                    let t = geo
                        .parameter_at(kind, [f64::from(x), f64::from(y)])
                        .unwrap();
                    lo = lo.min(t);
                    hi = hi.max(t);
                }
            }
            assert!(
                lo < 0.05,
                "{:?} never reaches the ramp start (min {lo})",
                kind
            );
            assert!(hi > 0.9, "{:?} never reaches the ramp end (max {hi})", kind);
            let _ = g.eval(hi);
        }
    }

    #[test]
    fn the_five_geometric_kinds_are_pairwise_distinct() {
        // The check that catches a copy-paste in the match: no two kinds
        // may agree everywhere on a probe set.
        let geo = GradientGeometry::from_drag([0.0, 0.0], [50.0, 0.0]).unwrap();
        let kinds = [
            GradientKind::Linear,
            GradientKind::Radial,
            GradientKind::Angular,
            GradientKind::Reflected,
            GradientKind::Diamond,
        ];
        let probe: Vec<[f64; 2]> = (-3..=3)
            .flat_map(|y| (-3..=3).map(move |x| [f64::from(x) * 17.0, f64::from(y) * 13.0]))
            .collect();
        for (i, a) in kinds.iter().enumerate() {
            for b in &kinds[i + 1..] {
                let differs = probe.iter().any(|p| {
                    let ta = geo.parameter_at(*a, *p).unwrap();
                    let tb = geo.parameter_at(*b, *p).unwrap();
                    (ta - tb).abs() > 1e-9
                });
                assert!(differs, "{a:?} and {b:?} are indistinguishable");
            }
        }
    }
}
