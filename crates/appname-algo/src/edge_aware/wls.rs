// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Weighted least squares edge-preserving decomposition — clean-room from
//! Farbman, Fattal, Lischinski, Szeliski, "Edge-Preserving Decompositions
//! for Multi-Scale Tone and Detail Manipulation", SIGGRAPH 2008 (contract
//! basis key `farbman08`).
//!
//! Spec: ALGORITHMS.md §14.B (WLS is the `best`-quality decomposition
//! alternative); dossier `detail-local-laplacian` selected row
//! ("best-decomposition alternative: WLS λ=1.0") and tiling row ("WLS
//! sparse solve CPU").
//!
//! Algorithm (farbman08 eq. 3–5): the smoothed image `u` minimizes
//!   Σ_p ( (u_p - g_p)^2 + λ ( a_x(p) (∂u/∂x)_p^2 + a_y(p) (∂u/∂y)_p^2 ) )
//! with smoothness weights computed on the guide `l` (the paper uses the
//! log-luminance channel; the caller chooses what to pass):
//!   a_x(p) = ( |∂l/∂x(p)|^α + ε )^-1,   ε = 1e-4 (paper's constant)
//! giving the sparse SPD linear system `(I + λ A) u = g` where `A` is the
//! five-point inhomogeneous Laplacian assembled from the weights. Paper
//! defaults: α = 1.2, λ ~ 1.0 (the dossier's `best` engine setting).
//!
//! The solve REUSES the crate's existing sparse machinery ([`crate::sparse`]:
//! thresholded incomplete-Cholesky preconditioner + conjugate gradients) —
//! no duplicate solver, per the module-reuse rule the matting engine set.
//! The system is symmetrically Jacobi-scaled to unit diagonal before the
//! factorization; on flat/near-flat guides, where the thresholded ichol
//! factor overruns its pattern-sized budget, the solve deterministically
//! falls back to unpreconditioned CG (≡ Jacobi-preconditioned on the
//! unit-diagonal scaled system) — rationale at the call site.
//!
//! Determinism: fixed row-major assembly order (diagonal-sorted CSR rows),
//! single-threaded solve, f64 throughout the system; `powf` routes through
//! platform libm (crate posture as documented in mls/deconv).

use crate::plane::Plane;
use crate::sparse::{cg, ichol, Csr};

/// Paper's weight-regularization constant ε (farbman08 eq. 5).
const WLS_EPS: f64 = 1e-4;

/// Solver tolerances/limits for the `(I + λA) u = g` CG solve.
const CG_RTOL: f64 = 1e-10;
const CG_MAXITER: usize = 2000;
/// Iteration cap for the unpreconditioned fallback path (flat/near-flat
/// guides, see the call site): without the ichol preconditioner CG needs
/// O(sqrt(cond)) iterations, so the cap is raised — deterministically,
/// the same fixed constant for every input that takes this path.
const CG_FALLBACK_MAXITER: usize = 20_000;

