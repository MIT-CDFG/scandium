// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Unit tests for the §11.3 effects stack.
//!
//! These are the **only** thing asserting this module exists and behaves:
//! nothing outside `appname-tools` can reach it (the honest wall), so there
//! is no golden, no conformance probe and no drone cell covering it yet.
//! That makes falsifiability the whole job here. Every test below was
//! developed by perturbing the implementation, observing RED, and
//! reverting to GREEN — a test that cannot go red is indistinguishable
//! from no test at all, and behind a wall that is the difference between
//! scaffolding and an empty claim.

use appname_core::{BlendMode, IRect};

use super::*;
use crate::select::MaskPlane;

// --------------------------------------------------------------------
// Fixtures
// --------------------------------------------------------------------

/// A fully opaque square of side `2·half`, centred on the origin, whose
/// plane rect is exactly the square (so every read outside it is the 0 of
/// §2.4/§13.2 rather than a padded zero we had to remember to write).
fn square(half: i32) -> MaskPlane {
    let mut m = MaskPlane::new(IRect::new(-half, -half, half, half));
    for v in &mut m.v {
        *v = 1.0;
    }
    m
}

fn mass(p: &MaskPlane) -> f32 {
    p.v.iter().sum()
}

/// Total coverage over the pixels whose document coords satisfy `f`.
fn mass_where(p: &MaskPlane, f: impl Fn(i32, i32) -> bool) -> f32 {
    let mut acc = 0.0;
    for y in p.rect.min.y..p.rect.max.y {
        for x in p.rect.min.x..p.rect.max.x {
            if f(x, y) {
                acc += p.get(x, y);
            }
        }
    }
    acc
}

fn only_pass(stack: &EffectStack, a: &MaskPlane, light: GlobalLight) -> EffectPass {
    let mut p = evaluate(a, stack, light);
    assert_eq!(p.len(), 1, "fixture expects exactly one effect");
    p.remove(0)
}

fn stack_of(e: LayerEffect) -> EffectStack {
    let mut s = EffectStack::new();
    s.set(e);
    s
}

// --------------------------------------------------------------------
// Drop shadow
// --------------------------------------------------------------------

/// §11.3: the shadow offsets **away** from the light. At UI angle `α = 0`
/// the light comes from the right, so the shadow lands on the **left**.
///
/// Perturbation evidence: negating the `dx`/`dy` signs in
/// `field::shadow_offset` (i.e. reading the prose's `(cos θ, sin θ)` with
/// `θ = α`, the natural misreading) puts the whole sliver on the right and
/// reds both assertions.
#[test]
fn drop_shadow_offsets_away_from_the_light() {
    let a = square(20);
    let s = stack_of(LayerEffect::DropShadow(ShadowParams {
        angle_deg: 0.0,
        distance_px: 10.0,
        size_px: 0.0,
        spread: 0.0,
        use_global_light: false,
        ..Default::default()
    }));
    let pass = only_pass(&s, &a, GlobalLight::default());

    let left = mass_where(&pass.coverage, |x, _| x < -20);
    let right = mass_where(&pass.coverage, |x, _| x >= 20);
    assert!(left > 0.0, "expected shadow mass left of the layer: {left}");
    assert_eq!(right, 0.0, "expected no shadow mass right of the layer");
}

/// §11.3 drop-shadow step 4: `S ← S·(1 − A)` — the layer knocks out its own
/// shadow, so no shadow survives under opaque coverage.
///
/// Perturbation evidence: deleting the `mul_inverse_in_place` call leaves
/// the translated square intact under the layer and `get(0, 0)` becomes
/// 1.0, reding the first assertion.
#[test]
fn drop_shadow_is_knocked_out_by_its_own_layer() {
    let a = square(20);
    let s = stack_of(LayerEffect::DropShadow(ShadowParams {
        angle_deg: 0.0,
        distance_px: 10.0,
        size_px: 0.0,
        spread: 0.0,
        use_global_light: false,
        ..Default::default()
    }));
    let pass = only_pass(&s, &a, GlobalLight::default());

    assert!(
        pass.coverage.get(0, 0) < 1e-6,
        "shadow leaked under opaque coverage: {}",
        pass.coverage.get(0, 0)
    );
    // ...but it does survive just outside the layer, or the test above
    // would pass on an all-zero plane.
    assert!(
        pass.coverage.get(-25, 0) > 0.5,
        "shadow vanished entirely: {}",
        pass.coverage.get(-25, 0)
    );
}

/// §11.3 "Global light" [NORMATIVE]: an effect with `use_global_light =
/// true` ignores its local `angle_deg` and reads the document pair, so one
/// `meta.set` retargets every opted-in effect — and must **not** move an
/// opted-out one.
///
/// This is the sharpest test in the file: it fails in one direction if the
/// global pair is ignored, and in the other if it is applied
/// unconditionally.
///
/// Perturbation evidence: forcing `let angle = light.angle_deg;` in
/// `drop_shadow` (dropping the opt-in check) reds the opted-out half;
/// forcing `let angle = p.angle_deg;` reds the opted-in half.
#[test]
fn global_light_retargets_only_opted_in_effects() {
    let a = square(20);
    let at = |use_global: bool, light_angle: f32| {
        let s = stack_of(LayerEffect::DropShadow(ShadowParams {
            angle_deg: 0.0,
            distance_px: 10.0,
            size_px: 0.0,
            spread: 0.0,
            use_global_light: use_global,
            ..Default::default()
        }));
        only_pass(
            &s,
            &a,
            GlobalLight {
                angle_deg: light_angle,
                altitude_deg: 30.0,
            },
        )
        .coverage
    };

    // Opted in: moving the document light moves the shadow.
    let g0 = at(true, 0.0);
    let g90 = at(true, 90.0);
    assert!(
        g0.v != g90.v,
        "opted-in shadow ignored the document global light"
    );
    // At α = 0 the shadow goes left; at α = 90 it goes down (+y).
    assert!(mass_where(&g0, |x, _| x < -20) > 0.0);
    assert!(mass_where(&g90, |_, y| y >= 20) > 0.0);

    // Opted out: the document light is irrelevant, local angle 0 governs.
    let l0 = at(false, 0.0);
    let l90 = at(false, 90.0);
    assert_eq!(
        l0.v, l90.v,
        "opted-out shadow followed the document global light"
    );
}

// --------------------------------------------------------------------
// Glows
// --------------------------------------------------------------------

/// §11.3 outer-glow step 2: exterior only, `G ← G·(1 − A)`.
///
/// Perturbation evidence: dropping the `mul_inverse_in_place` call fills
/// the interior (the `softer` dilate/blur of a solid square is 1.0 deep
/// inside) and reds the interior assertion.
#[test]
fn outer_glow_is_exterior_only() {
    let a = square(20);
    let s = stack_of(LayerEffect::OuterGlow(GlowParams {
        size_px: 6.0,
        spread: 0.0,
        range: 1.0,
        technique: GlowTechnique::Softer,
        ..Default::default()
    }));
    let pass = only_pass(&s, &a, GlobalLight::default());

    assert!(
        pass.coverage.get(0, 0) < 1e-6,
        "outer glow bled into the interior: {}",
        pass.coverage.get(0, 0)
    );
    assert!(
        pass.coverage.get(22, 0) > 0.0,
        "outer glow produced nothing outside the layer"
    );
}

