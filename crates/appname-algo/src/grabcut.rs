// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Implemented from the published method (C. Rother, V. Kolmogorov,
// A. Blake, "GrabCut: interactive foreground extraction using iterated
// graph cuts", SIGGRAPH 2004 — census paper key rother04; seeded
// formulation Y. Boykov, M.-P. Jolly, ICCV 2001 — boykov01). Census row
// B1-opencv-grabcut.yaml records OpenCV as behavior reference ONLY: the
// OpenCV tree is pin-recorded, was never staged or opened, and no
// expression from it is present here — this is a clean-room Rust
// implementation of the published iterated-GMM/graph-cut contract.
// Deterministic GMM initialization follows the Orchard–Bouman
// eigen-split quantization rother04 cites ("Color quantization of
// images", IEEE Trans. Signal Processing 1991) — RNG-free.

//! Spec: ALGORITHMS.md §8 `legacy_selection` primary
//! `grabcut_plus_boykov_jolly_plus_livewire` (level L1, basis [rother04,
//! boykov01, mortensen95], register keys P-7660463 "Expired - Fee
//! Related" / P-6973212 "Expired - Lifetime" / P-6744923 "Expired -
//! Lifetime"); ES §39.2/§39.4 wand + quick-select consumers;
//! INTEGRATION-NOTES §6 order 8 (`opencv-grabcut` slot, after the
//! [`crate::graphcut`] engine). Live-wire (mortensen95) is a separate
//! CP-1.0 tool item and is NOT built here.
//!
//! Iterated-GMM + graph-cut foreground extraction (rother04 §3), the
//! quick-select/wand ML-free tier:
//!
//! * K=5 full-covariance RGB Gaussians per side, deterministic
//!   Orchard–Bouman-style eigen-split initialization (no RNG anywhere);
//! * β = 1/(2·mean‖z_m − z_n‖²) over 4-connected neighbor pairs (§8.B
//!   [NORMATIVE] binds 4-connected N-links); n-links
//!   γ·exp(−β‖z_m − z_n‖²) with γ = 50 — rother04's dis(m,n)⁻¹ factor
//!   is identically 1 at 4-connectivity and is not carried in code;
//!   t-links from −log mixture likelihoods, shift-normalized per pixel
//!   (both terminals minus min(D_bg, D_fg): argmin-preserving, keeps
//!   capacities ≥ 0 where densities exceed 1 — see the step-3 site);
//!   definite labels pinned at K = 1 + 4γ (boykov01's
//!   K = 1 + max_p Σ_q B_{p,q});
//! * iterate assign-components → learn-GMMs → min-cut, bounded at 5
//!   iterations (contract bound) with the §8.B energy-Δ < 0.1% early
//!   stop; covariance regularization guards degenerate clusters;
//! * scribble seeding (Boykov–Jolly) is the same entry point: definite
//!   labels in the input trimap act as seeds and are never flipped.
//!
//! Energy monotonicity note (honest bound): the graph-cut step exactly
//! minimizes the mixture-likelihood energy for fixed GMMs, and the
//! learn step is one hard-EM pass (argmax component assignment + MLE
//! refit). Hard-EM improves the classification likelihood, which tracks
//! but does not exactly equal the summed mixture likelihood the t-links
//! use — so the tracked energy is non-increasing up to a small
//! tolerance, not exactly monotone; the test asserts a 0.5% slack and
//! this comment is the documentation of why.
//!
//! Pure function over planes: no document access. The O-TOOLS
//! wand/quick-select consumer adapts tile data at the brokered adapter
//! seam and lands the resulting mask as one `selection.set` through the
//! `appname.command.v1` envelope spine (D-48); this module never mutates
//! anything but its own outputs.

use crate::graphcut::CutGraph;
use crate::plane::Plane;

/// Trimap/result label: definite background (never flipped).
pub const LABEL_BG: u8 = 0;
/// Trimap/result label: definite foreground (never flipped).
pub const LABEL_FG: u8 = 1;
/// Trimap/result label: probable background (updated by the cut).
pub const LABEL_PROB_BG: u8 = 2;
/// Trimap/result label: probable foreground (updated by the cut).
pub const LABEL_PROB_FG: u8 = 3;

/// Foreground predicate for mask consumers.
#[inline]
pub fn label_is_fg(label: u8) -> bool {
    label == LABEL_FG || label == LABEL_PROB_FG
}

/// GMM component count per side (rother04: K = 5).
const K: usize = 5;
/// N-link contrast weight γ (rother04: 50).
const GAMMA: f64 = 50.0;
/// Definite-label t-link pin, boykov01's K = 1 + max_p Σ_q B_{p,q}.
/// A definite pixel's opposite t-link is 0, so the only way
/// a cut can strand it on the wrong side is by severing its n-links
/// instead of the pin; each of its ≤ 4 n-links is at most γ (the
/// exp(−βd²) contrast factor is ≤ 1), so K = 1 + 4γ strictly exceeds
/// that budget and the pin arc is never the cheaper cut.
const PIN: f64 = 1.0 + 4.0 * GAMMA;
/// Mixture-likelihood floor before −log (keeps t-links finite).
const LIKELIHOOD_FLOOR: f64 = 1e-100;
/// Base covariance regularization added to the diagonal (colors in
/// [0,1]; ≈ (0.0004)² per channel — inaudible against real image noise,
/// enough to keep single-color clusters invertible).
const COV_REG: f64 = 1.5e-7;

/// GrabCut output: updated trimap labels plus the per-iteration energy
/// trace (for convergence evidence).
#[derive(Debug)]
pub struct GrabcutResult {
    /// Updated labels, same codes as the input trimap; definite labels
    /// preserved, probable labels resolved by the final cut.
    pub labels: Vec<u8>,
    /// Total energy (data + smoothness) after each completed iteration.
    pub energies: Vec<f64>,
    /// Iterations actually run (≤ the requested bound).
    pub iterations: usize,
}

