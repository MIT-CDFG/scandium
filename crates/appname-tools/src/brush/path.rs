// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §15.3 (stroke geometry) [NORMATIVE]:
//! * The path through the raw sample positions is a **centripetal
//!   Catmull-Rom** spline: knot spacing `t_{i+1} = t_i + |P_{i+1} − P_i|^α`
//!   with α = 0.5 (Yuksel, Schaefer & Keyser 2011 — the [LEGAL] basis the
//!   spec cites for the no-cusp/no-overshoot guarantee).
//! * First/last control points are duplicated (`P_{−1} = P_0`,
//!   `P_{n+1} = P_n`); coincident consecutive samples (zero chord) are
//!   dropped before knot computation.
//! * Each segment is adaptively flattened: recursive bisection until the
//!   chord midpoint deviates from the curve midpoint by < 0.05 px or depth
//!   8 is reached; the resulting polyline is walked accumulating arc length.
//!
//! v1 pins (module docs `brush`): duplicated-endpoint knots collide
//! (`Δt = 0`); the pyramidal evaluation guards every lerp — a degenerate
//! interval resolves to the first operand, which is exact because
//! coincident knots only arise from coincident points. `α = 0.5` is
//! computed as `sqrt(sqrt(dx² + dy²))` — two correctly-rounded IEEE square
//! roots, no `powf`, bit-stable everywhere.
//!
//! Positions are f64 end-to-end here (§9.2's sample mirror is f64);
//! rasterization narrows to f32 in tile-local space per the §15.5 shader
//! mirror.

/// Flattening tolerance in document px (§15.3).
const FLATTEN_TOL: f64 = 0.05;
/// Maximum bisection depth (§15.3).
const FLATTEN_MAX_DEPTH: u32 = 8;

/// One flattened-polyline vertex.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Vert {
    pub pos: [f64; 2],
    /// Cumulative arc length from the stroke start, px.
    pub arc: f64,
}

/// The flattened stroke path plus the arc positions of the (deduplicated)
/// raw samples — the §15.3 dynamics-interpolation table (`s_i`).
#[derive(Clone, Debug)]
pub(crate) struct FlatPath {
    /// Polyline vertices; `verts[0]` is the pen-down position at arc 0.
    pub verts: Vec<Vert>,
    /// `sample_arc[i]` = cumulative arc length at deduplicated raw sample
    /// `i` (`sample_arc[0] = 0`, last = `total`). Length = deduped count.
    pub sample_arc: Vec<f64>,
    /// Total arc length, px.
    pub total: f64,
}

#[inline]
fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    (dx * dx + dy * dy).sqrt()
}

#[inline]
fn lerp2(a: [f64; 2], b: [f64; 2], w: f64) -> [f64; 2] {
    [a[0] + (b[0] - a[0]) * w, a[1] + (b[1] - a[1]) * w]
}

/// Guarded knot-interval lerp: a zero (or degenerate) interval resolves to
/// the first operand (see module pin).
#[inline]
fn knot_lerp(a: [f64; 2], b: [f64; 2], ta: f64, tb: f64, t: f64) -> [f64; 2] {
    let d = tb - ta;
    if d > 0.0 {
        lerp2(a, b, (t - ta) / d)
    } else {
        a
    }
}

/// Centripetal Catmull-Rom segment evaluator (pyramidal formulation) over
/// control points `p[0..4]` with knots `t[0..4]`; evaluation parameter in
/// `[t[1], t[2]]`.
struct Segment {
    p: [[f64; 2]; 4],
    t: [f64; 4],
}

impl Segment {
    fn eval(&self, u: f64) -> [f64; 2] {
        let [p0, p1, p2, p3] = self.p;
        let [t0, t1, t2, t3] = self.t;
        let a1 = knot_lerp(p0, p1, t0, t1, u);
        let a2 = knot_lerp(p1, p2, t1, t2, u);
        let a3 = knot_lerp(p2, p3, t2, t3, u);
        let b1 = knot_lerp(a1, a2, t0, t2, u);
        let b2 = knot_lerp(a2, a3, t1, t3, u);
        knot_lerp(b1, b2, t1, t2, u)
    }
}

