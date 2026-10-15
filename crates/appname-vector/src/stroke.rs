// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! `stroke_path` geometry (§40.5/§22.6 command; §20.2 stroke band
//! semantics; §13.5 selection boundary) — the Door-1 stroke arms'
//! engine half.
//!
//! SCOPE (the COMMAND's wire, not the §20.2 `VectorStroke` model): the
//! Stroke dialog carries width/position only — no dash, cap, join, or
//! width-profile knobs. PINNED ELECTIONS (fold-in register; the full
//! §20.2 tessellation with its cap/join/dash enums serves SHAPE-LAYER
//! strokes, a consumer this module does not preclude):
//! * THE BAND IS A DIRECT DISTANCE FIELD: coverage of
//!   `{p : dist(p, source) ≤ w/2}` — the §20.2 semantic itself (the
//!   Minkowski disc sweep of the curve), computed exactly rather than
//!   through offset-rail tessellation. Chosen at the NEW-1 fix round:
//!   rail polygons mis-wind whenever w/2 reaches a contour's local
//!   inradius (full radial collapse, partial neck collapse) and no
//!   rail-side guard discriminates every regime — the distance form is
//!   collapse-correct BY CONSTRUCTION in all of them.
//! * JOIN = ROUND and CAP = ROUND, as TRUE ARCS (inherent to the
//!   segment-distance metric — exact, exceeding §20.2 rule 2/3's
//!   ≤ 12° polygonized approximation; both are sanctioned §20.2 arms
//!   and the wire carries no knobs). No miter machinery: the band
//!   never exceeds w/2 from the source — no spikes, ever.
//! * FLATTEN TOLERANCE = 0.05 doc px (§20.3's zoom-independent
//!   tolerance; the distance metric runs over the flattened curve).
//! * POSITION (§20.2 rule 5's guaranteed property, achieved by SHARED
//!   COVERAGE rather than the tessellation route's booleans):
//!   `Center` = the 1× band; `Inside` = the 2×-centered band ∩ the
//!   source's fill coverage; `Outside` = the band ∩ the fill's
//!   complement — the stroke edge coincides with the fill edge by
//!   construction because band and fill share geometry. Open subpaths
//!   implicitly close for the fill (the §20.1 convention), pinned.
//! * SELECTION BOUNDARY [NORMATIVE §13.5]: the 0.5 iso-contour of the
//!   float coverage mask by MARCHING SQUARES over pixel-center
//!   samples (linear interpolation between adjacent centers; the
//!   plane is zero-padded so contours always close). Saddle cells
//!   split by the §13.5 center-sample rule (center ≥ 0.5 connects the
//!   high diagonal — both saddle codes, verified against the spec
//!   table's bit order). NO RDP SIMPLIFICATION here (§13.5 names
//!   ε = 0.75 px for the marching-ANTS overlay feed): stroking wants
//!   the exact traced geometry; the overlay consumer elects its own
//!   simplification — PINNED (review NOTE-9).
//! * EXACT-MASS ORACLES ride closed forms: for a SMOOTH closed curve
//!   the band is `w·L` (the tube formula); at polygon corners the
//!   round outer gain (π·r²/4 per right angle) and the sharp inner
//!   erosion loss (r²) do NOT cancel — the ring oracle carries the
//!   corrected form (236 + 240 + π). Open lines add `π·(w/2)²` for
//!   the two round caps.

use appname_core::engine::vector_ops::{LineCap, LineJoin, ShapeStrokeStyle, StrokePosition};
use appname_core::geom::IRect;
use appname_core::PathId;

use crate::boolean::BooleanError;
use crate::flatten::flatten_subpath;
use crate::geom::Vec2;
use crate::path::{FillRule, VectorPath};
use crate::raster::{rasterize_path, rasterize_polylines, CoveragePlane};

/// §20.3's zoom-independent flatten tolerance (module pin).
pub const STROKE_FLATTEN_TOL: f64 = 0.05;

/// One flattened source polyline for the distance metric (closed
/// contours wrap).
struct Polyline {
    pts: Vec<Vec2>,
    closed: bool,
}