/// Rect-seeded trimap (rother04 §3.1 user interface): inside the rect
/// probable foreground, outside definite background. `x0..x1`/`y0..y1`
/// are half-open pixel ranges, clamped to the plane.
pub fn rect_trimap(w: usize, h: usize, x0: usize, y0: usize, x1: usize, y1: usize) -> Vec<u8> {
    let mut labels = vec![LABEL_BG; w * h];
    for y in y0..y1.min(h) {
        for x in x0..x1.min(w) {
            labels[y * w + x] = LABEL_PROB_FG;
        }
    }
    labels
}

/// Hard foreground mask (1.0 fg / 0.0 bg) from result labels — the
/// adapter-seam output shape the wand/quick-select consumer feeds into
/// its `selection.set` command.
///
/// # Panics
///
/// Panics if `labels.len() != w * h`.
pub fn labels_to_mask(labels: &[u8], w: usize, h: usize) -> Plane {
    assert_eq!(labels.len(), w * h, "label buffer length mismatch");
    let data = labels
        .iter()
        .map(|&l| if label_is_fg(l) { 1.0f32 } else { 0.0f32 })
        .collect();
    Plane::from_vec(w, h, data)
}

// ---------------------------------------------------------------------
// Symmetric 3x3 eigen-decomposition (cyclic Jacobi, deterministic).
// ---------------------------------------------------------------------

/// Eigen-decomposition of a symmetric 3x3 matrix by cyclic Jacobi
/// rotations (fixed pivot order (0,1),(0,2),(1,2), fixed sweep cap —
/// fully deterministic). Returns (eigenvalues, eigenvectors-as-rows),
/// unsorted.
fn sym_eigen3(m: &[[f64; 3]; 3]) -> ([f64; 3], [[f64; 3]; 3]) {
    let mut a = *m;
    let mut v = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for _sweep in 0..50 {
        let off = a[0][1] * a[0][1] + a[0][2] * a[0][2] + a[1][2] * a[1][2];
        if off < 1e-30 {
            break;
        }
        for &(p, q) in &[(0usize, 1usize), (0, 2), (1, 2)] {
            if a[p][q].abs() < 1e-300 {
                continue;
            }
            let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
            let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
            let c = 1.0 / (t * t + 1.0).sqrt();
            let s = t * c;
            // Rotate rows/columns p and q of `a`.
            let app = a[p][p];
            let aqq = a[q][q];
            let apq = a[p][q];
            a[p][p] = c * c * app - 2.0 * s * c * apq + s * s * aqq;
            a[q][q] = s * s * app + 2.0 * s * c * apq + c * c * aqq;
            a[p][q] = 0.0;
            a[q][p] = 0.0;
            // Symmetric rotation touches a[r][p], a[p][r], a[r][q], a[q][r]
            // in one pass: the index math is load-bearing.
            #[allow(clippy::needless_range_loop)]
            for r in 0..3 {
                if r != p && r != q {
                    let arp = a[r][p];
                    let arq = a[r][q];
                    a[r][p] = c * arp - s * arq;
                    a[p][r] = a[r][p];
                    a[r][q] = s * arp + c * arq;
                    a[q][r] = a[r][q];
                }
            }
            // r is a column index across the two rotated rows of `v`.
            #[allow(clippy::needless_range_loop)]
            for r in 0..3 {
                let vpr = v[p][r];
                let vqr = v[q][r];
                v[p][r] = c * vpr - s * vqr;
                v[q][r] = s * vpr + c * vqr;
            }
        }
    }
    ([a[0][0], a[1][1], a[2][2]], v)
}

// ---------------------------------------------------------------------
// GMM (K full-covariance Gaussians, one instance per side).
// ---------------------------------------------------------------------

struct Gmm {
    weight: [f64; K],
    mean: [[f64; 3]; K],
    inv_cov: [[[f64; 3]; 3]; K],
    /// log of the covariance determinant per component.
    log_det: [f64; K],
}

fn det3(m: &[[f64; 3]; 3]) -> f64 {
    m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
}

fn inv3(m: &[[f64; 3]; 3], det: f64) -> [[f64; 3]; 3] {
    let d = 1.0 / det;
    [
        [
            (m[1][1] * m[2][2] - m[1][2] * m[2][1]) * d,
            (m[0][2] * m[2][1] - m[0][1] * m[2][2]) * d,
            (m[0][1] * m[1][2] - m[0][2] * m[1][1]) * d,
        ],
        [
            (m[1][2] * m[2][0] - m[1][0] * m[2][2]) * d,
            (m[0][0] * m[2][2] - m[0][2] * m[2][0]) * d,
            (m[0][2] * m[1][0] - m[0][0] * m[1][2]) * d,
        ],
        [
            (m[1][0] * m[2][1] - m[1][1] * m[2][0]) * d,
            (m[0][1] * m[2][0] - m[0][0] * m[2][1]) * d,
            (m[0][0] * m[1][1] - m[0][1] * m[1][0]) * d,
        ],
    ]
}

/// Mean and covariance of a pixel subset (indices into `px`).
fn mean_cov(px: &[[f64; 3]], idx: &[u32]) -> ([f64; 3], [[f64; 3]; 3]) {
    let n = idx.len() as f64;
    let mut mean = [0.0f64; 3];
    for &i in idx {
        for c in 0..3 {
            mean[c] += px[i as usize][c];
        }
    }
    for c in mean.iter_mut() {
        *c /= n;
    }
    let mut cov = [[0.0f64; 3]; 3];
    for &i in idx {
        let z = px[i as usize];
        for r in 0..3 {
            for c in 0..3 {
                cov[r][c] += (z[r] - mean[r]) * (z[c] - mean[c]);
            }
        }
    }
    for row in cov.iter_mut() {
        for c in row.iter_mut() {
            *c /= n;
        }
    }
    (mean, cov)
}

