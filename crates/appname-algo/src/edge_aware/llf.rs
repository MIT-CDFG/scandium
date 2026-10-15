// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Local Laplacian filtering — clean-room from Paris, Hasinoff, Kautz,
//! "Local Laplacian Filters: Edge-aware Image Processing with a Laplacian
//! Pyramid", SIGGRAPH 2011 (`paris11`), and Aubry, Paris, Hasinoff,
//! Kautz, Durand, "Fast Local Laplacian Filters: Theory and
//! Applications", ACM TOG 2014 (`aubry14`).
//!
//! Spec: ALGORITHMS.md §14.B [NORMATIVE] — fast local Laplacian drives
//! Clarity (mid-tone local contrast) and Texture (fine-scale,
//! level-limited); full LLF semantics preserved at quality `best`.
//! Dossier `detail-local-laplacian`: remap "r(i)=i+σ_r·smoothgain(i−g,
//! clarity/texture curves)", fast variant "M=14 discretized g levels,
//! interpolate coefficients", complexity "fast-LLF O(N·M)". Contract
//! entry: primary `fast_local_laplacian`, level L2, basis
//! [aubry14, paris11], `fto_flagship: true` — this area is routed to the
//! professional pre-1.0 FTO scan (ALGORITHMS.md §26 row 1). The
//! no-patent-found finding is ALGORITHMS.md §14.C's (the patent register
//! itself carries no local-Laplacian entry): "no patents found across
//! 10+ targeted searches (local Laplacian explicitly: no Adobe patent
//! located; four searches on assignees+phrases)" — the L2
//! `UNVERIFIED_NEGATIVE` class (machine field).
//!
//! Rights route: the authors' MATLAB demo carries NO license grant
//! (census B2-llf-author-code.yaml, verdict none-found) and the ximgproc
//! census row records NO local-Laplacian implementation at the pinned
//! commit — this module is from the two papers only; no reference code
//! was fetched or consulted.
//!
//! Algorithm. paris11 framework: for output Laplacian coefficient
//! (l, x, y), take the reference g0 = G_l(x, y) from the input's Gaussian
//! pyramid, remap the input with the pointwise r(., g0), build its
//! Laplacian pyramid, and keep coefficient (l, x, y); the coarsest level
//! is the input's own Gaussian residual; collapse the assembled pyramid.
//! [`local_laplacian_exact`] implements exactly that (the `best`/oracle
//! path, O(N^2)-class). aubry14 fast approximation
//! ([`fast_local_laplacian`]): discretize g0 into M reference values
//! spanning the input range, build the M remapped Laplacian pyramids
//! (O(N·M)), and per coefficient linearly interpolate between the two
//! pyramids whose reference values bracket G_l(x, y).
//!
//! Remap (dossier additive form, algebraically paris11's §5 detail/edge
//! point-wise functions): with Δ = i − g, t = |Δ|/σ_r,
//!   t <= 1:  r(i) = i + sign(Δ)·σ_r·(t^α − t)        (detail band)
//!   t >  1:  r(i) = i + sign(Δ)·(β − 1)·(|Δ| − σ_r)  (edge band)
//! i.e. `smoothgain` = f_d(t) − t inside the detail band with paris11's
//! f_d(t) = t^α (α < 1 boosts detail, α > 1 smooths, α = 1 identity) and
//! f_e(a) = β·a on edges (β = 1 leaves edges untouched, the pure
//! detail-manipulation setting; β < 1 compresses tone, β > 1 expands).
//! The Clarity/Texture slider -> (σ_r, α, β, level-range) curves are the
//! O-TOOLS consumer's mapping (dossier "clarity/texture curves"), not
//! fixed here.
//!
//! Pyramid substrate: crate-internal reuse of [`crate::fusion`]'s
//! Burt–Adelson pyramid functions (the BSD-2-Clause exposure-fusion port
//! already in tree — reuse, not a new acquisition; its notice is already
//! carried). The LLF logic itself is from the papers.
//!
//! Determinism: single-threaded fixed order (levels ascending, row-major
//! pixels, reference levels ascending), f64 remap arithmetic; `powf`
//! routes through platform libm (crate posture per mls/deconv docs) —
//! same-platform two-run bit equality is asserted; the α = 1 path uses no
//! libm and reconstructs the input exactly up to pyramid round-trip
//! float error.

