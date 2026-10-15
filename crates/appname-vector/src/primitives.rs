// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! §20.4 primitive shape constructors (CP-1.0; the geometry half of
//! `vector.shape_add` — the kind arm stores what these build).
//!
//! Spec: ES §20.4 "Primitive shape tools" [table]:
//! * Rectangle — 4 independent corner radii, "radii clamped so adjacent
//!   radii sum ≤ edge length; arcs are κ-cubics, κ = 4(√2−1)/3".
//! * Ellipse — 4 κ-cubics.
//! * Polygon — sides 3–100; star toggle + inset 0–99 %
//!   (`r_inner = r_outer·(1 − inset)`).
//! * Line — weight px; arrowheads start/end (length %, width %,
//!   concavity %); "a line is a thin rectangle path; arrowheads are
//!   appended subpaths".
//!
//! Pinned elections where the table is silent (fold-in register):
//! * Radii clamp realization: one uniform scale
//!   `s = min(1, min over the 4 edges of L/(rᵃ+rᵇ))` applied to all
//!   four radii — preserves the radii RATIOS while satisfying every
//!   adjacent-sum constraint (per-corner independent truncation would
//!   distort intent and is order-dependent).
//! * Arrowhead geometry: tip AT the line endpoint; the body rectangle
//!   shortens by each head's length; head width/length are % of the
//!   line WEIGHT (the table binds them to weight); concavity % indents
//!   the back edge toward the tip by that fraction of the head length
//!   (0 = flat triangle back; negative = convex bulge).
//! * All contours emitted CCW-positive (the crate's shoelace-positive
//!   convention, §20.3 output orientation); parametric-until-edited
//!   state (§20.4 "All primitives stay parametric") is the doc-model's
//!   concern, not geometry's.
//! * The star toggle IS the API: [`polygon`] builds the plain n-gon;
//!   [`star`] applies `r_inner = r·(1 − inset)` VERBATIM and always
//!   emits 2n vertices — at inset 0 that is a 2n-gon with midpoint
//!   vertices ON the circumcircle (continuous in inset), not the
//!   polygon (promoted to this register by the review round).

use crate::geom::Vec2;
use crate::path::{CubicSeg, SubPath};

/// Edge-collapse weld threshold: 2× the §20.1 invariant-3 degeneracy
/// epsilon (1e-9 from the CENTROID — a straight segment of length L
/// puts its endpoints at L/2, so segments up to L = 2e-9 are
/// degenerate; the re-review probed the [1e-9, 2e-9) window failing
/// validate under the old 1e-9 weld). Dimensions at or below this can
/// never yield valid geometry and are refused outright.
const WELD_EPS: f64 = 2e-9;

/// κ = 4(√2−1)/3 (§20.4): the quarter-arc cubic constant.
pub const KAPPA: f64 = 0.552_284_749_830_793_4;

fn close(segments: Vec<CubicSeg>) -> SubPath {
    SubPath {
        segments,
        closed: true,
    }
}

