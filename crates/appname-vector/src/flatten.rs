// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §20.1 "Length and point-at-length" — recursive subdivision
//! (De Casteljau split at t = 0.5) until flat; flatness test =
//! `max(dist(c1, chord p0p1), dist(c2, chord)) ≤ ε`; max depth 24.
//! Tolerance discipline [NORMATIVE tolerances named in §20.1/§20.3]:
//! measurement ε = 0.01 doc px, display flattening 0.1 device px,
//! booleans 0.05 doc px (zoom-independent, so results are stable and
//! serializable). Arc length ≈ Σ chord lengths of the flattened polyline;
//! `point_at_length` = cumulative table + binary search + lerp — the ONE
//! primitive serving dash walking (§20.2), text-on-path (§23.3), and the
//! crop spiral overlay.
//!
//! Determinism: splits only ever use exact midpoint halving (add +
//! multiply by 0.5 — no transcendental, no FMA variance), recursion order
//! is left-then-right, and tolerances are compile-time constants — the
//! same path flattens to the identical polyline on every platform.

use crate::geom::Vec2;
use crate::path::{CubicSeg, SubPath, VectorPath};

/// §20.1 measurement tolerance (doc px): arc length / point-at-length.
pub const TOL_MEASURE: f64 = 0.01;
/// §20.1 display flattening tolerance (device px); also §20.2 fill
/// rasterization step 1.
pub const TOL_DISPLAY: f64 = 0.1;
/// §20.3 [NORMATIVE] boolean flattening tolerance (doc px, zoom-free).
pub const TOL_BOOLEAN: f64 = 0.05;
/// §20.1 maximum subdivision depth.
pub const MAX_SPLIT_DEPTH: u32 = 24;

/// §20.1 flatness: both interior control points within `tol` of the chord.
/// `distance_to_segment` (not infinite line) keeps the test meaningful on
/// degenerate chords (p0 ≈ p1 loops) and is conservative — never less
/// strict than the spec's chord distance.
fn is_flat(seg: &CubicSeg, tol: f64) -> bool {
    seg.c1.distance_to_segment(seg.p0, seg.p1) <= tol
        && seg.c2.distance_to_segment(seg.p0, seg.p1) <= tol
}

fn flatten_rec(seg: &CubicSeg, tol: f64, depth: u32, out: &mut Vec<Vec2>) {
    if depth >= MAX_SPLIT_DEPTH || is_flat(seg, tol) {
        out.push(seg.p1);
        return;
    }
    let (a, b) = seg.split_half();
    flatten_rec(&a, tol, depth + 1, out);
    flatten_rec(&b, tol, depth + 1, out);
}

/// Flatten one segment, appending every vertex AFTER `p0` (p1 inclusive).
pub fn flatten_seg_into(seg: &CubicSeg, tol: f64, out: &mut Vec<Vec2>) {
    flatten_rec(seg, tol, 0, out);
}

/// Flatten a subpath to a polyline. The first vertex is `segments[0].p0`;
/// a closed subpath's final vertex equals its first (the §20.1 closure
/// invariant carries through bitwise).
pub fn flatten_subpath(sub: &SubPath, tol: f64) -> Vec<Vec2> {
    let mut out = Vec::new();
    let Some(first) = sub.segments.first() else {
        return out;
    };
    out.push(first.p0);
    for seg in &sub.segments {
        flatten_seg_into(seg, tol, &mut out);
    }
    out
}

/// Flatten every subpath (order preserved — deterministic).
pub fn flatten_path(path: &VectorPath, tol: f64) -> Vec<Vec<Vec2>> {
    path.subpaths
        .iter()
        .map(|s| flatten_subpath(s, tol))
        .collect()
}

/// §20.1 cumulative arc-length table over one flattened subpath, built at
/// `TOL_MEASURE` ("once per edit" — the caller owns caching).
#[derive(Clone, Debug)]
pub struct ArcLengthTable {
    pts: Vec<Vec2>,
    /// cum[i] = polyline length from pts[0] to pts[i]; cum[0] = 0.
    cum: Vec<f64>,
}

impl ArcLengthTable {
    pub fn new(sub: &SubPath) -> ArcLengthTable {
        let pts = flatten_subpath(sub, TOL_MEASURE);
        let mut cum = Vec::with_capacity(pts.len());
        let mut acc = 0.0;
        cum.push(0.0);
        for w in pts.windows(2) {
            acc += w[0].distance(w[1]);
            cum.push(acc);
        }
        ArcLengthTable { pts, cum }
    }

    /// Total arc length (Σ chord lengths, §20.1).
    pub fn total(&self) -> f64 {
        *self.cum.last().unwrap_or(&0.0)
    }

