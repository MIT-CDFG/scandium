// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Guided-filter matting — the `alpha_matting` contract's `interactive`
//! tier `guided_filter_matting` (level L2, basis he10 + he15fast).
//!
//! Clean-room basis: He, Sun, Tang, "Guided Image Filtering", ECCV 2010 /
//! TPAMI 2013 (`he10`) and He, Sun, "Fast Guided Filter", arXiv:1505.00996
//! (`he15fast`). No reference code consulted for the he15fast scheme —
//! implemented from the paper.
//!
//! Spec: ES §13.4 Select-and-Mask pipeline stage 3 [NORMATIVE] (verified
//! against CURRENT text): "solve alpha inside B with the guided filter
//! (He, Sun & Tang 2010) … window radius = r and regularizer eps (default
//! 1e-4); hard constraints α = 1 where d < −r, α = 0 where d > r".
//! ALGORITHMS.md §7.B: "Fast path — guided-filter matting [R-he10]
//! [R-he15fast] (L2) … radius r = 8·⌈diag/2048⌉, ε = 1e-4; this is the
//! interactive-preview engine and the whole story on `draft`"; §7.E
//! ladder step 1. The radius policy (8·⌈diag/2048⌉) is the CONSUMER's —
//! engines take r as a parameter.
//!
//! REUSE: the full-resolution path COMPOSES the existing
//! [`crate::edge_aware::guided::guided_filter`] engine (called, not
//! copied — that module is write-fenced this row). The he15fast
//! subsampled path cannot be composed from that engine's public surface
//! (it needs the per-window coefficients a, b BEFORE the final
//! averaging-and-output step, which the fenced API does not expose), so
//! [`fast_guided_filter`] implements the he15fast variant STANDALONE in
//! this module from the papers, duplicating the ~30-line box-mean +
//! coefficient math. HONEST DUPLICATION NOTE: flagged in the lane log as
//! a future hygiene-merge candidate (expose a coefficient-level seam in
//! `edge_aware/guided.rs` once that fence lifts).
//!
//! ROI seam: [`guided_filter_matting_roi`] refines only a dirty rect +
//! guard band — the workspace strokes locally (ES §13.4 "a slider change
//! re-runs from its own stage forward"; brush dabs dirty small rects).
//! With `guard >= 2·r` the interior agrees with the full-image run up to
//! prefix-sum rounding (the filter's total support is 2r); the guard ring
//! itself is recomputed but NOT written back.
//!
//! Determinism: single-threaded, fixed row-major order, f64 accumulation;
//! composition inherits the fenced engine's determinism.

use crate::edge_aware::guided::guided_filter;
use crate::plane::Plane;

use super::trimap_split;

/// Rectangular region of interest (pixel units, inclusive origin).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Roi {
    /// Left edge (px).
    pub x: usize,
    /// Top edge (px).
    pub y: usize,
    /// Width (px, > 0).
    pub w: usize,
    /// Height (px, > 0).
    pub h: usize,
}

/// Pin definite trimap regions: α = 1 on fg, α = 0 on bg (ES §13.4
/// stage-3 hard-constraint contract, expressed on the trimap the
/// consumer derives from the distance band).
fn pin_trimap(alpha: &mut Plane, trimap: &Plane) {
    let split = trimap_split(trimap);
    for i in 0..alpha.data.len() {
        if split.is_fg[i] {
            alpha.data[i] = 1.0;
        } else if split.is_bg[i] {
            alpha.data[i] = 0.0;
        }
    }
}

/// Interactive guided-filter matting (full-resolution path): refine a
/// coarse alpha (binary/soft mask or previous matte) against the guide
/// image, clamp to [0, 1], and pin definite `trimap` regions when given.
///
/// `guide` is one plane (the consumer's working luma; ES §13.4 uses the
/// working-RGB composite as guidance — the gray-guide reduction is this
/// engine's documented seam, the color-guide variant is a follow-up if
/// quality demands it). Errors mirror the underlying engine's validation
/// (dimension mismatch, empty planes, r == 0, bad eps).
pub fn guided_filter_matting(
    guide: &Plane,
    coarse_alpha: &Plane,
    trimap: Option<&Plane>,
    r: usize,
    eps: f64,
) -> Result<Plane, String> {
    if let Some(t) = trimap {
        if t.w != coarse_alpha.w || t.h != coarse_alpha.h {
            return Err(format!(
                "guided_filter_matting: trimap {}x{} vs alpha {}x{} dimension mismatch",
                t.w, t.h, coarse_alpha.w, coarse_alpha.h
            ));
        }
    }
    let mut alpha = guided_filter(guide, coarse_alpha, r, eps)?;
    for v in alpha.data.iter_mut() {
        *v = v.clamp(0.0, 1.0);
    }
    if let Some(t) = trimap {
        pin_trimap(&mut alpha, t);
    }
    Ok(alpha)
}