impl Polyline {
    fn from_points(pts: Vec<Vec2>, closed: bool) -> Polyline {
        // Drop consecutive duplicates (degenerate segments).
        let mut clean: Vec<Vec2> = Vec::with_capacity(pts.len());
        for p in pts {
            if clean.last().is_none_or(|&q| q.distance(p) > 1e-12) {
                clean.push(p);
            }
        }
        if closed && clean.len() > 1 {
            let (first, last) = (clean[0], *clean.last().expect("nonempty"));
            if first.distance(last) <= 1e-12 {
                clean.pop();
            }
        }
        Polyline { pts: clean, closed }
    }

    fn segments(&self) -> Vec<(Vec2, Vec2)> {
        let n = self.pts.len();
        if n < 2 {
            return Vec::new(); // a point strokes nothing (module pin)
        }
        let count = if self.closed { n } else { n - 1 };
        (0..count)
            .map(|i| (self.pts[i], self.pts[(i + 1) % n]))
            .collect()
    }
}

/// Exact point-to-segment distance (round caps/joins are INHERENT to
/// the segment-distance metric — the module's join/cap pins).
fn seg_distance(p: Vec2, a: Vec2, b: Vec2) -> f64 {
    let (abx, aby) = (b.x - a.x, b.y - a.y);
    let len2 = abx * abx + aby * aby;
    let t = if len2 <= 0.0 {
        0.0
    } else {
        (((p.x - a.x) * abx + (p.y - a.y) * aby) / len2).clamp(0.0, 1.0)
    };
    let (dx, dy) = (p.x - (a.x + abx * t), p.y - (a.y + aby * t));
    (dx * dx + dy * dy).sqrt()
}

/// The §20.2 stroke band computed as a DIRECT DISTANCE FIELD:
/// coverage of `{p : dist(p, source) ≤ half_width}` sampled at pixel
/// centers with a one-pixel linear AA ramp
/// (`cov = clamp(hw − d + ½, 0, 1)`).
///
/// WHY THIS FORM (re-review NEW-1's root cause retired): offset-rail
/// polygon bands mis-wind whenever the half-width reaches a contour's
/// local inradius — full radial collapse, partial neck collapse, and
/// every mixture — and no rail-side guard discriminates all regimes
/// (vertex distance and global orientation were both probed and
/// defeated). The distance form IS the §20.2 semantic (the band is
/// the Minkowski disc sweep of the curve), so every collapse regime
/// is correct BY CONSTRUCTION: erosion vanishes exactly where the
/// inradius does. Exactness: for straight edges the linear AA ramp
/// integrates to the true area (the tube formula `2·hw·L` for closed
/// curves — round join gains and losses cancel exactly).
///
/// PERF: O(pixels × segments) with a per-row segment prefilter; wide
/// strokes on long paths pay real cost — an SDF tile grid is the
/// recorded perf item.
fn distance_band_coverage(polys: &[Polyline], half_width: f64) -> CoveragePlane {
    let mut segs: Vec<(Vec2, Vec2)> = Vec::new();
    for poly in polys {
        segs.extend(poly.segments());
    }
    if segs.is_empty() || half_width <= 0.0 {
        return CoveragePlane::new(IRect::EMPTY);
    }
    let (mut min_x, mut min_y) = (f64::INFINITY, f64::INFINITY);
    let (mut max_x, mut max_y) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for &(a, b) in &segs {
        min_x = min_x.min(a.x).min(b.x);
        min_y = min_y.min(a.y).min(b.y);
        max_x = max_x.max(a.x).max(b.x);
        max_y = max_y.max(a.y).max(b.y);
    }
    let pad = half_width + 1.0;
    let rect = IRect::new(
        (min_x - pad).floor() as i32,
        (min_y - pad).floor() as i32,
        (max_x + pad).ceil() as i32,
        (max_y + pad).ceil() as i32,
    );
    let mut plane = CoveragePlane::new(rect);
    let w = rect.width();
    let reach = half_width + 1.0;
    for y in rect.min.y..rect.max.y {
        let py = f64::from(y) + 0.5;
        // Row prefilter: segments whose y-span reaches this row.
        let row: Vec<(Vec2, Vec2)> = segs
            .iter()
            .copied()
            .filter(|&(a, b)| a.y.min(b.y) - reach <= py && py <= a.y.max(b.y) + reach)
            .collect();
        if row.is_empty() {
            continue;
        }
        for x in rect.min.x..rect.max.x {
            let p = Vec2::new(f64::from(x) + 0.5, py);
            let mut d = f64::INFINITY;
            for &(a, b) in &row {
                d = d.min(seg_distance(p, a, b));
                if d <= half_width - 0.5 {
                    break; // fully covered — no closer answer matters
                }
            }
            let cov = (half_width - d + 0.5).clamp(0.0, 1.0);
            if cov > 0.0 {
                plane.v[((y - rect.min.y) * w + (x - rect.min.x)) as usize] = cov as f32;
            }
        }
    }
    plane
}

