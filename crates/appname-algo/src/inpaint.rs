// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Implemented from the published method (A. Telea, "An Image Inpainting
// Technique Based on the Fast Marching Method", JGT 9(1), 2004 — census
// paper key telea04; Navier-Stokes-class diffusion per bertalmio01 for the
// harmonic variant). NO OpenCV expression is ported here: the census row
// (B1-opencv-inpaint.yaml, Apache-2.0 at pin 21d60a9a, files_taken: [])
// routes this as "Rust port of the Telea FMM contract; OpenCV output used
// as behavior oracle in tests, not linked, not copied" — behavior-oracle
// use is credited in the area dossier per the obligations column.

//! Spec: ALGORITHMS.md §3 `content_aware_fill` — fallback
//! `boundary_diffusion` (level L0); ES §21.4 / §45.3 (fallback tier), §22.4;
//! INTEGRATION-NOTES §6 order 4.
//!
//! Boundary-diffusion hole filling, the L0 fallback under the
//! content-aware-fill surface and the spot-heal seed fill:
//!
//! * [`telea_inpaint`] — fast-marching inpainting: the hole is traversed in
//!   distance order from the boundary; each pixel is a normalized weighted
//!   average (direction · distance · level-set weights) of already-known
//!   neighbours with first-order gradient extrapolation.
//! * [`harmonic_fill`] — plain membrane diffusion (Gauss–Seidel on the
//!   Laplace equation), the analytically checkable cross-oracle for the
//!   same boundary conditions.
//!
//! Rights invariants: this fallback carries none of the §45.3
//! invariant-restricted structures — the march is a deterministic distance
//! front, there is no candidate-search field of any kind and no randomized
//! search window (contract `invariants` row quoted in the area docs).
//!
//! Determinism: the FMM heap orders by (distance bits, y, x) — bit-exact
//! tie-breaking, no platform-dependent float ordering; fixed-seed golden in
//! tests.

use crate::plane::Plane;
use std::collections::BinaryHeap;

/// Pixel states during the march.
#[derive(Clone, Copy, PartialEq)]
enum Flag {
    /// Value known (outside the hole, or already inpainted).
    Known,
    /// On the narrow band (front).
    Band,
    /// Unknown interior.
    Inside,
}

/// Max-heap entry ordered so the SMALLEST distance pops first, with total
/// deterministic order (distance bits, then y, then x).
#[derive(PartialEq, Eq)]
struct FrontEntry {
    t_bits: u64,
    y: u32,
    x: u32,
}

impl Ord for FrontEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Reverse: BinaryHeap is a max-heap, we want min-T first.
        other
            .t_bits
            .cmp(&self.t_bits)
            .then(other.y.cmp(&self.y))
            .then(other.x.cmp(&self.x))
    }
}

impl PartialOrd for FrontEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

const FOUR: [(isize, isize); 4] = [(-1, 0), (1, 0), (0, -1), (0, 1)];

/// Eikonal update: solve `|∇T| = 1` at (x, y) from known/band neighbours
/// (the standard two-axis quadratic, quadrant minimum).
fn eikonal(t: &[f64], flags: &[Flag], w: usize, h: usize, x: usize, y: usize) -> f64 {
    let get = |xx: isize, yy: isize| -> Option<f64> {
        if xx < 0 || yy < 0 || xx >= w as isize || yy >= h as isize {
            return None;
        }
        let i = yy as usize * w + xx as usize;
        if flags[i] == Flag::Inside {
            None
        } else {
            Some(t[i])
        }
    };
    let xi = x as isize;
    let yi = y as isize;
    let a = match (get(xi - 1, yi), get(xi + 1, yi)) {
        (Some(l), Some(r)) => Some(l.min(r)),
        (Some(l), None) => Some(l),
        (None, Some(r)) => Some(r),
        (None, None) => None,
    };
    let b = match (get(xi, yi - 1), get(xi, yi + 1)) {
        (Some(u), Some(d)) => Some(u.min(d)),
        (Some(u), None) => Some(u),
        (None, Some(d)) => Some(d),
        (None, None) => None,
    };
    match (a, b) {
        (Some(a), Some(b)) => {
            let diff = (a - b).abs();
            if diff < 1.0 {
                0.5 * (a + b + (2.0 - diff * diff).sqrt())
            } else {
                a.min(b) + 1.0
            }
        }
        (Some(a), None) => a + 1.0,
        (None, Some(b)) => b + 1.0,
        (None, None) => f64::INFINITY,
    }
}

