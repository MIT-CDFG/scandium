// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Portions ported from PyMatting (https://github.com/pymatting/pymatting,
// commit fcda1e52fdeb47d03ec2478c1aeafd0b7bd387de, vendored at
// appname-proto/vendor/pymatting/): pymatting/laplacian/cf_laplacian.py,
// pymatting/alpha/estimate_alpha_cf.py, pymatting/util/util.py
// (trimap_split), pymatting/util/boxfilter.py (boxfilter shape),
// pymatting/foreground/estimate_foreground_ml.py.
// MIT License, Copyright (c) 2020 PyMatting. License text carried in
// THIRD-PARTY-NOTICES.md and vendor/pymatting/LICENSE.md.

//! Spec: ES §13.4 "Quick Mask and Select-and-Mask" (the [NORMATIVE]
//! Select-and-Mask pipeline; verified against the CURRENT engineering-spec
//! text) + ES §13.2 refinement math + ES §39.4 selection tools (workspace
//! entry) + ES §32.5 "Select and Mask" workspace-modal. NOTE (stale
//! anchors): the contract's `spec_refs` `["§32.6", "§23.3", "§39.4"]` date
//! from an older ES numbering — current §32.6 is canvas overlays (only the
//! ants overlay), current §23.3 is TEXT layout; discrepancy logged in the
//! lane log for O-INT fold-in. ALGORITHMS.md §7 `alpha_matting`: primary
//! `closed_form_matting` (L1, basis levin08, `cleared_by: [US7692664B2]`),
//! interactive `guided_filter_matting` (L2, he10 + he15fast), quality
//! option `knn_matting` (L2, chen12), `revival_fallback:
//! guided_filter_plus_knn`; INTEGRATION-NOTES §6 order 1 (pymatting port).
//!
//! Closed-form alpha matting core (wave-1) plus the CP-1.0 Select & Mask
//! completion submodules: matting Laplacian (levin08 color-line model),
//! trimap split, ichol-preconditioned CG solve, multi-level foreground
//! estimation, and the row-B engines:
//!
//! - [`knn`] — chen12 KNN matting (the `best`-quality option), ported
//!   from the vendored pymatting KNN implementation.
//! - [`guided_tier`] — the interactive guided-filter matting tier
//!   (he10 via the existing [`crate::edge_aware::guided`] engine, plus
//!   the he15fast subsampled path and ROI refinement).
//! - [`decontaminate`] — color decontamination for matted output
//!   (ES §13.4 stage 8 defringe over [`estimate_foreground_ml`]).
//! - [`fallback`] — the contract's `guided_filter_plus_knn` revival
//!   fallback composition.
//!
//! ADAPTER SEAM (brokered; this crate never touches documents): the
//! CP-1.0 Select & Mask consumer converts the working-space composite +
//! selection trimap into [`Plane`]s, calls [`estimate_alpha_cf`] /
//! [`estimate_foreground_ml`] (and the submodule engines above), and
//! routes the resulting mask through the `selection.set` / `mask.edit`
//! envelope path (D-48 value capture). Consumer wiring for the workspace
//! doors (Selection/DeltaRecorder doors, §10.5 merge machinery) waits on
//! O-INT's door-surface relay; every engine here is pure and merge-ready
//! regardless.

pub mod decontaminate;
pub mod fallback;
pub mod guided_tier;
pub mod knn;

use crate::plane::Plane;
use crate::sparse::{cg, ichol, Csr};

/// Trimap classification per pymatting `trimap_split` (defaults): foreground
/// where value >= 0.9, background where value <= 0.1, unknown elsewhere.
pub struct TrimapSplit {
    /// Per-pixel foreground flags.
    pub is_fg: Vec<bool>,
    /// Per-pixel background flags.
    pub is_bg: Vec<bool>,
    /// Known = fg | bg.
    pub is_known: Vec<bool>,
    /// Unknown = !known.
    pub is_unknown: Vec<bool>,
}

/// Split a trimap plane into fg/bg/known/unknown masks.
pub fn trimap_split(trimap: &Plane) -> TrimapSplit {
    let n = trimap.data.len();
    let mut is_fg = vec![false; n];
    let mut is_bg = vec![false; n];
    let mut is_known = vec![false; n];
    let mut is_unknown = vec![false; n];
    for (i, &v) in trimap.data.iter().enumerate() {
        is_fg[i] = v >= 0.9;
        is_bg[i] = v <= 0.1;
        is_known[i] = is_fg[i] || is_bg[i];
        is_unknown[i] = !is_known[i];
    }
    TrimapSplit {
        is_fg,
        is_bg,
        is_known,
        is_unknown,
    }
}