/// §20.4 rectangle with per-corner radii `[tl, tr, br, bl]` (doc-space
/// y-down; corners named visually). Radii are clamped per the pinned
/// uniform-scale rule; zero radii yield exact line segments. Returns
/// `None` outside the domain (w ≤ 0 or h ≤ 0) — every constructor here
/// refuses degenerate inputs rather than emitting geometry that the
/// crate's own strict `validate()` would reject (review round).
pub fn rectangle(x0: f64, y0: f64, w: f64, h: f64, radii: [f64; 4]) -> Option<SubPath> {
    // Finite inputs, dims above the weld epsilon (re-review residuals:
    // an infinite radius reached the clamp as inf·0 = NaN; sub-ε dims
    // produced weld-inconsistent corners).
    if !(x0.is_finite() && y0.is_finite() && w.is_finite() && h.is_finite())
        || !radii.iter().all(|r| r.is_finite())
        || w <= WELD_EPS
        || h <= WELD_EPS
    {
        return None;
    }
    let [tl, tr, br, bl] = radii.map(|r| r.max(0.0));
    // Adjacent-sum clamp, one uniform scale (pinned election above).
    let mut s = 1.0f64;
    for (a, b, edge) in [
        (tl, tr, w), // top
        (tr, br, h), // right
        (br, bl, w), // bottom
        (bl, tl, h), // left
    ] {
        if a + b > edge && a + b > 0.0 {
            s = s.min(edge / (a + b));
        }
    }
    let (tl, tr, br, bl) = (tl * s, tr * s, br * s, bl * s);
    let (x1, y1) = (x0 + w, y0 + h);

    let mut segs: Vec<CubicSeg> = Vec::new();
    let line_to = |segs: &mut Vec<CubicSeg>, from: Vec2, to: Vec2| {
        if from != to {
            // welded edges are bitwise-equal and emit nothing
            segs.push(CubicSeg::line(from, to));
        }
    };
    // Corner quarter-arc: from edge point `p`, to edge point `q`,
    // bulging toward corner `c` (κ-cubic).
    let arc = |p: Vec2, q: Vec2, c: Vec2| CubicSeg {
        p0: p,
        c1: p.lerp(c, KAPPA),
        c2: q.lerp(c, KAPPA),
        p1: q,
    };

    // Walk CW visually in y-down = CCW by the shoelace-positive
    // convention: top edge L→R, right edge T→B, bottom R→L, left B→T.
    let mut t0 = Vec2::new(x0 + tl, y0);
    let mut t1 = Vec2::new(x1 - tr, y0);
    let mut r0 = Vec2::new(x1, y0 + tr);
    let mut r1 = Vec2::new(x1, y1 - br);
    let mut b0 = Vec2::new(x1 - br, y1);
    let mut b1 = Vec2::new(x0 + bl, y1);
    let mut l0 = Vec2::new(x0, y1 - bl);
    let mut l1 = Vec2::new(x0, y0 + tl);
    // BITWISE WELD (review MAJOR): when the clamp collapses an edge,
    // the inexact scale division can leave the two edge endpoints a
    // few ulp apart (sometimes reversed), emitting a degenerate
    // micro-segment that the crate's strict validate() rejects.
    // Snap sub-ε edge runs to ONE shared value so the adjacent arcs
    // are bitwise-contiguous and no line is emitted at all.
    let weld = |p: &mut Vec2, q: &mut Vec2| {
        if p.distance(*q) <= WELD_EPS {
            let m = p.midpoint(*q);
            *p = m;
            *q = m;
        }
    };
    weld(&mut t0, &mut t1);
    weld(&mut r0, &mut r1);
    weld(&mut b0, &mut b1);
    weld(&mut l0, &mut l1);

    line_to(&mut segs, t0, t1);
    if tr > 0.0 {
        segs.push(arc(t1, r0, Vec2::new(x1, y0)));
    }
    line_to(&mut segs, r0, r1);
    if br > 0.0 {
        segs.push(arc(r1, b0, Vec2::new(x1, y1)));
    }
    line_to(&mut segs, b0, b1);
    if bl > 0.0 {
        segs.push(arc(b1, l0, Vec2::new(x0, y1)));
    }
    line_to(&mut segs, l0, l1);
    if tl > 0.0 {
        segs.push(arc(l1, t0, Vec2::new(x0, y0)));
    }
    Some(close(segs))
}

