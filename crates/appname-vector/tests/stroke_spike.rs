// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! `stroke_path` geometry battery (§20.2 band semantics, §13.5
//! iso-contour, the module pins) — exact-mass oracles via closed
//! forms: smooth-curve bands follow the tube formula w·L; polygon
//! corners carry the round-outer-gain/sharp-inner-loss correction;
//! open lines add π·(w/2)² for the round caps; Inside erosion corners
//! are sharp, so those masses are exact rectangle arithmetic.

use appname_core::engine::vector_ops::StrokePosition;
use appname_core::geom::IRect;
use appname_core::PathId;
use appname_vector::geom::Vec2;
use appname_vector::path::{CubicSeg, FillRule, SubPath, VectorPath};
use appname_vector::raster::CoveragePlane;
use appname_vector::stroke::{stroke_boundary_coverage, stroke_coverage, trace_iso_contours};

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

#[test]
fn center_stroke_of_a_rect_is_the_exact_ring() {
    // Rect 10..90 × 10..50, width 2, center. The band's OUTER boundary
    // rounds at the corners (arcs, radius 1: dilation = 240 + π); the
    // INNER boundary is SHARP (interior distance is min-over-edges:
    // 3200 − 78·38 = 236). True area = 236 + 240 + π = 479.1416.
    let p = rect_path(10.0, 10.0, 90.0, 50.0);
    let cov = stroke_coverage(&p, 2.0, StrokePosition::Center).unwrap();
    let want = 236.0 + 240.0 + std::f64::consts::PI;
    assert!(
        (cov.total() - want).abs() < 0.1,
        "distance-band ring {want}: {}",
        cov.total()
    );
}

#[test]
fn inside_and_outside_strokes_split_the_double_band_exactly() {
    // §20.2 rule 5 via shared coverage. Inside: fill ∩ {d ≤ 2} — the
    // erosion corner is SHARP (interior distance is min-over-edges),
    // so 3200 − 76×36 = 464 exactly. Outside: {d ≤ 2} ∖ fill — outer
    // corners are ROUND quarter-arcs radius 2: 3696 − (16 − 4π) − 3200.
    let p = rect_path(10.0, 10.0, 90.0, 50.0);
    let inside = stroke_coverage(&p, 2.0, StrokePosition::Inside).unwrap();
    let outside = stroke_coverage(&p, 2.0, StrokePosition::Outside).unwrap();
    assert!(
        (inside.total() - 464.0).abs() < 0.05,
        "inside ring: got {}",
        inside.total()
    );
    let outside_true = 496.0 - (16.0 - 4.0 * std::f64::consts::PI);
    assert!(
        (outside.total() - outside_true).abs() < 0.5,
        "outside ring {outside_true} (±0.5 AA curvature bias): got {}",
        outside.total()
    );
    // The stroke edge coincides with the fill edge (the rule's point):
    // INK extent, not allocation rect — inside ink never leaves the
    // fill; outside ink never enters it.
    let ink_max_x = |c: &appname_vector::raster::CoveragePlane| {
        let mut m = i32::MIN;
        for y in c.rect.min.y..c.rect.max.y {
            for x in c.rect.min.x..c.rect.max.x {
                if c.get(x, y) > 0.0 {
                    m = m.max(x);
                }
            }
        }
        m
    };
    assert!(ink_max_x(&inside) <= 89, "inside ink stays within the fill");
    let mut out_in_fill = false;
    for y in 12..48 {
        for x in 12..88 {
            if outside.get(x, y) > 0.0 {
                out_in_fill = true;
            }
        }
    }
    assert!(!out_in_fill, "outside ink never enters the fill interior");
}

