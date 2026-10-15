// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! §11.3 layer effects/styles — the effects-stack internals.
//!
//! ## ★ THIS MODULE IS BEHIND AN HONEST WALL — NO WIRE SURFACE ★
//!
//! Layer styles have **no spec rows yet**. §11.3 specifies the *math* and
//! the effect struct shapes, but the §10.3 command catalog has **no**
//! effect-edit command in its `layer.*` family — §11.1's `effects`
//! property forward-references "effect edit commands (§11.3)" that the
//! catalog never defines. The rows land at the v5.6 fold-in; wire and serve
//! happen after, spec-first.
//!
//! So this module is deliberately **unreachable**, and the compiler is what
//! enforces it: `effects` is a **private** module of `appname-tools`
//! (`mod effects;`, not `pub mod`), exported by nothing. There is no
//! command kind, no registry entry, no **command params** schema, no ABI
//! symbol and no shell surface, and no crate outside this one can name a
//! single item in here. Its only callers are its own unit tests. That is
//! the intended state until the fold-in lands, and the `dead_code` allow
//! below is the honest record of it rather than a wart to be silenced by
//! premature wiring.
//!
//! **The REGISTRY count and the SERVED count are different numbers, and a
//! wall claim has to say which.** This module moves neither: the REGISTRY
//! is terminal at **102** kinds, and the SERVED count is **98** — the four
//! between them are the three † out-of-undo-stack kinds plus `doc.new`.
//! Adding a kind would move the registry count and is exactly what is
//! reserved to the fold-in.
//!
//! ★ **One earlier wall claim was too broad and is corrected here.** "No
//! schema" was wrong as stated: `contracts/schemas/document.v1.json`
//! already carries `layer.effects` and all ten `fx*` shapes, frozen, with
//! wire names and defaults. What does not exist is a *command* to edit an
//! effect and the params schema that would come with it. See [`params`]
//! for the correction in full, and for the seven places the frozen schema
//! and §11.3 disagree.
//!
//! ## What the stack is
//!
//! Effect evaluation is a pure function of the layer's coverage:
//!
//! ```text
//!     evaluate(A, stack, global_light)  ->  Vec<EffectPass>
//! ```
//!
//! `A` is the layer's composited coverage **at full fill opacity** — which
//! is the whole reason effects survive `fill_opacity = 0` (the hollow
//! shape with a stroke and a shadow, §11.1). `fill_opacity` is not a
//! parameter of this function and cannot become one: the stack never sees
//! it, so it cannot accidentally attenuate an effect by it.
//!
//! ## What the stack is not
//!
//! It does not blend. Each [`EffectPass`] carries coverage, a colour, a
//! [`BlendMode`] and an opacity, plus the slot and order index that say
//! where in §11.3's fixed stack it belongs — and the Porter-Duff/blend
//! arithmetic of §4.4/§4.5 stays where it already lives, in
//! `appname-render`. "One compositing truth" is a standing invariant, and
//! an effects stack that grew its own blender would quietly become a
//! second one.

// The honest wall (see the module docs): every item here is reachable only
// from this module's own tests until the v5.6 rows land and the wire
// surface is built on top. Removing this allow is part of *wiring*, not
// part of cleanup.
#![allow(dead_code)]

use appname_core::{BlendMode, IRect};

use crate::select::MaskPlane;

pub(crate) mod field;
pub(crate) mod params;

#[cfg(test)]
mod tests;

use field::Field;
pub(crate) use params::{
    GlobalLight, GlowParams, GlowSource, GlowTechnique, LayerEffect, OverlayParams, Rgb,
    ShadowParams, StrokeAlign, StrokeFill, StrokeParams,
};

/// Where a pass sits relative to the layer's own fill pixels, per the
/// §11.3 inter-effect render order [NORMATIVE].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Slot {
    /// Below the layer's fill pixels (drop shadow, outer glow).
    Under,
    /// Above the fill pixels and **clipped to `A`** — the "interior stack"
    /// of §11.3 steps 4–10.
    Interior,
    /// Above everything (stroke, step 11).
    Above,
}

