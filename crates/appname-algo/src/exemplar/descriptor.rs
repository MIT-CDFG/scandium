// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §45.3 constant table "Descriptor projection" row (PCA → 16
//! dims; basis fit per level on ≤ 2¹⁵ S-patches sampled by stream
//! `"appname.caf.pca"`); §21.4 DESCRIPTORS stage. Patch-descriptor
//! machinery is the hertzmann01 approximate-nearest-neighbor lineage
//! (clean-room from the paper; census key `caf_primary_cleanroom`,
//! verdict none-found by design).
//!
//! Deterministic PCA: covariance accumulated in f64 over the seeded
//! sample draw, then a FIXED 40-round orthogonal (subspace) iteration
//! with modified Gram–Schmidt — no tolerance branches, no data-dependent
//! iteration counts, so the basis is a pure function of (samples, seed).
//! The basis is fit once per level on the identity library's S-patches
//! and shared by every augmented library's index and by query projection
//! (the projection is a pruning device only — final candidate choice is
//! always the exact 7×7 SSD re-rank, §45.3 "Index" row).

use crate::exemplar::stream::{draw_u64, STREAM_PCA};
use crate::plane::Plane;

/// Patch radius r = 3 → 7×7 patches (§45.3 constant table).
pub const PATCH_RADIUS: usize = 3;
/// Patch side = 7.
pub const PATCH_SIDE: usize = 2 * PATCH_RADIUS + 1;
/// Pixels per patch = 49.
pub const PATCH_AREA: usize = PATCH_SIDE * PATCH_SIDE;
/// Descriptor dimensionality after projection (§45.3: 16).
pub const PCA_DIMS: usize = 16;
/// PCA sample budget: ≤ 2¹⁵ S-patches (§45.3).
pub const PCA_SAMPLE_CAP: usize = 1 << 15;

/// Centers whose full 7×7 patch lies inside the canvas AND inside the
/// valid-sample mask (§21.4 candidate rule: "a patch is a valid candidate
/// only when it lies fully inside S"). Raster order (y, then x) —
/// deterministic.
pub fn valid_centers(valid: &[bool], w: usize, h: usize) -> Vec<(u32, u32)> {
    let r = PATCH_RADIUS;
    let mut out = Vec::new();
    if w < PATCH_SIDE || h < PATCH_SIDE {
        return out;
    }
    for y in r..h - r {
        'px: for x in r..w - r {
            for dy in 0..PATCH_SIDE {
                for dx in 0..PATCH_SIDE {
                    if !valid[(y - r + dy) * w + (x - r + dx)] {
                        continue 'px;
                    }
                }
            }
            out.push((x as u32, y as u32));
        }
    }
    out
}

/// Flattened patch values (channel-major, then row-major offsets),
/// edge-clamped. Length = `planes.len() * PATCH_AREA`.
pub fn extract_patch(planes: &[Plane], cx: usize, cy: usize, out: &mut Vec<f64>) {
    out.clear();
    let r = PATCH_RADIUS as isize;
    for p in planes {
        for dy in -r..=r {
            for dx in -r..=r {
                out.push(p.at_clamped(cx as isize + dx, cy as isize + dy) as f64);
            }
        }
    }
}

/// A fitted per-level PCA basis: `project` maps a raw patch vector to the
/// 16-dim descriptor space.
pub struct Pca {
    /// Raw patch dimensionality (`channels * PATCH_AREA`).
    pub dim: usize,
    /// Number of retained components (`min(16, dim)`).
    pub k: usize,
    /// Sample mean, length `dim`.
    pub mean: Vec<f64>,
    /// Row-major basis, `k` rows of length `dim` (orthonormal, or zero
    /// rows for degenerate directions).
    pub basis: Vec<f64>,
}

