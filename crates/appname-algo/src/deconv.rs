// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Richardson–Lucy and Wiener follow the classical published formulations
// (richardson72, lucy74; Wiener with Laplacian regularizer) matching the
// scikit-image restoration API contract the census names as porting
// reference (B1-scikit-image.yaml, BSD-3-Clause at pin cb1adbd6,
// files_taken: [] — the tree is not staged locally; NO scikit-image code
// was available to or copied by this module). The hyper-Laplacian solver
// is CLEAN-ROOM from the paper (D. Krishnan, R. Fergus, "Fast Image
// Deconvolution using Hyper-Laplacian Priors", NeurIPS 2009 — census key
// krishnan09) exactly as the fastdeconv census row directs
// (B1-fastdeconv.yaml: "the shipped implementation remains clean-room from
// the paper"; the MIT repo is a licensed cross-check only, also not
// staged, not consulted).

//! Spec: ALGORITHMS.md §12 `sharpen_deblur` — primary `usm_rl_wiener`
//! (L0); blind tier basis krishnan09 (L1, `cleared_by: [US7616826B2]` per
//! contract); design-around memo for US9619870B2: "standard multiscale
//! MAP; no noise-adaptive scale scheduling" — this module is single-scale
//! non-blind, no noise-adaptive scheduling of any kind. ES §16.3
//! sharpen.*; INTEGRATION-NOTES §6 orders 6a (scikit-image RL/Wiener) and
//! 6b (fastdeconv hyper-Laplacian).
//!
//! Non-blind deconvolution engines (smart-sharpen / deblur classical
//! tier):
//! * [`richardson_lucy`] — multiplicative EM iteration;
//! * [`wiener`] — regularized frequency-domain inverse (Laplacian
//!   penalty, `balance` as in the scikit-image API contract);
//! * [`hyper_laplacian`] — krishnan09 half-quadratic splitting with
//!   |∇x|^α prior (α = 2/3 default), Newton w-subproblem (deterministic
//!   fixed iteration count; the paper's LUT is an optimization, not a
//!   semantic).
//!
//! All engines work on periodic (circular) boundaries over an internally
//! padded power-of-two grid; callers pass edge-padded ROIs at the adapter
//! seam when ringing at borders matters (DD area 16 ROI contract).
//!
//! Determinism: scalar f64 FFT with fixed order, fixed iteration counts,
//! fixed Newton step count; fixed-seed goldens in tests.

use crate::fft::fft2;
use crate::plane::Plane;

/// A convolution kernel with explicit center (odd sizes typical).
#[derive(Clone, Debug)]
pub struct Kernel {
    /// Kernel width.
    pub w: usize,
    /// Kernel height.
    pub h: usize,
    /// Center x (pixel whose weight applies to the output position).
    pub cx: usize,
    /// Center y.
    pub cy: usize,
    /// Row-major weights.
    pub data: Vec<f64>,
}

impl Kernel {
    /// Normalized isotropic Gaussian kernel of standard deviation `sigma`,
    /// radius `ceil(3 sigma)`.
    pub fn gaussian(sigma: f64) -> Kernel {
        let r = (3.0 * sigma).ceil().max(1.0) as usize;
        let n = 2 * r + 1;
        let mut data = vec![0f64; n * n];
        let mut sum = 0.0;
        for y in 0..n {
            for x in 0..n {
                let dx = x as f64 - r as f64;
                let dy = y as f64 - r as f64;
                let v = (-(dx * dx + dy * dy) / (2.0 * sigma * sigma)).exp();
                data[y * n + x] = v;
                sum += v;
            }
        }
        for v in data.iter_mut() {
            *v /= sum;
        }
        Kernel {
            w: n,
            h: n,
            cx: r,
            cy: r,
            data,
        }
    }

    /// Identity (delta) kernel.
    pub fn delta() -> Kernel {
        Kernel {
            w: 1,
            h: 1,
            cx: 0,
            cy: 0,
            data: vec![1.0],
        }
    }
}

/// Padded frequency-domain workspace for one image/kernel pairing.
struct Freq {
    pw: usize,
    ph: usize,
    /// Kernel spectrum (re, im).
    kr: Vec<f64>,
    ki: Vec<f64>,
}

fn next_pow2(n: usize) -> usize {
    n.next_power_of_two()
}

