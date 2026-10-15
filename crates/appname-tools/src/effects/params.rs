// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! §11.3 layer-effect parameter structs — the MVP five.
//!
//! Field names, types, ranges and defaults are transcribed from the
//! **§11.3 "Parameters — types, ranges, defaults" [NORMATIVE]** table.
//! Only the MVP five of the ten effects are modelled here (drop shadow,
//! stroke, outer glow, inner glow, colour overlay); `InnerShadow`,
//! `GradientOverlay`, `PatternOverlay`, `Bevel` and `Satin` are
//! deliberately absent rather than stubbed — an empty seat is honest,
//! a stub that silently renders nothing is not.
//!
//! ## Wire status — READ THIS BEFORE ADDING A `serde` DERIVE
//!
//! These are **engine-internal** structs. They are NOT a params schema and
//! MUST NOT gain `Serialize`/`Deserialize`, a JSON shape, a contract mirror
//! or a registry kind until the v5.6 fold-in lands the layer-style
//! capability rows. §11.3 specifies the *math* and these Rust struct
//! shapes; what it does not specify anywhere is a **command** to edit an
//! effect — the §10.3 catalog's `layer.*` family has no effect row, and
//! §11.1's `effects` property forward-references "effect edit commands
//! (§11.3)" that do not exist. Naming those params on the wire is exactly
//! the decision reserved to the fold-in.
//!
//! ## ★ CORRECTION: the DOCUMENT schema already froze these shapes ★
//!
//! An earlier version of this note, and the wall proof that went with it,
//! claimed "no schema". That is **too broad, and it was wrong.** What does
//! not exist is an effect-edit **command** and its params schema. The
//! *persisted document* schema, `contracts/schemas/document.v1.json`, has
//! carried the whole thing all along: a `layer.effects` array, an `effect`
//! `oneOf` over all **ten** `fx*` definitions, and full property sets with
//! wire names and defaults (`fxDropShadow`, `fxOuterGlow`, `fxInnerGlow`,
//! `fxStroke`, `fxColorOverlay`, plus the five not modelled here).
//!
//! That is load-bearing for the fold-in, in both directions. It means the
//! rows do **not** get a free hand on parameter names — `distancePx`,
//! `sizePx`, `spread`, `range`, `technique`, `source`, `position`,
//! `useGlobalLight`, `angleDeg`, `enabled` are already frozen in a
//! persisted format. And it means modelling one of those names here coins
//! nothing: `enabled` was added for exactly that reason.
//!
//! ## Where the schema and §11.3 DISAGREE — fold-in inputs, not local fixes
//!
//! Found by reading the artifact rather than the section. The engine
//! follows §11.3 in every case below; the disagreement is recorded, not
//! resolved, and `effects::tests::defaults_match_the_frozen_document_schema_where_they_agree`
//! pins both sides so neither can drift unnoticed.
//!
//! 1. **Glow `opacity` default.** Schema: `0.5`. §11.3 table: `0.35`. A
//!    document written by one and read by the other renders differently.
//! 2. **No upper bounds at all.** Schema types `distancePx` and `sizePx`
//!    as `{"type": "number", "minimum": 0}` — **no maximum**. §11.3 types
//!    them `0..=30_000` and `0..=250`. So the allocation hazard recorded
//!    on [`super::EffectStack::reach_px`] is *worse* than the §11.3 bound
//!    suggests: the ~14.6 GB figure assumes a limit the persisted schema
//!    does not impose. A schema-valid `distancePx: 1e30` is admissible.
//!    Measured behaviour: `reach_px` saturates to `i32::MAX` and
//!    `IRect::inflate` then **panics on i32 overflow** — an abort, in the
//!    ROI arithmetic, *before* any allocation is attempted. This is a
//!    third sibling of the FI-16/FI-18 class (contract-valid numbers
//!    scaling allocations without limit, failing by abort rather than
//!    typed refusal), and it says a ceiling must be enforced before the
//!    reach arithmetic, not merely before the allocation.
//! 3. **`range` has no minimum.** Schema: `{"maximum": 1, "default": 0.5}`.
//!    §11.3: `0.01..=1`. A schema-valid `range: 0` is a division by zero;
//!    [`super::apply_range`]'s clamp contains it, but the engine is then
//!    silently disagreeing with the document rather than refusing it.
//! 4. **Stroke `sizePx` minimum.** Schema: `0`. §11.3: `1..=250`.
//! 5. **A glow may have neither `color` nor `gradient`.** Neither is
//!    `required` and `color` carries no default, so the fill is undefined.
//! 6. **`layer.effects` is an unbounded array with no per-`type`
//!    uniqueness.** Two drop shadows on one layer are schema-valid, while
//!    §11.3's order table has exactly one seat each — which is what
//!    [`super::EffectStack`] implements. The predecessor recorded "several
//!    of one kind" as an open question; the persisted schema has already
//!    answered it the other way, and this engine cannot represent a
//!    document the schema admits.
//! 7. **`fxStroke.fill` is the full `fill` `$ref`** (solid/gradient/
//!    pattern) while [`StrokeFill`] models solid only, so a schema-valid
//!    gradient stroke cannot round-trip. Deliberate MVP scope, recorded
//!    here so it is a known gap rather than a surprise.