#[test]
fn open_line_stroke_gets_round_caps_exactly() {
    // Horizontal open segment (10,30)→(90,30), width 4: the strip
    // 80×4 = 320 plus two ROUND half-caps = π·2² (the round-cap pin).
    let p = open_hline(10.0, 90.0, 30.0);
    let cov = stroke_coverage(&p, 4.0, StrokePosition::Center).unwrap();
    let want = 320.0 + 4.0 * std::f64::consts::PI;
    assert!(
        (cov.total() - want).abs() < 0.5,
        "round-capped line {want} (±0.5 AA curvature bias): got {}",
        cov.total()
    );
}

#[test]
fn iso_contour_of_a_single_pixel_is_the_half_diamond() {
    // BG-026's named unit shape: one full pixel at (5,5) — marching
    // squares at 0.5 yields the diamond through the four midpoints
    // between the pixel center and its zero neighbors: area 0.5.
    let mut plane = CoveragePlane::new(IRect::new(5, 5, 6, 6));
    plane.v[0] = 1.0;
    let loops = trace_iso_contours(&plane);
    assert_eq!(loops.len(), 1, "one closed loop");
    assert_eq!(loops[0].len(), 4, "four midpoint vertices");
    // Shoelace area (loops orient deterministically).
    let a: f64 = loops[0]
        .iter()
        .zip(loops[0].iter().cycle().skip(1))
        .map(|(p, q)| p.x * q.y - q.x * p.y)
        .sum::<f64>()
        .abs()
        / 2.0;
    assert!((a - 0.5).abs() < 1e-12, "diamond area exactly ½: {a}");
}

#[test]
fn boundary_stroke_of_a_filled_rect_rings_the_iso_contour() {
    // Selection = full-coverage rect 20..60 × 20..40. The §13.5
    // contour runs through the half-steps at the coverage edge
    // (pixel-center lerp midpoints → the exact integer rect), with
    // half-pixel corner chamfers. Width-2 centered ring ≈ 240 minus
    // the four chamfer corners' effect — pinned within ±6.
    let rect = IRect::new(20, 20, 60, 40);
    let mut plane = CoveragePlane::new(rect);
    for v in plane.v.iter_mut() {
        *v = 1.0;
    }
    let loops = trace_iso_contours(&plane);
    assert_eq!(loops.len(), 1, "one boundary loop");
    let (mut min_x, mut max_x) = (f64::INFINITY, f64::NEG_INFINITY);
    for p in &loops[0] {
        min_x = min_x.min(p.x);
        max_x = max_x.max(p.x);
    }
    assert_eq!((min_x, max_x), (20.0, 60.0), "contour at the exact edges");
    let cov = stroke_boundary_coverage(&plane, 2.0, StrokePosition::Center, PathId::nil()).unwrap();
    // Tube formula on the chamfered contour: L = 2·(39+19) + 4·(0.5√2).
    let ring = 2.0 * (2.0 * (39.0 + 19.0) + 4.0 * 0.5 * std::f64::consts::SQRT_2);
    assert!(
        (cov.total() - ring).abs() < 3.0,
        "chamfered ring mass near {ring}: got {}",
        cov.total()
    );
}

#[test]
fn degenerate_sources_stroke_nothing() {
    // A single point (all segments degenerate) strokes nothing; an
    // empty selection plane traces nothing.
    let mut p = VectorPath::new(PathId::nil(), FillRule::NonZero);
    p.subpaths.push(SubPath {
        segments: vec![CubicSeg::line(Vec2::new(5.0, 5.0), Vec2::new(5.0, 5.0))],
        closed: false,
    });
    let cov = stroke_coverage(&p, 4.0, StrokePosition::Center).unwrap();
    assert_eq!(cov.total(), 0.0);
    let plane = CoveragePlane::new(IRect::new(0, 0, 8, 8));
    let loops = trace_iso_contours(&plane);
    assert!(loops.is_empty(), "all-zero plane has no 0.5 contour");
}

// ---------------------------------------------------------------------
// §5 review fix-round pins (MAJOR-1 saddles, MINOR-5 pinch loops)
// ---------------------------------------------------------------------

