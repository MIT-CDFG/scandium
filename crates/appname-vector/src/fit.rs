// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! §20.3 step 4 — refit flattened polylines to cubic Béziers.
//!
//! Spec: ES §20.3 [NORMATIVE]: merge collinear runs; mark a **corner**
//! wherever the turning angle between adjacent edges exceeds **30°**;
//! least-squares-fit each corner-to-corner run by the Schneider
//! algorithm (Philip J. Schneider, "An Algorithm for Automatically
//! Fitting Digitized Curves", *Graphics Gems*, 1990 [LEGAL: public
//! literature — implemented from the published algorithm description])
//! with max error **0.1 px**, splitting at the max-error parameter on
//! failure. Corners become `Corner` anchors, interior fit joints
//! `Smooth` (anchor kinds are DERIVED from the stored segments, §20.1 —
//! this module only has to emit geometry that classifies correctly:
//! collinear handles across smooth joints, independent at corners).
//!
//! Everything is f64 (§20.3 numerical policy) and deterministic: fixed
//! iteration counts, no ambient state.

use crate::geom::Vec2;
use crate::path::CubicSeg;

/// §20.3 corner threshold: turning angle > 30°.
const CORNER_COS: f64 = 0.866_025_403_784_438_6; // cos(30°)

/// §20.3 refit tolerance (px, document space).
pub const TOL_REFIT: f64 = 0.1;

/// Newton–Raphson reparameterization attempts before splitting
/// (Schneider's iterationError escape hatch, fixed for determinism).
const MAX_REPARAM: usize = 4;

/// Split a CLOSED polyline into corner-to-corner runs (§20.3: corner =
/// turning angle > 30°). Consecutive duplicate points must already be
/// merged (the boolean engine guarantees it). Returns runs of point
/// lists, each beginning and ending at a corner; a loop with NO corner
/// comes back as one wrapped run starting at index 0 (deterministic —
/// the boolean engine's contour start is canonical).
fn split_at_corners(pts: &[Vec2]) -> Vec<Vec<Vec2>> {
    let n = pts.len();
    debug_assert!(n >= 3, "closed contour needs ≥ 3 distinct points");
    let mut corners: Vec<usize> = Vec::new();
    for i in 0..n {
        let prev = pts[(i + n - 1) % n];
        let cur = pts[i];
        let next = pts[(i + 1) % n];
        let a = (cur - prev).normalize();
        let b = (next - cur).normalize();
        match (a, b) {
            (Some(a), Some(b)) => {
                if a.dot(b) < CORNER_COS {
                    corners.push(i);
                }
            }
            // Degenerate (zero-length) edge: treat as a corner so the
            // fit never spans it.
            _ => corners.push(i),
        }
    }
    if corners.is_empty() {
        // Smooth loop (no corner anywhere): handled by the caller with
        // an averaged seam tangent — signalled by an empty run list.
        return Vec::new();
    }
    let mut runs = Vec::with_capacity(corners.len());
    for (k, &c) in corners.iter().enumerate() {
        let end = corners[(k + 1) % corners.len()];
        let mut run = Vec::new();
        let mut i = c;
        loop {
            run.push(pts[i]);
            if i == end && run.len() > 1 {
                break;
            }
            i = (i + 1) % n;
            if run.len() > n + 1 {
                debug_assert!(false, "corner run failed to close");
                break;
            }
        }
        runs.push(run);
    }
    runs
}

/// Merge collinear interior points of a run (§20.3 "merge collinear
/// runs"): a point whose turn is numerically zero adds nothing to the
/// fit and would anchor spurious joints on long straight edges.
fn merge_collinear(run: &[Vec2]) -> Vec<Vec2> {
    if run.len() <= 2 {
        return run.to_vec();
    }
    let mut out = vec![run[0]];
    for i in 1..run.len() - 1 {
        let a = run[i] - run[i - 1];
        let b = run[i + 1] - run[i];
        // Exactly-collinear same-direction points only (booleans emit
        // these along split straight edges); curved samples survive.
        if a.cross(b) == 0.0 && a.dot(b) > 0.0 {
            continue;
        }
        out.push(run[i]);
    }
    out.push(run[run.len() - 1]);
    out
}