use appname_core::BlendMode;

/// Straight (un-premultiplied) linear RGB. Effects are colour-managed and
/// render at working precision (§11.3); the working-space conversion is the
/// caller's, so this is a plain triple with no colour-space tag.
pub(crate) type Rgb = [f32; 3];

/// The document's one global light pair (§11.3 "Global light" [NORMATIVE]),
/// default `(120, 30)`. Stored on the document and mutated by
/// `meta.set{key:"globalLight"}` — a command row that **already exists** in
/// §10.3 today, with nothing in the engine consuming it until this stack
/// does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GlobalLight {
    pub angle_deg: f32,
    pub altitude_deg: f32,
}

impl Default for GlobalLight {
    fn default() -> Self {
        GlobalLight {
            angle_deg: 120.0,
            altitude_deg: 30.0,
        }
    }
}

/// §11.3 `ShadowParams`. Drop shadow only in the MVP; the identical struct
/// serves `InnerShadow` when that effect lands (`spread` reads as "choke").
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ShadowParams {
    /// `document.v1.json` `fx*.enabled`, default `true` — see
    /// [`LayerEffect::enabled`]. The wire name is already frozen.
    pub enabled: bool,
    pub color: Rgb,
    pub blend: BlendMode,
    /// `0..=1`
    pub opacity: f32,
    /// `−180..=180` deg. The UI angle `α`: CCW, 0 = light from the right.
    pub angle_deg: f32,
    /// `0..=30_000` px
    pub distance_px: f32,
    /// `0..=1` — a **fraction of `size_px`**, not a pixel count.
    pub spread: f32,
    /// `0..=250` px
    pub size_px: f32,
    pub use_global_light: bool,
}

impl Default for ShadowParams {
    fn default() -> Self {
        ShadowParams {
            enabled: true,
            color: [0.0, 0.0, 0.0],
            blend: BlendMode::Multiply,
            opacity: 0.35,
            angle_deg: 120.0,
            distance_px: 5.0,
            spread: 0.0,
            size_px: 5.0,
            use_global_light: true,
        }
    }
}

/// §11.3 `GlowTechnique`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GlowTechnique {
    Softer,
    Precise,
}

/// §11.3 `GlowSource` — inner glow only; outer glow ignores it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GlowSource {
    Edge,
    Center,
}

/// §11.3 `GlowParams`, shared by outer and inner glow.
///
/// Three spec'd fields are **deliberately absent** from this MVP struct
/// rather than present-and-ignored. A field that exists but does nothing is
/// the worse failure mode: it reads as supported to every later caller, and
/// nothing reds when it silently has no effect. An absent field cannot lie.
/// * `gradient: Option<Gradient>` — glow filled from a §22.4 gradient
///   instead of `color`. Deferred with the other gradient/pattern fills
///   (see [`StrokeFill`]).
/// * `jitter`, `noise` — §11.3 step 5 dithers alpha and gradient position
///   with **blue noise**. The committed blue-noise table lives in
///   `appname-render` (`bn64`), which this crate cannot reach and must not
///   invert the crate layering to reach. Placement is an open architecture
///   question, recorded for the fold-in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GlowParams {
    /// `document.v1.json` `fx*.enabled`, default `true`.
    pub enabled: bool,
    pub color: Rgb,
    pub blend: BlendMode,
    /// `0..=1`
    pub opacity: f32,
    pub technique: GlowTechnique,
    /// `0..=1` — fraction of `size_px`.
    pub spread: f32,
    /// `0..=250` px
    pub size_px: f32,
    /// Inner glow only.
    pub source: GlowSource,
    /// `0.01..=1` — falloff-domain scale.
    pub range: f32,
}