/// §11.3 inner-glow: clipped to `A`, so nothing outside the layer.
///
/// Perturbation evidence: swapping `mul_in_place` for
/// `mul_inverse_in_place` inverts the clip and reds both assertions at
/// once.
#[test]
fn inner_glow_is_interior_only() {
    let a = square(20);
    let s = stack_of(LayerEffect::InnerGlow(GlowParams {
        size_px: 6.0,
        range: 1.0,
        source: GlowSource::Edge,
        ..Default::default()
    }));
    let pass = only_pass(&s, &a, GlobalLight::default());

    assert_eq!(
        mass_where(&pass.coverage, |x, y| !(-20..20).contains(&x)
            || !(-20..20).contains(&y)),
        0.0,
        "inner glow wrote outside the layer"
    );
    // `edge` ramps from the boundary inward: bright at the edge, dark deep
    // inside (the square is 40 px across, far deeper than the 6 px size).
    assert!(pass.coverage.get(-19, 0) > 0.5, "edge should be lit");
    assert!(
        pass.coverage.get(0, 0) < 1e-6,
        "deep interior should be dark"
    );
}

/// §11.3 inner-glow `source = center`: "maximum at the deepest interior" —
/// the exact inversion of `edge`.
///
/// This pins the reading chosen for the undefined `size_max` symbol (see
/// `inner_glow`'s docs). If the fold-in rules a different reading, this
/// test is where it lands.
///
/// Perturbation evidence: dropping the `1.0 -` inversion makes the centre
/// dark and the edge bright, reding both assertions.
#[test]
fn inner_glow_center_source_is_brightest_deepest() {
    let a = square(20);
    let s = stack_of(LayerEffect::InnerGlow(GlowParams {
        size_px: 6.0,
        range: 1.0,
        source: GlowSource::Center,
        ..Default::default()
    }));
    let pass = only_pass(&s, &a, GlobalLight::default());

    let centre = pass.coverage.get(0, 0);
    let edge = pass.coverage.get(-19, 0);
    assert!(
        centre > edge,
        "centre ({centre}) should out-glow the edge ({edge})"
    );
    assert!(centre > 0.9, "deepest interior should be near maximum");
}

/// §11.3 glow `range`: the identity at `range = 1`, and tightening below
/// it. The literal `G ← f(G/range)` reading of the spec fails *both* halves
/// (it inverts, and it is not the identity at 1) — this test is the
/// standing evidence for the reading `apply_range` implements.
///
/// Perturbation evidence: implementing `apply_range` literally as
/// `field::ramp(v / r)` reds the **third** assertion — measured, after the
/// mechanical battery caught an earlier version of this comment claiming
/// the *identity* assertion would red. It does not: `apply_range` returns
/// early at `range == 1`, so the literal body never executes in the
/// identity case, and at `range = 0.5` the inverted glow has strictly more
/// mass than the correct one, so the "tightens" assertion passes too. The
/// literal reading therefore survived both of the original assertions.
///
/// Worse, the original "far exterior" probe was **vacuous**: it summed over
/// `x > 40` on a plane whose rect stops at `x = 30` (the layer's 20 px
/// half-extent plus the 10 px apron), so it summed over nothing and would
/// have returned 0 for any implementation whatsoever. The battery is what
/// exposed that — the probe now runs through [`evaluate_in`] on a rect wide
/// enough to *contain* a far exterior, and is asserted non-empty before it
/// is trusted. Under the literal reading an exterior zero comes back as
/// `ramp(0) = 1.0`, so the whole far field lights up and both probes red.
#[test]
fn glow_range_is_identity_at_one_and_tightens_below() {
    let a = square(20);
    let size = 9.0f32;
    // The glow reaches `size` px past the layer edge at x = 20, so the far
    // field starts at x = 30. Evaluate over a rect with 20 px of room
    // beyond that, or "the far exterior stays empty" is a claim about
    // pixels that do not exist.
    let far_from = 20 + size as i32 + 1;
    let glow = |range: f32| {
        let s = stack_of(LayerEffect::OuterGlow(GlowParams {
            size_px: size,
            spread: 0.0,
            range,
            technique: GlowTechnique::Precise,
            ..Default::default()
        }));
        let pad = s.reach_px() + 20;
        // UNCROPPED on purpose. `evaluate_in` crops each pass to its own
        // reach, which would shrink this plane back to exactly the glow's
        // 10 px apron — and then "the far exterior stays empty" would be a
        // claim about pixels the crop removed rather than about pixels the
        // implementation left dark. That is the same vacuity this test was
        // rewritten to escape once already, and when the crop landed the
        // `full.rect.max.x > far_from + 8` guard below is what caught it.
        let mut p = evaluate_in_uncropped(&a, &s, GlobalLight::default(), a.rect.inflate(pad, pad));
        assert_eq!(p.len(), 1);
        p.remove(0).coverage
    };

    let full = glow(1.0);
    // Guard the guard: the far field must actually be sampled.
    assert!(
        full.rect.max.x > far_from + 8,
        "fixture too small to have a far exterior at all"
    );

    // Identity at range = 1: the far exterior stays empty.
    assert_eq!(
        mass_where(&full, |x, _| x > far_from),
        0.0,
        "range = 1 must not manufacture coverage"
    );
    // Below 1 the falloff tightens => strictly more mass, never less.
    let tight = glow(0.5);
    assert!(
        mass(&tight) > mass(&full),
        "range < 1 should tighten the falloff (more mass): {} vs {}",
        mass(&tight),
        mass(&full)
    );
    // ...and it tightens the falloff *within the effect's reach* — it is a
    // domain rescale, not a source of new coverage. A `range` that lights
    // up the far exterior is the inverted reading.
    assert_eq!(
        mass_where(&tight, |x, _| x > far_from),
        0.0,
        "range < 1 must not manufacture coverage beyond the glow's reach"
    );
}

// --------------------------------------------------------------------
// Stroke
// --------------------------------------------------------------------

