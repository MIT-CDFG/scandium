// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §8.2 "Palette quantization" [NORMATIVE]: modified median-cut to
//! `max_colors`, then Floyd–Steinberg error diffusion by default
//! (serpentine scan, weights 7/16, 3/16, 5/16, 1/16, error clamped at
//! palette-space bounds); `Ordered8x8` uses the standard 8×8 Bayer matrix;
//! `None` nearest-color only. DD ⑫.
//!
//! Implemented in-crate (era-classic algorithms; the gif crate's optional
//! NeuQuant path is deliberately NOT compiled — §8.2 names median-cut).
//! Determinism: histogram in a `BTreeMap` (sorted iteration), stable sorts,
//! index-order tie-breaks — identical input planes yield identical
//! palettes, indices, and therefore bytes.

use std::collections::BTreeMap;

use crate::opts::Dither;

/// Standard 8×8 Bayer threshold matrix (values 0..64).
const BAYER8: [[u8; 8]; 8] = [
    [0, 32, 8, 40, 2, 34, 10, 42],
    [48, 16, 56, 24, 50, 18, 58, 26],
    [12, 44, 4, 36, 14, 46, 6, 38],
    [60, 28, 52, 20, 62, 30, 54, 22],
    [3, 35, 11, 43, 1, 33, 9, 41],
    [51, 19, 59, 27, 49, 17, 57, 25],
    [15, 47, 7, 39, 13, 45, 5, 37],
    [63, 31, 55, 23, 61, 29, 53, 21],
];

/// One box of the median-cut: a slice of the (color, count) histogram.
struct McBox {
    colors: Vec<([u8; 3], u32)>,
    count: u64,
}

impl McBox {
    fn new(colors: Vec<([u8; 3], u32)>) -> McBox {
        let count = colors.iter().map(|(_, n)| u64::from(*n)).sum();
        McBox { colors, count }
    }

    /// (channel, range) of the widest channel; ties prefer the lower
    /// channel index (deterministic).
    fn widest(&self) -> (usize, u8) {
        let mut best = (0usize, 0u8);
        for c in 0..3 {
            let mut lo = u8::MAX;
            let mut hi = 0u8;
            for (rgb, _) in &self.colors {
                lo = lo.min(rgb[c]);
                hi = hi.max(rgb[c]);
            }
            let range = hi - lo;
            if range > best.1 {
                best = (c, range);
            }
        }
        best
    }

    /// Population-weighted average color, rounded.
    fn average(&self) -> [u8; 3] {
        let mut acc = [0u64; 3];
        let mut n = 0u64;
        for (rgb, cnt) in &self.colors {
            for c in 0..3 {
                acc[c] += u64::from(rgb[c]) * u64::from(*cnt);
            }
            n += u64::from(*cnt);
        }
        if n == 0 {
            return [0; 3];
        }
        [0, 1, 2].map(|c| ((acc[c] + n / 2) / n) as u8)
    }
}

/// Modified median-cut over the opaque pixels of straight sRGB RGBA data.
/// Returns a palette of at most `max_colors` entries, sorted
/// lexicographically (deterministic index assignment).
pub fn median_cut_palette(rgba: &[u8], max_colors: usize) -> Vec<[u8; 3]> {
    // Histogram (BTreeMap: deterministic order).
    let mut hist: BTreeMap<[u8; 3], u32> = BTreeMap::new();
    for px in rgba.chunks_exact(4) {
        if px[3] >= 128 {
            *hist.entry([px[0], px[1], px[2]]).or_insert(0) += 1;
        }
    }
    if hist.is_empty() {
        return vec![[0, 0, 0]];
    }
    let mut boxes = vec![McBox::new(hist.into_iter().collect())];
    while boxes.len() < max_colors {
        // The "modified" step: split the box with the largest
        // population×range product (ties → lower box index).
        let mut pick: Option<(usize, u128)> = None;
        for (i, b) in boxes.iter().enumerate() {
            let (_, range) = b.widest();
            if b.colors.len() < 2 || range == 0 {
                continue;
            }
            let score = u128::from(b.count) * u128::from(range);
            if pick.is_none_or(|(_, s)| score > s) {
                pick = Some((i, score));
            }
        }
        let Some((i, _)) = pick else { break };
        let mut b = boxes.swap_remove(i);
        let (ch, _) = b.widest();
        // Stable sort along the widest channel, then full-color tiebreak
        // (BTreeMap order already lexicographic — sort is stable).
        b.colors
            .sort_by(|a, z| a.0[ch].cmp(&z.0[ch]).then(a.0.cmp(&z.0)));
        // Weighted median split point (at least one color each side).
        let half = b.count / 2;
        let mut acc = 0u64;
        let mut split = 1usize;
        for (j, (_, cnt)) in b.colors.iter().enumerate() {
            acc += u64::from(*cnt);
            if acc >= half && j + 1 < b.colors.len() {
                split = j + 1;
                break;
            }
        }
        let right = b.colors.split_off(split);
        boxes.push(McBox::new(b.colors));
        boxes.push(McBox::new(right));
    }
    let mut palette: Vec<[u8; 3]> = boxes.iter().map(McBox::average).collect();
    palette.sort_unstable();
    palette.dedup();
    palette
}

/// Nearest palette entry by squared Euclidean RGB distance; ties → lowest
/// index (deterministic).
fn nearest(palette: &[[u8; 3]], rgb: [i32; 3]) -> usize {
    let mut best = (0usize, i64::MAX);
    for (i, p) in palette.iter().enumerate() {
        let d: i64 = (0..3)
            .map(|c| {
                let d = i64::from(rgb[c]) - i64::from(p[c]);
                d * d
            })
            .sum();
        if d < best.1 {
            best = (i, d);
        }
    }
    best.0
}