/// One renderable effect pass: coverage plus how it composites.
///
/// This is the hand-off shape to the compositor. It carries no pixels of
/// the layer and performs no blending — `coverage` is the effect's own
/// alpha before `opacity` is applied.
#[derive(Clone, Debug)]
pub(crate) struct EffectPass {
    /// The §11.3 order index, 1..=11. Passes are returned already sorted by
    /// it, and the compositor composites in this order.
    pub order: u8,
    pub slot: Slot,
    /// The effect's alpha over [`EffectPass::coverage`]`.rect`, in `[0,1]`,
    /// **before** `opacity`.
    pub coverage: MaskPlane,
    /// Straight linear RGB fill for the pass.
    pub color: Rgb,
    pub blend: BlendMode,
    /// `0..=1`, applied by the compositor together with `blend`.
    pub opacity: f32,
}

/// A layer's effect stack.
///
/// §11.3's inter-effect order is a fixed table with one seat per effect and
/// the rule that "effects never reorder", so this holds **at most one of
/// each kind** — [`EffectStack::set`] replaces rather than appends.
///
/// ★ **That was recorded as an open question; the frozen artifact has
/// already answered it the other way.** `document.v1.json` types
/// `layer.effects` as a plain array of `effect` with no `maxItems` and no
/// per-`type` uniqueness, so **two drop shadows on one layer are
/// schema-valid** — and this engine cannot represent such a document. The
/// §11.3 order table (one seat each) and the persisted schema (a bag)
/// disagree, and the fold-in has to pick one. `set` implements the §11.3
/// reading deliberately; it is the narrower of the two, so widening later
/// is additive while the reverse would not be.
#[derive(Clone, Debug, Default)]
pub(crate) struct EffectStack {
    effects: Vec<LayerEffect>,
}

impl EffectStack {
    pub(crate) fn new() -> EffectStack {
        EffectStack {
            effects: Vec::new(),
        }
    }

    /// Insert `e`, replacing any existing effect of the same kind.
    ///
    /// New effects are inserted at their §11.3 **seat**, not appended, so
    /// the stack is stored in render order and [`EffectStack::iter`] and
    /// [`evaluate`] agree.
    ///
    /// This is **load-bearing, not tidiness**: [`render_all`] does not
    /// sort. It asserts. Appending here would produce passes in insertion
    /// order and trip that `debug_assert` from every multi-effect path.
    /// (It used to be tidiness — an evaluator sort covered for it — and
    /// that sort is gone precisely because it made a broken `set`
    /// unfalsifiable. See [`render_all`].)
    pub(crate) fn set(&mut self, e: LayerEffect) {
        match self
            .effects
            .iter_mut()
            .find(|x| x.kind_index() == e.kind_index())
        {
            Some(slot) => *slot = e,
            None => {
                let at = self.effects.partition_point(|x| x.order() < e.order());
                self.effects.insert(at, e);
            }
        }
    }

    pub(crate) fn remove_kind(&mut self, kind_index: u8) {
        self.effects.retain(|e| e.kind_index() != kind_index);
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.effects.is_empty()
    }