/// §11.3 stroke band placement by `position`: `outside: 0 ≤ d ≤ size`,
/// `inside: −size ≤ d ≤ 0`, `center: |d| ≤ size/2`.
///
/// Perturbation evidence: swapping the `Inside`/`Outside` `(lo, hi)` pairs
/// reds the first two blocks; widening `Center` from `±size/2` to `±size`
/// reds the two "must stop at" probes in the third.
///
/// An earlier version of this test asserted only that the centre band was
/// *symmetric* about the contour and that each band's mass fell mostly on
/// the right side. The battery caught it: a centre band widened to `±size`
/// is still perfectly symmetric, so that assertion stayed GREEN under a
/// real defect. Probing the band **edges** by signed distance is what makes
/// the width falsifiable, so the probes below are pinned to exact `d`.
#[test]
fn stroke_band_sits_where_position_says() {
    // The square's rect is [-20, 20), so x = -20 is the last inside column
    // and x = -21 the first outside one. With the §13.2 ±(D − 0.5) lattice
    // pin, the signed distance along y = 0 is:
    //
    //   x   | -25  -24  -23  -22  -21 | -20  -19  -18  -17  -16
    //   d   | +4.5 +3.5 +2.5 +1.5 +0.5| -0.5 -1.5 -2.5 -3.5 -4.5
    let a = square(20);
    let size = 4.0f32;
    // UNCROPPED, over a rect wide enough for every probe below to land on
    // a real pixel.
    //
    // This is a **pre-existing** vacuity, not one the crop introduced: an
    // inside stroke reaches 0 px past `A`, so `evaluate`'s own rect was
    // already only `a.rect` + the 1 px AA band, and the committed
    // `ins.get(-22, 0) < 0.5` probe — "inside band must not reach
    // d = +1.5" — was reading two columns beyond that plane. It was
    // scoring the absent-read 0 of §2.4 and would have passed for *any*
    // implementation, including the `Inside`/`Outside` swap the doc
    // comment above claims it catches. (The swap does still red this
    // test, via the `d = −2.5` probe; but this assertion was not the one
    // doing the work.) Same family as the vacuous `range` probe: an
    // assertion whose subject is outside the plane it samples.
    let probe_rect = a.rect.inflate(12, 12);
    let banded = |position: StrokeAlign| {
        let s = stack_of(LayerEffect::Stroke(StrokeParams {
            size_px: size,
            position,
            ..Default::default()
        }));
        let mut p = evaluate_in_uncropped(&a, &s, GlobalLight::default(), probe_rect);
        assert_eq!(p.len(), 1);
        p.remove(0).coverage
    };
    // Guard the guard: every probe column below must be inside the plane.
    assert!(
        probe_rect.min.x <= -25 && probe_rect.max.x > -16,
        "probe rect {probe_rect:?} does not cover the columns this test reads"
    );

    // outside: 0 ≤ d ≤ 4 — covers d = +1.5 and +3.5, stops before +4.5.
    let out = banded(StrokeAlign::Outside);
    assert!(out.get(-22, 0) > 0.5, "outside band should cover d = +1.5");
    assert!(out.get(-24, 0) > 0.5, "outside band should cover d = +3.5");
    assert!(out.get(-25, 0) < 0.5, "outside band must stop at d = +4");
    assert!(
        out.get(-18, 0) < 0.5,
        "outside band must not reach d = −2.5"
    );

    // inside: −4 ≤ d ≤ 0 — the mirror image.
    let ins = banded(StrokeAlign::Inside);
    assert!(ins.get(-18, 0) > 0.5, "inside band should cover d = −2.5");
    assert!(ins.get(-17, 0) > 0.5, "inside band should cover d = −3.5");
    assert!(ins.get(-16, 0) < 0.5, "inside band must stop at d = −4");
    assert!(ins.get(-22, 0) < 0.5, "inside band must not reach d = +1.5");

    // center: |d| ≤ 2 — straddles the contour, and is HALF as wide.
    let ctr = banded(StrokeAlign::Center);
    assert!(ctr.get(-21, 0) > 0.5, "centre band should cover d = +0.5");
    assert!(ctr.get(-20, 0) > 0.5, "centre band should cover d = −0.5");
    assert!(
        ctr.get(-23, 0) < 0.5,
        "centre band must stop at d = +size/2, not +size"
    );
    assert!(
        ctr.get(-18, 0) < 0.5,
        "centre band must stop at d = −size/2, not −size"
    );
}

// --------------------------------------------------------------------
// Stack semantics
// --------------------------------------------------------------------

/// §11.3 "Inter-effect render order" [NORMATIVE]: one fixed order, and
/// effects never reorder. The seats are the full 1..=11 table, so the MVP
/// five must come back as exactly `[1, 2, 6, 8, 11]` regardless of the
/// order they were added in.
///
/// Perturbation evidence: changing `ColorOverlay`'s `order()` from 6 to 3
/// reds the equality.
///
/// **The other half of this comment was true when written and is not any
/// more, which is why the battery is re-run rather than trusted.** It read
/// "removing the `sort_by_key` in `evaluate_in` reds it too, since the
/// fixture inserts them in deliberately scrambled order". Once
/// `EffectStack::set` began inserting at the §11.3 seat, the fixture below
/// stopped being scrambled *in storage* — it is scrambled only in the
/// order the `set` calls are written — so the sort became dead code and
/// deleting it reded **nothing**. The sort is gone; `set` is now the one
/// mechanism, `render_all`'s `debug_assert` reports a violation instead of
/// papering over it, and `stack_iterates_in_seat_order` is the test that
/// holds it up.
#[test]
fn render_order_is_the_fixed_seat_table() {
    let a = square(12);
    let mut s = EffectStack::new();
    // Deliberately scrambled relative to the render order.
    s.set(LayerEffect::Stroke(StrokeParams::default()));
    s.set(LayerEffect::ColorOverlay(OverlayParams::default()));
    s.set(LayerEffect::DropShadow(ShadowParams::default()));
    s.set(LayerEffect::InnerGlow(GlowParams::default()));
    s.set(LayerEffect::OuterGlow(GlowParams::default()));

    let orders: Vec<u8> = evaluate(&a, &s, GlobalLight::default())
        .iter()
        .map(|p| p.order)
        .collect();
    assert_eq!(orders, vec![1, 2, 6, 8, 11]);

    let slots: Vec<Slot> = evaluate(&a, &s, GlobalLight::default())
        .iter()
        .map(|p| p.slot)
        .collect();
    assert_eq!(
        slots,
        vec![
            Slot::Under,    // drop shadow
            Slot::Under,    // outer glow
            Slot::Interior, // colour overlay
            Slot::Interior, // inner glow
            Slot::Above,    // stroke
        ]
    );
}

/// The stack holds at most one effect of each kind (§11.3's order table has
/// exactly one seat per effect), so `set` replaces rather than appends.
///
/// Perturbation evidence: changing `set` to always `push` makes the length
/// 2 and reds the first assertion.
#[test]
fn stack_holds_at_most_one_effect_per_kind() {
    let mut s = EffectStack::new();
    s.set(LayerEffect::ColorOverlay(OverlayParams {
        color: [1.0, 0.0, 0.0],
        ..Default::default()
    }));
    s.set(LayerEffect::ColorOverlay(OverlayParams {
        color: [0.0, 1.0, 0.0],
        ..Default::default()
    }));
    assert_eq!(s.len(), 1, "second set of a kind should replace the first");

    let a = square(8);
    let pass = only_pass(&s, &a, GlobalLight::default());
    assert_eq!(pass.color, [0.0, 1.0, 0.0], "replacement lost its params");

    s.remove_kind(LayerEffect::ColorOverlay(OverlayParams::default()).kind_index());
    assert!(s.is_empty());
    assert!(evaluate(&a, &s, GlobalLight::default()).is_empty());
}

/// §11.3: `A` is the coverage **at full fill opacity**, which is what makes
/// effects survive `fill_opacity = 0` (the hollow-shape-with-effects case).
///
/// The structural half of this is enforced by the compiler — `evaluate`
/// has no `fill_opacity` parameter, so it cannot attenuate by one. The
/// behavioural half is here: colour overlay's coverage is `A` itself,
/// undiminished, so a layer whose fill contributes nothing still yields a
/// fully covered overlay pass.
///
/// Perturbation evidence: multiplying `color_overlay`'s coverage by any
/// factor < 1 (simulating a fill-opacity leak) reds the interior
/// assertion.
#[test]
fn effects_read_coverage_at_full_fill_opacity() {
    let a = square(20);
    let s = stack_of(LayerEffect::ColorOverlay(OverlayParams::default()));
    let pass = only_pass(&s, &a, GlobalLight::default());

    assert_eq!(
        pass.coverage.get(0, 0),
        1.0,
        "overlay coverage must be A at FULL fill opacity"
    );
    assert_eq!(pass.slot, Slot::Interior);
}