/// Fit one corner-to-corner run to cubic segments, Schneider style.
fn fit_run(run: &[Vec2], tol: f64, out: &mut Vec<CubicSeg>) {
    let pts = merge_collinear(run);
    let n = pts.len();
    if n < 2 {
        return;
    }
    if n == 2 {
        out.push(CubicSeg::line(pts[0], pts[1]));
        return;
    }
    // Unit end tangents from the run's ends (corner-clamped per §20.3:
    // runs begin/end at corners, so tangents never cross a corner).
    let t0 = (pts[1] - pts[0]).normalize().unwrap_or(Vec2::new(1.0, 0.0));
    let t1 = (pts[n - 2] - pts[n - 1])
        .normalize()
        .unwrap_or(Vec2::new(-1.0, 0.0));
    fit_cubic(&pts, t0, t1, tol, 0, out);
}

/// Chord-length parameterization over `pts` (Schneider's initial u).
fn chord_lengths(pts: &[Vec2]) -> Vec<f64> {
    let mut u = Vec::with_capacity(pts.len());
    u.push(0.0);
    for w in pts.windows(2) {
        let last = *u.last().unwrap();
        u.push(last + w[0].distance(w[1]));
    }
    let total = *u.last().unwrap();
    if total > 0.0 {
        for v in u.iter_mut() {
            *v /= total;
        }
    }
    u
}

fn bernstein(seg: &CubicSeg, t: f64) -> Vec2 {
    seg.eval(t)
}

/// Least-squares cubic for `pts` with fixed unit end tangents
/// (Schneider's generate_bezier: solve the 2×2 normal equations for the
/// two handle lengths α₁, α₂).
fn generate_bezier(pts: &[Vec2], u: &[f64], t0: Vec2, t1: Vec2) -> CubicSeg {
    let n = pts.len();
    let first = pts[0];
    let last = pts[n - 1];
    let (mut c00, mut c01, mut c11, mut x0, mut x1) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for i in 0..n {
        let t = u[i];
        let b0 = (1.0 - t).powi(3);
        let b1 = 3.0 * t * (1.0 - t).powi(2);
        let b2 = 3.0 * t * t * (1.0 - t);
        let b3 = t.powi(3);
        let a0 = t0 * b1;
        let a1 = t1 * b2;
        let tmp = pts[i] - (first * (b0 + b1)) - (last * (b2 + b3));
        c00 += a0.dot(a0);
        c01 += a0.dot(a1);
        c11 += a1.dot(a1);
        x0 += a0.dot(tmp);
        x1 += a1.dot(tmp);
    }
    let det = c00 * c11 - c01 * c01;
    let (mut alpha0, mut alpha1) = if det.abs() > 1e-12 {
        ((x0 * c11 - x1 * c01) / det, (c00 * x1 - c01 * x0) / det)
    } else {
        (0.0, 0.0)
    };
    // Wu/Barsky heuristic fallback for degenerate or wrong-sided
    // solutions (Schneider's guard, verbatim in spirit).
    let seg_len = first.distance(last);
    let eps = 1e-6 * seg_len;
    if alpha0 < eps || alpha1 < eps {
        alpha0 = seg_len / 3.0;
        alpha1 = alpha0;
    }
    CubicSeg {
        p0: first,
        c1: first + t0 * alpha0,
        c2: last + t1 * alpha1,
        p1: last,
    }
}

/// Max squared error and its index (Schneider's compute_max_error).
fn max_error(pts: &[Vec2], seg: &CubicSeg, u: &[f64]) -> (f64, usize) {
    let mut worst = 0.0;
    let mut at = pts.len() / 2;
    for i in 1..pts.len() - 1 {
        let d = bernstein(seg, u[i]).distance(pts[i]);
        let d2 = d * d;
        if d2 > worst {
            worst = d2;
            at = i;
        }
    }
    (worst, at)
}

