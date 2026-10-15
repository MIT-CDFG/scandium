// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! §20.2 SHAPE-LAYER stroke battery (`layer.stroke`: cap · join ·
//! miterLimit · align) — the knobs the §40.5 `stroke_path` command wire
//! does not carry, so the distance band cannot serve them.
//!
//! Every mass here is a CLOSED FORM, not a recorded output:
//!
//! * a MITER-joined rect band is exact rectangle arithmetic — the outer
//!   hull is the inflated rect, the inner hull the deflated one, both with
//!   SHARP corners;
//! * a BEVEL band cuts each right-angle corner kite (area h²) in half;
//! * a ROUND band replaces that kite with a quarter disc (π·h²/4), which
//!   is ALSO what the independent distance-field machine computes — the
//!   two are cross-checked against each other here;
//! * caps on an open line are exact: butt = L·w, square = (L + w)·w,
//!   round = L·w + π·(w/2)².

use appname_core::engine::vector_ops::{
    LineCap, LineJoin, ShapeStrokeStyle, StrokePosition, STROKE_DEFAULT_MITER_LIMIT,
};
use appname_core::PathId;
use appname_vector::geom::Vec2;
use appname_vector::path::{CubicSeg, FillRule, SubPath, VectorPath};
use appname_vector::stroke::{shape_stroke_coverage, stroke_coverage};

fn rect_path(x0: f64, y0: f64, x1: f64, y1: f64) -> VectorPath {
    let mut p = VectorPath::new(PathId::nil(), FillRule::NonZero);
    let (a, b, c, d) = (
        Vec2::new(x0, y0),
        Vec2::new(x1, y0),
        Vec2::new(x1, y1),
        Vec2::new(x0, y1),
    );
    p.subpaths.push(SubPath {
        segments: vec![
            CubicSeg::line(a, b),
            CubicSeg::line(b, c),
            CubicSeg::line(c, d),
            CubicSeg::line(d, a),
        ],
        closed: true,
    });
    p
}

fn open_hline(x0: f64, x1: f64, y: f64) -> VectorPath {
    let mut p = VectorPath::new(PathId::nil(), FillRule::NonZero);
    p.subpaths.push(SubPath {
        segments: vec![CubicSeg::line(Vec2::new(x0, y), Vec2::new(x1, y))],
        closed: false,
    });
    p
}

fn polyline(pts: &[(f64, f64)], closed: bool) -> VectorPath {
    let mut p = VectorPath::new(PathId::nil(), FillRule::NonZero);
    let v: Vec<Vec2> = pts.iter().map(|&(x, y)| Vec2::new(x, y)).collect();
    let n = v.len();
    let last = if closed { n } else { n - 1 };
    p.subpaths.push(SubPath {
        segments: (0..last)
            .map(|i| CubicSeg::line(v[i], v[(i + 1) % n]))
            .collect(),
        closed,
    });
    p
}

fn style(width: f64, cap: LineCap, join: LineJoin, align: StrokePosition) -> ShapeStrokeStyle {
    ShapeStrokeStyle {
        width,
        cap,
        join,
        align,
    }
}

fn miter() -> LineJoin {
    LineJoin::Miter(STROKE_DEFAULT_MITER_LIMIT)
}

/// §20.2 rules 2/3 polygonize round arcs at ≤ 12°, i.e. into
/// `ceil(360/12) = 30` INSCRIBED sides. The area of that regular 30-gon is
/// the EXACT oracle for every round piece — not π·r², which it approaches
/// from below by `π·r² − 15·r²·sin 12° ≈ 0.0229·r²`.
const ARC_SIDES: f64 = 30.0;

fn poly_disc_area(r: f64) -> f64 {
    0.5 * ARC_SIDES * r * r * (std::f64::consts::TAU / ARC_SIDES).sin()
}

#[track_caller]
fn mass_is(got: f64, want: f64, tol: f64, what: &str) {
    assert!(
        (got - want).abs() < tol,
        "{what}: mass {got} vs closed form {want} (tol {tol})"
    );
}

// ------------------------------------------------------------------
// Joins — the three §20.2 rule-2 arms on the SAME right-angle corner
// ------------------------------------------------------------------