/// Telea fast-marching inpainting.
///
/// `planes` are the channels to fill (same dimensions); `mask` is > 0.5
/// inside the hole. `radius` is the neighbourhood radius ε (paper default
/// class: 3..6 px). Returns filled planes.
pub fn telea_inpaint(planes: &[Plane], mask: &Plane, radius: usize) -> Vec<Plane> {
    let w = mask.w;
    let h = mask.h;
    for p in planes {
        assert!(p.w == w && p.h == h, "telea_inpaint: dimension mismatch");
    }
    let n = w * h;
    let radius = radius.max(1);

    let mut flags = vec![Flag::Known; n];
    let mut t = vec![0f64; n];
    for i in 0..n {
        if mask.data[i] > 0.5 {
            flags[i] = Flag::Inside;
            t[i] = f64::INFINITY;
        }
    }
    // Initial band: known pixels bordering the hole get T = 0 fronts; the
    // hole-side boundary pixels enter the heap with their eikonal estimate.
    let mut heap = BinaryHeap::new();
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if flags[i] != Flag::Inside {
                continue;
            }
            let mut is_boundary = false;
            for (dx, dy) in FOUR {
                let xx = x as isize + dx;
                let yy = y as isize + dy;
                if xx >= 0 && yy >= 0 && xx < w as isize && yy < h as isize {
                    let j = yy as usize * w + xx as usize;
                    if flags[j] == Flag::Known {
                        is_boundary = true;
                    }
                }
            }
            if is_boundary {
                flags[i] = Flag::Band;
                let est = eikonal(&t, &flags, w, h, x, y);
                t[i] = est;
                heap.push(FrontEntry {
                    t_bits: est.to_bits(),
                    y: y as u32,
                    x: x as u32,
                });
            }
        }
    }

    let mut out: Vec<Plane> = planes.to_vec();

    // Central-difference gradient of T where both neighbours are usable.
    let grad_t = |t: &[f64], flags: &[Flag], x: usize, y: usize| -> (f64, f64) {
        let sample = |xx: isize, yy: isize| -> Option<f64> {
            if xx < 0 || yy < 0 || xx >= w as isize || yy >= h as isize {
                return None;
            }
            let i = yy as usize * w + xx as usize;
            if flags[i] == Flag::Inside || !t[i].is_finite() {
                None
            } else {
                Some(t[i])
            }
        };
        let xi = x as isize;
        let yi = y as isize;
        let here = t[y * w + x];
        let gx = match (sample(xi - 1, yi), sample(xi + 1, yi)) {
            (Some(l), Some(r)) => (r - l) * 0.5,
            (Some(l), None) => here - l,
            (None, Some(r)) => r - here,
            (None, None) => 0.0,
        };
        let gy = match (sample(xi, yi - 1), sample(xi, yi + 1)) {
            (Some(u), Some(d)) => (d - u) * 0.5,
            (Some(u), None) => here - u,
            (None, Some(d)) => d - here,
            (None, None) => 0.0,
        };
        (gx, gy)
    };

    while let Some(FrontEntry { t_bits, y, x }) = heap.pop() {
        let x = x as usize;
        let y = y as usize;
        let i = y * w + x;
        if flags[i] == Flag::Known {
            continue; // stale heap entry
        }
        if t[i].to_bits() != t_bits {
            continue; // superseded estimate
        }

        // Inpaint this pixel from known pixels within the ε-ball (telea04
        // §3: w = dir * dst * lev, first-order image gradient
        // extrapolation).
        let (ntx, nty) = grad_t(&t, &flags, x, y);
        let nlen = (ntx * ntx + nty * nty).sqrt();
        let (ntx, nty) = if nlen > 1e-12 {
            (ntx / nlen, nty / nlen)
        } else {
            (0.0, 0.0)
        };
        let t_here = t[i];
        let d0 = 1.0f64; // reference distance (1 px)
        let mut wsum = 0.0f64;
        let mut acc = vec![0f64; planes.len()];
        let r = radius as isize;
        for dy in -r..=r {
            for dx in -r..=r {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let xx = x as isize + dx;
                let yy = y as isize + dy;
                if xx < 0 || yy < 0 || xx >= w as isize || yy >= h as isize {
                    continue;
                }
                let j = yy as usize * w + xx as usize;
                if flags[j] != Flag::Known {
                    continue;
                }
                let rx = -(dx as f64); // vector from neighbour toward pixel
                let ry = -(dy as f64);
                let rlen2 = rx * rx + ry * ry;
                if rlen2 as usize > radius * radius {
                    continue;
                }
                let rlen = rlen2.sqrt();
                let dir = ((rx * ntx + ry * nty) / rlen).abs().max(1e-6);
                let dst = d0 * d0 / rlen2;
                let lev = d0 / (d0 + (t_here - t[j]).abs());
                let wgt = dir * dst * lev;
                // First-order extrapolation along the image gradient at the
                // known neighbour (central differences over known pixels).
                for (c, plane) in out.iter().enumerate() {
                    let sample = |xx2: isize, yy2: isize| -> Option<f64> {
                        if xx2 < 0 || yy2 < 0 || xx2 >= w as isize || yy2 >= h as isize {
                            return None;
                        }
                        let k = yy2 as usize * w + xx2 as usize;
                        if flags[k] == Flag::Known {
                            Some(plane.data[k] as f64)
                        } else {
                            None
                        }
                    };
                    let v = plane.data[j] as f64;
                    let gx = match (sample(xx - 1, yy), sample(xx + 1, yy)) {
                        (Some(l), Some(rr)) => (rr - l) * 0.5,
                        _ => 0.0,
                    };
                    let gy = match (sample(xx, yy - 1), sample(xx, yy + 1)) {
                        (Some(u), Some(d)) => (d - u) * 0.5,
                        _ => 0.0,
                    };
                    acc[c] += wgt * (v + gx * rx + gy * ry);
                }
                wsum += wgt;
            }
        }
        if wsum > 0.0 {
            for (c, plane) in out.iter_mut().enumerate() {
                plane.data[i] = (acc[c] / wsum) as f32;
            }
        }
        flags[i] = Flag::Known;

        // Advance the front into remaining INSIDE neighbours.
        for (dx, dy) in FOUR {
            let xx = x as isize + dx;
            let yy = y as isize + dy;
            if xx < 0 || yy < 0 || xx >= w as isize || yy >= h as isize {
                continue;
            }
            let j = yy as usize * w + xx as usize;
            if flags[j] == Flag::Known {
                continue;
            }
            let est = eikonal(&t, &flags, w, h, xx as usize, yy as usize);
            if flags[j] == Flag::Inside || est < t[j] {
                flags[j] = Flag::Band;
                t[j] = est;
                heap.push(FrontEntry {
                    t_bits: est.to_bits(),
                    y: yy as u32,
                    x: xx as u32,
                });
            }
        }
    }
    out
}