/// §11.3 "Effect scale on transform" [NORMATIVE]: pixel-valued parameters
/// scale by `s`; unitless ones do not. `spread` is the trap — it is a
/// *fraction of size*, so scaling it as well would compound with the
/// already-scaled `size_px`.
///
/// Perturbation evidence: adding `p.spread *= s;` to `scale_effects` reds
/// the spread assertion.
#[test]
fn scale_effects_scales_pixel_params_only() {
    let mut s = EffectStack::new();
    s.set(LayerEffect::DropShadow(ShadowParams {
        distance_px: 5.0,
        size_px: 8.0,
        spread: 0.25,
        opacity: 0.35,
        angle_deg: 120.0,
        ..Default::default()
    }));
    // DISABLED on purpose: `scale_effects` edits stored parameters, so it
    // must touch disabled effects too, unlike `reach_px` and `render_all`
    // which ask what will be drawn. An effect that sat out a transform
    // while disabled and came back at its old size is data loss.
    //
    // Perturbation evidence: changing `scale_effects` to iterate
    // `self.enabled()` instead of `self.effects` reds the stroke arm.
    s.set(LayerEffect::Stroke(StrokeParams {
        enabled: false,
        size_px: 3.0,
        opacity: 1.0,
        ..Default::default()
    }));
    s.scale_effects(2.0);
    assert!(
        s.iter().any(|e| !e.enabled()),
        "fixture must contain a disabled effect for the scale claim to bite"
    );

    for e in s.iter() {
        match e {
            LayerEffect::DropShadow(p) => {
                assert_eq!(p.distance_px, 10.0);
                assert_eq!(p.size_px, 16.0);
                assert_eq!(p.spread, 0.25, "spread is unitless and must not scale");
                assert_eq!(p.opacity, 0.35, "opacity must not scale");
                assert_eq!(p.angle_deg, 120.0, "angle must not scale");
            }
            LayerEffect::Stroke(p) => {
                assert_eq!(p.size_px, 6.0);
                assert_eq!(p.opacity, 1.0);
            }
            other => panic!("unexpected effect {other:?}"),
        }
    }
}

/// The apron [`EffectStack::reach_px`] promises must actually contain every
/// pixel the stack writes — this is what §4's `propagate_roi` relies on,
/// and an under-promise is precisely the incremental-compositor seam bug.
///
/// The `spread` sweep is the point: the dilate/blur split means the reach
/// is `distance + size` for *every* spread, and this test holds each of
/// them to the same promised apron.
///
/// Perturbation evidence: reducing the returned reach by 2 px (e.g.
/// `saturating_add(1)` → `saturating_sub(1)`) reds the `spread = 0` case,
/// where the Gaussian tail reaches the full `size_px`.
///
/// **This test must run UNCROPPED, and the reason is the whole point of
/// it.** `evaluate_in` crops each pass to `a.rect` inflated by that
/// effect's own `reach_px` — the very quantity under test. Asking the
/// cropped plane "did any coverage escape the reach?" is asking a question
/// the crop has already answered: `escaped` would be structurally 0 for
/// every implementation, including one whose reach is far too small. The
/// test would have gone on passing and stopped meaning anything, which is
/// exactly the shape of the vacuous `range` probe the previous slice
/// found. Measuring the uncropped plane keeps the escape observable, and
/// `cropping_a_pass_loses_no_coverage` then ties the crop back to it.
#[test]
fn reach_contains_every_pixel_the_stack_writes() {
    let a = square(16);
    for spread in [0.0f32, 0.25, 0.5, 1.0] {
        let s = stack_of(LayerEffect::DropShadow(ShadowParams {
            angle_deg: 0.0,
            distance_px: 7.0,
            size_px: 9.0,
            spread,
            use_global_light: false,
            ..Default::default()
        }));
        let reach = s.reach_px();
        let promised = a.rect.inflate(reach, reach);
        // Evaluate over a rect deliberately WIDER than the promise, so
        // anything written beyond it is visible rather than clipped away.
        let slack = reach + 8;
        let pass = {
            let mut p =
                evaluate_in_uncropped(&a, &s, GlobalLight::default(), a.rect.inflate(slack, slack));
            p.remove(0)
        };
        // Guard the guard: the plane must extend past the promised apron,
        // or "nothing escaped" is a statement about no pixels at all.
        assert!(
            pass.coverage.rect.max.x > promised.max.x,
            "fixture cannot observe an escape: plane {:?} does not exceed the promised {promised:?}",
            pass.coverage.rect
        );

        let escaped = mass_where(&pass.coverage, |x, y| {
            !promised.contains(appname_core::IVec2::new(x, y))
        });
        assert_eq!(
            escaped, 0.0,
            "spread {spread}: {escaped} coverage escaped the promised {reach}px apron"
        );
        // And the apron is not vacuously large: real mass lands inside it.
        assert!(mass(&pass.coverage) > 0.0, "spread {spread}: nothing drawn");
    }
}

/// A stack whose effects have wildly different reaches: the long drop
/// shadow, and four effects that reach barely past `A` or not at all.
///
/// This is the fixture the per-pass crop exists for, so the crop tests
/// share it rather than each inventing their own spread of reaches.
fn mixed_reach_stack() -> EffectStack {
    let mut s = EffectStack::new();
    s.set(LayerEffect::DropShadow(ShadowParams {
        angle_deg: 0.0,
        distance_px: 18.0,
        size_px: 6.0,
        use_global_light: false,
        ..Default::default()
    }));
    s.set(LayerEffect::OuterGlow(GlowParams {
        size_px: 5.0,
        ..Default::default()
    }));
    s.set(LayerEffect::InnerGlow(GlowParams {
        size_px: 5.0,
        source: GlowSource::Edge,
        ..Default::default()
    }));
    s.set(LayerEffect::Stroke(StrokeParams {
        size_px: 2.0,
        position: StrokeAlign::Outside,
        ..Default::default()
    }));
    s.set(LayerEffect::ColorOverlay(OverlayParams::default()));
    s
}

/// Each pass is returned over **its own** reach, not the stack's.
///
/// Before the crop, every pass came back at the stack apron, so one long
/// drop shadow sized all five planes: a colour overlay that cannot write a
/// single pixel outside `A` was handed back inflated by 25 px on every
/// side. The compositor would then blend four mostly-empty planes at the
/// shadow's size.
///
/// Perturbation evidence: setting `crop: false` at `evaluate_in`'s call to
/// `render_all` reds the per-effect rect assertions (every rect comes back
/// as the stack apron). Changing `LayerEffect::reach_px` to return the
/// stack max — i.e. undoing the per-effect split — reds them the same way.
#[test]
fn pass_rects_are_per_effect_not_stack_wide() {
    let a = square(20);
    let s = mixed_reach_stack();
    let stack_reach = s.reach_px();
    assert_eq!(stack_reach, 25, "shadow 18 + 6, +1 AA");

    let passes = evaluate(&a, &s, GlobalLight::default());
    assert_eq!(passes.len(), 5);

    // order -> expected reach past `A`.
    let expected: &[(u8, i32)] = &[
        (1, 25), // drop shadow: distance 18 + size 6 + 1
        (2, 6),  // outer glow: size 5 + 1
        (6, 1),  // colour overlay: clipped to A, +1 AA band
        (8, 1),  // inner glow: clipped to A, +1 AA band
        (11, 3), // outside stroke: size 2 + 1
    ];
    let mut narrower_than_stack = 0;
    for (p, (order, reach)) in passes.iter().zip(expected.iter()) {
        assert_eq!(p.order, *order, "fixture/expectation desync");
        assert_eq!(
            p.coverage.rect,
            a.rect.inflate(*reach, *reach),
            "effect {order} came back at the wrong reach"
        );
        if *reach < stack_reach {
            narrower_than_stack += 1;
        }
    }
    // Guard the guard: the fixture must actually exercise the difference,
    // or every assertion above would also hold for a stack-wide crop.
    assert_eq!(
        narrower_than_stack, 4,
        "fixture does not distinguish per-effect reach from the stack apron"
    );

    // And the saving is real, not notional.
    let cropped: u64 = passes.iter().map(|p| p.coverage.rect.area()).sum();
    let stack_wide = a.rect.inflate(stack_reach, stack_reach).area() * 5;
    assert!(
        cropped * 2 < stack_wide,
        "crop should more than halve the retained coverage: {cropped} vs {stack_wide}"
    );
}

