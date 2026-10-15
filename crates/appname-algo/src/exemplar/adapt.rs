// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §45.3 constant table "Boundary feather (edge decontamination)"
//! and "Color adaptation ring" rows; §45.3 "Color adaptation levels"
//! (`none` / `default` global gain-bias / `high` per-64×64-cell bilinear)
//! and the Content-Aware Move strength mapping (`gain' = 1 + w·(gain−1)`,
//! `bias' = w·bias`); §21.4 output stage.
//!
//! [LEGAL] (ES §45.3): adaptation runs AFTER synthesis at the boundary —
//! decontamination — never as an in-loop per-patch gain/bias match (the
//! gated formulation of the register's transform-tuple family,
//! P-8861868); nothing in this module is reachable from the candidate
//! search. Foreground/colour separation ideas follow the matting module's
//! ml-estimation lineage only in spirit; the fit here is the §45.3
//! ring-statistics gain/bias, nothing else.
//!
//! Determinism: ring/fill statistics accumulate in f64 raster order;
//! cellwise fits iterate a fixed cell grid; smoothstep feather is a pure
//! per-pixel function of the BFS hole depth.

use crate::exemplar::diffuse::hole_depth;
use crate::plane::Plane;

/// Adaptation mode + strength (strength `w` per the §45.3 CAM mapping;
/// the plain fill uses `w = 1`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColorAdaptation {
    /// No adaptation (also forced by the `draft` preset, §21.4).
    None,
    /// §45.3 `default`: one global gain/bias fit from the 3-px boundary
    /// ring vs the fill statistics.
    Global { strength: f32 },
    /// §45.3 `high`: per-64×64-px-cell fits, bilinearly interpolated
    /// between cell centers.
    Cellwise { strength: f32 },
}

/// Smoothstep feather coverage over the 2 px inside ∂H (§45.3): depth-1
/// and depth-2 hole pixels ramp (`smoothstep(d/3)`), deeper pixels are 1.
pub(crate) fn feather_coverage(hole: &[bool], w: usize, h: usize) -> Vec<f32> {
    let depth = hole_depth(hole, w, h);
    hole.iter()
        .zip(depth.iter())
        .map(|(&in_h, &d)| {
            if !in_h {
                0.0
            } else if d == u32::MAX || d >= 3 {
                1.0
            } else {
                let t = d as f32 / 3.0;
                t * t * (3.0 - 2.0 * t)
            }
        })
        .collect()
}

struct Fit {
    gain: f64,
    bias: f64,
}

/// Per-channel gain/bias from (fill mean/σ, ring mean/σ) with the §45.3
/// degenerate guard (`σ_fill < 1e−6 ⇒ gain = 1`).
fn fit_from_stats(mu_f: f64, sd_f: f64, mu_r: f64, sd_r: f64) -> Fit {
    let gain = if sd_f < 1e-6 { 1.0 } else { sd_r / sd_f };
    Fit {
        gain,
        bias: mu_r - gain * mu_f,
    }
}

fn stats<I: Iterator<Item = f64>>(it: I) -> (f64, f64, usize) {
    let mut n = 0usize;
    let mut sum = 0f64;
    let mut sq = 0f64;
    for v in it {
        n += 1;
        sum += v;
        sq += v * v;
    }
    if n == 0 {
        return (0.0, 0.0, 0);
    }
    let mean = sum / n as f64;
    let var = (sq / n as f64 - mean * mean).max(0.0);
    (mean, var.sqrt(), n)
}