// ---------------------------------------------------------------------------
// he15fast subsampled path (standalone; see module docs for the honest
// duplication note)
// ---------------------------------------------------------------------------

/// Truncated-window box mean (f64), same convention as the fenced
/// engine: separable prefix sums normalized by the true window count.
fn box_mean_f64(src: &[f64], w: usize, h: usize, r: usize) -> Vec<f64> {
    let mut tmp = vec![0f64; w * h];
    let mut prefix = vec![0f64; w + 1];
    for y in 0..h {
        for x in 0..w {
            prefix[x + 1] = prefix[x] + src[y * w + x];
        }
        for x in 0..w {
            let lo = x.saturating_sub(r);
            let hi = (x + r + 1).min(w);
            tmp[y * w + x] = prefix[hi] - prefix[lo];
        }
    }
    let mut out = vec![0f64; w * h];
    let mut prefix_v = vec![0f64; h + 1];
    for x in 0..w {
        for y in 0..h {
            prefix_v[y + 1] = prefix_v[y] + tmp[y * w + x];
        }
        for y in 0..h {
            let lo = y.saturating_sub(r);
            let hi = (y + r + 1).min(h);
            out[y * w + x] = prefix_v[hi] - prefix_v[lo];
        }
    }
    let count = |i: usize, n: usize| -> f64 {
        let lo = i.saturating_sub(r);
        let hi = (i + r + 1).min(n);
        (hi - lo) as f64
    };
    for y in 0..h {
        let cy = count(y, h);
        for x in 0..w {
            out[y * w + x] /= count(x, w) * cy;
        }
    }
    out
}

/// s×s block-mean decimation to ceil(w/s) × ceil(h/s) (truncated edge
/// blocks; deterministic). The he15fast paper subsamples I and p by
/// ratio s — block means are this implementation's documented
/// anti-aliasing choice (nearest decimation is the paper's cheapest
/// variant; means keep the low-res window statistics closer to the
/// full-resolution ones on synthetic fixtures).
fn downsample(src: &[f64], w: usize, h: usize, s: usize) -> (Vec<f64>, usize, usize) {
    let wl = w.div_ceil(s);
    let hl = h.div_ceil(s);
    let mut out = vec![0f64; wl * hl];
    for yl in 0..hl {
        for xl in 0..wl {
            let x0 = xl * s;
            let y0 = yl * s;
            let x1 = (x0 + s).min(w);
            let y1 = (y0 + s).min(h);
            let mut acc = 0f64;
            for y in y0..y1 {
                for x in x0..x1 {
                    acc += src[y * w + x];
                }
            }
            out[yl * wl + xl] = acc / ((x1 - x0) * (y1 - y0)) as f64;
        }
    }
    (out, wl, hl)
}

/// Bilinear upsample from (wl, hl) to (w, h) with block-center alignment
/// (`xf -> (xf + 0.5)/s - 0.5` in low-res coordinates, edge-clamped).
fn upsample_bilinear(src: &[f64], wl: usize, hl: usize, w: usize, h: usize, s: usize) -> Vec<f64> {
    let mut out = vec![0f64; w * h];
    let sf = s as f64;
    for y in 0..h {
        let fy = ((y as f64 + 0.5) / sf - 0.5).clamp(0.0, (hl - 1) as f64);
        let y0 = fy.floor() as usize;
        let y1 = (y0 + 1).min(hl - 1);
        let ty = fy - y0 as f64;
        for x in 0..w {
            let fx = ((x as f64 + 0.5) / sf - 0.5).clamp(0.0, (wl - 1) as f64);
            let x0 = fx.floor() as usize;
            let x1 = (x0 + 1).min(wl - 1);
            let tx = fx - x0 as f64;
            let a = src[y0 * wl + x0];
            let b = src[y0 * wl + x1];
            let c = src[y1 * wl + x0];
            let d = src[y1 * wl + x1];
            out[y * w + x] = a * (1.0 - tx) * (1.0 - ty)
                + b * tx * (1.0 - ty)
                + c * (1.0 - tx) * ty
                + d * tx * ty;
        }
    }
    out
}

