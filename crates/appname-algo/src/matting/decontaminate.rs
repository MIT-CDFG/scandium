// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Color decontamination for matted output — new-background compositing
//! prep for the Select & Mask "Decontaminate colors" option.
//!
//! Spec: ES §13.4 Select-and-Mask pipeline stage 8 [NORMATIVE] (verified
//! against CURRENT text, quoted): "alpha-weighted color reassignment for
//! band pixels: B̂(p) = mean of I over the nearest pixels with α < 0.02;
//! output color F̂(p) = (I(p) − (1−α)·B̂(p)) / max(α, 0.05), alpha
//! unchanged". ALGORITHMS.md §7.B sanctions the op: "Decontamination:
//! α-weighted foreground re-estimation per Levin's F,B solve"; the
//! F-side re-estimation here is the crate's existing
//! [`crate::matting::estimate_foreground_ml`] (wave-1 pymatting port —
//! REUSED, not rebuilt), the band defringe is the §13.4 stage-8 formula.
//!
//! Output contract: `(F planes, alpha)` ready for `layer` composition by
//! the consumer (ES §13.4 output rows `new_layer` /
//! `new_layer_with_mask`: "Because it rewrites colors, decontaminate
//! forces the new-layer output modes … and never touches the source
//! layer"). Alpha passes through UNCHANGED per the spec sentence.
//!
//! Interpretation choices (documented; the spec leaves them open):
//! "nearest pixels with α < 0.02" is realized as a deterministic
//! multi-source BFS color diffusion — background pixels seed their own
//! color at depth 0; each farther pixel takes the MEAN of its already-
//! assigned 4-neighbors one ring closer (fixed row-major order inside a
//! ring, rings processed in depth order), which extends the nearest
//! background colors outward without any nearest-neighbor search
//! nondeterminism. The band is `bg_alpha_max < α < fg_alpha_min`
//! (defaults 0.02 / 0.98) unless the consumer passes its §13.4 stage-2
//! EDT band explicitly. Pixels with α ≥ `fg_alpha_min` keep `F = I`
//! exactly; non-band, non-opaque pixels carry the ml-estimated F.
//!
//! Determinism: pure function, single-threaded, fixed iteration order;
//! BFS rings are index-ordered, no hashing, no RNG.

use crate::plane::Plane;

use super::estimate_foreground_ml;

/// Parameters of [`decontaminate`] (ES §13.4 stage-8 constants as
/// defaults).
#[derive(Clone, Debug)]
pub struct DecontaminateParams {
    /// Background classification threshold (spec 0.02): pixels with
    /// `α < bg_alpha_max` seed the B̂ estimate.
    pub bg_alpha_max: f32,
    /// Opaque threshold: pixels with `α >= fg_alpha_min` keep `F = I`
    /// exactly (default 0.98).
    pub fg_alpha_min: f32,
    /// Divisor floor of the unmixing formula (spec 0.05).
    pub alpha_floor: f32,
    /// `estimate_foreground_ml` regularization (wave-1 port default).
    pub ml_regularization: f64,
    /// `estimate_foreground_ml` small-level iterations.
    pub ml_small_iterations: usize,
    /// `estimate_foreground_ml` big-level iterations.
    pub ml_big_iterations: usize,
    /// `estimate_foreground_ml` coarsest-level size.
    pub ml_small_size: usize,
}

impl Default for DecontaminateParams {
    fn default() -> DecontaminateParams {
        DecontaminateParams {
            bg_alpha_max: 0.02,
            fg_alpha_min: 0.98,
            alpha_floor: 0.05,
            ml_regularization: 1e-5,
            ml_small_iterations: 10,
            ml_big_iterations: 2,
            ml_small_size: 32,
        }
    }
}

/// Decontamination result: foreground planes + pass-through alpha.
pub struct Decontaminated {
    /// Estimated foreground color planes (defringed in the band).
    pub fg: [Plane; 3],
    /// Alpha, unchanged (ES §13.4 stage 8: "alpha unchanged").
    pub alpha: Plane,
}