/// Map straight sRGB RGBA data to palette indices with the §8.2 dither
/// modes. `transparent` is the index written for pixels whose alpha
/// binarizes to transparent (< 0.5, the §13.5 iso-level → < 128 in 8-bit);
/// pass `None` when the caller guarantees full opacity.
pub fn index_plane(
    rgba: &[u8],
    width: usize,
    height: usize,
    palette: &[[u8; 3]],
    dither: Dither,
    transparent: Option<u8>,
) -> Vec<u8> {
    let mut out = vec![0u8; width * height];
    match dither {
        Dither::None | Dither::Ordered8x8 => {
            for y in 0..height {
                for x in 0..width {
                    let i = (y * width + x) * 4;
                    if rgba[i + 3] < 128 {
                        if let Some(t) = transparent {
                            out[y * width + x] = t;
                            continue;
                        }
                    }
                    let mut rgb = [
                        i32::from(rgba[i]),
                        i32::from(rgba[i + 1]),
                        i32::from(rgba[i + 2]),
                    ];
                    if dither == Dither::Ordered8x8 {
                        // Standard Bayer offset, symmetric around 0; the
                        // amplitude (48/64 ≈ 0.75 of a full step across the
                        // matrix) is an implementation choice — §8.2 names
                        // the matrix, not the spread.
                        let t = i32::from(BAYER8[y % 8][x % 8]) * 2 - 63; // -63..=63
                        for c in &mut rgb {
                            *c = (*c + t * 24 / 64).clamp(0, 255);
                        }
                    }
                    out[y * width + x] = nearest(palette, rgb) as u8;
                }
            }
        }
        Dither::FloydSteinberg => {
            // Serpentine scan, weights 7/16 3/16 5/16 1/16, error clamped
            // at palette-space bounds [NORMATIVE §8.2].
            let mut err = vec![[0i32; 3]; width * height];
            for y in 0..height {
                let serp = y % 2 == 1;
                let xs: Box<dyn Iterator<Item = usize>> = if serp {
                    Box::new((0..width).rev())
                } else {
                    Box::new(0..width)
                };
                for x in xs {
                    let pi = y * width + x;
                    let i = pi * 4;
                    if rgba[i + 3] < 128 {
                        if let Some(t) = transparent {
                            out[pi] = t;
                            continue;
                        }
                    }
                    let mut rgb = [0i32; 3];
                    for c in 0..3 {
                        // Error clamped at palette-space bounds (0..=255).
                        rgb[c] = (i32::from(rgba[i + c]) + err[pi][c]).clamp(0, 255);
                    }
                    let idx = nearest(palette, rgb);
                    out[pi] = idx as u8;
                    let chosen = palette[idx];
                    let e = [0, 1, 2].map(|c| rgb[c] - i32::from(chosen[c]));
                    let mut push = |dx: i32, dy: i32, num: i32| {
                        let nx = x as i32 + if serp { -dx } else { dx };
                        let ny = y as i32 + dy;
                        if nx >= 0 && (nx as usize) < width && (ny as usize) < height {
                            let ni = ny as usize * width + nx as usize;
                            for c in 0..3 {
                                err[ni][c] += e[c] * num / 16;
                            }
                        }
                    };
                    push(1, 0, 7);
                    push(-1, 1, 3);
                    push(0, 1, 5);
                    push(1, 1, 1);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_of_a_flat_plane_is_one_entry() {
        let rgba: Vec<u8> = [10u8, 200, 30, 255].repeat(64);
        let p = median_cut_palette(&rgba, 16);
        assert_eq!(p, vec![[10, 200, 30]]);
    }

    #[test]
    fn palette_respects_max_colors_and_is_deterministic() {
        // 256 distinct colors, ask for 8.
        let mut rgba = Vec::new();
        for i in 0..256u32 {
            rgba.extend_from_slice(&[
                (i % 256) as u8,
                (i * 7 % 256) as u8,
                (i * 13 % 256) as u8,
                255,
            ]);
        }
        let a = median_cut_palette(&rgba, 8);
        let b = median_cut_palette(&rgba, 8);
        assert!(a.len() <= 8 && a.len() >= 2);
        assert_eq!(a, b);
        assert!(a.windows(2).all(|w| w[0] < w[1]), "sorted, deduped");
    }

    #[test]
    fn two_color_plane_quantizes_exactly() {
        let mut rgba = Vec::new();
        for i in 0..32 {
            let c = if i % 2 == 0 {
                [0u8, 0, 0]
            } else {
                [255, 255, 255]
            };
            rgba.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
        let p = median_cut_palette(&rgba, 4);
        assert_eq!(p, vec![[0, 0, 0], [255, 255, 255]]);
        let idx = index_plane(&rgba, 32, 1, &p, Dither::None, None);
        for (i, v) in idx.iter().enumerate() {
            assert_eq!(*v, (i % 2) as u8);
        }
        // FS dither on an exactly-representable plane changes nothing.
        let fs = index_plane(&rgba, 32, 1, &p, Dither::FloydSteinberg, None);
        assert_eq!(idx, fs);
    }

    #[test]
    fn transparent_index_binarizes_at_half() {
        let rgba = vec![9u8, 9, 9, 127, 9, 9, 9, 128];
        let p = vec![[9u8, 9, 9]];
        let idx = index_plane(&rgba, 2, 1, &p, Dither::None, Some(1));
        assert_eq!(idx, vec![1, 0]);
    }
}
