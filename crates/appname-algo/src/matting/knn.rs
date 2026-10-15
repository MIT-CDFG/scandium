// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Portions ported from PyMatting (https://github.com/pymatting/pymatting,
// commit fcda1e52fdeb47d03ec2478c1aeafd0b7bd387de, vendored at
// appname-proto/vendor/pymatting/): pymatting/laplacian/knn_laplacian.py,
// pymatting/alpha/estimate_alpha_knn.py, pymatting/laplacian/laplacian.py
// (make_linear_system), pymatting/util/util.py (normalize_rows),
// pymatting/util/kdtree.py (k-d tree scheme: widest-dimension splits —
// at the dimension MIDPOINT upstream, at the MEDIAN in this port (see
// the determinism note below) — leaf buckets, bound-pruned exact
// k-NN), and
// pymatting/preconditioner/jacobi.py (diagonal preconditioner shape).
// MIT License, Copyright (c) 2020 PyMatting. License text carried in
// THIRD-PARTY-NOTICES.md and vendor/pymatting/LICENSE.md.

//! KNN matting (chen12) — the `alpha_matting` contract's `quality_option`
//! `knn_matting` (level L2, basis chen12).
//!
//! Spec: ES §13.4 Select-and-Mask (commit-quality tier; verified against
//! current text — the contract's older `spec_refs` are noted stale in
//! [`crate::matting`]); ALGORITHMS.md §7.B "KNN matting [R-chen12] (L2)
//! covers non-local strands (hair against busy backgrounds) as a quality
//! option" and §7.E ladder step 2 (`quality:best` commit).
//!
//! Route (Part 47): PORT of the vendored pymatting KNN implementation —
//! `vendor/pymatting/` at the pin DOES ship `estimate_alpha_knn` +
//! `knn_laplacian`, so this port rides the same MIT PROVENANCE and item-1
//! obligations the wave-1 closed-form port already carries. No clean-room
//! re-derivation was needed; the chen12 paper formulation is what the
//! ported code implements (per-pixel feature vectors RGB + weighted
//! normalized spatial xy, k nearest neighbors in feature space, affinity
//! kernel, graph Laplacian, penalty-constrained solve
//! `(L + λ C) α = λ v` with `C = Diag(is_known)`, `v = is_fg`).
//!
//! Kernel choice: upstream defaults to the `binary` kernel (affinity 1
//! per neighbor edge) and documents the `soft` kernel
//! `1 − ‖X(i)−X(j)‖₁ / C` (the chen12 paper affinity `A(i,j) =
//! 1 − ‖X(i)−X(j)‖/C`) as near-identical; both are ported and the
//! DEFAULT here is [`KnnKernel::Soft`] — the contract row's basis is the
//! chen12 paper, so the paper's affinity is the default surface and the
//! upstream-default binary kernel stays available.
//!
//! Solver: the crate's existing [`crate::sparse`] ichol-preconditioned CG
//! (wave-1 port) on the symmetrically Jacobi-scaled system, with the
//! wave-2 deterministic fallback pattern of `edge_aware/wls.rs` MIRRORED
//! (not touched): when the thresholded ichol factor overruns its
//! pattern-sized budget on a degenerate system, the solve falls back to
//! CG with the identity preconditioner on the unit-diagonal scaled
//! system — which IS the upstream `jacobi.py` diagonal preconditioner —
//! with a deterministically raised iteration cap. HONEST NOTE (upstream
//! behavior preserved): `normalize_rows` makes `W` row-stochastic and
//! therefore `L = I − W` slightly NONSYMMETRIC; upstream runs CG on it
//! regardless (with the Jacobi preconditioner) and converges in practice.
//! The port keeps that behavior; determinism is unaffected (fixed
//! iteration order, no threading).
//!
//! Determinism (D-48): the k-NN index is a pure function of the feature
//! set — widest-dimension splits (ties: lowest dimension index), median
//! split with the total order (coordinate `total_cmp`, then point index),
//! and neighbor selection by the TOTAL order (squared distance
//! `total_cmp`, then point index). DOCUMENTED DEVIATION (index scheme):
//! upstream `kdtree.py` splits the widest dimension at its MIDPOINT
//! (`lo + 0.5·length`, leaf-size-clamped partition); this port splits at
//! the MEDIAN under the total order. Both trees are exact-k-NN indexes —
//! only the partition boundaries differ, so neighbor RESULTS are
//! identical under the total tie-break — and the median rule gives
//! balanced, comparison-count-deterministic recursion. Equal-distance
//! neighbor sets are index-canonical and input-permutation maps commute
//! with selection. Two runs are bit-identical; a pinned golden locks
//! the stream.

use crate::plane::Plane;
use crate::sparse::{cg, ichol, Csr};

use super::trimap_split;

/// Affinity kernel for the KNN weight matrix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KnnKernel {
    /// Upstream default: every neighbor edge carries weight 1.
    Binary,
    /// chen12 paper affinity `1 − ‖f_i − f_j‖₁ / dim` (upstream `soft`).
    Soft,
}

/// Parameters of [`knn_laplacian`] (upstream defaults, kernel per module
/// docs).
#[derive(Clone, Debug)]
pub struct KnnParams {
    /// Neighbor counts of the merged k-NN passes (upstream `[20, 10]`).
    pub n_neighbors: Vec<usize>,
    /// Spatial-coordinate weight per pass (upstream `[2.0, 0.1]`).
    pub distance_weights: Vec<f64>,
    /// Affinity kernel (default [`KnnKernel::Soft`], see module docs).
    pub kernel: KnnKernel,
    /// Constraint penalty λ of `(L + λC) α = λ v` (upstream 100.0).
    pub lambda: f64,
    /// CG relative tolerance (crate matting default 1e-8 class).
    pub cg_rtol: f64,
    /// CG iteration cap on the preconditioned primary path.
    pub cg_maxiter: usize,
}

impl Default for KnnParams {
    fn default() -> KnnParams {
        KnnParams {
            n_neighbors: vec![20, 10],
            distance_weights: vec![2.0, 0.1],
            kernel: KnnKernel::Soft,
            lambda: 100.0,
            cg_rtol: 1e-8,
            cg_maxiter: 2000,
        }
    }
}