impl Default for GlowParams {
    fn default() -> Self {
        GlowParams {
            enabled: true,
            // §11.3 default "pale yellow".
            color: [1.0, 1.0, 0.75],
            blend: BlendMode::Screen,
            opacity: 0.35,
            technique: GlowTechnique::Softer,
            spread: 0.0,
            size_px: 5.0,
            source: GlowSource::Edge,
            range: 0.5,
        }
    }
}

/// §20.2 stroke alignment, as §11.3 `StrokeFxParams::position` uses it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StrokeAlign {
    Inside,
    Center,
    Outside,
}

/// The stroke's fill. §11.3 types this as the full `FillContent` (§20.2),
/// which admits solid / gradient / pattern. The MVP models **solid only**:
/// gradient and pattern fills pull in the §22.4 gradient sampler and the
/// §22.5 pattern asset store, and — more to the point — an `AssetId`
/// reference is a document-graph edge whose lifetime and serialization are
/// precisely what the missing spec rows have to settle. A one-variant enum
/// keeps the shape honest and makes the widening additive.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum StrokeFill {
    Solid(Rgb),
}

/// §11.3 `StrokeFxParams`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct StrokeParams {
    /// `document.v1.json` `fx*.enabled`, default `true`.
    pub enabled: bool,
    /// `1..=250` px
    pub size_px: f32,
    pub position: StrokeAlign,
    pub blend: BlendMode,
    /// `0..=1`
    pub opacity: f32,
    pub fill: StrokeFill,
}

impl Default for StrokeParams {
    fn default() -> Self {
        StrokeParams {
            enabled: true,
            size_px: 3.0,
            position: StrokeAlign::Outside,
            blend: BlendMode::Normal,
            opacity: 1.0,
            fill: StrokeFill::Solid([0.0, 0.0, 0.0]),
        }
    }
}

/// §11.3 `OverlayParams` — colour overlay.
///
/// The spec's default colour is "foreground-at-creation", i.e. resolved
/// from tool state by whatever creates the effect. There is no tool state
/// at this layer, so the struct default is black and the real default is
/// the creating command's job — one more thing the missing rows must say.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct OverlayParams {
    /// `document.v1.json` `fx*.enabled`, default `true`.
    pub enabled: bool,
    pub color: Rgb,
    pub blend: BlendMode,
    /// `0..=1`
    pub opacity: f32,
}

impl Default for OverlayParams {
    fn default() -> Self {
        OverlayParams {
            enabled: true,
            color: [0.0, 0.0, 0.0],
            blend: BlendMode::Normal,
            opacity: 1.0,
        }
    }
}

/// One effect. The MVP five of the ten §11.3 variants.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum LayerEffect {
    DropShadow(ShadowParams),
    OuterGlow(GlowParams),
    ColorOverlay(OverlayParams),
    InnerGlow(GlowParams),
    Stroke(StrokeParams),
}

impl LayerEffect {
    /// The effect's slot in the §11.3 **inter-effect render order**
    /// [NORMATIVE], bottom → top. The full ten-effect numbering is kept
    /// (not compacted to 1..=5) so the absent effects' seats stay reserved
    /// and adding one later cannot silently renumber a shipped order:
    ///
    /// | # | effect |
    /// |---|---|
    /// | 1 | `DropShadow` |
    /// | 2 | `OuterGlow` |
    /// | 3 | *the layer's own fill pixels × `fill_opacity`* |
    /// | 4 | `PatternOverlay` *(absent)* |
    /// | 5 | `GradientOverlay` *(absent)* |
    /// | 6 | `ColorOverlay` |
    /// | 7 | `Satin` *(absent)* |
    /// | 8 | `InnerGlow` |
    /// | 9 | `InnerShadow` *(absent)* |
    /// | 10 | `Bevel` *(absent)* |
    /// | 11 | `Stroke` |
    pub(crate) fn order(&self) -> u8 {
        match self {
            LayerEffect::DropShadow(_) => 1,
            LayerEffect::OuterGlow(_) => 2,
            LayerEffect::ColorOverlay(_) => 6,
            LayerEffect::InnerGlow(_) => 8,
            LayerEffect::Stroke(_) => 11,
        }
    }

    /// A stable discriminant used for the "at most one of each kind" rule
    /// (see [`super::EffectStack`]).
    pub(crate) fn kind_index(&self) -> u8 {
        self.order()
    }