    /// Point at arc length `s`, clamped to [0, total]. Binary-searches the
    /// span and lerps within the chord (§20.1).
    pub fn point_at_length(&self, s: f64) -> Option<Vec2> {
        if self.pts.is_empty() {
            return None;
        }
        if self.pts.len() == 1 {
            return Some(self.pts[0]);
        }
        let s = s.clamp(0.0, self.total());
        // partition_point: first index with cum[i] >= s ⇒ span (i−1, i].
        let i = self.cum.partition_point(|&c| c < s).max(1);
        let (c0, c1) = (self.cum[i - 1], self.cum[i]);
        let t = if c1 > c0 { (s - c0) / (c1 - c0) } else { 0.0 };
        Some(self.pts[i - 1].lerp(self.pts[i], t))
    }

    /// Unit tangent of the chord containing arc length `s` (text-on-path
    /// §23.3 orientation groundwork). `None` for degenerate tables.
    pub fn tangent_at_length(&self, s: f64) -> Option<Vec2> {
        if self.pts.len() < 2 {
            return None;
        }
        let s = s.clamp(0.0, self.total());
        let i = self.cum.partition_point(|&c| c < s).max(1);
        (self.pts[i] - self.pts[i - 1]).normalize()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bounds::tests_support::circle_path;
    use crate::path::{CubicSeg, SubPath};

    fn v(x: f64, y: f64) -> Vec2 {
        Vec2::new(x, y)
    }

    #[test]
    fn flatten_respects_tolerance_against_dense_samples() {
        // Fixed-seed sweep: every dense curve sample must lie within tol
        // of the flattened polyline (the flatness-bound oracle).
        let mut s: u64 = 0x0123456789ABCDEF;
        let mut next = move || {
            s ^= s >> 12;
            s ^= s << 25;
            s ^= s >> 27;
            (s.wrapping_mul(0x2545F4914F6CDD1D) >> 11) as f64 / (1u64 << 53) as f64 * 64.0 - 32.0
        };
        for tol in [TOL_DISPLAY, TOL_BOOLEAN, TOL_MEASURE] {
            for _ in 0..40 {
                let seg = CubicSeg::new(
                    v(next(), next()),
                    v(next(), next()),
                    v(next(), next()),
                    v(next(), next()),
                );
                let sub = SubPath {
                    closed: false,
                    segments: vec![seg],
                };
                let poly = flatten_subpath(&sub, tol);
                assert!(poly.len() >= 2);
                for i in 0..=256 {
                    let p = seg.eval(i as f64 / 256.0);
                    let d = poly
                        .windows(2)
                        .map(|w| p.distance_to_segment(w[0], w[1]))
                        .fold(f64::INFINITY, f64::min);
                    assert!(d <= tol + 1e-9, "sample strayed {d} > tol {tol}");
                }
            }
        }
    }

    #[test]
    fn flatten_is_deterministic_bitwise() {
        let seg = CubicSeg::new(v(0.0, 0.0), v(7.3, -4.1), v(-2.2, 9.9), v(5.0, 5.0));
        let sub = SubPath {
            closed: false,
            segments: vec![seg],
        };
        let a = flatten_subpath(&sub, TOL_DISPLAY);
        let b = flatten_subpath(&sub, TOL_DISPLAY);
        assert_eq!(a.len(), b.len());
        for (p, q) in a.iter().zip(&b) {
            assert_eq!(p, q); // bitwise Vec2 equality
        }
    }

    #[test]
    fn circle_arc_length_and_point_at_length() {
        let path = circle_path(0.0, 0.0, 10.0);
        let table = ArcLengthTable::new(&path.subpaths[0]);
        let circumference = 2.0 * core::f64::consts::PI * 10.0;
        // κ-cubic circle ≈ true circle to ~2.8e-4·r; chord sum under-
        // measures by O(tol) per chord — generous combined tolerance.
        assert!(
            (table.total() - circumference).abs() < 0.05,
            "total {} vs {}",
            table.total(),
            circumference
        );
        // Quarter way round from (r, 0) heading +y (the §7.5 segment
        // order): expect ≈ (0, r).
        let q = table.point_at_length(table.total() * 0.25).unwrap();
        assert!(q.distance(v(0.0, 10.0)) < 0.05, "quarter point {q:?}");
        // Endpoints clamp.
        let z = table.point_at_length(-5.0).unwrap();
        assert!(z.distance(v(10.0, 0.0)) < 1e-12);
        let e = table.point_at_length(1e9).unwrap();
        assert!(e.distance(v(10.0, 0.0)) < 1e-9); // closed loop returns home
                                                  // Tangent at start ≈ +y.
        let t = table.tangent_at_length(0.0).unwrap();
        assert!(t.distance(v(0.0, 1.0)) < 0.05);
    }

    #[test]
    fn max_depth_terminates_on_pathological_segment() {
        // A cusp-heavy segment with huge handles still terminates.
        let seg = CubicSeg::new(v(0.0, 0.0), v(1e9, 1e9), v(-1e9, -1e9), v(1.0, 0.0));
        let sub = SubPath {
            closed: false,
            segments: vec![seg],
        };
        let poly = flatten_subpath(&sub, TOL_DISPLAY);
        // Bounded by the depth cap.
        assert!(poly.len() <= (1 << MAX_SPLIT_DEPTH) + 1);
        assert!(poly.len() > 2);
    }
}
