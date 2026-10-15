// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §17.1 Histogram computation [NORMATIVE]: bins index the
//! **display-referred encoding** of the working value — `bin =
//! clamp(⌊encode(v)·255 + 0.5⌋, 0, 255)`; R/G/B planes count
//! un-premultiplied channels; pixels with `a = 0` are excluded, all others
//! count once (unweighted); the fourth plane is luma of the ENCODED
//! values, `Lum = 0.3R + 0.59G + 0.11B` (§4.5 coefficients). Per-tile
//! counts merge associatively — tiles in any order; an edit re-counts only
//! dirty tiles (the tile cache itself is the evaluator's, §4.6 — this
//! module is the pure counting + merge service).

use appname_tools::resample::{EdgePolicy, Plane};

use crate::kernel::display_encode;

/// §17.1 `Histogram`.
#[derive(Clone, Debug, PartialEq)]
pub struct Histogram {
    /// R, G, B, Luma.
    pub bins: [[u32; 256]; 4],
    /// Counted pixels (`a > 0`).
    pub total: u64,
}

impl Default for Histogram {
    fn default() -> Histogram {
        Histogram {
            bins: [[0; 256]; 4],
            total: 0,
        }
    }
}

/// The [NORMATIVE] bin index.
#[inline]
pub fn bin_of(encoded: f32) -> usize {
    ((encoded * 255.0 + 0.5).floor()).clamp(0.0, 255.0) as usize
}

impl Histogram {
    /// Count a premultiplied working-space plane (§17.1 rules).
    pub fn of_plane(plane: &Plane) -> Histogram {
        let mut h = Histogram::default();
        for y in plane.rect.min.y..plane.rect.max.y {
            for x in plane.rect.min.x..plane.rect.max.x {
                let p = plane.get(x, y, EdgePolicy::Transparent);
                if p[3] == 0.0 {
                    continue; // excluded
                }
                let s = appname_core::io::srgb::unpremul(p);
                let enc = [
                    display_encode(s[0]),
                    display_encode(s[1]),
                    display_encode(s[2]),
                ];
                for (c, &e) in enc.iter().enumerate() {
                    h.bins[c][bin_of(e)] += 1;
                }
                let luma = 0.3 * enc[0] + 0.59 * enc[1] + 0.11 * enc[2];
                h.bins[3][bin_of(luma)] += 1;
                h.total += 1;
            }
        }
        h
    }

    /// Associative merge (per-tile counts sum in any order, §17.1).
    pub fn merge(&mut self, other: &Histogram) {
        for c in 0..4 {
            for b in 0..256 {
                self.bins[c][b] += other.bins[c][b];
            }
        }
        self.total += other.total;
    }

    /// §17.1 clipping indicators.
    pub fn clip_low(&self, channel: usize) -> u32 {
        self.bins[channel][0]
    }

    pub fn clip_high(&self, channel: usize) -> u32 {
        self.bins[channel][255]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use appname_core::IRect;

    #[test]
    fn bins_are_the_normative_index_and_rules() {
        // bin = clamp(⌊enc·255 + 0.5⌋, 0, 255).
        assert_eq!(bin_of(0.0), 0);
        assert_eq!(bin_of(1.0), 255);
        assert_eq!(bin_of(0.5), 128); // 127.5+0.5 = 128
        assert_eq!(bin_of(128.4 / 255.0), 128);
        assert_eq!(bin_of(2.0), 255, "HDR clamps into the top bin");

        let mut p = Plane::new(IRect::new(0, 0, 4, 1));
        p.set(0, 0, [0.0, 0.0, 0.0, 0.0]); // a = 0 → excluded
        p.set(1, 0, [1.0, 0.0, 0.0, 1.0]); // pure red
        p.set(2, 0, [0.5, 0.5, 0.5, 0.5]); // straight 1.0 gray at α .5
        p.set(3, 0, [0.0, 0.0, 0.0, 1.0]); // black
        let h = Histogram::of_plane(&p);
        assert_eq!(h.total, 3, "a = 0 excluded, others once");
        // Red pixel: R bin 255, G/B bin 0.
        assert_eq!(h.bins[0][255], 2, "red + unpremultiplied white gray");
        assert_eq!(h.bins[1][0], 2, "green zero for red + black");
        // Luma of pure red = 0.3·encode(1.0) — derived through the SAME
        // transfer the counter uses (encode(1.0) is 1 − ~5e-8 in f32, which
        // moves the ⌊·+0.5⌋ bin off the idealized 77).
        let red_luma_bin = bin_of(0.3 * display_encode(1.0));
        assert_eq!(h.bins[3][red_luma_bin], 1);
        // Black pixel lands in bin 0 everywhere; clip indicators see it.
        assert!(h.clip_low(0) >= 1 && h.clip_high(0) >= 1);
    }

    #[test]
    fn unpremultiplied_counting_and_encoding_axis() {
        // Premul 0.25 at α 0.5 = straight 0.5 linear → encoded ≈ 0.7354
        // (sRGB): the bin is the ENCODED one, not the linear.
        let mut p = Plane::new(IRect::new(0, 0, 1, 1));
        p.set(0, 0, [0.25, 0.25, 0.25, 0.5]);
        let h = Histogram::of_plane(&p);
        let enc = display_encode(0.5);
        assert_eq!(h.bins[0].iter().position(|&n| n > 0), Some(bin_of(enc)));
    }

    #[test]
    fn merge_is_associative_and_matches_whole_counts() {
        let mut p = Plane::new(IRect::new(0, 0, 8, 8));
        for y in 0..8 {
            for x in 0..8 {
                let v = ((x * 13 + y * 7) % 11) as f32 / 10.0;
                p.set(x, y, [v * 0.6, v * 0.3, v, 1.0]);
            }
        }
        let whole = Histogram::of_plane(&p);
        // Split into quadrant planes and merge in two different orders.
        let mut quads = Vec::new();
        for (qx, qy) in [(0, 0), (4, 0), (0, 4), (4, 4)] {
            let mut q = Plane::new(IRect::new(qx, qy, qx + 4, qy + 4));
            for y in q.rect.min.y..q.rect.max.y {
                for x in q.rect.min.x..q.rect.max.x {
                    q.set(x, y, p.get(x, y, EdgePolicy::Transparent));
                }
            }
            quads.push(Histogram::of_plane(&q));
        }
        let mut a = Histogram::default();
        for q in &quads {
            a.merge(q);
        }
        let mut b = Histogram::default();
        for q in quads.iter().rev() {
            b.merge(q);
        }
        assert_eq!(a, whole);
        assert_eq!(b, whole, "order-independent merge");
    }
}