impl Gmm {
    /// MLE fit from hard component assignments; empty components get
    /// zero weight (and never win an assignment). Covariance
    /// regularization: `COV_REG` on the diagonal always, escalated
    /// deterministically until the determinant is invertible.
    fn learn(px: &[[f64; 3]], comp: &[u8], members: &[u32]) -> Gmm {
        let mut buckets: [Vec<u32>; K] = Default::default();
        for &i in members {
            buckets[comp[i as usize] as usize].push(i);
        }
        let total = members.len() as f64;
        let mut g = Gmm {
            weight: [0.0; K],
            mean: [[0.0; 3]; K],
            inv_cov: [[[0.0; 3]; 3]; K],
            log_det: [0.0; K],
        };
        for (k, bucket) in buckets.iter().enumerate() {
            if bucket.is_empty() {
                continue;
            }
            let (mean, mut cov) = mean_cov(px, bucket);
            let mut reg = COV_REG;
            for (d, row) in cov.iter_mut().enumerate() {
                row[d] += reg;
            }
            let mut det = det3(&cov);
            while det <= 1e-30 {
                reg *= 10.0;
                for (d, row) in cov.iter_mut().enumerate() {
                    row[d] += reg;
                }
                det = det3(&cov);
            }
            g.weight[k] = bucket.len() as f64 / total;
            g.mean[k] = mean;
            g.inv_cov[k] = inv3(&cov, det);
            g.log_det[k] = det.ln();
        }
        g
    }

    /// Squared Mahalanobis distance to component `k`.
    #[inline]
    fn mahalanobis2(&self, k: usize, z: &[f64; 3]) -> f64 {
        let d = [
            z[0] - self.mean[k][0],
            z[1] - self.mean[k][1],
            z[2] - self.mean[k][2],
        ];
        let ic = &self.inv_cov[k];
        let mut q = 0.0;
        for r in 0..3 {
            q += d[r] * (ic[r][0] * d[0] + ic[r][1] * d[1] + ic[r][2] * d[2]);
        }
        q
    }

    /// Most-likely component for `z` (rother04 step 1); deterministic
    /// tie-break: lowest index wins.
    fn best_component(&self, z: &[f64; 3]) -> u8 {
        let mut best_k = 0usize;
        let mut best_score = f64::NEG_INFINITY;
        for k in 0..K {
            if self.weight[k] <= 0.0 {
                continue;
            }
            let score = self.weight[k].ln() - 0.5 * self.log_det[k] - 0.5 * self.mahalanobis2(k, z);
            if score > best_score {
                best_score = score;
                best_k = k;
            }
        }
        best_k as u8
    }

    /// −log of the summed mixture likelihood (the t-link data term),
    /// floored to stay finite.
    fn neg_log_likelihood(&self, z: &[f64; 3]) -> f64 {
        // (2π)^3, so that `sqrt(NORM · det)` below is the full
        // (2π)^{3/2}·√det Gaussian normalization for d = 3.
        const NORM: f64 = 248.050_213_442_398_53;
        let mut p = 0.0;
        for k in 0..K {
            if self.weight[k] <= 0.0 {
                continue;
            }
            let q = self.mahalanobis2(k, z);
            p += self.weight[k] * (-0.5 * q).exp() / (NORM * self.log_det[k].exp()).sqrt();
        }
        -(p.max(LIKELIHOOD_FLOOR)).ln()
    }
}

/// Deterministic Orchard–Bouman-style eigen-split initialization: start
/// with one cluster, repeatedly split the cluster with the largest
/// principal eigenvalue along its principal axis at the mean, until K
/// clusters (or nothing left to split). RNG-free; ties break to the
/// lowest cluster index.
fn init_components(px: &[[f64; 3]], members: &[u32], comp: &mut [u8]) {
    let mut clusters: Vec<Vec<u32>> = vec![members.to_vec()];
    while clusters.len() < K {
        // Pick the cluster with the largest principal eigenvalue.
        let mut best: Option<(usize, f64, [f64; 3], [f64; 3])> = None;
        for (ci, cluster) in clusters.iter().enumerate() {
            if cluster.len() < 2 {
                continue;
            }
            let (mean, cov) = mean_cov(px, cluster);
            let (evals, evecs) = sym_eigen3(&cov);
            let mut k_max = 0;
            for k in 1..3 {
                if evals[k] > evals[k_max] {
                    k_max = k;
                }
            }
            let lead = evals[k_max];
            if best.as_ref().is_none_or(|b| lead > b.1) {
                best = Some((ci, lead, mean, evecs[k_max]));
            }
        }
        let (ci, lead, mean, axis) = match best {
            Some(b) if b.1 > 1e-24 => b,
            _ => break, // every remaining cluster is a point mass
        };
        let _ = lead;
        let cluster = std::mem::take(&mut clusters[ci]);
        let (mut lo, mut hi) = (Vec::new(), Vec::new());
        for &i in &cluster {
            let z = &px[i as usize];
            let proj = (z[0] - mean[0]) * axis[0]
                + (z[1] - mean[1]) * axis[1]
                + (z[2] - mean[2]) * axis[2];
            if proj <= 0.0 {
                lo.push(i);
            } else {
                hi.push(i);
            }
        }
        if lo.is_empty() || hi.is_empty() {
            // Numerically unsplittable; restore and stop.
            clusters[ci] = cluster;
            break;
        }
        clusters[ci] = lo;
        clusters.push(hi);
    }
    for (k, cluster) in clusters.iter().enumerate() {
        for &i in cluster {
            comp[i as usize] = k as u8;
        }
    }
}

// ---------------------------------------------------------------------
// Orchestration.
// ---------------------------------------------------------------------

/// 4-connectivity forward offsets from pixel (x, y): E, S — each
/// unordered neighbor pair visited exactly once (§8.B [NORMATIVE]:
/// 4-connected N-links). Every neighbor distance is exactly 1, so
/// rother04's dis(m,n)⁻¹ n-link factor is identically 1 and no distance
/// divisor is carried.
const FWD: [(isize, isize); 2] = [(1, 0), (0, 1)];

#[inline]
fn pixel(rgb: &[Plane; 3], x: usize, y: usize) -> [f64; 3] {
    [
        rgb[0].at(x, y) as f64,
        rgb[1].at(x, y) as f64,
        rgb[2].at(x, y) as f64,
    ]
}