/// Rect 10..90 × 10..50 (80 × 40), width 2 (h = 1), centered.
///
/// * miter: outer 82 × 42 minus inner 78 × 38 = 3444 − 2964 = **480**;
/// * bevel: each of the 4 right-angle corners loses HALF its kite
///   (kite area = h² = 1) ⇒ 480 − 4·0.5 = **478**;
/// * round: each kite (1) becomes a quarter disc (π/4) ⇒
///   480 − 4 + π = **479.1416**.
#[test]
fn the_three_joins_differ_by_exactly_their_corner_geometry() {
    let p = rect_path(10.0, 10.0, 90.0, 50.0);
    let m = shape_stroke_coverage(
        &p,
        style(2.0, LineCap::Butt, miter(), StrokePosition::Center),
    );
    let b = shape_stroke_coverage(
        &p,
        style(2.0, LineCap::Butt, LineJoin::Bevel, StrokePosition::Center),
    );
    let r = shape_stroke_coverage(
        &p,
        style(2.0, LineCap::Butt, LineJoin::Round, StrokePosition::Center),
    );
    mass_is(m.total(), 480.0, 0.05, "miter ring");
    mass_is(b.total(), 478.0, 0.05, "bevel ring");
    // The four right-angle corners' arcs sum to ONE disc, polygonized.
    mass_is(
        r.total(),
        480.0 - 4.0 + poly_disc_area(1.0),
        0.05,
        "round ring",
    );
    // The three are genuinely distinct pixel answers, not one arm
    // spelled three ways.
    assert!(m.total() > r.total() && r.total() > b.total());
}

/// The ROUND arm of the new piece-union machine and the INDEPENDENT
/// distance-field machine (`stroke_path`'s band) must agree: both are the
/// Minkowski disc sweep, one exactly and one at the §20.2 ≤ 12°
/// polygonization. This is the cross-check that keeps the second machine
/// honest — a bug in the union construction cannot hide behind its own
/// oracle.
#[test]
fn round_join_union_agrees_with_the_distance_field_band() {
    for (w, r) in [
        (2.0, (10.0, 10.0, 90.0, 50.0)),
        (6.0, (20.0, 20.0, 60.0, 44.0)),
    ] {
        let p = rect_path(r.0, r.1, r.2, r.3);
        let union = shape_stroke_coverage(
            &p,
            style(w, LineCap::Round, LineJoin::Round, StrokePosition::Center),
        );
        let field = stroke_coverage(&p, w, StrokePosition::Center).unwrap();
        // The field pads its plane by half_width + 1; the union's rect is
        // the exact vertex hull. Containment is the honest relation.
        assert!(
            field.rect.contains_rect(&union.rect),
            "union band {:?} inside the field plane {:?}",
            union.rect,
            field.rect
        );
        // The ONLY difference is the inscribed polygonization of the four
        // corner arcs, which sum to one disc of radius w/2.
        let deficit = std::f64::consts::PI * (w / 2.0).powi(2) - poly_disc_area(w / 2.0);
        mass_is(
            union.total(),
            field.total(),
            deficit + 0.05,
            "round union vs distance field",
        );
        assert!(
            union.total() < field.total(),
            "inscribed polygonization under-covers, never over"
        );
    }
}

/// §20.2 rule 2's SVG fallback: past `miter_length / width > limit` the
/// join is a bevel. A 20° V is ratio 1/sin 10° ≈ 5.76 — beyond the
/// contract-silent default of 4, inside a hand-raised limit of 8.
#[test]
fn the_miter_limit_falls_back_to_bevel_exactly_at_the_spec_ratio() {
    // Apex at (50, 60); two arms 40 long, ±10° off vertical ⇒ interior
    // angle 20°.
    let t = 10.0_f64.to_radians();
    let arm = 40.0;
    let v = polyline(
        &[
            (50.0 - arm * t.sin(), 60.0 - arm * t.cos()),
            (50.0, 60.0),
            (50.0 + arm * t.sin(), 60.0 - arm * t.cos()),
        ],
        false,
    );
    let st = |j| style(4.0, LineCap::Butt, j, StrokePosition::Center);
    let bevel = shape_stroke_coverage(&v, st(LineJoin::Bevel)).total();
    let limit_4 = shape_stroke_coverage(&v, st(LineJoin::Miter(4.0))).total();
    let limit_8 = shape_stroke_coverage(&v, st(LineJoin::Miter(8.0))).total();
    mass_is(
        limit_4,
        bevel,
        1e-9,
        "ratio 5.76 > limit 4 ⇒ bevel, bit-identical",
    );
    assert!(
        limit_8 > bevel + 1.0,
        "ratio 5.76 < limit 8 ⇒ the spike is drawn: {limit_8} vs bevel {bevel}"
    );
    // The drawn spike reaches h/sin(θ/2) = 2/sin(10°) ≈ 11.52 past the
    // apex, so the band's rect must extend above it.
    let spiked = shape_stroke_coverage(&v, st(LineJoin::Miter(8.0)));
    assert!(
        f64::from(spiked.rect.max.y) >= 60.0 + 11.0,
        "miter spike extends the band rect: {:?}",
        spiked.rect
    );
}

