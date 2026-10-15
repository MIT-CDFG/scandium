// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Guided filter — clean-room from He, Sun, Tang, "Guided Image
//! Filtering", ECCV 2010 / TPAMI 2013 (contract basis key `he10`).
//!
//! Spec: ALGORITHMS.md §14.B (guided filter builds the Shadows/Highlights
//! base/detail split, r = 30 px @ 12 MP reference scale, eps = 0.01^2 in
//! linear luma); dossier `shadows-highlights` (selected + math rows); ES
//! §17.2 `adjustment.shadows_highlights` parameter surface (the lift
//! curves themselves are the O-TOOLS consumer's, not this module's).
//!
//! DESIGN-AROUND (patent-register `P-9241093`, US9241093B2, machine field
//! `design_around: US9241093B2` on contract entry `guided_filter_split`).
//! Register memo, quoted verbatim: "base guided filter only; claimed
//! variance-gain video pipeline not implemented". This module follows the
//! memo structurally: it exposes exactly the base guided filter of the
//! he10 paper (per-window linear model, means via box filters) and a plain
//! base/detail SPLIT ([`guided_split`]: base = filtered luma, detail =
//! luma - base). There is NO variance-driven per-pixel gain stage, no
//! gain look-up pipeline, and no temporal/video path here or downstream in
//! the split API — tone lifts are applied by the consumer to the BASE
//! signal per ES §17.2, which is not the claimed pipeline.
//!
//! Algorithm (he10): within each (2r+1)^2 window w_k the filter output is
//! the linear model q = a_k * I + b_k with
//!   a_k = cov_w(I, p) / (var_w(I) + eps),   b_k = mean_w(p) - a_k * mean_w(I)
//! and per-pixel output q(i) = mean_i(a) * I(i) + mean_i(b) (the paper's
//! eq. 5–8). All window means are O(1)-per-pixel box means computed by
//! prefix sums with truncated (edge-clamped) windows normalized by the
//! true window pixel count. Border handling is this implementation's own
//! documented choice — no in-tree source anchors a border convention for
//! the paper's `boxfilter`; truncated-window/true-count normalization
//! keeps every window mean an actual mean of in-bounds samples.
//!
//! Determinism: single-threaded, fixed row-major order, f64 prefix-sum
//! accumulation; no RNG, no libm transcendentals on the default path.

use crate::plane::Plane;

/// Horizontal running-sum pass: `out[y][x] = sum(src[y][x-r ..= x+r])`
/// with the window truncated at the borders (f64 accumulation, prefix
/// sums — O(1) per pixel, deterministic left-to-right order).
fn box_sum_h(src: &[f64], w: usize, h: usize, r: usize) -> Vec<f64> {
    let mut out = vec![0f64; w * h];
    let mut prefix = vec![0f64; w + 1];
    for y in 0..h {
        let row = &src[y * w..(y + 1) * w];
        for x in 0..w {
            prefix[x + 1] = prefix[x] + row[x];
        }
        for x in 0..w {
            let lo = x.saturating_sub(r);
            let hi = (x + r + 1).min(w);
            out[y * w + x] = prefix[hi] - prefix[lo];
        }
    }
    out
}

/// Vertical running-sum pass (same convention as [`box_sum_h`]).
fn box_sum_v(src: &[f64], w: usize, h: usize, r: usize) -> Vec<f64> {
    let mut out = vec![0f64; w * h];
    let mut prefix = vec![0f64; h + 1];
    for x in 0..w {
        for y in 0..h {
            prefix[y + 1] = prefix[y] + src[y * w + x];
        }
        for y in 0..h {
            let lo = y.saturating_sub(r);
            let hi = (y + r + 1).min(h);
            out[y * w + x] = prefix[hi] - prefix[lo];
        }
    }
    out
}

/// Truncated-window pixel count along one axis at index `i` of extent `n`.
#[inline]
fn axis_count(i: usize, n: usize, r: usize) -> f64 {
    let lo = i.saturating_sub(r);
    let hi = (i + r + 1).min(n);
    (hi - lo) as f64
}

/// Box MEAN with truncated windows (he10 `boxfilter(x) ./ N` convention):
/// separable running sums divided by the true per-pixel window count.
fn box_mean_f64(src: &[f64], w: usize, h: usize, r: usize) -> Vec<f64> {
    let mut out = box_sum_v(&box_sum_h(src, w, h, r), w, h, r);
    for y in 0..h {
        let cy = axis_count(y, h, r);
        for x in 0..w {
            out[y * w + x] /= axis_count(x, w, r) * cy;
        }
    }
    out
}