/// β = 1/(2·mean‖z_m − z_n‖²) over all 4-connected neighbor pairs
/// (rother04 §3.2 contrast adaptation, over the §8.B-bound 4-connected
/// neighborhood); 0 for a constant image.
fn compute_beta(rgb: &[Plane; 3], w: usize, h: usize) -> f64 {
    let mut sum = 0.0f64;
    let mut count = 0u64;
    for y in 0..h {
        for x in 0..w {
            let z = pixel(rgb, x, y);
            for &(dx, dy) in &FWD {
                let (nx, ny) = (x as isize + dx, y as isize + dy);
                if nx < 0 || ny < 0 || nx >= w as isize || ny >= h as isize {
                    continue;
                }
                let zn = pixel(rgb, nx as usize, ny as usize);
                let d2 = (z[0] - zn[0]).powi(2) + (z[1] - zn[1]).powi(2) + (z[2] - zn[2]).powi(2);
                sum += d2;
                count += 1;
            }
        }
    }
    if sum <= 0.0 || count == 0 {
        0.0
    } else {
        1.0 / (2.0 * sum / count as f64)
    }
}

/// GrabCut / seeded graph-cut segmentation (rother04 iterated
/// minimization; boykov01 seeding semantics via definite trimap labels).
///
/// * `rgb` — RGB planes in [0,1], equal dimensions;
/// * `trimap` — per-pixel [`LABEL_BG`]/[`LABEL_FG`]/[`LABEL_PROB_BG`]/
///   [`LABEL_PROB_FG`], length `w·h` (see [`rect_trimap`]);
/// * `max_iterations` — clamped to the contract bound of 5; the §8.B
///   energy-Δ < 0.1% early stop also applies.
///
/// Requires at least one background-side and one foreground-side pixel;
/// otherwise the trimap is returned unchanged with an empty energy
/// trace.
///
/// # Errors
///
/// Returns `Err` if any trimap byte is not one of the four label codes
/// ([`LABEL_BG`]/[`LABEL_FG`]/[`LABEL_PROB_BG`]/[`LABEL_PROB_FG`]) —
/// malformed input is rejected up front, never silently reinterpreted.
///
/// # Panics
///
/// Panics if the RGB planes have mismatched dimensions or if
/// `trimap.len() != w * h`.
pub fn grabcut(
    rgb: &[Plane; 3],
    trimap: &[u8],
    max_iterations: usize,
) -> Result<GrabcutResult, String> {
    let (w, h) = (rgb[0].w, rgb[0].h);
    assert!(
        rgb.iter().all(|p| p.w == w && p.h == h),
        "plane dims mismatch"
    );
    assert_eq!(trimap.len(), w * h, "trimap length mismatch");
    if let Some((i, &code)) = trimap.iter().enumerate().find(|&(_, &l)| l > LABEL_PROB_FG) {
        return Err(format!(
            "invalid trimap code {code} at index {i} (valid: 0..=3)"
        ));
    }
    let n = w * h;
    let mut labels = trimap.to_vec();
    let max_iterations = max_iterations.min(5);

    let px: Vec<[f64; 3]> = (0..n).map(|i| pixel(rgb, i % w, i / w)).collect();

    let side_members = |labels: &[u8], fg: bool| -> Vec<u32> {
        (0..n as u32)
            .filter(|&i| label_is_fg(labels[i as usize]) == fg)
            .collect()
    };
    if side_members(&labels, true).is_empty() || side_members(&labels, false).is_empty() {
        return Ok(GrabcutResult {
            labels,
            energies: Vec::new(),
            iterations: 0,
        });
    }

    let beta = compute_beta(rgb, w, h);

    // Precompute n-link weights once (image-dependent only).
    // nlink[i][d] = weight of the FWD[d] link from pixel i (0 = off-grid).
    // No distance divisor: at 4-connectivity dis(m,n) = 1 for every pair.
    let mut nlinks = vec![[0.0f64; 2]; n];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let z = &px[i];
            for (d, &(dx, dy)) in FWD.iter().enumerate() {
                let (nx, ny) = (x as isize + dx, y as isize + dy);
                if nx < 0 || ny < 0 || nx >= w as isize || ny >= h as isize {
                    continue;
                }
                let j = ny as usize * w + nx as usize;
                let zn = &px[j];
                let d2 = (z[0] - zn[0]).powi(2) + (z[1] - zn[1]).powi(2) + (z[2] - zn[2]).powi(2);
                nlinks[i][d] = GAMMA * (-beta * d2).exp();
            }
        }
    }

    let mut comp = vec![0u8; n];
    let mut bg_gmm: Option<Gmm> = None;
    let mut fg_gmm: Option<Gmm> = None;
    let mut energies: Vec<f64> = Vec::new();
    let mut iterations = 0usize;

    for _iter in 0..max_iterations {
        let bg_members = side_members(&labels, false);
        let fg_members = side_members(&labels, true);
        if bg_members.is_empty() || fg_members.is_empty() {
            break;
        }

        // Step 1: component assignment — eigen-split init on the first
        // pass, most-likely component thereafter (rother04 step 1).
        match (&bg_gmm, &fg_gmm) {
            (Some(bg), Some(fg)) => {
                for &i in &bg_members {
                    comp[i as usize] = bg.best_component(&px[i as usize]);
                }
                for &i in &fg_members {
                    comp[i as usize] = fg.best_component(&px[i as usize]);
                }
            }
            _ => {
                init_components(&px, &bg_members, &mut comp);
                init_components(&px, &fg_members, &mut comp);
            }
        }

        // Step 2: learn GMM parameters (rother04 step 2).
        let bg = Gmm::learn(&px, &comp, &bg_members);
        let fg = Gmm::learn(&px, &comp, &fg_members);

        // Step 3: min-cut over t-links + n-links (rother04 step 3;
        // source terminal = foreground side).
        // Per pixel: ≤ 2 n-link arc pairs (E, S) + ≤ 2 terminal arc pairs.
        let mut graph = CutGraph::with_capacity(n, n * 4);
        for i in 0..n {
            let (from_source, to_sink) = match labels[i] {
                LABEL_FG => (PIN, 0.0),
                LABEL_BG => (0.0, PIN),
                // Validation above admits exactly the four label codes, so
                // this arm is the two probable labels. The rother04 data
                // terms D_bg/D_fg are −log mixture likelihoods and go
                // NEGATIVE where the mixture density exceeds 1 (e.g.
                // covariance-floor point-mass clusters on low-variance
                // regions); `CutGraph::add_terminal` keeps only positive
                // capacities, so raw negative t-links would be truncated
                // to 0, distorting the energy. Shift-normalize per pixel
                // instead: subtract m = min(D_bg, D_fg) from both
                // terminals. Every s-t cut severs exactly ONE of the two
                // t-links of each probable pixel, so a per-pixel constant
                // added to both terminals changes every cut's cost by the
                // same Σm — the argmin cut is exactly preserved — while
                // both capacities stay ≥ 0 (one becomes exactly 0 and is
                // simply not materialized).
                _ => {
                    let d_bg = bg.neg_log_likelihood(&px[i]);
                    let d_fg = fg.neg_log_likelihood(&px[i]);
                    let m = d_bg.min(d_fg);
                    (d_bg - m, d_fg - m)
                }
            };
            graph.add_terminal(i, from_source, to_sink);
            let (x, y) = (i % w, i / w);
            for (d, &(dx, dy)) in FWD.iter().enumerate() {
                let wgt = nlinks[i][d];
                if wgt > 0.0 {
                    let j = (y as isize + dy) as usize * w + (x as isize + dx) as usize;
                    graph.add_edge(i, j, wgt, wgt);
                }
            }
        }
        graph.solve();
        let fg_side = graph.source_side();
        for i in 0..n {
            if labels[i] == LABEL_PROB_BG || labels[i] == LABEL_PROB_FG {
                labels[i] = if fg_side[i] {
                    LABEL_PROB_FG
                } else {
                    LABEL_PROB_BG
                };
            }
        }
        iterations += 1;

        // Energy of the new labeling under the current GMMs (data term =
        // −log mixture likelihood of the assigned side, all pixels;
        // smoothness = n-link weight over label-discontinuous pairs).
        let mut energy = 0.0f64;
        for i in 0..n {
            energy += if label_is_fg(labels[i]) {
                fg.neg_log_likelihood(&px[i])
            } else {
                bg.neg_log_likelihood(&px[i])
            };
        }
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                for (d, &(dx, dy)) in FWD.iter().enumerate() {
                    let wgt = nlinks[i][d];
                    if wgt > 0.0 {
                        let j = (y as isize + dy) as usize * w + (x as isize + dx) as usize;
                        if label_is_fg(labels[i]) != label_is_fg(labels[j]) {
                            energy += wgt;
                        }
                    }
                }
            }
        }
        let converged = energies
            .last()
            .is_some_and(|&prev| (prev - energy) < 0.001 * prev.abs());
        energies.push(energy);
        bg_gmm = Some(bg);
        fg_gmm = Some(fg);
        if converged {
            break;
        }
    }

    Ok(GrabcutResult {
        labels,
        energies,
        iterations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plane::{plane_hash, SplitMix64};

    /// Two-mode fixture: reddish square (true foreground) on a bluish
    /// field, per-channel noise from a pinned SplitMix64 stream.
    /// Returns (rgb, ground-truth fg flags).
    fn two_region_fixture(w: usize, h: usize, seed: u64) -> ([Plane; 3], Vec<bool>) {
        let mut rng = SplitMix64(seed);
        let mut planes = [Plane::new(w, h), Plane::new(w, h), Plane::new(w, h)];
        let mut truth = vec![false; w * h];
        let (fx0, fy0, fx1, fy1) = (w / 4, h / 4, w * 3 / 4, h * 3 / 4);
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                let inside = x >= fx0 && x < fx1 && y >= fy0 && y < fy1;
                truth[i] = inside;
                let base: [f32; 3] = if inside {
                    [0.80, 0.22, 0.18]
                } else {
                    [0.12, 0.32, 0.70]
                };
                for (c, base_c) in base.iter().enumerate() {
                    let noise = (rng.next_f64() as f32 - 0.5) * 0.08;
                    planes[c].data[i] = (base_c + noise).clamp(0.0, 1.0);
                }
            }
        }
        (planes, truth)
    }

    #[test]
    fn grabcut_two_region_accuracy() {
        let (w, h) = (40, 40);
        let (rgb, truth) = two_region_fixture(w, h, 0x08AB_C07A_0000_0001);
        // Loose rect: covers the true square plus a background margin.
        let trimap = rect_trimap(w, h, w / 8, h / 8, w * 7 / 8, h * 7 / 8);
        let out = grabcut(&rgb, &trimap, 5).unwrap();
        let correct = (0..w * h)
            .filter(|&i| label_is_fg(out.labels[i]) == truth[i])
            .count();
        let acc = correct as f64 / (w * h) as f64;
        assert!(acc >= 0.98, "accuracy {acc} below 0.98 bound");
        assert!(out.iterations >= 1 && out.iterations <= 5);
    }

    #[test]
    fn grabcut_definite_labels_preserved() {
        // Boykov–Jolly seeding path: definite scribbles must never flip,
        // even placed against the color evidence.
        let (w, h) = (24, 24);
        let (rgb, _) = two_region_fixture(w, h, 0x08AB_C07A_0000_0002);
        let mut trimap = vec![LABEL_PROB_BG; w * h];
        // Fg scribble in the true-bg corner, bg scribble in the true-fg
        // center, probables everywhere else.
        for t in trimap.iter_mut().take(4) {
            *t = LABEL_FG;
        }
        let center = (h / 2) * w + w / 2;
        trimap[center] = LABEL_BG;
        for x in 0..w {
            trimap[(h - 1) * w + x] = LABEL_PROB_FG;
        }
        let out = grabcut(&rgb, &trimap, 5).unwrap();
        for x in 0..4 {
            assert_eq!(out.labels[x], LABEL_FG, "definite fg seed flipped");
        }
        assert_eq!(out.labels[center], LABEL_BG, "definite bg seed flipped");
        for (i, &l) in out.labels.iter().enumerate() {
            if trimap[i] == LABEL_PROB_BG || trimap[i] == LABEL_PROB_FG {
                assert!(
                    l == LABEL_PROB_BG || l == LABEL_PROB_FG,
                    "probable pixel {i} left the probable label set"
                );
            }
        }
    }

    #[test]
    fn grabcut_energy_track_and_iteration_bound() {
        let (w, h) = (32, 32);
        let (rgb, _) = two_region_fixture(w, h, 0x08AB_C07A_0000_0003);
        let trimap = rect_trimap(w, h, 4, 4, w - 4, h - 4);
        let out = grabcut(&rgb, &trimap, 5).unwrap();
        assert!(out.iterations <= 5, "iteration bound violated");
        assert_eq!(out.energies.len(), out.iterations);
        // Non-increasing up to the documented hard-EM tolerance (module
        // doc: mixture vs classification likelihood makes exact
        // monotonicity approximate).
        // (Energies can be negative: −log of a mixture density > 1, so the
        // slack must be magnitude-based, not multiplicative.)
        for pair in out.energies.windows(2) {
            assert!(
                pair[1] <= pair[0] + 0.005 * pair[0].abs() + 1e-6,
                "energy rose beyond tolerance: {} -> {}",
                pair[0],
                pair[1]
            );
        }
        // A request above the contract bound is clamped.
        let clamped = grabcut(&rgb, &trimap, 50).unwrap();
        assert!(clamped.iterations <= 5, "contract iteration clamp missing");
    }

    #[test]
    fn grabcut_degenerate_and_empty_side_guards() {
        // Constant-color image: beta = 0, single-point GMM clusters —
        // the covariance regularization must keep everything finite.
        let (w, h) = (12, 12);
        let rgb = [
            Plane::from_vec(w, h, vec![0.5; w * h]),
            Plane::from_vec(w, h, vec![0.5; w * h]),
            Plane::from_vec(w, h, vec![0.5; w * h]),
        ];
        let trimap = rect_trimap(w, h, 3, 3, 9, 9);
        let out = grabcut(&rgb, &trimap, 5).unwrap();
        assert!(out.energies.iter().all(|e| e.is_finite()));
        // All-bg trimap: no fg side, input returned unchanged.
        let all_bg = vec![LABEL_BG; w * h];
        let out = grabcut(&rgb, &all_bg, 5).unwrap();
        assert_eq!(out.labels, all_bg);
        assert_eq!(out.iterations, 0);
    }

    #[test]
    fn grabcut_rejects_invalid_trimap_codes() {
        // Malformed trimap bytes are rejected up front with an error
        // naming the first offending index — never silently
        // reinterpreted as a probable label.
        let (w, h) = (8, 8);
        let (rgb, _) = two_region_fixture(w, h, 0x08AB_C07A_0000_0004);
        let mut trimap = rect_trimap(w, h, 2, 2, 6, 6);
        trimap[10] = 4; // first invalid code past LABEL_PROB_FG
        trimap[20] = 255;
        let err = grabcut(&rgb, &trimap, 5).unwrap_err();
        assert!(
            err.contains("invalid trimap code 4") && err.contains("index 10"),
            "error should name the first invalid code and index: {err}"
        );
        // All four valid codes together still succeed.
        let mut ok_trimap = rect_trimap(w, h, 2, 2, 6, 6);
        ok_trimap[0] = LABEL_FG;
        ok_trimap[1] = LABEL_PROB_BG;
        assert!(grabcut(&rgb, &ok_trimap, 5).is_ok());
    }

    #[test]
    fn grabcut_golden_and_determinism() {
        let (w, h) = (40, 40);
        let (rgb, _) = two_region_fixture(w, h, 0x08AB_C07A_0000_0001);
        let trimap = rect_trimap(w, h, w / 8, h / 8, w * 7 / 8, h * 7 / 8);
        let a = grabcut(&rgb, &trimap, 5).unwrap();
        let b = grabcut(&rgb, &trimap, 5).unwrap();
        assert_eq!(a.labels, b.labels, "two-run label mismatch");
        let ha = plane_hash(&labels_to_mask(&a.labels, w, h).data);
        let hb = plane_hash(&labels_to_mask(&b.labels, w, h).data);
        assert_eq!(ha, hb, "two-run hash mismatch");
        // Pinned AFTER the behavioral oracles above went green.
        assert_eq!(
            ha, 13898236568138461349u64,
            "golden hash drift — investigate before re-pinning (O-QA rule)"
        );
    }

    #[test]
    fn eigen_split_init_is_balanced_and_deterministic() {
        // sym_eigen3 sanity on a known matrix: diag(4, 1, 0.25) rotated
        // is overkill — use the diagonal directly; eigenvalues must come
        // back {4, 1, 0.25} and the principal axis must be x.
        let m = [[4.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 0.25]];
        let (evals, evecs) = sym_eigen3(&m);
        let mut k_max = 0;
        for k in 1..3 {
            if evals[k] > evals[k_max] {
                k_max = k;
            }
        }
        assert!((evals[k_max] - 4.0).abs() < 1e-12);
        assert!(evecs[k_max][0].abs() > 0.999);
        // Two clearly separated color blobs must land in different
        // components after the first split.
        let mut px = Vec::new();
        for i in 0..8 {
            let t = i as f64 * 0.01;
            px.push([0.1 + t, 0.1, 0.1]);
        }
        for i in 0..8 {
            let t = i as f64 * 0.01;
            px.push([0.9 - t, 0.9, 0.9]);
        }
        let members: Vec<u32> = (0..16).collect();
        let mut comp = vec![0u8; 16];
        init_components(&px, &members, &mut comp);
        for i in 0..8 {
            for j in 8..16 {
                assert_ne!(comp[i], comp[j], "blob pixels {i}/{j} share a component");
            }
        }
        let mut comp2 = vec![0u8; 16];
        init_components(&px, &members, &mut comp2);
        assert_eq!(comp, comp2, "init not deterministic");
    }

    #[test]
    fn grabcut_all_definite_trimap_is_fixed_point() {
        // No unknowns anywhere: the cut has nothing to decide, so the
        // output labels must equal the input EXACTLY — even when half
        // the definite labels oppose the color evidence (checkerboard
        // flip), because definite pins are never flipped and there are
        // no probable pixels to update.
        let (w, h) = (16, 16);
        let (rgb, truth) = two_region_fixture(w, h, 0x08AB_C07A_0000_0005);
        let mut trimap = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                let agree = (x + y) % 2 == 0;
                let fg = truth[i] == agree;
                trimap[i] = if fg { LABEL_FG } else { LABEL_BG };
            }
        }
        let out = grabcut(&rgb, &trimap, 5).unwrap();
        assert_eq!(
            out.labels, trimap,
            "definite-only trimap must be a fixed point"
        );
        assert!(
            out.iterations >= 1 && out.iterations <= 5,
            "definite-only trimap must still run the iteration loop"
        );
        assert_eq!(out.energies.len(), out.iterations);
        assert!(out.energies.iter().all(|e| e.is_finite()));
    }

    #[test]
    fn grabcut_point_mass_fg_seed_segments_exact_two_tone_patch() {
        // Foreground evidence = ONE definite pixel of an exact color:
        // the fg GMM is a single point-mass cluster sitting on the
        // covariance floor with K − 1 empty components. The noiseless
        // two-tone image makes the expected segmentation exact — the
        // whole red patch must join the seed through the n-links, and
        // every exact-blue pixel must stay background.
        let (w, h) = (16, 16);
        let blue = [0.1f32, 0.3, 0.7];
        let red = [0.8f32, 0.2, 0.2];
        let in_patch = |x: usize, y: usize| (5..10).contains(&x) && (5..10).contains(&y);
        let mut rgb = [Plane::new(w, h), Plane::new(w, h), Plane::new(w, h)];
        for y in 0..h {
            for x in 0..w {
                let c = if in_patch(x, y) { red } else { blue };
                for ch in 0..3 {
                    rgb[ch].data[y * w + x] = c[ch];
                }
            }
        }
        let mut trimap = vec![LABEL_PROB_BG; w * h];
        trimap[7 * w + 7] = LABEL_FG;
        trimap[0] = LABEL_BG;
        let out = grabcut(&rgb, &trimap, 5).unwrap();
        for y in 0..h {
            for x in 0..w {
                assert_eq!(
                    label_is_fg(out.labels[y * w + x]),
                    in_patch(x, y),
                    "wrong side at ({x}, {y})"
                );
            }
        }
        assert!(out.energies.iter().all(|e| e.is_finite()));
        let rerun = grabcut(&rgb, &trimap, 5).unwrap();
        assert_eq!(
            out.labels, rerun.labels,
            "degenerate point-mass GMM path not deterministic"
        );
    }

    #[test]
    fn grabcut_constant_image_all_probable_collapse_pinned() {
        // Pinned behavior. HONEST NOTE (wave-2 fix round): the previous
        // version of this pin documented the PRE-FIX truncation — raw
        // negative −log likelihoods dropped by `add_terminal`'s cap > 0
        // gate, which distorted the rother04 energy. T-links are now
        // shift-normalized per pixel (subtract m = min(D_bg, D_fg) from
        // both terminals; argmin-preserving, see the step-3 site). On a
        // CONSTANT image the observable outcome is unchanged, but for
        // the correct reason: both GMMs learn the identical point mass,
        // so D_bg == D_fg exactly, the shifted terminals are (0, 0), and
        // the graph legitimately carries no data preference. With no
        // definite seeds there are no pins either: max-flow is 0, the
        // source-side set is empty, and every probable pixel collapses
        // to the background side in one iteration (the fg side empties,
        // so the loop exits at the top of iteration 2). The case where
        // the fix DOES change labeling is
        // `grabcut_negative_tlinks_carry_data_preference` below.
        let (w, h) = (8, 8);
        let rgb = [
            Plane::from_vec(w, h, vec![0.5; w * h]),
            Plane::from_vec(w, h, vec![0.5; w * h]),
            Plane::from_vec(w, h, vec![0.5; w * h]),
        ];
        let mut trimap = vec![LABEL_PROB_BG; w * h];
        for l in trimap.iter_mut().skip(w * h / 2) {
            *l = LABEL_PROB_FG;
        }
        let out = grabcut(&rgb, &trimap, 5).unwrap();
        assert!(
            out.labels.iter().all(|&l| l == LABEL_PROB_BG),
            "pinned collapse-to-background behavior changed"
        );
        assert_eq!(out.iterations, 1);
        assert_eq!(out.energies.len(), 1);
        assert!(out.energies[0].is_finite());
    }

    #[test]
    fn grabcut_negative_tlinks_carry_data_preference() {
        // Wave-2 t-link ruling, the case the fix changes: a NEAR-tone
        // noiseless image with an all-probable trimap aligned to the
        // tones and NO definite seeds. The tones differ by 3e-4 per
        // channel — comparable to the covariance-floor σ = √COV_REG ≈
        // 3.9e-4 — so each point-mass GMM sees the WRONG tone at a
        // small Mahalanobis distance (q ≈ 1.8) and BOTH data terms are
        // negative on every pixel (iteration-1 census: right side
        // ≈ −20.81, wrong side ≈ −19.91). Pre-fix, BOTH t-links of
        // every probable pixel were truncated to 0, max-flow was 0,
        // and the whole image collapsed to background (verified by
        // reverting the shift-normalization in a scratch copy: this
        // fixture collapses 0/64 foreground; the previous WELL-
        // SEPARATED two-tone fixture did NOT discriminate — there the
        // wrong-side density underflows to the 1e-100 floor, +230.26,
        // so one t-link stayed positive and carried the preference
        // even pre-fix). Post-fix the shifted terminals keep the
        // per-pixel D_bg − D_fg preference (≈ 0.90 here), so each tone
        // stays on its likelier side — the two-tone labeling is a
        // fixed point.
        let (w, h) = (8, 8);
        let a = [0.5f32, 0.5, 0.5];
        let b = [0.5003f32, 0.5003, 0.5003];
        let mut rgb = [Plane::new(w, h), Plane::new(w, h), Plane::new(w, h)];
        let mut trimap = vec![LABEL_PROB_BG; w * h];
        for y in 0..h {
            for x in 0..w {
                let c = if x < w / 2 { a } else { b };
                for ch in 0..3 {
                    rgb[ch].data[y * w + x] = c[ch];
                }
                if x >= w / 2 {
                    trimap[y * w + x] = LABEL_PROB_FG;
                }
            }
        }
        let out = grabcut(&rgb, &trimap, 5).unwrap();
        for y in 0..h {
            for x in 0..w {
                assert_eq!(
                    label_is_fg(out.labels[y * w + x]),
                    x >= w / 2,
                    "tone lost its side at ({x}, {y})"
                );
            }
        }
        assert!(out.energies.iter().all(|e| e.is_finite()));
        let rerun = grabcut(&rgb, &trimap, 5).unwrap();
        assert_eq!(out.labels, rerun.labels, "negative-t-link path replay");
    }

    #[test]
    fn grabcut_one_pixel_and_single_row_column_images() {
        // 1×1: one pixel can never populate both sides — the input
        // comes back unchanged with an empty trace, for every code.
        for code in [LABEL_BG, LABEL_FG, LABEL_PROB_BG, LABEL_PROB_FG] {
            let rgb = [
                Plane::from_vec(1, 1, vec![0.4]),
                Plane::from_vec(1, 1, vec![0.5]),
                Plane::from_vec(1, 1, vec![0.6]),
            ];
            let out = grabcut(&rgb, &[code], 5).unwrap();
            assert_eq!(out.labels, vec![code]);
            assert_eq!(out.iterations, 0);
            assert!(out.energies.is_empty());
        }
        // 8×1 row (E n-links only) and 1×8 column (S n-links only):
        // exact two-tone gray with definite endpoints must split at the
        // color boundary — the strong same-tone n-links (γ = 50) hold
        // each half together while the contrast-suppressed boundary
        // link is the cheap place to cut.
        let vals: Vec<f32> = (0..8).map(|i| if i < 4 { 0.1 } else { 0.9 }).collect();
        let mut trimap = vec![LABEL_PROB_BG; 8];
        trimap[0] = LABEL_BG;
        trimap[7] = LABEL_FG;
        for (w, h) in [(8usize, 1usize), (1usize, 8usize)] {
            let rgb = [
                Plane::from_vec(w, h, vals.clone()),
                Plane::from_vec(w, h, vals.clone()),
                Plane::from_vec(w, h, vals.clone()),
            ];
            let out = grabcut(&rgb, &trimap, 5).unwrap();
            for i in 0..8 {
                assert_eq!(
                    label_is_fg(out.labels[i]),
                    i >= 4,
                    "two-tone split wrong at {i} ({w}x{h})"
                );
            }
            assert!(out.energies.iter().all(|e| e.is_finite()));
        }
    }

    #[test]
    fn grabcut_rect_trimap_clamps_and_construction_paths_agree() {
        // rect_trimap with a rect running past both edges must clamp to
        // the plane; the clamped helper output and a hand-built label
        // plane are two construction paths for the SAME fixture and
        // must produce bit-identical results.
        let (w, h) = (10, 8);
        let rect = rect_trimap(w, h, 3, 2, w + 5, h + 9);
        let mut hand = vec![LABEL_BG; w * h];
        for y in 2..h {
            for x in 3..w {
                hand[y * w + x] = LABEL_PROB_FG;
            }
        }
        assert_eq!(rect, hand, "clamped rect trimap mismatch");
        let (rgb, _) = two_region_fixture(w, h, 0x08AB_C07A_0000_0006);
        let a = grabcut(&rgb, &rect, 5).unwrap();
        let b = grabcut(&rgb, &hand, 5).unwrap();
        assert_eq!(a.labels, b.labels, "construction paths diverged");
        assert_eq!(a.iterations, b.iterations);
        for (ea, eb) in a.energies.iter().zip(&b.energies) {
            assert_eq!(ea.to_bits(), eb.to_bits(), "energy trace diverged");
        }
        let ha = plane_hash(&labels_to_mask(&a.labels, w, h).data);
        let hb = plane_hash(&labels_to_mask(&b.labels, w, h).data);
        assert_eq!(ha, hb);
    }

    #[test]
    fn grabcut_nan_input_pinned_current_behavior() {
        // Documents current behavior, not a contract. A NaN sample
        // poisons β (NaN propagates through the neighbor-distance mean,
        // so every n-link weight is NaN and `wgt > 0.0` is false — NO
        // n-links anywhere) and any GMM statistics it touches; t-links
        // stay finite because `p.max(LIKELIHOOD_FLOOR)` maps a NaN
        // mixture density to the floor. The run completes with valid
        // label codes, preserved definite seeds, finite energies, and
        // bit-identical replays.
        let (w, h) = (8, 8);
        let (mut rgb, _) = two_region_fixture(w, h, 0x08AB_C07A_0000_0007);
        rgb[1].data[3 * w + 4] = f32::NAN;
        let mut trimap = rect_trimap(w, h, 2, 2, 6, 6);
        trimap[0] = LABEL_BG;
        trimap[w + 1] = LABEL_FG;
        let a = grabcut(&rgb, &trimap, 5).unwrap();
        let b = grabcut(&rgb, &trimap, 5).unwrap();
        assert_eq!(a.labels, b.labels, "NaN input broke replay determinism");
        assert_eq!(a.iterations, b.iterations);
        for (ea, eb) in a.energies.iter().zip(&b.energies) {
            assert_eq!(ea.to_bits(), eb.to_bits(), "NaN energy trace diverged");
        }
        assert!(a.labels.iter().all(|&l| l <= LABEL_PROB_FG));
        assert_eq!(a.labels[0], LABEL_BG, "definite bg seed lost under NaN");
        assert_eq!(a.labels[w + 1], LABEL_FG, "definite fg seed lost under NaN");
        assert!(a.energies.iter().all(|e| e.is_finite()));
    }
}