// ------------------------------------------------------------------
// Caps — the three §20.2 rule-3 arms on the SAME open line
// ------------------------------------------------------------------

/// Line y = 30, x ∈ [10, 90] (L = 80), width 4 (h = 2).
/// butt = 320 · square = (80 + 4)·4 = 336 · round = 320 + π·2².
#[test]
fn the_three_caps_differ_by_exactly_their_end_geometry() {
    let p = open_hline(10.0, 90.0, 30.0);
    let st = |c| style(4.0, c, miter(), StrokePosition::Center);
    mass_is(
        shape_stroke_coverage(&p, st(LineCap::Butt)).total(),
        320.0,
        0.05,
        "butt",
    );
    mass_is(
        shape_stroke_coverage(&p, st(LineCap::Square)).total(),
        336.0,
        0.05,
        "square",
    );
    // The two round caps are two HALVES of the 30-gon (which is mirror-
    // symmetric about the cap chord), i.e. exactly one polygonized disc.
    mass_is(
        shape_stroke_coverage(&p, st(LineCap::Round)).total(),
        320.0 + poly_disc_area(2.0),
        0.05,
        "round",
    );
}

/// Caps exist only on OPEN ends: a closed contour's cap knob is inert
/// (every vertex is a join). Pinned so a future dash slice — which mints
/// open runs inside closed contours — cannot silently change closed-shape
/// pixels.
#[test]
fn a_closed_contour_ignores_the_cap_knob() {
    let p = rect_path(10.0, 10.0, 90.0, 50.0);
    let base = shape_stroke_coverage(
        &p,
        style(6.0, LineCap::Butt, miter(), StrokePosition::Center),
    );
    for cap in [LineCap::Round, LineCap::Square] {
        let other = shape_stroke_coverage(&p, style(6.0, cap, miter(), StrokePosition::Center));
        assert_eq!(base, other, "closed contour is cap-invariant ({cap:?})");
    }
}

// ------------------------------------------------------------------
// Align — §20.2 rule 5
// ------------------------------------------------------------------

/// Rect 80 × 40, width 2, MITER. The rule builds the 2×-width centered
/// band and gates it by the source's own fill:
/// * inside = 3200 − 76 × 36 = **464** (inner corners are sharp either
///   way — the two quads' overlap IS the corner);
/// * outside = 84 × 44 − 3200 = **496** (outer corners are the miter
///   apexes, i.e. SHARP — this is exactly where a miter band diverges
///   from the round distance band's 496 − 16 + 4π).
#[test]
fn align_inside_and_outside_split_the_double_band_with_miter_corners() {
    let p = rect_path(10.0, 10.0, 90.0, 50.0);
    let inside = shape_stroke_coverage(
        &p,
        style(2.0, LineCap::Butt, miter(), StrokePosition::Inside),
    );
    let outside = shape_stroke_coverage(
        &p,
        style(2.0, LineCap::Butt, miter(), StrokePosition::Outside),
    );
    mass_is(inside.total(), 464.0, 0.05, "inside");
    mass_is(outside.total(), 496.0, 0.05, "outside");
    // The §20.2 rule-5 property: the two partition the 2× band exactly.
    let centered_2x = shape_stroke_coverage(
        &p,
        style(4.0, LineCap::Butt, miter(), StrokePosition::Center),
    );
    mass_is(
        inside.total() + outside.total(),
        centered_2x.total(),
        0.05,
        "inside ⊎ outside = the 2× band",
    );
}

// ------------------------------------------------------------------
// The collapse regime — the reason this machine is a piece UNION
// ------------------------------------------------------------------