/// Fast guided filter (he15fast): compute the per-window linear-model
/// coefficients on an s-times-subsampled grid with radius `r/s`,
/// bilinearly upsample the averaged coefficients, and evaluate
/// `q = ā·I + b̄` against the FULL-resolution guide. `s == 1` is the
/// exact he10 path (this module's own arithmetic; agreement with the
/// fenced engine is a tested bound, not a bit claim).
pub fn fast_guided_filter(
    guide: &Plane,
    input: &Plane,
    r: usize,
    eps: f64,
    s: usize,
) -> Result<Plane, String> {
    if guide.w == 0 || guide.h == 0 {
        return Err("fast_guided_filter: empty plane".to_string());
    }
    if guide.w != input.w || guide.h != input.h {
        return Err(format!(
            "fast_guided_filter: guide {}x{} vs input {}x{} dimension mismatch",
            guide.w, guide.h, input.w, input.h
        ));
    }
    if r == 0 {
        return Err("fast_guided_filter: radius must be >= 1".to_string());
    }
    if s == 0 {
        return Err("fast_guided_filter: subsample ratio must be >= 1".to_string());
    }
    if !(eps.is_finite() && eps > 0.0) {
        return Err(format!(
            "fast_guided_filter: eps must be finite and > 0 (got {eps})"
        ));
    }
    let (w, h) = (guide.w, guide.h);
    let i_full: Vec<f64> = guide.data.iter().map(|&v| v as f64).collect();
    let p_full: Vec<f64> = input.data.iter().map(|&v| v as f64).collect();
    let (i_lo, wl, hl) = downsample(&i_full, w, h, s);
    let (p_lo, _, _) = downsample(&p_full, w, h, s);
    let rl = (r / s).max(1);
    let n_lo = wl * hl;
    let mean_i = box_mean_f64(&i_lo, wl, hl, rl);
    let mean_p = box_mean_f64(&p_lo, wl, hl, rl);
    let ii: Vec<f64> = (0..n_lo).map(|k| i_lo[k] * i_lo[k]).collect();
    let ip: Vec<f64> = (0..n_lo).map(|k| i_lo[k] * p_lo[k]).collect();
    let corr_ii = box_mean_f64(&ii, wl, hl, rl);
    let corr_ip = box_mean_f64(&ip, wl, hl, rl);
    let mut a = vec![0f64; n_lo];
    let mut b = vec![0f64; n_lo];
    for k in 0..n_lo {
        let var_i = corr_ii[k] - mean_i[k] * mean_i[k];
        let cov_ip = corr_ip[k] - mean_i[k] * mean_p[k];
        a[k] = cov_ip / (var_i + eps);
        b[k] = mean_p[k] - a[k] * mean_i[k];
    }
    let mean_a = box_mean_f64(&a, wl, hl, rl);
    let mean_b = box_mean_f64(&b, wl, hl, rl);
    let a_full = upsample_bilinear(&mean_a, wl, hl, w, h, s);
    let b_full = upsample_bilinear(&mean_b, wl, hl, w, h, s);
    let mut out = Plane::new(w, h);
    for k in 0..w * h {
        out.data[k] = (a_full[k] * i_full[k] + b_full[k]) as f32;
    }
    Ok(out)
}

/// Interactive-tier fast path: [`fast_guided_filter`] + clamp + trimap
/// pinning (the he15fast preview engine; the paper recommends `s = r/4`
/// — the consumer picks s per its draft/interactive quality setting).
pub fn guided_filter_matting_fast(
    guide: &Plane,
    coarse_alpha: &Plane,
    trimap: Option<&Plane>,
    r: usize,
    eps: f64,
    s: usize,
) -> Result<Plane, String> {
    if let Some(t) = trimap {
        if t.w != coarse_alpha.w || t.h != coarse_alpha.h {
            return Err(format!(
                "guided_filter_matting_fast: trimap {}x{} vs alpha {}x{} dimension mismatch",
                t.w, t.h, coarse_alpha.w, coarse_alpha.h
            ));
        }
    }
    let mut alpha = fast_guided_filter(guide, coarse_alpha, r, eps, s)?;
    for v in alpha.data.iter_mut() {
        *v = v.clamp(0.0, 1.0);
    }
    if let Some(t) = trimap {
        pin_trimap(&mut alpha, t);
    }
    Ok(alpha)
}

