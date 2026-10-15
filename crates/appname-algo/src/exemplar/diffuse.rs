// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §21.4 "coarsest initialization: fill H by boundary diffusion —
//! repeat over hole pixels in BFS order from ∂H: p ← mean of already-known/
//! filled 4-neighbors; iterate until stable"; ES §45.3 constant table
//! "Coarsest init" row and failure rule 1 (the same diffusion run at every
//! level as the insufficient-samples fallback); §21.4 "re-diffuse 1 px at
//! seams" on level upsample.
//!
//! This is the §21.4/§45.3 BFS boundary diffusion — a distinct stage from
//! the [`crate::inpaint`] fast-marching engine (that module is the
//! standalone small-defect fill; ALGORITHMS.md §3.B routes the CAF
//! degenerate path through *this* diffusion, "Telea/NS-class PDE smoothing
//! is the small-defect engine in §4, not a CAF fallback").
//!
//! Determinism: multi-source BFS layers expand in fixed raster order;
//! Gauss–Seidel sweeps run in the recorded BFS order with a fixed
//! tolerance and a fixed sweep cap. No search of any kind happens here.

use crate::plane::Plane;

const FOUR: [(isize, isize); 4] = [(-1, 0), (1, 0), (0, -1), (0, 1)];

/// Multi-source BFS distance from the non-hole region into the hole:
/// `0` outside the hole, `1` for hole pixels 4-adjacent to a non-hole
/// pixel, increasing inward; `u32::MAX` for hole pixels unreachable from
/// any known pixel (fully enclosed by canvas edges).
pub fn hole_depth(hole: &[bool], w: usize, h: usize) -> Vec<u32> {
    let mut depth = vec![u32::MAX; w * h];
    let mut frontier: Vec<usize> = Vec::new();
    for (i, &inside) in hole.iter().enumerate() {
        if !inside {
            depth[i] = 0;
            frontier.push(i);
        }
    }
    let mut d = 0u32;
    while !frontier.is_empty() {
        d += 1;
        let mut next = Vec::new();
        for &i in &frontier {
            let (x, y) = (i % w, i / w);
            for (dx, dy) in FOUR {
                let (xx, yy) = (x as isize + dx, y as isize + dy);
                if xx < 0 || yy < 0 || xx >= w as isize || yy >= h as isize {
                    continue;
                }
                let j = yy as usize * w + xx as usize;
                if hole[j] && depth[j] == u32::MAX {
                    depth[j] = d;
                    next.push(j);
                }
            }
        }
        // Fixed intra-layer order: BFS layer membership is order-free
        // (set semantics); sorting pins the traversal order exactly.
        next.sort_unstable();
        next.dedup();
        frontier = next;
    }
    depth
}

/// §21.4 boundary diffusion over `hole` pixels of `planes`, in place.
///
/// Pass 1 walks hole pixels in BFS order from ∂H and sets each to the mean
/// of its already-known/filled 4-neighbors; subsequent Gauss–Seidel sweeps
/// repeat the same order until stable (max update < `1e-7` in f64) or the
/// sweep cap (1024) is hit — "iterate until stable" with a deterministic
/// termination rule. Hole pixels unreachable from any known pixel are left
/// untouched (the §45.3 failure-rule-2 caller errors before this runs).
pub fn bfs_diffuse(planes: &mut [Plane], hole: &[bool], w: usize, h: usize) {
    let depth = hole_depth(hole, w, h);
    // BFS order: (depth, linear index) ascending — layer by layer, raster
    // order within a layer.
    let mut order: Vec<usize> = (0..w * h)
        .filter(|&i| hole[i] && depth[i] != u32::MAX)
        .collect();
    order.sort_unstable_by_key(|&i| (depth[i], i));

    // Pass 1: seed from already-known/filled neighbors only.
    let mut seeded = vec![false; w * h];
    for i in 0..w * h {
        if !hole[i] {
            seeded[i] = true;
        }
    }
    for &i in &order {
        let (x, y) = (i % w, i / w);
        for plane in planes.iter_mut() {
            let mut sum = 0f64;
            let mut cnt = 0usize;
            for (dx, dy) in FOUR {
                let (xx, yy) = (x as isize + dx, y as isize + dy);
                if xx < 0 || yy < 0 || xx >= w as isize || yy >= h as isize {
                    continue;
                }
                let j = yy as usize * w + xx as usize;
                if seeded[j] {
                    sum += plane.data[j] as f64;
                    cnt += 1;
                }
            }
            if cnt > 0 {
                plane.data[i] = (sum / cnt as f64) as f32;
            }
        }
        seeded[i] = true;
    }

    // Gauss–Seidel to stability in the same order.
    for _sweep in 0..1024 {
        let mut max_delta = 0f64;
        for &i in &order {
            let (x, y) = (i % w, i / w);
            for plane in planes.iter_mut() {
                let mut sum = 0f64;
                let mut cnt = 0usize;
                for (dx, dy) in FOUR {
                    let (xx, yy) = (x as isize + dx, y as isize + dy);
                    if xx < 0 || yy < 0 || xx >= w as isize || yy >= h as isize {
                        continue;
                    }
                    let j = yy as usize * w + xx as usize;
                    if !hole[j] || depth[j] != u32::MAX {
                        sum += plane.data[j] as f64;
                        cnt += 1;
                    }
                }
                if cnt > 0 {
                    let v = (sum / cnt as f64) as f32;
                    max_delta = max_delta.max((v as f64 - plane.data[i] as f64).abs());
                    plane.data[i] = v;
                }
            }
        }
        if max_delta < 1e-7 {
            break;
        }
    }
}