/// One Newton–Raphson step of parameter refinement per point
/// (Schneider's reparameterize).
fn reparameterize(pts: &[Vec2], seg: &CubicSeg, u: &mut [f64]) {
    for i in 0..pts.len() {
        let t = u[i];
        // Q(t), Q'(t), Q''(t) via the control polygon.
        let q = seg.eval(t);
        let d1 = (seg.c1 - seg.p0) * 3.0;
        let d2 = (seg.c2 - seg.c1) * 3.0;
        let d3 = (seg.p1 - seg.c2) * 3.0;
        let mt = 1.0 - t;
        let q1 = d1 * (mt * mt) + d2 * (2.0 * mt * t) + d3 * (t * t);
        let q2 = (d2 - d1) * (2.0 * mt) + (d3 - d2) * (2.0 * t);
        let diff = q - pts[i];
        let num = diff.dot(q1);
        let den = q1.dot(q1) + diff.dot(q2);
        if den.abs() > 1e-12 {
            u[i] = (t - num / den).clamp(0.0, 1.0);
        }
    }
    // Keep the parameterization monotone (Newton can locally swap).
    for i in 1..u.len() {
        if u[i] < u[i - 1] {
            u[i] = u[i - 1];
        }
    }
}

/// Schneider's fit_cubic: fit, reparameterize on near-miss, split at
/// the worst point on failure. `depth` caps recursion for determinism
/// and stack safety.
fn fit_cubic(pts: &[Vec2], t0: Vec2, t1: Vec2, tol: f64, depth: usize, out: &mut Vec<CubicSeg>) {
    let n = pts.len();
    if n == 2 {
        out.push(CubicSeg::line(pts[0], pts[1]));
        return;
    }
    let mut u = chord_lengths(pts);
    let mut seg = generate_bezier(pts, &u, t0, t1);
    let (mut err2, mut worst) = max_error(pts, &seg, &u);
    let tol2 = tol * tol;
    if err2 <= tol2 {
        out.push(seg);
        return;
    }
    // Near miss: iterate reparameterization (Schneider uses err < 4·tol²
    // as the "worth iterating" band).
    if err2 <= 4.0 * tol2 {
        for _ in 0..MAX_REPARAM {
            reparameterize(pts, &seg, &mut u);
            seg = generate_bezier(pts, &u, t0, t1);
            let (e2, w) = max_error(pts, &seg, &u);
            err2 = e2;
            worst = w;
            if err2 <= tol2 {
                out.push(seg);
                return;
            }
        }
    }
    if depth >= 32 {
        // Give up splitting: emit the polyline exactly (never silently
        // exceed the 0.1 px contract).
        for w in pts.windows(2) {
            out.push(CubicSeg::line(w[0], w[1]));
        }
        return;
    }
    // Split at the max-error point; center tangent per Schneider.
    let worst = worst.clamp(1, n - 2);
    let vc = (pts[worst - 1] - pts[worst + 1])
        .normalize()
        .unwrap_or(Vec2::new(1.0, 0.0));
    fit_cubic(&pts[..=worst], t0, vc, tol, depth + 1, out);
    fit_cubic(&pts[worst..], -vc, t1, tol, depth + 1, out);
}

/// Fit EXACTLY ONE cubic to a run with fixed unit end tangents — the
/// §20.4 delete-anchor door ("neighbors re-joined by a SINGLE fitted
/// segment (§20.3 refit, 0.1 px)"): the least-squares cubic over the
/// neighbors' sampled outline, Newton-refined, NEVER split — across a
/// deleted corner the outline legitimately deviates (removing the
/// corner is the point); 0.1 px is the fit's tolerance target where a
/// single cubic can achieve it, not a preservation guarantee.
pub(crate) fn fit_single(pts: &[Vec2], t0: Vec2, t1: Vec2) -> Option<CubicSeg> {
    if pts.len() < 2 {
        return None;
    }
    if pts.len() == 2 {
        return Some(CubicSeg::line(pts[0], pts[1]));
    }
    let mut u = chord_lengths(pts);
    let mut seg = generate_bezier(pts, &u, t0, t1);
    for _ in 0..MAX_REPARAM {
        reparameterize(pts, &seg, &mut u);
        seg = generate_bezier(pts, &u, t0, t1);
    }
    Some(seg)
}