/// Pixel-wise position combine over the band's rect (§13.1 coverage
/// intersection): `Inside` = band ∩ fill; `Outside` = band ∩ ¬fill —
/// the band and the fill SHARE coverage geometry, so the stroke edge
/// coincides with the fill edge by construction (the §20.2 position
/// rule's guaranteed property, achieved without the tessellation
/// route's booleans).
fn combine_position(
    band: CoveragePlane,
    fill: &CoveragePlane,
    position: StrokePosition,
) -> CoveragePlane {
    let mut out = band;
    let w = out.rect.width();
    for y in out.rect.min.y..out.rect.max.y {
        for x in out.rect.min.x..out.rect.max.x {
            let idx = ((y - out.rect.min.y) * w + (x - out.rect.min.x)) as usize;
            let f = fill.get(x, y).clamp(0.0, 1.0);
            let gate = match position {
                StrokePosition::Inside => f,
                StrokePosition::Outside => 1.0 - f,
                StrokePosition::Center => 1.0,
            };
            out.v[idx] = out.v[idx].min(gate);
        }
    }
    out
}

/// §40.5 `stroke_path` coverage for a PATH source (§20.2 position
/// rule): `Center` = the 1×-width distance band; `Inside`/`Outside` =
/// the 2×-width centered band gated by the source's fill coverage
/// (open subpaths implicitly close for the fill — the §20.1
/// convention, module pin).
pub fn stroke_coverage(
    path: &VectorPath,
    width: f64,
    position: StrokePosition,
) -> Result<CoveragePlane, BooleanError> {
    let polys: Vec<Polyline> = path
        .subpaths
        .iter()
        .map(|sub| Polyline::from_points(flatten_subpath(sub, STROKE_FLATTEN_TOL), sub.closed))
        .collect();
    match position {
        StrokePosition::Center => Ok(distance_band_coverage(&polys, width / 2.0)),
        StrokePosition::Inside | StrokePosition::Outside => {
            // The 2×-width centered band = {d ≤ width}.
            let band = distance_band_coverage(&polys, width);
            let fill = rasterize_path(path, STROKE_FLATTEN_TOL);
            Ok(combine_position(band, &fill, position))
        }
    }
}

// ---------------------------------------------------------------------
// §20.2 SHAPE-LAYER stroke band — cap/join/align, the layer.stroke model
// ---------------------------------------------------------------------
//
// WHY A SECOND MACHINE (pinned, and deliberately NOT a replacement for
// the distance band above): the §40.5 `stroke_path` COMMAND wire carries
// width + position and nothing else, so its band is round-join/round-cap
// by construction and the exact distance form is both correct and
// cheapest. The §20.2 `layer.stroke` MODEL carries cap/join/miterLimit,
// and a miter apex or a butt chord is NOT a level set of the distance to
// the source — no threshold on `dist(p, curve)` produces a corner spike
// or a squared end. Serving those knobs therefore needs geometry, and the
// two doors stay separate: `stroke_path`'s committed pixels are untouched
// by this module addition.
//
// THE CONSTRUCTION — a union of INDEPENDENTLY-ORIENTED CONVEX PIECES
// rasterized NonZero, which is exactly what §20.2 rule 2 licenses
// ("overlap is harmless under nonzero fill of the stroke polygon"):
//
// * each flattened segment contributes its own closed ±h quad;
// * each interior vertex contributes its own OUTER join piece (round =
//   the full disc at the vertex — its inner half merely overlaps the two
//   quads; miter = the kite v→rail₀→apex→rail₁ with the §20.2 rule-2
//   `miter_length / width > limit` bevel fallback; bevel = the triangle);
// * each open end contributes its own cap piece (butt = nothing; square =
//   the h-extension quad; round = the full disc).
//
// Every piece is pushed with POSITIVE shoelace orientation, so winding
// only ever ACCUMULATES across overlaps (≥ 1 inside the union, 0 outside)
// and `nonzero: a = min(1, |w|)` returns the union exactly. This is the
// root-cause answer to the offset-rail collapse that drove the distance
// election for the command arm (module header, NEW-1): there are no
// rails, so there is no reversed inner rail to cancel a covered region to
// zero — a half-width past the local inradius simply piles more winding
// on the same pixels. Interior piece boundaries are seam-free for the
// same reason: they separate winding 1 from winding 2, and both clamp to
// full coverage.
//
// Round pieces polygonize at ≤ 12° per §20.2 rules 2/3 (the spec's own
// sanctioned approximation), inscribed — sagitta ≤ r·(1 − cos 6°) ≈
// 0.0055·r doc px.