/// Region-of-interest refinement: run the full-resolution tier on
/// `roi` inflated by `guard` px, write the refined values back INSIDE
/// `roi` only, and keep `coarse_alpha` (with trimap pinning) elsewhere.
///
/// Seam contract: with `guard >= 2·r` the result inside `roi` agrees
/// with a whole-image [`guided_filter_matting`] run up to prefix-sum
/// rounding (the guided filter's total support is `2r`: window stats
/// reach `r`, coefficient averaging another `r`); smaller guards trade
/// accuracy at the roi rim for speed — the workspace decides.
pub fn guided_filter_matting_roi(
    guide: &Plane,
    coarse_alpha: &Plane,
    trimap: Option<&Plane>,
    r: usize,
    eps: f64,
    roi: Roi,
    guard: usize,
) -> Result<Plane, String> {
    let (w, h) = (coarse_alpha.w, coarse_alpha.h);
    if roi.w == 0 || roi.h == 0 {
        return Err("guided_filter_matting_roi: empty roi".to_string());
    }
    if roi.x + roi.w > w || roi.y + roi.h > h {
        return Err(format!(
            "guided_filter_matting_roi: roi {}+{}x{}+{} exceeds plane {}x{}",
            roi.x, roi.w, roi.y, roi.h, w, h
        ));
    }
    if guide.w != w || guide.h != h {
        return Err(format!(
            "guided_filter_matting_roi: guide {}x{} vs alpha {}x{} dimension mismatch",
            guide.w, guide.h, w, h
        ));
    }
    // Inflated crop window.
    let cx0 = roi.x.saturating_sub(guard);
    let cy0 = roi.y.saturating_sub(guard);
    let cx1 = (roi.x + roi.w + guard).min(w);
    let cy1 = (roi.y + roi.h + guard).min(h);
    let (cw, ch) = (cx1 - cx0, cy1 - cy0);
    let crop = |src: &Plane| -> Plane {
        let mut out = Plane::new(cw, ch);
        for y in 0..ch {
            for x in 0..cw {
                out.data[y * cw + x] = src.at(cx0 + x, cy0 + y);
            }
        }
        out
    };
    let guide_c = crop(guide);
    let alpha_c = crop(coarse_alpha);
    let trimap_c = trimap.map(crop);
    let refined_c = guided_filter_matting(&guide_c, &alpha_c, trimap_c.as_ref(), r, eps)?;
    let mut out = coarse_alpha.clone();
    for v in out.data.iter_mut() {
        *v = v.clamp(0.0, 1.0);
    }
    if let Some(t) = trimap {
        pin_trimap(&mut out, t);
    }
    for y in roi.y..roi.y + roi.h {
        for x in roi.x..roi.x + roi.w {
            out.data[y * w + x] = refined_c.at(x - cx0, y - cy0);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plane::{plane_hash, SplitMix64};

    /// Hair-like fixture: thin bright strands (1-px, varying phase) over a
    /// dark background — the thin-structure class the interactive tier
    /// exists for. Returns (guide, true alpha).
    fn hair_fixture(w: usize, h: usize) -> (Plane, Plane) {
        let mut guide = Plane::new(w, h);
        let mut truth = Plane::new(w, h);
        for y in 0..h {
            for x in 0..w {
                // Strands: every 4th column, drifting one px every 8 rows.
                let strand = (x + y / 8) % 4 == 0;
                let a = if strand { 1.0 } else { 0.0 };
                truth.data[y * w + x] = a;
                guide.data[y * w + x] = 0.15 + 0.7 * a;
            }
        }
        (guide, truth)
    }

    /// Noisy coarse alpha: truth + seeded zero-mean uniform noise,
    /// deliberately UNCLAMPED (a clamped 0/1 matte gives one-sided,
    /// guide-correlated noise — a systematic shrink no filter should be
    /// expected to undo; coarse inputs outside [0, 1] are legal engine
    /// inputs and the tier clamps its OUTPUT).
    fn noisy(truth: &Plane, amp: f32, seed: u64) -> Plane {
        let mut rng = SplitMix64(seed);
        let mut out = truth.clone();
        for v in out.data.iter_mut() {
            *v += amp * (2.0 * rng.next_f64() as f32 - 1.0);
        }
        out
    }

    fn mean_abs_err(a: &Plane, b: &Plane) -> f64 {
        a.data
            .iter()
            .zip(&b.data)
            .map(|(x, y)| (x - y).abs() as f64)
            .sum::<f64>()
            / a.data.len() as f64
    }

    #[test]
    fn refines_noisy_coarse_alpha_toward_truth_on_thin_structures() {
        // The tier's whole point: a noisy coarse matte over hair-like
        // strands must move TOWARD the ground truth, guided by the image
        // (quantified: error at least halved; measured mean |err|
        // 0.174 -> 0.0114, a 15x reduction).
        let (guide, truth) = hair_fixture(48, 32);
        let coarse = noisy(&truth, 0.35, 0x0B_0001);
        let refined = guided_filter_matting(&guide, &coarse, None, 2, 1e-4).unwrap();
        let before = mean_abs_err(&coarse, &truth);
        let after = mean_abs_err(&refined, &truth);
        assert!(
            after < 0.5 * before,
            "refinement did not help: {before} -> {after}"
        );
        assert!(refined.data.iter().all(|&v| (0.0..=1.0).contains(&v)));
    }

    #[test]
    fn trimap_pinning_is_exact() {
        let (guide, truth) = hair_fixture(32, 16);
        let coarse = noisy(&truth, 0.3, 0x0B_0002);
        // Trimap: left quarter definite bg, right quarter definite fg.
        let mut trimap = Plane::from_vec(32, 16, vec![0.5; 32 * 16]);
        for y in 0..16 {
            for x in 0..8 {
                trimap.data[y * 32 + x] = 0.0;
            }
            for x in 24..32 {
                trimap.data[y * 32 + x] = 1.0;
            }
        }
        let refined = guided_filter_matting(&guide, &coarse, Some(&trimap), 3, 1e-4).unwrap();
        for y in 0..16 {
            for x in 0..8 {
                assert_eq!(refined.at(x, y), 0.0);
            }
            for x in 24..32 {
                assert_eq!(refined.at(x, y), 1.0);
            }
        }
    }

    #[test]
    fn fast_path_agrees_with_full_resolution_within_bound() {
        // he15fast approximation class: the paper reports the subsampled
        // filter as a close approximation for r >= s on natural content.
        // Honest bounds, measured on this fixture (smooth blob + noisy
        // coarse alpha): s=2 worst deviation 0.00523, asserted 0.02 with
        // margin. s=1 measured 0.0 exactly on this fixture — this
        // module's standalone arithmetic happens to reproduce the fenced
        // engine's prefix-sum order bit-for-bit here; asserted at the
        // 1e-5 class, NOT claimed as a bit contract.
        let (w, h) = (48usize, 40usize);
        let mut guide = Plane::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let dx = x as f32 - 24.0;
                let dy = y as f32 - 20.0;
                let d = (dx * dx + dy * dy).sqrt();
                guide.data[y * w + x] = 0.2 + 0.6 / (1.0 + (0.5 * (d - 10.0)).exp());
            }
        }
        let coarse = {
            let mut c = guide.clone();
            let mut rng = SplitMix64(0x0B_0003);
            for v in c.data.iter_mut() {
                *v = (*v + 0.1 * (2.0 * rng.next_f64() as f32 - 1.0)).clamp(0.0, 1.0);
            }
            c
        };
        let full = guided_filter_matting(&guide, &coarse, None, 4, 1e-4).unwrap();
        let s1 = guided_filter_matting_fast(&guide, &coarse, None, 4, 1e-4, 1).unwrap();
        let s2 = guided_filter_matting_fast(&guide, &coarse, None, 4, 1e-4, 2).unwrap();
        let worst = |a: &Plane, b: &Plane| {
            a.data
                .iter()
                .zip(&b.data)
                .map(|(x, y)| (x - y).abs())
                .fold(0f32, f32::max)
        };
        let d1 = worst(&s1, &full);
        let d2 = worst(&s2, &full);
        assert!(d1 <= 1e-5, "s=1 vs fenced engine deviation {d1}");
        assert!(d2 <= 0.02, "s=2 vs full-resolution deviation {d2}");
    }

    #[test]
    fn roi_matches_full_inside_roi_with_2r_guard() {
        let (guide, truth) = hair_fixture(64, 48);
        let coarse = noisy(&truth, 0.3, 0x0B_0004);
        let r = 3usize;
        let roi = Roi {
            x: 20,
            y: 12,
            w: 16,
            h: 16,
        };
        let full = guided_filter_matting(&guide, &coarse, None, r, 1e-4).unwrap();
        let part = guided_filter_matting_roi(&guide, &coarse, None, r, 1e-4, roi, 2 * r).unwrap();
        // Inside the roi: agree up to prefix-sum rounding (measured 0.0
        // exactly on this fixture — truncated-window means make the
        // cropped run's f64 sums identical here; asserted at the 1e-5
        // class, NOT claimed as a bit contract).
        let mut worst = 0f32;
        for y in roi.y..roi.y + roi.h {
            for x in roi.x..roi.x + roi.w {
                worst = worst.max((full.at(x, y) - part.at(x, y)).abs());
            }
        }
        assert!(worst <= 1e-5, "roi interior deviation {worst}");
        // Outside the roi: exactly the (clamped) coarse input.
        for y in 0..48 {
            for x in 0..64 {
                let inside =
                    (roi.x..roi.x + roi.w).contains(&x) && (roi.y..roi.y + roi.h).contains(&y);
                if !inside {
                    assert_eq!(part.at(x, y), coarse.at(x, y).clamp(0.0, 1.0));
                }
            }
        }
    }

    #[test]
    fn validation_errors() {
        let p = Plane::from_vec(8, 8, vec![0.5; 64]);
        let q = Plane::from_vec(9, 8, vec![0.5; 72]);
        assert!(guided_filter_matting(&p, &q, None, 2, 1e-4).is_err());
        assert!(guided_filter_matting(&p, &p, Some(&q), 2, 1e-4).is_err());
        assert!(fast_guided_filter(&p, &p, 0, 1e-4, 2).is_err());
        assert!(fast_guided_filter(&p, &p, 2, 1e-4, 0).is_err());
        assert!(fast_guided_filter(&p, &p, 2, 0.0, 2).is_err());
        let roi_bad = Roi {
            x: 6,
            y: 0,
            w: 4,
            h: 4,
        };
        assert!(guided_filter_matting_roi(&p, &p, None, 2, 1e-4, roi_bad, 2).is_err());
    }

    #[test]
    fn determinism_two_runs_and_pinned_goldens() {
        // Pinned from this crate's own runs AFTER the behavioral oracles
        // above were green (rule 8). Fixture: hair 48x32, noise seed
        // 0x0B_601D, r = 2, eps = 1e-4; fast path s = 2.
        let (guide, truth) = hair_fixture(48, 32);
        let coarse = noisy(&truth, 0.3, 0x0B_601D);
        let a = guided_filter_matting(&guide, &coarse, None, 2, 1e-4).unwrap();
        let b = guided_filter_matting(&guide, &coarse, None, 2, 1e-4).unwrap();
        assert_eq!(plane_hash(&a.data), plane_hash(&b.data), "full two-run");
        let fa = guided_filter_matting_fast(&guide, &coarse, None, 2, 1e-4, 2).unwrap();
        let fb = guided_filter_matting_fast(&guide, &coarse, None, 2, 1e-4, 2).unwrap();
        assert_eq!(plane_hash(&fa.data), plane_hash(&fb.data), "fast two-run");
        assert_eq!(plane_hash(&a.data), PINNED_FULL_GOLDEN, "full golden");
        assert_eq!(plane_hash(&fa.data), PINNED_FAST_GOLDEN, "fast golden");
    }

    /// Pinned after oracle green (see `determinism_two_runs_and_pinned_goldens`).
    const PINNED_FULL_GOLDEN: u64 = 17861346464792005789;
    const PINNED_FAST_GOLDEN: u64 = 3361578419145876366;

    // ---- tester round (Row B) ----

    #[test]
    fn roi_at_canvas_corners_clipped_guard_still_matches_full() {
        // The 2r-guard seam claim at the canvas rim: when the guard
        // band is CLIPPED by the image edge, the crop boundary
        // coincides with the canvas boundary, so the truncated-window
        // box means see exactly the same pixels as the full-image run —
        // the interior bound must hold at all four rim positions.
        // Measured worst deviation 0.0 at every corner; asserted at the
        // builder's 1e-5 class, NOT claimed as a bit contract.
        let (guide, truth) = hair_fixture(64, 48);
        let coarse = noisy(&truth, 0.3, 0x0B_0005);
        let r = 3usize;
        let full = guided_filter_matting(&guide, &coarse, None, r, 1e-4).unwrap();
        for roi in [
            Roi {
                x: 0,
                y: 0,
                w: 10,
                h: 10,
            }, // top-left corner
            Roi {
                x: 54,
                y: 0,
                w: 10,
                h: 10,
            }, // top-right corner
            Roi {
                x: 0,
                y: 38,
                w: 10,
                h: 10,
            }, // bottom-left corner
            Roi {
                x: 54,
                y: 38,
                w: 10,
                h: 10,
            }, // bottom-right corner
            Roi {
                x: 20,
                y: 0,
                w: 16,
                h: 8,
            }, // top edge
        ] {
            let part =
                guided_filter_matting_roi(&guide, &coarse, None, r, 1e-4, roi, 2 * r).unwrap();
            let mut worst = 0f32;
            for y in roi.y..roi.y + roi.h {
                for x in roi.x..roi.x + roi.w {
                    worst = worst.max((full.at(x, y) - part.at(x, y)).abs());
                }
            }
            assert!(worst <= 1e-5, "roi {roi:?} interior deviation {worst}");
        }
    }

    #[test]
    fn roi_covering_whole_image_is_bit_identical_to_full_path() {
        // Degenerate ROI = the whole canvas: the crop is the identity,
        // so the refined crop IS a full-path run and the write-back
        // covers every pixel — bit identity is structural here (same
        // arithmetic on the same buffers), and measured true. Guard 0
        // exercises the no-inflation path.
        let (guide, truth) = hair_fixture(64, 48);
        let coarse = noisy(&truth, 0.3, 0x0B_0006);
        let full = guided_filter_matting(&guide, &coarse, None, 3, 1e-4).unwrap();
        let whole = Roi {
            x: 0,
            y: 0,
            w: 64,
            h: 48,
        };
        let part = guided_filter_matting_roi(&guide, &coarse, None, 3, 1e-4, whole, 0).unwrap();
        assert_eq!(plane_hash(&part.data), plane_hash(&full.data));
    }

    #[test]
    fn perfect_coarse_alpha_refinement_is_near_identity() {
        // Coarse alpha already exact (the hair truth itself) with a
        // guide that is an affine function of it: the guided filter's
        // linear model represents the input almost perfectly and
        // refinement must be near-identity (eps shrinks the slope
        // slightly). Measured worst |refined − truth| = 9.8e-4;
        // asserted 5e-3 with margin.
        let (guide, truth) = hair_fixture(48, 32);
        let refined = guided_filter_matting(&guide, &truth, None, 2, 1e-4).unwrap();
        let worst = refined
            .data
            .iter()
            .zip(&truth.data)
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        assert!(worst < 5e-3, "near-identity violated: worst dev {worst}");
    }

    #[test]
    fn nan_input_propagates_into_output_pinned() {
        // NaN input policy, pinned honestly: this tier does NOT
        // validate pixel data (parameter validation only) — a NaN in
        // the guide or the coarse alpha spreads through the box means
        // over the filter support and the output CONTAINS NaN (the
        // final clamp keeps NaN: f32::clamp of NaN is NaN). Measured:
        // 96 NaN pixels from one seeded NaN at r = 2 on 16x8.
        // Documents current behavior, not a contract — upstream
        // detectors validate before this seam.
        let mut guide = Plane::new(16, 8);
        for (i, v) in guide.data.iter_mut().enumerate() {
            *v = (i % 7) as f32 / 7.0;
        }
        let coarse = Plane::from_vec(16, 8, vec![0.5; 128]);
        let mut g_nan = guide.clone();
        g_nan.data[40] = f32::NAN;
        let out = guided_filter_matting(&g_nan, &coarse, None, 2, 1e-4).unwrap();
        assert!(
            out.data.iter().any(|v| v.is_nan()),
            "guide NaN did not surface"
        );
        let mut c_nan = coarse.clone();
        c_nan.data[40] = f32::NAN;
        let out = guided_filter_matting(&guide, &c_nan, None, 2, 1e-4).unwrap();
        assert!(
            out.data.iter().any(|v| v.is_nan()),
            "coarse NaN did not surface"
        );
        // The fast path shares the arithmetic; same policy.
        let out = guided_filter_matting_fast(&g_nan, &coarse, None, 2, 1e-4, 2).unwrap();
        assert!(
            out.data.iter().any(|v| v.is_nan()),
            "fast-path NaN did not surface"
        );
    }
}