    /// Whether this effect participates in evaluation at all.
    ///
    /// **This field is not invented here, and that is the point.** The
    /// module's rule is that no wire name may be coined ahead of the v5.6
    /// rows — but `enabled` is already **frozen** in
    /// `contracts/schemas/document.v1.json`: every one of the ten `fx*`
    /// definitions carries `"enabled": {"type": "boolean", "default":
    /// true}`. Modelling it names nothing new; *omitting* it was the
    /// defect, because a persisted `enabled: false` had nowhere to land and
    /// the effect would have rendered anyway.
    ///
    /// A disabled effect contributes no pass **and no apron** — see
    /// [`super::EffectStack::reach_px`]. Skipping the pass while still
    /// inflating the ROI by a disabled 30 000 px shadow would be the
    /// expensive half of the bug without the visible half.
    pub(crate) fn enabled(&self) -> bool {
        match self {
            LayerEffect::DropShadow(p) => p.enabled,
            LayerEffect::OuterGlow(p) | LayerEffect::InnerGlow(p) => p.enabled,
            LayerEffect::Stroke(p) => p.enabled,
            LayerEffect::ColorOverlay(p) => p.enabled,
        }
    }

    /// How far beyond `A` **this one effect** can write, in px.
    ///
    /// [`super::EffectStack::reach_px`] is the max of this over the stack,
    /// and [`super::evaluate_in`] crops each pass to its own reach — so
    /// this table has exactly one home and the two uses cannot drift.
    ///
    /// The exact reach falls out of the dilate/blur **split**: an effect
    /// dilates by `spread·size_px` and then blurs with `σ =
    /// (1−spread)·size_px/3`, whose §16.3 kernel truncates at `3σ =
    /// (1−spread)·size_px`. The two terms sum to `size_px` for *every*
    /// value of `spread`, so the reach is `|offset| + size_px` (plus 1 px
    /// for the §13.2 re-AA band) and does not depend on `spread` at all.
    ///
    /// Inner glow and colour overlay are clipped to `A`, so they reach 0
    /// past it; an inside stroke likewise. The `+1` is still added to all
    /// of them, uniformly — it is the AA band's width, not an effect
    /// parameter, and a zero-reach effect whose plane stopped exactly at
    /// `A`'s rect would be the one shape whose edge pixels a downstream
    /// resample could not read a neighbour for.
    ///
    /// `max(0.0)` before the cast is not decoration: this side of the wall
    /// has **no validation door**, so a negative `distance_px` is
    /// reachable, and an unguarded negative would *deflate* the working
    /// rect rather than grow it — turning an apron into a crop and
    /// reintroducing exactly the value-changing narrow-rect defect
    /// [`super::evaluate_in`] documents. It is NaN-safe for free:
    /// `f32::max` returns the non-NaN operand.
    pub(crate) fn reach_px(&self) -> i32 {
        let r: f32 = match self {
            LayerEffect::DropShadow(p) => p.distance_px + p.size_px,
            LayerEffect::OuterGlow(p) => p.size_px,
            // Both are clipped to A, so they never write outside it.
            LayerEffect::InnerGlow(_) | LayerEffect::ColorOverlay(_) => 0.0,
            LayerEffect::Stroke(p) => match p.position {
                StrokeAlign::Outside => p.size_px,
                StrokeAlign::Center => p.size_px / 2.0,
                StrokeAlign::Inside => 0.0,
            },
        };
        // +1 px for the §13.2 re-AA band.
        (r.max(0.0).ceil() as i32).saturating_add(1)
    }

    /// §11.3 "Effect scale on transform" [NORMATIVE]: pixel-valued effect
    /// parameters multiply by `s = √|det M₂ₓ₂|` when a transform command
    /// asks for it (`scaleEffects`). Unitless parameters (`opacity`,
    /// `spread`, `range`, angles) do **not** scale — `spread` in particular
    /// is a *fraction of size*, so scaling `size_px` already scales the
    /// dilation it implies, and scaling `spread` too would square the
    /// effect.
    pub(crate) fn scale_effects(&mut self, s: f32) {
        match self {
            LayerEffect::DropShadow(p) => {
                p.distance_px *= s;
                p.size_px *= s;
            }
            LayerEffect::OuterGlow(p) | LayerEffect::InnerGlow(p) => {
                p.size_px *= s;
            }
            LayerEffect::Stroke(p) => {
                p.size_px *= s;
            }
            // Colour overlay has no pixel-valued parameter.
            LayerEffect::ColorOverlay(_) => {}
        }
    }
}
