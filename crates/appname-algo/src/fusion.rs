// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Ported from the authors' reference implementation of Exposure Fusion
// (https://github.com/Mericam/exposure-fusion, commit 03e24695607550dce1…,
// vendored at appname-proto/vendor/exposure-fusion/): exposure_fusion.m,
// gaussian_pyramid.m, laplacian_pyramid.m, reconstruct_laplacian_pyramid.m,
// pyramid_filter.m, downsample.m, upsample.m.
// BSD-2-Clause, Copyright (c) 2015, Tom Mertens. License text carried in
// THIRD-PARTY-NOTICES.md and vendor/exposure-fusion/LICENSE.

//! Spec: ES §24.3 / §17.2 HDR rows; ALGORITHMS.md §16 `hdr_tonemap` primary
//! `exposure_fusion_mertens` (level L2, basis mertens07); INTEGRATION-NOTES
//! §6 order 2 (`exposure-fusion` port + goldens).
//!
//! Mertens–Kautz–Van Reeth exposure fusion: per-pixel quality measures
//! (contrast, saturation, well-exposedness) combined into normalized weight
//! maps, blended across a Laplacian pyramid of the inputs against Gaussian
//! pyramids of the weights.
//!
//! The design-around note from the register rides along: the engine keeps
//! "independent wgpu fusion kernels on tile substrate" (contract
//! `design_around` memo for US9740959B2) — this CPU port is the reference /
//! golden oracle; any GPU pair lands via O-RENDER's parity harness.
//!
//! ADAPTER SEAM (brokered): the CP-1.0 HDR develop stage converts the
//! bracketed stack into `[Plane; 3]` per exposure (linear working space,
//! values nominally [0, 1]), calls [`exposure_fusion`], and lands the result
//! through the develop-stage command envelope (D-48).

use crate::plane::Plane;

/// The 1-D 5-tap pyramid filter from `pyramid_filter.m`.
const PYRAMID_FILTER: [f64; 5] = [0.0625, 0.25, 0.375, 0.25, 0.0625];

/// Measure exponents (`m` in `exposure_fusion.m`): a measure with exponent 0
/// is skipped exactly as upstream.
#[derive(Clone, Copy, Debug)]
pub struct FusionWeights {
    /// Contrast exponent (Laplacian-response magnitude on the gray image).
    pub contrast: f64,
    /// Saturation exponent (per-pixel std-dev of R,G,B).
    pub saturation: f64,
    /// Well-exposedness exponent (Gaussian around 0.5, sigma 0.2).
    pub well_exposedness: f64,
}

impl Default for FusionWeights {
    fn default() -> Self {
        FusionWeights {
            contrast: 1.0,
            saturation: 1.0,
            well_exposedness: 1.0,
        }
    }
}

/// Border handling for the separable pyramid convolutions.
#[derive(Clone, Copy, PartialEq)]
enum Border {
    /// MATLAB `imfilter(..., 'symmetric')` (half-sample mirror), used by
    /// `downsample.m`.
    Symmetric,
    /// MATLAB `imfilter` default (zero padding), used by `upsample.m`.
    Zero,
}

#[inline]
fn border_index(i: isize, n: usize, border: Border) -> Option<usize> {
    if i >= 0 && (i as usize) < n {
        return Some(i as usize);
    }
    match border {
        Border::Zero => None,
        Border::Symmetric => {
            // Half-sample symmetric mirror: -1 -> 0, -2 -> 1, n -> n-1, ...
            let m = 2 * n as isize;
            let mut k = i.rem_euclid(m);
            if k >= n as isize {
                k = m - 1 - k;
            }
            Some(k as usize)
        }
    }
}

