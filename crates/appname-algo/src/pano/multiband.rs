// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Burt–Adelson multiband (multiresolution-spline) blending — own
//! implementation from burt83 ("A Multiresolution Spline with Application
//! to Image Mosaics", ACM TOG 2(4), 1983).
//!
//! Rights route: the GPL blending tool covering this method
//! (INTEGRATION-NOTES §5.5's none-fetched area) was NEVER fetched or
//! consulted — no memo exists for it and none was needed; this file works
//! from the paper alone. Contract: `pano_align_stack_flow`
//! `compositing.blend: burt_adelson_multiband` (L0 era method).
//!
//! Method (paper §3.2 "image splining"): build Laplacian pyramids of the
//! source images and a Gaussian pyramid of the (seam) mask; blend each
//! Laplacian level under the same-level mask weights; collapse. Low
//! frequencies blend over wide transition zones, high frequencies over
//! narrow ones — the seam disappears without ghosting.
//!
//! Pyramid substrate: reuses the crate's own [`crate::fusion`] pyramid
//! helpers (`gaussian_pyramid`, `laplacian_pyramid`,
//! `reconstruct_laplacian_pyramid`, `pyramid_levels`) — they are already
//! `pub(crate)` from the item-10 edge-aware reuse; no visibility change
//! was needed. Their BSD-2-Clause port provenance and notice (see
//! `fusion.rs` header) cover them wherever in the crate they are called
//! from. The blending logic in THIS file is not part of that port — it is
//! own work from burt83 (the exposure-fusion reference blends N inputs
//! under computed quality weights; this engine blends under a caller
//! seam mask).
//!
//! Determinism (D-48): pure function, single-threaded fixed order, no
//! RNG.
//!
//! ADAPTER SEAM: the CP-1.0 Photomerge flow computes seam masks with the
//! item-8 min-cut engine ([`crate::graphcut`]) over overlap cost graphs
//! (that construction is integration work, recorded as a remainder),
//! then hands each channel plane pair + mask here.

use crate::fusion::{
    gaussian_pyramid, laplacian_pyramid, pyramid_levels, reconstruct_laplacian_pyramid,
};
use crate::plane::Plane;