/// Build the closed-form matting Laplacian (levin08) for an RGB image.
///
/// Direct port of pymatting `cf_laplacian` with `radius = 1`: per 3x3
/// window, the regularized color covariance is inverted analytically and the
/// pairwise affinities are accumulated into a (2r+1)²-banded sparse matrix.
/// Windows fully inside `is_known` are skipped exactly as upstream (their
/// rows are never solved).
pub fn cf_laplacian(rgb: &[Plane; 3], epsilon: f64, is_known: &[bool]) -> Csr {
    let w = rgb[0].w;
    let h = rgb[0].h;
    let n = w * h;
    let r = 1usize; // radius fixed at 1 (upstream default; window 3x3)
    let size = 2 * r + 1;
    let window_area = (size * size) as f64;
    let band = 4 * r + 1; // 5: relative offsets -2..=2 in each axis

    // Banded value accumulator values[i][dy][dx] and CSR indices, exactly as
    // the upstream shape (all n rows carry band² slots; out-of-image slots
    // keep index 0 with value 0, matching the scipy construction).
    let mut values = vec![0f64; n * band * band];
    let mut indices = vec![0usize; n * band * band];
    let mut indptr = vec![0usize; n + 1];

    for yi in 0..h as isize {
        for xi in 0..w as isize {
            let i = (xi + yi * w as isize) as usize;
            let mut k = i * band * band;
            for yj in yi - 2..=yi + 2 {
                for xj in xi - 2..=xi + 2 {
                    if 0 <= xj && xj < w as isize && 0 <= yj && yj < h as isize {
                        indices[k] = (xj + yj * w as isize) as usize;
                    }
                    k += 1;
                }
            }
            indptr[i + 1] = k;
        }
    }

    // Centered window colors c[dy][dx][channel].
    let mut c = [[[0f64; 3]; 3]; 3];

    for y in r..h - r {
        for x in r..w - r {
            // Skip windows fully known.
            let mut all_known = true;
            'kn: for dy in 0..size {
                for dx in 0..size {
                    if !is_known[(y + dy - r) * w + (x + dx - r)] {
                        all_known = false;
                        break 'kn;
                    }
                }
            }
            if all_known {
                continue;
            }

            for (dc, plane) in rgb.iter().enumerate() {
                let mut s = 0.0;
                for dy in 0..size {
                    for dx in 0..size {
                        s += plane.at(x + dx - r, y + dy - r) as f64;
                    }
                }
                for (dy, row) in c.iter_mut().enumerate().take(size) {
                    for (dx, cell) in row.iter_mut().enumerate().take(size) {
                        cell[dc] = plane.at(x + dx - r, y + dy - r) as f64 - s / window_area;
                    }
                }
            }

            // Regularized color covariance (upper triangle).
            let mut a00 = epsilon;
            let mut a01 = 0.0;
            let mut a02 = 0.0;
            let mut a11 = epsilon;
            let mut a12 = 0.0;
            let mut a22 = epsilon;
            for row in c.iter().take(size) {
                for p in row.iter().take(size) {
                    a00 += p[0] * p[0];
                    a01 += p[0] * p[1];
                    a02 += p[0] * p[2];
                    a11 += p[1] * p[1];
                    a12 += p[1] * p[2];
                    a22 += p[2] * p[2];
                }
            }
            a00 /= window_area;
            a01 /= window_area;
            a02 /= window_area;
            a11 /= window_area;
            a12 /= window_area;
            a22 /= window_area;

            let det = a00 * a12 * a12 + a01 * a01 * a22 + a02 * a02 * a11
                - a00 * a11 * a22
                - 2.0 * a01 * a02 * a12;
            let inv_det = 1.0 / det;
            let m00 = (a12 * a12 - a11 * a22) * inv_det;
            let m01 = (a01 * a22 - a02 * a12) * inv_det;
            let m02 = (a02 * a11 - a01 * a12) * inv_det;
            let m11 = (a02 * a02 - a00 * a22) * inv_det;
            let m12 = (a00 * a12 - a01 * a02) * inv_det;
            let m22 = (a01 * a01 - a00 * a11) * inv_det;

            for dyi in 0..size {
                for dxi in 0..size {
                    let s = c[dyi][dxi][0];
                    let t = c[dyi][dxi][1];
                    let u = c[dyi][dxi][2];
                    let c0 = m00 * s + m01 * t + m02 * u;
                    let c1 = m01 * s + m11 * t + m12 * u;
                    let c2 = m02 * s + m12 * t + m22 * u;
                    for (dyj, rowj) in c.iter().enumerate().take(size) {
                        for (dxj, cj) in rowj.iter().enumerate().take(size) {
                            let xi = x + dxi - r;
                            let yi = y + dyi - r;
                            let xj = x + dxj - r;
                            let yj = y + dyj - r;
                            let i = xi + yi * w;
                            let temp = c0 * cj[0] + c1 * cj[1] + c2 * cj[2];
                            let same = if xi == xj && yi == yj { 1.0 } else { 0.0 };
                            let value = same - (1.0 + temp) / window_area;
                            let dx = (xj + 2) as isize - xi as isize;
                            let dy = (yj + 2) as isize - yi as isize;
                            values[i * band * band + dy as usize * band + dx as usize] += value;
                        }
                    }
                }
            }
        }
    }

    // The banded layout already IS a valid CSR with duplicate-free slots
    // (each (row, offset) pair occurs once); zero-valued out-of-image slots
    // all carry column index 0 and value 0 and are harmless for matvec, but
    // we compact them away for the ichol pattern's sake.
    let mut cindptr = vec![0usize];
    let mut cindices = Vec::new();
    let mut cvalues = Vec::new();
    for i in 0..n {
        // Collect (col, val) of this row's non-empty slots, keep ascending
        // column order (dy/dx scan already ascends in column index).
        for k in i * band * band..(i + 1) * band * band {
            let v = values[k];
            let j = indices[k];
            if v != 0.0 || (j == 0 && k == i * band * band && i == 0) {
                // keep genuinely stored entries; the (0,0) guard keeps at
                // least the diagonal slot of pixel 0 if it is zero
                cindices.push(j);
                cvalues.push(v);
            }
        }
        cindptr.push(cindices.len());
    }
    Csr {
        n,
        indptr: cindptr,
        indices: cindices,
        values: cvalues,
    }
}