/// Box mean of a plane (truncated windows, radius `r`) — the O(1)
/// running-sum path. Public so tests and consumers can reach the exact
/// mean the filter uses internally.
///
/// # Panics
/// Panics if the plane is empty.
pub fn box_mean(src: &Plane, r: usize) -> Plane {
    assert!(src.w > 0 && src.h > 0, "box_mean: empty plane");
    let src64: Vec<f64> = src.data.iter().map(|&v| v as f64).collect();
    let m = box_mean_f64(&src64, src.w, src.h, r);
    Plane::from_vec(src.w, src.h, m.iter().map(|&v| v as f32).collect())
}

/// Guided filter (he10), gray guide: filter `input` (p) guided by `guide`
/// (I), window radius `r`, regularization `eps`.
///
/// Errors on dimension mismatch, empty planes, `r == 0`, or
/// non-finite/non-positive `eps`.
pub fn guided_filter(guide: &Plane, input: &Plane, r: usize, eps: f64) -> Result<Plane, String> {
    if guide.w == 0 || guide.h == 0 {
        return Err("guided_filter: empty plane".to_string());
    }
    if guide.w != input.w || guide.h != input.h {
        return Err(format!(
            "guided_filter: guide {}x{} vs input {}x{} dimension mismatch",
            guide.w, guide.h, input.w, input.h
        ));
    }
    if r == 0 {
        return Err("guided_filter: radius must be >= 1".to_string());
    }
    if !(eps.is_finite() && eps > 0.0) {
        return Err(format!(
            "guided_filter: eps must be finite and > 0 (got {eps})"
        ));
    }
    let (w, h) = (guide.w, guide.h);
    let n = w * h;
    let i64v: Vec<f64> = guide.data.iter().map(|&v| v as f64).collect();
    let p64v: Vec<f64> = input.data.iter().map(|&v| v as f64).collect();

    let mean_i = box_mean_f64(&i64v, w, h, r);
    let mean_p = box_mean_f64(&p64v, w, h, r);
    let ii: Vec<f64> = (0..n).map(|k| i64v[k] * i64v[k]).collect();
    let ip: Vec<f64> = (0..n).map(|k| i64v[k] * p64v[k]).collect();
    let corr_ii = box_mean_f64(&ii, w, h, r);
    let corr_ip = box_mean_f64(&ip, w, h, r);

    // Per-window linear model coefficients (he10 eq. 5–6).
    let mut a = vec![0f64; n];
    let mut b = vec![0f64; n];
    for k in 0..n {
        let var_i = corr_ii[k] - mean_i[k] * mean_i[k];
        let cov_ip = corr_ip[k] - mean_i[k] * mean_p[k];
        a[k] = cov_ip / (var_i + eps);
        b[k] = mean_p[k] - a[k] * mean_i[k];
    }
    // Per-pixel output from averaged coefficients (he10 eq. 8).
    let mean_a = box_mean_f64(&a, w, h, r);
    let mean_b = box_mean_f64(&b, w, h, r);
    let mut out = Plane::new(w, h);
    for k in 0..n {
        out.data[k] = (mean_a[k] * i64v[k] + mean_b[k]) as f32;
    }
    Ok(out)
}

/// Self-guided edge-preserving smoothing: `guided_filter(p, p, r, eps)`
/// (the he10 "edge-preserving filtering" special case, where
/// a = var/(var + eps) acts as an edge/flat selector).
pub fn guided_filter_self(input: &Plane, r: usize, eps: f64) -> Result<Plane, String> {
    guided_filter(input, input, r, eps)
}