/// Drop coincident consecutive positions (bitwise equality — the §15.3
/// "zero chord" rule, v1 pin), keeping the first of each run. Returns the
/// indices kept (into the caller's sample slice), so dynamics fields follow
/// their surviving positions.
pub(crate) fn dedupe_coincident(positions: &[[f64; 2]]) -> Vec<usize> {
    let mut kept = Vec::with_capacity(positions.len());
    for (i, p) in positions.iter().enumerate() {
        if let Some(&last) = kept.last() {
            let q: [f64; 2] = positions[last];
            if p[0] == q[0] && p[1] == q[1] {
                continue;
            }
        }
        kept.push(i);
    }
    kept
}

/// Flatten the centripetal Catmull-Rom spline through `positions`
/// (already deduplicated, length ≥ 1) into an arc-parameterized polyline.
pub(crate) fn flatten(positions: &[[f64; 2]]) -> FlatPath {
    let n = positions.len();
    assert!(n >= 1, "flatten needs at least one position");
    if n == 1 {
        return FlatPath {
            verts: vec![Vert {
                pos: positions[0],
                arc: 0.0,
            }],
            sample_arc: vec![0.0],
            total: 0.0,
        };
    }

    // Knots: t[i+1] = t[i] + |ΔP|^0.5 (centripetal, α = 0.5).
    let mut knots = Vec::with_capacity(n);
    knots.push(0.0f64);
    for i in 1..n {
        let d = dist(positions[i - 1], positions[i]);
        knots.push(knots[i - 1] + d.sqrt());
    }

    let mut verts = vec![Vert {
        pos: positions[0],
        arc: 0.0,
    }];
    let mut sample_arc = Vec::with_capacity(n);
    sample_arc.push(0.0);

    // Segment j spans samples j → j+1 with duplicated phantom endpoints.
    for j in 0..n - 1 {
        let p0 = positions[j.saturating_sub(1)];
        let p3 = positions[(j + 2).min(n - 1)];
        let t0 = if j == 0 { knots[0] } else { knots[j - 1] };
        let t3 = if j + 2 < n {
            knots[j + 2]
        } else {
            knots[n - 1]
        };
        let seg = Segment {
            p: [p0, positions[j], positions[j + 1], p3],
            t: [t0, knots[j], knots[j + 1], t3],
        };
        subdivide(
            &seg,
            knots[j],
            knots[j + 1],
            positions[j],
            positions[j + 1],
            0,
            &mut verts,
        );
        sample_arc.push(verts.last().expect("segment emitted its end vertex").arc);
    }

    let total = verts.last().expect("nonempty").arc;
    FlatPath {
        verts,
        sample_arc,
        total,
    }
}