/// WLS edge-preserving smoothing of `input` (g), with smoothness weights
/// from `guide` (the paper's log-luminance channel; pass `input` again
/// for self-guided smoothing). `lambda` >= 0 balances fidelity vs
/// smoothness (0 = identity); `alpha` > 0 is the gradient-sensitivity
/// exponent (paper default 1.2).
pub fn wls_smooth(input: &Plane, guide: &Plane, lambda: f64, alpha: f64) -> Result<Plane, String> {
    if input.w == 0 || input.h == 0 {
        return Err("wls_smooth: empty plane".to_string());
    }
    if input.w != guide.w || input.h != guide.h {
        return Err(format!(
            "wls_smooth: input {}x{} vs guide {}x{} dimension mismatch",
            input.w, input.h, guide.w, guide.h
        ));
    }
    if !(lambda.is_finite() && lambda >= 0.0) {
        return Err(format!(
            "wls_smooth: lambda must be finite and >= 0 (got {lambda})"
        ));
    }
    if !(alpha.is_finite() && alpha > 0.0) {
        return Err(format!(
            "wls_smooth: alpha must be finite and > 0 (got {alpha})"
        ));
    }
    let (w, h) = (input.w, input.h);
    let n = w * h;

    // Smoothness weights on the guide: wx[p] couples p=(x,y) with its
    // RIGHT neighbor, wy[p] with its DOWN neighbor (forward differences;
    // the symmetric system uses each edge weight once for both endpoints).
    let g64: Vec<f64> = guide.data.iter().map(|&v| v as f64).collect();
    let mut wx = vec![0f64; n];
    let mut wy = vec![0f64; n];
    for y in 0..h {
        for x in 0..w {
            let k = y * w + x;
            if x + 1 < w {
                let d = (g64[k + 1] - g64[k]).abs();
                wx[k] = 1.0 / (d.powf(alpha) + WLS_EPS);
            }
            if y + 1 < h {
                let d = (g64[k + w] - g64[k]).abs();
                wy[k] = 1.0 / (d.powf(alpha) + WLS_EPS);
            }
        }
    }

    // Assemble (I + λ A) in CSR, rows in pixel order, columns ascending:
    // for pixel p the row is  -λ w_up, -λ w_left, 1 + λ Σ w, -λ w_right,
    // -λ w_down  at columns  p-w, p-1, p, p+1, p+w.
    let mut indptr = Vec::with_capacity(n + 1);
    let mut indices = Vec::with_capacity(5 * n);
    let mut values = Vec::with_capacity(5 * n);
    indptr.push(0usize);
    for y in 0..h {
        for x in 0..w {
            let k = y * w + x;
            let mut diag = 1.0;
            if y > 0 {
                let wgt = lambda * wy[k - w];
                diag += wgt;
                indices.push(k - w);
                values.push(-wgt);
            }
            if x > 0 {
                let wgt = lambda * wx[k - 1];
                diag += wgt;
                indices.push(k - 1);
                values.push(-wgt);
            }
            let diag_slot = values.len();
            indices.push(k);
            values.push(0.0); // patched below once the row total is known
            if x + 1 < w {
                let wgt = lambda * wx[k];
                diag += wgt;
                indices.push(k + 1);
                values.push(-wgt);
            }
            if y + 1 < h {
                let wgt = lambda * wy[k];
                diag += wgt;
                indices.push(k + w);
                values.push(-wgt);
            }
            values[diag_slot] = diag;
            indptr.push(indices.len());
        }
    }
    let a = Csr {
        n,
        indptr,
        indices,
        values,
    };

    // Symmetric Jacobi (diagonal) scaling before the solve: the WLS
    // weights reach 1/ε = 1e4, and on that raw scale the ichol drop
    // threshold keeps nearly all fill (the factor overruns its pattern
    // budget). Solving S A S y = S b with S = diag(1/sqrt(diag(A)))
    // (unit-diagonal system), then u = S y, brings TEXTURED-guide systems
    // into the range where the crate's ichol+CG machinery works untouched.
    // CORRECTION (wave-2 fix round; the earlier claim that this scaling
    // "keeps ichol effective" was false for the flat class and untested):
    // on a FLAT or near-flat guide every weight is 1/ε = 1e4 and every
    // scaled off-diagonal is ≈ −λw/(1 + 4λw) ≈ −0.25, uniformly — far
    // above the 0.01 discard threshold, so ichol keeps its fill, overruns
    // `sparse.rs`'s pattern-sized cap, and reports max_nnz overflow. That
    // class is handled by the fallback below, not by this scaling.
    // Deterministic: S is a pure function of the assembly.
    let scale: Vec<f64> = (0..n)
        .map(|k| {
            let row = &a.values[a.indptr[k]..a.indptr[k + 1]];
            let cols = &a.indices[a.indptr[k]..a.indptr[k + 1]];
            let d = cols
                .iter()
                .position(|&c| c == k)
                .map(|i| row[i])
                .unwrap_or(1.0);
            1.0 / d.sqrt()
        })
        .collect();
    let mut scaled = a;
    for row in 0..n {
        for idx in scaled.indptr[row]..scaled.indptr[row + 1] {
            scaled.values[idx] *= scale[row] * scale[scaled.indices[idx]];
        }
    }
    let b: Vec<f64> = input
        .data
        .iter()
        .enumerate()
        .map(|(k, &v)| v as f64 * scale[k])
        .collect();
    // Preconditioner selection. Primary path: thresholded ichol on the
    // scaled system (textured guides; fill drops below the 0.01
    // threshold and the pattern-sized cap holds). Fallback path: when
    // ichol reports max_nnz overflow — the flat/near-flat guide class
    // documented above — solve by CG with the identity preconditioner,
    // which on this unit-diagonal scaled system IS the diagonal (Jacobi)
    // preconditioner. CG on the SPD system converges regardless of
    // preconditioning (slower is acceptable and honest); the branch is
    // deterministic because ichol itself is deterministic, so a given
    // input always takes the same path. Non-overflow ichol errors still
    // propagate (the system is SPD by construction, so the shift ladder's
    // definiteness failure is unexpected and must surface).
    let y = match ichol(&scaled.to_csc(), 0.01) {
        Ok(factor) => cg(&scaled, &b, |r| factor.apply(r), CG_RTOL, 0.0, CG_MAXITER)?,
        Err(e) if e == "ichol: max_nnz overflow" => cg(
            &scaled,
            &b,
            |r| r.to_vec(),
            CG_RTOL,
            0.0,
            CG_FALLBACK_MAXITER,
        )?,
        Err(e) => return Err(e),
    };
    Ok(Plane::from_vec(
        w,
        h,
        y.iter()
            .enumerate()
            .map(|(k, &v)| (v * scale[k]) as f32)
            .collect(),
    ))
}