/// Harmonic (membrane) fill: Gauss–Seidel iterations of the Laplace
/// equation inside the mask with Dirichlet boundary from the known pixels
/// — the diffusion-class cross-oracle (bertalmio01-class steady state).
pub fn harmonic_fill(plane: &Plane, mask: &Plane, iterations: usize) -> Plane {
    let w = plane.w;
    let h = plane.h;
    let mut out = plane.clone();
    // Seed the hole with the boundary mean for faster settling.
    let mut sum = 0.0f64;
    let mut cnt = 0usize;
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if mask.data[i] <= 0.5 {
                continue;
            }
            for (dx, dy) in FOUR {
                let xx = x as isize + dx;
                let yy = y as isize + dy;
                if xx >= 0 && yy >= 0 && xx < w as isize && yy < h as isize {
                    let j = yy as usize * w + xx as usize;
                    if mask.data[j] <= 0.5 {
                        sum += plane.data[j] as f64;
                        cnt += 1;
                    }
                }
            }
        }
    }
    let seed = if cnt > 0 {
        (sum / cnt as f64) as f32
    } else {
        0.0
    };
    for i in 0..w * h {
        if mask.data[i] > 0.5 {
            out.data[i] = seed;
        }
    }
    for _ in 0..iterations {
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if mask.data[i] <= 0.5 {
                    continue;
                }
                let mut s = 0.0f64;
                let mut k = 0usize;
                for (dx, dy) in FOUR {
                    let v = out.at_clamped(x as isize + dx, y as isize + dy);
                    s += v as f64;
                    k += 1;
                }
                out.data[i] = (s / k as f64) as f32;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plane::{plane_hash, SplitMix64};

    fn disk_mask(w: usize, h: usize, cx: f64, cy: f64, r: f64) -> Plane {
        let mut m = Plane::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let d = ((x as f64 - cx).powi(2) + (y as f64 - cy).powi(2)).sqrt();
                m.data[y * w + x] = if d <= r { 1.0 } else { 0.0 };
            }
        }
        m
    }

    #[test]
    fn constant_region_fills_exactly() {
        let p = Plane::from_vec(24, 24, vec![0.42; 24 * 24]);
        let m = disk_mask(24, 24, 12.0, 12.0, 6.0);
        let filled = telea_inpaint(std::slice::from_ref(&p), &m, 3);
        for (i, &v) in filled[0].data.iter().enumerate() {
            assert!(
                (v - 0.42).abs() < 1e-5,
                "pixel {i} = {v} (constant fill must stay constant)"
            );
        }
        let hf = harmonic_fill(&p, &m, 200);
        for &v in &hf.data {
            assert!((v - 0.42).abs() < 1e-5);
        }
    }

    #[test]
    fn linear_ramp_is_recovered() {
        // A linear function is harmonic: the membrane fill reproduces it
        // (tight); the FMM weighted-average with gradient extrapolation
        // tracks it closely (looser).
        let w = 32;
        let h = 24;
        let mut p = Plane::new(w, h);
        for y in 0..h {
            for x in 0..w {
                p.data[y * w + x] = 0.2 + 0.6 * x as f32 / (w - 1) as f32;
            }
        }
        let m = disk_mask(w, h, 15.0, 12.0, 5.0);
        let hf = harmonic_fill(&p, &m, 2000);
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                assert!(
                    (hf.data[i] - p.data[i]).abs() < 2e-3,
                    "harmonic ({x},{y}): {} vs {}",
                    hf.data[i],
                    p.data[i]
                );
            }
        }
        let t = telea_inpaint(&[p.clone()], &m, 4);
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                assert!(
                    (t[0].data[i] - p.data[i]).abs() < 0.05,
                    "telea ({x},{y}): {} vs {}",
                    t[0].data[i],
                    p.data[i]
                );
            }
        }
    }

    #[test]
    fn filled_values_stay_in_boundary_hull() {
        // Weighted averaging + small-gradient extrapolation must not fling
        // values far outside the surrounding value range.
        let w = 28;
        let h = 28;
        let mut rng = SplitMix64(99);
        let mut p = Plane::new(w, h);
        for v in p.data.iter_mut() {
            *v = (0.3 + 0.4 * rng.next_f64()) as f32;
        }
        let m = disk_mask(w, h, 14.0, 14.0, 6.0);
        let filled = telea_inpaint(&[p], &m, 3);
        for (i, &v) in filled[0].data.iter().enumerate() {
            if m.data[i] > 0.5 {
                assert!(v > 0.1 && v < 0.9, "pixel {i} escaped hull: {v}");
            }
        }
    }

    #[test]
    fn inpainting_golden_and_determinism() {
        // Fixed-seed structured fixture (two-tone + noise), pinned hash.
        let w = 32;
        let h = 32;
        let mut rng = SplitMix64(0x7E1EA);
        let mut p = Plane::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let base = if x < w / 2 { 0.25 } else { 0.75 };
                p.data[y * w + x] = base + 0.05 * rng.next_f64() as f32;
            }
        }
        let m = disk_mask(w, h, 16.0, 16.0, 7.0);
        let a = telea_inpaint(&[p.clone()], &m, 4);
        let b = telea_inpaint(&[p], &m, 4);
        assert_eq!(plane_hash(&a[0].data), plane_hash(&b[0].data));
        assert_eq!(
            plane_hash(&a[0].data),
            GOLDEN,
            "golden hash drift — investigate before re-pinning (O-QA rule)"
        );
    }

    // Pinned 2026-08-13 after the behavioral oracles above were green
    // (constant exactness, ramp recovery, hull containment).
    const GOLDEN: u64 = 1669578233529008005;
}