/// Closed-form alpha matting (port of pymatting `estimate_alpha_cf`).
///
/// `rgb` are the color planes in [0, 1]; `trimap` uses the 0.1/0.9 split.
/// Returns the alpha plane clipped to [0, 1].
pub fn estimate_alpha_cf(
    rgb: &[Plane; 3],
    trimap: &Plane,
    epsilon: f64,
    cg_rtol: f64,
    cg_maxiter: usize,
) -> Result<Plane, String> {
    let w = trimap.w;
    let h = trimap.h;
    let split = trimap_split(trimap);
    if !split.is_unknown.iter().any(|&u| u) {
        // Fully known trimap: alpha is the (thresholded) trimap itself.
        let data = trimap
            .data
            .iter()
            .zip(&split.is_fg)
            .map(|(_, &fg)| if fg { 1.0 } else { 0.0 })
            .collect();
        return Ok(Plane::from_vec(w, h, data));
    }

    // Zero-RHS constraint-class short-circuit (lane ruling, Row B tester
    // defect; DELIBERATE, DOCUMENTED DEVIATION from upstream): with NO
    // foreground constraint the system is `L_U x_U = −R m` with `m ≡ 0`,
    // i.e. rhs = 0, and x_U = 0 is the EXACT solution of the constrained
    // system (definite pixels keep their trimap values). Symmetrically,
    // with NO background constraint every known pixel has m = 1, and
    // because the matting Laplacian is a graph Laplacian (rows sum to
    // zero), `L_U·1_U = −L_{U,K}·1_K = −R m`, so x_U = 1 is exact. Both
    // cases fire BEFORE the solver — deterministic, no parameter
    // dependence. Upstream (vendored pymatting, `solver/cg.py`) reaches
    // its CG with rhs = 0 and strict `<` guards against `atol = 0`: the
    // exact x = 0 iterate is rejected, the first step divides 0/0, and
    // the solve runs to its cap and errors — the short-circuit replaces
    // that consumer-seam hazard with the exact answer. An all-unknown
    // trimap (both classes empty) resolves through the no-foreground arm
    // to the all-zero alpha (an exact solution of the then-singular
    // system, chosen deterministically).
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

    let l = cf_laplacian(rgb, epsilon, &split.is_known);

    // L_U x_U = -R m  with m = is_fg over known pixels.
    let l_u = l.submatrix(&split.is_unknown, &split.is_unknown);
    let m_by_old: Vec<f64> = split
        .is_fg
        .iter()
        .map(|&f| if f { 1.0 } else { 0.0 })
        .collect();
    let mut rhs = Vec::new();
    l.masked_rect_matvec(&split.is_unknown, &split.is_known, &m_by_old, &mut rhs);
    for v in rhs.iter_mut() {
        *v = -*v;
    }

    let factor = ichol(&l_u.to_csc(), 1e-4)?;
    let x_u = cg(&l_u, &rhs, |r| factor.apply(r), cg_rtol, 0.0, cg_maxiter)?;

    let mut alpha = Plane::new(w, h);
    let mut ui = 0usize;
    for i in 0..w * h {
        let v = if split.is_unknown[i] {
            let v = x_u[ui];
            ui += 1;
            v
        } else if split.is_fg[i] {
            1.0
        } else {
            0.0
        };
        alpha.data[i] = (v.clamp(0.0, 1.0)) as f32;
    }
    Ok(alpha)
}

