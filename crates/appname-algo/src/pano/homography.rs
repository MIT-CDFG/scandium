// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Planar homography estimation — normalized DLT from hartley04
//! ("Multiple View Geometry in Computer Vision", 2nd ed., Alg. 4.2 with
//! the Alg. 4.1 normalization) inside a deterministic seeded RANSAC
//! (fischler81 hypothesize-and-verify).
//!
//! Contract note (recorded honestly): the `pano_align_stack_flow` area
//! row carries `geometry.ransac_magsac: L0` and the dossier's selection
//! line reads "model election RANSAC/MAGSAC". This v1 implements PLAIN
//! RANSAC with a fixed inlier threshold; the threshold-marginalizing
//! MAGSAC-class upgrade (and the homography-vs-rotation model election)
//! are named CP-1.0 integration remainders in the lane log. Both are L0
//! (era-clear classical methods).
//!
//! Determinism (D-48): the RANSAC seed is a required, documented
//! parameter ([`RansacParams::seed`], crate [`SplitMix64`] stream);
//! sampling, the adaptive iteration update, and every tie-break are
//! fixed-order, so results replay bit-identically. The DLT eigenvector
//! comes from a cyclic Jacobi sweep with fixed rotation order.
//!
//! No external solver: the smallest eigenvector of the 9×9 normal matrix
//! AᵀA is found with an own symmetric Jacobi eigensolver (fixed sweep
//! order, deterministic convergence threshold).

use crate::plane::SplitMix64;

/// 3×3 homography, row-major; maps (x, y, 1)ᵀ up to scale.
pub type Homography = [[f64; 3]; 3];

/// Apply `h` to a point (perspective divide). Returns None when the
/// denominator vanishes (point at infinity).
pub fn apply_homography(h: &Homography, x: f64, y: f64) -> Option<(f64, f64)> {
    let wz = h[2][0] * x + h[2][1] * y + h[2][2];
    if wz.abs() < 1e-12 {
        return None;
    }
    Some((
        (h[0][0] * x + h[0][1] * y + h[0][2]) / wz,
        (h[1][0] * x + h[1][1] * y + h[1][2]) / wz,
    ))
}

/// Hartley-normalization output: the 3x3 similarity `T` plus the
/// transformed points.
type NormalizedPoints = ([[f64; 3]; 3], Vec<(f64, f64)>);

/// Hartley normalization (hartley04 Alg. 4.1): translate the centroid to
/// the origin and scale the mean distance to √2. Returns (T, normalized
/// points); None when the points are all coincident.
fn normalize(pts: &[(f64, f64)]) -> Option<NormalizedPoints> {
    let n = pts.len() as f64;
    let (mut cx, mut cy) = (0.0, 0.0);
    for (x, y) in pts {
        cx += x;
        cy += y;
    }
    cx /= n;
    cy /= n;
    let mut mean_d = 0.0;
    for (x, y) in pts {
        mean_d += ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
    }
    mean_d /= n;
    if mean_d < 1e-12 {
        return None;
    }
    let s = std::f64::consts::SQRT_2 / mean_d;
    let t = [[s, 0.0, -s * cx], [0.0, s, -s * cy], [0.0, 0.0, 1.0]];
    let out = pts
        .iter()
        .map(|(x, y)| (s * (x - cx), s * (y - cy)))
        .collect();
    Some((t, out))
}