/// The crop is **lossless**: it discards only samples that are already
/// zero. This is what makes `LayerEffect::reach_px` load-bearing rather
/// than a hint — an under-promising reach would silently amputate coverage
/// instead of merely returning a bigger plane than needed.
///
/// Measured against the **uncropped** planes, because a cropped plane
/// cannot testify about what the crop removed.
///
/// # The working rect is a fixed constant, NOT `stack.reach_px()`
///
/// Sizing it from `reach_px` would make this test self-referential, and
/// the battery caught exactly that: with `rect = a.rect.inflate(reach)`,
/// shrinking `reach_px` shrinks the working rect by the same amount, so
/// the uncropped plane shrinks in step with the cropped one, the two stay
/// equal, and **no mass loss is observable**. Both the offset-term and
/// AA-band defects below survived that version. It is the same trap the
/// previous slice recorded on `evaluate_is_rect_independent_above_the_apron`
/// — a test whose fixture is computed from the quantity under test can
/// only ever measure self-consistency.
///
/// A constant wider than any effect's true reach breaks the loop: the
/// uncropped plane stays put while a wrong crop pulls in, and the
/// difference is real coverage.
///
/// Perturbation evidence: changing the drop-shadow arm of
/// `LayerEffect::reach_px` from `distance_px + size_px` to `size_px` alone
/// (dropping the offset term, the natural mistake) reds effect 1 with a
/// large mass loss. Making `field::crop` return zeros rather than copy reds
/// every effect.
///
/// **What it does NOT catch, measured rather than assumed:** shrinking the
/// uniform `+1` AA band to `+0` leaves this test GREEN, and so does
/// cropping a further 1 px inside the reach. Both were expected to red it
/// and do not, for a reason worth recording — at the outermost pixel of a
/// band or a truncated Gaussian the smoothstep has already reached
/// *exactly* 0, so the pixel the crop removes carries no mass and a
/// losslessness test cannot see it go. The AA band is still load-bearing
/// and still falsifiable, just elsewhere: `+1 → +0` reds
/// `reach_contains_every_pixel_the_stack_writes` (the drop shadow's
/// Gaussian tail does escape) and `pass_rects_are_per_effect_not_stack_wide`
/// (which pins the reaches exactly). Two tests hold that pixel up; this is
/// not one of them, and saying so is cheaper than a later reader trusting
/// it to be.
#[test]
fn cropping_a_pass_loses_no_coverage() {
    let a = square(20);
    let s = mixed_reach_stack();
    // Fixed, and deliberately wider than the fixture's true 25 px reach.
    const WORKING: i32 = 60;
    assert!(
        WORKING > s.reach_px(),
        "working rect must exceed the stack reach, or the crop cannot be seen to lose anything"
    );
    let rect = a.rect.inflate(WORKING, WORKING);

    let cropped = evaluate_in(&a, &s, GlobalLight::default(), rect);
    let whole = evaluate_in_uncropped(&a, &s, GlobalLight::default(), rect);
    assert_eq!(cropped.len(), 5);
    assert_eq!(whole.len(), 5);

    let mut shrank = 0;
    for (c, w) in cropped.iter().zip(whole.iter()) {
        assert_eq!(c.order, w.order);
        assert_eq!(w.coverage.rect, rect, "uncropped pass should span the rect");
        // Non-vacuity: the effect must have drawn something, or "no mass
        // was lost" is a statement about a pair of empty planes.
        assert!(
            mass(&w.coverage) > 0.0,
            "effect {} drew nothing; the comparison would be vacuous",
            w.order
        );
        assert!(
            (mass(&c.coverage) - mass(&w.coverage)).abs() < 1e-4,
            "effect {} lost {} coverage to the crop",
            c.order,
            mass(&w.coverage) - mass(&c.coverage)
        );
        // Values, not just totals: an offsetting pair of errors would keep
        // the sum and still be wrong.
        for y in c.coverage.rect.min.y..c.coverage.rect.max.y {
            for x in c.coverage.rect.min.x..c.coverage.rect.max.x {
                assert_eq!(
                    c.coverage.get(x, y),
                    w.coverage.get(x, y),
                    "effect {} value changed at ({x}, {y}) — a crop must copy, not recompute",
                    c.order
                );
            }
        }
        if c.coverage.rect != w.coverage.rect {
            shrank += 1;
        }
    }
    assert_eq!(
        shrank, 5,
        "every pass should shrink below the {WORKING}px working rect"
    );
}

/// The [`Slot`] a pass carries is a promise about **where its coverage
/// can be**, and every effect in the MVP five has to keep it — not just
/// the ones with a bespoke test.
///
/// * `Under` (drop shadow, outer glow) composites beneath the layer's own
///   pixels, and both are knocked out by `·(1 − A)`, so neither may write
///   under opaque coverage. A pass that did would darken the layer itself.
/// * `Interior` (colour overlay, inner glow) is clipped to `A`, so neither
///   may write outside it.
/// * `Above` (stroke) straddles the contour deliberately and is
///   unconstrained here.
///
/// The per-effect tests each pin one of these; this pins the *class*, so a
/// sixth effect added at a slot inherits the obligation rather than needing
/// someone to remember it.
///
/// Perturbation evidence: deleting `mul_inverse_in_place` from
/// `drop_shadow` reds the `Under` arm (effect 1 writes 1.0 under opaque
/// coverage); deleting `mul_in_place` from `inner_glow` reds the
/// `Interior` arm; changing `color_overlay`'s slot to `Slot::Under` reds
/// the `Under` arm too, since `A` is exactly what it must not write under.
#[test]
fn slot_promises_hold_for_every_effect() {
    let a = square(20);
    let s = mixed_reach_stack();
    let passes = evaluate(&a, &s, GlobalLight::default());
    assert_eq!(passes.len(), 5);

    // The interior probe must actually be inside `A`, or the `Under` arm's
    // knockout assertion is about a pixel with nothing to be knocked out
    // of. (There is deliberately no matching "exterior probe" assertion:
    // `MaskPlane::get` returns the §2.4 absent-read 0 outside its rect, so
    // asserting `a.get(-24, 0) == 0.0` would hold for every conceivable
    // fixture and prove nothing. The `Interior` arm sweeps the whole
    // exterior with `mass_where` instead.)
    assert_eq!(a.get(0, 0), 1.0, "interior probe is not inside A");

    let mut seen = [0u32; 3];
    for p in &passes {
        assert!(
            p.coverage.v.iter().all(|v| (0.0..=1.0).contains(v)),
            "effect {} produced coverage outside [0, 1]",
            p.order
        );
        match p.slot {
            Slot::Under => {
                seen[0] += 1;
                assert!(
                    p.coverage.get(0, 0) < 1e-6,
                    "Under effect {} wrote {} beneath opaque coverage",
                    p.order,
                    p.coverage.get(0, 0)
                );
                // ...and it is not simply an empty plane.
                assert!(
                    mass(&p.coverage) > 0.0,
                    "Under effect {} drew nothing; its knockout claim is vacuous",
                    p.order
                );
            }
            Slot::Interior => {
                seen[1] += 1;
                let outside = mass_where(&p.coverage, |x, y| {
                    !(-20..20).contains(&x) || !(-20..20).contains(&y)
                });
                assert_eq!(
                    outside, 0.0,
                    "Interior effect {} wrote {outside} outside A",
                    p.order
                );
                assert!(
                    mass(&p.coverage) > 0.0,
                    "Interior effect {} drew nothing; its clip claim is vacuous",
                    p.order
                );
            }
            Slot::Above => seen[2] += 1,
        }
    }
    // Guard the guard: all three slots must be represented, or the match
    // arms above are being asserted about an empty set.
    assert_eq!(seen, [2, 2, 1], "fixture does not cover all three slots");
}