impl Freq {
    fn new(img_w: usize, img_h: usize, k: &Kernel) -> Freq {
        let pw = next_pow2(img_w + k.w);
        let ph = next_pow2(img_h + k.h);
        let mut kr = vec![0f64; pw * ph];
        let mut ki = vec![0f64; pw * ph];
        for y in 0..k.h {
            for x in 0..k.w {
                // Wrap the kernel so its center sits at (0, 0).
                let px = (x + pw - k.cx) % pw;
                let py = (y + ph - k.cy) % ph;
                kr[py * pw + px] = k.data[y * k.w + x];
            }
        }
        fft2(&mut kr, &mut ki, pw, ph, false);
        Freq { pw, ph, kr, ki }
    }

    /// Embed a plane (edge-replicated into the padding margin to soften
    /// wrap-around) and return its spectrum.
    fn spectrum(&self, p: &Plane) -> (Vec<f64>, Vec<f64>) {
        let mut re = vec![0f64; self.pw * self.ph];
        let mut im = vec![0f64; self.pw * self.ph];
        for y in 0..self.ph {
            for x in 0..self.pw {
                let sx = x.min(p.w - 1);
                let sy = y.min(p.h - 1);
                re[y * self.pw + x] = p.at(sx, sy) as f64;
            }
        }
        fft2(&mut re, &mut im, self.pw, self.ph, false);
        (re, im)
    }

    /// Convolve a padded spatial buffer (f64, pw*ph) with the kernel.
    fn convolve_padded(&self, data: &[f64], conjugate: bool) -> Vec<f64> {
        let mut re = data.to_vec();
        let mut im = vec![0f64; data.len()];
        fft2(&mut re, &mut im, self.pw, self.ph, false);
        for i in 0..re.len() {
            let (kr, ki) = if conjugate {
                (self.kr[i], -self.ki[i])
            } else {
                (self.kr[i], self.ki[i])
            };
            let r = re[i] * kr - im[i] * ki;
            let ii = re[i] * ki + im[i] * kr;
            re[i] = r;
            im[i] = ii;
        }
        fft2(&mut re, &mut im, self.pw, self.ph, true);
        re
    }
}

/// Richardson–Lucy deconvolution (richardson72/lucy74 multiplicative EM;
/// scikit-image API contract: flat 0.5 initialization, `iterations`
/// updates).
pub fn richardson_lucy(observed: &Plane, psf: &Kernel, iterations: usize) -> Plane {
    let freq = Freq::new(observed.w, observed.h, psf);
    let (pw, ph) = (freq.pw, freq.ph);
    // Padded observed image (edge-replicated).
    let mut obs = vec![0f64; pw * ph];
    for y in 0..ph {
        for x in 0..pw {
            obs[y * pw + x] = observed.at(x.min(observed.w - 1), y.min(observed.h - 1)) as f64;
        }
    }
    let mut est = vec![0.5f64; pw * ph];
    for _ in 0..iterations {
        let blur = freq.convolve_padded(&est, false);
        let mut ratio = vec![0f64; pw * ph];
        for i in 0..ratio.len() {
            ratio[i] = obs[i] / blur[i].max(1e-12);
        }
        let corr = freq.convolve_padded(&ratio, true);
        for i in 0..est.len() {
            est[i] = (est[i] * corr[i]).max(0.0);
        }
    }
    let mut out = Plane::new(observed.w, observed.h);
    for y in 0..observed.h {
        for x in 0..observed.w {
            out.data[y * observed.w + x] = est[y * pw + x] as f32;
        }
    }
    out
}