#[test]
fn both_saddle_codes_follow_the_center_sample_rule() {
    // Review MAJOR-1 (the reviewer's probes): §13.5 [NORMATIVE] —
    // center ≥ 0.5 CONNECTS the high diagonal, center < 0.5 isolates,
    // CONSISTENTLY for both saddle codes. 2×2 planes, both diagonals,
    // both regimes.
    let saddle = |tl: f32, tr: f32, bl: f32, br: f32| -> usize {
        let mut plane = CoveragePlane::new(IRect::new(0, 0, 2, 2));
        plane.v = vec![tl, tr, bl, br];
        trace_iso_contours(&plane).len()
    };
    // TR+BL high, center 0.65 ≥ 0.5 → connected: ONE contour.
    assert_eq!(saddle(0.4, 0.9, 0.9, 0.4), 1, "TR+BL connects at avg 0.65");
    // TR+BL high, center 0.35 < 0.5 → isolated: TWO contours.
    assert_eq!(saddle(0.1, 0.6, 0.6, 0.1), 2, "TR+BL isolates at avg 0.35");
    // TL+BR mirror, both regimes — must agree with its sibling.
    assert_eq!(saddle(0.9, 0.4, 0.4, 0.9), 1, "TL+BR connects at avg 0.65");
    assert_eq!(saddle(0.6, 0.1, 0.1, 0.6), 2, "TL+BR isolates at avg 0.35");
}

#[test]
fn exact_half_samples_never_mint_degenerate_loops() {
    // Review MINOR-5: a figure-8 pinch whose center sample is exactly
    // 0.5 puts crossing vertices ON the sample — the tracer must emit
    // only area-bounding loops (≥ 3 distinct vertices, nonzero area).
    let mut plane = CoveragePlane::new(IRect::new(0, 0, 3, 3));
    plane.v = vec![1.0, 0.0, 1.0, 0.0, 0.5, 0.0, 1.0, 0.0, 1.0];
    let loops = trace_iso_contours(&plane);
    for l in &loops {
        assert!(l.len() >= 3, "no degenerate loops: {l:?}");
        let a: f64 = l
            .iter()
            .zip(l.iter().cycle().skip(1))
            .map(|(p, q)| p.x * q.y - q.x * p.y)
            .sum::<f64>()
            .abs();
        assert!(a > 0.0, "every loop bounds area");
    }
}

#[test]
fn the_true_dirty_inflation_bound_is_pinned_and_the_contract_claim_divergence_recorded() {
    // Review MAJOR-3, NARROWED by the distance-band rewrite: with
    // round joins the band NEVER exceeds w/2 from the source, so
    // `Center` now CONFORMS to §40.5's claim (D = source ⊕ ⌈w/2⌉, plus
    // the one-pixel AA ramp). The remaining divergence is `Outside`:
    // it extends the FULL width past the fill edge — pinned here; the
    // §40.5 reconciliation stays ESCALATED (fold-in register +
    // QUESTIONS-FOR-BILL).
    let p = rect_path(20.0, 20.0, 84.0, 52.0);
    // Center w=4: rect hull ⊕ (2 + 1 AA) bounds the coverage rect.
    let center = stroke_coverage(&p, 4.0, StrokePosition::Center).unwrap();
    assert!(center.rect.min.x >= 20 - 3 && center.rect.max.x <= 84 + 3);
    // A sharp vee: no join spike beyond w/2 under round joins.
    let mut vee = VectorPath::new(PathId::nil(), FillRule::NonZero);
    vee.subpaths.push(SubPath {
        segments: vec![
            CubicSeg::line(Vec2::new(20.0, 20.0), Vec2::new(50.0, 40.0)),
            CubicSeg::line(Vec2::new(50.0, 40.0), Vec2::new(20.0, 44.0)),
        ],
        closed: false,
    });
    let cov = stroke_coverage(&vee, 4.0, StrokePosition::Center).unwrap();
    assert!(
        f64::from(cov.rect.max.x) <= 50.0 + 2.0 + 1.0,
        "round joins never spike past w/2: max.x {}",
        cov.rect.max.x
    );
    // Outside w=4 reaches x = 16 — PAST the claimed 20 − ⌈2⌉ = 18:
    // the §40.5 claim divergence that remains real and escalated.
    let outside = stroke_coverage(&p, 4.0, StrokePosition::Outside).unwrap();
    assert!(
        outside.rect.min.x <= 16,
        "the Outside claim violation is real"
    );
    assert!(
        outside.rect.min.x >= 20 - 4 - 2,
        "…bounded by the full width"
    );
}