/// Apply the §45.3 output stage in place: color adaptation (under the
/// feather mask) followed by the 2-px feather composite against the
/// original content. `original` is the unfilled finest-level image.
pub(crate) fn apply_output(
    planes: &mut [Plane],
    original: &[Plane],
    hole: &[bool],
    w: usize,
    h: usize,
    adaptation: ColorAdaptation,
) {
    let coverage = feather_coverage(hole, w, h);
    // Boundary ring: known pixels within 3 px of the hole (§45.3 "3-px
    // boundary ring statistics"). Ring depth = BFS distance from the hole
    // into the known region.
    let inverted: Vec<bool> = hole.iter().map(|&b| !b).collect();
    let ring_depth = hole_depth(&inverted, w, h);
    let ring: Vec<usize> = (0..w * h)
        .filter(|&i| !hole[i] && ring_depth[i] >= 1 && ring_depth[i] <= 3)
        .collect();
    let fill_px: Vec<usize> = (0..w * h).filter(|&i| hole[i]).collect();

    for (c, plane) in planes.iter_mut().enumerate() {
        let orig = &original[c];
        match adaptation {
            ColorAdaptation::None => {}
            ColorAdaptation::Global { strength } => {
                let (mu_r, sd_r, nr) = stats(ring.iter().map(|&i| orig.data[i] as f64));
                let (mu_f, sd_f, nf) = stats(fill_px.iter().map(|&i| plane.data[i] as f64));
                if nr > 0 && nf > 0 {
                    let f = fit_from_stats(mu_f, sd_f, mu_r, sd_r);
                    apply_fit_uniform(plane, &fill_px, &f, strength as f64);
                }
            }
            ColorAdaptation::Cellwise { strength } => {
                apply_cellwise(plane, orig, hole, &ring, w, h, strength as f64);
            }
        }
        // Feather composite (the §16.2 selection-coverage lerp is the
        // evaluator's job and happens outside this crate).
        for &i in &fill_px {
            let cv = coverage[i];
            plane.data[i] = orig.data[i] + (plane.data[i] - orig.data[i]) * cv;
        }
    }
}

fn apply_fit_uniform(plane: &mut Plane, fill_px: &[usize], f: &Fit, w_str: f64) {
    let gain = 1.0 + w_str * (f.gain - 1.0);
    let bias = w_str * f.bias;
    for &i in fill_px {
        plane.data[i] = (gain * plane.data[i] as f64 + bias) as f32;
    }
}