/// Separable 5-tap convolution (horizontal then vertical), f64 accumulation.
fn conv_separable(src: &Plane, border: Border) -> Plane {
    let (w, h) = (src.w, src.h);
    let mut tmp = vec![0f64; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut s = 0.0;
            for (t, &f) in PYRAMID_FILTER.iter().enumerate() {
                let xi = x as isize + t as isize - 2;
                if let Some(xs) = border_index(xi, w, border) {
                    s += f * src.at(xs, y) as f64;
                }
            }
            tmp[y * w + x] = s;
        }
    }
    let mut out = Plane::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let mut s = 0.0;
            for (t, &f) in PYRAMID_FILTER.iter().enumerate() {
                let yi = y as isize + t as isize - 2;
                if let Some(ys) = border_index(yi, h, border) {
                    s += f * tmp[ys * w + x];
                }
            }
            out.data[y * w + x] = s as f32;
        }
    }
    out
}

/// `downsample.m`: symmetric-border low-pass then decimate by 2 (keeping
/// even indices; odd sizes keep the last sample).
fn downsample(src: &Plane) -> Plane {
    let lp = conv_separable(src, Border::Symmetric);
    let dw = src.w.div_ceil(2);
    let dh = src.h.div_ceil(2);
    let mut out = Plane::new(dw, dh);
    for y in 0..dh {
        for x in 0..dw {
            out.data[y * dw + x] = lp.at(x * 2, y * 2);
        }
    }
    out
}

/// `upsample.m`: replicate-pad by 1, scatter `4 * I` on the even grid of the
/// doubled image, zero-border low-pass, crop back to `(target_w, target_h)`.
fn upsample(src: &Plane, target_w: usize, target_h: usize) -> Plane {
    let (w, h) = (src.w, src.h);
    let (pw, ph) = (w + 2, h + 2);
    let (rw, rh) = (2 * pw, 2 * ph);
    let mut big = Plane::new(rw, rh);
    for y in 0..ph {
        for x in 0..pw {
            let v = src.at_clamped(x as isize - 1, y as isize - 1);
            big.data[(2 * y) * rw + 2 * x] = 4.0 * v;
        }
    }
    let sm = conv_separable(&big, Border::Zero);
    // MATLAB crop R(3 : r-2-odd(1), 3 : c-2-odd(2)) with odd = 2*size(src)
    // - target: 0-based start row/col 2, extent = target dims.
    debug_assert!(target_h >= 2 * h - 1 && target_h <= 2 * h);
    debug_assert!(target_w >= 2 * w - 1 && target_w <= 2 * w);
    let mut out = Plane::new(target_w, target_h);
    for y in 0..target_h {
        for x in 0..target_w {
            out.data[y * target_w + x] = sm.at(x + 2, y + 2);
        }
    }
    out
}

/// Number of pyramid levels: `floor(log(min(r, c)) / log(2))` per
/// `gaussian_pyramid.m` (natural log ratio == log2).
///
/// `pub(crate)`: the pyramid machinery below is also the substrate for the
/// [`crate::edge_aware`] local-Laplacian engines and the
/// [`crate::pano::multiband`] seam blend (crate-internal reuse; the
/// BSD-2-Clause port provenance and notice above cover these functions
/// wherever in the crate they are called from).
pub(crate) fn pyramid_levels(w: usize, h: usize) -> usize {
    let m = w.min(h) as f64;
    (m.ln() / 2f64.ln()).floor().max(1.0) as usize
}

/// Gaussian pyramid (finest first), `nlev` levels.
pub(crate) fn gaussian_pyramid(img: &Plane, nlev: usize) -> Vec<Plane> {
    let mut pyr = Vec::with_capacity(nlev);
    pyr.push(img.clone());
    for _ in 1..nlev {
        let next = downsample(pyr.last().unwrap());
        pyr.push(next);
    }
    pyr
}