/// Deterministic nearest-background color extension: multi-source BFS
/// from all pixels with `α < bg_alpha_max`; ring d pixels average their
/// ring-(d−1) 4-neighbors' values. Returns `None` when no background
/// seed exists.
fn background_color_extension(
    rgb: &[Plane; 3],
    alpha: &Plane,
    bg_alpha_max: f32,
) -> Option<[Vec<f64>; 3]> {
    let w = alpha.w;
    let h = alpha.h;
    let n = w * h;
    const UNSET: u32 = u32::MAX;
    let mut depth = vec![UNSET; n];
    let mut frontier: Vec<usize> = Vec::new();
    for (i, &a) in alpha.data.iter().enumerate() {
        if a < bg_alpha_max {
            depth[i] = 0;
            frontier.push(i); // row-major by construction
        }
    }
    if frontier.is_empty() {
        return None;
    }
    let mut bhat = [vec![0f64; n], vec![0f64; n], vec![0f64; n]];
    for &i in &frontier {
        for c in 0..3 {
            bhat[c][i] = rgb[c].data[i] as f64;
        }
    }
    let mut d = 1u32;
    while !frontier.is_empty() {
        // Collect the next ring in ascending index order (scan the
        // current ring's neighbors, then sort + dedup — deterministic).
        let mut next: Vec<usize> = Vec::new();
        for &i in &frontier {
            let x = i % w;
            let y = i / w;
            let mut push = |cond: bool, j: usize| {
                if cond && depth[j] == UNSET {
                    depth[j] = d;
                    next.push(j);
                }
            };
            push(x > 0, i.wrapping_sub(1));
            push(x + 1 < w, i + 1);
            push(y > 0, i.wrapping_sub(w));
            push(y + 1 < h, i + w);
        }
        next.sort_unstable();
        for &i in &next {
            let x = i % w;
            let y = i / w;
            let mut acc = [0f64; 3];
            let mut cnt = 0f64;
            let mut take = |cond: bool, j: usize| {
                if cond && depth[j] == d - 1 {
                    for (a, b) in acc.iter_mut().zip(bhat.iter()) {
                        *a += b[j];
                    }
                    cnt += 1.0;
                }
            };
            take(x > 0, i.wrapping_sub(1));
            take(x + 1 < w, i + 1);
            take(y > 0, i.wrapping_sub(w));
            take(y + 1 < h, i + w);
            debug_assert!(cnt > 0.0, "BFS ring pixel without a parent ring neighbor");
            for c in 0..3 {
                bhat[c][i] = acc[c] / cnt;
            }
        }
        frontier = next;
        d += 1;
    }
    Some(bhat)
}