/// §45.3 `high`: 64×64 cells over the hole bounding box; each cell fits
/// against its nearest boundary-ring pixels (all ring pixels within
/// Chebyshev 96 px of the cell center; global fallback when fewer than 8)
/// and (gain, bias) interpolate bilinearly between cell centers.
fn apply_cellwise(
    plane: &mut Plane,
    orig: &Plane,
    hole: &[bool],
    ring: &[usize],
    w: usize,
    h: usize,
    w_str: f64,
) {
    const CELL: usize = 64;
    const NEAR: isize = 96;
    // Hole bbox.
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
    for y in 0..h {
        for x in 0..w {
            if hole[y * w + x] {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    if x0 == usize::MAX {
        return;
    }
    let cols = (x1 - x0) / CELL + 1;
    let rows = (y1 - y0) / CELL + 1;
    // Global fallback fit.
    let (gmu_r, gsd_r, gnr) = stats(ring.iter().map(|&i| orig.data[i] as f64));
    let (gmu_f, gsd_f, gnf) = stats(
        (0..w * h)
            .filter(|&i| hole[i])
            .map(|i| plane.data[i] as f64),
    );
    if gnr == 0 || gnf == 0 {
        return;
    }
    let global = fit_from_stats(gmu_f, gsd_f, gmu_r, gsd_r);
    let mut fits: Vec<Fit> = Vec::with_capacity(rows * cols);
    for cr in 0..rows {
        for cc in 0..cols {
            let ccx = (x0 + cc * CELL + CELL / 2).min(w - 1) as isize;
            let ccy = (y0 + cr * CELL + CELL / 2).min(h - 1) as isize;
            let near_ring = ring.iter().copied().filter(|&i| {
                let (x, y) = ((i % w) as isize, (i / w) as isize);
                (x - ccx).abs() <= NEAR && (y - ccy).abs() <= NEAR
            });
            let (mu_r, sd_r, nr) = stats(near_ring.map(|i| orig.data[i] as f64));
            let cell_fill = (0..w * h).filter(|&i| {
                if !hole[i] {
                    return false;
                }
                let (x, y) = (i % w, i / w);
                x >= x0 + cc * CELL
                    && x < x0 + (cc + 1) * CELL
                    && y >= y0 + cr * CELL
                    && y < y0 + (cr + 1) * CELL
            });
            let (mu_f, sd_f, nf) = stats(cell_fill.map(|i| plane.data[i] as f64));
            fits.push(if nr >= 8 && nf > 0 {
                fit_from_stats(mu_f, sd_f, mu_r, sd_r)
            } else {
                Fit {
                    gain: global.gain,
                    bias: global.bias,
                }
            });
        }
    }
    // Bilinear interpolation between cell centers, clamped at edges.
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if !hole[i] {
                continue;
            }
            let fx = (x as f64 - (x0 + CELL / 2) as f64) / CELL as f64;
            let fy = (y as f64 - (y0 + CELL / 2) as f64) / CELL as f64;
            let cx0 = fx.floor().clamp(0.0, cols as f64 - 1.0) as usize;
            let cy0 = fy.floor().clamp(0.0, rows as f64 - 1.0) as usize;
            let cx1 = (cx0 + 1).min(cols - 1);
            let cy1 = (cy0 + 1).min(rows - 1);
            let tx = (fx - cx0 as f64).clamp(0.0, 1.0);
            let ty = (fy - cy0 as f64).clamp(0.0, 1.0);
            let lerp = |a: f64, b: f64, t: f64| a + (b - a) * t;
            let g = lerp(
                lerp(fits[cy0 * cols + cx0].gain, fits[cy0 * cols + cx1].gain, tx),
                lerp(fits[cy1 * cols + cx0].gain, fits[cy1 * cols + cx1].gain, tx),
                ty,
            );
            let b = lerp(
                lerp(fits[cy0 * cols + cx0].bias, fits[cy0 * cols + cx1].bias, tx),
                lerp(fits[cy1 * cols + cx0].bias, fits[cy1 * cols + cx1].bias, tx),
                ty,
            );
            let gain = 1.0 + w_str * (g - 1.0);
            let bias = w_str * b;
            plane.data[i] = (gain * plane.data[i] as f64 + bias) as f32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feather_ramps_over_two_pixels() {
        let (w, h) = (12, 12);
        let mut hole = vec![false; w * h];
        for y in 2..10 {
            for x in 2..10 {
                hole[y * w + x] = true;
            }
        }
        let cov = feather_coverage(&hole, w, h);
        assert_eq!(cov[w + 1], 0.0); // outside
        let c1 = cov[2 * w + 5]; // depth 1
        let c2 = cov[3 * w + 5]; // depth 2
        let c3 = cov[4 * w + 5]; // depth 3
        assert!(c1 > 0.0 && c1 < c2 && c2 < 1.0);
        assert_eq!(c3, 1.0);
    }

    #[test]
    fn global_fit_matches_ring_statistics() {
        // Fill mean 0.2 / ring mean 0.6: full-strength adaptation must
        // move interior fill pixels to the ring mean (σ_fill ≈ 0 ⇒
        // gain = 1, pure bias).
        let (w, h) = (16, 16);
        let mut hole = vec![false; w * h];
        for y in 5..11 {
            for x in 5..11 {
                hole[y * w + x] = true;
            }
        }
        let original = vec![Plane::from_vec(w, h, vec![0.6; w * h])];
        let mut planes = original.clone();
        for (i, &in_h) in hole.iter().enumerate() {
            if in_h {
                planes[0].data[i] = 0.2;
            }
        }
        apply_output(
            &mut planes,
            &original,
            &hole,
            w,
            h,
            ColorAdaptation::Global { strength: 1.0 },
        );
        // Deep interior (coverage 1, bias fully applied).
        let center = planes[0].data[8 * w + 8];
        assert!((center - 0.6).abs() < 1e-5, "center = {center}");
    }
}