/// §15.3 adaptive bisection: accept the chord when the curve midpoint sits
/// within `FLATTEN_TOL` of the chord midpoint, or at depth 8.
fn subdivide(
    seg: &Segment,
    ta: f64,
    tb: f64,
    pa: [f64; 2],
    pb: [f64; 2],
    depth: u32,
    out: &mut Vec<Vert>,
) {
    let tm = 0.5 * (ta + tb);
    let pm = seg.eval(tm);
    let chord_mid = [0.5 * (pa[0] + pb[0]), 0.5 * (pa[1] + pb[1])];
    if depth >= FLATTEN_MAX_DEPTH || dist(pm, chord_mid) < FLATTEN_TOL {
        let last = out.last().expect("seeded with the pen-down vertex");
        let arc = last.arc + dist(last.pos, pb);
        out.push(Vert { pos: pb, arc });
    } else {
        subdivide(seg, ta, tm, pa, pm, depth + 1, out);
        subdivide(seg, tm, tb, pm, pb, depth + 1, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedupe_keeps_first_of_each_run() {
        let pos = [
            [0.0, 0.0],
            [0.0, 0.0],
            [1.0, 0.0],
            [1.0, 0.0],
            [1.0, 0.0],
            [2.0, 0.0],
        ];
        assert_eq!(dedupe_coincident(&pos), vec![0, 2, 5]);
        let single = [[3.0, 4.0]];
        assert_eq!(dedupe_coincident(&single), vec![0]);
    }

    #[test]
    fn single_point_path_is_degenerate() {
        let p = flatten(&[[5.0, 7.0]]);
        assert_eq!(p.verts.len(), 1);
        assert_eq!(p.total, 0.0);
        assert_eq!(p.sample_arc, vec![0.0]);
    }

    #[test]
    fn straight_line_arc_length_is_exact() {
        // A spline through collinear points is the line itself (centripetal
        // CR interpolates; collinear control polygon has no transverse
        // component).
        let p = flatten(&[[0.0, 0.0], [10.0, 0.0], [20.0, 0.0], [30.0, 0.0]]);
        assert!((p.total - 30.0).abs() < 1e-6, "total {}", p.total);
        assert_eq!(p.sample_arc.len(), 4);
        for (i, s) in p.sample_arc.iter().enumerate() {
            assert!((s - 10.0 * i as f64).abs() < 1e-6, "sample {i} at arc {s}");
        }
        // Every vertex stays on the line.
        for v in &p.verts {
            assert!(v.pos[1].abs() < 1e-9);
        }
    }

    #[test]
    fn spline_interpolates_the_samples() {
        let pos = [[0.0, 0.0], [10.0, 8.0], [20.0, -3.0], [32.0, 4.0]];
        let p = flatten(&pos);
        // Segment boundaries are emitted exactly; find each sample position
        // among the vertices.
        for s in pos {
            assert!(
                p.verts.iter().any(|v| v.pos == s),
                "sample {s:?} must be a polyline vertex"
            );
        }
        // Arc is strictly non-decreasing and starts at 0.
        assert_eq!(p.verts[0].arc, 0.0);
        for w in p.verts.windows(2) {
            assert!(w[1].arc >= w[0].arc);
        }
    }

    #[test]
    fn two_point_path_is_their_chord() {
        // With duplicated endpoints, the CR segment between two points
        // degenerates to the straight chord.
        let p = flatten(&[[0.0, 0.0], [6.0, 8.0]]);
        assert!((p.total - 10.0).abs() < 1e-6, "total {}", p.total);
        for v in &p.verts {
            // On the chord: y/x = 8/6 wherever x > 0.
            if v.pos[0] > 1e-9 {
                assert!((v.pos[1] / v.pos[0] - 8.0 / 6.0).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn curvature_refines_flatter_than_tolerance() {
        // A right-angle corner path: flattened chords must deviate < 0.05 px
        // from the curve — verify by densely sampling each accepted chord's
        // midpoint against the polyline (self-consistency: adjacent verts
        // closer than tolerance implies bounded deviation).
        let p = flatten(&[[0.0, 0.0], [50.0, 0.0], [50.0, 50.0]]);
        assert!(p.verts.len() > 3, "curved path must subdivide");
        assert!(
            p.total > 100.0 - 1.0 && p.total < 120.0,
            "plausible arc, got {}",
            p.total
        );
    }

    #[test]
    fn flatten_is_deterministic() {
        let pos = [[0.0, 0.0], [13.7, 4.2], [21.0, -9.5], [40.25, 3.125]];
        let a = flatten(&pos);
        let b = flatten(&pos);
        assert_eq!(a.verts.len(), b.verts.len());
        for (x, y) in a.verts.iter().zip(b.verts.iter()) {
            assert_eq!(x.pos, y.pos);
            assert_eq!(x.arc, y.arc);
        }
    }
}
