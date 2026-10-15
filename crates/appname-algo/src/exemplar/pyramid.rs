// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §45.3 constant table "Pyramid" row (÷2 per level, Lanczos-3
//! per §18.2, floor `min(w,h) ≥ 32` px, `draft` 64); §21.4 pyramid stage;
//! ES §45.3 sample-region semantics ("S and H downsample with the pyramid
//! using the mask ≥ 0.5 rule").
//!
//! Deterministic pyramid substrate for the exemplar engine: separable
//! Lanczos-3 ÷2 image downsampling, binary-mask downsampling (2×2
//! coverage mean thresholded at ≥ 0.5 — the negative Lanczos lobes are
//! deliberately kept away from binary data), and the ×2 nearest upsample
//! used for the colors + patch-ownership carry between levels (§21.4
//! "upsample colors + patch-ownership map (×2, nearest)").

use crate::plane::Plane;

/// Lanczos-3 kernel `L(x) = sinc(x)·sinc(x/3)` for `|x| < 3`.
fn lanczos3(x: f64) -> f64 {
    if x == 0.0 {
        return 1.0;
    }
    if x.abs() >= 3.0 {
        return 0.0;
    }
    let pix = std::f64::consts::PI * x;
    let a = pix.sin() / pix;
    let b = (pix / 3.0).sin() / (pix / 3.0);
    a * b
}

/// One-axis ÷2 Lanczos-3 resample of a row-major buffer along x.
/// Output length `out_len = (len + 1) / 2`; output sample `o` is centered
/// at input coordinate `2o + 0.5` and reads the 12 taps within the scaled
/// support (kernel argument `(j − c)/2`), edge-clamped, weights
/// normalized. Fixed tap order → deterministic.
fn down_axis(src: &[f64], len: usize, out: &mut [f64]) {
    let out_len = out.len();
    debug_assert_eq!(out_len, len.div_ceil(2));
    for (o, slot) in out.iter_mut().enumerate() {
        let c = 2.0 * o as f64 + 0.5;
        let j0 = (c - 6.0).ceil() as isize;
        let j1 = (c + 6.0).floor() as isize;
        let mut acc = 0.0;
        let mut wsum = 0.0;
        for j in j0..=j1 {
            let w = lanczos3((j as f64 - c) / 2.0);
            if w == 0.0 {
                continue;
            }
            let jj = j.clamp(0, len as isize - 1) as usize;
            acc += w * src[jj];
            wsum += w;
        }
        *slot = acc / wsum;
    }
}

/// Separable Lanczos-3 ÷2 downsample of a plane (rows then columns; f64
/// accumulation, one f32 round at the end).
pub fn downsample_lanczos3(p: &Plane) -> Plane {
    let (w, h) = (p.w, p.h);
    let ow = w.div_ceil(2);
    let oh = h.div_ceil(2);
    // Rows.
    let mut tmp = vec![0f64; ow * h];
    let mut row = vec![0f64; w];
    let mut orow = vec![0f64; ow];
    for y in 0..h {
        for (x, slot) in row.iter_mut().enumerate() {
            *slot = p.data[y * w + x] as f64;
        }
        down_axis(&row, w, &mut orow);
        tmp[y * ow..(y + 1) * ow].copy_from_slice(&orow);
    }
    // Columns.
    let mut out = Plane::new(ow, oh);
    let mut col = vec![0f64; h];
    let mut ocol = vec![0f64; oh];
    for x in 0..ow {
        for (y, slot) in col.iter_mut().enumerate() {
            *slot = tmp[y * ow + x];
        }
        down_axis(&col, h, &mut ocol);
        for (y, &v) in ocol.iter().enumerate() {
            out.data[y * ow + x] = v as f32;
        }
    }
    out
}

/// Mask ÷2 downsample by the ≥ 0.5 coverage rule: each output pixel is
/// the mean of its (up to) 2×2 source block, thresholded at ≥ 0.5.
pub fn downsample_mask(mask: &[bool], w: usize, h: usize) -> (Vec<bool>, usize, usize) {
    let ow = w.div_ceil(2);
    let oh = h.div_ceil(2);
    let mut out = vec![false; ow * oh];
    for oy in 0..oh {
        for ox in 0..ow {
            let mut sum = 0usize;
            let mut cnt = 0usize;
            for dy in 0..2 {
                for dx in 0..2 {
                    let x = ox * 2 + dx;
                    let y = oy * 2 + dy;
                    if x < w && y < h {
                        cnt += 1;
                        if mask[y * w + x] {
                            sum += 1;
                        }
                    }
                }
            }
            out[oy * ow + ox] = 2 * sum >= cnt; // mean ≥ 0.5
        }
    }
    (out, ow, oh)
}

/// ×2 nearest upsample of coarse per-pixel data into fine dimensions
/// (`fine = coarse[y/2][x/2]`, clamped for odd edges).
pub fn upsample_nearest<T: Copy>(
    coarse: &[T],
    cw: usize,
    ch: usize,
    fw: usize,
    fh: usize,
) -> Vec<T> {
    let mut out = Vec::with_capacity(fw * fh);
    for y in 0..fh {
        let cy = (y / 2).min(ch - 1);
        for x in 0..fw {
            let cx = (x / 2).min(cw - 1);
            out.push(coarse[cy * cw + cx]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_plane_survives_downsample() {
        let p = Plane::from_vec(9, 7, vec![0.375; 63]);
        let d = downsample_lanczos3(&p);
        assert_eq!((d.w, d.h), (5, 4));
        for &v in &d.data {
            assert!((v - 0.375).abs() < 1e-6, "constant not preserved: {v}");
        }
    }

    #[test]
    fn mask_rule_is_majority() {
        // 2×2 blocks: 2-of-4 true → ≥ 0.5 → true; 1-of-4 → false.
        let m = vec![
            true, false, true, false, //
            true, false, false, false,
        ];
        let (d, ow, oh) = downsample_mask(&m, 4, 2);
        assert_eq!((ow, oh), (2, 1));
        assert!(d[0]); // 2/4
        assert!(!d[1]); // 1/4
    }

    #[test]
    fn nearest_upsample_replicates() {
        let c = vec![1u32, 2, 3, 4];
        let f = upsample_nearest(&c, 2, 2, 4, 4);
        assert_eq!(f[0], 1);
        assert_eq!(f[1], 1);
        assert_eq!(f[3], 2);
        assert_eq!(f[15], 4);
    }
}