/// Color decontamination (ES §13.4 stage 8 over the wave-1 ml foreground
/// estimate). `band`: optional explicit band mask (> 0.5 = in band, the
/// consumer's §13.4 stage-2 EDT band); when `None` the band is
/// `bg_alpha_max < α < fg_alpha_min`.
pub fn decontaminate(
    rgb: &[Plane; 3],
    alpha: &Plane,
    band: Option<&Plane>,
    params: &DecontaminateParams,
) -> Result<Decontaminated, String> {
    let w = alpha.w;
    let h = alpha.h;
    if w == 0 || h == 0 {
        return Err("decontaminate: empty plane".to_string());
    }
    if rgb[0].w != w || rgb[0].h != h {
        return Err(format!(
            "decontaminate: rgb {}x{} vs alpha {}x{} dimension mismatch",
            rgb[0].w, rgb[0].h, w, h
        ));
    }
    if let Some(b) = band {
        if b.w != w || b.h != h {
            return Err(format!(
                "decontaminate: band {}x{} vs alpha {}x{} dimension mismatch",
                b.w, b.h, w, h
            ));
        }
    }
    if !(params.bg_alpha_max > 0.0 && params.fg_alpha_min < 1.0 + f32::EPSILON) {
        return Err("decontaminate: thresholds out of range".to_string());
    }
    // Base F everywhere: the wave-1 multi-level estimator (REUSE).
    let mut fg = estimate_foreground_ml(
        rgb,
        alpha,
        params.ml_regularization,
        params.ml_small_iterations,
        params.ml_big_iterations,
        params.ml_small_size,
    );
    // Opaque pixels: F = I exactly (unchanged-region contract).
    for i in 0..w * h {
        if alpha.data[i] >= params.fg_alpha_min {
            for c in 0..3 {
                fg[c].data[i] = rgb[c].data[i];
            }
        }
    }
    // Band defringe per the stage-8 formula, when a background estimate
    // exists (an all-foreground alpha has no B̂ source; ml F carries).
    if let Some(bhat) = background_color_extension(rgb, alpha, params.bg_alpha_max) {
        // Index loop kept: i addresses alpha, band, bhat, rgb and fg in
        // lockstep (crate precedent for multi-array index loops).
        #[allow(clippy::needless_range_loop)]
        for i in 0..w * h {
            let a = alpha.data[i];
            let in_band = match band {
                Some(b) => b.data[i] > 0.5,
                None => a > params.bg_alpha_max && a < params.fg_alpha_min,
            };
            if !in_band || a >= params.fg_alpha_min {
                continue;
            }
            let af = a as f64;
            let div = (a.max(params.alpha_floor)) as f64;
            for c in 0..3 {
                let ic = rgb[c].data[i] as f64;
                let f = (ic - (1.0 - af) * bhat[c][i]) / div;
                fg[c].data[i] = f.clamp(0.0, 1.0) as f32;
            }
        }
    }
    Ok(Decontaminated {
        fg,
        alpha: alpha.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matting::fixtures::{two_color_band, FIX_BG, FIX_FG};
    use crate::plane::plane_hash;

    #[test]
    fn halo_over_new_background_is_reduced_vs_naive_alpha_multiply() {
        // Composite over a saturated new background: naive α·I carries the
        // OLD background inside the soft edge (halo); the decontaminated
        // α·F̂ must land far closer to the ideal α·F_true composite.
        // Measured on this fixture: naive mean-halo 0.1354, decontaminated
        // 1.8e-8 (every band α on this fixture is ≥ the 0.05 divisor
        // floor, so the unmixing is exact up to f32 rounding). Asserted:
        // at least 100x reduction — far above the measured margin.
        let (rgb, _, truth) = two_color_band(16, 8);
        let alpha = Plane::from_vec(16, 8, truth.iter().map(|&a| a as f32).collect());
        let d = decontaminate(&rgb, &alpha, None, &DecontaminateParams::default()).unwrap();
        let new_bg = [0.0f64, 0.9, 0.0];
        let mut naive_halo = 0f64;
        let mut decon_halo = 0f64;
        let mut count = 0usize;
        // Index loop kept: the assertions reference several arrays by i.
        #[allow(clippy::needless_range_loop)]
        for i in 0..truth.len() {
            let a = truth[i];
            if a <= 0.0 || a >= 1.0 {
                continue;
            }
            for c in 0..3 {
                let ideal = a * FIX_FG[c] as f64 + (1.0 - a) * new_bg[c];
                let naive = a * rgb[c].data[i] as f64 + (1.0 - a) * new_bg[c];
                let decon = a * d.fg[c].data[i] as f64 + (1.0 - a) * new_bg[c];
                naive_halo += (naive - ideal).abs();
                decon_halo += (decon - ideal).abs();
            }
            count += 3;
        }
        let naive_halo = naive_halo / count as f64;
        let decon_halo = decon_halo / count as f64;
        assert!(
            decon_halo < naive_halo / 100.0,
            "halo not reduced enough: naive {naive_halo} decontaminated {decon_halo}"
        );
    }

    #[test]
    fn exact_alpha_regions_unchanged() {
        let (rgb, _, truth) = two_color_band(16, 8);
        let alpha = Plane::from_vec(16, 8, truth.iter().map(|&a| a as f32).collect());
        let d = decontaminate(&rgb, &alpha, None, &DecontaminateParams::default()).unwrap();
        // Alpha passes through bit-exactly.
        assert_eq!(plane_hash(&d.alpha.data), plane_hash(&alpha.data));
        // α = 1 pixels: F = I exactly. α = 0 pixels: composite over any
        // new background is (1)·bg regardless of F — check F finite/valid.
        // Index loop kept: the assertions reference several arrays by i.
        #[allow(clippy::needless_range_loop)]
        for i in 0..truth.len() {
            if truth[i] >= 1.0 {
                for c in 0..3 {
                    assert_eq!(d.fg[c].data[i], rgb[c].data[i], "opaque pixel {i} changed");
                }
            }
            for c in 0..3 {
                let v = d.fg[c].data[i];
                assert!((0.0..=1.0).contains(&v), "fg[{c}][{i}] = {v}");
            }
        }
    }

    #[test]
    fn band_recovers_generating_foreground_where_alpha_above_floor() {
        // On the two-color fixture the stage-8 formula is exact once
        // α ≥ the 0.05 divisor floor: B̂ = FIX_BG (constant background
        // diffuses its own value), so F̂ = (αF + (1−α)B − (1−α)B)/α = F.
        let (rgb, _, truth) = two_color_band(16, 8);
        let alpha = Plane::from_vec(16, 8, truth.iter().map(|&a| a as f32).collect());
        let d = decontaminate(&rgb, &alpha, None, &DecontaminateParams::default()).unwrap();
        // Index loop kept: the assertions reference several arrays by i.
        #[allow(clippy::needless_range_loop)]
        for i in 0..truth.len() {
            let a = truth[i];
            if a > 0.06 && a < 1.0 {
                for c in 0..3 {
                    let dev = (d.fg[c].data[i] - FIX_FG[c]).abs();
                    assert!(dev < 1e-3, "band pixel {i} chan {c}: F̂ off by {dev}");
                }
            }
        }
    }

    #[test]
    fn no_background_pixels_degrades_to_ml_estimate() {
        // α everywhere ≥ 0.5: no B̂ seed exists; the op must still return
        // (ml F with opaque override), not error.
        let (rgb, _, _) = two_color_band(8, 8);
        let alpha = Plane::from_vec(8, 8, vec![0.75; 64]);
        let d = decontaminate(&rgb, &alpha, None, &DecontaminateParams::default()).unwrap();
        assert!(d.fg.iter().all(|p| p
            .data
            .iter()
            .all(|&v| v.is_finite() && (0.0..=1.0).contains(&v))));
    }

    #[test]
    fn explicit_band_mask_limits_the_defringe() {
        let (rgb, _, truth) = two_color_band(16, 8);
        let alpha = Plane::from_vec(16, 8, truth.iter().map(|&a| a as f32).collect());
        // Band mask covering NOTHING: result must equal the no-defringe
        // baseline (ml F + opaque override) everywhere.
        let empty_band = Plane::new(16, 8);
        let with = decontaminate(
            &rgb,
            &alpha,
            Some(&empty_band),
            &DecontaminateParams::default(),
        )
        .unwrap();
        let default_band =
            decontaminate(&rgb, &alpha, None, &DecontaminateParams::default()).unwrap();
        // The empty band must differ from the default band in the soft
        // edge (proving the mask is honored) …
        let mut differs = false;
        for c in 0..3 {
            if plane_hash(&with.fg[c].data) != plane_hash(&default_band.fg[c].data) {
                differs = true;
            }
        }
        assert!(differs, "band mask had no effect");
        // … and opaque pixels stay I in both.
        // Index loop kept: the assertions reference several arrays by i.
        #[allow(clippy::needless_range_loop)]
        for i in 0..truth.len() {
            if truth[i] >= 1.0 {
                for c in 0..3 {
                    assert_eq!(with.fg[c].data[i], rgb[c].data[i]);
                }
            }
        }
    }

    #[test]
    fn validation_errors() {
        let (rgb, _, _) = two_color_band(8, 8);
        let alpha_bad = Plane::from_vec(4, 4, vec![0.5; 16]);
        assert!(decontaminate(&rgb, &alpha_bad, None, &DecontaminateParams::default()).is_err());
        let alpha = Plane::from_vec(8, 8, vec![0.5; 64]);
        let band_bad = Plane::from_vec(4, 4, vec![0.0; 16]);
        assert!(decontaminate(
            &rgb,
            &alpha,
            Some(&band_bad),
            &DecontaminateParams::default()
        )
        .is_err());
    }

    #[test]
    fn determinism_two_runs_and_pinned_golden() {
        // Pinned from this crate's own runs AFTER the behavioral oracles
        // above were green (rule 8). Fixture: shared two-color band 16x8,
        // default params; hash over the red F plane.
        let (rgb, _, truth) = two_color_band(16, 8);
        let alpha = Plane::from_vec(16, 8, truth.iter().map(|&a| a as f32).collect());
        let a = decontaminate(&rgb, &alpha, None, &DecontaminateParams::default()).unwrap();
        let b = decontaminate(&rgb, &alpha, None, &DecontaminateParams::default()).unwrap();
        for c in 0..3 {
            assert_eq!(
                plane_hash(&a.fg[c].data),
                plane_hash(&b.fg[c].data),
                "two-run bit equality chan {c}"
            );
        }
        assert_eq!(
            plane_hash(&a.fg[0].data),
            PINNED_DECON_GOLDEN,
            "pinned golden hash"
        );
    }

    /// Pinned after oracle green (see `determinism_two_runs_and_pinned_golden`).
    const PINNED_DECON_GOLDEN: u64 = 16478136911506650881;

    // ---- tester round (Row B) ----

    #[test]
    fn fully_opaque_alpha_is_a_color_noop() {
        // α ≡ 1 everywhere: every pixel is opaque, so F = I EXACTLY
        // (the opaque override is unconditional and the band is empty —
        // no pixel has α < fg_alpha_min), and alpha passes through
        // bit-exactly. Asserted as exact equality, not a bound.
        let (rgb, _, _) = two_color_band(8, 8);
        let alpha = Plane::from_vec(8, 8, vec![1.0; 64]);
        let d = decontaminate(&rgb, &alpha, None, &DecontaminateParams::default()).unwrap();
        // Index loop kept: c addresses d.fg and rgb in lockstep (crate
        // precedent for multi-array index loops).
        #[allow(clippy::needless_range_loop)]
        for c in 0..3 {
            assert_eq!(
                plane_hash(&d.fg[c].data),
                plane_hash(&rgb[c].data),
                "opaque F != I on channel {c}"
            );
        }
        assert_eq!(plane_hash(&d.alpha.data), plane_hash(&alpha.data));
    }

    #[test]
    fn fully_transparent_alpha_yields_finite_foreground_pinned() {
        // α ≡ 0 everywhere: every pixel seeds the background estimate
        // and the band is empty, so F is the raw ml estimate. With
        // α = 0 the ml color model attributes nothing to the
        // foreground, and the estimate stays at its zero
        // initialization (measured: F ≡ 0.0 on all channels). A
        // composite ignores F entirely at α = 0, so the only honest
        // requirements are finiteness and range; the zero value is
        // pinned as current behavior, not a contract.
        let (rgb, _, _) = two_color_band(8, 8);
        let alpha = Plane::new(8, 8); // all zeros
        let d = decontaminate(&rgb, &alpha, None, &DecontaminateParams::default()).unwrap();
        for c in 0..3 {
            assert!(d.fg[c]
                .data
                .iter()
                .all(|&v| v.is_finite() && (0.0..=1.0).contains(&v)));
            assert!(
                d.fg[c].data.iter().all(|&v| v == 0.0),
                "current behavior pin: transparent F is the zero ml estimate"
            );
        }
        assert_eq!(plane_hash(&d.alpha.data), plane_hash(&alpha.data));
    }

    #[test]
    fn concave_c_shaped_band_extends_background_correctly() {
        // BFS correctness on a CONCAVE band shape: a C-shaped soft band
        // (α = 0.5, opening left) wraps an opaque block, with the only
        // background seeds in the two left columns. The diffusion field
        // covers EVERY pixel (rings propagate through opaque pixels
        // too), so the far arm of the C sits many rings deep (depth ≥
        // 12 here) behind chained ring means. Because the background
        // color is CONSTANT, the correct extension is exactly that
        // color at every pixel regardless of depth (means of a constant
        // stay the constant), so the stage-8 formula recovers the
        // generating F exactly at α = 0.5 ≥ the divisor floor:
        // F̂ = (0.5F + 0.5B − 0.5B)/0.5 = F. Any BFS defect
        // (unassigned-parent read, ring-order slip, missed frontier
        // pixel) would contaminate a mean with zeros and show up as a
        // large F̂ error on some band pixel.
        let (w, h) = (16usize, 12usize);
        let f_true = FIX_FG;
        let b_true = FIX_BG;
        let mut alpha = Plane::new(w, h);
        let mut rgb = [Plane::new(w, h), Plane::new(w, h), Plane::new(w, h)];
        // Geometry: bg seeds in columns 0-1; opaque block x 5..=12,
        // y 3..=8; the band is the one-pixel ring x 4..=13 × y 2..=9
        // around the block MINUS its left column (x == 4 stays opaque
        // filler), leaving a C of top arm, right arm, bottom arm;
        // everything else opaque filler.
        for y in 0..h {
            for x in 0..w {
                let in_block = (5..=12).contains(&x) && (3..=8).contains(&y);
                let in_band = !in_block && (4..=13).contains(&x) && (2..=9).contains(&y) && x != 4;
                let a = if in_block {
                    1.0
                } else if in_band {
                    0.5
                } else if x < 2 {
                    0.0
                } else {
                    1.0 // opaque filler (keeps the band concave)
                };
                alpha.data[y * w + x] = a;
                for c in 0..3 {
                    rgb[c].data[y * w + x] = a * f_true[c] + (1.0 - a) * b_true[c];
                }
            }
        }
        let d = decontaminate(&rgb, &alpha, None, &DecontaminateParams::default()).unwrap();
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if alpha.data[i] == 0.5 {
                    // Index loop kept: c addresses d.fg and f_true in
                    // lockstep (crate precedent).
                    #[allow(clippy::needless_range_loop)]
                    for c in 0..3 {
                        let dev = (d.fg[c].data[i] - f_true[c]).abs();
                        assert!(dev < 1e-3, "band pixel ({x},{y}) chan {c}: F̂ off by {dev}");
                    }
                }
            }
        }
    }

    #[test]
    fn nan_alpha_propagates_pinned() {
        // NaN input policy, pinned honestly: decontaminate does NOT
        // validate pixel data (dimension/threshold validation only). A
        // NaN alpha pixel fails the seed test and the band test
        // (comparisons with NaN are false), flows into the ml estimate
        // and the alpha pass-through, and the result CARRIES NaN.
        // Documents current behavior, not a contract — upstream
        // detectors validate before this seam.
        let (rgb, _, truth) = two_color_band(16, 8);
        let mut alpha = Plane::from_vec(16, 8, truth.iter().map(|&a| a as f32).collect());
        alpha.data[3 * 16 + 8] = f32::NAN;
        let d = decontaminate(&rgb, &alpha, None, &DecontaminateParams::default()).unwrap();
        assert!(
            d.alpha.data.iter().any(|v| v.is_nan()),
            "alpha pass-through lost the NaN"
        );
        assert!(
            d.fg.iter().any(|p| p.data.iter().any(|v| v.is_nan())),
            "ml estimate did not surface the NaN"
        );
    }
}