/// Symmetric Jacobi eigensolver: returns the eigenvector of the smallest
/// eigenvalue of the symmetric matrix `a` (9×9 here). Cyclic upper-
/// triangle sweeps, fixed order, off-diagonal threshold 1e-14 · ‖A‖,
/// capped at 64 sweeps (converges in far fewer; the cap bounds work
/// deterministically).
fn smallest_eigenvector_9(a: &mut [[f64; 9]; 9]) -> [f64; 9] {
    let mut v = [[0f64; 9]; 9];
    for (i, row) in v.iter_mut().enumerate() {
        row[i] = 1.0;
    }
    let norm: f64 = a
        .iter()
        .flat_map(|r| r.iter())
        .map(|x| x * x)
        .sum::<f64>()
        .sqrt()
        .max(1e-300);
    let tol = 1e-14 * norm;
    for _sweep in 0..64 {
        let mut off = 0.0f64;
        for (p, row) in a.iter().enumerate() {
            for x in &row[p + 1..] {
                off += x * x;
            }
        }
        if off.sqrt() < tol {
            break;
        }
        for p in 0..9 {
            for q in p + 1..9 {
                let apq = a[p][q];
                if apq.abs() < 1e-300 {
                    continue;
                }
                let theta = (a[q][q] - a[p][p]) / (2.0 * apq);
                let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;
                for row in a.iter_mut() {
                    let akp = row[p];
                    let akq = row[q];
                    row[p] = c * akp - s * akq;
                    row[q] = s * akp + c * akq;
                }
                // k is a column index across the two rotated rows of `a`.
                #[allow(clippy::needless_range_loop)]
                for k in 0..9 {
                    let apk = a[p][k];
                    let aqk = a[q][k];
                    a[p][k] = c * apk - s * aqk;
                    a[q][k] = s * apk + c * aqk;
                }
                for row in v.iter_mut() {
                    let vkp = row[p];
                    let vkq = row[q];
                    row[p] = c * vkp - s * vkq;
                    row[q] = s * vkp + c * vkq;
                }
            }
        }
    }
    // Smallest diagonal entry (first minimal index wins — deterministic).
    let mut mi = 0usize;
    for i in 1..9 {
        if a[i][i] < a[mi][mi] {
            mi = i;
        }
    }
    let mut out = [0f64; 9];
    for k in 0..9 {
        out[k] = v[k][mi];
    }
    out
}

/// Normalized DLT (hartley04 Alg. 4.2): estimate H with `dst ≈ H · src`
/// from ≥ 4 correspondences. Errors on degenerate input (coincident
/// clouds or a rank-deficient system).
pub fn dlt_homography(src: &[(f64, f64)], dst: &[(f64, f64)]) -> Result<Homography, String> {
    if src.len() != dst.len() {
        return Err("dlt_homography: length mismatch".into());
    }
    if src.len() < 4 {
        return Err(format!(
            "dlt_homography: need >= 4 points, got {}",
            src.len()
        ));
    }
    let (ts, ns) = normalize(src).ok_or("dlt_homography: degenerate (coincident) src")?;
    let (td, nd) = normalize(dst).ok_or("dlt_homography: degenerate (coincident) dst")?;
    // Accumulate AᵀA directly (2 rows per correspondence).
    let mut ata = [[0f64; 9]; 9];
    let mut acc = |row: [f64; 9]| {
        for i in 0..9 {
            for j in i..9 {
                ata[i][j] += row[i] * row[j];
            }
        }
    };
    for ((x, y), (u, v)) in ns.iter().zip(&nd) {
        acc([-x, -y, -1.0, 0.0, 0.0, 0.0, u * x, u * y, *u]);
        acc([0.0, 0.0, 0.0, -x, -y, -1.0, v * x, v * y, *v]);
    }
    // Mirror the upper triangle across the diagonal: needs ata[i][j] and
    // ata[j][i] together -- the index math is load-bearing.
    #[allow(clippy::needless_range_loop)]
    for i in 0..9 {
        for j in 0..i {
            ata[i][j] = ata[j][i];
        }
    }
    let hv = smallest_eigenvector_9(&mut ata);
    let hn = [
        [hv[0], hv[1], hv[2]],
        [hv[3], hv[4], hv[5]],
        [hv[6], hv[7], hv[8]],
    ];
    // Denormalize: H = T_dst⁻¹ · Ĥ · T_src. T_dst is a similarity
    // [[s,0,-s·cx],[0,s,-s·cy],[0,0,1]]: inverse in closed form.
    let sd = td[0][0];
    let tdi = [
        [1.0 / sd, 0.0, -td[0][2] / sd],
        [0.0, 1.0 / sd, -td[1][2] / sd],
        [0.0, 0.0, 1.0],
    ];
    let m1 = mat_mul(&tdi, &hn);
    let mut h = mat_mul(&m1, &ts);
    // Fixed scale/sign convention: divide by the largest-|·| entry (first
    // maximal index wins), then flip so h[2][2] >= 0 (or the first
    // nonzero entry is positive when h[2][2] == 0) — deterministic.
    let mut mi = (0, 0);
    for (i, row) in h.iter().enumerate() {
        for (j, e) in row.iter().enumerate() {
            if e.abs() > h[mi.0][mi.1].abs() {
                mi = (i, j);
            }
        }
    }
    let m = h[mi.0][mi.1];
    if m.abs() < 1e-300 {
        return Err("dlt_homography: rank-deficient system (zero solution)".into());
    }
    for row in h.iter_mut() {
        for e in row.iter_mut() {
            *e /= m;
        }
    }
    let sign_ref = if h[2][2].abs() > 1e-12 {
        h[2][2]
    } else {
        *h.iter()
            .flat_map(|r| r.iter())
            .find(|e| e.abs() > 1e-12)
            .unwrap_or(&1.0)
    };
    if sign_ref < 0.0 {
        for row in h.iter_mut() {
            for e in row.iter_mut() {
                *e = -*e;
            }
        }
    }
    Ok(h)
}

