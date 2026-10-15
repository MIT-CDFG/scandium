// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §12 (sharpen/deblur engines, "own Rust FFT path per engine" —
//! census scikit-image row); support module for [`crate::deconv`].
//!
//! Minimal iterative radix-2 FFT (Cooley–Tukey, textbook formulation) for
//! the frequency-domain deconvolution engines. Power-of-two sizes only;
//! callers pad. Scalar f64, fixed butterfly order — bit-deterministic
//! across runs and platforms with IEEE-754 doubles.

use std::f64::consts::PI;

/// In-place radix-2 FFT of `re`/`im` (length must be a power of two).
/// `inverse` applies the conjugate transform and 1/N scaling.
pub fn fft(re: &mut [f64], im: &mut [f64], inverse: bool) {
    let n = re.len();
    assert_eq!(n, im.len());
    assert!(n.is_power_of_two(), "fft: length {n} not a power of two");
    if n <= 1 {
        return;
    }
    // Bit-reversal permutation.
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    // Butterflies.
    let sign = if inverse { 1.0 } else { -1.0 };
    let mut len = 2usize;
    while len <= n {
        let ang = sign * 2.0 * PI / len as f64;
        let (wr, wi) = (ang.cos(), ang.sin());
        let mut i = 0usize;
        while i < n {
            let mut cr = 1.0f64;
            let mut ci = 0.0f64;
            for k in 0..len / 2 {
                let a = i + k;
                let b = i + k + len / 2;
                let tr = re[b] * cr - im[b] * ci;
                let ti = re[b] * ci + im[b] * cr;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let ncr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = ncr;
            }
            i += len;
        }
        len <<= 1;
    }
    if inverse {
        let inv = 1.0 / n as f64;
        for v in re.iter_mut() {
            *v *= inv;
        }
        for v in im.iter_mut() {
            *v *= inv;
        }
    }
}

/// 2-D FFT over a row-major `w x h` grid (both powers of two), in place.
pub fn fft2(re: &mut [f64], im: &mut [f64], w: usize, h: usize, inverse: bool) {
    assert_eq!(re.len(), w * h);
    // Rows.
    let mut row_re = vec![0f64; w];
    let mut row_im = vec![0f64; w];
    for y in 0..h {
        row_re.copy_from_slice(&re[y * w..(y + 1) * w]);
        row_im.copy_from_slice(&im[y * w..(y + 1) * w]);
        fft(&mut row_re, &mut row_im, inverse);
        re[y * w..(y + 1) * w].copy_from_slice(&row_re);
        im[y * w..(y + 1) * w].copy_from_slice(&row_im);
    }
    // Columns.
    let mut col_re = vec![0f64; h];
    let mut col_im = vec![0f64; h];
    for x in 0..w {
        for y in 0..h {
            col_re[y] = re[y * w + x];
            col_im[y] = im[y * w + x];
        }
        fft(&mut col_re, &mut col_im, inverse);
        for y in 0..h {
            re[y * w + x] = col_re[y];
            im[y * w + x] = col_im[y];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impulse_transforms_to_flat_spectrum() {
        let mut re = vec![0.0; 8];
        let mut im = vec![0.0; 8];
        re[0] = 1.0;
        fft(&mut re, &mut im, false);
        for k in 0..8 {
            assert!((re[k] - 1.0).abs() < 1e-12 && im[k].abs() < 1e-12);
        }
    }

    #[test]
    fn round_trip_restores_signal() {
        let n = 32;
        let mut re: Vec<f64> = (0..n).map(|i| ((i * 7 + 3) % 13) as f64 * 0.1).collect();
        let orig = re.clone();
        let mut im = vec![0.0; n];
        fft(&mut re, &mut im, false);
        fft(&mut re, &mut im, true);
        for i in 0..n {
            assert!((re[i] - orig[i]).abs() < 1e-12 && im[i].abs() < 1e-12);
        }
    }

    #[test]
    fn parseval_energy_is_preserved() {
        let n = 16;
        let mut re: Vec<f64> = (0..n).map(|i| (i as f64 * 0.37).sin()).collect();
        let mut im = vec![0.0; n];
        let e_time: f64 = re.iter().map(|v| v * v).sum();
        fft(&mut re, &mut im, false);
        let e_freq: f64 = re.iter().zip(&im).map(|(r, i)| r * r + i * i).sum::<f64>() / n as f64;
        assert!((e_time - e_freq).abs() < 1e-10);
    }

    #[test]
    fn fft2_round_trip() {
        let (w, h) = (8, 4);
        let mut re: Vec<f64> = (0..w * h).map(|i| (i % 5) as f64 * 0.2 - 0.3).collect();
        let orig = re.clone();
        let mut im = vec![0.0; w * h];
        fft2(&mut re, &mut im, w, h, false);
        fft2(&mut re, &mut im, w, h, true);
        for i in 0..w * h {
            assert!((re[i] - orig[i]).abs() < 1e-12);
        }
    }
}