/// §20.2 rules 2/3: round joins/caps subdivide into ≤ 12° segments.
const ARC_MAX_STEP_RAD: f64 = 12.0 * std::f64::consts::PI / 180.0;

/// Below this the two incident directions are treated as collinear (no
/// join geometry is missing) or as an exact reversal (the miter apex runs
/// to infinity → §20.2 rule 2's bevel fallback, which is degenerate for a
/// reversal and contributes nothing the two quads do not already cover).
const JOIN_EPS: f64 = 1e-12;

/// Push one piece with POSITIVE shoelace orientation (the union
/// invariant); degenerate pieces (< 3 vertices or zero area) are dropped.
fn push_piece(out: &mut Vec<Vec<Vec2>>, mut poly: Vec<Vec2>) {
    if poly.len() < 3 {
        return;
    }
    let n = poly.len();
    let mut twice_area = 0.0;
    for i in 0..n {
        let (p, q) = (poly[i], poly[(i + 1) % n]);
        twice_area += p.x * q.y - q.x * p.y;
    }
    if twice_area.abs() <= JOIN_EPS {
        return;
    }
    if twice_area < 0.0 {
        poly.reverse();
    }
    out.push(poly);
}

/// A closed disc polygonized at ≤ 12° (round joins and round caps).
fn disc(c: Vec2, r: f64) -> Vec<Vec2> {
    let steps = ((std::f64::consts::TAU / ARC_MAX_STEP_RAD).ceil() as usize).max(3);
    (0..steps)
        .map(|i| {
            let t = std::f64::consts::TAU * i as f64 / steps as f64;
            Vec2::new(c.x + r * t.cos(), c.y + r * t.sin())
        })
        .collect()
}

/// Unit LEFT normal of a unit direction (y-down doc space — the sign is
/// only ever used symmetrically, so the handedness convention is free).
fn left_normal(d: Vec2) -> Vec2 {
    Vec2::new(-d.y, d.x)
}