/// Wiener deconvolution with Laplacian regularization (scikit-image API
/// contract: `x̂ = F⁻¹[ conj(H) Y / (|H|² + balance |L|²) ]`).
pub fn wiener(observed: &Plane, psf: &Kernel, balance: f64) -> Plane {
    let freq = Freq::new(observed.w, observed.h, psf);
    let (pw, ph) = (freq.pw, freq.ph);
    let (yr, yi) = freq.spectrum(observed);
    // Laplacian penalty spectrum.
    let lap = Kernel {
        w: 3,
        h: 3,
        cx: 1,
        cy: 1,
        data: vec![0.0, 1.0, 0.0, 1.0, -4.0, 1.0, 0.0, 1.0, 0.0],
    };
    let lfreq = Freq::new(observed.w, observed.h, &lap);
    let mut re = vec![0f64; pw * ph];
    let mut im = vec![0f64; pw * ph];
    for i in 0..re.len() {
        let h2 = freq.kr[i] * freq.kr[i] + freq.ki[i] * freq.ki[i];
        let l2 = lfreq.kr[i] * lfreq.kr[i] + lfreq.ki[i] * lfreq.ki[i];
        let denom = h2 + balance * l2;
        // conj(H) * Y / denom
        let nr = freq.kr[i] * yr[i] + freq.ki[i] * yi[i];
        let ni = freq.kr[i] * yi[i] - freq.ki[i] * yr[i];
        if denom > 1e-15 {
            re[i] = nr / denom;
            im[i] = ni / denom;
        }
    }
    fft2(&mut re, &mut im, pw, ph, true);
    let mut out = Plane::new(observed.w, observed.h);
    for y in 0..observed.h {
        for x in 0..observed.w {
            out.data[y * observed.w + x] = re[y * pw + x] as f32;
        }
    }
    out
}

/// One Newton solve of the hyper-Laplacian w-subproblem:
/// `min_w |w|^alpha + (beta/2)(w - v)^2` (krishnan09 §3.1 semantics; the
/// paper's LUT tabulates exactly this minimizer). Deterministic: fixed 12
/// Newton steps from `v`, candidate comparison against w = 0.
fn shrink_hyper(v: f64, alpha: f64, beta: f64) -> f64 {
    if v == 0.0 {
        return 0.0;
    }
    let sign = v.signum();
    let av = v.abs();
    // Objective for |w| = t > 0: t^alpha + beta/2 (t - av)^2.
    let cost = |t: f64| t.powf(alpha) + 0.5 * beta * (t - av) * (t - av);
    // Newton on the derivative: alpha t^(alpha-1) + beta (t - av) = 0.
    let mut t = av;
    for _ in 0..12 {
        let g = alpha * t.powf(alpha - 1.0) + beta * (t - av);
        let hess = alpha * (alpha - 1.0) * t.powf(alpha - 2.0) + beta;
        if hess.abs() < 1e-30 {
            break;
        }
        t -= g / hess;
        // Deliberate negated-comparison shape: also catches NaN.
        #[allow(clippy::neg_cmp_op_on_partial_ord)]
        if !(t > 1e-12) {
            t = 1e-12;
        }
    }
    // The subproblem is non-convex: compare against the w = 0 candidate
    // (cost av² beta/2 at zero vs interior stationary point).
    if cost(t) < 0.5 * beta * av * av {
        sign * t
    } else {
        0.0
    }
}