/// §20.4 ellipse: 4 κ-cubics about (cx, cy) with radii (rx, ry).
/// `None` outside the domain (rx ≤ 0 or ry ≤ 0).
pub fn ellipse(cx: f64, cy: f64, rx: f64, ry: f64) -> Option<SubPath> {
    if !(cx.is_finite() && cy.is_finite() && rx.is_finite() && ry.is_finite())
        || rx <= WELD_EPS
        || ry <= WELD_EPS
    {
        return None;
    }
    let (kx, ky) = (KAPPA * rx, KAPPA * ry);
    let e = Vec2::new(cx + rx, cy);
    let s = Vec2::new(cx, cy + ry);
    let w = Vec2::new(cx - rx, cy);
    let n = Vec2::new(cx, cy - ry);
    Some(close(vec![
        CubicSeg {
            p0: e,
            c1: Vec2::new(cx + rx, cy + ky),
            c2: Vec2::new(cx + kx, cy + ry),
            p1: s,
        },
        CubicSeg {
            p0: s,
            c1: Vec2::new(cx - kx, cy + ry),
            c2: Vec2::new(cx - rx, cy + ky),
            p1: w,
        },
        CubicSeg {
            p0: w,
            c1: Vec2::new(cx - rx, cy - ky),
            c2: Vec2::new(cx - kx, cy - ry),
            p1: n,
        },
        CubicSeg {
            p0: n,
            c1: Vec2::new(cx + kx, cy - ry),
            c2: Vec2::new(cx + rx, cy - ky),
            p1: e,
        },
    ]))
}

/// §20.4 polygon (star toggle OFF): `sides` 3–100 (clamped),
/// circumradius `r`, apex at angle `rotation` radians (0 = pointing up
/// in y-down doc space, i.e. −y; §20.4's Shift-snap to 15° is a tool
/// concern). `None` when r ≤ 0. The toggle lives in the API: this
/// builds the plain n-gon; [`star`] applies the inset formula VERBATIM
/// and always emits 2n vertices — at inset 0 that is a 2n-gon with
/// midpoint vertices ON the circumcircle, exactly what
/// `r_inner = r·(1 − 0)` says, NOT this polygon (pinned election,
/// promoted from a doc aside by the review round: the two functions ARE
/// the spec's toggle, and star's shape is continuous in inset).
pub fn polygon(cx: f64, cy: f64, r: f64, sides: u32, rotation: f64) -> Option<SubPath> {
    ring(cx, cy, r, sides, rotation, None)
}

/// §20.4 star (toggle ON): inset 0–0.99 as a fraction,
/// `r_inner = r·(1 − inset)` verbatim; emits 2·sides vertices always.
/// `None` when r ≤ 0.
pub fn star(cx: f64, cy: f64, r: f64, sides: u32, inset: f64, rotation: f64) -> Option<SubPath> {
    let inset = inset.clamp(0.0, 0.99);
    ring(cx, cy, r, sides, rotation, Some(inset))
}

/// Shared n-gon / 2n-star vertex walk. DETERMINISM NOTE (review round,
/// recorded for the fold-in register): this is the crate's only
/// production trig — vertex positions inherit platform-libm ulp
/// variance. D-48 posture: the `vector.shape_add` kind arm VALUE-
/// CAPTURES the generated sidecar at creation, so replay reads stored
/// geometry and never re-runs these; parametric re-edit recomputes
/// only on explicit user action (a fresh command with fresh captures).
fn ring(
    cx: f64,
    cy: f64,
    r: f64,
    sides: u32,
    rotation: f64,
    inset: Option<f64>,
) -> Option<SubPath> {
    if !(cx.is_finite() && cy.is_finite() && rotation.is_finite() && r.is_finite()) || r <= WELD_EPS
    {
        return None;
    }
    let sides = sides.clamp(3, 100);
    let c = Vec2::new(cx, cy);
    let apex = -core::f64::consts::FRAC_PI_2 + rotation; // up in y-down
    let n = sides as usize;
    let mut pts: Vec<Vec2> = Vec::with_capacity(2 * n);
    for i in 0..n {
        let a = apex + core::f64::consts::TAU * (i as f64) / (n as f64);
        pts.push(c + Vec2::new(a.cos(), a.sin()) * r);
        if let Some(inset) = inset {
            let mid = a + core::f64::consts::PI / (n as f64);
            pts.push(c + Vec2::new(mid.cos(), mid.sin()) * (r * (1.0 - inset)));
        }
    }
    let m = pts.len();
    Some(close(
        (0..m)
            .map(|i| CubicSeg::line(pts[i], pts[(i + 1) % m]))
            .collect(),
    ))
}