/// The §20.2 stroke pieces for one flattened polyline at half-width `h`.
fn polyline_pieces(
    poly: &Polyline,
    h: f64,
    cap: LineCap,
    join: LineJoin,
    out: &mut Vec<Vec<Vec2>>,
) {
    let pts = &poly.pts;
    let n = pts.len();
    if n < 2 {
        // A point strokes nothing — the module's standing pin (shared
        // with the command arm's `Polyline::segments`), recorded rather
        // than silently differing: SVG would draw a round-cap dot here.
        return;
    }
    let segs = poly.segments();
    // Segment bodies.
    for &(a, b) in &segs {
        let Some(d) = Vec2::new(b.x - a.x, b.y - a.y).normalize() else {
            continue;
        };
        let nrm = left_normal(d);
        let off = Vec2::new(nrm.x * h, nrm.y * h);
        push_piece(
            out,
            vec![
                Vec2::new(a.x + off.x, a.y + off.y),
                Vec2::new(b.x + off.x, b.y + off.y),
                Vec2::new(b.x - off.x, b.y - off.y),
                Vec2::new(a.x - off.x, a.y - off.y),
            ],
        );
    }
    // Joins: every vertex that has BOTH an incoming and an outgoing
    // segment (all of them when the contour is closed).
    let join_count = if poly.closed { n } else { n.saturating_sub(2) };
    for k in 0..join_count {
        let vi = if poly.closed { k } else { k + 1 };
        let prev = pts[(vi + n - 1) % n];
        let v = pts[vi];
        let next = pts[(vi + 1) % n];
        let (Some(d0), Some(d1)) = (
            Vec2::new(v.x - prev.x, v.y - prev.y).normalize(),
            Vec2::new(next.x - v.x, next.y - v.y).normalize(),
        ) else {
            continue;
        };
        let cross = d0.cross(d1);
        if cross.abs() <= JOIN_EPS {
            // Collinear: a CONTINUATION (d0·d1 > 0) needs no join piece —
            // the two quads already abut. An exact REVERSAL (a hairpin,
            // d0·d1 < 0) is the one degenerate case that still owes
            // geometry: miter's apex runs to infinity and §20.2 rule 2
            // falls back to a bevel, which for a reversal is a zero-area
            // triangle (nothing) — but a ROUND join owes the half-disc
            // turnaround, which no quad covers. The disc is the same piece
            // the non-degenerate arm pushes.
            if join == LineJoin::Round && d0.dot(d1) < 0.0 {
                push_piece(out, disc(v, h));
            }
            continue;
        }
        if join == LineJoin::Round {
            push_piece(out, disc(v, h));
            continue;
        }
        // OUTER side = away from the turn. The turn goes toward
        // left_normal(d0) exactly when cross > 0.
        let s = -cross.signum();
        let (n0, n1) = (left_normal(d0), left_normal(d1));
        let r0 = Vec2::new(v.x + s * n0.x * h, v.y + s * n0.y * h);
        let r1 = Vec2::new(v.x + s * n1.x * h, v.y + s * n1.y * h);
        // apex = rail₀ ∩ rail₁ (the lines through r0 ∥ d0 and r1 ∥ d1);
        // §20.2 rule 2 / SVG: fall back to BEVEL once
        // `miter_length / width > limit`, and miter_length = 2·|apex − v|
        // against width = 2·h, so the ratio is |apex − v| / h.
        let miter = match join {
            LineJoin::Miter(limit) => {
                let t = Vec2::new(r1.x - r0.x, r1.y - r0.y).cross(d1) / cross;
                let apex = Vec2::new(r0.x + d0.x * t, r0.y + d0.y * t);
                (apex.is_finite() && apex.distance(v) <= limit.max(0.0) * h).then_some(apex)
            }
            _ => None,
        };
        match miter {
            Some(apex) => push_piece(out, vec![v, r0, apex, r1]),
            None => push_piece(out, vec![v, r0, r1]),
        }
    }
    // Caps on the two open ends.
    if !poly.closed {
        let ends = [
            (pts[0], Vec2::new(pts[0].x - pts[1].x, pts[0].y - pts[1].y)),
            (
                pts[n - 1],
                Vec2::new(pts[n - 1].x - pts[n - 2].x, pts[n - 1].y - pts[n - 2].y),
            ),
        ];
        for (p, outward) in ends {
            let Some(d) = outward.normalize() else {
                continue;
            };
            match cap {
                LineCap::Butt => {} // the chord IS the quad's own edge
                LineCap::Round => push_piece(out, disc(p, h)),
                LineCap::Square => {
                    let nrm = left_normal(d);
                    let (ox, oy) = (nrm.x * h, nrm.y * h);
                    let (ex, ey) = (d.x * h, d.y * h);
                    push_piece(
                        out,
                        vec![
                            Vec2::new(p.x + ox, p.y + oy),
                            Vec2::new(p.x + ox + ex, p.y + oy + ey),
                            Vec2::new(p.x - ox + ex, p.y - oy + ey),
                            Vec2::new(p.x - ox, p.y - oy),
                        ],
                    );
                }
            }
        }
    }
}