/// Blend `a` over `b` under `mask` (1 = take `a`, 0 = take `b`) with
/// Laplacian-pyramid splining over `levels` pyramid levels.
///
/// `levels = None` uses the full natural depth
/// (`pyramid_levels(w, h)`); an explicit value is clamped to that depth
/// and must be >= 1. All three planes must share dimensions; mask values
/// are expected in [0, 1] (validated to a small slack).
pub fn multiband_blend(
    a: &Plane,
    b: &Plane,
    mask: &Plane,
    levels: Option<usize>,
) -> Result<Plane, String> {
    if a.w != b.w || a.h != b.h || a.w != mask.w || a.h != mask.h {
        return Err(format!(
            "multiband_blend: dimension mismatch a {}x{} b {}x{} mask {}x{}",
            a.w, a.h, b.w, b.h, mask.w, mask.h
        ));
    }
    if a.w < 2 || a.h < 2 {
        return Err(format!("multiband_blend: image {}x{} below 2x2", a.w, a.h));
    }
    for &m in &mask.data {
        if !(-1e-4..=1.0 + 1e-4).contains(&(m as f64)) {
            return Err(format!("multiband_blend: mask value {m} outside [0, 1]"));
        }
    }
    let natural = pyramid_levels(a.w, a.h);
    let nlev = match levels {
        None => natural,
        Some(0) => return Err("multiband_blend: levels must be >= 1".into()),
        Some(l) => l.min(natural),
    };
    let pa = laplacian_pyramid(a, nlev);
    let pb = laplacian_pyramid(b, nlev);
    let pm = gaussian_pyramid(mask, nlev);
    let mut blended: Vec<Plane> = Vec::with_capacity(nlev);
    for l in 0..nlev {
        let mut lev = Plane::new(pa[l].w, pa[l].h);
        for i in 0..lev.data.len() {
            let m = pm[l].data[i] as f64;
            lev.data[i] = (m * pa[l].data[i] as f64 + (1.0 - m) * pb[l].data[i] as f64) as f32;
        }
        blended.push(lev);
    }
    Ok(reconstruct_laplacian_pyramid(&blended))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plane::{plane_hash, SplitMix64};

    /// Smooth deterministic scene (same construction family as the fusion
    /// test fixtures).
    fn scene(w: usize, h: usize, seed: u64) -> Plane {
        let mut rng = SplitMix64(seed);
        let (p1, p2) = (rng.next_f64(), rng.next_f64());
        let mut p = Plane::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let fx = x as f64 / w as f64 - 0.5;
                let fy = y as f64 / h as f64 - 0.5;
                let v = 0.5 + 0.35 * ((7.0 * fx + p1).sin() * (4.0 * fy - p2).cos())
                    - 0.2 * (fx * fx + fy * fy);
                p.data[y * w + x] = v.clamp(0.0, 1.0) as f32;
            }
        }
        p
    }

    fn hard_vertical_mask(w: usize, h: usize, split: usize) -> Plane {
        let mut m = Plane::new(w, h);
        for y in 0..h {
            for x in 0..w {
                m.data[y * w + x] = if x < split { 1.0 } else { 0.0 };
            }
        }
        m
    }

    #[test]
    fn blending_identical_images_returns_the_input() {
        // With a == b every level blends to the common Laplacian pyramid,
        // so the result is the pyramid round trip of the input (bound
        // 1e-5, the fusion substrate's demonstrated round-trip accuracy).
        let img = scene(48, 33, 5);
        let mask = hard_vertical_mask(48, 33, 20);
        let out = multiband_blend(&img, &img, &mask, None).unwrap();
        for i in 0..out.data.len() {
            assert!(
                (out.data[i] - img.data[i]).abs() < 1e-5,
                "pixel {i}: {} vs {}",
                out.data[i],
                img.data[i]
            );
        }
    }

    #[test]
    fn hard_seam_between_flat_colors_becomes_smooth_monotone_transition() {
        // Two flat fields 0.2 / 0.8 with a hard vertical seam, 4 levels
        // (at the natural depth 5 the widest band reaches the borders of
        // this 64-px fixture — measured endpoints 0.794/0.222 — which is
        // the paper's intended wide-low-frequency behavior, not a bug;
        // 4 levels keeps the band interior so endpoints are testable).
        // Oracle bundle:
        //  - no ringing: output stays within [0.2 − eps, 0.8 + eps];
        //  - the transition is monotone non-increasing along x to a small
        //    tolerance (the 0.8 field is on the left);
        //  - seam-gradient reduction: the max |d/dx| of the blend
        //    (measured 0.035/px) is below the hard-cut step (0.6/px) by
        //    the stated factor 8× (asserted < 0.075 with 2× margin).
        let (w, h) = (64, 32);
        let a = Plane::from_vec(w, h, vec![0.8; w * h]);
        let b = Plane::from_vec(w, h, vec![0.2; w * h]);
        let mask = hard_vertical_mask(w, h, 32);
        let out = multiband_blend(&a, &b, &mask, Some(4)).unwrap();
        let eps = 0.02;
        for &v in &out.data {
            assert!(
                (0.2 - eps..=0.8 + eps).contains(&(v as f64)),
                "ringing: value {v}"
            );
        }
        let row = h / 2;
        let mut max_step = 0.0f64;
        for x in 1..w {
            let step = out.at(x, row) as f64 - out.at(x - 1, row) as f64;
            assert!(step <= 1e-3, "non-monotone at x={x}: step {step}");
            max_step = max_step.max(-step);
        }
        assert!(
            max_step < 0.6 / 8.0,
            "seam gradient {max_step}/px not below the hard cut 0.6 by 8x"
        );
        // Endpoints hold the source values.
        assert!((out.at(1, row) - 0.8).abs() < 0.01);
        assert!((out.at(w - 2, row) - 0.2).abs() < 0.01);
    }

    #[test]
    fn mask_one_region_preserves_image_a_within_bound() {
        // Away from the seam (here: >= 16 px, half the 64-px width at the
        // deepest mask-pyramid support), the mask's Gaussian pyramid stays
        // ~1 and A must come through within a small bound. Bound 5e-3:
        // the residual is the far tail of the widest transition band,
        // measured ~2e-3 at the probe columns; exact equality is NOT
        // claimed (the paper's wide low-frequency bands genuinely reach
        // far — the honest contract is "visually exact", pinned here at
        // 5e-3 of a 0.6 step).
        let (w, h) = (64, 32);
        let a = scene(w, h, 21);
        let b = scene(w, h, 99);
        let mask = hard_vertical_mask(w, h, 32);
        let out = multiband_blend(&a, &b, &mask, None).unwrap();
        for y in 0..h {
            for x in 0..8 {
                let d = (out.at(x, y) - a.at(x, y)).abs();
                assert!(d < 5e-3, "A-side pixel ({x},{y}) deviates {d}");
            }
        }
    }

    #[test]
    fn input_validation_and_level_clamp() {
        let a = scene(16, 16, 1);
        let b = scene(16, 16, 2);
        let m = hard_vertical_mask(16, 16, 8);
        assert!(multiband_blend(&a, &scene(8, 8, 2), &m, None).is_err());
        assert!(multiband_blend(&a, &b, &m, Some(0)).is_err());
        let mut bad = m.clone();
        bad.data[0] = 2.0;
        assert!(multiband_blend(&a, &b, &bad, None).is_err());
        // An over-deep request clamps to the natural depth and still
        // reconstructs.
        let out = multiband_blend(&a, &a, &m, Some(99)).unwrap();
        for i in 0..out.data.len() {
            assert!((out.data[i] - a.data[i]).abs() < 1e-5);
        }
    }

    #[test]
    fn determinism_two_run_and_golden() {
        let (w, h) = (56, 40);
        let a = scene(w, h, 31);
        let b = scene(w, h, 77);
        let mask = hard_vertical_mask(w, h, 23);
        let o1 = multiband_blend(&a, &b, &mask, None).unwrap();
        let o2 = multiband_blend(&a, &b, &mask, None).unwrap();
        let (h1, h2) = (plane_hash(&o1.data), plane_hash(&o2.data));
        assert_eq!(h1, h2, "blend must replay bit-identically");
        assert_eq!(
            h1, GOLDEN,
            "golden hash drift — investigate before re-pinning (O-QA rule)"
        );
    }

    // Self-pinned golden (see determinism_two_run_and_golden): pinned
    // 2026-08-13 from the first run whose behavioral oracles (identity
    // blend, seam smoothing, A-side preservation) were all green.
    // Re-pinning requires investigation + a fresh oracle pass.
    const GOLDEN: u64 = 16613958203145254358;
}