/// Laplacian pyramid (finest first), `nlev` levels; coarsest level holds the
/// low-pass residual.
pub(crate) fn laplacian_pyramid(img: &Plane, nlev: usize) -> Vec<Plane> {
    let mut pyr = Vec::with_capacity(nlev);
    let mut j = img.clone();
    for _ in 0..nlev - 1 {
        let down = downsample(&j);
        let up = upsample(&down, j.w, j.h);
        let mut lev = Plane::new(j.w, j.h);
        for i in 0..lev.data.len() {
            lev.data[i] = j.data[i] - up.data[i];
        }
        pyr.push(lev);
        j = down;
    }
    pyr.push(j);
    pyr
}

/// Collapse a Laplacian pyramid back to the full-resolution image.
pub(crate) fn reconstruct_laplacian_pyramid(pyr: &[Plane]) -> Plane {
    let mut r = pyr.last().unwrap().clone();
    for lev in pyr.iter().rev().skip(1) {
        let up = upsample(&r, lev.w, lev.h);
        let mut next = lev.clone();
        for i in 0..next.data.len() {
            next.data[i] += up.data[i];
        }
        r = next;
    }
    r
}

/// MATLAB `rgb2gray` luma weights (ITU-R BT.601 as MATLAB evaluates them).
fn gray(rgb: &[Plane; 3]) -> Plane {
    let mut g = Plane::new(rgb[0].w, rgb[0].h);
    for i in 0..g.data.len() {
        g.data[i] = (0.298936021293775 * rgb[0].data[i] as f64
            + 0.587043074451121 * rgb[1].data[i] as f64
            + 0.114020904255103 * rgb[2].data[i] as f64) as f32;
    }
    g
}

/// Contrast measure: |Laplacian| of the gray image, replicate border
/// (`contrast()` in `exposure_fusion.m`).
fn contrast_measure(rgb: &[Plane; 3]) -> Plane {
    let g = gray(rgb);
    let (w, h) = (g.w, g.h);
    let mut c = Plane::new(w, h);
    for y in 0..h as isize {
        for x in 0..w as isize {
            let lap = g.at_clamped(x - 1, y) as f64
                + g.at_clamped(x + 1, y) as f64
                + g.at_clamped(x, y - 1) as f64
                + g.at_clamped(x, y + 1) as f64
                - 4.0 * g.at_clamped(x, y) as f64;
            c.data[y as usize * w + x as usize] = lap.abs() as f32;
        }
    }
    c
}

/// Saturation measure: per-pixel std-dev of the three channels.
fn saturation_measure(rgb: &[Plane; 3]) -> Plane {
    let mut s = Plane::new(rgb[0].w, rgb[0].h);
    for i in 0..s.data.len() {
        let r = rgb[0].data[i] as f64;
        let g = rgb[1].data[i] as f64;
        let b = rgb[2].data[i] as f64;
        let mu = (r + g + b) / 3.0;
        s.data[i] = (((r - mu).powi(2) + (g - mu).powi(2) + (b - mu).powi(2)) / 3.0).sqrt() as f32;
    }
    s
}

/// Well-exposedness: product over channels of a Gaussian around 0.5
/// (sigma = 0.2).
fn well_exposedness_measure(rgb: &[Plane; 3]) -> Plane {
    let sig2 = 0.2f64 * 0.2;
    let mut e = Plane::new(rgb[0].w, rgb[0].h);
    for i in 0..e.data.len() {
        let mut p = 1.0f64;
        for plane in rgb {
            let v = plane.data[i] as f64 - 0.5;
            p *= (-0.5 * v * v / sig2).exp();
        }
        e.data[i] = p as f32;
    }
    e
}