/// §21.4 upsample repair: one diffusion update over hole pixels within
/// 1 px (4-connectivity) of an ownership seam — "re-diffuse 1 px at
/// seams". `owner` is the upsampled per-pixel patch-ownership id
/// (`u32::MAX` = unowned/known). Runs a single fixed-order pass.
pub fn rediffuse_seams(planes: &mut [Plane], hole: &[bool], owner: &[u32], w: usize, h: usize) {
    let mut at_seam = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if !hole[i] {
                continue;
            }
            for (dx, dy) in FOUR {
                let (xx, yy) = (x as isize + dx, y as isize + dy);
                if xx < 0 || yy < 0 || xx >= w as isize || yy >= h as isize {
                    continue;
                }
                let j = yy as usize * w + xx as usize;
                if hole[j] && owner[j] != owner[i] {
                    at_seam[i] = true;
                }
            }
        }
    }
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if !at_seam[i] {
                continue;
            }
            for plane in planes.iter_mut() {
                let mut sum = 0f64;
                let mut cnt = 0usize;
                for (dx, dy) in FOUR {
                    let (xx, yy) = (x as isize + dx, y as isize + dy);
                    if xx < 0 || yy < 0 || xx >= w as isize || yy >= h as isize {
                        continue;
                    }
                    let j = yy as usize * w + xx as usize;
                    sum += plane.data[j] as f64;
                    cnt += 1;
                }
                if cnt > 0 {
                    plane.data[i] = (sum / cnt as f64) as f32;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect_hole(w: usize, h: usize, x0: usize, y0: usize, x1: usize, y1: usize) -> Vec<bool> {
        let mut m = vec![false; w * h];
        for y in y0..y1 {
            for x in x0..x1 {
                m[y * w + x] = true;
            }
        }
        m
    }

    #[test]
    fn constant_boundary_diffuses_to_constant() {
        let mut p = Plane::from_vec(16, 16, vec![0.6; 256]);
        let hole = rect_hole(16, 16, 4, 4, 12, 12);
        for (i, &inside) in hole.iter().enumerate() {
            if inside {
                p.data[i] = 0.0;
            }
        }
        bfs_diffuse(std::slice::from_mut(&mut p), &hole, 16, 16);
        for &v in &p.data {
            assert!((v - 0.6).abs() < 1e-5);
        }
    }

    #[test]
    fn linear_ramp_is_harmonic_fixed_point() {
        let (w, h) = (20, 14);
        let mut p = Plane::new(w, h);
        for y in 0..h {
            for x in 0..w {
                p.data[y * w + x] = x as f32 / (w - 1) as f32;
            }
        }
        let truth = p.clone();
        let hole = rect_hole(w, h, 6, 4, 14, 10);
        for (i, &inside) in hole.iter().enumerate() {
            if inside {
                p.data[i] = 9.0; // garbage to be replaced
            }
        }
        bfs_diffuse(std::slice::from_mut(&mut p), &hole, w, h);
        for i in 0..w * h {
            assert!(
                (p.data[i] - truth.data[i]).abs() < 5e-3,
                "pixel {i}: {} vs {}",
                p.data[i],
                truth.data[i]
            );
        }
    }

    #[test]
    fn depth_layers_are_correct() {
        let hole = rect_hole(8, 8, 2, 2, 6, 6);
        let d = hole_depth(&hole, 8, 8);
        assert_eq!(d[0], 0);
        assert_eq!(d[2 * 8 + 2], 1); // corner of hole
        assert_eq!(d[3 * 8 + 3], 2);
        assert_eq!(d[4 * 8 + 4], 2); // 4x4 hole interior max depth 2
    }
}