// ---------------------------------------------------------------------
// Re-review NEW-1 pins: collapse regimes under the distance band
// ---------------------------------------------------------------------

#[test]
fn thick_strokes_of_small_contours_have_no_holes() {
    // Re-review NEW-1 (the reviewer's exact repros): half-width ≥ the
    // contour inradius collapses the erosion — under the DISTANCE
    // BAND this is correct by construction: {d ≤ hw} covers the whole
    // dilated fill, no ring machinery exists to mis-wind.
    let mut tri = VectorPath::new(PathId::nil(), FillRule::NonZero);
    let (a, b, c) = (
        Vec2::new(0.0, 0.0),
        Vec2::new(5.0, 0.0),
        Vec2::new(5.0, 5.0),
    );
    tri.subpaths.push(SubPath {
        segments: vec![
            CubicSeg::line(a, b),
            CubicSeg::line(b, c),
            CubicSeg::line(c, a),
        ],
        closed: true,
    });
    // Center w=4 (hw 2 > inradius ≈1.46): the interior is FULL ink and
    // the band is the whole round-cornered dilation:
    // A + L·r + π·r² = 12.5 + 17.0711·2 + 4π.
    let cov = stroke_coverage(&tri, 4.0, StrokePosition::Center).unwrap();
    assert_eq!(cov.get(3, 1), 1.0, "interior pixel full (was ~0.015)");
    let l = 10.0 + 50.0f64.sqrt();
    let want = 12.5 + l * 2.0 + 4.0 * std::f64::consts::PI;
    // ±1.0: the linear AA ramp biases slanted/curved boundaries
    // slightly high; the STRUCTURAL asserts (interior full, healthy
    // hole kept) carry the correctness weight here.
    assert!(
        (cov.total() - want).abs() < 1.0,
        "dilated-fill mass {want}, hole healed: {}",
        cov.total()
    );
    // Inside w=2: the rim covers the WHOLE fill — exactly its area.
    let inside = stroke_coverage(&tri, 2.0, StrokePosition::Inside).unwrap();
    assert!(
        (inside.total() - 12.5).abs() < 1e-6,
        "inside band = the entire 12.5 fill: {}",
        inside.total()
    );
    // A 16-gon of radius 3 stroked at w=10: the center is INK.
    let mut gon = VectorPath::new(PathId::nil(), FillRule::NonZero);
    let pts: Vec<Vec2> = (0..16)
        .map(|i| {
            let t = std::f64::consts::TAU * f64::from(i) / 16.0;
            Vec2::new(10.0 + 3.0 * t.cos(), 10.0 + 3.0 * t.sin())
        })
        .collect();
    gon.subpaths.push(SubPath {
        segments: (0..16)
            .map(|i| CubicSeg::line(pts[i], pts[(i + 1) % 16]))
            .collect(),
        closed: true,
    });
    let cov = stroke_coverage(&gon, 10.0, StrokePosition::Center).unwrap();
    assert_eq!(cov.get(10, 10), 1.0, "polygon center is ink (was 0)");
    // CONTROL — a HEALTHY ring must keep its hole (no overpaint from
    // the guard): the same triangle at w=1 (hw 0.5 < inradius).
    let cov = stroke_coverage(&tri, 1.0, StrokePosition::Center).unwrap();
    assert_eq!(
        cov.get(3, 1),
        0.0,
        "healthy annulus keeps its interior hole"
    );
}