use crate::fusion::{
    gaussian_pyramid, laplacian_pyramid, pyramid_levels, reconstruct_laplacian_pyramid,
};
use crate::plane::Plane;

/// Dossier default reference-level count for the fast path ("M=14
/// discretized g levels").
pub const LLF_DEFAULT_M: usize = 14;

/// Remapping-function parameters (paris11 §5 detail/edge manipulation in
/// the dossier's additive `smoothgain` form).
#[derive(Clone, Copy, Debug)]
pub struct LlfRemap {
    /// Detail/edge threshold σ_r (> 0): intensity differences below σ_r
    /// are detail, above are edges.
    pub sigma_r: f64,
    /// Detail exponent α (> 0): f_d(t) = t^α. α < 1 boosts detail,
    /// α > 1 smooths it, α = 1 is the identity on the detail band.
    pub detail_alpha: f64,
    /// Edge factor β (>= 0): f_e(a) = β·a. β = 1 leaves edges untouched.
    pub edge_beta: f64,
}

impl LlfRemap {
    /// r(i) around reference g (f64; the additive form from the module
    /// doc). α = 1 and β = 1 make this the exact identity.
    #[inline]
    fn remap(&self, i: f64, g: f64) -> f64 {
        let delta = i - g;
        let mag = delta.abs();
        let sign = if delta >= 0.0 { 1.0 } else { -1.0 };
        let t = mag / self.sigma_r;
        if t <= 1.0 {
            // Exact-identity fast path for α = 1 (no libm, bit-clean).
            if self.detail_alpha == 1.0 {
                i
            } else {
                i + sign * self.sigma_r * (t.powf(self.detail_alpha) - t)
            }
        } else {
            i + sign * (self.edge_beta - 1.0) * (mag - self.sigma_r)
        }
    }

    fn validate(&self) -> Result<(), String> {
        if !(self.sigma_r.is_finite() && self.sigma_r > 0.0) {
            return Err(format!(
                "llf: sigma_r must be finite and > 0 (got {})",
                self.sigma_r
            ));
        }
        if !(self.detail_alpha.is_finite() && self.detail_alpha > 0.0) {
            return Err(format!(
                "llf: detail_alpha must be finite and > 0 (got {})",
                self.detail_alpha
            ));
        }
        if !(self.edge_beta.is_finite() && self.edge_beta >= 0.0) {
            return Err(format!(
                "llf: edge_beta must be finite and >= 0 (got {})",
                self.edge_beta
            ));
        }
        Ok(())
    }
}

/// Remap the whole plane around a single reference value.
fn remap_plane(src: &Plane, g: f64, r: &LlfRemap) -> Plane {
    let mut out = Plane::new(src.w, src.h);
    for k in 0..src.data.len() {
        out.data[k] = r.remap(src.data[k] as f64, g) as f32;
    }
    out
}

fn validate_common(src: &Plane, remap: &LlfRemap, nlev: Option<usize>) -> Result<usize, String> {
    if src.w < 2 || src.h < 2 {
        return Err(format!(
            "llf: plane must be at least 2x2 (got {}x{})",
            src.w, src.h
        ));
    }
    remap.validate()?;
    let max_lev = pyramid_levels(src.w, src.h);
    let nlev = nlev.unwrap_or(max_lev);
    if nlev < 2 || nlev > max_lev {
        return Err(format!("llf: nlev must be in 2..={max_lev} (got {nlev})"));
    }
    Ok(nlev)
}