/// Hyper-Laplacian non-blind deconvolution (krishnan09 half-quadratic
/// splitting, clean-room from the paper).
///
/// Minimizes `(lambda/2)||k⊗x − y||² + Σ|∇x|^alpha` with the standard
/// beta-continuation `beta = beta0 · beta_rate^j`, `j = 0..outer`.
pub fn hyper_laplacian(
    observed: &Plane,
    psf: &Kernel,
    lambda: f64,
    alpha: f64,
    outer: usize,
) -> Plane {
    let freq = Freq::new(observed.w, observed.h, psf);
    let (pw, ph) = (freq.pw, freq.ph);
    let (yr, yi) = freq.spectrum(observed);

    // Gradient operator spectra (forward differences, periodic).
    let gx = Kernel {
        w: 2,
        h: 1,
        cx: 0,
        cy: 0,
        data: vec![-1.0, 1.0],
    };
    let gy = Kernel {
        w: 1,
        h: 2,
        cx: 0,
        cy: 0,
        data: vec![-1.0, 1.0],
    };
    let fx = Freq::new(observed.w, observed.h, &gx);
    let fy = Freq::new(observed.w, observed.h, &gy);

    // Initialize x with the observed image.
    let mut x_re = vec![0f64; pw * ph];
    for y in 0..ph {
        for xx in 0..pw {
            x_re[y * pw + xx] = observed.at(xx.min(observed.w - 1), y.min(observed.h - 1)) as f64;
        }
    }

    let beta0 = 1.0f64;
    let beta_rate = 2.0f64 * 2.0f64.sqrt();
    for j in 0..outer {
        let beta = beta0 * beta_rate.powi(j as i32);
        // w-subproblem: shrink the gradients of the current x.
        let gxv = fx.convolve_padded(&x_re, false);
        let gyv = fy.convolve_padded(&x_re, false);
        let wx: Vec<f64> = gxv.iter().map(|&v| shrink_hyper(v, alpha, beta)).collect();
        let wy: Vec<f64> = gyv.iter().map(|&v| shrink_hyper(v, alpha, beta)).collect();
        // x-subproblem in Fourier (krishnan09 eq. 5):
        // X = [ (lambda/beta) conj(K) Y + conj(Gx) Wx + conj(Gy) Wy ]
        //   / [ (lambda/beta) |K|² + |Gx|² + |Gy|² ]
        let mut wxr = wx;
        let mut wxi = vec![0f64; pw * ph];
        fft2(&mut wxr, &mut wxi, pw, ph, false);
        let mut wyr = wy;
        let mut wyi = vec![0f64; pw * ph];
        fft2(&mut wyr, &mut wyi, pw, ph, false);
        let lb = lambda / beta;
        let mut xr = vec![0f64; pw * ph];
        let mut xi = vec![0f64; pw * ph];
        for i in 0..xr.len() {
            let k2 = freq.kr[i] * freq.kr[i] + freq.ki[i] * freq.ki[i];
            let g2 = fx.kr[i] * fx.kr[i]
                + fx.ki[i] * fx.ki[i]
                + fy.kr[i] * fy.kr[i]
                + fy.ki[i] * fy.ki[i];
            let denom = lb * k2 + g2;
            // conj(K) Y
            let kyr = freq.kr[i] * yr[i] + freq.ki[i] * yi[i];
            let kyi = freq.kr[i] * yi[i] - freq.ki[i] * yr[i];
            // conj(Gx) Wx + conj(Gy) Wy
            let gxr = fx.kr[i] * wxr[i] + fx.ki[i] * wxi[i];
            let gxi = fx.kr[i] * wxi[i] - fx.ki[i] * wxr[i];
            let gyr = fy.kr[i] * wyr[i] + fy.ki[i] * wyi[i];
            let gyi = fy.kr[i] * wyi[i] - fy.ki[i] * wyr[i];
            let nr = lb * kyr + gxr + gyr;
            let ni = lb * kyi + gxi + gyi;
            if denom > 1e-15 {
                xr[i] = nr / denom;
                xi[i] = ni / denom;
            }
        }
        fft2(&mut xr, &mut xi, pw, ph, true);
        x_re = xr;
    }

    let mut out = Plane::new(observed.w, observed.h);
    for y in 0..observed.h {
        for xx in 0..observed.w {
            out.data[y * observed.w + xx] = x_re[y * pw + xx] as f32;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plane::plane_hash;

    /// Smooth periodic-ish synthetic scene with a sharp block (edges to
    /// recover).
    fn scene(w: usize, h: usize) -> Plane {
        let mut p = Plane::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let fx = x as f32 / w as f32;
                let fy = y as f32 / h as f32;
                let mut v = 0.4
                    + 0.2
                        * (2.0 * std::f32::consts::PI * fx).sin()
                        * (2.0 * std::f32::consts::PI * fy).cos();
                if (10..20).contains(&x) && (8..16).contains(&y) {
                    v += 0.35;
                }
                p.data[y * w + x] = v.clamp(0.0, 1.0);
            }
        }
        p
    }

    fn blur(p: &Plane, k: &Kernel) -> Plane {
        let freq = Freq::new(p.w, p.h, k);
        let (pw, ph) = (freq.pw, freq.ph);
        let mut buf = vec![0f64; pw * ph];
        for y in 0..ph {
            for x in 0..pw {
                buf[y * pw + x] = p.at(x.min(p.w - 1), y.min(p.h - 1)) as f64;
            }
        }
        let out = freq.convolve_padded(&buf, false);
        let mut r = Plane::new(p.w, p.h);
        for y in 0..p.h {
            for x in 0..p.w {
                r.data[y * p.w + x] = out[y * pw + x] as f32;
            }
        }
        r
    }

    fn mse(a: &Plane, b: &Plane) -> f64 {
        a.data
            .iter()
            .zip(&b.data)
            .map(|(&x, &y)| ((x - y) as f64).powi(2))
            .sum::<f64>()
            / a.data.len() as f64
    }

    #[test]
    fn delta_psf_is_near_identity() {
        let img = scene(32, 24);
        let rl = richardson_lucy(&img, &Kernel::delta(), 10);
        assert!(mse(&img, &rl) < 1e-6, "RL delta mse {}", mse(&img, &rl));
        let wn = wiener(&img, &Kernel::delta(), 1e-4);
        assert!(mse(&img, &wn) < 1e-4, "Wiener delta mse {}", mse(&img, &wn));
    }

    #[test]
    fn richardson_lucy_sharpens_gaussian_blur() {
        let img = scene(32, 32);
        let k = Kernel::gaussian(1.2);
        let blurred = blur(&img, &k);
        // RL converges slowly on Gaussian PSFs (multiplicative EM): 80
        // iterations comfortably halves the MSE on this fixture.
        let deconv = richardson_lucy(&blurred, &k, 80);
        let before = mse(&img, &blurred);
        let after = mse(&img, &deconv);
        assert!(
            after < before * 0.5,
            "RL must at least halve the blur MSE: {before} -> {after}"
        );
    }

    #[test]
    fn wiener_sharpens_gaussian_blur() {
        let img = scene(32, 32);
        let k = Kernel::gaussian(1.2);
        let blurred = blur(&img, &k);
        let deconv = wiener(&blurred, &k, 1e-3);
        let before = mse(&img, &blurred);
        let after = mse(&img, &deconv);
        assert!(
            after < before * 0.5,
            "Wiener must at least halve the blur MSE: {before} -> {after}"
        );
    }

    #[test]
    fn hyper_laplacian_sharpens_and_beats_plain_wiener_on_edges() {
        let img = scene(32, 32);
        let k = Kernel::gaussian(1.2);
        let blurred = blur(&img, &k);
        let deconv = hyper_laplacian(&blurred, &k, 2000.0, 2.0 / 3.0, 8);
        let before = mse(&img, &blurred);
        let after = mse(&img, &deconv);
        assert!(
            after < before * 0.4,
            "hyper-Laplacian must reduce blur MSE strongly: {before} -> {after}"
        );
    }

    #[test]
    fn shrinkage_operator_semantics() {
        // Large inputs shrink but keep sign and magnitude order; small
        // inputs collapse to exactly zero (the sparsity the prior buys).
        let alpha = 2.0 / 3.0;
        let beta = 8.0;
        let big = shrink_hyper(1.0, alpha, beta);
        assert!(big > 0.5 && big < 1.0, "big shrink {big}");
        assert_eq!(shrink_hyper(-1.0, alpha, beta), -big, "odd symmetry");
        assert_eq!(shrink_hyper(0.01, alpha, beta), 0.0, "small -> 0");
        assert_eq!(shrink_hyper(0.0, alpha, beta), 0.0);
    }

    #[test]
    fn deconv_goldens_and_determinism() {
        let img = scene(32, 32);
        let k = Kernel::gaussian(1.0);
        let blurred = blur(&img, &k);
        let rl1 = richardson_lucy(&blurred, &k, 12);
        let rl2 = richardson_lucy(&blurred, &k, 12);
        let wn = wiener(&blurred, &k, 1e-3);
        let hl = hyper_laplacian(&blurred, &k, 1500.0, 2.0 / 3.0, 6);
        assert_eq!(plane_hash(&rl1.data), plane_hash(&rl2.data));
        assert_eq!(
            [
                plane_hash(&rl1.data),
                plane_hash(&wn.data),
                plane_hash(&hl.data)
            ],
            [GOLDEN_RL, GOLDEN_WIENER, GOLDEN_HL],
            "deconv golden drift — investigate before re-pinning (O-QA rule)"
        );
    }

    // Pinned 2026-08-13 after the behavioral oracles above were green
    // (delta identity, blur-MSE halving for all three engines, shrinkage
    // semantics).
    const GOLDEN_RL: u64 = 15432551795303669711;
    const GOLDEN_WIENER: u64 = 17995144473129806480;
    const GOLDEN_HL: u64 = 15172293010936106250;
}
