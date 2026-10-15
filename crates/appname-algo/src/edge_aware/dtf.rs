// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Domain transform for edge-aware filtering — clean-room from Gastal &
//! Oliveira, "Domain Transform for Edge-Aware Image and Video
//! Processing", SIGGRAPH 2011 (contract basis key `gastal11`).
//!
//! Spec: ALGORITHMS.md §14.B (domain transform is the real-time preview
//! engine, iterations = 3); dossier `detail-local-laplacian` selected row
//! ("preview: domain transform NC 3 iters") and math row ("DT: iterated
//! 1D box over domain-transformed coords, σ_H=σ_s, NC kernel").
//!
//! Variants implemented and why: **NC** (normalized convolution — a box
//! kernel over the transformed domain) because the dossier selects it as
//! the preview engine, and **RF** (recursive filtering — the paper's
//! O(N) feedback-coefficient variant) because it is the paper's primary
//! recommendation for edge-aware smoothing and the cheapest full-quality
//! pass; the **IC** (interpolated convolution) variant is NOT implemented
//! — nothing in the contract, dossier, or ES selects it.
//!
//! Algorithm (gastal11): the 1-D domain transform of a row/column is the
//! cumulative arc length in the joint spatial/range metric,
//!   ct(x) = Σ_{k<=x} ( 1 + (σ_s/σ_r) * |g(k) - g(k-1)| )
//! (paper eq. 11, L1 isometric form) computed on the GUIDE `g` (joint
//! filtering). Each of the `iterations` passes runs a horizontal then a
//! vertical 1-D filter over the transformed coordinates with the
//! per-iteration kernel scale of eq. 14:
//!   σ_H_i = σ_H * sqrt(3) * 2^(N-i) / sqrt(4^N - 1),  σ_H = σ_s
//! NC uses a box kernel of radius sqrt(3)*σ_H_i over ct (normalized by
//! the in-window sample count); RF uses the feedback coefficient
//! a = exp(-sqrt(2)/σ_H_i) with per-sample weight a^d, d = ct(x)-ct(x-1)
//! (paper eq. 21), applied left-to-right then right-to-left.
//!
//! Determinism: single-threaded, fixed pass order (horizontal before
//! vertical, ascending iteration), f64 accumulation. `exp`/`powf` route
//! through platform libm — same-platform runs are bit-identical (the
//! golden claim below); cross-platform bit stability rests on libm, the
//! crate's documented posture since the deconv/mls modules.

use crate::plane::Plane;

/// The paper's per-iteration kernel-scale schedule (gastal11 eq. 14):
/// `σ_H_i = σ_H * sqrt(3) * 2^(N-i) / sqrt(4^N - 1)` for `i = 1..=N`,
/// with `σ_H = σ_s` (dossier math row).
pub fn sigma_h_schedule(sigma_s: f64, iterations: usize) -> Vec<f64> {
    let n = iterations as i32;
    (1..=iterations)
        .map(|i| sigma_s * 3f64.sqrt() * 2f64.powi(n - i as i32) / (4f64.powi(n) - 1.0).sqrt())
        .collect()
}

/// Per-row domain-transform derivative `d ct/dx = 1 + (σ_s/σ_r)|Δg|`
/// accumulated into absolute coordinates `ct[0] = 0, ct[x] = ct[x-1] + d`.
fn row_ct(guide_row: &[f64], ratio: f64, ct: &mut [f64]) {
    ct[0] = 0.0;
    for x in 1..guide_row.len() {
        ct[x] = ct[x - 1] + 1.0 + ratio * (guide_row[x] - guide_row[x - 1]).abs();
    }
}