/// Shadows/Highlights base/detail split (contract `guided_filter_split`):
/// `base` = self-guided filter of `luma`, `detail` = `luma - base`.
///
/// Design-around posture (register `P-9241093` memo: "base guided filter
/// only; claimed variance-gain video pipeline not implemented"): this is
/// a pure decomposition — the §17.2 tone-lift curves apply to `base` at
/// the O-TOOLS seam and `detail` is re-added unchanged; no variance-gain
/// stage exists on this path. Dossier defaults: r = 30 px at the 12 MP
/// reference scale (caller scales with document diagonal), eps = 0.01^2
/// on linear luma.
pub fn guided_split(luma: &Plane, r: usize, eps: f64) -> Result<(Plane, Plane), String> {
    let base = guided_filter_self(luma, r, eps)?;
    let mut detail = Plane::new(luma.w, luma.h);
    for k in 0..luma.data.len() {
        detail.data[k] = luma.data[k] - base.data[k];
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

    /// Vertical step edge with a flat value per side.
    fn step_plane(w: usize, h: usize, lo: f32, hi: f32) -> Plane {
        let mut p = Plane::new(w, h);
        for y in 0..h {
            for x in 0..w {
                p.data[y * w + x] = if x < w / 2 { lo } else { hi };
            }
        }
        p
    }

    /// Naive O(r^2) truncated-window box mean — the direct-sum oracle.
    fn naive_box_mean(src: &Plane, r: usize) -> Plane {
        let (w, h) = (src.w, src.h);
        let mut out = Plane::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let (mut s, mut c) = (0f64, 0f64);
                for yy in y.saturating_sub(r)..(y + r + 1).min(h) {
                    for xx in x.saturating_sub(r)..(x + r + 1).min(w) {
                        s += src.at(xx, yy) as f64;
                        c += 1.0;
                    }
                }
                out.data[y * w + x] = (s / c) as f32;
            }
        }
        out
    }

    #[test]
    fn box_mean_matches_naive_direct_sums() {
        // O(1) running-sum path vs naive O(r^2) direct sums, 1e-5 class.
        let p = random_plane(23, 17, 0xEA10_0001);
        for r in [1usize, 3, 8] {
            let fast = box_mean(&p, r);
            let slow = naive_box_mean(&p, r);
            let max = fast
                .data
                .iter()
                .zip(&slow.data)
                .map(|(a, b)| (a - b).abs())
                .fold(0f32, f32::max);
            assert!(max <= 1e-5, "r={r}: max box-mean deviation {max}");
        }
    }

    #[test]
    fn constant_image_is_identity_for_any_eps() {
        // var = 0, cov = 0 => a = 0, b = mean(p) = c => q = c, eps-independent.
        let c = 0.4375f32;
        let p = Plane::from_vec(19, 13, vec![c; 19 * 13]);
        for eps in [1e-6, 1e-2, 10.0] {
            let q = guided_filter_self(&p, 4, eps).unwrap();
            let max = q.data.iter().map(|v| (v - c).abs()).fold(0f32, f32::max);
            assert!(max <= 1e-6, "eps={eps}: max deviation from constant {max}");
        }
    }

    #[test]
    fn huge_eps_collapses_to_cascaded_box_mean() {
        // eps -> inf: a -> 0, b -> mean(p), q = mean(b) = box(box(p)).
        let p = random_plane(24, 20, 0xEA10_0002);
        let q = guided_filter_self(&p, 3, 1e9).unwrap();
        let bb = naive_box_mean(&naive_box_mean(&p, 3), 3);
        let max = q
            .data
            .iter()
            .zip(&bb.data)
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        assert!(max <= 1e-4, "max deviation from double box mean {max}");
    }

    #[test]
    fn edge_preservation_beats_plain_box_blur() {
        // Step edge, small eps: guided filter must track the step far more
        // faithfully than the box mean of the same radius (quantified).
        let p = step_plane(32, 16, 0.2, 0.8);
        let r = 4;
        let q = guided_filter_self(&p, r, 1e-4).unwrap();
        let blurred = naive_box_mean(&p, r);
        let err = |img: &Plane| {
            img.data
                .iter()
                .zip(&p.data)
                .map(|(a, b)| (a - b).abs() as f64)
                .sum::<f64>()
                / img.data.len() as f64
        };
        let (guided_err, box_err) = (err(&q), err(&blurred));
        assert!(
            guided_err < 0.2 * box_err,
            "guided mean |err| {guided_err} not << box mean |err| {box_err}"
        );
        // Far side of the edge stays essentially untouched.
        let far = q.at(31, 8);
        assert!(
            (far - 0.8).abs() < 0.01,
            "far-from-edge value drifted: {far}"
        );
    }

    #[test]
    fn self_guide_equals_gray_guide_on_same_input() {
        let p = random_plane(20, 15, 0xEA10_0003);
        let a = guided_filter_self(&p, 3, 1e-3).unwrap();
        let b = guided_filter(&p, &p, 3, 1e-3).unwrap();
        assert_eq!(
            a, b,
            "self-guide path must equal guide==input path bit-exactly"
        );
    }

    #[test]
    fn split_reconstructs_and_detail_is_zero_on_flat() {
        let p = random_plane(16, 16, 0xEA10_0004);
        let (base, detail) = guided_split(&p, 3, 1e-4).unwrap();
        for k in 0..p.data.len() {
            // detail = luma - base in f32; re-adding rounds by <= 1 ulp.
            assert!((base.data[k] + detail.data[k] - p.data[k]).abs() <= 1e-6);
        }
        let flat = Plane::from_vec(12, 12, vec![0.25; 144]);
        let (_, d) = guided_split(&flat, 4, 1e-4).unwrap();
        let max = d.data.iter().map(|v| v.abs()).fold(0f32, f32::max);
        assert!(max <= 1e-6, "flat-image detail not ~0: {max}");
    }

    #[test]
    fn validation_errors() {
        let p = random_plane(8, 8, 1);
        let q = random_plane(9, 8, 2);
        assert!(guided_filter(&p, &q, 2, 1e-3).is_err());
        assert!(guided_filter_self(&p, 0, 1e-3).is_err());
        assert!(guided_filter_self(&p, 2, 0.0).is_err());
        assert!(guided_filter_self(&p, 2, f64::NAN).is_err());
    }

    #[test]
    fn radius_at_least_image_extent_equals_global_stats() {
        // r >= max(w, h) - 1: every truncated window is the whole image,
        // so every window mean is the global mean and the output is the
        // single global linear model q = a*I + b with
        // a = var/(var + eps), b = mean*(1 - a). Two checks: (1) any two
        // over-sized radii are BIT-identical (the clamp makes them the
        // same computation); (2) the output matches the independently
        // computed global-statistics closed form.
        let p = random_plane(12, 9, 0xEA10_0005);
        let a = guided_filter_self(&p, 12, 1e-3).unwrap();
        let b = guided_filter_self(&p, 1000, 1e-3).unwrap();
        assert_eq!(a, b, "over-sized radii must clamp to the same windows");
        let n = (p.w * p.h) as f64;
        let mi = p.data.iter().map(|&v| v as f64).sum::<f64>() / n;
        let mii = p.data.iter().map(|&v| (v as f64) * (v as f64)).sum::<f64>() / n;
        let var = mii - mi * mi;
        let ga = var / (var + 1e-3);
        let gb = mi * (1.0 - ga);
        let worst = p
            .data
            .iter()
            .zip(&a.data)
            .map(|(&i, &q)| (q as f64 - (ga * i as f64 + gb)).abs())
            .fold(0f64, f64::max);
        assert!(worst <= 1e-6, "global-stat closed form deviation {worst}");
    }

    #[test]
    fn single_row_and_column_step_images() {
        // 1xN / Nx1 degenerate extents: the vertical (resp. horizontal)
        // box pass is width-1 and must be inert. A two-tone step with
        // small eps stays essentially the input (per-window linear fit is
        // near-exact on a two-value signal; measured max dev 3.5e-4, the
        // eps-driven shrink). Row and column give the same values by
        // symmetry of the separable passes.
        let vals: Vec<f32> = (0..12).map(|i| if i < 6 { 0.2 } else { 0.8 }).collect();
        let row = Plane::from_vec(12, 1, vals.clone());
        let col = Plane::from_vec(1, 12, vals.clone());
        let qr = guided_filter_self(&row, 2, 1e-4).unwrap();
        let qc = guided_filter_self(&col, 2, 1e-4).unwrap();
        // Index loop kept: the assertion references qr.data[i]/vals[i].
        #[allow(clippy::needless_range_loop)]
        for i in 0..12 {
            assert!(
                (qr.data[i] - vals[i]).abs() <= 1e-3,
                "row step deviated at {i}: {} vs {}",
                qr.data[i],
                vals[i]
            );
        }
        assert_eq!(qr.data, qc.data, "row/column symmetry broken");
    }

    #[test]
    fn tiny_eps_on_zero_variance_patch_has_no_division_blowup() {
        // eps = 0 is rejected by validation; the smallest admissible eps
        // class (1e-300, far below any float noise) on an exactly
        // constant image probes the var = 0 division: a = cov/(var + eps)
        // = 0/eps = 0 exactly (cov is exactly 0 in f64 on a constant
        // input), b = mean, output identical to the input and finite —
        // no 0/0, no denormal blowup.
        let c = 0.3f32;
        let p = Plane::from_vec(9, 7, vec![c; 63]);
        let q = guided_filter_self(&p, 3, 1e-300).unwrap();
        assert!(q.data.iter().all(|v| v.is_finite()));
        let max = q.data.iter().map(|v| (v - c).abs()).fold(0f32, f32::max);
        assert!(max <= 1e-6, "tiny-eps constant deviated by {max}");
    }

    #[test]
    fn constant_guide_joint_filter_is_cascaded_box_mean() {
        // Joint filtering with a CONSTANT guide (guide != input): var_I =
        // 0 so a = 0 and b = mean_w(p), hence q = mean(b) = box(box(p))
        // exactly — an independent closed-form oracle for the joint path
        // that a self-guided run cannot exercise (measured dev 6e-8).
        let p = random_plane(14, 11, 0xEA10_0006);
        let flat = Plane::from_vec(14, 11, vec![0.5; 14 * 11]);
        let q = guided_filter(&flat, &p, 3, 1e-4).unwrap();
        let bb = box_mean(&box_mean(&p, 3), 3);
        let max = q
            .data
            .iter()
            .zip(&bb.data)
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        assert!(max <= 1e-6, "constant-guide vs box(box) deviation {max}");
    }

    #[test]
    fn extreme_dynamic_range_is_scale_equivariant() {
        // Coefficient stability at 1e6 scale: the he10 model is exactly
        // scale-equivariant when eps scales with the squared signal
        // (var, cov ~ s^2 => a invariant, b ~ s), so
        // filter(s*p, eps*s^2) == s * filter(p, eps) up to f32 rounding
        // (measured 6e-8 after unscaling). Guards the f64 prefix-sum
        // pipeline against catastrophic cancellation at large magnitudes.
        let p = random_plane(12, 9, 0xEA10_0005);
        let s = 1e6f32;
        let big = Plane::from_vec(p.w, p.h, p.data.iter().map(|&v| v * s).collect());
        let qb = guided_filter_self(&big, 3, 1e-3 * (s as f64) * (s as f64)).unwrap();
        let qs = guided_filter_self(&p, 3, 1e-3).unwrap();
        assert!(qb.data.iter().all(|v| v.is_finite()));
        let rel = qb
            .data
            .iter()
            .zip(&qs.data)
            .map(|(&a, &b)| ((a / s) - b).abs())
            .fold(0f32, f32::max);
        assert!(rel <= 1e-5, "scale-equivariance deviation {rel}");
    }

    #[test]
    fn one_pixel_image_all_four_family_engines() {
        // Cross-module 1x1 probe of the whole edge-aware family (placed
        // here with the family's first module): guided, dtf (both
        // variants) and wls must be exact identities on a single pixel
        // (no neighbors, no smoothing possible); llf declares its >= 2x2
        // minimum and must return a clean Err, not panic.
        use crate::edge_aware::{dtf, llf, wls};
        let one = Plane::from_vec(1, 1, vec![0.42]);
        let g = guided_filter_self(&one, 3, 1e-3).unwrap();
        assert_eq!(g.data, vec![0.42], "guided 1x1 not identity");
        let n = dtf::domain_transform_nc(&one, &one, 10.0, 0.2, 3).unwrap();
        let r = dtf::domain_transform_rf(&one, &one, 10.0, 0.2, 3).unwrap();
        assert_eq!(n.data, vec![0.42], "dtf NC 1x1 not identity");
        assert_eq!(r.data, vec![0.42], "dtf RF 1x1 not identity");
        let u = wls::wls_smooth(&one, &one, 1.0, 1.2).unwrap();
        assert_eq!(u.data, vec![0.42], "wls 1x1 not identity");
        let remap = llf::LlfRemap {
            sigma_r: 0.2,
            detail_alpha: 0.5,
            edge_beta: 1.0,
        };
        assert!(
            llf::fast_local_laplacian(&one, &remap, 8, None).is_err(),
            "llf 1x1 must be a clean Err"
        );
        assert!(llf::local_laplacian_exact(&one, &remap, None).is_err());
    }

    #[test]
    fn determinism_two_runs_and_pinned_golden() {
        // Pinned from this crate's own runs AFTER the behavioral oracles
        // above were green (rule 8). Fixture: 32x32 SplitMix64 seed
        // 0xEA10_601D, r = 4, eps = 1e-3, self-guided.
        let p = random_plane(32, 32, 0xEA10_601D);
        let a = guided_filter_self(&p, 4, 1e-3).unwrap();
        let b = guided_filter_self(&p, 4, 1e-3).unwrap();
        assert_eq!(
            plane_hash(&a.data),
            plane_hash(&b.data),
            "two-run bit equality"
        );
        assert_eq!(
            plane_hash(&a.data),
            18421120086937190042,
            "pinned golden hash"
        );
    }
}