/// Multi-level foreground estimation (port of pymatting
/// `estimate_foreground_ml`), used for edge decontamination: returns the
/// estimated foreground color planes.
///
/// The coarse-to-fine scheme alternates gradient-descent-style local solves
/// of the foreground/background color model at each pyramid level.
pub fn estimate_foreground_ml(
    rgb: &[Plane; 3],
    alpha: &Plane,
    regularization: f64,
    n_small_iterations: usize,
    n_big_iterations: usize,
    small_size: usize,
) -> [Plane; 3] {
    let w = rgb[0].w;
    let h = rgb[0].h;

    // Recursive coarse-to-fine over power-of-two-ish levels, iterative form.
    // Build level dimension list from coarsest (<= small_size) to full.
    let mut dims = vec![(w, h)];
    let (mut cw, mut ch) = (w, h);
    while cw > small_size || ch > small_size {
        cw = cw.div_ceil(2);
        ch = ch.div_ceil(2);
        dims.push((cw, ch));
    }
    dims.reverse();

    let resize = |src: &Plane, dw: usize, dh: usize| -> Plane {
        // Nearest-neighbour resize (matches the upstream ml estimator's
        // simple pyramid resampling contract closely enough for the color
        // model; deterministic and exact).
        let mut dst = Plane::new(dw, dh);
        for y in 0..dh {
            for x in 0..dw {
                let sx = (x * src.w) / dw;
                let sy = (y * src.h) / dh;
                dst.data[y * dw + x] = src.at(sx, sy);
            }
        }
        dst
    };

    let mut fg = [Plane::new(1, 1), Plane::new(1, 1), Plane::new(1, 1)];
    let mut bg = [Plane::new(1, 1), Plane::new(1, 1), Plane::new(1, 1)];
    for c in 0..3 {
        fg[c] = Plane::new(dims[0].0, dims[0].1);
        bg[c] = Plane::new(dims[0].0, dims[0].1);
    }

    for (li, &(lw, lh)) in dims.iter().enumerate() {
        let a_l = resize(alpha, lw, lh);
        let img_l: Vec<Plane> = rgb.iter().map(|p| resize(p, lw, lh)).collect();
        if li > 0 {
            for c in 0..3 {
                fg[c] = resize(&fg[c], lw, lh);
                bg[c] = resize(&bg[c], lw, lh);
            }
        }
        let iters = if lw <= small_size && lh <= small_size {
            n_small_iterations
        } else {
            n_big_iterations
        };
        for _ in 0..iters {
            for y in 0..lh {
                for x in 0..lw {
                    let a = a_l.at(x, y) as f64;
                    // Neighbour regularization terms.
                    let neighbors = [
                        (x as isize - 1, y as isize),
                        (x as isize + 1, y as isize),
                        (x as isize, y as isize - 1),
                        (x as isize, y as isize + 1),
                    ];
                    for c in 0..3 {
                        let i_c = img_l[c].at(x, y) as f64;
                        // Model: I = a F + (1-a) B; local quadratic solve
                        // with smoothness on F and B (upstream's per-pixel
                        // update with gradient-of-alpha-weighted coupling).
                        let mut fg_num = a * i_c;
                        let mut fg_den = a * a + regularization;
                        let mut bg_num = (1.0 - a) * i_c;
                        let mut bg_den = (1.0 - a) * (1.0 - a) + regularization;
                        for &(nx, ny) in &neighbors {
                            let da = (a_l.at_clamped(nx, ny) as f64 - a).abs();
                            let wgt = regularization + da;
                            fg_num += wgt * fg[c].at_clamped(nx, ny) as f64;
                            fg_den += wgt;
                            bg_num += wgt * bg[c].at_clamped(nx, ny) as f64;
                            bg_den += wgt;
                        }
                        // Cross-coupling: subtract the other layer's share.
                        let b_cur = bg[c].at(x, y) as f64;
                        let f_new = ((fg_num - a * (1.0 - a) * b_cur) / fg_den).clamp(0.0, 1.0);
                        let f_cur = fg[c].at(x, y) as f64;
                        let b_new = ((bg_num - a * (1.0 - a) * f_cur) / bg_den).clamp(0.0, 1.0);
                        *fg[c].at_mut(x, y) = f_new as f32;
                        *bg[c].at_mut(x, y) = b_new as f32;
                    }
                }
            }
        }
    }
    fg
}