/// The offset-rail failure the module header records (NEW-1): once the
/// half-width reaches a contour's local inradius, rail polygons mis-wind
/// and punch holes. A union of independently POSITIVE-oriented convex
/// pieces cannot: winding only accumulates. Probed across the full
/// collapse sweep — half-width under, at, and far past the inradius — on
/// every join arm.
#[test]
fn thick_strokes_of_small_contours_have_no_holes_on_any_join() {
    let p = rect_path(40.0, 40.0, 60.0, 50.0); // inradius 5
    for join in [miter(), LineJoin::Round, LineJoin::Bevel] {
        for w in [4.0, 10.0, 11.0, 40.0] {
            let cov =
                shape_stroke_coverage(&p, style(w, LineCap::Butt, join, StrokePosition::Center));
            // Every interior pixel of the source rect must be FULLY
            // covered once the band swallows it (w/2 ≥ inradius = 5).
            if w / 2.0 >= 5.0 {
                for y in 41..49 {
                    for x in 41..59 {
                        assert!(
                            cov.get(x, y) > 0.999,
                            "hole at ({x},{y}) w={w} join={join:?}: {}",
                            cov.get(x, y)
                        );
                    }
                }
            }
            // And the band is never empty nor NaN.
            assert!(cov.total() > 0.0 && cov.total().is_finite());
        }
    }
}

/// Degenerate sources stroke nothing (the module's standing pin, shared
/// with the command arm): a zero-width band, an empty path, a
/// single-point subpath.
#[test]
fn degenerate_shape_strokes_are_empty() {
    let p = rect_path(10.0, 10.0, 90.0, 50.0);
    for w in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let cov = shape_stroke_coverage(
            &p,
            style(w, LineCap::Round, miter(), StrokePosition::Center),
        );
        assert_eq!(cov.total(), 0.0, "width {w} strokes nothing");
    }
    let empty = VectorPath::new(PathId::nil(), FillRule::NonZero);
    assert_eq!(
        shape_stroke_coverage(
            &empty,
            style(4.0, LineCap::Round, miter(), StrokePosition::Center)
        )
        .total(),
        0.0
    );
    let point = polyline(&[(20.0, 20.0), (20.0, 20.0)], false);
    assert_eq!(
        shape_stroke_coverage(
            &point,
            style(4.0, LineCap::Round, miter(), StrokePosition::Center)
        )
        .total(),
        0.0,
        "a point strokes nothing (module pin; SVG would draw a dot)"
    );
}

/// A miter corner is SHARP where a round one is not — the pixel-level
/// discriminator behind the mass arithmetic above. At a 90° corner with
/// h = 4 the apex sits 4√2 ≈ 5.66 out along the diagonal, so the corner
/// pixel just outside the round arc's reach is covered by miter and bare
/// by round.
#[test]
fn the_miter_apex_covers_a_corner_pixel_the_round_arc_leaves_bare() {
    let p = rect_path(40.0, 40.0, 100.0, 80.0);
    let st = |j| style(8.0, LineCap::Butt, j, StrokePosition::Center);
    let m = shape_stroke_coverage(&p, st(miter()));
    let r = shape_stroke_coverage(&p, st(LineJoin::Round));
    // The kite for a right angle at h = 4 is the square [36, 40]², so
    // pixel (36, 36) — the apex pixel — lies wholly inside it; its nearest
    // point to the corner (37, 37) is 4.24 away, outside the radius-4 arc.
    assert!(
        m.get(36, 36) > 0.99,
        "miter apex covers the corner pixel: {}",
        m.get(36, 36)
    );
    assert!(
        r.get(36, 36) < 0.01,
        "the round arc leaves it bare: {}",
        r.get(36, 36)
    );
}

// ------------------------------------------------------------------
// Adversarial round 2 — the arms the first battery did not discriminate
// ------------------------------------------------------------------

/// The outer side of a turn is chosen from `sign(d0 × d1)`, so it must be
/// CONTOUR-ORIENTATION-AGNOSTIC. The first battery only ever walks a rect
/// one way round; walked the OTHER way every cross product flips sign, and
/// a hard-coded side would silently put every join piece on the INNER side
/// — where the two quads already cover it, so the miter mass would collapse
/// onto the bevel mass instead of exceeding it. Pinned on all three joins.
#[test]
fn the_join_side_does_not_depend_on_which_way_the_contour_is_walked() {
    let cw = polyline(
        &[(10.0, 10.0), (90.0, 10.0), (90.0, 50.0), (10.0, 50.0)],
        true,
    );
    let ccw = polyline(
        &[(10.0, 10.0), (10.0, 50.0), (90.0, 50.0), (90.0, 10.0)],
        true,
    );
    for (join, want) in [
        (miter(), 480.0),
        (LineJoin::Bevel, 478.0),
        (LineJoin::Round, 480.0 - 4.0 + poly_disc_area(1.0)),
    ] {
        let a = shape_stroke_coverage(&cw, style(2.0, LineCap::Butt, join, StrokePosition::Center));
        let b = shape_stroke_coverage(
            &ccw,
            style(2.0, LineCap::Butt, join, StrokePosition::Center),
        );
        mass_is(a.total(), want, 0.05, "clockwise walk");
        mass_is(b.total(), want, 0.05, "counter-clockwise walk");
    }
}