/// Exposure fusion (port of `exposure_fusion.m`).
///
/// `stack` holds N exposures as RGB plane triples (same dimensions, values
/// nominally in [0, 1]). Returns the fused RGB planes.
pub fn exposure_fusion(stack: &[[Plane; 3]], m: FusionWeights) -> [Plane; 3] {
    assert!(!stack.is_empty(), "exposure_fusion: empty stack");
    let w = stack[0][0].w;
    let h = stack[0][0].h;
    for exp in stack {
        for p in exp {
            assert!(p.w == w && p.h == h, "exposure_fusion: dimension mismatch");
        }
    }
    let n = stack.len();

    // Weight maps.
    let mut weights: Vec<Plane> = Vec::with_capacity(n);
    for exp in stack {
        let mut wmap = Plane::from_vec(w, h, vec![1.0; w * h]);
        if m.contrast > 0.0 {
            let c = contrast_measure(exp);
            for i in 0..wmap.data.len() {
                wmap.data[i] = ((wmap.data[i] as f64) * (c.data[i] as f64).powf(m.contrast)) as f32;
            }
        }
        if m.saturation > 0.0 {
            let s = saturation_measure(exp);
            for i in 0..wmap.data.len() {
                wmap.data[i] =
                    ((wmap.data[i] as f64) * (s.data[i] as f64).powf(m.saturation)) as f32;
            }
        }
        if m.well_exposedness > 0.0 {
            let e = well_exposedness_measure(exp);
            for i in 0..wmap.data.len() {
                wmap.data[i] =
                    ((wmap.data[i] as f64) * (e.data[i] as f64).powf(m.well_exposedness)) as f32;
            }
        }
        weights.push(wmap);
    }
    // Normalize (+1e-12 exactly as upstream to avoid division by zero).
    for i in 0..w * h {
        let mut sum = 0.0f64;
        for wmap in &weights {
            sum += wmap.data[i] as f64 + 1e-12;
        }
        for wmap in weights.iter_mut() {
            wmap.data[i] = (((wmap.data[i] as f64) + 1e-12) / sum) as f32;
        }
    }

    // Multiresolution blend.
    let nlev = pyramid_levels(w, h);
    let mut acc: Vec<[Plane; 3]> = gaussian_pyramid(&Plane::new(w, h), nlev)
        .into_iter()
        .map(|z| [z.clone(), z.clone(), z])
        .collect();
    for (exp, wmap) in stack.iter().zip(&weights) {
        let pyr_w = gaussian_pyramid(wmap, nlev);
        for c in 0..3 {
            let pyr_i = laplacian_pyramid(&exp[c], nlev);
            for l in 0..nlev {
                for i in 0..acc[l][c].data.len() {
                    acc[l][c].data[i] += pyr_w[l].data[i] * pyr_i[l].data[i];
                }
            }
        }
    }
    let collapse = |c: usize| -> Plane {
        let pyr: Vec<Plane> = acc.iter().map(|lev| lev[c].clone()).collect();
        reconstruct_laplacian_pyramid(&pyr)
    };
    [collapse(0), collapse(1), collapse(2)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plane::{plane_hash, SplitMix64};

    fn synthetic_scene(w: usize, h: usize, seed: u64) -> [Plane; 3] {
        // Smooth deterministic scene: radial gradient + per-channel phase.
        let mut rng = SplitMix64(seed);
        let ph: Vec<f64> = (0..3).map(|_| rng.next_f64()).collect();
        let mut planes = [Plane::new(w, h), Plane::new(w, h), Plane::new(w, h)];
        for (c, plane) in planes.iter_mut().enumerate() {
            for y in 0..h {
                for x in 0..w {
                    let fx = x as f64 / w as f64 - 0.5;
                    let fy = y as f64 / h as f64 - 0.5;
                    let v = 0.5 + 0.4 * ((6.0 * fx + ph[c]).sin() * (5.0 * fy - ph[c]).cos())
                        - 0.3 * (fx * fx + fy * fy);
                    plane.data[y * w + x] = v.clamp(0.0, 1.0) as f32;
                }
            }
        }
        planes
    }

    fn exposed(scene: &[Plane; 3], ev: f64) -> [Plane; 3] {
        let mut out = scene.clone();
        for p in out.iter_mut() {
            for v in p.data.iter_mut() {
                *v = ((*v as f64) * 2f64.powf(ev)).clamp(0.0, 1.0) as f32;
            }
        }
        out
    }

    #[test]
    fn laplacian_pyramid_is_invertible() {
        let scene = synthetic_scene(33, 24, 7);
        let nlev = pyramid_levels(33, 24);
        let pyr = laplacian_pyramid(&scene[0], nlev);
        let rec = reconstruct_laplacian_pyramid(&pyr);
        for i in 0..rec.data.len() {
            assert!(
                (rec.data[i] - scene[0].data[i]).abs() < 1e-5,
                "pixel {i}: {} vs {}",
                rec.data[i],
                scene[0].data[i]
            );
        }
    }

    #[test]
    fn fusing_identical_stack_returns_the_image() {
        // Equal inputs get weight 1/N everywhere; the blend degenerates to
        // the image's own Laplacian pyramid, so reconstruction returns it.
        let scene = synthetic_scene(32, 21, 11);
        let stack = vec![scene.clone(), scene.clone(), scene.clone()];
        let fused = exposure_fusion(&stack, FusionWeights::default());
        for c in 0..3 {
            for i in 0..fused[c].data.len() {
                assert!(
                    (fused[c].data[i] - scene[c].data[i]).abs() < 1e-4,
                    "c{c} pixel {i}"
                );
            }
        }
    }

    #[test]
    fn well_exposedness_prefers_the_mid_exposure() {
        // Constant gray stack at 0.1 / 0.5 / 0.9 with only the
        // well-exposedness measure: fusion must land near 0.5 (the weight
        // ratio is exp(0) vs exp(-2) per channel).
        let mk = |v: f32| {
            let p = Plane::from_vec(16, 16, vec![v; 256]);
            [p.clone(), p.clone(), p]
        };
        let stack = vec![mk(0.1), mk(0.5), mk(0.9)];
        let m = FusionWeights {
            contrast: 0.0,
            saturation: 0.0,
            well_exposedness: 1.0,
        };
        let fused = exposure_fusion(&stack, m);
        let v = fused[0].data[128];
        assert!((v - 0.5).abs() < 0.05, "fused constant = {v}");
    }

    #[test]
    fn bracketed_stack_golden_and_determinism() {
        // Fixed-seed golden: the bracketed fusion output is pinned by hash
        // (self-pinned oracle from the first verified run of this port;
        // any bit change in the pipeline trips this).
        let scene = synthetic_scene(40, 28, 42);
        let stack = vec![
            exposed(&scene, -2.0),
            exposed(&scene, 0.0),
            exposed(&scene, 2.0),
        ];
        let fused1 = exposure_fusion(&stack, FusionWeights::default());
        let fused2 = exposure_fusion(&stack, FusionWeights::default());
        let h1: Vec<u64> = fused1.iter().map(|p| plane_hash(&p.data)).collect();
        let h2: Vec<u64> = fused2.iter().map(|p| plane_hash(&p.data)).collect();
        assert_eq!(h1, h2, "fusion must be bit-exact across runs");
        assert_eq!(
            h1,
            vec![GOLDEN_R, GOLDEN_G, GOLDEN_B],
            "golden hash drift — investigate before re-pinning (O-QA rule)"
        );
        // Sanity on values: fused output of a clamped stack stays in a sane
        // range (pyramid blending may ring slightly outside [0,1]).
        for plane in &fused1 {
            for &v in &plane.data {
                assert!(v > -0.2 && v < 1.2, "fused value out of range: {v}");
            }
        }
    }

    // Self-pinned golden hashes (see bracketed_stack_golden_and_determinism):
    // pinned 2026-08-13 from the first run whose behavioral oracles
    // (invertibility, identity fusion, well-exposedness preference) were all
    // green. Re-pinning requires investigation + a fresh oracle pass.
    const GOLDEN_R: u64 = 3070641149237235452;
    const GOLDEN_G: u64 = 367424594395032685;
    const GOLDEN_B: u64 = 8068049078984441518;
}