impl Pca {
    /// Project a raw patch vector (length `dim`) into descriptor space.
    pub fn project(&self, x: &[f64], out: &mut Vec<f64>) {
        debug_assert_eq!(x.len(), self.dim);
        out.clear();
        for row in 0..self.k {
            let b = &self.basis[row * self.dim..(row + 1) * self.dim];
            let mut acc = 0.0;
            for j in 0..self.dim {
                acc += b[j] * (x[j] - self.mean[j]);
            }
            out.push(acc);
        }
    }
}

/// Fit the per-level PCA basis on the identity library's valid S-patch
/// centers. When more than 2¹⁵ centers exist, the sample set is the
/// seeded draw of §45.3 (`STREAM_PCA`: draw `k` reads counter `k`, index
/// = `draw mod n` — duplicates permissible, the draw is a covariance
/// estimator, not a selection of exclusive rights of way).
pub fn fit_pca(planes: &[Plane], centers: &[(u32, u32)], key: &[u8; 32]) -> Pca {
    let channels = planes.len();
    let dim = channels * PATCH_AREA;
    let k = PCA_DIMS.min(dim);
    let n = centers.len();

    // Sample index list, deterministic.
    let samples: Vec<usize> = if n <= PCA_SAMPLE_CAP {
        (0..n).collect()
    } else {
        (0..PCA_SAMPLE_CAP)
            .map(|i| (draw_u64(key, STREAM_PCA, i as u64) % n as u64) as usize)
            .collect()
    };

    let mut mean = vec![0f64; dim];
    let mut patch = Vec::with_capacity(dim);
    if !samples.is_empty() {
        for &si in &samples {
            let (cx, cy) = centers[si];
            extract_patch(planes, cx as usize, cy as usize, &mut patch);
            for (m, &v) in mean.iter_mut().zip(patch.iter()) {
                *m += v;
            }
        }
        let inv = 1.0 / samples.len() as f64;
        for m in mean.iter_mut() {
            *m *= inv;
        }
    }

    // Covariance (f64, fixed accumulation order).
    let mut cov = vec![0f64; dim * dim];
    for &si in &samples {
        let (cx, cy) = centers[si];
        extract_patch(planes, cx as usize, cy as usize, &mut patch);
        for a in 0..dim {
            let da = patch[a] - mean[a];
            let row = &mut cov[a * dim..(a + 1) * dim];
            for (b, slot) in row.iter_mut().enumerate() {
                *slot += da * (patch[b] - mean[b]);
            }
        }
    }
    if !samples.is_empty() {
        let inv = 1.0 / samples.len() as f64;
        for c in cov.iter_mut() {
            *c *= inv;
        }
    }

    // Top-k subspace by fixed-round orthogonal iteration. Init: unit
    // vectors on the k largest-variance coordinates (ties by index).
    let mut diag_order: Vec<usize> = (0..dim).collect();
    diag_order.sort_by(|&a, &b| {
        cov[b * dim + b]
            .total_cmp(&cov[a * dim + a])
            .then(a.cmp(&b))
    });
    let mut basis = vec![0f64; k * dim];
    for row in 0..k {
        basis[row * dim + diag_order[row]] = 1.0;
    }
    let mut tmp = vec![0f64; dim];
    for _round in 0..40 {
        for row in 0..k {
            // tmp = cov · basis_row
            for (a, slot) in tmp.iter_mut().enumerate() {
                let crow = &cov[a * dim..(a + 1) * dim];
                let brow = &basis[row * dim..(row + 1) * dim];
                let mut acc = 0.0;
                for j in 0..dim {
                    acc += crow[j] * brow[j];
                }
                *slot = acc;
            }
            // Modified Gram–Schmidt against previous rows.
            for prev in 0..row {
                let prow = &basis[prev * dim..(prev + 1) * dim];
                let mut dot = 0.0;
                for j in 0..dim {
                    dot += tmp[j] * prow[j];
                }
                for j in 0..dim {
                    tmp[j] -= dot * prow[j];
                }
            }
            let norm = tmp.iter().map(|v| v * v).sum::<f64>().sqrt();
            let brow = &mut basis[row * dim..(row + 1) * dim];
            if norm > 1e-12 {
                for (slot, &v) in brow.iter_mut().zip(tmp.iter()) {
                    *slot = v / norm;
                }
            } else {
                // Degenerate direction (e.g. constant image): keep a zero
                // row — projections along it are 0, deterministically.
                for slot in brow.iter_mut() {
                    *slot = 0.0;
                }
            }
        }
    }
    Pca {
        dim,
        k,
        mean,
        basis,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exemplar::stream::level_key;

    #[test]
    fn valid_centers_respect_patch_footprint() {
        let (w, h) = (10, 9);
        let valid = vec![true; w * h];
        let c = valid_centers(&valid, w, h);
        // Interior (10−6)×(9−6) = 4×3 centers.
        assert_eq!(c.len(), 12);
        assert_eq!(c[0], (3, 3));
        assert_eq!(*c.last().unwrap(), (6, 5));
    }

    #[test]
    fn pca_finds_the_dominant_axis() {
        // A horizontal ramp: variance concentrates in the x-offset
        // coordinates; a patch's projection must separate left-shifted
        // from right-shifted patches.
        let (w, h) = (24, 12);
        let mut p = Plane::new(w, h);
        for y in 0..h {
            for x in 0..w {
                p.data[y * w + x] = x as f32 / w as f32;
            }
        }
        let planes = vec![p];
        let valid = vec![true; w * h];
        let centers = valid_centers(&valid, w, h);
        let key = level_key(0, 0);
        let pca = fit_pca(&planes, &centers, &key);
        assert_eq!(pca.dim, PATCH_AREA);
        assert_eq!(pca.k, PCA_DIMS);
        // Projections of patches at different x must differ; same x same.
        let mut pa = Vec::new();
        let mut pb = Vec::new();
        let mut da = Vec::new();
        let mut db = Vec::new();
        let mut dc = Vec::new();
        extract_patch(&planes, 5, 5, &mut pa);
        extract_patch(&planes, 15, 5, &mut pb);
        pca.project(&pa, &mut da);
        pca.project(&pb, &mut db);
        extract_patch(&planes, 5, 7, &mut pa);
        pca.project(&pa, &mut dc);
        let dist_ab: f64 = da.iter().zip(&db).map(|(a, b)| (a - b) * (a - b)).sum();
        let dist_ac: f64 = da.iter().zip(&dc).map(|(a, b)| (a - b) * (a - b)).sum();
        assert!(dist_ab > 1e-4, "different columns must separate");
        assert!(dist_ac < 1e-12, "same column must coincide");
    }

    #[test]
    fn pca_is_deterministic_and_seed_stable() {
        let (w, h) = (16, 16);
        let mut p = Plane::new(w, h);
        for (i, v) in p.data.iter_mut().enumerate() {
            *v = ((i * 37) % 101) as f32 / 101.0;
        }
        let planes = vec![p];
        let valid = vec![true; w * h];
        let centers = valid_centers(&valid, w, h);
        let key = level_key(3, 1);
        let a = fit_pca(&planes, &centers, &key);
        let b = fit_pca(&planes, &centers, &key);
        assert_eq!(a.basis, b.basis);
        assert_eq!(a.mean, b.mean);
    }

    #[test]
    fn constant_image_degenerates_deterministically() {
        let p = Plane::from_vec(12, 12, vec![0.5; 144]);
        let planes = vec![p];
        let valid = vec![true; 144];
        let centers = valid_centers(&valid, 12, 12);
        let key = level_key(0, 0);
        let pca = fit_pca(&planes, &centers, &key);
        let mut raw = Vec::new();
        let mut d = Vec::new();
        extract_patch(&planes, 6, 6, &mut raw);
        pca.project(&raw, &mut d);
        for &v in &d {
            assert_eq!(v, 0.0);
        }
    }
}