/// Shared test fixtures for the matting family (test builds only).
#[cfg(test)]
pub(crate) mod fixtures {
    use crate::plane::Plane;

    /// Constant foreground color of the two-color band fixture.
    pub const FIX_FG: [f32; 3] = [0.9, 0.8, 0.7];
    /// Constant background color of the two-color band fixture.
    pub const FIX_BG: [f32; 3] = [0.1, 0.15, 0.2];

    /// Synthetic matting fixture (the item-1 closed-form fixture, shared
    /// so every matting tier is measured on comparable ground): dark
    /// background, bright foreground, a 5-column unknown band (the
    /// trimap marks `band0..=band1` INCLUSIVE — `w/2 − 2 ..= w/2 + 2`)
    /// with a linear composite between them; the ground-truth alpha ramp
    /// spans the 4-pixel interval between the band edges. Returns
    /// (rgb, trimap, true alpha).
    pub fn two_color_band(w: usize, h: usize) -> ([Plane; 3], Plane, Vec<f64>) {
        let mut planes = [Plane::new(w, h), Plane::new(w, h), Plane::new(w, h)];
        let mut trimap = Plane::new(w, h);
        let mut true_alpha = vec![0f64; w * h];
        let band0 = w / 2 - 2;
        let band1 = w / 2 + 2;
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                let a = ((x as f64 - band0 as f64) / (band1 - band0) as f64).clamp(0.0, 1.0);
                true_alpha[i] = a;
                for c in 0..3 {
                    planes[c].data[i] = (a as f32) * FIX_FG[c] + (1.0 - a as f32) * FIX_BG[c];
                }
                trimap.data[i] = if x < band0 {
                    0.0
                } else if x > band1 {
                    1.0
                } else {
                    0.5
                };
            }
        }
        (planes, trimap, true_alpha)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plane::plane_hash;

    // The shared two-color-band fixture (`fixtures::two_color_band`).
    // Wave-1 kept a private byte-duplicate here; review MINOR item 3
    // replaced it with the shared fixture so the cross-tier
    // comparability claim cannot silently break. The composite stays
    // exactly on the two-color line, where the levin08 color-line model
    // is exact — the assertions below are unchanged.
    use fixtures::two_color_band as fixture;

    #[test]
    fn closed_form_alpha_recovers_linear_band() {
        let (rgb, trimap, truth) = fixture(16, 8);
        let alpha = estimate_alpha_cf(&rgb, &trimap, 1e-7, 1e-8, 2000).unwrap();
        // Known regions exact.
        for (i, &t) in trimap.data.iter().enumerate() {
            if t <= 0.1 {
                assert_eq!(alpha.data[i], 0.0);
            }
            if t >= 0.9 {
                assert_eq!(alpha.data[i], 1.0);
            }
        }
        // Unknown band close to the generating ramp (color-line model is
        // exact for a two-color composite, so tolerance is tight).
        // Index loop kept: the assertion references alpha.data[i]/truth[i].
        #[allow(clippy::needless_range_loop)]
        for i in 0..alpha.data.len() {
            assert!(
                (alpha.data[i] as f64 - truth[i]).abs() < 0.05,
                "alpha[{i}] = {} truth = {}",
                alpha.data[i],
                truth[i]
            );
        }
    }

    #[test]
    fn closed_form_alpha_is_deterministic() {
        let (rgb, trimap, _) = fixture(16, 8);
        let a1 = estimate_alpha_cf(&rgb, &trimap, 1e-7, 1e-8, 2000).unwrap();
        let a2 = estimate_alpha_cf(&rgb, &trimap, 1e-7, 1e-8, 2000).unwrap();
        assert_eq!(plane_hash(&a1.data), plane_hash(&a2.data));
    }

    #[test]
    fn laplacian_rows_sum_to_zero() {
        // The matting Laplacian is a graph Laplacian: each full row over an
        // interior window sums to ~0 (constant vectors in its null space).
        let (rgb, _, _) = fixture(12, 8);
        let is_known = vec![false; 12 * 8];
        let l = cf_laplacian(&rgb, 1e-7, &is_known);
        let ones = vec![1.0; l.n];
        let mut y = vec![0.0; l.n];
        l.matvec(&ones, &mut y);
        for (i, v) in y.iter().enumerate() {
            assert!(v.abs() < 1e-9, "row {i} sums to {v}");
        }
    }

    // ---- tester round (Row B): NaN input policy for the wave-1
    // closed-form family. The matting entries do NOT validate pixel
    // data (parameter validation only); the tests below pin what the
    // engines currently DO with a NaN pixel. Each pin documents current
    // behavior, not a contract.

    #[test]
    fn trimap_split_nan_pixel_classifies_as_unknown() {
        // NaN fails both `>= 0.9` and `<= 0.1`, so a NaN trimap pixel
        // lands in `is_unknown` — deterministic and side-effect-free.
        // Documents current behavior, not a contract.
        let trimap = Plane::from_vec(3, 1, vec![0.0, f32::NAN, 1.0]);
        let split = trimap_split(&trimap);
        assert!(split.is_bg[0] && split.is_fg[2]);
        assert!(!split.is_fg[1] && !split.is_bg[1] && !split.is_known[1]);
        assert!(split.is_unknown[1]);
    }

    #[test]
    fn closed_form_nan_rgb_returns_err_never_nan_alpha() {
        // A NaN color pixel reaches the Laplacian, poisons the CG
        // residual norm, and every convergence comparison goes false —
        // the solve runs to its cap and returns Err (measured:
        // "cg: no convergence within N iterations (|r| = NaN)").
        // The entry therefore FAILS LOUDLY instead of returning a
        // NaN-bearing alpha. Documents current behavior, not a
        // contract.
        let (rgb, trimap, _) = fixture(16, 8);
        let mut rgb_nan = rgb;
        rgb_nan[0].data[3 * 16 + 8] = f32::NAN; // inside the unknown band
        let r1 = estimate_alpha_cf(&rgb_nan, &trimap, 1e-7, 1e-8, 2000);
        let r2 = estimate_alpha_cf(&rgb_nan, &trimap, 1e-7, 1e-8, 2000);
        assert!(r1.is_err(), "expected Err, got Ok");
        // Same branch on both runs (deterministic failure).
        assert_eq!(r1.unwrap_err(), r2.unwrap_err());
    }

    #[test]
    fn closed_form_nan_trimap_pixel_solves_as_unknown_without_nan_output() {
        // Per the trimap_split pin above, a NaN trimap pixel is simply
        // one more unknown; with clean color data the solve proceeds
        // and the output carries no NaN (measured: the pixel solves to
        // its neighborhood-consistent alpha). Documents current
        // behavior, not a contract.
        let (rgb, trimap, _) = fixture(16, 8);
        let mut tri_nan = trimap;
        tri_nan.data[3 * 16 + 2] = f32::NAN; // interior of the bg region
        let alpha = estimate_alpha_cf(&rgb, &tri_nan, 1e-7, 1e-8, 2000).unwrap();
        assert!(alpha.data.iter().all(|v| v.is_finite()));
        assert!(alpha.data.iter().all(|&v| (0.0..=1.0).contains(&v)));
        // The NaN pixel sits deep in the background: its solved value
        // must land at the background side (measured 0.0 exactly).
        assert!(alpha.data[3 * 16 + 2] < 0.05);
    }

    #[test]
    fn foreground_estimation_matches_generating_colors_in_band() {
        let (rgb, trimap, _) = fixture(16, 8);
        let alpha = estimate_alpha_cf(&rgb, &trimap, 1e-7, 1e-8, 2000).unwrap();
        let fg = estimate_foreground_ml(&rgb, &alpha, 1e-5, 10, 2, 4);
        // Where alpha ~ 1 the foreground estimate must match the image.
        for y in 0..8 {
            for x in 12..16 {
                for c in 0..3 {
                    let d = (fg[c].at(x, y) - rgb[c].at(x, y)).abs();
                    assert!(d < 0.05, "fg[{c}]({x},{y}) off by {d}");
                }
            }
        }
    }
}