/// Iteration cap for the unpreconditioned fallback path (mirrors the
/// `edge_aware/wls.rs` wave-2 pattern: a fixed, deterministic raise).
const CG_FALLBACK_MAXITER: usize = 20_000;

/// Feature-space dimension: RGB + weighted normalized x, y.
const FEATURE_DIM: usize = 5;

// ---------------------------------------------------------------------------
// Deterministic exact k-NN index
// ---------------------------------------------------------------------------

/// Node of the k-d tree (leaf = contiguous `order` slice).
enum Node {
    Split {
        dim: u16,
        val: f64,
        left: u32,
        right: u32,
    },
    Leaf {
        start: u32,
        end: u32,
    },
}

/// Leaf bucket capacity (upstream `min_leaf_size` class).
const LEAF_CAP: usize = 8;

/// Immutable exact-k-NN k-d tree over `n` points in `dim`-d space, with
/// TOTAL tie-breaking: neighbors are the k smallest under the
/// lexicographic order (squared distance by `total_cmp`, then original
/// point index). This is the "small own k-NN index" of the fence rules —
/// the exemplar-crate tree is a top-L-leaf ANN candidate walk (different
/// contract) and stays untouched.
struct KnnTree {
    dim: usize,
    coords: Vec<f64>,
    order: Vec<u32>,
    nodes: Vec<Node>,
    root: u32,
}

impl KnnTree {
    fn build(coords: Vec<f64>, dim: usize) -> KnnTree {
        assert!(dim > 0);
        let n = coords.len() / dim;
        let mut order: Vec<u32> = (0..n as u32).collect();
        let mut nodes = Vec::new();
        let root = if n == 0 {
            nodes.push(Node::Leaf { start: 0, end: 0 });
            0
        } else {
            Self::build_rec(&coords, dim, &mut order, 0, n, &mut nodes)
        };
        KnnTree {
            dim,
            coords,
            order,
            nodes,
            root,
        }
    }

    fn build_rec(
        coords: &[f64],
        dim: usize,
        order: &mut [u32],
        start: usize,
        end: usize,
        nodes: &mut Vec<Node>,
    ) -> u32 {
        let count = end - start;
        if count <= LEAF_CAP {
            nodes.push(Node::Leaf {
                start: start as u32,
                end: end as u32,
            });
            return (nodes.len() - 1) as u32;
        }
        // Widest dimension over the slice (ties: lowest dim index).
        let mut best_dim = 0usize;
        let mut best_spread = -1.0f64;
        for d in 0..dim {
            let mut lo = f64::INFINITY;
            let mut hi = f64::NEG_INFINITY;
            for &pi in &order[start..end] {
                let v = coords[pi as usize * dim + d];
                lo = lo.min(v);
                hi = hi.max(v);
            }
            let spread = hi - lo;
            if spread > best_spread {
                best_spread = spread;
                best_dim = d;
            }
        }
        // Median split with the total order (coordinate bits, then index).
        order[start..end].sort_by(|&a, &b| {
            coords[a as usize * dim + best_dim]
                .total_cmp(&coords[b as usize * dim + best_dim])
                .then(a.cmp(&b))
        });
        let mid = start + count / 2;
        let val = coords[order[mid] as usize * dim + best_dim];
        let left = Self::build_rec(coords, dim, order, start, mid, nodes);
        let right = Self::build_rec(coords, dim, order, mid, end, nodes);
        nodes.push(Node::Split {
            dim: best_dim as u16,
            val,
            left,
            right,
        });
        (nodes.len() - 1) as u32
    }

    /// Exact k nearest neighbors of `q` under the total order
    /// (squared distance `total_cmp`, then index), ascending. `best` is
    /// the caller's scratch, returned sorted; the query point itself (at
    /// distance 0) participates like upstream.
    fn knn(&self, q: &[f64], k: usize, best: &mut Vec<(f64, u32)>) {
        best.clear();
        if k == 0 || self.order.is_empty() {
            return;
        }
        self.search(self.root, q, k, best);
    }