fn mat_mul(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut c = [[0f64; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            for (k, brow) in b.iter().enumerate() {
                c[i][j] += a[i][k] * brow[j];
            }
        }
    }
    c
}

/// RANSAC parameters. The seed and inlier threshold are the documented
/// determinism-relevant knobs (D-48: both join any golden key).
#[derive(Clone, Copy, Debug)]
pub struct RansacParams {
    /// Seed of the crate [`SplitMix64`] sampling stream (required
    /// documented parameter — there is no unseeded mode).
    pub seed: u64,
    /// Maximum hypothesis iterations (also the fixed count when
    /// `adaptive` is off).
    pub max_iterations: usize,
    /// Inlier threshold on the forward reprojection error |H·src − dst|
    /// in pixels.
    pub inlier_threshold: f64,
    /// Adaptive early termination (hartley04 Alg. 4.5 iteration bound):
    /// N = log(1−confidence) / log(1−w⁴), recomputed deterministically
    /// whenever the best model improves. When off, exactly
    /// `max_iterations` hypotheses are drawn.
    pub adaptive: bool,
    /// Confidence for the adaptive bound (0.99 customary).
    pub confidence: f64,
}

impl Default for RansacParams {
    fn default() -> Self {
        RansacParams {
            seed: 0,
            max_iterations: 2000,
            inlier_threshold: 3.0,
            adaptive: true,
            confidence: 0.99,
        }
    }
}

/// RANSAC result: the refit model, its inlier set, and the hypothesis
/// count actually drawn (evidence for the adaptive bound).
#[derive(Clone, Debug)]
pub struct RansacResult {
    /// Final homography, least-squares (DLT) refit on all inliers.
    pub h: Homography,
    /// Indices of the inlier correspondences under the final model.
    pub inliers: Vec<usize>,
    /// Hypotheses drawn before termination.
    pub iterations_run: usize,
}

/// Three points are collinear when the doubled triangle area is below
/// `tol` (scale-aware: tol = 1e-8 · span²).
fn collinear(a: (f64, f64), b: (f64, f64), c: (f64, f64), tol: f64) -> bool {
    ((b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)).abs() < tol
}