/// The stack is *stored* in §11.3 seat order, so `iter` and `evaluate`
/// cannot disagree about what "the order" means.
///
/// `evaluate` sorts, so this is not what keeps rendering correct — it is
/// what keeps a second, insertion-shaped order from existing at all for a
/// later reader to pick up by mistake.
///
/// Perturbation evidence: reverting `set`'s `insert(at, e)` to `push(e)`
/// reds this — the scrambled insertion order comes straight back out.
#[test]
fn stack_iterates_in_seat_order() {
    let mut s = EffectStack::new();
    s.set(LayerEffect::Stroke(StrokeParams::default()));
    s.set(LayerEffect::ColorOverlay(OverlayParams::default()));
    s.set(LayerEffect::DropShadow(ShadowParams::default()));
    s.set(LayerEffect::InnerGlow(GlowParams::default()));
    s.set(LayerEffect::OuterGlow(GlowParams::default()));

    let stored: Vec<u8> = s.iter().map(|e| e.order()).collect();
    assert_eq!(stored, vec![1, 2, 6, 8, 11]);

    // Replacement keeps the seat rather than moving the effect to the end.
    s.set(LayerEffect::DropShadow(ShadowParams {
        distance_px: 99.0,
        ..Default::default()
    }));
    let stored: Vec<u8> = s.iter().map(|e| e.order()).collect();
    assert_eq!(stored, vec![1, 2, 6, 8, 11], "replacement moved a seat");
}

/// `EffectStack::reach_px` is the max of the per-effect reaches, and an
/// empty stack reaches 0.
///
/// The empty case is the one worth pinning: when the reach table lived
/// inline in `EffectStack::reach_px` it folded from a `0.0` seed and then
/// added the AA band unconditionally, so an empty stack claimed a 1 px
/// apron — an antialiasing band around an edge that does not exist.
///
/// Perturbation evidence: restoring the unconditional `+1` (e.g.
/// `.unwrap_or(0)` → `.unwrap_or(1)`) reds the empty assertion; changing
/// the reduction from `max` to `min` reds the max assertion.
#[test]
fn stack_reach_is_the_max_of_its_effects() {
    assert_eq!(EffectStack::new().reach_px(), 0, "no effect, no apron");

    let s = mixed_reach_stack();
    let per_effect: Vec<i32> = s.iter().map(|e| e.reach_px()).collect();
    assert_eq!(per_effect, vec![25, 6, 1, 1, 3]);
    assert_eq!(s.reach_px(), 25);
    assert_eq!(s.reach_px(), *per_effect.iter().max().unwrap());

    // A negative pixel parameter must not *deflate* the apron. There is no
    // validation door on this side of the wall, so this is reachable, and
    // an apron that shrank below zero would turn `evaluate`'s working rect
    // into a crop — the value-changing narrow-rect defect, arrived at from
    // the other direction.
    let neg = stack_of(LayerEffect::DropShadow(ShadowParams {
        distance_px: -50.0,
        size_px: 0.0,
        ..Default::default()
    }));
    assert!(
        neg.reach_px() >= 0,
        "negative distance deflated the apron to {}",
        neg.reach_px()
    );
}

/// A disabled effect contributes **no pass and no apron**.
///
/// `enabled` is frozen in `document.v1.json` on all ten `fx*` definitions
/// with `default: true`, and until this slice the engine had nowhere to put
/// it — a persisted `enabled: false` would have rendered exactly like
/// `true`. The apron half matters as much as the pass half: a disabled
/// 30 000 px drop shadow that still sized the working rect would cost the
/// full ~14.6 GB to draw nothing at all.
///
/// Perturbation evidence: reverting `EffectStack::reach_px` to iterate
/// `self.effects` instead of `self.enabled()` reds the apron assertion
/// (reach stays 25) while leaving the pass assertions green — which is why
/// they are separate assertions. Reverting `render_all` to `stack.iter()`
/// reds the pass count.
///
/// **What does NOT red, measured:** changing `evaluate`'s early-out from
/// `draws_nothing()` back to `is_empty()`. An earlier version of this
/// comment claimed it would. It does not, and the reason is structural:
/// `render_all` filters by `enabled()` regardless, so an all-disabled
/// stack returns no passes either way. That guard is therefore a pure
/// **optimisation** — it skips building a `Field` and running an EDT for a
/// stack that will draw nothing — and no assertion here holds it up.
/// Recorded rather than removed: the saving is real, and a reader who
/// believed a test was defending it would be wrong.
#[test]
fn a_disabled_effect_draws_nothing_and_costs_no_apron() {
    let a = square(20);
    let mut s = mixed_reach_stack();
    assert_eq!(s.reach_px(), 25);
    assert_eq!(evaluate(&a, &s, GlobalLight::default()).len(), 5);

    // Disable only the effect that dominates the apron.
    s.set(LayerEffect::DropShadow(ShadowParams {
        enabled: false,
        angle_deg: 0.0,
        distance_px: 18.0,
        size_px: 6.0,
        use_global_light: false,
        ..Default::default()
    }));
    assert_eq!(s.len(), 5, "disabling must not remove the effect");
    assert!(!s.is_empty(), "the stack still holds it");

    let passes = evaluate(&a, &s, GlobalLight::default());
    assert_eq!(passes.len(), 4, "the disabled effect still produced a pass");
    assert!(
        !passes.iter().any(|p| p.order == 1),
        "the drop shadow seat is still being rendered"
    );
    assert_eq!(
        s.reach_px(),
        6,
        "a disabled effect must not size the working rect"
    );

    // Disable every effect: the stack is not empty, but it draws nothing.
    let all_off = {
        let mut t = EffectStack::new();
        t.set(LayerEffect::ColorOverlay(OverlayParams {
            enabled: false,
            ..Default::default()
        }));
        t
    };
    assert!(
        !all_off.is_empty(),
        "is_empty asks whether it HOLDS anything"
    );
    assert!(
        all_off.draws_nothing(),
        "...draws_nothing asks whether it RENDERS"
    );
    assert!(evaluate(&a, &all_off, GlobalLight::default()).is_empty());
    assert_eq!(all_off.reach_px(), 0);
}