/// Exact local Laplacian filter (paris11) — the per-pixel reference path
/// (quality `best`, and the convergence oracle for the fast path).
/// O(N) pyramid builds per pixel: use on small regions/fixtures or where
/// quality outranks time. `nlev = None` uses the full pyramid depth.
pub fn local_laplacian_exact(
    src: &Plane,
    remap: &LlfRemap,
    nlev: Option<usize>,
) -> Result<Plane, String> {
    let nlev = validate_common(src, remap, nlev)?;
    let gauss = gaussian_pyramid(src, nlev);
    // Output pyramid: remapped Laplacian coefficients per (l, x, y);
    // coarsest = input's Gaussian residual (paris11 §4).
    let mut out_pyr: Vec<Plane> = (0..nlev - 1)
        .map(|l| Plane::new(gauss[l].w, gauss[l].h))
        .collect();
    for l in 0..nlev - 1 {
        let (lw, lh) = (gauss[l].w, gauss[l].h);
        for y in 0..lh {
            for x in 0..lw {
                let g0 = gauss[l].at(x, y) as f64;
                let remapped = remap_plane(src, g0, remap);
                // Levels 0..=l+1 of the remapped pyramid are enough for
                // coefficient l, but the crate's pyramid builder returns
                // whole pyramids; build l+2 levels only.
                let lap = laplacian_pyramid(&remapped, l + 2);
                out_pyr[l].data[y * lw + x] = lap[l].at(x, y);
            }
        }
    }
    out_pyr.push(gauss[nlev - 1].clone());
    Ok(reconstruct_laplacian_pyramid(&out_pyr))
}