/// One NC (normalized-convolution box) pass over a single line.
/// `ct` is monotone; window = samples with |ct(q) - ct(x)| <= radius,
/// evaluated with two monotone pointers + prefix sums (O(n) per line).
fn nc_line(vals: &mut [f64], ct: &[f64], radius: f64, prefix: &mut [f64]) {
    let n = vals.len();
    prefix[0] = 0.0;
    for x in 0..n {
        prefix[x + 1] = prefix[x] + vals[x];
    }
    let (mut lo, mut hi) = (0usize, 0usize);
    for x in 0..n {
        let (l, r) = (ct[x] - radius, ct[x] + radius);
        while ct[lo] < l {
            lo += 1;
        }
        if hi < x {
            hi = x;
        }
        while hi + 1 < n && ct[hi + 1] <= r {
            hi += 1;
        }
        vals[x] = (prefix[hi + 1] - prefix[lo]) / (hi + 1 - lo) as f64;
    }
}

/// One RF (recursive-filtering) pass over a single line: left-to-right
/// then right-to-left feedback with per-sample weight `a^d` (paper
/// eq. 21–23).
fn rf_line(vals: &mut [f64], ct: &[f64], sigma_h_i: f64) {
    let n = vals.len();
    let a = (-(2f64.sqrt()) / sigma_h_i).exp();
    for x in 1..n {
        let d = ct[x] - ct[x - 1];
        let w = a.powf(d);
        vals[x] += w * (vals[x - 1] - vals[x]);
    }
    for x in (0..n - 1).rev() {
        let d = ct[x + 1] - ct[x];
        let w = a.powf(d);
        vals[x] += w * (vals[x + 1] - vals[x]);
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Variant {
    Nc,
    Rf,
}

fn domain_transform(
    src: &Plane,
    guide: &Plane,
    sigma_s: f64,
    sigma_r: f64,
    iterations: usize,
    variant: Variant,
) -> Result<Plane, String> {
    if src.w == 0 || src.h == 0 {
        return Err("domain_transform: empty plane".to_string());
    }
    if src.w != guide.w || src.h != guide.h {
        return Err(format!(
            "domain_transform: src {}x{} vs guide {}x{} dimension mismatch",
            src.w, src.h, guide.w, guide.h
        ));
    }
    if !(sigma_s.is_finite() && sigma_s > 0.0) || !(sigma_r.is_finite() && sigma_r > 0.0) {
        return Err(format!(
            "domain_transform: sigma_s/sigma_r must be finite and > 0 (got {sigma_s}, {sigma_r})"
        ));
    }
    if iterations == 0 {
        return Err("domain_transform: iterations must be >= 1".to_string());
    }
    let (w, h) = (src.w, src.h);
    let ratio = sigma_s / sigma_r;
    let g64: Vec<f64> = guide.data.iter().map(|&v| v as f64).collect();
    let mut img: Vec<f64> = src.data.iter().map(|&v| v as f64).collect();

    // Precompute row/column transforms once (the guide is fixed).
    let mut ct_rows = vec![0f64; w * h];
    for y in 0..h {
        row_ct(
            &g64[y * w..(y + 1) * w],
            ratio,
            &mut ct_rows[y * w..(y + 1) * w],
        );
    }
    let mut ct_cols = vec![0f64; w * h]; // column-major: [x * h + y]
    let mut col = vec![0f64; h];
    let mut ct_col = vec![0f64; h];
    for x in 0..w {
        for y in 0..h {
            col[y] = g64[y * w + x];
        }
        row_ct(&col, ratio, &mut ct_col);
        ct_cols[x * h..(x + 1) * h].copy_from_slice(&ct_col);
    }

    let schedule = sigma_h_schedule(sigma_s, iterations);
    let mut line = vec![0f64; w.max(h)];
    let mut prefix = vec![0f64; w.max(h) + 1];
    for &sh in &schedule {
        // NC kernel radius (paper's box radius sqrt(3)*σ_H_i).
        let box_radius = 3f64.sqrt() * sh;
        // Horizontal pass.
        for y in 0..h {
            let row = &mut img[y * w..(y + 1) * w];
            match variant {
                Variant::Nc => nc_line(row, &ct_rows[y * w..(y + 1) * w], box_radius, &mut prefix),
                Variant::Rf => rf_line(row, &ct_rows[y * w..(y + 1) * w], sh),
            }
        }
        // Vertical pass.
        for x in 0..w {
            for y in 0..h {
                line[y] = img[y * w + x];
            }
            match variant {
                Variant::Nc => nc_line(
                    &mut line[..h],
                    &ct_cols[x * h..(x + 1) * h],
                    box_radius,
                    &mut prefix,
                ),
                Variant::Rf => rf_line(&mut line[..h], &ct_cols[x * h..(x + 1) * h], sh),
            }
            for y in 0..h {
                img[y * w + x] = line[y];
            }
        }
    }
    Ok(Plane::from_vec(
        w,
        h,
        img.iter().map(|&v| v as f32).collect(),
    ))
}

/// Dossier default iteration count for the preview engine.
pub const DTF_DEFAULT_ITERATIONS: usize = 3;

/// Domain transform, NC (normalized-convolution) variant — the dossier's
/// selected preview engine (3 iterations by default). Joint filtering:
/// `guide` supplies the domain metric (pass `src` again for self-guiding).
pub fn domain_transform_nc(
    src: &Plane,
    guide: &Plane,
    sigma_s: f64,
    sigma_r: f64,
    iterations: usize,
) -> Result<Plane, String> {
    domain_transform(src, guide, sigma_s, sigma_r, iterations, Variant::Nc)
}

/// Domain transform, RF (recursive-filtering) variant (paper eq. 21).
pub fn domain_transform_rf(
    src: &Plane,
    guide: &Plane,
    sigma_s: f64,
    sigma_r: f64,
    iterations: usize,
) -> Result<Plane, String> {
    domain_transform(src, guide, sigma_s, sigma_r, iterations, Variant::Rf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plane::{plane_hash, SplitMix64};

    fn random_plane(w: usize, h: usize, seed: u64) -> Plane {
        let mut rng = SplitMix64(seed);
        Plane::from_vec(w, h, (0..w * h).map(|_| rng.next_f64() as f32).collect())
    }

    /// Step edge + per-side noise: left ~0.2, right ~0.8, amplitude `amp`.
    fn noisy_step(w: usize, h: usize, amp: f32, seed: u64) -> Plane {
        let mut rng = SplitMix64(seed);
        let mut p = Plane::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let base = if x < w / 2 { 0.2 } else { 0.8 };
                p.data[y * w + x] = base + amp * (2.0 * rng.next_f64() as f32 - 1.0);
            }
        }
        p
    }

    fn side_variance(p: &Plane, x0: usize, x1: usize) -> f64 {
        let mut vals = Vec::new();
        for y in 0..p.h {
            for x in x0..x1 {
                vals.push(p.at(x, y) as f64);
            }
        }
        let mu = vals.iter().sum::<f64>() / vals.len() as f64;
        vals.iter().map(|v| (v - mu) * (v - mu)).sum::<f64>() / vals.len() as f64
    }

    #[test]
    fn schedule_matches_paper_formula() {
        // Independently computed values of eq. 14 for σ_s = 100, N = 3:
        // sqrt(3)*100 * {4, 2, 1} / sqrt(63).
        let s = sigma_h_schedule(100.0, 3);
        let k = 100.0 * 3f64.sqrt() / 63f64.sqrt();
        let expect = [4.0 * k, 2.0 * k, k];
        assert_eq!(s.len(), 3);
        for (a, e) in s.iter().zip(expect) {
            assert!((a - e).abs() < 1e-12, "schedule {a} vs closed form {e}");
        }
        // N = 1 collapses to σ_H * sqrt(3) * 1 / sqrt(3) = σ_s exactly.
        let one = sigma_h_schedule(7.5, 1);
        assert!((one[0] - 7.5).abs() < 1e-12);
    }

    #[test]
    fn constant_image_is_identity_both_variants() {
        let c = 0.625f32;
        let p = Plane::from_vec(17, 11, vec![c; 17 * 11]);
        for f in [domain_transform_nc, domain_transform_rf] {
            let q = f(&p, &p, 20.0, 0.3, 3).unwrap();
            let max = q.data.iter().map(|v| (v - c).abs()).fold(0f32, f32::max);
            assert!(max <= 1e-6, "constant image deviated by {max}");
        }
    }

    #[test]
    fn step_preserved_flats_smoothed() {
        // Small σ_r: the domain distance across the step is huge, so the
        // edge survives while in-region noise variance collapses.
        let p = noisy_step(40, 16, 0.05, 0xD7F0_0001);
        for f in [domain_transform_nc, domain_transform_rf] {
            let q = f(&p, &p, 8.0, 0.1, 3).unwrap();
            // Edge magnitude retained (>= 90% of the 0.6 step).
            let mut min_jump = f32::MAX;
            for y in 0..p.h {
                let jump = q.at(p.w / 2, y) - q.at(p.w / 2 - 1, y);
                min_jump = min_jump.min(jump);
            }
            assert!(min_jump > 0.54, "step jump collapsed to {min_jump}");
            // Flat-region variance reduced by >= 10x (quantified smoothing).
            let (v_in, v_out) = (
                side_variance(&p, 2, p.w / 2 - 2),
                side_variance(&q, 2, p.w / 2 - 2),
            );
            assert!(
                v_out < v_in / 10.0,
                "flat variance {v_out} not << input {v_in}"
            );
        }
    }

    #[test]
    fn joint_guide_transfers_edges() {
        // Filtering a smooth ramp with a step GUIDE must keep the output
        // discontinuous at the guide's edge column (joint filtering works).
        let (w, h) = (32usize, 8usize);
        let mut ramp = Plane::new(w, h);
        for y in 0..h {
            for x in 0..w {
                ramp.data[y * w + x] = x as f32 / (w - 1) as f32;
            }
        }
        let mut guide = Plane::new(w, h);
        for y in 0..h {
            for x in 0..w {
                guide.data[y * w + x] = if x < w / 2 { 0.0 } else { 1.0 };
            }
        }
        let q = domain_transform_rf(&ramp, &guide, 12.0, 0.05, 3).unwrap();
        let jump = q.at(w / 2, 4) - q.at(w / 2 - 1, 4);
        let interior = q.at(w / 4 + 1, 4) - q.at(w / 4, 4);
        assert!(
            jump > 10.0 * interior.abs(),
            "guide edge not transferred: jump {jump} vs interior slope {interior}"
        );
    }

    #[test]
    fn validation_errors() {
        let p = random_plane(8, 8, 1);
        let q = random_plane(7, 8, 2);
        assert!(domain_transform_nc(&p, &q, 10.0, 0.2, 3).is_err());
        assert!(domain_transform_nc(&p, &p, 0.0, 0.2, 3).is_err());
        assert!(domain_transform_nc(&p, &p, 10.0, -1.0, 3).is_err());
        assert!(domain_transform_nc(&p, &p, 10.0, 0.2, 0).is_err());
        assert!(domain_transform_rf(&p, &p, f64::NAN, 0.2, 3).is_err());
    }

    #[test]
    fn sigma_r_tiny_is_exact_identity_on_nonflat_input() {
        // σ_r -> 0 limit: the domain-metric ratio σ_s/σ_r explodes, so
        // ANY nonzero guide gradient puts adjacent samples at enormous
        // domain distance. NC: each window collapses to the pixel itself
        // (mean of one sample — exact). RF: a^d underflows to 0 exactly.
        // Both variants must return the input bit-near-exactly — no
        // smoothing across any gradient.
        let p = random_plane(20, 14, 0xD7F0_0002);
        let qn = domain_transform_nc(&p, &p, 10.0, 1e-9, 3).unwrap();
        let qr = domain_transform_rf(&p, &p, 10.0, 1e-9, 3).unwrap();
        let dev = |q: &Plane| {
            q.data
                .iter()
                .zip(&p.data)
                .map(|(a, b)| (a - b).abs())
                .fold(0f32, f32::max)
        };
        assert!(dev(&qn) <= 1e-6, "NC tiny-sigma_r deviated {}", dev(&qn));
        assert!(dev(&qr) <= 1e-6, "RF tiny-sigma_r deviated {}", dev(&qr));
    }

    #[test]
    fn sigma_r_huge_is_guide_independent_plain_smoothing() {
        // σ_r -> inf limit: ratio -> 0 and ct(x) -> x for EVERY guide, so
        // the filter degenerates to content-independent spatial smoothing
        // (the paper's plain-1D-kernel limit). Oracle: filtering the same
        // src with two completely different guides must give the same
        // result (measured dev 0), and it must actually smooth (variance
        // of a random plane collapses; measured 59x for NC).
        let p = random_plane(20, 14, 0xD7F0_0002);
        let g = random_plane(20, 14, 0xD7F0_0003);
        let var_of = |q: &Plane| {
            let m = q.data.iter().map(|&v| v as f64).sum::<f64>() / q.data.len() as f64;
            q.data.iter().map(|&v| (v as f64 - m).powi(2)).sum::<f64>() / q.data.len() as f64
        };
        for f in [domain_transform_nc, domain_transform_rf] {
            let qa = f(&p, &p, 5.0, 1e12, 3).unwrap();
            let qb = f(&p, &g, 5.0, 1e12, 3).unwrap();
            let dev = qa
                .data
                .iter()
                .zip(&qb.data)
                .map(|(a, b)| (a - b).abs())
                .fold(0f32, f32::max);
            assert!(dev <= 1e-6, "huge-sigma_r output depends on guide: {dev}");
            assert!(
                var_of(&qa) < var_of(&p) / 20.0,
                "huge-sigma_r did not smooth: {} vs {}",
                var_of(&qa),
                var_of(&p)
            );
        }
    }

    #[test]
    fn single_row_and_column_transpose_symmetry() {
        // Nx1 row (vertical passes are width-1 no-ops) vs 1xN column
        // (horizontal passes are no-ops): the same 1-D data must produce
        // BIT-identical outputs — the per-line kernels and the ct
        // construction are one code path (row_ct) in both orientations.
        let vals = vec![0.1f32, 0.7, 0.3, 0.9, 0.2, 0.8, 0.4, 0.6];
        let row = Plane::from_vec(8, 1, vals.clone());
        let col = Plane::from_vec(1, 8, vals.clone());
        for f in [domain_transform_nc, domain_transform_rf] {
            let qr = f(&row, &row, 4.0, 0.5, 3).unwrap();
            let qc = f(&col, &col, 4.0, 0.5, 3).unwrap();
            assert_eq!(qr.data, qc.data, "row/column transpose symmetry broken");
            assert!(qr.data.iter().all(|v| v.is_finite()));
        }
    }

    #[test]
    fn schedule_total_variance_is_iteration_invariant() {
        // Eq. 14 exists precisely so that N iterations sum to the SAME
        // total kernel variance: Σ_i σ_H_i^2 = σ_H^2 for every N
        // (algebraic identity: Σ 3·4^{N-i}/(4^N-1) = 1). This is why
        // "more iterations => monotonically more smoothing" would be a
        // FALSE oracle — iterations trade box-kernel artifacts for a
        // Gaussian-like profile at constant total smoothing (measured
        // flat-noise variance reduction: NC 2700x at 1 iter vs 1760x at
        // 3; RF 526x vs 1124x — not monotone, not claimed). The honest
        // pins: the exact variance identity, plus "every iteration count
        // smooths strongly" (>= 100x on flat noise) below.
        for n in [1usize, 2, 3, 5, 8] {
            let s = sigma_h_schedule(10.0, n);
            let ss: f64 = s.iter().map(|v| v * v).sum();
            assert!(
                (ss - 100.0).abs() < 1e-9,
                "N={n}: total variance {ss} != sigma_s^2"
            );
        }
        let mut rng = SplitMix64(0xD7F0_0004);
        let mut noisy = Plane::new(24, 16);
        for k in 0..24 * 16 {
            noisy.data[k] = 0.5 + 0.05 * (2.0 * rng.next_f64() as f32 - 1.0);
        }
        let var_of = |q: &Plane| {
            let m = q.data.iter().map(|&v| v as f64).sum::<f64>() / q.data.len() as f64;
            q.data.iter().map(|&v| (v as f64 - m).powi(2)).sum::<f64>() / q.data.len() as f64
        };
        let v0 = var_of(&noisy);
        for iters in [1usize, 3] {
            for f in [domain_transform_nc, domain_transform_rf] {
                let q = f(&noisy, &noisy, 8.0, 10.0, iters).unwrap();
                assert!(
                    var_of(&q) < v0 / 100.0,
                    "iters={iters}: variance {} not << {v0}",
                    var_of(&q)
                );
            }
        }
    }

    #[test]
    fn nan_policy_pinned_current_behavior() {
        // Documents current behavior, not a contract. NaN in the GUIDE
        // poisons the cumulative ct from that point on; NaN comparisons
        // are all false, so nc_line's monotone pointers simply stop
        // moving (no out-of-bounds, no hang) and the NC output is
        // NaN-free; RF's a^NaN weights propagate NaN through the whole
        // feedback line (measured: every output sample NaN). NaN in the
        // SRC spreads through NC windows that contain it (partial NaN
        // output) and through the whole RF line. All four combinations
        // complete, and replay bit-identically (the pin that matters
        // for D-48).
        let p = random_plane(20, 14, 0xD7F0_0002);
        let mut poisoned = p.clone();
        poisoned.data[5 * 20 + 7] = f32::NAN;
        // (variant, src, guide, expected NaN presence in output)
        type NanCase<'a> = (&'a str, &'a Plane, &'a Plane, fn(usize, usize) -> bool);
        let cases: [NanCase; 4] = [
            ("nc/nan-guide", &p, &poisoned, |nan, _| nan == 0),
            ("nc/nan-src", &poisoned, &p, |nan, n| nan > 0 && nan < n),
            ("rf/nan-guide", &p, &poisoned, |nan, n| nan == n),
            ("rf/nan-src", &poisoned, &p, |nan, n| nan == n),
        ];
        for (name, src, guide, expect) in cases {
            let f = if name.starts_with("nc") {
                domain_transform_nc
            } else {
                domain_transform_rf
            };
            let a = f(src, guide, 10.0, 0.2, 3).unwrap();
            let b = f(src, guide, 10.0, 0.2, 3).unwrap();
            assert_eq!(
                plane_hash(&a.data),
                plane_hash(&b.data),
                "{name}: NaN replay determinism broken"
            );
            let nan = a.data.iter().filter(|v| v.is_nan()).count();
            assert!(
                expect(nan, a.data.len()),
                "{name}: pinned NaN propagation shape changed ({nan}/{} NaN)",
                a.data.len()
            );
        }
    }

    #[test]
    fn determinism_two_runs_and_pinned_goldens() {
        // Pinned from this crate's own runs AFTER the behavioral oracles
        // above were green (rule 8). Fixture: 32x32 SplitMix64 seed
        // 0xD7F0_601D, σ_s = 10, σ_r = 0.25, dossier default 3 iterations.
        let p = random_plane(32, 32, 0xD7F0_601D);
        let nc1 = domain_transform_nc(&p, &p, 10.0, 0.25, DTF_DEFAULT_ITERATIONS).unwrap();
        let nc2 = domain_transform_nc(&p, &p, 10.0, 0.25, DTF_DEFAULT_ITERATIONS).unwrap();
        let rf1 = domain_transform_rf(&p, &p, 10.0, 0.25, DTF_DEFAULT_ITERATIONS).unwrap();
        let rf2 = domain_transform_rf(&p, &p, 10.0, 0.25, DTF_DEFAULT_ITERATIONS).unwrap();
        assert_eq!(
            plane_hash(&nc1.data),
            plane_hash(&nc2.data),
            "NC two-run bit equality"
        );
        assert_eq!(
            plane_hash(&rf1.data),
            plane_hash(&rf2.data),
            "RF two-run bit equality"
        );
        assert_eq!(
            plane_hash(&nc1.data),
            3950490525662773995,
            "pinned NC golden hash"
        );
        assert_eq!(
            plane_hash(&rf1.data),
            1448793666933048533,
            "pinned RF golden hash"
        );
    }
}