    pub(crate) fn len(&self) -> usize {
        self.effects.len()
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &LayerEffect> {
        self.effects.iter()
    }

    /// §11.3 "Effect scale on transform" [NORMATIVE] — the `scaleEffects`
    /// arm, `s = √|det M₂ₓ₂|`.
    ///
    /// Iterates **every** effect, disabled ones included — deliberately
    /// asymmetric with [`reach_px`](EffectStack::reach_px) and
    /// [`render_all`], which skip them. Those two ask "what will be drawn";
    /// this one edits stored parameters. An effect that sat out three
    /// transforms while disabled and came back at its original size would
    /// be a data-loss bug, not an optimisation.
    pub(crate) fn scale_effects(&mut self, s: f32) {
        for e in &mut self.effects {
            e.scale_effects(s);
        }
    }

    /// The stack's apron in px: how far beyond `A` any effect can write.
    ///
    /// This feeds §4's `propagate_roi` step 2a (`Reff` = `Rout` expanded by
    /// the reach of the node's layer effects).
    ///
    /// It is the **max** of [`LayerEffect::reach_px`] over the stack, which
    /// is where the per-effect derivation lives. The max is the right
    /// reduction for a *working* rect — every effect is evaluated against
    /// one shared [`Field`], so the field must cover the widest of them —
    /// but it is emphatically **not** the right rect for each returned
    /// pass, and [`evaluate_in`] crops accordingly.
    ///
    /// An empty stack reaches 0: no effect, no apron. (It is not 1 px, as
    /// an unconditional AA band would give — there is no edge to
    /// antialias.)
    ///
    /// §4 writes the same apron as
    /// `ceil(blur_radius + |offset| + spread + stroke_width)`, adding
    /// `spread` **as if it were pixels** when §11.3's table types it as a
    /// unitless `0..=1` *fraction of `size_px`*. That is a dimensional
    /// confusion rather than a correctness bug — since `spread ≤ 1` the §4
    /// form over-covers the true reach by at most 1 px, and an over-wide
    /// apron costs work, never pixels. It is still worth the fold-in
    /// removing the term (or writing it `spread·size_px`), because a reader
    /// tightening that formula on the assumption the term is load-bearing
    /// would be tightening the wrong one.
    ///
    /// **Allocation hazard, and a fold-in input.** §11.3 types shadow
    /// `distance` as `0..=30_000` px, so a single legal drop shadow returns
    /// an apron of 30_250 px — and [`evaluate`] inflates the layer rect by
    /// it *on all four sides*, which for even a 1×1 layer is a
    /// 60_501² plane, ~14.6 GB of `f32`. Nothing here clamps it, and
    /// deliberately so: this side of the wall has no validation door, and
    /// inventing a silent cap would make the engine disagree with the
    /// (unwritten) rows. The rows must therefore say what bounds effect
    /// geometry — a hard parameter cap, a document-diagonal-relative cap,
    /// or a §4 ROI intersection that clips `Reff` to the canvas before any
    /// plane is allocated. The last is the likeliest right answer and it is
    /// a *compositor* obligation, not one this function can discharge.
    ///
    /// **The per-pass crop narrows that hazard without closing it, and the
    /// distinction is the fold-in input.** Before the crop, a stack of `n`
    /// effects returned `n` planes at the *stack* apron, so one legal
    /// drop shadow beside four zero-reach effects retained ~73 GB rather
    /// than ~14.6 GB. Cropping removes the ×`n`. It does **not** remove
    /// the 14.6 GB: the shared [`Field`] is still allocated at the stack
    /// apron before any pass exists, so the peak is unchanged and only the
    /// retained footprint improves. A ceiling still has to come from
    /// above — see the paragraph above, and FI-16/FI-18's class.
    pub(crate) fn reach_px(&self) -> i32 {
        self.enabled().map(|e| e.reach_px()).max().unwrap_or(0)
    }

    /// The effects that will actually render, in seat order.
    ///
    /// One filter, in one place: [`reach_px`](EffectStack::reach_px) and
    /// [`render_all`] must agree about what "enabled" means, or a disabled
    /// effect either sizes an apron it never fills or fills a pass the
    /// apron was never sized for.
    pub(crate) fn enabled(&self) -> impl Iterator<Item = &LayerEffect> {
        self.effects.iter().filter(|e| e.enabled())
    }

    /// Whether any effect will render. Distinct from
    /// [`is_empty`](EffectStack::is_empty), which asks whether the stack
    /// *holds* anything — a stack of five disabled effects is not empty,
    /// but it draws nothing.
    pub(crate) fn draws_nothing(&self) -> bool {
        self.enabled().next().is_none()
    }
}

/// Evaluate the stack against a layer's coverage.
///
/// `a` is `A` — coverage at **full fill opacity**. Returns the passes in
/// §11.3 render order (ascending `order`); an empty stack returns no
/// passes.
///
/// Each pass comes back over **its own** reach, not the stack's: see
/// [`evaluate_in`]. The working rect is sized from the stack apron because
/// one shared [`Field`] serves every effect, but a colour overlay in a
/// stack with a long drop shadow is still returned at the layer's own size.
pub(crate) fn evaluate(a: &MaskPlane, stack: &EffectStack, light: GlobalLight) -> Vec<EffectPass> {
    if stack.draws_nothing() {
        return Vec::new();
    }
    let pad = stack.reach_px();
    let rect = a.rect.inflate(pad, pad);
    evaluate_in(a, stack, light, rect)
}

/// [`evaluate`] over an explicit working rect.
///
/// # Returned passes are cropped to each effect's own reach
///
/// `rect` is where the stack is *evaluated*; it is not where each pass is
/// *returned*. Every effect shares one [`Field`], so the working rect has
/// to span the widest effect's apron — but the passes do not, and returning
/// all of them at that width means a stack of `n` effects hands the
/// compositor `n` planes sized for its single largest one. Each pass is
/// therefore cropped to `a.rect` inflated by its own
/// [`LayerEffect::reach_px`], intersected with `rect`.
///
/// This is safe **only** because it is a crop of the result and not of the
/// evaluation — the distinction the section below is entirely about. Every
/// discarded sample was computed against the full apron and is provably
/// zero (`field::crop` is a copy; `LayerEffect::reach_px` is what makes it
/// lossless), so the arithmetic is unchanged and only the returned plane
/// shrinks. `effects::tests::cropping_a_pass_loses_no_coverage` is the
/// falsifier, and it measures the **uncropped** planes on purpose.
///
/// # The rect must contain the whole apron — effects are NOT tile-local
///
/// `rect` MUST contain `a.rect` inflated by [`EffectStack::reach_px`].
/// Evaluating over anything smaller does not merely clip the output, it
/// **changes the values inside the overlap**, so a caller that tiled this
/// function would get visible seams rather than a cropped-but-correct
/// result. Two independent mechanisms cause it, and neither is a bug that
/// can be fixed here:
///
/// * [`Field::new`] resamples `A` onto `rect` and reads 0 outside it, so
///   both the §13.2 EDT and the §16.3 Gaussian see coverage that *ends* at
///   the rect edge. A blur whose kernel overhangs the edge integrates
///   those false zeros — measured on a 40 px square with a 6 px softer
///   outer glow, a tile cut 4 px above the layer reports 0.240 where the
///   apron-sized evaluation reports 0.400.
/// * Inner glow at `source = center` is worse than edge-local: its
///   `size_max` is a **global reduction over the layer's whole interior**
///   (see [`inner_glow`]), so cropping rescales the entire ramp. Same
///   fixture, a quadrant crop reports 1.000 where the correct value is
///   0.333 — a 3× error in the *middle* of the tile, nowhere near its edge.
///
/// The containment precondition makes both harmless: with the full apron
/// present, every pixel the EDT and the blur read is real coverage, and the
/// interior that `size_max` reduces over is entire. The `debug_assert`
/// below is what keeps that precondition from being merely documented.
///
/// This is a **fold-in input, not a local API wart**: §4's `propagate_roi`
/// hands the compositor a per-node `Reff`, and the natural next step —
/// evaluating effects per tile inside that ROI — is exactly what this
/// function cannot do. Whether layer effects get a tile-local formulation
/// (possible for shadow/stroke/outer glow, *impossible* for centre-source
/// inner glow as §11.3 defines it) or are pinned as whole-layer passes is a
/// determination the layer-style rows have to make.
pub(crate) fn evaluate_in(
    a: &MaskPlane,
    stack: &EffectStack,
    light: GlobalLight,
    rect: IRect,
) -> Vec<EffectPass> {
    render_all(a, stack, light, rect, true)
}

/// [`evaluate_in`] without the per-pass crop: every pass comes back over
/// the **whole** working `rect`.
///
/// This exists so the crop stays falsifiable. Cropping a pass to its own
/// reach is only sound if no coverage lives outside that reach — and a test
/// that asks "did anything escape?" of the *cropped* planes is asking a
/// question the crop has already answered, which is a green that could not
/// have been red. The escape and losslessness tests therefore measure the
/// uncropped planes and compare, so a wrong [`LayerEffect::reach_px`] shows
/// up as lost mass rather than as a rect that helpfully shrank around the
/// mistake.
///
/// It is not a production path: nothing but the tests calls it.
pub(crate) fn evaluate_in_uncropped(
    a: &MaskPlane,
    stack: &EffectStack,
    light: GlobalLight,
    rect: IRect,
) -> Vec<EffectPass> {
    render_all(a, stack, light, rect, false)
}

/// The shared body of [`evaluate_in`] and [`evaluate_in_uncropped`].
///
/// `crop` is applied **inside the map**, while the effect that produced the
/// pass is still in hand, so the per-effect reach never has to be
/// re-associated with a pass after the fact.
///
/// The crop is a crop of the **result**, taken after the pass has been
/// evaluated over the full working rect. That is the whole distinction
/// [`evaluate_in`]'s docs turn on: narrowing the rect an effect is
/// *evaluated over* changes values inside the overlap, while discarding
/// already-computed pixels that are provably zero does not. Note that it
/// therefore buys no arithmetic — every pass is computed at full working
/// size either way, and only the returned plane is smaller.
///
/// # Render order is `EffectStack`'s invariant, asserted here, not re-sorted
///
/// This used to end with a `sort_by_key(|p| p.order)`. Once
/// [`EffectStack::set`] began inserting at the §11.3 seat, that sort became
/// **dead** — the stack is stored in render order, so the passes come out
/// of the map already ascending and the sort could never move anything.
///
/// It is not kept as belt-and-braces, and the reason is worth stating: a
/// sort here would *silently repair* a broken [`EffectStack::set`], and a
/// repair is strictly worse than a report. The `debug_assert` below reports
/// instead — and the difference is measured, not asserted. Perturbing `set`
/// to append reds **8** tests with the assert in place and **4** with it
/// defanged, so the assert doubles the blast radius of a broken seat
/// insertion. (Defanging the assert on its own reds nothing, as a guard
/// should: it fires only on a violation, so it is falsifiable in
/// combination and not alone.)
///
/// It also removes a second answer to "what is the render order". §11.3's
/// table is [NORMATIVE] and fixed; [`LayerEffect::order`] is its one home,
/// and a sort in the evaluator would have been a second place that
/// ordering happened.
///
/// The battery is how this was found, not review: after the seat-order
/// change, deleting the sort reded **nothing at all**.
fn render_all(
    a: &MaskPlane,
    stack: &EffectStack,
    light: GlobalLight,
    rect: IRect,
    crop: bool,
) -> Vec<EffectPass> {
    let pad = stack.reach_px();
    debug_assert!(
        rect.contains_rect(&a.rect.inflate(pad, pad)),
        "effects working rect {rect:?} is narrower than the {pad}px apron around {:?}: \
         evaluating here would change values inside the overlap, not just clip them",
        a.rect
    );
    let f = Field::new(a, rect);
    let passes: Vec<EffectPass> = stack
        .enabled()
        .map(|e| {
            let mut p = render_one(&f, e, light);
            if crop {
                let r = e.reach_px();
                // The `intersect` is **provably a no-op**, and the honest
                // record of that is worth more than the reassuring comment
                // it replaces (which claimed it caught non-centred working
                // rects — it cannot; removing it reds nothing, measured).
                // The apron precondition asserted above gives
                // `rect ⊇ a.rect.inflate(pad)`, and `pad` is the max of
                // every `r`, so `a.rect.inflate(r) ⊆ rect` always.
                //
                // It stays because it is what keeps `field::crop`'s own
                // containment assert from depending on a *caller's*
                // precondition holding: without it, a future relaxation of
                // the apron rule turns a documented contract violation into
                // a panic inside a helper two frames away.
                p.coverage = field::crop(&p.coverage, a.rect.inflate(r, r).intersect(&rect));
            }
            p
        })
        .collect();
    debug_assert!(
        passes.windows(2).all(|w| w[0].order < w[1].order),
        "effect stack is not stored in §11.3 seat order: {:?} — EffectStack::set must \
         insert at the seat, and this evaluator deliberately does not re-sort",
        passes.iter().map(|p| p.order).collect::<Vec<_>>()
    );
    passes
}

/// Dispatch to the per-effect renderer, stamping the pass with the effect's
/// §11.3 seat.
///
/// `order` is threaded in from [`LayerEffect::order`] rather than written
/// again inside each renderer: the render order is [NORMATIVE] and fixed,
/// so it gets exactly one home. Two copies of a fixed table is how a table
/// stops being fixed.
fn render_one(f: &Field, e: &LayerEffect, light: GlobalLight) -> EffectPass {
    let order = e.order();
    match e {
        LayerEffect::DropShadow(p) => drop_shadow(order, f, p, light),
        LayerEffect::OuterGlow(p) => outer_glow(order, f, p),
        LayerEffect::ColorOverlay(p) => color_overlay(order, f, p),
        LayerEffect::InnerGlow(p) => inner_glow(order, f, p),
        LayerEffect::Stroke(p) => stroke(order, f, p),
    }
}

/// §11.3 algorithm 1 — drop shadow.
///
/// 1. `S = A` translated by the shadow offset;
/// 2. dilate by `spread·size_px` when `spread > 0`;
/// 3. blur with `σ = (1−spread)·size_px/3`;
/// 4. `S ← S·(1 − A)` — the layer knocks out its own shadow;
/// 5. fill with `color` at `S·opacity`, composite **under** the layer.
fn drop_shadow(order: u8, f: &Field, p: &ShadowParams, light: GlobalLight) -> EffectPass {
    let angle = if p.use_global_light {
        light.angle_deg
    } else {
        p.angle_deg
    };
    let (dx, dy) = field::shadow_offset(angle, p.distance_px);
    let mut s = field::translate(&f.a, f.rect, dx, dy);
    if p.spread > 0.0 {
        s = field::dilate(&s, f.rect, p.spread * p.size_px);
    }
    field::blur(
        &mut s,
        field::sigma_for_radius((1.0 - p.spread) * p.size_px),
    );
    field::mul_inverse_in_place(&mut s, &f.a);
    EffectPass {
        order,
        slot: Slot::Under,
        coverage: s,
        color: p.color,
        blend: p.blend,
        opacity: p.opacity,
    }
}

/// §11.3 algorithm 3 — outer glow.
///
/// `softer` grows-and-blurs the coverage; `precise` reads the ramp straight
/// off the distance field. Then exterior-only, then the `range` rescale, in
/// that order (the section's steps 1→2→3).
///
/// `spread` participates in `softer` only — §11.3's `precise` formula is
/// `f(d/size)` with no `spread` term, so the parameter is inert in that
/// technique. That asymmetry is what the section says rather than a
/// shortcut taken here, but a UI that leaves a live `spread` slider next to
/// a `precise` technique picker would be lying to the user, so it is
/// recorded as a fold-in input.
fn outer_glow(order: u8, f: &Field, p: &GlowParams) -> EffectPass {
    let mut g = match p.technique {
        GlowTechnique::Softer => {
            let mut t = field::dilate(&f.a, f.rect, p.spread * p.size_px);
            field::blur(
                &mut t,
                field::sigma_for_radius((1.0 - p.spread) * p.size_px),
            );
            t
        }
        GlowTechnique::Precise => {
            let mut t = f.empty_plane();
            let size = p.size_px.max(f32::EPSILON);
            for (o, dv) in t.v.iter_mut().zip(f.d.iter()) {
                *o = field::ramp(dv / size);
            }
            t
        }
    };
    // Step 2 — exterior only.
    field::mul_inverse_in_place(&mut g, &f.a);
    // Step 3 — the `range` falloff rescale. See `apply_range`.
    apply_range(&mut g, p.range);
    EffectPass {
        order,
        slot: Slot::Under,
        coverage: g,
        color: p.color,
        blend: p.blend,
        opacity: p.opacity,
    }
}

/// §11.3 algorithm 4 — inner glow.
///
/// `source = edge` ramps from the boundary inward, `G = f(−d/size)·A`.
/// `source = center` "inverts the ramp (maximum at the deepest interior)",
/// `G = (1 − f(−d/size_max))·A`.
///
/// **`size_max` is undefined in the spec** — the symbol occurs exactly once
/// in the whole document, in that formula, and is introduced nowhere. The
/// reading implemented here is the one the surrounding prose forces: the
/// deepest interior distance of this layer's own coverage,
/// `size_max = max(−d)` over the interior, which puts the maximum exactly
/// at the deepest point and zero at the contour. Note the consequence —
/// under this reading `size_px` does **not** participate in `center` mode
/// at all, which is surprising enough to be worth the fold-in pinning it
/// explicitly rather than leaving the next reader to re-derive it.
///
/// The second consequence is sharper and is a **fold-in input**: that
/// `max` is a **global reduction over the layer's entire interior**, so
/// centre-source inner glow is the one effect in the MVP five that is not
/// a local operator. Every other effect reads `A` and `d` within a bounded
/// apron of the pixel being written; this one's value at the centre of a
/// shape depends on how far away the shape's *farthest* interior point is.
/// A tile-local evaluation of it is therefore not merely inaccurate but
/// ill-defined (see [`evaluate_in`] for the measured 3× error), and any
/// incremental/tiled compositor design has to special-case it. If the
/// fold-in prefers a local reading — `size_max = size_px`, making `center`
/// the plain inversion of `edge` — that would remove the anomaly, and it
/// is worth choosing deliberately rather than inheriting.
///
/// `technique` and `spread` are likewise unused here: §11.3 gives inner
/// glow only these two closed-form formulas and says merely "as outer glow
/// mirrored inside", never restating the softer/precise split or the
/// dilation for the inner case. Rather than invent a mirrored variant, the
/// MVP implements what is written and records the gap.
fn inner_glow(order: u8, f: &Field, p: &GlowParams) -> EffectPass {
    let mut g = f.empty_plane();
    match p.source {
        GlowSource::Edge => {
            let size = p.size_px.max(f32::EPSILON);
            for (o, dv) in g.v.iter_mut().zip(f.d.iter()) {
                *o = field::ramp(-dv / size);
            }
        }
        GlowSource::Center => {
            // max(−d) over the interior (d < 0). Guarded: a fully empty
            // coverage has no interior and yields a zero plane.
            let size_max =
                f.d.iter()
                    .filter(|dv| **dv < 0.0)
                    .fold(0.0f32, |m, dv| m.max(-dv))
                    .max(f32::EPSILON);
            for (o, dv) in g.v.iter_mut().zip(f.d.iter()) {
                *o = 1.0 - field::ramp(-dv / size_max);
            }
        }
    }
    // Interior only.
    field::mul_in_place(&mut g, &f.a);
    apply_range(&mut g, p.range);
    EffectPass {
        order,
        slot: Slot::Interior,
        coverage: g,
        color: p.color,
        blend: p.blend,
        opacity: p.opacity,
    }
}

/// §11.3 algorithm 5 — stroke. A `size`-wide band along the alpha edge,
/// selected by signed distance and antialiased at both edges.
///
/// The band placement is exact per the section: `outside: 0 ≤ d ≤ size`,
/// `inside: −size ≤ d ≤ 0`, `center: |d| ≤ size/2`.
///
/// §11.3 puts Stroke at order 11 (topmost) but also says its inside
/// portions composite "over the interior result" and its outside portions
/// "above all exterior effects" — i.e. the effect straddles the interior
/// clip. One pass at [`Slot::Above`] is the faithful model for an opaque
/// stroke; for a non-`normal` blend the split could differ, which is a
/// compositor-placement question for the fold-in.
fn stroke(order: u8, f: &Field, p: &StrokeParams) -> EffectPass {
    let (lo, hi) = match p.position {
        StrokeAlign::Outside => (0.0, p.size_px),
        StrokeAlign::Inside => (-p.size_px, 0.0),
        StrokeAlign::Center => (-p.size_px / 2.0, p.size_px / 2.0),
    };
    let mut b = f.empty_plane();
    field::band(&f.d, lo, hi, &mut b);
    // Irrefutable today because `StrokeFill` has one variant. That is
    // deliberate: when gradient/pattern fills land this line stops
    // compiling, so widening the fill cannot silently fall through to the
    // solid path.
    let StrokeFill::Solid(color) = p.fill;
    EffectPass {
        order,
        slot: Slot::Above,
        coverage: b,
        color,
        blend: p.blend,
        opacity: p.opacity,
    }
}

/// §11.3 algorithm 6 — colour overlay. Fill `A` with `color`; composite
/// over the interior stack. The simplest effect in the set, and the one
/// that most clearly shows the `fill_opacity` property: its coverage is
/// `A` at full fill, never the attenuated fill alpha.
fn color_overlay(order: u8, f: &Field, p: &OverlayParams) -> EffectPass {
    EffectPass {
        order,
        slot: Slot::Interior,
        coverage: f.a.clone(),
        color: p.color,
        blend: p.blend,
        opacity: p.opacity,
    }
}

/// §11.3 glow step 3, the `range` falloff rescale.
///
/// The spec writes this as `G ← f(G/range)` with `f` the same decreasing
/// clamped ramp used to build the glow. Applied literally that **inverts**
/// the glow — `f` maps 0 to 1, so a fully transparent exterior pixel would
/// come back fully opaque and the effect would render inside-out. Since
/// `range ∈ 0.01..=1` and the step is described as "rescales the falloff
/// domain", the coherent reading is a domain rescale with the same clamp:
/// `G ← clamp(G/range, 0, 1)`, which tightens the falloff as `range`
/// shrinks and is the identity at `range = 1` (the value at which a rescale
/// must be a no-op — a property the literal reading also fails).
///
/// This is a genuine defect in the §11.3 text, not a local preference;
/// it is recorded as a fold-in input.
fn apply_range(g: &mut MaskPlane, range: f32) {
    let r = range.clamp(0.01, 1.0);
    if (r - 1.0).abs() < f32::EPSILON {
        return;
    }
    for v in &mut g.v {
        *v = (*v / r).clamp(0.0, 1.0);
    }
}