/// A HAIRPIN (an exact 180° reversal) is the one degenerate vertex that
/// still owes geometry: the two quads lie on top of each other and cover
/// nothing past the turn, so a ROUND join must add its half-disc
/// turnaround. Miter's apex runs to infinity there and §20.2 rule 2's
/// bevel fallback is a zero-area triangle — so butt/miter/bevel are
/// identical and round is strictly larger, by exactly the half-disc.
#[test]
fn a_round_join_still_turns_the_corner_at_an_exact_hairpin() {
    let hairpin = polyline(&[(10.0, 30.0), (50.0, 30.0), (10.0, 30.0)], false);
    let h = 3.0; // width 6
    let st = |j| style(2.0 * h, LineCap::Butt, j, StrokePosition::Center);
    let bevel = shape_stroke_coverage(&hairpin, st(LineJoin::Bevel)).total();
    let mitered = shape_stroke_coverage(&hairpin, st(miter())).total();
    let round = shape_stroke_coverage(&hairpin, st(LineJoin::Round)).total();
    // The doubled-back band is one 40 × 6 quad: 240.
    mass_is(bevel, 240.0, 0.05, "hairpin, bevel");
    mass_is(mitered, 240.0, 0.05, "hairpin, miter falls back");
    // Round adds the HALF disc past x = 50 (the other half lies inside
    // the quad), polygonized at ≤ 12°.
    mass_is(
        round,
        240.0 + poly_disc_area(h) / 2.0,
        0.05,
        "hairpin, round",
    );
    assert!(
        round > bevel + 10.0,
        "round genuinely turns: {round} vs {bevel}"
    );
}

/// A collinear CONTINUATION is not a hairpin: a straight run split into
/// two segments must stroke exactly like the undivided line, on every
/// join arm — including round, whose turnaround the hairpin arm above
/// switches on. This is the perturbation partner of that test: it proves
/// the hairpin fix keys on the REVERSAL, not merely on "cross ≈ 0".
#[test]
fn a_collinear_split_strokes_exactly_like_the_undivided_line() {
    let whole = open_hline(10.0, 90.0, 30.0);
    let split = polyline(&[(10.0, 30.0), (50.0, 30.0), (90.0, 30.0)], false);
    for join in [miter(), LineJoin::Round, LineJoin::Bevel] {
        for cap in [LineCap::Butt, LineCap::Round, LineCap::Square] {
            let a = shape_stroke_coverage(&whole, style(6.0, cap, join, StrokePosition::Center));
            let b = shape_stroke_coverage(&split, style(6.0, cap, join, StrokePosition::Center));
            assert_eq!(a, b, "collinear split is inert ({join:?}, {cap:?})");
        }
    }
}

/// `miterLimit` is `{type: number}` on the wire with NO minimum, so a
/// zero or negative limit is a contract-valid payload. The recorded
/// election clamps it at 0, i.e. "never miter" — bit-identical to a bevel
/// join, never a NaN, never an unbounded spike.
#[test]
fn a_zero_or_negative_miter_limit_is_exactly_a_bevel() {
    let p = rect_path(10.0, 10.0, 90.0, 50.0);
    let bevel = shape_stroke_coverage(
        &p,
        style(4.0, LineCap::Butt, LineJoin::Bevel, StrokePosition::Center),
    );
    for limit in [0.0, -1.0, -1e9] {
        let got = shape_stroke_coverage(
            &p,
            style(
                4.0,
                LineCap::Butt,
                LineJoin::Miter(limit),
                StrokePosition::Center,
            ),
        );
        assert_eq!(got, bevel, "miterLimit {limit} ⇒ bevel, bit-identical");
    }
    // And the default limit of 4 DOES miter a right angle (ratio √2), so
    // the pin above is not passing because everything bevels.
    let default_limit = shape_stroke_coverage(
        &p,
        style(4.0, LineCap::Butt, miter(), StrokePosition::Center),
    );
    assert!(
        default_limit.total() > bevel.total() + 3.0,
        "the default limit still miters: {} vs {}",
        default_limit.total(),
        bevel.total()
    );
}