/// The §20.2 band of a shape layer's `VectorStroke` over its path.
///
/// `Center` is the 1×-width band directly; `Inside`/`Outside` build the
/// 2×-width CENTERED band and gate it by the source's own fill coverage —
/// §20.2 rule 5's guaranteed property (the stroke edge coincides with the
/// fill edge) reached through the shared-coverage route this module
/// already pinned for the command arm, rather than through the rule's
/// sketched §20.3 booleans. Open subpaths implicitly close for the fill
/// (the §20.1 convention, module pin).
pub fn shape_stroke_coverage(path: &VectorPath, style: ShapeStrokeStyle) -> CoveragePlane {
    if style.width <= 0.0 || !style.width.is_finite() {
        return CoveragePlane::new(IRect::EMPTY);
    }
    let polys: Vec<Polyline> = path
        .subpaths
        .iter()
        .map(|sub| Polyline::from_points(flatten_subpath(sub, STROKE_FLATTEN_TOL), sub.closed))
        .collect();
    let h = match style.align {
        StrokePosition::Center => style.width / 2.0,
        StrokePosition::Inside | StrokePosition::Outside => style.width,
    };
    let mut pieces: Vec<Vec<Vec2>> = Vec::new();
    for poly in &polys {
        polyline_pieces(poly, h, style.cap, style.join, &mut pieces);
    }
    if pieces.is_empty() {
        return CoveragePlane::new(IRect::EMPTY);
    }
    let band = rasterize_polylines(&pieces, FillRule::NonZero);
    match style.align {
        StrokePosition::Center => band,
        StrokePosition::Inside | StrokePosition::Outside => {
            let fill = rasterize_path(path, STROKE_FLATTEN_TOL);
            combine_position(band, &fill, style.align)
        }
    }
}

// ---------------------------------------------------------------------
// §13.5 selection boundary — marching squares at 0.5
// ---------------------------------------------------------------------

/// Trace the 0.5 iso-contours of a coverage plane [NORMATIVE §13.5:
/// "the iso-contour at 0.5 of the mask, extracted by marching
/// squares"]. Samples sit at pixel centers; the plane is implicitly
/// zero-padded so every contour closes; vertices interpolate linearly
/// between adjacent samples; saddles split by the average-sample rule
/// (module pin). Output loops are closed polylines in doc space.
pub fn trace_iso_contours(plane: &CoveragePlane) -> Vec<Vec<Vec2>> {
    const T: f32 = 0.5;
    let (w, h) = (plane.rect.width(), plane.rect.height());
    if w <= 0 || h <= 0 {
        return Vec::new();
    }
    // Padded sample grid: (w+2) × (h+2), zeros around the plane.
    let (gw, gh) = (w + 2, h + 2);
    let sample = |gx: i32, gy: i32| -> f32 {
        let (x, y) = (gx - 1, gy - 1);
        if x < 0 || y < 0 || x >= w || y >= h {
            0.0
        } else {
            plane.v[(y * w + x) as usize]
        }
    };
    // Doc-space position of grid sample (gx, gy): pixel centers.
    let pos = |gx: i32, gy: i32| -> Vec2 {
        Vec2::new(
            f64::from(plane.rect.min.x + gx - 1) + 0.5,
            f64::from(plane.rect.min.y + gy - 1) + 0.5,
        )
    };
    let lerp = |a: Vec2, va: f32, b: Vec2, vb: f32| -> Vec2 {
        let t = if (vb - va).abs() <= f32::EPSILON {
            0.5
        } else {
            f64::from((T - va) / (vb - va))
        };
        Vec2::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
    };
    // Segment soup keyed by cell; then stitched into loops. Edge ids:
    // for cell (cx, cy): 0 = top, 1 = right, 2 = bottom, 3 = left —
    // shared edges get identical interpolated vertices by construction
    // (same sample pair), so stitching matches bitwise.
    let mut segments: Vec<(Vec2, Vec2)> = Vec::new();
    for cy in 0..gh - 1 {
        for cx in 0..gw - 1 {
            let (v00, v10) = (sample(cx, cy), sample(cx + 1, cy));
            let (v01, v11) = (sample(cx, cy + 1), sample(cx + 1, cy + 1));
            let (p00, p10) = (pos(cx, cy), pos(cx + 1, cy));
            let (p01, p11) = (pos(cx, cy + 1), pos(cx + 1, cy + 1));
            let case = u8::from(v00 >= T)
                | u8::from(v10 >= T) << 1
                | u8::from(v11 >= T) << 2
                | u8::from(v01 >= T) << 3;
            let top = || lerp(p00, v00, p10, v10);
            let right = || lerp(p10, v10, p11, v11);
            let bottom = || lerp(p01, v01, p11, v11);
            let left = || lerp(p00, v00, p01, v01);
            // Emit directed segments (inside on the LEFT of travel) so
            // loops orient deterministically.
            match case {
                0 | 15 => {}
                1 => segments.push((left(), top())),
                2 => segments.push((top(), right())),
                3 => segments.push((left(), right())),
                4 => segments.push((right(), bottom())),
                5 => {
                    // Saddle: average-sample rule (module pin).
                    let avg = (v00 + v10 + v01 + v11) / 4.0;
                    if avg >= T {
                        segments.push((left(), bottom()));
                        segments.push((right(), top()));
                    } else {
                        segments.push((left(), top()));
                        segments.push((right(), bottom()));
                    }
                }
                6 => segments.push((top(), bottom())),
                7 => segments.push((left(), bottom())),
                8 => segments.push((bottom(), left())),
                9 => segments.push((bottom(), top())),
                10 => {
                    // TR+BL high (the spec table's 0101 in ITS bit
                    // order): center ≥ T CONNECTS the high diagonal —
                    // top–left + right–bottom; center < T isolates —
                    // top–right + left–bottom (§13.5 [NORMATIVE];
                    // review MAJOR-1: the two arms were swapped, making
                    // this saddle inconsistent with its case-5 mirror).
                    let avg = (v00 + v10 + v01 + v11) / 4.0;
                    if avg >= T {
                        segments.push((bottom(), right()));
                        segments.push((top(), left()));
                    } else {
                        segments.push((top(), right()));
                        segments.push((bottom(), left()));
                    }
                }
                11 => segments.push((bottom(), right())),
                12 => segments.push((right(), left())),
                13 => segments.push((right(), top())),
                14 => segments.push((top(), left())),
                _ => unreachable!("4-bit case"),
            }
        }
    }
    stitch_loops(segments)
}