/// Refit one CLOSED contour polyline (first point NOT repeated at the
/// end) into cubic segments per §20.3 step 4.
pub fn refit_closed_contour(pts: &[Vec2], tol: f64) -> Vec<CubicSeg> {
    let mut out = Vec::new();
    if pts.len() < 3 {
        return out;
    }
    let runs = split_at_corners(pts);
    if runs.is_empty() {
        // Corner-free smooth loop: fit the wrapped run with an AVERAGED
        // seam tangent — the centered difference across the seam point —
        // so the seam joint's handles come out collinear and the anchor
        // classifies Smooth per §20.3 "interior fit joints become
        // Smooth" (one-sided end tangents classified it Corner: review
        // finding, slice-3 round).
        let run = {
            let mut r = merge_collinear(pts);
            // merge_collinear treats the slice as open; the seam point
            // itself can't be merged away (it's the first/last), which
            // is fine — it is the deliberate anchor.
            r.push(r[0]);
            r
        };
        if run.len() < 4 {
            return out;
        }
        let seam = (run[1] - run[run.len() - 2])
            .normalize()
            .unwrap_or(Vec2::new(1.0, 0.0));
        fit_cubic(&run, seam, -seam, tol, 0, &mut out);
        return out;
    }
    for run in runs {
        fit_run(&run, tol, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flatten::flatten_seg_into;

    fn rect_pts(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<Vec2> {
        vec![
            Vec2::new(x0, y0),
            Vec2::new(x1, y0),
            Vec2::new(x1, y1),
            Vec2::new(x0, y1),
        ]
    }

    #[test]
    fn rectangle_refits_to_exact_lines() {
        let segs = refit_closed_contour(&rect_pts(0.0, 0.0, 10.0, 6.0), TOL_REFIT);
        assert_eq!(segs.len(), 4, "four corner-to-corner straight runs");
        for s in &segs {
            // Lines: control points on the chord thirds (CubicSeg::line).
            assert_eq!(s.c1, s.p0.lerp(s.p1, 1.0 / 3.0));
            assert_eq!(s.c2, s.p0.lerp(s.p1, 2.0 / 3.0));
        }
        // Chain is closed and walks the rect corners.
        for w in segs.windows(2) {
            assert_eq!(w[0].p1, w[1].p0);
        }
        assert_eq!(segs[3].p1, segs[0].p0);
    }

    #[test]
    fn collinear_interior_points_are_merged() {
        // A rect edge peppered with collinear points still refits to 4
        // exact lines (no spurious joints).
        let mut pts = Vec::new();
        for i in 0..=10 {
            pts.push(Vec2::new(f64::from(i), 0.0));
        }
        pts.push(Vec2::new(10.0, 6.0));
        pts.push(Vec2::new(0.0, 6.0));
        let segs = refit_closed_contour(&pts, TOL_REFIT);
        assert_eq!(segs.len(), 4);
    }

    #[test]
    fn flattened_kappa_arc_refits_within_tolerance() {
        // Flatten a κ-cubic quarter arc at the boolean tolerance, refit,
        // and verify the fit stays within 0.1 px of the samples.
        let k = 0.552_284_749_830_793_4;
        let r = 50.0;
        let arc = CubicSeg {
            p0: Vec2::new(r, 0.0),
            c1: Vec2::new(r, r * k),
            c2: Vec2::new(r * k, r),
            p1: Vec2::new(0.0, r),
        };
        let mut pts = Vec::new();
        flatten_seg_into(&arc, crate::flatten::TOL_BOOLEAN, &mut pts);
        // Close with two straight edges through the center (quarter pie).
        pts.push(Vec2::new(0.0, 0.0));
        let segs = refit_closed_contour(&pts, TOL_REFIT);
        assert!(!segs.is_empty());
        // Every source sample lies within tolerance of the fitted chain.
        // Distance is measured to the PIECEWISE-LINEAR dense sampling of
        // the fitted cubics (distance_to_segment over consecutive dense
        // samples) — measuring to the discrete points themselves would
        // add the ~1.2 px sample spacing to the metric and fail a fit
        // that is exactly at tolerance (observed 0.1024 px: a fit
        // accepted at ≈0.098 + point-sampling slack). The residual
        // chord-sagitta error of the dense sampling at r≈50 is ≈0.004 px,
        // covered by the 0.005 slack term.
        let dense: Vec<Vec2> = segs
            .iter()
            .flat_map(|s| (0..=64).map(move |i| s.eval(f64::from(i) / 64.0)))
            .collect();
        for p in &pts {
            let d = dense
                .windows(2)
                .map(|w| p.distance_to_segment(w[0], w[1]))
                .fold(f64::INFINITY, f64::min);
            assert!(d <= TOL_REFIT + 5e-3, "sample strayed {d} px");
        }
        // The arc region is smooth: far fewer segments than samples.
        assert!(segs.len() < pts.len() / 2);
    }

    #[test]
    fn smooth_loop_seam_classifies_smooth_not_corner() {
        // A corner-free loop (flattened κ-circle) must refit with a
        // SMOOTH seam anchor (§20.3 "interior fit joints become
        // Smooth") — the averaged seam tangent guarantees collinear
        // handles at the wrap point (review finding, slice-3 round:
        // one-sided end tangents classified it Corner).
        let k = 0.552_284_749_830_793_4;
        let r = 50.0;
        let quads = [
            [(r, 0.0), (r, r * k), (r * k, r), (0.0, r)],
            [(0.0, r), (-r * k, r), (-r, r * k), (-r, 0.0)],
            [(-r, 0.0), (-r, -r * k), (-r * k, -r), (0.0, -r)],
            [(0.0, -r), (r * k, -r), (r, -r * k), (r, 0.0)],
        ];
        let mut pts = Vec::new();
        for q in &quads {
            let seg = CubicSeg {
                p0: Vec2::new(q[0].0, q[0].1),
                c1: Vec2::new(q[1].0, q[1].1),
                c2: Vec2::new(q[2].0, q[2].1),
                p1: Vec2::new(q[3].0, q[3].1),
            };
            let mut flat = Vec::new();
            flatten_seg_into(&seg, crate::flatten::TOL_BOOLEAN, &mut flat);
            flat.pop(); // avoid duplicating each quadrant boundary
            pts.extend(flat);
        }
        let segs = refit_closed_contour(&pts, TOL_REFIT);
        assert!(segs.len() >= 2);
        // Seam anchor = first segment's start == last segment's end.
        let first = &segs[0];
        let last = segs.last().unwrap();
        assert!(
            first.p0.distance(last.p1) < 1e-9,
            "chain closes at the seam"
        );
        let kind = crate::anchor::classify(first.p0, last.c2, first.c1);
        assert_ne!(
            kind,
            crate::anchor::AnchorKind::Corner,
            "seam must not classify Corner (in {:?} out {:?})",
            last.c2 - first.p0,
            first.c1 - first.p0,
        );
        // And every interior joint stays non-corner too.
        for w in segs.windows(2) {
            let k2 = crate::anchor::classify(w[0].p1, w[0].c2, w[1].c1);
            assert_ne!(k2, crate::anchor::AnchorKind::Corner);
        }
    }

    #[test]
    fn refit_is_deterministic() {
        let pts = rect_pts(0.25, 0.25, 7.75, 3.5);
        assert_eq!(
            refit_closed_contour(&pts, TOL_REFIT),
            refit_closed_contour(&pts, TOL_REFIT)
        );
    }
}