/// The Rust structs are checked against the **frozen** wire shapes in
/// `contracts/schemas/document.v1.json`, which already defines all ten
/// `fx*` effects even though no *command* exists to edit one.
///
/// This test pins the defaults that agree, so that the ones that do **not**
/// agree — recorded in `params.rs` and fed to PW-1 — cannot be quietly
/// "fixed" in either direction without someone noticing. Where the §11.3
/// table and the schema disagree, the engine follows §11.3 and the
/// disagreement is a fold-in determination, not a bug to patch here.
///
/// Perturbation evidence: changing `ShadowParams::default`'s `opacity` to
/// 0.5 reds the shadow arm; setting any `enabled` default to `false` reds
/// the enabled arm.
#[test]
fn defaults_match_the_frozen_document_schema_where_they_agree() {
    // Agreed with document.v1.json:
    let sh = ShadowParams::default();
    assert!(sh.enabled, "fxDropShadow.enabled default is true");
    assert_eq!(sh.opacity, 0.35, "fxDropShadow.opacity default");
    assert_eq!(sh.angle_deg, 120.0, "fxDropShadow.angleDeg default");
    assert!(sh.use_global_light, "fxDropShadow.useGlobalLight default");
    assert_eq!(sh.spread, 0.0, "fxDropShadow.spread default");
    assert_eq!(sh.blend, BlendMode::Multiply, "fxDropShadow.blend default");

    let g = GlowParams::default();
    assert!(g.enabled);
    assert_eq!(g.technique, GlowTechnique::Softer, "fx*Glow.technique");
    assert_eq!(g.source, GlowSource::Edge, "fxInnerGlow.source");
    assert_eq!(g.range, 0.5, "fx*Glow.range default");
    assert_eq!(g.blend, BlendMode::Screen, "fx*Glow.blend default");

    let st = StrokeParams::default();
    assert!(st.enabled);
    assert_eq!(st.position, StrokeAlign::Outside, "fxStroke.position");
    assert_eq!(st.opacity, 1.0, "fxStroke.opacity default");
    assert_eq!(st.blend, BlendMode::Normal, "fxStroke.blend default");

    let ov = OverlayParams::default();
    assert!(ov.enabled);
    assert_eq!(ov.opacity, 1.0, "fxColorOverlay.opacity default");
    assert_eq!(ov.blend, BlendMode::Normal, "fxColorOverlay.blend default");

    // KNOWN DISAGREEMENT, asserted so it cannot drift unnoticed: the
    // schema gives the glows `"opacity": {"default": 0.5}`; §11.3's
    // parameter table gives 0.35. The engine follows §11.3. If the fold-in
    // rules for the schema, this line is where it lands.
    assert_eq!(
        g.opacity, 0.35,
        "glow opacity follows §11.3 (0.35), NOT document.v1.json (0.5) — \
         see the disagreement recorded in params.rs"
    );
}

/// An empty stack is not an error and costs nothing.
#[test]
fn empty_stack_produces_no_passes() {
    let a = square(8);
    let s = EffectStack::new();
    assert!(evaluate(&a, &s, GlobalLight::default()).is_empty());
}

/// §11.3: "All blurs are the separable Gaussian of §16.3 with `σ =
/// radius/3`". The ratio has one home; this pins it.
///
/// Perturbation evidence: changing `sigma_for_radius` to `/ 2.0` (the
/// §13.2 *feather* convention, the most plausible wrong neighbour — and
/// exactly the confusion FI-10 was opened about) reds this.
#[test]
fn effect_blur_sigma_is_radius_over_three() {
    assert_eq!(field::sigma_for_radius(9.0), 3.0);
    assert_eq!(field::sigma_for_radius(0.0), 0.0);
    // Never negative, whatever a caller passes.
    assert_eq!(field::sigma_for_radius(-5.0), 0.0);
}

/// The pass hand-off carries blend/opacity for the compositor to apply and
/// does **not** pre-apply them — the effects stack never blends, so a
/// pass's coverage must be independent of its blend mode.
///
/// Perturbation evidence: multiplying `coverage` by `opacity` inside
/// `color_overlay` reds the second assertion.
#[test]
fn passes_carry_blend_state_without_applying_it() {
    let a = square(10);
    let mk = |blend: BlendMode, opacity: f32| {
        let s = stack_of(LayerEffect::ColorOverlay(OverlayParams {
            enabled: true,
            color: [0.2, 0.4, 0.6],
            blend,
            opacity,
        }));
        only_pass(&s, &a, GlobalLight::default())
    };

    let p = mk(BlendMode::Multiply, 0.25);
    assert_eq!(p.blend, BlendMode::Multiply);
    assert_eq!(p.opacity, 0.25);
    assert_eq!(p.color, [0.2, 0.4, 0.6]);
    // Coverage is pre-opacity: identical whatever the blend state says.
    let q = mk(BlendMode::Screen, 1.0);
    assert_eq!(
        p.coverage.v, q.coverage.v,
        "coverage must not depend on blend/opacity"
    );
}

// --------------------------------------------------------------------
// Working-rect contract
// --------------------------------------------------------------------

/// The apron promise of [`EffectStack::reach_px`] has a second half that
/// [`reach_contains_every_pixel_the_stack_writes`] does not test: not only
/// must no coverage escape the apron, but **evaluating over the apron must
/// give the same answer as evaluating over any larger rect**. Otherwise
/// `evaluate`'s own default rect would be a lucky choice rather than a
/// sufficient one, and the compositor's ROI could not be sized from
/// `reach_px` at all.
///
/// All five effects, together, so a per-effect regression cannot hide
/// behind the others.
///
/// Perturbation evidence: making [`EffectStack::reach_px`] under-promise by
/// **7 px** (`saturating_add(1)` → `saturating_sub(6)`) reds this with a max
/// delta of 1.6e-1. Both the test's rect and the `debug_assert` in
/// [`evaluate_in`] read the same perturbed `reach_px`, so the guard stays
/// self-consistent and the value comparison is what fires.
///
/// The 7 px figure is **measured, not chosen**, and the measurement is
/// worth recording: on this fixture the apron carries ≈6 px of genuine
/// slack — cuts of 1 through 6 px leave the overlap *bit-identical*
/// (max delta exactly 0), and only the 7th px reaches values that are not
/// already zero. An earlier version of this comment asserted a 2 px cut
/// would red it; the mechanical battery showed that it does not.
///
/// That slack is also why this test is **not** the sharper of the pair:
/// [`reach_contains_every_pixel_the_stack_writes`] reds on a 2 px
/// under-promise, six pixels earlier. The two are kept because they pin
/// different guarantees, not different sensitivities — that one says *no
/// coverage escapes the apron*, this one says *the apron is where the
/// values have converged*, so `reach_px` cannot be merely a box big enough
/// to hold the mass. Together with
/// [`evaluate_in_rejects_a_rect_narrower_than_the_apron`] they bracket the
/// working-rect contract from both sides.
#[test]
fn evaluate_is_rect_independent_above_the_apron() {
    let a = square(20);
    let mut s = EffectStack::new();
    s.set(LayerEffect::DropShadow(ShadowParams {
        angle_deg: 0.0,
        distance_px: 6.0,
        size_px: 7.0,
        spread: 0.25,
        use_global_light: false,
        ..Default::default()
    }));
    s.set(LayerEffect::OuterGlow(GlowParams {
        size_px: 6.0,
        technique: GlowTechnique::Softer,
        ..Default::default()
    }));
    s.set(LayerEffect::InnerGlow(GlowParams {
        size_px: 6.0,
        source: GlowSource::Center,
        ..Default::default()
    }));
    s.set(LayerEffect::Stroke(StrokeParams::default()));
    s.set(LayerEffect::ColorOverlay(OverlayParams::default()));

    let reach = s.reach_px();
    let snug = evaluate_in(&a, &s, GlobalLight::default(), a.rect.inflate(reach, reach));
    let wide = evaluate_in(
        &a,
        &s,
        GlobalLight::default(),
        a.rect.inflate(reach + 37, reach + 37),
    );
    assert_eq!(snug.len(), 5);
    assert_eq!(wide.len(), 5);

    for (sp, wp) in snug.iter().zip(wide.iter()) {
        assert_eq!(sp.order, wp.order);
        let mut worst = 0.0f32;
        for y in sp.coverage.rect.min.y..sp.coverage.rect.max.y {
            for x in sp.coverage.rect.min.x..sp.coverage.rect.max.x {
                worst = worst.max((sp.coverage.get(x, y) - wp.coverage.get(x, y)).abs());
            }
        }
        assert!(
            worst < 1e-6,
            "effect {} is rect-dependent above its apron: max delta {worst}",
            sp.order
        );
    }
}