/// Base/detail decomposition via WLS: `base` = [`wls_smooth`], `detail` =
/// `input - base` (the paper's single-level decomposition; multi-scale
/// stacks are built by the consumer with increasing λ).
pub fn wls_split(
    input: &Plane,
    guide: &Plane,
    lambda: f64,
    alpha: f64,
) -> Result<(Plane, Plane), String> {
    let base = wls_smooth(input, guide, lambda, alpha)?;
    let mut detail = Plane::new(input.w, input.h);
    for k in 0..input.data.len() {
        detail.data[k] = input.data[k] - base.data[k];
    }
    Ok((base, detail))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plane::{plane_hash, SplitMix64};

    fn random_plane(w: usize, h: usize, seed: u64) -> Plane {
        let mut rng = SplitMix64(seed);
        Plane::from_vec(w, h, (0..w * h).map(|_| rng.next_f64() as f32).collect())
    }

    fn noisy_step(w: usize, h: usize, amp: f32, seed: u64) -> Plane {
        let mut rng = SplitMix64(seed);
        let mut p = Plane::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let base = if x < w / 2 { 0.2 } else { 0.8 };
                p.data[y * w + x] = base + amp * (2.0 * rng.next_f64() as f32 - 1.0);
            }
        }
        p
    }

    /// Rebuild the system exactly as `wls_smooth` does and return the
    /// worst per-row residual of `(I + λA)u - g` NORMALIZED by that row's
    /// 1-norm (WLS weights reach 1/ε = 1e4, so an absolute residual bound
    /// would only measure the f32 storage of `u`, not solver quality).
    fn system_residual(input: &Plane, guide: &Plane, lambda: f64, alpha: f64, u: &Plane) -> f64 {
        let (w, h) = (input.w, input.h);
        let g64: Vec<f64> = guide.data.iter().map(|&v| v as f64).collect();
        let u64v: Vec<f64> = u.data.iter().map(|&v| v as f64).collect();
        let wgt = |a: f64, b: f64| 1.0 / ((a - b).abs().powf(alpha) + WLS_EPS);
        let mut worst = 0f64;
        for y in 0..h {
            for x in 0..w {
                let k = y * w + x;
                let mut acc = u64v[k];
                let mut row_norm = 1.0f64;
                let mut add = |kn: usize, wv: f64| {
                    acc += lambda * wv * (u64v[k] - u64v[kn]);
                    row_norm += 2.0 * lambda * wv;
                };
                if x + 1 < w {
                    add(k + 1, wgt(g64[k], g64[k + 1]));
                }
                if x > 0 {
                    add(k - 1, wgt(g64[k - 1], g64[k]));
                }
                if y + 1 < h {
                    add(k + w, wgt(g64[k], g64[k + w]));
                }
                if y > 0 {
                    add(k - w, wgt(g64[k - w], g64[k]));
                }
                worst = worst.max((acc - input.data[k] as f64).abs() / row_norm);
            }
        }
        worst
    }

    #[test]
    fn residual_below_tolerance() {
        let p = random_plane(24, 18, 0x315F_0001);
        let u = wls_smooth(&p, &p, 1.0, 1.2).unwrap();
        let r = system_residual(&p, &p, 1.0, 1.2, &u);
        // Row-normalized bound: dominated by the f32 rounding of the
        // returned `u` (~1e-7 class), far above the CG rtol contribution.
        assert!(
            r < 1e-6,
            "row-normalized system residual {r} above tolerance"
        );
    }

    #[test]
    fn lambda_zero_is_identity() {
        let p = random_plane(16, 12, 0x315F_0002);
        let u = wls_smooth(&p, &p, 0.0, 1.2).unwrap();
        let max = u
            .data
            .iter()
            .zip(&p.data)
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        assert!(max <= 1e-6, "lambda = 0 deviated from identity by {max}");
    }

    #[test]
    fn matches_dense_direct_solve_on_small_grid() {
        // 6x5 grid: assemble the same (I + λA) densely and solve by plain
        // Gaussian elimination with partial pivoting (test-only oracle, no
        // new dependency), then compare.
        let (w, h) = (6usize, 5usize);
        let n = w * h;
        let (lambda, alpha) = (0.8f64, 1.2f64);
        let p = random_plane(w, h, 0x315F_0003);
        let g64: Vec<f64> = p.data.iter().map(|&v| v as f64).collect();
        let wgt = |a: f64, b: f64| 1.0 / ((a - b).abs().powf(alpha) + WLS_EPS);
        let mut m = vec![0f64; n * n];
        let mut rhs: Vec<f64> = g64.clone();
        for y in 0..h {
            for x in 0..w {
                let k = y * w + x;
                m[k * n + k] += 1.0;
                let mut couple = |kn: usize, wv: f64| {
                    m[k * n + k] += lambda * wv;
                    m[k * n + kn] -= lambda * wv;
                };
                if x + 1 < w {
                    couple(k + 1, wgt(g64[k], g64[k + 1]));
                }
                if x > 0 {
                    couple(k - 1, wgt(g64[k - 1], g64[k]));
                }
                if y + 1 < h {
                    couple(k + w, wgt(g64[k], g64[k + w]));
                }
                if y > 0 {
                    couple(k - w, wgt(g64[k - w], g64[k]));
                }
            }
        }
        // Gaussian elimination with partial pivoting.
        for col in 0..n {
            let piv = (col..n)
                .max_by(|&a, &b| {
                    m[a * n + col]
                        .abs()
                        .partial_cmp(&m[b * n + col].abs())
                        .unwrap()
                })
                .unwrap();
            if piv != col {
                for j in 0..n {
                    m.swap(col * n + j, piv * n + j);
                }
                rhs.swap(col, piv);
            }
            let d = m[col * n + col];
            for row in col + 1..n {
                let f = m[row * n + col] / d;
                if f != 0.0 {
                    for j in col..n {
                        m[row * n + j] -= f * m[col * n + j];
                    }
                    rhs[row] -= f * rhs[col];
                }
            }
        }
        let mut dense = vec![0f64; n];
        for row in (0..n).rev() {
            let mut acc = rhs[row];
            for j in row + 1..n {
                acc -= m[row * n + j] * dense[j];
            }
            dense[row] = acc / m[row * n + row];
        }
        let sparse = wls_smooth(&p, &p, lambda, alpha).unwrap();
        let max = sparse
            .data
            .iter()
            .zip(&dense)
            .map(|(a, b)| (*a as f64 - b).abs())
            .fold(0f64, f64::max);
        assert!(max < 1e-6, "sparse vs dense direct solve deviation {max}");
    }

    #[test]
    fn edge_stopping_smooths_flats_keeps_step() {
        // Paper-canonical usage: smoothness weights on the LOG-luminance
        // channel (farbman08 §3), so the 0.2 | 0.8 step is a large guide
        // gradient while the small texture stays weakly weighted.
        let p = noisy_step(32, 16, 0.02, 0x315F_0004);
        let log_guide =
            Plane::from_vec(p.w, p.h, p.data.iter().map(|&v| (v + 1e-4).ln()).collect());
        let u = wls_smooth(&p, &log_guide, 1.0, 1.2).unwrap();
        // Step retained.
        let mut min_jump = f32::MAX;
        for y in 0..p.h {
            min_jump = min_jump.min(u.at(p.w / 2, y) - u.at(p.w / 2 - 1, y));
        }
        assert!(min_jump > 0.5, "step jump collapsed to {min_jump}");
        // Quantified smoothing: in-side gradient energy (mean squared
        // adjacent difference) collapses by >= 10x. Gradients, not
        // variance-about-mean, so the smooth cross-edge equilibrium ramp
        // (jump-tested above) does not pollute the noise metric.
        let grad_energy = |img: &Plane, x0: usize, x1: usize| {
            let mut s = 0f64;
            let mut cnt = 0usize;
            for y in 0..img.h {
                for x in x0..x1 - 1 {
                    let d = (img.at(x + 1, y) - img.at(x, y)) as f64;
                    s += d * d;
                    cnt += 1;
                }
            }
            s / cnt as f64
        };
        let before = grad_energy(&p, 1, p.w / 2 - 1);
        let after = grad_energy(&u, 1, p.w / 2 - 1);
        assert!(
            after < before / 10.0,
            "flat gradient energy {after} not << {before}"
        );
    }

    #[test]
    fn constant_input_is_identity_at_dossier_defaults() {
        // Wave-2 blocker repro (previously Err("ichol: max_nnz overflow")):
        // a constant plane at the dossier `best`-engine defaults (λ = 1.0,
        // α = 1.2, self-guide) must solve — the exact solution of
        // (I + λA) u = c·1 is u = c·1 since A annihilates constants — via
        // the deterministic unpreconditioned-CG fallback path. Sibling
        // modules (guided/dtf/llf) all carry this constant-input test;
        // WLS was missing it. 32x32 and 64x64 both exercised (both were
        // verified repros).
        for (w, h, c) in [(32usize, 32usize, 0.5f32), (64, 64, 0.25)] {
            let p = Plane::from_vec(w, h, vec![c; w * h]);
            let u = wls_smooth(&p, &p, 1.0, 1.2).unwrap();
            let max = u.data.iter().map(|&v| (v - c).abs()).fold(0f32, f32::max);
            assert!(max <= 1e-6, "{w}x{h} constant deviated by {max}");
            // Determinism on the fallback path: two-run bit equality.
            let v = wls_smooth(&p, &p, 1.0, 1.2).unwrap();
            assert_eq!(plane_hash(&u.data), plane_hash(&v.data));
        }
    }

    #[test]
    fn near_flat_ramp_solves_at_dossier_defaults() {
        // Second verified repro of the wave-2 blocker: constant + 1e-5/px
        // horizontal ramp (guide gradients far below the ε = 1e-4 knee, so
        // every weight is still ≈ 1/ε). Must solve, satisfy the system to
        // the row-normalized residual bound, and replay bit-identically.
        let (w, h) = (32usize, 32usize);
        let p = Plane::from_vec(
            w,
            h,
            (0..w * h).map(|k| 0.5 + 1e-5 * (k % w) as f32).collect(),
        );
        let u = wls_smooth(&p, &p, 1.0, 1.2).unwrap();
        let r = system_residual(&p, &p, 1.0, 1.2, &u);
        assert!(r < 1e-6, "row-normalized residual {r} above tolerance");
        let v = wls_smooth(&p, &p, 1.0, 1.2).unwrap();
        assert_eq!(plane_hash(&u.data), plane_hash(&v.data));
    }

    #[test]
    fn large_flat_region_in_textured_image_solves() {
        // Smooth-region robustness in context (skies, fills): a textured
        // image whose right two-thirds is a large exactly-flat region.
        // Whichever solve path the mixed system takes, the result must
        // satisfy the system residual bound, keep the flat region flat
        // (the smoothing must not invent detail there), and replay
        // bit-identically.
        let (w, h) = (48usize, 32usize);
        let mut p = random_plane(w, h, 0x315F_0006);
        for y in 0..h {
            for x in w / 3..w {
                p.data[y * w + x] = 0.6;
            }
        }
        let u = wls_smooth(&p, &p, 1.0, 1.2).unwrap();
        let r = system_residual(&p, &p, 1.0, 1.2, &u);
        assert!(r < 1e-6, "row-normalized residual {r} above tolerance");
        // The flat region stays SMOOTH and near its value. It does not
        // stay exactly at 0.6: with weights at 1/ε = 1e4 the diffusion
        // length sqrt(λ/ε) = 100 px exceeds the region, so the whole
        // plateau equilibrates a little toward the textured side
        // (measured max deviation ≈ 5.3e-3). Assert the honest bounds:
        // bounded deviation, and per-pixel gradients far below the
        // plateau shift (no invented detail).
        let mut max_dev = 0f32;
        let mut max_grad = 0f32;
        for y in 0..h {
            for x in w / 3 + 1..w {
                max_dev = max_dev.max((u.at(x, y) - 0.6).abs());
                if x + 1 < w {
                    max_grad = max_grad.max((u.at(x + 1, y) - u.at(x, y)).abs());
                }
                if y + 1 < h {
                    max_grad = max_grad.max((u.at(x, y + 1) - u.at(x, y)).abs());
                }
            }
        }
        assert!(max_dev <= 2e-2, "flat plateau shifted by {max_dev}");
        assert!(
            max_grad <= 1e-3,
            "flat plateau roughened: max gradient {max_grad}"
        );
        let v = wls_smooth(&p, &p, 1.0, 1.2).unwrap();
        assert_eq!(plane_hash(&u.data), plane_hash(&v.data));
    }

    #[test]
    fn split_reconstructs_exactly() {
        let p = random_plane(12, 10, 0x315F_0005);
        let (base, detail) = wls_split(&p, &p, 1.0, 1.2).unwrap();
        for k in 0..p.data.len() {
            // detail = input - base in f32; re-adding rounds by <= 1 ulp.
            assert!((base.data[k] + detail.data[k] - p.data[k]).abs() <= 1e-6);
        }
    }

    #[test]
    fn validation_errors() {
        let p = random_plane(8, 8, 1);
        let q = random_plane(8, 7, 2);
        assert!(wls_smooth(&p, &q, 1.0, 1.2).is_err());
        assert!(wls_smooth(&p, &p, -1.0, 1.2).is_err());
        assert!(wls_smooth(&p, &p, 1.0, 0.0).is_err());
        assert!(wls_smooth(&p, &p, f64::INFINITY, 1.2).is_err());
    }

    #[test]
    fn flat_one_row_shapes_solve_on_fallback_path() {
        // Adversarial degenerate shape for the fallback class: 1xN with a
        // FLAT guide forces every weight to 1/ε = 1e4 (ichol overflow =>
        // unpreconditioned-CG fallback) on a pure 1-D chain system.
        // Flat input: A annihilates constants, so u = input exactly.
        let flat_row = Plane::from_vec(32, 1, vec![0.5; 32]);
        let u = wls_smooth(&flat_row, &flat_row, 1.0, 1.2).unwrap();
        let max = u.data.iter().map(|v| (v - 0.5).abs()).fold(0f32, f32::max);
        assert!(max <= 1e-6, "flat 1x32 deviated by {max}");
        // Same shape as a column: identical values by symmetry.
        let flat_col = Plane::from_vec(1, 32, vec![0.5; 32]);
        let v = wls_smooth(&flat_col, &flat_col, 1.0, 1.2).unwrap();
        assert_eq!(u.data, v.data, "row/column symmetry broken on fallback");
    }

    #[test]
    fn hot_pixel_flat_guide_obeys_maximum_principle() {
        // Single hot pixel (1.0 in a 0.5 field) with a FLAT guide: the
        // fallback path again, now with a nontrivial right-hand side.
        // (I + λA) has row sums 1 with nonpositive off-diagonals, so its
        // inverse is a row-stochastic nonnegative matrix and the exact
        // solution obeys the maximum principle: u within [min g, max g].
        // With uniform weights 1/ε the diffusion length sqrt(λ/ε) = 100
        // far exceeds the 32-px chain, so the spike spreads nearly to
        // the global mean 0.5 + 0.5/32 ≈ 0.5156 (measured range
        // [0.51555, 0.51576]). Asserted: maximum principle with 1e-5 CG
        // slack, near-mean equilibration, two-run bit determinism of the
        // fallback path on this fixture.
        let mut hot = Plane::from_vec(32, 1, vec![0.5; 32]);
        hot.data[16] = 1.0;
        let guide = Plane::from_vec(32, 1, vec![0.5; 32]);
        let u = wls_smooth(&hot, &guide, 1.0, 1.2).unwrap();
        for (i, &v) in u.data.iter().enumerate() {
            assert!(
                (0.5 - 1e-5..=1.0 + 1e-5).contains(&v),
                "maximum principle violated at {i}: {v}"
            );
        }
        let mean = 0.5 + 0.5 / 32.0;
        let max = u.data.iter().map(|v| (v - mean).abs()).fold(0f32, f32::max);
        assert!(max <= 1e-3, "spike did not equilibrate near mean: {max}");
        let v = wls_smooth(&hot, &guide, 1.0, 1.2).unwrap();
        assert_eq!(
            plane_hash(&u.data),
            plane_hash(&v.data),
            "fallback-path two-run bit equality"
        );
    }

    #[test]
    fn extreme_lambda_flat_guide_approaches_global_mean() {
        // λ = 1e6 with a flat guide: smoothness dominates fidelity by
        // λ/ε = 1e10, so the minimizer is the global mean of the input
        // to first order (measured max dev 5e-7). Also probes that the
        // fallback CG converges at this conditioning instead of erroring
        // at its 20000-iteration cap. Random input, exact global-mean
        // oracle.
        let p = random_plane(16, 12, 0x315F_0007);
        let guide = Plane::from_vec(16, 12, vec![0.5; 16 * 12]);
        let u = wls_smooth(&p, &guide, 1e6, 1.2).unwrap();
        let mean = p.data.iter().map(|&v| v as f64).sum::<f64>() / p.data.len() as f64;
        let max = u
            .data
            .iter()
            .map(|&v| (v as f64 - mean).abs())
            .fold(0f64, f64::max);
        assert!(max <= 1e-4, "lambda=1e6 max dev from global mean {max}");
    }

    #[test]
    fn textured_solve_obeys_maximum_principle() {
        // The same row-stochastic-inverse argument holds for ANY weight
        // pattern, so the primary (ichol) path must also keep u within
        // the input's range (measured violation 0). Self-guided random
        // texture at dossier defaults.
        let p = random_plane(24, 18, 0x315F_0008);
        let u = wls_smooth(&p, &p, 1.0, 1.2).unwrap();
        let (lo, hi) = p
            .data
            .iter()
            .fold((f32::MAX, f32::MIN), |(l, h), &v| (l.min(v), h.max(v)));
        let viol = u
            .data
            .iter()
            .map(|&v| (lo - v).max(v - hi).max(0.0))
            .fold(0f32, f32::max);
        assert!(viol <= 1e-5, "maximum principle violated by {viol}");
    }

    #[test]
    fn determinism_two_runs_and_pinned_golden() {
        // Pinned from this crate's own runs AFTER the behavioral oracles
        // above were green (rule 8). Fixture: 32x32 SplitMix64 seed
        // 0x315F_601D, λ = 1.0 (dossier best-engine default), α = 1.2.
        let p = random_plane(32, 32, 0x315F_601D);
        let a = wls_smooth(&p, &p, 1.0, 1.2).unwrap();
        let b = wls_smooth(&p, &p, 1.0, 1.2).unwrap();
        assert_eq!(
            plane_hash(&a.data),
            plane_hash(&b.data),
            "two-run bit equality"
        );
        assert_eq!(
            plane_hash(&a.data),
            806735439416774481,
            "pinned golden hash"
        );
    }
}