    fn search(&self, node: u32, q: &[f64], k: usize, best: &mut Vec<(f64, u32)>) {
        match &self.nodes[node as usize] {
            Node::Leaf { start, end } => {
                for &pi in &self.order[*start as usize..*end as usize] {
                    let base = pi as usize * self.dim;
                    let mut d = 0.0f64;
                    for (qc, cc) in q.iter().zip(&self.coords[base..base + self.dim]) {
                        let diff = qc - cc;
                        d += diff * diff;
                    }
                    let cand = (d, pi);
                    if best.len() < k {
                        let pos = best.partition_point(|&(bd, bi)| {
                            bd.total_cmp(&cand.0).then(bi.cmp(&cand.1)).is_lt()
                        });
                        best.insert(pos, cand);
                    } else {
                        let worst = best[k - 1];
                        if cand
                            .0
                            .total_cmp(&worst.0)
                            .then(cand.1.cmp(&worst.1))
                            .is_lt()
                        {
                            best.pop();
                            let pos = best.partition_point(|&(bd, bi)| {
                                bd.total_cmp(&cand.0).then(bi.cmp(&cand.1)).is_lt()
                            });
                            best.insert(pos, cand);
                        }
                    }
                }
            }
            Node::Split {
                dim,
                val,
                left,
                right,
            } => {
                let d = *dim as usize;
                let diff = q[d] - *val;
                let (near, far) = if diff < 0.0 {
                    (*left, *right)
                } else {
                    (*right, *left)
                };
                self.search(near, q, k, best);
                // Visit the far side unless it provably cannot improve the
                // worst kept entry: prune only on a STRICTLY larger plane
                // bound, so equal-distance ties on the far side are still
                // found and resolved by index (total tie-break).
                if best.len() < k || diff * diff <= best[best.len() - 1].0 {
                    self.search(far, q, k, best);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// KNN Laplacian
// ---------------------------------------------------------------------------

/// Per-pass feature vectors: `[r, g, b, dw·x/(w−1), dw·y/(h−1)]`
/// (upstream `linspace(0, 1, w)`; degenerate 1-wide axes contribute 0).
fn features(rgb: &[Plane; 3], distance_weight: f64) -> Vec<f64> {
    let w = rgb[0].w;
    let h = rgb[0].h;
    let n = w * h;
    let xs = |x: usize| {
        if w > 1 {
            x as f64 / (w - 1) as f64
        } else {
            0.0
        }
    };
    let ys = |y: usize| {
        if h > 1 {
            y as f64 / (h - 1) as f64
        } else {
            0.0
        }
    };
    let mut f = vec![0f64; n * FEATURE_DIM];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            f[i * FEATURE_DIM] = rgb[0].data[i] as f64;
            f[i * FEATURE_DIM + 1] = rgb[1].data[i] as f64;
            f[i * FEATURE_DIM + 2] = rgb[2].data[i] as f64;
            f[i * FEATURE_DIM + 3] = distance_weight * xs(x);
            f[i * FEATURE_DIM + 4] = distance_weight * ys(y);
        }
    }
    f
}

/// Build the KNN matting Laplacian `L = I − normalize_rows(W + Wᵀ)`
/// (port of pymatting `knn_laplacian`): merged k-NN passes per
/// `params.n_neighbors` / `params.distance_weights`, affinity per
/// `params.kernel`, symmetrized by adding the transpose (duplicate
/// entries sum, exactly the upstream COO construction), then row
/// normalization (rows with sum ≤ 0 left as-is). DOCUMENTED
/// MICRO-DEVIATION (guard, benign hardening — NOT upstream-equivalent):
/// upstream `normalize_rows(threshold=0.0)` only skips rows with sum
/// `< 0` and would divide by zero on a row summing to exactly 0; this
/// port's `> 0.0` guard also leaves an exactly-zero row unscaled.
/// Binary-kernel rows always sum ≥ 1 (the zero-distance self/duplicate
/// edge), so there the guards coincide; soft-kernel affinities can go
/// negative (`l1 > FEATURE_DIM` is reachable at large distance
/// weights), where an exactly-zero sum is degenerate-but-conceivable —
/// the hardened guard keeps that case finite instead of upstream's
/// division by zero.
pub fn knn_laplacian(rgb: &[Plane; 3], params: &KnnParams) -> Csr {
    let n = rgb[0].data.len();
    // COO triplets of W (pre-symmetrization).
    let mut rows: Vec<u32> = Vec::new();
    let mut cols: Vec<u32> = Vec::new();
    let mut vals: Vec<f64> = Vec::new();
    let mut best: Vec<(f64, u32)> = Vec::new();
    for (&k, &dw) in params.n_neighbors.iter().zip(&params.distance_weights) {
        let k = k.min(n);
        if k == 0 {
            continue;
        }
        let f = features(rgb, dw);
        let tree = KnnTree::build(f.clone(), FEATURE_DIM);
        for i in 0..n {
            let q = &f[i * FEATURE_DIM..(i + 1) * FEATURE_DIM];
            tree.knn(q, k, &mut best);
            for &(_, j) in best.iter() {
                let wij = match params.kernel {
                    KnnKernel::Binary => 1.0,
                    KnnKernel::Soft => {
                        let mut l1 = 0.0f64;
                        for c in 0..FEATURE_DIM {
                            l1 += (q[c] - f[j as usize * FEATURE_DIM + c]).abs();
                        }
                        1.0 - l1 / FEATURE_DIM as f64
                    }
                };
                rows.push(i as u32);
                cols.push(j);
                vals.push(wij);
            }
        }
    }
    // Symmetrize: append the transpose triplets (duplicates sum below).
    let m = rows.len();
    for t in 0..m {
        rows.push(cols[t]);
        cols.push(rows[t]);
        vals.push(vals[t]);
    }
    // COO -> CSR with duplicate summing: counting sort by row, then an
    // in-row sort by column and merge (deterministic, no hashing).
    let mut counts = vec![0usize; n + 1];
    for &r in &rows {
        counts[r as usize + 1] += 1;
    }
    for i in 0..n {
        counts[i + 1] += counts[i];
    }
    let mut slot = counts.clone();
    let total = rows.len();
    let mut col_of = vec![0u32; total];
    let mut val_of = vec![0f64; total];
    for t in 0..total {
        let r = rows[t] as usize;
        let dst = slot[r];
        col_of[dst] = cols[t];
        val_of[dst] = vals[t];
        slot[r] += 1;
    }
    // Row sums of the merged W for normalization, then emit L = I − D⁻¹W.
    let mut indptr = vec![0usize];
    let mut indices: Vec<usize> = Vec::new();
    let mut values: Vec<f64> = Vec::new();
    let mut row_entries: Vec<(u32, f64)> = Vec::new();
    for i in 0..n {
        row_entries.clear();
        let (a, b) = (counts[i], counts[i + 1]);
        let mut idx: Vec<usize> = (a..b).collect();
        idx.sort_by_key(|&t| col_of[t]);
        for t in idx {
            match row_entries.last_mut() {
                Some(last) if last.0 == col_of[t] => last.1 += val_of[t],
                _ => row_entries.push((col_of[t], val_of[t])),
            }
        }
        let row_sum: f64 = row_entries.iter().map(|&(_, v)| v).sum();
        let inv = if row_sum > 0.0 { 1.0 / row_sum } else { 1.0 };
        // L row: diagonal 1 − w_ii/D, off-diagonals −w_ij/D, ascending
        // column order with the diagonal merged in place.
        let mut wrote_diag = false;
        for &(j, v) in row_entries.iter() {
            let j = j as usize;
            if j == i {
                indices.push(j);
                values.push(1.0 - v * inv);
                wrote_diag = true;
            } else {
                if !wrote_diag && j > i {
                    indices.push(i);
                    values.push(1.0);
                    wrote_diag = true;
                }
                indices.push(j);
                values.push(-v * inv);
            }
        }
        if !wrote_diag {
            indices.push(i);
            values.push(1.0);
        }
        indptr.push(indices.len());
    }
    Csr {
        n,
        indptr,
        indices,
        values,
    }
}

/// Solve `A x = b` with the crate's ichol-CG primary path on the
/// symmetrically Jacobi-scaled system, falling back to identity-
/// preconditioned CG (≡ the upstream `jacobi.py` diagonal preconditioner
/// on the unit-diagonal scaled system) when ichol overruns its
/// pattern-sized budget — the `edge_aware/wls.rs` wave-2 pattern,
/// mirrored here without touching that module. Deterministic: ichol is
/// deterministic, so a given system always takes the same branch.
fn solve_scaled(a: &Csr, b: &[f64], rtol: f64, maxiter: usize) -> Result<Vec<f64>, String> {
    let n = a.n;
    let scale: Vec<f64> = (0..n)
        .map(|k| {
            let row = &a.values[a.indptr[k]..a.indptr[k + 1]];
            let cols = &a.indices[a.indptr[k]..a.indptr[k + 1]];
            let d = cols
                .iter()
                .position(|&c| c == k)
                .map(|i| row[i])
                .unwrap_or(1.0);
            if d > 0.0 {
                1.0 / d.sqrt()
            } else {
                1.0
            }
        })
        .collect();
    let mut scaled = a.clone();
    for row in 0..n {
        for idx in scaled.indptr[row]..scaled.indptr[row + 1] {
            scaled.values[idx] *= scale[row] * scale[scaled.indices[idx]];
        }
    }
    let bs: Vec<f64> = b.iter().enumerate().map(|(k, &v)| v * scale[k]).collect();
    let y = match ichol(&scaled.to_csc(), 0.01) {
        Ok(factor) => cg(&scaled, &bs, |r| factor.apply(r), rtol, 0.0, maxiter)?,
        Err(e) if e == "ichol: max_nnz overflow" => {
            cg(&scaled, &bs, |r| r.to_vec(), rtol, 0.0, CG_FALLBACK_MAXITER)?
        }
        Err(e) => return Err(e),
    };
    Ok(y.iter().enumerate().map(|(k, &v)| v * scale[k]).collect())
}

/// KNN alpha matting (port of pymatting `estimate_alpha_knn` +
/// `make_linear_system`): `rgb` in [0, 1], `trimap` with the 0.1/0.9
/// split. Returns alpha clipped to [0, 1] with definite trimap regions
/// PINNED exactly (deviation from upstream, which only clips: the
/// Select & Mask contract treats definite trimap pixels as hard
/// constraints, matching the closed-form path's behavior).
pub fn estimate_alpha_knn(
    rgb: &[Plane; 3],
    trimap: &Plane,
    params: &KnnParams,
) -> Result<Plane, String> {
    let w = trimap.w;
    let h = trimap.h;
    if rgb[0].w != w || rgb[0].h != h {
        return Err(format!(
            "estimate_alpha_knn: rgb {}x{} vs trimap {}x{} dimension mismatch",
            rgb[0].w, rgb[0].h, w, h
        ));
    }
    if w == 0 || h == 0 {
        return Err("estimate_alpha_knn: empty plane".to_string());
    }
    if params.n_neighbors.len() != params.distance_weights.len() {
        return Err("estimate_alpha_knn: n_neighbors/distance_weights length mismatch".to_string());
    }
    if !(params.lambda.is_finite() && params.lambda > 0.0) {
        return Err(format!(
            "estimate_alpha_knn: lambda must be finite and > 0 (got {})",
            params.lambda
        ));
    }
    let split = trimap_split(trimap);
    if !split.is_unknown.iter().any(|&u| u) {
        let data = split
            .is_fg
            .iter()
            .map(|&fg| if fg { 1.0 } else { 0.0 })
            .collect();
        return Ok(Plane::from_vec(w, h, data));
    }
    // Zero-RHS constraint-class short-circuit (lane ruling, Row B tester
    // defect; DELIBERATE, DOCUMENTED DEVIATION from upstream): with NO
    // foreground constraint, `b = λ·is_fg ≡ 0` and x = 0 is an EXACT
    // solution of `(L + λC) x = 0` (definite pixels keep their trimap
    // values — all background here). Symmetrically, with NO background
    // constraint, `v = is_fg = is_known`, and because every Laplacian row
    // sums to zero, `(L + λC)·1 = λ·is_known = λ·v` — x = 1 is exact.
    // Uniqueness is conditional for THIS engine: the symmetrized k-NN
    // graph is not guaranteed connected, and an orphaned component with
    // no edge to any known pixel makes the system singular there — the
    // returned 0/1 is then a deterministic choice among exact solutions,
    // the same convention the all-unknown case below already documents.
    // Both cases fire BEFORE the solver — deterministic, no parameter
    // dependence. Upstream (vendored pymatting `solver/cg.py`) reaches CG
    // with the zero RHS and strict `<` guards against `atol = 0`: the
    // exact x = 0 iterate is rejected, the first step divides 0/0, and
    // the solve runs to its cap and errors — the short-circuit replaces
    // that consumer-seam hazard with the exact answer. An all-unknown
    // trimap (both classes empty) resolves through the no-foreground arm
    // to the all-zero alpha (an exact solution of the then-singular
    // `L x = 0` system, chosen deterministically).
    let any_fg = split.is_fg.iter().any(|&f| f);
    let any_bg = split.is_bg.iter().any(|&b| b);
    if !any_fg || !any_bg {
        let unknown_val: f32 = if any_fg { 1.0 } else { 0.0 };
        let data = split
            .is_fg
            .iter()
            .zip(&split.is_bg)
            .map(|(&fg, &bg)| {
                if fg {
                    1.0
                } else if bg {
                    0.0
                } else {
                    unknown_val
                }
            })
            .collect();
        return Ok(Plane::from_vec(w, h, data));
    }
    // A = L + λC, b = λ·is_fg (upstream make_linear_system).
    let mut a = knn_laplacian(rgb, params);
    for i in 0..a.n {
        if split.is_known[i] {
            for idx in a.indptr[i]..a.indptr[i + 1] {
                if a.indices[idx] == i {
                    a.values[idx] += params.lambda;
                }
            }
        }
    }
    let b: Vec<f64> = split
        .is_fg
        .iter()
        .map(|&fg| if fg { params.lambda } else { 0.0 })
        .collect();
    let x = solve_scaled(&a, &b, params.cg_rtol, params.cg_maxiter)?;
    let mut alpha = Plane::new(w, h);
    // Index loop kept: i addresses the split masks, x and alpha in
    // lockstep (crate precedent for multi-array index loops).
    #[allow(clippy::needless_range_loop)]
    for i in 0..w * h {
        alpha.data[i] = if split.is_fg[i] {
            1.0
        } else if split.is_bg[i] {
            0.0
        } else {
            x[i].clamp(0.0, 1.0) as f32
        };
    }
    Ok(alpha)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matting::fixtures::two_color_band;
    use crate::plane::plane_hash;

    #[test]
    fn knn_alpha_recovers_two_color_band() {
        // The shared item-1 fixture, for cross-tier comparability with the
        // closed-form test (its bound is 0.05; KNN's non-local averaging
        // is a different smoothness prior — measured worst deviation on
        // this fixture 0.0758, bound set honestly above measurement).
        let (rgb, trimap, truth) = two_color_band(16, 8);
        let alpha = estimate_alpha_knn(&rgb, &trimap, &KnnParams::default()).unwrap();
        let mut worst = 0f64;
        // Index loop kept: the assertions reference several arrays by i.
        #[allow(clippy::needless_range_loop)]
        for i in 0..truth.len() {
            worst = worst.max((alpha.data[i] as f64 - truth[i]).abs());
        }
        assert!(worst < 0.10, "worst |alpha - truth| = {worst}");
    }

    #[test]
    fn knn_definite_regions_exact() {
        let (rgb, trimap, _) = two_color_band(16, 8);
        let alpha = estimate_alpha_knn(&rgb, &trimap, &KnnParams::default()).unwrap();
        for (i, &t) in trimap.data.iter().enumerate() {
            if t <= 0.1 {
                assert_eq!(alpha.data[i], 0.0, "bg pixel {i} not pinned");
            }
            if t >= 0.9 {
                assert_eq!(alpha.data[i], 1.0, "fg pixel {i} not pinned");
            }
        }
    }

    #[test]
    fn knn_laplacian_rows_sum_to_zero() {
        // L = I − normalize_rows(W): every row with positive weight mass
        // sums to exactly (1 − Σ w/D) = 0 up to summation rounding.
        let (rgb, _, _) = two_color_band(12, 8);
        for kernel in [KnnKernel::Binary, KnnKernel::Soft] {
            let params = KnnParams {
                kernel,
                ..KnnParams::default()
            };
            let l = knn_laplacian(&rgb, &params);
            let ones = vec![1.0; l.n];
            let mut y = vec![0.0; l.n];
            l.matvec(&ones, &mut y);
            for (i, v) in y.iter().enumerate() {
                assert!(v.abs() < 1e-12, "{kernel:?} row {i} sums to {v}");
            }
        }
    }

    #[test]
    fn knn_index_total_tiebreak_matches_bruteforce_under_permutation() {
        // 2-d point set with MANY exact distance ties (integer grid ring
        // around each query). The tree must return exactly the k smallest
        // under (distance, index) — verified against the brute-force total
        // order — and after permuting the point order the selected
        // NEIGHBOR IDENTITIES must be the permuted images of the
        // canonical selection re-broken by the new indices, i.e. the
        // rule "lowest index wins ties" holds in both layouts.
        let pts: Vec<(f64, f64)> = vec![
            (1.0, 0.0),
            (0.0, 1.0),
            (-1.0, 0.0),
            (0.0, -1.0),
            (2.0, 0.0),
            (0.0, 2.0),
            (1.0, 1.0),
            (-1.0, -1.0),
            (3.0, 4.0),
            (4.0, 3.0),
        ];
        let flat =
            |ps: &[(f64, f64)]| -> Vec<f64> { ps.iter().flat_map(|&(x, y)| [x, y]).collect() };
        let brute = |ps: &[(f64, f64)], q: (f64, f64), k: usize| -> Vec<(f64, u32)> {
            let mut all: Vec<(f64, u32)> = ps
                .iter()
                .enumerate()
                .map(|(i, &(x, y))| {
                    let (dx, dy) = (x - q.0, y - q.1);
                    (dx * dx + dy * dy, i as u32)
                })
                .collect();
            all.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            all.truncate(k);
            all
        };
        let mut best = Vec::new();
        for k in [1usize, 2, 3, 4, 6] {
            let tree = KnnTree::build(flat(&pts), 2);
            tree.knn(&[0.0, 0.0], k, &mut best);
            assert_eq!(best, brute(&pts, (0.0, 0.0), k), "k={k} canonical order");
            // Reversed layout: same points, indices reversed.
            let rev: Vec<(f64, f64)> = pts.iter().rev().copied().collect();
            let tree_r = KnnTree::build(flat(&rev), 2);
            tree_r.knn(&[0.0, 0.0], k, &mut best);
            assert_eq!(best, brute(&rev, (0.0, 0.0), k), "k={k} reversed order");
        }
    }

    #[test]
    fn knn_alpha_constant_image_stays_valid_and_deterministic() {
        // Degenerate feature space: constant color (all color coordinates
        // equal — distances driven by the spatial terms alone, maximal tie
        // pressure). Whatever solve path the system takes, the result must
        // be a valid clipped alpha with definite regions pinned, and two
        // runs must be bit-identical.
        let w = 12;
        let h = 8;
        let rgb = [
            Plane::from_vec(w, h, vec![0.5; w * h]),
            Plane::from_vec(w, h, vec![0.5; w * h]),
            Plane::from_vec(w, h, vec![0.5; w * h]),
        ];
        let mut trimap = Plane::from_vec(w, h, vec![0.5; w * h]);
        for y in 0..h {
            trimap.data[y * w] = 0.0;
            trimap.data[y * w + w - 1] = 1.0;
        }
        let a = estimate_alpha_knn(&rgb, &trimap, &KnnParams::default()).unwrap();
        assert!(a.data.iter().all(|&v| (0.0..=1.0).contains(&v)));
        for y in 0..h {
            assert_eq!(a.data[y * w], 0.0);
            assert_eq!(a.data[y * w + w - 1], 1.0);
        }
        let b = estimate_alpha_knn(&rgb, &trimap, &KnnParams::default()).unwrap();
        assert_eq!(plane_hash(&a.data), plane_hash(&b.data));
    }

    #[test]
    fn solver_fallback_path_solves_degenerate_system() {
        // Crafted degenerate system in the wls-documented overflow class:
        // (I + λA) with UNIFORM huge weights (the flat-guide five-point
        // stencil, w = 1e4) — after unit-diagonal scaling every
        // off-diagonal is ≈ −0.25, far above the 0.01 ichol discard
        // threshold, so the factor overruns its pattern budget and
        // `solve_scaled` MUST take the identity/Jacobi fallback branch
        // and still solve. Oracle: constant b ⇒ x = b (A annihilates
        // constants). Also asserts the branch actually fired by checking
        // ichol errors with overflow on this system.
        let (w, h) = (16usize, 16usize);
        let n = w * h;
        let wgt = 1e4f64;
        let mut indptr = vec![0usize];
        let mut indices = Vec::new();
        let mut values = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let k = y * w + x;
                let mut diag = 1.0;
                let mut row: Vec<(usize, f64)> = Vec::new();
                let mut nb = |cond: bool, j: usize| {
                    if cond {
                        row.push((j, -wgt));
                    }
                };
                nb(y > 0, k.wrapping_sub(w));
                nb(x > 0, k.wrapping_sub(1));
                nb(x + 1 < w, k + 1);
                nb(y + 1 < h, k + w);
                diag += wgt * row.len() as f64;
                row.push((k, diag));
                row.sort_by_key(|&(j, _)| j);
                for (j, v) in row {
                    indices.push(j);
                    values.push(v);
                }
                indptr.push(indices.len());
            }
        }
        let a = Csr {
            n,
            indptr,
            indices,
            values,
        };
        // Prove the primary path is unavailable for this system.
        let scale_probe = {
            let mut scaled = a.clone();
            let s: Vec<f64> = (0..n)
                .map(|k| {
                    let row = &a.values[a.indptr[k]..a.indptr[k + 1]];
                    let cols = &a.indices[a.indptr[k]..a.indptr[k + 1]];
                    let d = cols.iter().position(|&c| c == k).map(|i| row[i]).unwrap();
                    1.0 / d.sqrt()
                })
                .collect();
            for row in 0..n {
                for idx in scaled.indptr[row]..scaled.indptr[row + 1] {
                    scaled.values[idx] *= s[row] * s[scaled.indices[idx]];
                }
            }
            ichol(&scaled.to_csc(), 0.01)
        };
        assert!(
            matches!(scale_probe, Err(e) if e == "ichol: max_nnz overflow"),
            "fixture no longer exercises the overflow branch"
        );
        let b = vec![0.75f64; n];
        let x = solve_scaled(&a, &b, 1e-10, 2000).unwrap();
        for (i, v) in x.iter().enumerate() {
            assert!((v - 0.75).abs() < 1e-6, "x[{i}] = {v}");
        }
    }

    #[test]
    fn knn_validation_errors() {
        let (rgb, trimap, _) = two_color_band(8, 8);
        let bad = KnnParams {
            n_neighbors: vec![20],
            distance_weights: vec![2.0, 0.1],
            ..KnnParams::default()
        };
        assert!(estimate_alpha_knn(&rgb, &trimap, &bad).is_err());
        let bad_lambda = KnnParams {
            lambda: 0.0,
            ..KnnParams::default()
        };
        assert!(estimate_alpha_knn(&rgb, &trimap, &bad_lambda).is_err());
        let small = Plane::from_vec(4, 4, vec![0.5; 16]);
        assert!(estimate_alpha_knn(&rgb, &small, &KnnParams::default()).is_err());
    }

    #[test]
    fn knn_fully_known_trimap_short_circuits() {
        let (rgb, mut trimap, _) = two_color_band(8, 8);
        for v in trimap.data.iter_mut() {
            *v = if *v >= 0.5 { 1.0 } else { 0.0 };
        }
        let a = estimate_alpha_knn(&rgb, &trimap, &KnnParams::default()).unwrap();
        for (av, tv) in a.data.iter().zip(&trimap.data) {
            assert_eq!(*av, *tv);
        }
    }

    #[test]
    fn determinism_two_runs_and_pinned_golden() {
        // Pinned from this crate's own runs AFTER the behavioral oracles
        // above were green (rule 8). Fixture: shared two-color band 16x8,
        // default params (soft kernel).
        let (rgb, trimap, _) = two_color_band(16, 8);
        let a = estimate_alpha_knn(&rgb, &trimap, &KnnParams::default()).unwrap();
        let b = estimate_alpha_knn(&rgb, &trimap, &KnnParams::default()).unwrap();
        assert_eq!(
            plane_hash(&a.data),
            plane_hash(&b.data),
            "two-run bit equality"
        );
        assert_eq!(plane_hash(&a.data), PINNED_KNN_GOLDEN, "pinned golden hash");
    }

    /// Pinned after oracle green (see `determinism_two_runs_and_pinned_golden`).
    const PINNED_KNN_GOLDEN: u64 = 4975592368267590319;

    // ---- tester round (Row B) ----

    #[test]
    fn fully_known_nonexact_trimap_thresholds_exactly() {
        // Zero-unknown trimap whose definite values are NOT exactly 0/1
        // (0.05 / 0.95): the short-circuit must return the THRESHOLDED
        // trimap (exact 0.0 / 1.0), not the raw values.
        let (rgb, mut trimap, _) = two_color_band(8, 8);
        for v in trimap.data.iter_mut() {
            *v = if *v >= 0.5 { 0.95 } else { 0.05 };
        }
        let a = estimate_alpha_knn(&rgb, &trimap, &KnnParams::default()).unwrap();
        for (av, tv) in a.data.iter().zip(&trimap.data) {
            assert_eq!(*av, if *tv >= 0.5 { 1.0 } else { 0.0 });
        }
    }

    #[test]
    fn single_unknown_pixel_bounded_and_deterministic() {
        // Degenerate band: exactly ONE unknown pixel (center of the
        // shared fixture's band, true alpha 0.5), everything else
        // definite. The non-local KNN prior averages over feature-space
        // neighbors on BOTH sides plus itself, so the solution is
        // biased off the local truth — measured alpha 0.7537 (dev
        // 0.2537 from truth 0.5). Pinned as a measured-behavior bound
        // (dev < 0.35), NOT a quality contract: the assertions that
        // matter are validity, exact definite pinning, and bit
        // determinism.
        let (rgb, _, truth) = two_color_band(16, 8);
        let mut tri = Plane::new(16, 8);
        for (i, v) in tri.data.iter_mut().enumerate() {
            *v = if truth[i] >= 0.5 { 1.0 } else { 0.0 };
        }
        let center = 4 * 16 + 8; // x = 8: truth alpha = 0.5
        tri.data[center] = 0.5;
        let a = estimate_alpha_knn(&rgb, &tri, &KnnParams::default()).unwrap();
        assert!(a.data.iter().all(|&v| (0.0..=1.0).contains(&v)));
        for (i, &t) in tri.data.iter().enumerate() {
            if i != center {
                assert_eq!(a.data[i], if t >= 0.9 { 1.0 } else { 0.0 });
            }
        }
        let dev = (a.data[center] as f64 - truth[center]).abs();
        assert!(dev < 0.35, "single-pixel dev {dev} (measured 0.2537)");
        let b = estimate_alpha_knn(&rgb, &tri, &KnnParams::default()).unwrap();
        assert_eq!(plane_hash(&a.data), plane_hash(&b.data));
    }

    #[test]
    fn border_touching_unknown_band_with_uniform_anchors_is_exact() {
        // Unknown band flush against the LEFT image edge (neighbor
        // features at edge coordinates, spatial term 0 at x = 0). With
        // every definite pixel foreground, α ≡ 1 satisfies
        // (L + λC)·1 = λ·v EXACTLY (Laplacian rows sum to zero), so the
        // entry must return ~1 everywhere. NOTE (zero-RHS fix round):
        // this trimap is the empty-background class, so the entry now
        // answers via the short-circuit (exact by construction — the
        // same identity this oracle asserted; pre-fix the solve
        // measured worst |α − 1| = 1.2e-7 on this fixture).
        let (w, h) = (12usize, 8usize);
        let rgb = [
            Plane::from_vec(w, h, (0..w * h).map(|i| (i % 5) as f32 / 5.0).collect()),
            Plane::from_vec(w, h, (0..w * h).map(|i| (i % 3) as f32 / 3.0).collect()),
            Plane::from_vec(w, h, (0..w * h).map(|i| (i % 7) as f32 / 7.0).collect()),
        ];
        let mut tri_fg = Plane::from_vec(w, h, vec![1.0; w * h]);
        for y in 0..h {
            for x in 0..3 {
                tri_fg.data[y * w + x] = 0.5;
            }
        }
        let a = estimate_alpha_knn(&rgb, &tri_fg, &KnnParams::default()).unwrap();
        for (i, &v) in a.data.iter().enumerate() {
            assert!(
                (v as f64 - 1.0).abs() < 1e-5,
                "fg-anchored alpha[{i}] = {v}"
            );
        }
    }

    #[test]
    fn no_foreground_trimap_zero_rhs_returns_all_zero_alpha() {
        // Zero-RHS fix (lane ruling; this test's PREVIOUS incarnation,
        // `no_foreground_trimap_zero_rhs_returns_err_pinned`, honestly
        // pinned the PRE-FIX behavior: b = λ·is_fg ≡ 0, strict `<` CG
        // guards against atol = 0 rejected the exact x = 0 iterate,
        // 0/0 on the first step, solve ran to its cap and returned
        // Err — upstream-faithful but a consumer-seam hazard). The
        // engine-side short-circuit now returns the EXACT solution of
        // the constrained system: α = 0 over unknowns, definites keep
        // their trimap values (all background here).
        let (w, h) = (12usize, 8usize);
        let rgb = [
            Plane::from_vec(w, h, (0..w * h).map(|i| (i % 5) as f32 / 5.0).collect()),
            Plane::from_vec(w, h, (0..w * h).map(|i| (i % 3) as f32 / 3.0).collect()),
            Plane::from_vec(w, h, (0..w * h).map(|i| (i % 7) as f32 / 7.0).collect()),
        ];
        let mut tri_bg = Plane::from_vec(w, h, vec![0.0; w * h]);
        for y in 0..h {
            for x in 0..3 {
                tri_bg.data[y * w + x] = 0.5;
            }
        }
        let a1 = estimate_alpha_knn(&rgb, &tri_bg, &KnnParams::default()).unwrap();
        assert!(a1.data.iter().all(|&v| v == 0.0), "exact all-zero alpha");
        let a2 = estimate_alpha_knn(&rgb, &tri_bg, &KnnParams::default()).unwrap();
        assert_eq!(plane_hash(&a1.data), plane_hash(&a2.data));
        // Same class on the closed-form entry (rhs = −R·m with m ≡ 0):
        // same exact short-circuit.
        let cf = crate::matting::estimate_alpha_cf(&rgb, &tri_bg, 1e-7, 1e-8, 2000).unwrap();
        assert!(cf.data.iter().all(|&v| v == 0.0), "closed-form all-zero");
    }

    #[test]
    fn no_background_trimap_returns_unit_alpha() {
        // Symmetric arm of the zero-RHS fix: NO background constraint
        // (foreground + unknown only). Every Laplacian row sums to
        // zero, so α ≡ 1 satisfies (L + λC)·1 = λ·is_known = λ·v
        // exactly (and L_U·1 = −R·1 for the closed-form system) — the
        // short-circuit returns it without touching the solver.
        let (w, h) = (12usize, 8usize);
        let rgb = [
            Plane::from_vec(w, h, (0..w * h).map(|i| (i % 5) as f32 / 5.0).collect()),
            Plane::from_vec(w, h, (0..w * h).map(|i| (i % 3) as f32 / 3.0).collect()),
            Plane::from_vec(w, h, (0..w * h).map(|i| (i % 7) as f32 / 7.0).collect()),
        ];
        let mut tri_fg = Plane::from_vec(w, h, vec![1.0; w * h]);
        for y in 0..h {
            for x in 0..3 {
                tri_fg.data[y * w + x] = 0.5;
            }
        }
        let a1 = estimate_alpha_knn(&rgb, &tri_fg, &KnnParams::default()).unwrap();
        assert!(a1.data.iter().all(|&v| v == 1.0), "exact all-one alpha");
        let a2 = estimate_alpha_knn(&rgb, &tri_fg, &KnnParams::default()).unwrap();
        assert_eq!(plane_hash(&a1.data), plane_hash(&a2.data));
        let cf = crate::matting::estimate_alpha_cf(&rgb, &tri_fg, 1e-7, 1e-8, 2000).unwrap();
        assert!(cf.data.iter().all(|&v| v == 1.0), "closed-form all-one");
        // All-unknown trimap (both constraint classes empty): resolves
        // deterministically through the no-foreground arm to all-zero.
        let tri_unknown = Plane::from_vec(w, h, vec![0.5; w * h]);
        let u = estimate_alpha_knn(&rgb, &tri_unknown, &KnnParams::default()).unwrap();
        assert!(u.data.iter().all(|&v| v == 0.0), "all-unknown → all-zero");
        let ucf = crate::matting::estimate_alpha_cf(&rgb, &tri_unknown, 1e-7, 1e-8, 2000).unwrap();
        assert!(ucf.data.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn knn_tree_duplicate_points_lowest_index_wins() {
        // Duplicate-position degeneracy: many IDENTICAL feature vectors
        // (spread 0 in every dimension — the median split partitions
        // purely by index). The k nearest under the total order must be
        // the k LOWEST indices at distance 0, and a mixed set (dupes +
        // distinct) must match brute force.
        let n = 20usize; // > 2·LEAF_CAP: forces splits over pure ties
        let coords: Vec<f64> = (0..n).flat_map(|_| [0.5, 0.5]).collect();
        let tree = KnnTree::build(coords, 2);
        let mut best = Vec::new();
        tree.knn(&[0.5, 0.5], 5, &mut best);
        let expect: Vec<(f64, u32)> = (0..5).map(|i| (0.0, i)).collect();
        assert_eq!(best, expect, "pure-duplicate set");
        // Mixed: 10 duplicates at the origin + 6 distinct points, some
        // at equal distances from the query.
        let mut pts: Vec<(f64, f64)> = vec![(0.0, 0.0); 10];
        pts.extend([
            (1.0, 0.0),
            (0.0, 1.0),
            (-1.0, 0.0),
            (2.0, 2.0),
            (1.0, 1.0),
            (0.5, 0.5),
        ]);
        let flat: Vec<f64> = pts.iter().flat_map(|&(x, y)| [x, y]).collect();
        let tree = KnnTree::build(flat, 2);
        for k in [1usize, 3, 10, 12, 16] {
            tree.knn(&[0.0, 0.0], k, &mut best);
            let mut all: Vec<(f64, u32)> = pts
                .iter()
                .enumerate()
                .map(|(i, &(x, y))| (x * x + y * y, i as u32))
                .collect();
            all.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            all.truncate(k);
            assert_eq!(best, all, "mixed duplicates k={k}");
        }
    }

    #[test]
    fn lambda_monotone_constraint_violation() {
        // Penalty-method sanity: λ→∞ pins the definite pixels harder.
        // Metric: max over known pixels of |x_i − target_i| on the RAW
        // (pre-pin, pre-clip) solution of (L + λC)x = λv — the entry
        // pins definites afterwards, so the raw system is solved here
        // directly. Measured on the shared fixture: λ=10 → 4.19e-3,
        // λ=100 → 4.39e-4, λ=1000 → 4.41e-5 (≈ 1/λ, as penalty theory
        // predicts). Asserted strictly decreasing across decades.
        let (rgb, trimap, _) = two_color_band(16, 8);
        let split = crate::matting::trimap_split(&trimap);
        let violation = |lambda: f64| -> f64 {
            let params = KnnParams::default();
            let mut a = knn_laplacian(&rgb, &params);
            for i in 0..a.n {
                if split.is_known[i] {
                    for idx in a.indptr[i]..a.indptr[i + 1] {
                        if a.indices[idx] == i {
                            a.values[idx] += lambda;
                        }
                    }
                }
            }
            let b: Vec<f64> = split
                .is_fg
                .iter()
                .map(|&fg| if fg { lambda } else { 0.0 })
                .collect();
            let x = solve_scaled(&a, &b, 1e-10, 4000).unwrap();
            let mut v = 0f64;
            // Index loop kept: i addresses the split masks and x in
            // lockstep (crate precedent for multi-array index loops).
            #[allow(clippy::needless_range_loop)]
            for i in 0..a.n {
                if split.is_fg[i] {
                    v = v.max((x[i] - 1.0).abs());
                } else if split.is_bg[i] {
                    v = v.max(x[i].abs());
                }
            }
            v
        };
        let (v10, v100, v1000) = (violation(10.0), violation(100.0), violation(1000.0));
        assert!(
            v10 > v100 && v100 > v1000,
            "violations not monotone: {v10} {v100} {v1000}"
        );
        assert!(v1000 < 1e-3, "λ=1000 violation {v1000}");
    }

    #[test]
    fn nan_rgb_returns_err_never_nan_alpha() {
        // NaN input policy, pinned honestly: a NaN color pixel poisons
        // the affinity kernel and the CG residual norm; every
        // convergence comparison goes false and the solve runs to its
        // cap, returning Err (measured: "cg: no convergence …
        // (|r| = NaN)"). The entry fails loudly instead of returning a
        // NaN-bearing alpha. Documents current behavior, not a
        // contract.
        let (rgb, trimap, _) = two_color_band(16, 8);
        let mut rgb_nan = rgb;
        rgb_nan[0].data[3 * 16 + 8] = f32::NAN;
        let r1 = estimate_alpha_knn(&rgb_nan, &trimap, &KnnParams::default());
        let r2 = estimate_alpha_knn(&rgb_nan, &trimap, &KnnParams::default());
        assert!(r1.is_err());
        assert_eq!(r1.unwrap_err(), r2.unwrap_err(), "deterministic failure");
    }

    #[test]
    fn cross_tier_comparability_regression_bounds() {
        // The shared-fixture comparability numbers claimed in this
        // module's docs and the closed-form test, locked as regression
        // bounds with margin: closed-form stays in its claimed 0.05
        // class (measured worst 0.0010), KNN stays under 0.09 (claimed
        // measurement 0.0758, re-measured here identically). If either
        // tier drifts past its class, the cross-tier claim is stale and
        // this test names it.
        let (rgb, trimap, truth) = two_color_band(16, 8);
        let cf = crate::matting::estimate_alpha_cf(&rgb, &trimap, 1e-7, 1e-8, 2000).unwrap();
        let knn = estimate_alpha_knn(&rgb, &trimap, &KnnParams::default()).unwrap();
        let worst = |a: &Plane| {
            a.data
                .iter()
                .zip(&truth)
                .map(|(x, t)| (*x as f64 - t).abs())
                .fold(0f64, f64::max)
        };
        let (wcf, wknn) = (worst(&cf), worst(&knn));
        assert!(wcf < 0.05, "closed-form left its 0.05 class: {wcf}");
        assert!(wknn < 0.09, "KNN left its 0.0758-measured class: {wknn}");
    }
}