/// ...and the precondition that makes the above true is *enforced*, not
/// merely documented. A rect narrower than the apron trips the
/// `debug_assert` in [`evaluate_in`] rather than silently returning the
/// seam-producing values measured in that function's docs.
///
/// Perturbation evidence: deleting the `debug_assert` makes this test fail
/// with "test did not panic as expected" — the guard is the only thing
/// this test can be passing on.
#[test]
#[should_panic(expected = "narrower than")]
fn evaluate_in_rejects_a_rect_narrower_than_the_apron() {
    let a = square(20);
    let s = stack_of(LayerEffect::OuterGlow(GlowParams {
        size_px: 6.0,
        ..Default::default()
    }));
    // A tile cut through the middle of the layer — the shape a naive
    // tiling compositor would hand in.
    let _ = evaluate_in(&a, &s, GlobalLight::default(), IRect::new(-26, -4, -14, 4));
}

/// Degenerate inputs must not produce a NaN, an infinity or a panic.
///
/// Nothing downstream of a coverage plane checks for a NaN — it survives
/// every clamp (`f32::clamp` propagates it), so a single one would flow
/// straight through the compositor into an exported pixel. Behind a wall
/// there is no fuzz corpus and no conformance probe to catch that, so the
/// three degenerate cases the parameter ranges actually admit are pinned
/// here: an all-zero coverage (a fully masked-out layer), a zero-area
/// layer rect, and `size_px = 0` (below §11.3's `1..=250` stroke range, but
/// reachable for the glows whose range starts at 0).
///
/// Perturbation evidence: dropping the `[0, 1]` clamp from [`field::ramp`]
/// (`1.0 - t`, the "inputs are obviously in range" simplification) reds
/// this — with `size_px = 0` the ramp argument is ±∞ and an unclamped ramp
/// passes that straight through to the pass coverage.
///
/// That is the **measured** falsifier, and it is not the one this test was
/// first written against. The battery was run against the two
/// `.max(f32::EPSILON)` divisor guards in [`inner_glow`] instead, and
/// **neither reds it** — they are provably inert, for two independent
/// reasons worth writing down rather than rediscovering:
/// * `size_px = 0` makes the quotient ±∞, never NaN, and `ramp`'s clamp
///   maps ±∞ to `0`/`1`. The clamp, not the guard, is what keeps the output
///   finite.
/// * NaN would need `0.0 / 0.0`, i.e. a distance of exactly zero — and
///   §13.2's ±(D − 0.5) lattice pin means `d` never *is* exactly zero
///   (counted: 0 of 2704 samples on the 40 px fixture).
///
/// The guards are kept anyway, as the cheaper of two ways to be right: they
/// keep this module from *depending* on IEEE infinity semantics surviving a
/// downstream clamp. But they are belt-and-braces, and this comment is the
/// honest record that no test holds them up.
#[test]
fn degenerate_inputs_stay_finite() {
    let mut all = EffectStack::new();
    all.set(LayerEffect::DropShadow(ShadowParams::default()));
    all.set(LayerEffect::OuterGlow(GlowParams::default()));
    all.set(LayerEffect::InnerGlow(GlowParams {
        source: GlowSource::Center,
        ..Default::default()
    }));
    all.set(LayerEffect::Stroke(StrokeParams::default()));
    all.set(LayerEffect::ColorOverlay(OverlayParams::default()));

    let finite = |passes: Vec<EffectPass>, case: &str| {
        assert_eq!(passes.len(), 5, "{case}: expected all five passes");
        for p in passes {
            assert!(
                p.coverage.v.iter().all(|v| v.is_finite()),
                "{case}: effect {} produced a non-finite sample",
                p.order
            );
        }
    };

    // A fully masked-out layer: no coverage anywhere, so no effect has an
    // edge to work from and every pass must be empty rather than NaN.
    let zero = MaskPlane::new(IRect::new(-10, -10, 10, 10));
    let passes = evaluate(&zero, &all, GlobalLight::default());
    for p in &passes {
        assert_eq!(
            mass(&p.coverage),
            0.0,
            "effect {} manufactured coverage from an empty layer",
            p.order
        );
    }
    finite(passes, "all-zero coverage");

    // A zero-area layer rect.
    finite(
        evaluate(
            &MaskPlane::new(IRect::new(0, 0, 0, 0)),
            &all,
            GlobalLight::default(),
        ),
        "zero-area rect",
    );

    // size_px = 0 — the divisor in both glow ramps.
    let a = square(8);
    let mut z = EffectStack::new();
    z.set(LayerEffect::DropShadow(ShadowParams {
        size_px: 0.0,
        ..Default::default()
    }));
    z.set(LayerEffect::OuterGlow(GlowParams {
        size_px: 0.0,
        ..Default::default()
    }));
    z.set(LayerEffect::InnerGlow(GlowParams {
        size_px: 0.0,
        source: GlowSource::Edge,
        ..Default::default()
    }));
    z.set(LayerEffect::Stroke(StrokeParams {
        size_px: 0.0,
        ..Default::default()
    }));
    z.set(LayerEffect::ColorOverlay(OverlayParams::default()));
    finite(evaluate(&a, &z, GlobalLight::default()), "size_px = 0");
}

/// The allocation hazard, pinned as arithmetic rather than left in a doc
/// comment — and pinned **without allocating anything**, since the whole
/// point is that the plane this implies cannot be allocated.
///
/// §11.3 types shadow `distance` as `0..=30_000` px and `size` as
/// `0..=250`, so the largest *legal* drop shadow reaches 30_251 px, and
/// `evaluate` inflates the layer rect by it on all four sides. For a 1×1
/// layer that is a 60_503² working plane — **~14.6 GB** of `f32` — from a
/// document that violates no stated range.
///
/// This test exists to make a silent cap impossible to add by accident.
/// Nothing on this side of the wall may clamp effect geometry: there is no
/// validation door here, and a cap invented in the engine would make it
/// disagree with the (unwritten) capability rows. So if someone later
/// clamps `reach_px`, this reds and sends them to the rows instead. The
/// ceiling has to arrive as a row plus a §4 ROI intersection against the
/// canvas — a *compositor* obligation.
///
/// **The second half is the newer finding.** Out-of-range values do not
/// merely allocate too much, they abort *before* allocation: measured,
/// `distance_px = f32::MAX` (and `INFINITY`) saturate `reach_px` to
/// `i32::MAX`, and `IRect::inflate` then panics on i32 overflow in
/// `appname-core::geom` — an abort, not a typed refusal, and one that
/// happens in the ROI arithmetic that §4's `propagate_roi` also performs.
/// A ceiling therefore has to be enforced *before* the reach arithmetic,
/// not merely before the allocation. (`NAN` is the benign case: `f32::max`
/// returns the non-NaN operand, so it collapses to a 1 px reach.)
///
/// Perturbation evidence: adding any clamp to `LayerEffect::reach_px` —
/// e.g. `.min(4096)` — reds the reach assertion.
#[test]
fn legal_parameters_imply_an_unbounded_working_rect() {
    let s = stack_of(LayerEffect::DropShadow(ShadowParams {
        distance_px: 30_000.0, // §11.3 maximum
        size_px: 250.0,        // §11.3 maximum
        ..Default::default()
    }));
    assert_eq!(
        s.reach_px(),
        30_251,
        "the largest legal drop shadow's reach is not being reported honestly"
    );

    // A 1x1 layer, inflated by that reach on all four sides. Rect
    // arithmetic only — no plane is built.
    let tiny = IRect::new(0, 0, 1, 1);
    let reach = s.reach_px();
    let working = tiny.inflate(reach, reach);
    assert_eq!(working.width(), 60_503);
    let bytes = working.area() * 4;
    assert!(
        bytes > 14_000_000_000,
        "the hazard is understated: {bytes} bytes"
    );
}