/// Stitch directed segments into closed loops (shared cell-edge
/// vertices match bitwise; a hash on exact bits keys the adjacency).
fn stitch_loops(segments: Vec<(Vec2, Vec2)>) -> Vec<Vec<Vec2>> {
    use std::collections::HashMap;
    let key = |p: Vec2| -> (u64, u64) { (p.x.to_bits(), p.y.to_bits()) };
    // Exact-0.5 samples put crossing vertices ON the sample (lerp
    // t = 0), minting zero-length segments that would self-close into
    // degenerate loops (review MINOR-5) — drop them before stitching.
    let segments: Vec<(Vec2, Vec2)> = segments
        .into_iter()
        .filter(|(a, b)| key(*a) != key(*b))
        .collect();
    let mut by_start: HashMap<(u64, u64), Vec<usize>> = HashMap::new();
    for (i, (a, _)) in segments.iter().enumerate() {
        by_start.entry(key(*a)).or_default().push(i);
    }
    let mut used = vec![false; segments.len()];
    let mut loops = Vec::new();
    for start in 0..segments.len() {
        if used[start] {
            continue;
        }
        let mut loop_ = vec![segments[start].0];
        let mut cur = start;
        used[start] = true;
        loop {
            let end = segments[cur].1;
            if key(end) == key(loop_[0]) {
                // Degenerate guard (review MINOR-5): a closed loop
                // needs three distinct vertices to bound area.
                if loop_.len() >= 3 {
                    loops.push(loop_);
                }
                break;
            }
            loop_.push(end);
            let Some(cands) = by_start.get(&key(end)) else {
                break; // open chain: drop (cannot occur on padded grids)
            };
            let Some(&next) = cands.iter().find(|&&i| !used[i]) else {
                break;
            };
            used[next] = true;
            cur = next;
        }
    }
    loops
}

/// §40.5 `stroke_path` coverage for the SELECTION source: trace the
/// §13.5 boundary, then stroke the traced contours (the traced loops
/// ARE the fill region for the position rule).
pub fn stroke_boundary_coverage(
    plane: &CoveragePlane,
    width: f64,
    position: StrokePosition,
    _id: PathId,
) -> Result<CoveragePlane, BooleanError> {
    let contours = trace_iso_contours(plane);
    let polys: Vec<Polyline> = contours
        .iter()
        .map(|c| Polyline::from_points(c.clone(), true))
        .collect();
    match position {
        StrokePosition::Center => Ok(distance_band_coverage(&polys, width / 2.0)),
        StrokePosition::Inside | StrokePosition::Outside => {
            let band = distance_band_coverage(&polys, width);
            let fill = rasterize_polylines(&contours, FillRule::NonZero);
            Ok(combine_position(band, &fill, position))
        }
    }
}