/// Fast local Laplacian filter (aubry14) — the contract PRIMARY
/// (`fast_local_laplacian`, machine flag `fto_flagship: true` in the
/// contract entry). `m_levels` discretized reference values (dossier
/// default [`LLF_DEFAULT_M`] = 14), `nlev = None` uses full pyramid
/// depth.
pub fn fast_local_laplacian(
    src: &Plane,
    remap: &LlfRemap,
    m_levels: usize,
    nlev: Option<usize>,
) -> Result<Plane, String> {
    let nlev = validate_common(src, remap, nlev)?;
    if m_levels < 2 {
        return Err(format!("llf: m_levels must be >= 2 (got {m_levels})"));
    }
    let gauss = gaussian_pyramid(src, nlev);

    // Reference values span the input's actual range (aubry14 samples the
    // intensity range uniformly). Degenerate (constant) input: remap is
    // identity around its own value — return the input unchanged.
    let (mut lo, mut hi) = (f64::MAX, f64::MIN);
    for &v in &src.data {
        lo = lo.min(v as f64);
        hi = hi.max(v as f64);
    }
    let span = hi - lo;
    // Deliberate negated-comparison shape: also catches NaN.
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    if !(span > 1e-12) {
        return Ok(src.clone());
    }

    // M remapped Laplacian pyramids, references ascending (fixed order).
    let refs: Vec<f64> = (0..m_levels)
        .map(|m| lo + span * m as f64 / (m_levels - 1) as f64)
        .collect();
    let pyramids: Vec<Vec<Plane>> = refs
        .iter()
        .map(|&g| laplacian_pyramid(&remap_plane(src, g, remap), nlev))
        .collect();

    // Interpolate coefficients between the two bracketing references.
    let scale = (m_levels - 1) as f64 / span;
    let mut out_pyr: Vec<Plane> = Vec::with_capacity(nlev);
    for l in 0..nlev - 1 {
        let (lw, lh) = (gauss[l].w, gauss[l].h);
        let mut lev = Plane::new(lw, lh);
        for k in 0..lw * lh {
            let g = (gauss[l].data[k] as f64).clamp(lo, hi);
            let t = (g - lo) * scale;
            let m0 = (t.floor() as usize).min(m_levels - 2);
            let f = t - m0 as f64;
            let c0 = pyramids[m0][l].data[k] as f64;
            let c1 = pyramids[m0 + 1][l].data[k] as f64;
            lev.data[k] = ((1.0 - f) * c0 + f * c1) as f32;
        }
        out_pyr.push(lev);
    }
    out_pyr.push(gauss[nlev - 1].clone());
    Ok(reconstruct_laplacian_pyramid(&out_pyr))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plane::{plane_hash, SplitMix64};

    /// Step edge (0.2 | 0.8) with a fine sinusoidal texture of amplitude
    /// `amp` on both sides — the halo/contrast fixture.
    fn textured_step(w: usize, h: usize, amp: f32) -> Plane {
        let mut p = Plane::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let base = if x < w / 2 { 0.2 } else { 0.8 };
                let tex = amp * ((x as f32) * 1.9).sin() * ((y as f32) * 1.7).cos();
                p.data[y * w + x] = base + tex;
            }
        }
        p
    }

    fn random_plane(w: usize, h: usize, seed: u64) -> Plane {
        let mut rng = SplitMix64(seed);
        Plane::from_vec(w, h, (0..w * h).map(|_| rng.next_f64() as f32).collect())
    }

    fn max_abs_diff(a: &Plane, b: &Plane) -> f32 {
        a.data
            .iter()
            .zip(&b.data)
            .map(|(x, y)| (x - y).abs())
            .fold(0f32, f32::max)
    }

    /// Mean local contrast: mean |v - box3(v)| (detail-energy metric).
    fn local_contrast(p: &Plane) -> f64 {
        let mut acc = 0f64;
        for y in 0..p.h as isize {
            for x in 0..p.w as isize {
                let mut s = 0f64;
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        s += p.at_clamped(x + dx, y + dy) as f64;
                    }
                }
                acc += (p.at_clamped(x, y) as f64 - s / 9.0).abs();
            }
        }
        acc / (p.w * p.h) as f64
    }

    /// Test-only unsharp mask: out = in + gain * (in - box3(in)).
    fn unsharp(p: &Plane, gain: f32) -> Plane {
        let mut out = Plane::new(p.w, p.h);
        for y in 0..p.h as isize {
            for x in 0..p.w as isize {
                let mut s = 0f64;
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        s += p.at_clamped(x + dx, y + dy) as f64;
                    }
                }
                let v = p.at_clamped(x, y);
                out.data[y as usize * p.w + x as usize] = v + gain * (v - (s / 9.0) as f32);
            }
        }
        out
    }

    const IDENTITY: LlfRemap = LlfRemap {
        sigma_r: 0.2,
        detail_alpha: 1.0,
        edge_beta: 1.0,
    };

    const BOOST: LlfRemap = LlfRemap {
        sigma_r: 0.2,
        detail_alpha: 0.5,
        edge_beta: 1.0,
    };

    #[test]
    fn identity_remap_reconstructs_input_both_paths() {
        // α = β = 1 makes r(i) = i exactly; the result must match the
        // input within the pyramid decompose/collapse round-trip bound.
        let p = random_plane(32, 32, 0x11F0_0001);
        let fast = fast_local_laplacian(&p, &IDENTITY, 8, None).unwrap();
        assert!(
            max_abs_diff(&fast, &p) <= 1e-5,
            "fast identity round-trip deviation {}",
            max_abs_diff(&fast, &p)
        );
        let small = random_plane(16, 16, 0x11F0_0002);
        let exact = local_laplacian_exact(&small, &IDENTITY, None).unwrap();
        assert!(
            max_abs_diff(&exact, &small) <= 1e-5,
            "exact identity round-trip deviation {}",
            max_abs_diff(&exact, &small)
        );
    }

    #[test]
    fn detail_boost_raises_local_contrast() {
        let p = textured_step(32, 32, 0.04);
        let q = fast_local_laplacian(&p, &BOOST, LLF_DEFAULT_M, None).unwrap();
        let (before, after) = (local_contrast(&p), local_contrast(&q));
        assert!(
            after > 1.25 * before,
            "detail boost did not raise local contrast: {before} -> {after}"
        );
    }

    #[test]
    fn detail_boost_bounds_overshoot_vs_unsharp_mask() {
        // Halo metric: excursion beyond the input's own range at a step
        // edge. The LLF boost leaves |Δ| > σ_r untouched (β = 1), so its
        // overshoot stays within the boosted-texture bound; an unsharp
        // mask with comparable mid-band gain halos across the edge.
        let p = textured_step(32, 32, 0.04);
        let llf = fast_local_laplacian(&p, &BOOST, LLF_DEFAULT_M, None).unwrap();
        // Match mid-band amplification: pick USM gain so its local
        // contrast is >= the LLF result's (same-or-more detail energy).
        let usm = unsharp(&p, 1.4);
        assert!(
            local_contrast(&usm) >= local_contrast(&llf),
            "USM comparison underpowered: {} < {}",
            local_contrast(&usm),
            local_contrast(&llf)
        );
        let (in_lo, in_hi) = p
            .data
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
        let overshoot = |img: &Plane| {
            img.data.iter().fold(0f32, |m, &v| {
                m.max((in_lo - v).max(0.0)).max((v - in_hi).max(0.0))
            })
        };
        let (o_llf, o_usm) = (overshoot(&llf), overshoot(&usm));
        assert!(
            o_llf < 0.5 * o_usm,
            "LLF overshoot {o_llf} not well below USM overshoot {o_usm}"
        );
        // Absolute halo bound: bounded by the boosted texture band, far
        // below the USM edge halo scale.
        assert!(o_llf <= 0.08, "LLF absolute overshoot {o_llf} above bound");
    }

    #[test]
    fn fast_path_converges_to_exact_as_m_grows() {
        let p = textured_step(16, 16, 0.04);
        let exact = local_laplacian_exact(&p, &BOOST, Some(3)).unwrap();
        let errs: Vec<f32> = [4usize, 8, 16]
            .iter()
            .map(|&m| {
                let fast = fast_local_laplacian(&p, &BOOST, m, Some(3)).unwrap();
                max_abs_diff(&fast, &exact)
            })
            .collect();
        assert!(
            errs[1] < errs[0] && errs[2] < errs[1],
            "errors not monotonically decreasing: {errs:?}"
        );
        // Stated bound: the α = 0.5 remap has a sqrt kink at Δ = 0, so
        // level interpolation converges at ~O(sqrt(spacing)) near the
        // kink, not O(spacing); 0.04 expresses that rate for M = 16 over
        // this fixture's intensity span (empirical 0.0248).
        assert!(
            errs[2] <= 0.04,
            "M=16 error {} above stated bound ({errs:?})",
            errs[2]
        );
    }

    #[test]
    fn constant_input_is_returned_unchanged() {
        let p = Plane::from_vec(16, 16, vec![0.5; 256]);
        let q = fast_local_laplacian(&p, &BOOST, 8, None).unwrap();
        assert_eq!(p, q, "constant input must pass through bit-exactly");
    }

    #[test]
    fn validation_errors() {
        let p = random_plane(16, 16, 1);
        let tiny = random_plane(1, 8, 2);
        assert!(fast_local_laplacian(&tiny, &BOOST, 8, None).is_err());
        assert!(fast_local_laplacian(&p, &BOOST, 1, None).is_err());
        assert!(fast_local_laplacian(&p, &BOOST, 8, Some(1)).is_err());
        assert!(fast_local_laplacian(&p, &BOOST, 8, Some(99)).is_err());
        let bad = LlfRemap {
            sigma_r: 0.0,
            ..BOOST
        };
        assert!(fast_local_laplacian(&p, &bad, 8, None).is_err());
        let bad = LlfRemap {
            detail_alpha: -1.0,
            ..BOOST
        };
        assert!(fast_local_laplacian(&p, &bad, 8, None).is_err());
        let bad = LlfRemap {
            edge_beta: f64::NAN,
            ..BOOST
        };
        assert!(fast_local_laplacian(&p, &bad, 8, None).is_err());
    }

    #[test]
    fn degenerate_sizes_error_cleanly_and_4x4_is_smallest_working() {
        // pyramid_levels(w, h) = floor(log2(min(w, h))), and nlev must be
        // >= 2 — so 2x2 and 3x3 (max_lev = 1) can never host a two-level
        // pyramid and must return the nlev-range Err (documents current
        // behavior: the validate_common 2x2 size gate passes but the
        // level gate rejects; not a panic). Nx1/1xN fail the size gate.
        // 4x4 (max_lev = 2) is the smallest working input: both paths
        // run, agree with each other closely, and replay bit-identically.
        for (w, h) in [(2usize, 2usize), (3, 3), (2, 16)] {
            let p = random_plane(w, h, 0x11F0_0003);
            let e = fast_local_laplacian(&p, &BOOST, 8, None);
            assert!(e.is_err(), "{w}x{h} must Err (got Ok)");
            assert!(
                e.unwrap_err().contains("nlev"),
                "{w}x{h}: expected the nlev-range error"
            );
            assert!(local_laplacian_exact(&p, &BOOST, None).is_err());
        }
        for (w, h) in [(8usize, 1usize), (1usize, 8usize)] {
            let p = random_plane(w, h, 0x11F0_0004);
            assert!(fast_local_laplacian(&p, &BOOST, 8, None).is_err());
        }
        let p = random_plane(4, 4, 0x11F0_0005);
        let fast = fast_local_laplacian(&p, &BOOST, 8, None).unwrap();
        let exact = local_laplacian_exact(&p, &BOOST, None).unwrap();
        assert!(fast.data.iter().all(|v| v.is_finite()));
        assert!(exact.data.iter().all(|v| v.is_finite()));
        let rerun = fast_local_laplacian(&p, &BOOST, 8, None).unwrap();
        assert_eq!(plane_hash(&fast.data), plane_hash(&rerun.data));
    }

    #[test]
    fn sigma_r_tiny_with_unit_beta_is_identity() {
        // σ_r -> 0 with β = 1: every |Δ| > σ_r lands in the edge band
        // where f_e is the identity at β = 1, and the vanishing detail
        // band contributes nothing — the remap is the identity, so the
        // output must match the input within the pyramid round-trip
        // bound (measured 6e-8).
        let p = random_plane(16, 16, 0x11F0_0006);
        let r = LlfRemap {
            sigma_r: 1e-12,
            detail_alpha: 0.5,
            edge_beta: 1.0,
        };
        let q = fast_local_laplacian(&p, &r, 8, None).unwrap();
        assert!(
            max_abs_diff(&q, &p) <= 1e-5,
            "tiny-sigma_r identity deviation {}",
            max_abs_diff(&q, &p)
        );
    }

    #[test]
    fn sigma_r_huge_pinned_current_behavior() {
        // Documents current behavior, not a contract. σ_r = 1e9 with
        // α = 0.5 puts every Δ deep inside the detail band where the
        // boost term is sign(Δ)·(sqrt(σ_r·|Δ|) − |Δ|) — amplification
        // GROWS with σ_r without bound (measured output range ≈ ±2.3e4
        // on a unit-range input). The honest pins: output finite (no
        // NaN/inf from powf), two-run bit determinism, and the α = 1
        // short-circuit stays an exact identity at the same σ_r.
        let p = random_plane(16, 16, 0x11F0_0007);
        let boost = LlfRemap {
            sigma_r: 1e9,
            detail_alpha: 0.5,
            edge_beta: 1.0,
        };
        let a = fast_local_laplacian(&p, &boost, 8, None).unwrap();
        assert!(
            a.data.iter().all(|v| v.is_finite()),
            "huge-sigma_r non-finite"
        );
        let b = fast_local_laplacian(&p, &boost, 8, None).unwrap();
        assert_eq!(plane_hash(&a.data), plane_hash(&b.data));
        let ident = LlfRemap {
            sigma_r: 1e9,
            detail_alpha: 1.0,
            edge_beta: 1.0,
        };
        let q = fast_local_laplacian(&p, &ident, 8, None).unwrap();
        assert!(
            max_abs_diff(&q, &p) <= 1e-5,
            "alpha=1 at huge sigma_r deviated {}",
            max_abs_diff(&q, &p)
        );
    }

    #[test]
    fn two_step_fixture_bounds_interpolation_overshoot_and_monotonicity() {
        // Crafted two-step fixture (plateaus 0.1 | 0.5 | 0.9, monotone
        // in x): every plateau is flat (no detail to boost) and β = 1
        // leaves both edges alone, so the ideal output is the input.
        // The fast path's level interpolation may ripple near the edges;
        // stated bounds (measured 3.4e-3 overshoot, 3.8e-3 worst local
        // decrease at M = 14): excursion beyond the input range <= 0.01
        // — an order below the σ_r·max(t^α − t) = 0.05 detail-band cap —
        // and per-row monotonicity preserved up to the same 0.01.
        let (w, h) = (48usize, 8usize);
        let mut p = Plane::new(w, h);
        for y in 0..h {
            for x in 0..w {
                p.data[y * w + x] = if x < w / 3 {
                    0.1
                } else if x < 2 * w / 3 {
                    0.5
                } else {
                    0.9
                };
            }
        }
        let q = fast_local_laplacian(&p, &BOOST, LLF_DEFAULT_M, None).unwrap();
        let over = q
            .data
            .iter()
            .map(|&v| (0.1 - v).max(v - 0.9).max(0.0))
            .fold(0f32, f32::max);
        assert!(over <= 0.01, "overshoot {over} beyond stated bound");
        let mut worst_dec = 0f32;
        for y in 0..h {
            for x in 0..w - 1 {
                worst_dec = worst_dec.max(q.at(x, y) - q.at(x + 1, y));
            }
        }
        assert!(
            worst_dec <= 0.01,
            "monotonicity violated by {worst_dec} (stated bound 0.01)"
        );
    }

    #[test]
    fn determinism_two_runs_and_pinned_goldens() {
        // Pinned from this crate's own runs AFTER the behavioral oracles
        // above were green (rule 8). Fixtures: 32x32 SplitMix64 seed
        // 0x11F0_601D fast path (σ_r = 0.2, α = 0.5, β = 1, M = 14);
        // 16x16 seed 0x11F0_601E exact path (same remap, nlev = 3).
        let p = random_plane(32, 32, 0x11F0_601D);
        let a = fast_local_laplacian(&p, &BOOST, LLF_DEFAULT_M, None).unwrap();
        let b = fast_local_laplacian(&p, &BOOST, LLF_DEFAULT_M, None).unwrap();
        assert_eq!(
            plane_hash(&a.data),
            plane_hash(&b.data),
            "fast two-run bit equality"
        );
        assert_eq!(
            plane_hash(&a.data),
            2217648903639312578,
            "pinned fast-LLF golden hash"
        );

        let q = random_plane(16, 16, 0x11F0_601E);
        let e1 = local_laplacian_exact(&q, &BOOST, Some(3)).unwrap();
        let e2 = local_laplacian_exact(&q, &BOOST, Some(3)).unwrap();
        assert_eq!(
            plane_hash(&e1.data),
            plane_hash(&e2.data),
            "exact two-run bit equality"
        );
        assert_eq!(
            plane_hash(&e1.data),
            4206083170496503110,
            "pinned exact-LLF golden hash"
        );
    }
}