/// A 4-point minimal sample is degenerate when any 3 of the 4 points are
/// collinear in either cloud.
fn sample_degenerate(s: &[(f64, f64); 4], d: &[(f64, f64); 4], tol_s: f64, tol_d: f64) -> bool {
    const TRIPLES: [(usize, usize, usize); 4] = [(0, 1, 2), (0, 1, 3), (0, 2, 3), (1, 2, 3)];
    TRIPLES
        .iter()
        .any(|&(i, j, k)| collinear(s[i], s[j], s[k], tol_s) || collinear(d[i], d[j], d[k], tol_d))
}

fn span2(pts: &[(f64, f64)]) -> f64 {
    let (mut xmin, mut xmax, mut ymin, mut ymax) = (
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
    );
    for &(x, y) in pts {
        xmin = xmin.min(x);
        xmax = xmax.max(x);
        ymin = ymin.min(y);
        ymax = ymax.max(y);
    }
    ((xmax - xmin).powi(2) + (ymax - ymin).powi(2)).max(1.0)
}

/// Deterministic seeded RANSAC homography with final inlier refit.
///
/// `src[i] ↦ dst[i]` are putative correspondences. Errors when fewer
/// than 4 correspondences exist or no valid model is found (all samples
/// degenerate / nothing meets the threshold).
pub fn ransac_homography(
    src: &[(f64, f64)],
    dst: &[(f64, f64)],
    p: &RansacParams,
) -> Result<RansacResult, String> {
    if src.len() != dst.len() {
        return Err("ransac_homography: length mismatch".into());
    }
    let n = src.len();
    if n < 4 {
        return Err(format!(
            "ransac_homography: need >= 4 correspondences, got {n}"
        ));
    }
    // Deliberate negated-comparison shape: also catches NaN.
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    if !(p.inlier_threshold > 0.0) {
        return Err("ransac_homography: inlier_threshold must be positive".into());
    }
    if p.max_iterations == 0 {
        return Err("ransac_homography: max_iterations must be >= 1".into());
    }
    let tol_s = 1e-8 * span2(src);
    let tol_d = 1e-8 * span2(dst);
    let mut rng = SplitMix64(p.seed);
    let mut best_inliers: Vec<usize> = Vec::new();
    let mut best_err = f64::INFINITY;
    let thr2 = p.inlier_threshold * p.inlier_threshold;
    let mut budget = p.max_iterations;
    let mut it = 0usize;
    while it < budget {
        it += 1;
        // Draw 4 distinct indices (retry duplicates; fixed order —
        // deterministic).
        let mut idx = [0usize; 4];
        let mut k = 0;
        let mut guard = 0;
        while k < 4 {
            let cand = (rng.next_u64() % n as u64) as usize;
            if idx[..k].contains(&cand) {
                guard += 1;
                if guard > 1000 {
                    break;
                }
                continue;
            }
            idx[k] = cand;
            k += 1;
        }
        if k < 4 {
            continue;
        }
        let s4 = [src[idx[0]], src[idx[1]], src[idx[2]], src[idx[3]]];
        let d4 = [dst[idx[0]], dst[idx[1]], dst[idx[2]], dst[idx[3]]];
        if sample_degenerate(&s4, &d4, tol_s, tol_d) {
            continue;
        }
        let Ok(h) = dlt_homography(&s4, &d4) else {
            continue;
        };
        // Score: inlier count, then total squared error (deterministic
        // strict improvement; ties keep the earlier hypothesis).
        let mut inliers = Vec::new();
        let mut err_sum = 0.0;
        for i in 0..n {
            let Some((px, py)) = apply_homography(&h, src[i].0, src[i].1) else {
                continue;
            };
            let e2 = (px - dst[i].0).powi(2) + (py - dst[i].1).powi(2);
            if e2 < thr2 {
                inliers.push(i);
                err_sum += e2;
            }
        }
        let better = inliers.len() > best_inliers.len()
            || (inliers.len() == best_inliers.len()
                && !best_inliers.is_empty()
                && err_sum < best_err);
        if better {
            best_inliers = inliers;
            best_err = err_sum;
            if p.adaptive && best_inliers.len() >= 4 {
                // hartley04 Alg. 4.5: N = log(1−p) / log(1−w⁴).
                let w = best_inliers.len() as f64 / n as f64;
                let denom = (1.0 - w.powi(4)).max(1e-12).ln();
                let need = ((1.0 - p.confidence).ln() / denom).ceil();
                let need = if need.is_finite() && need >= 1.0 {
                    need as usize
                } else {
                    1
                };
                budget = budget.min(it.max(need));
            }
        }
    }
    if best_inliers.len() < 4 {
        return Err("ransac_homography: no valid model (degenerate or all outliers)".into());
    }
    // Final least-squares refit on all inliers.
    let rs: Vec<(f64, f64)> = best_inliers.iter().map(|&i| src[i]).collect();
    let rd: Vec<(f64, f64)> = best_inliers.iter().map(|&i| dst[i]).collect();
    let h = dlt_homography(&rs, &rd)?;
    // Recompute the inlier set under the refit model (reported set is
    // consistent with the reported H).
    let mut inliers = Vec::new();
    for i in 0..n {
        if let Some((px, py)) = apply_homography(&h, src[i].0, src[i].1) {
            if (px - dst[i].0).powi(2) + (py - dst[i].1).powi(2) < thr2 {
                inliers.push(i);
            }
        }
    }
    Ok(RansacResult {
        h,
        inliers,
        iterations_run: it,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plane::plane_hash_f64;

    /// Ground-truth test homography: mild projective transform.
    fn gt() -> Homography {
        [
            [0.98, -0.05, 6.0],
            [0.04, 1.02, -3.5],
            [1.5e-4, -1.0e-4, 1.0],
        ]
    }

    fn grid_points() -> Vec<(f64, f64)> {
        let mut pts = Vec::new();
        for gy in 0..6 {
            for gx in 0..6 {
                pts.push((10.0 + 20.0 * gx as f64, 8.0 + 18.0 * gy as f64));
            }
        }
        pts
    }

    #[test]
    fn dlt_recovers_exact_homography_from_clean_points() {
        let src = grid_points();
        let h_true = gt();
        let dst: Vec<(f64, f64)> = src
            .iter()
            .map(|&(x, y)| apply_homography(&h_true, x, y).unwrap())
            .collect();
        let h = dlt_homography(&src, &dst).unwrap();
        // Compare action, not entries: max reprojection deviation on an
        // independent probe grid < 1e-8 px.
        for gy in 0..5 {
            for gx in 0..5 {
                let p = (15.0 + 21.0 * gx as f64, 12.0 + 19.0 * gy as f64);
                let a = apply_homography(&h_true, p.0, p.1).unwrap();
                let b = apply_homography(&h, p.0, p.1).unwrap();
                let d = ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
                assert!(d < 1e-8, "probe {:?}: deviation {d}", p);
            }
        }
    }

    #[test]
    fn ransac_recovers_h_under_outliers_with_fixed_seed() {
        let src = grid_points();
        let h_true = gt();
        let mut dst: Vec<(f64, f64)> = src
            .iter()
            .map(|&(x, y)| apply_homography(&h_true, x, y).unwrap())
            .collect();
        // Deterministic small noise (±0.2 px) + 30% gross outliers.
        let mut rng = SplitMix64(7);
        for d in dst.iter_mut() {
            d.0 += (rng.next_f64() - 0.5) * 0.4;
            d.1 += (rng.next_f64() - 0.5) * 0.4;
        }
        let n_out = src.len() * 3 / 10;
        for i in 0..n_out {
            let j = (rng.next_u64() % src.len() as u64) as usize;
            dst[j] = (rng.next_f64() * 200.0, rng.next_f64() * 150.0);
            let _ = i;
        }
        let p = RansacParams {
            seed: 42,
            inlier_threshold: 1.5,
            ..RansacParams::default()
        };
        let r = ransac_homography(&src, &dst, &p).unwrap();
        assert!(
            r.inliers.len() >= src.len() - n_out - 2,
            "only {} inliers",
            r.inliers.len()
        );
        // Reprojection error against ground truth on a probe grid < 0.5 px
        // (noise floor 0.2 px per axis).
        for gy in 0..5 {
            for gx in 0..5 {
                let pt = (15.0 + 21.0 * gx as f64, 12.0 + 19.0 * gy as f64);
                let a = apply_homography(&h_true, pt.0, pt.1).unwrap();
                let b = apply_homography(&r.h, pt.0, pt.1).unwrap();
                let d = ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
                assert!(d < 0.5, "probe {:?}: deviation {d}", pt);
            }
        }
        // Adaptive bound engaged: far fewer hypotheses than the cap.
        assert!(r.iterations_run < p.max_iterations);
    }

    #[test]
    fn collinear_minimal_samples_are_rejected() {
        // All source points on one line: every 4-sample is degenerate and
        // RANSAC must fail cleanly, not return a junk model.
        let src: Vec<(f64, f64)> = (0..12)
            .map(|i| (i as f64 * 10.0, 5.0 + i as f64 * 2.0))
            .collect();
        let dst: Vec<(f64, f64)> = (0..12).map(|i| (i as f64 * 9.0, 40.0 - i as f64)).collect();
        let p = RansacParams {
            seed: 3,
            max_iterations: 200,
            adaptive: false,
            ..RansacParams::default()
        };
        assert!(ransac_homography(&src, &dst, &p).is_err());
        // And the degeneracy predicate itself fires on a collinear triple.
        assert!(sample_degenerate(
            &[(0.0, 0.0), (1.0, 1.0), (2.0, 2.0), (5.0, 1.0)],
            &[(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)],
            1e-6,
            1e-6
        ));
    }

    #[test]
    fn determinism_two_run_and_golden() {
        let src = grid_points();
        let h_true = gt();
        let mut dst: Vec<(f64, f64)> = src
            .iter()
            .map(|&(x, y)| apply_homography(&h_true, x, y).unwrap())
            .collect();
        let mut rng = SplitMix64(11);
        for d in dst.iter_mut() {
            d.0 += (rng.next_f64() - 0.5) * 0.4;
            d.1 += (rng.next_f64() - 0.5) * 0.4;
        }
        for _ in 0..8 {
            let j = (rng.next_u64() % src.len() as u64) as usize;
            dst[j] = (rng.next_f64() * 180.0, rng.next_f64() * 140.0);
        }
        let p = RansacParams {
            seed: 1234,
            inlier_threshold: 1.5,
            ..RansacParams::default()
        };
        let r1 = ransac_homography(&src, &dst, &p).unwrap();
        let r2 = ransac_homography(&src, &dst, &p).unwrap();
        let flat = |r: &RansacResult| -> Vec<f64> {
            let mut v: Vec<f64> = r.h.iter().flat_map(|row| row.iter().copied()).collect();
            v.extend(r.inliers.iter().map(|&i| i as f64));
            v.push(r.iterations_run as f64);
            v
        };
        let (h1, h2) = (plane_hash_f64(&flat(&r1)), plane_hash_f64(&flat(&r2)));
        assert_eq!(h1, h2, "RANSAC must replay bit-identically at fixed seed");
        assert_eq!(
            h1, GOLDEN,
            "golden hash drift — investigate before re-pinning (O-QA rule)"
        );
    }

    // Self-pinned golden (see determinism_two_run_and_golden): pinned
    // 2026-08-13 from the first run whose behavioral oracles (clean DLT
    // recovery, outlier-robust recovery, degeneracy rejection) were all
    // green. Golden key includes seed 1234 + threshold 1.5. Re-pinning
    // requires investigation + a fresh oracle pass.
    const GOLDEN: u64 = 3590875880576856261;
}