/// §20.4 arrowhead parameters (percent-of-weight lengths per the
/// table). Accepted domain (documented, clamped in [`line`]):
/// length/width % clamp at ≥ 0; concavity % clamps at ≤ 99 (100+
/// would invert the head through its own tip); negative concavity =
/// convex back, unbounded below.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Arrowhead {
    pub length_pct: f64,
    pub width_pct: f64,
    pub concavity_pct: f64,
}

/// §20.4 line: a thin rectangle path of `weight` px from `a` to `b`,
/// arrowheads appended as their own subpaths (pinned geometry in the
/// module header). Returns the subpaths in body-first order. A
/// zero-length or zero-weight line yields no subpaths.
pub fn line(
    a: Vec2,
    b: Vec2,
    weight: f64,
    head_start: Option<Arrowhead>,
    head_end: Option<Arrowhead>,
) -> Vec<SubPath> {
    if !(a.is_finite() && b.is_finite() && weight.is_finite()) || weight <= WELD_EPS {
        return Vec::new();
    }
    let Some(dir) = (b - a).normalize() else {
        return Vec::new();
    };
    let n = Vec2::new(dir.y, -dir.x); // left normal (crate convention)
    let half = weight / 2.0;

    let head_len = |h: &Arrowhead| weight * h.length_pct.max(0.0) / 100.0;
    let ls = head_start.as_ref().map_or(0.0, head_len);
    let le = head_end.as_ref().map_or(0.0, head_len);
    // Body shortens by each head's length (pinned); degenerate overlap
    // collapses the body but keeps the heads.
    let body_a = a + dir * ls;
    let body_b = b - dir * le;

    let mut out = Vec::new();
    if (body_b - body_a).dot(dir) > 0.0 {
        let p0 = body_a + n * half;
        let p1 = body_b + n * half;
        let p2 = body_b - n * half;
        let p3 = body_a - n * half;
        out.push(close(vec![
            CubicSeg::line(p0, p1),
            CubicSeg::line(p1, p2),
            CubicSeg::line(p2, p3),
            CubicSeg::line(p3, p0),
        ]));
    }
    let mut head = |tip: Vec2, inward: Vec2, h: &Arrowhead| {
        let l = weight * h.length_pct.max(0.0) / 100.0;
        let wdt = weight * h.width_pct.max(0.0) / 100.0;
        if l <= 0.0 || wdt <= 0.0 {
            return; // zero-area head: nothing to append
        }
        let back = tip + inward * l;
        let notch = back - inward * (l * h.concavity_pct.min(99.0) / 100.0);
        let side = Vec2::new(inward.y, -inward.x) * (wdt / 2.0);
        out.push(close(vec![
            CubicSeg::line(tip, back + side),
            CubicSeg::line(back + side, notch),
            CubicSeg::line(notch, back - side),
            CubicSeg::line(back - side, tip),
        ]));
    };
    if let Some(h) = head_start {
        head(a, dir, &h);
    }
    if let Some(h) = head_end {
        head(b, -dir, &h);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flatten::flatten_subpath;

    fn area(sub: &SubPath) -> f64 {
        let poly = flatten_subpath(sub, 0.01);
        let mut s = 0.0;
        for i in 0..poly.len() {
            let a = poly[i];
            let b = poly[(i + 1) % poly.len()];
            s += a.x * b.y - a.y * b.x;
        }
        s / 2.0
    }

    #[test]
    fn plain_rectangle_is_exact_lines() {
        let r = rectangle(1.0, 2.0, 10.0, 6.0, [0.0; 4]).unwrap();
        assert_eq!(r.segments.len(), 4);
        assert!((area(&r) - 60.0).abs() < 1e-9, "area {}", area(&r));
    }

    #[test]
    fn rounded_rectangle_area_matches_the_analytic_value() {
        // Area = wh − (4 − π)·r² for uniform radius r.
        let r = 2.0;
        let sub = rectangle(0.0, 0.0, 10.0, 6.0, [r; 4]).unwrap();
        let want = 60.0 - (4.0 - core::f64::consts::PI) * r * r;
        // κ-arc vs true circle ≈ 2.7e-4 relative; flatten adds ~1e-4.
        assert!((area(&sub) - want).abs() < 0.02, "area {}", area(&sub));
    }

    #[test]
    fn radii_clamp_scales_uniformly_and_preserves_ratios() {
        // tl=8, tr=8 on a 10-wide top edge → scale 10/16 = 0.625.
        let sub = rectangle(0.0, 0.0, 10.0, 100.0, [8.0, 8.0, 0.0, 0.0]).unwrap();
        // The top edge's straight run collapses to 10 − 5 − 5 = 0: the
        // first segment must start at x = 5 (= 8·0.625).
        let first = &sub.segments[0];
        assert!(
            (first.p0.x - 5.0).abs() < 1e-9,
            "clamped radius 5, got {}",
            first.p0.x
        );
    }

    #[test]
    fn ellipse_area_is_pi_ab_within_kappa_tolerance() {
        let sub = ellipse(3.0, 4.0, 5.0, 2.0).unwrap();
        let want = core::f64::consts::PI * 5.0 * 2.0;
        // Oracle note: the flattened-polygon area UNDERESTIMATES a
        // convex curve by the summed chord sagitta (≈ ⅔·tol·perimeter —
        // at tol 0.01 that is ~0.15 on this ellipse, dwarfing the
        // κ-cubic's own ~3e-4 relative error). Measure at tol 1e-3 and
        // bound by κ-error + the remaining flatten term.
        let poly = flatten_subpath(&sub, 1e-3);
        let mut got = 0.0;
        for i in 0..poly.len() {
            let a = poly[i];
            let b = poly[(i + 1) % poly.len()];
            got += a.x * b.y - a.y * b.x;
        }
        got /= 2.0;
        assert!((got - want).abs() < want * 1e-3, "area {got}");
    }

    #[test]
    fn polygon_area_matches_the_closed_form() {
        // A = (n/2) r² sin(2π/n).
        for n in [3u32, 5, 8, 100] {
            let sub = polygon(0.0, 0.0, 4.0, n, 0.0).unwrap();
            assert_eq!(sub.segments.len(), n as usize);
            let want = (f64::from(n) / 2.0) * 16.0 * (core::f64::consts::TAU / f64::from(n)).sin();
            assert!(
                (area(&sub).abs() - want).abs() < 1e-9,
                "n={n}: {}",
                area(&sub)
            );
        }
    }

    #[test]
    fn star_emits_double_vertices_and_respects_inset() {
        let sub = star(0.0, 0.0, 4.0, 5, 0.5, 0.0).unwrap();
        assert_eq!(sub.segments.len(), 10);
        // Inner vertices at r·(1−inset) = 2: check one midpoint radius.
        let inner = sub.segments[0].p1;
        assert!(
            (inner.length() - 2.0).abs() < 1e-9,
            "inner radius {}",
            inner.length()
        );
        // sides clamp: 2 → 3; 500 → 100.
        assert_eq!(polygon(0.0, 0.0, 1.0, 2, 0.0).unwrap().segments.len(), 3);
        assert_eq!(
            polygon(0.0, 0.0, 1.0, 500, 0.0).unwrap().segments.len(),
            100
        );
    }

    #[test]
    fn line_is_a_weight_by_length_rectangle() {
        let subs = line(Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0), 2.0, None, None);
        assert_eq!(subs.len(), 1);
        assert!((area(&subs[0]).abs() - 20.0).abs() < 1e-9);
    }

    #[test]
    fn arrowheads_append_subpaths_with_exact_areas() {
        // weight 2, head length 300 % = 6 px, width 400 % = 8 px,
        // concavity 0: triangle area = ½·8·6 = 24. Body shortens to 4.
        let h = Arrowhead {
            length_pct: 300.0,
            width_pct: 400.0,
            concavity_pct: 0.0,
        };
        let subs = line(
            Vec2::new(0.0, 0.0),
            Vec2::new(10.0, 0.0),
            2.0,
            None,
            Some(h),
        );
        assert_eq!(subs.len(), 2, "body + one head");
        assert!((area(&subs[0]).abs() - 8.0).abs() < 1e-9, "body 4×2");
        assert!((area(&subs[1]).abs() - 24.0).abs() < 1e-9, "head ½·8·6");
        // Concave notch removes ½·w·(l·c%) : c=50 % → notch 12 → 12 net.
        let hc = Arrowhead {
            concavity_pct: 50.0,
            ..h
        };
        let subs = line(
            Vec2::new(0.0, 0.0),
            Vec2::new(10.0, 0.0),
            2.0,
            None,
            Some(hc),
        );
        assert!((area(&subs[1]).abs() - 12.0).abs() < 1e-9, "notched head");
    }

    #[test]
    fn primitives_are_deterministic() {
        let a = star(1.0, 2.0, 5.0, 7, 0.3, 0.4).unwrap();
        let b = star(1.0, 2.0, 5.0, 7, 0.3, 0.4).unwrap();
        assert_eq!(a.segments, b.segments);
        let r1 = rectangle(0.0, 0.0, 8.0, 5.0, [1.0, 2.0, 3.0, 0.5]).unwrap();
        let r2 = rectangle(0.0, 0.0, 8.0, 5.0, [1.0, 2.0, 3.0, 0.5]).unwrap();
        assert_eq!(r1.segments, r2.segments);
    }

    #[test]
    fn degenerate_inputs_are_refused_uniformly() {
        assert!(line(Vec2::new(1.0, 1.0), Vec2::new(1.0, 1.0), 2.0, None, None).is_empty());
        assert!(line(Vec2::new(0.0, 0.0), Vec2::new(5.0, 0.0), 0.0, None, None).is_empty());
        assert!(rectangle(0.0, 0.0, 0.0, 5.0, [0.0; 4]).is_none());
        assert!(rectangle(0.0, 0.0, 5.0, -1.0, [0.0; 4]).is_none());
        assert!(ellipse(0.0, 0.0, 0.0, 2.0).is_none());
        assert!(polygon(0.0, 0.0, 0.0, 5, 0.0).is_none());
        assert!(star(0.0, 0.0, -1.0, 5, 0.5, 0.0).is_none());
    }

    #[test]
    fn constructor_output_always_passes_strict_validate() {
        // Review MAJOR regression: the clamp's inexact scale division
        // used to emit ulp-scale micro-segments on collapsed edges,
        // failing the crate's own writer contract. The bitwise weld
        // must make every constructor output validate() clean —
        // including the reviewer's exact repro (non-dyadic radii
        // collapsing the top edge of a 1-px-wide rect).
        use crate::path::{FillRule, VectorPath};
        use appname_core::PathId;
        let subs = [
            rectangle(0.0, 0.0, 1.0, 100.0, [0.6, 0.7, 0.0, 0.0]).unwrap(),
            rectangle(0.0, 0.0, 10.0, 6.0, [3.0, 3.0, 3.0, 3.0]).unwrap(),
            rectangle(0.0, 0.0, 4.0, 4.0, [2.0, 2.0, 2.0, 2.0]).unwrap(), // all edges collapse
            ellipse(3.0, 4.0, 5.0, 2.0).unwrap(),
            star(0.0, 0.0, 4.0, 5, 0.5, 0.3).unwrap(),
            polygon(1.0, 1.0, 3.0, 7, 0.1).unwrap(),
        ];
        for (i, sub) in subs.into_iter().enumerate() {
            let mut p = VectorPath::new(PathId::nil(), FillRule::NonZero);
            p.subpaths.push(sub);
            assert!(
                p.validate().is_ok(),
                "constructor output {i} failed validate: {:?}",
                p.validate()
            );
        }
        for h in line(
            Vec2::new(0.0, 0.0),
            Vec2::new(10.0, 3.0),
            2.0,
            Some(Arrowhead {
                length_pct: 300.0,
                width_pct: 400.0,
                concavity_pct: 50.0,
            }),
            Some(Arrowhead {
                length_pct: 250.0,
                width_pct: 350.0,
                concavity_pct: -20.0,
            }),
        ) {
            let mut p = VectorPath::new(PathId::nil(), FillRule::NonZero);
            p.subpaths.push(h);
            assert!(p.validate().is_ok());
        }
    }

    #[test]
    fn star_at_inset_zero_is_the_verbatim_formula_not_the_polygon() {
        // Pinned election (promoted by review): the toggle is the API;
        // star applies r_inner = r(1 − inset) verbatim, so inset 0
        // emits a 2n-gon with midpoint vertices ON the circumcircle —
        // continuous as inset → 0 — while polygon() is the plain n-gon.
        let s0 = star(0.0, 0.0, 4.0, 5, 0.0, 0.0).unwrap();
        assert_eq!(s0.segments.len(), 10, "2n vertices at inset 0");
        let mid = s0.segments[0].p1;
        assert!(
            (mid.length() - 4.0).abs() < 1e-9,
            "midpoint on circumcircle"
        );
        let tiny = star(0.0, 0.0, 4.0, 5, 1e-9, 0.0).unwrap();
        assert_eq!(tiny.segments.len(), 10, "no shape discontinuity");
        assert_eq!(polygon(0.0, 0.0, 4.0, 5, 0.0).unwrap().segments.len(), 5);
    }

    #[test]
    fn weld_window_and_non_finite_residuals_are_closed() {
        use crate::path::{FillRule, VectorPath};
        use appname_core::PathId;
        // Re-review probe: an edge run of 1.5e-9 sat in the [1e-9,
        // 2e-9) window — §20.1-degenerate but under the old weld.
        let sub = rectangle(0.0, 0.0, 10.0, 10.0, [5.0, 5.0 - 1.5e-9, 0.0, 0.0]).unwrap();
        let mut p = VectorPath::new(PathId::nil(), FillRule::NonZero);
        p.subpaths.push(sub);
        assert!(
            p.validate().is_ok(),
            "weld-window repro: {:?}",
            p.validate()
        );
        // Non-finite residuals refuse cleanly.
        assert!(rectangle(0.0, 0.0, 5.0, 5.0, [f64::INFINITY, 0.0, 0.0, 0.0]).is_none());
        assert!(rectangle(f64::NAN, 0.0, 5.0, 5.0, [0.0; 4]).is_none());
        assert!(ellipse(f64::NAN, 0.0, 1.0, 1.0).is_none());
        assert!(polygon(0.0, 0.0, 1.0, 5, f64::NAN).is_none());
        assert!(line(
            Vec2::new(0.0, 0.0),
            Vec2::new(5.0, 0.0),
            f64::NAN,
            None,
            None
        )
        .is_empty());
        // Sub-ε dims refuse (5e-10 used to pass w > 0 and fail validate).
        assert!(rectangle(0.0, 0.0, 5e-10, 5.0, [0.0; 4]).is_none());
    }

    #[test]
    fn arrowhead_domain_clamps() {
        // Negative length/width: zero-area head → not appended.
        let bad = Arrowhead {
            length_pct: -50.0,
            width_pct: 400.0,
            concavity_pct: 0.0,
        };
        let subs = line(
            Vec2::new(0.0, 0.0),
            Vec2::new(10.0, 0.0),
            2.0,
            None,
            Some(bad),
        );
        assert_eq!(subs.len(), 1, "only the body");
        // Concavity ≥ 100 clamps at 99 (head never inverts through its tip).
        let deep = Arrowhead {
            length_pct: 300.0,
            width_pct: 400.0,
            concavity_pct: 250.0,
        };
        let subs = line(
            Vec2::new(0.0, 0.0),
            Vec2::new(10.0, 0.0),
            2.0,
            None,
            Some(deep),
        );
        let head_area = area(&subs[1]).abs();
        let want = 0.5 * 8.0 * 6.0 * (1.0 - 0.99);
        assert!(
            (head_area - want).abs() < 1e-9,
            "clamped at 99%: {head_area}"
        );
    }
}
