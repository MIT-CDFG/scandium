// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES Part 45 (algorithm engines); DD area 16 (ROI/filter contracts).
//!
//! Minimal planar raster carrier shared by the `appname-algo` engines.
//!
//! Engines in this crate are PURE functions over planar `f32`/`f64` data +
//! dimensions. They never touch documents: consumers (appname-tools filter
//! corpus, appname-core op registry) adapt tile data into planes at the
//! brokered adapter seam and route every mutation through the
//! `appname.command.v1` envelope spine (D-48). No side-door mutation APIs.
//!
//! Determinism: all engines here are single-threaded, fixed-iteration-order
//! scalar code; any randomness comes from the caller-seeded [`SplitMix64`]
//! stream. `plane_hash` is the fixed-seed oracle used by the lane's
//! determinism evidence tests (FNV-1a over the IEEE-754 bit patterns, so a
//! hash equality is a bit-exact replay claim).

/// One planar channel of `f32` samples, row-major, no padding.
#[derive(Clone, Debug, PartialEq)]
pub struct Plane {
    /// Width in samples.
    pub w: usize,
    /// Height in samples.
    pub h: usize,
    /// Row-major samples; `data.len() == w * h`.
    pub data: Vec<f32>,
}

impl Plane {
    /// New zero-filled plane.
    pub fn new(w: usize, h: usize) -> Plane {
        Plane {
            w,
            h,
            data: vec![0.0; w * h],
        }
    }

    /// New plane from existing samples (length must be `w * h`).
    pub fn from_vec(w: usize, h: usize, data: Vec<f32>) -> Plane {
        assert_eq!(data.len(), w * h, "plane data length mismatch");
        Plane { w, h, data }
    }

    /// Sample at (x, y); caller guarantees bounds.
    #[inline]
    pub fn at(&self, x: usize, y: usize) -> f32 {
        self.data[y * self.w + x]
    }

    /// Mutable sample at (x, y); caller guarantees bounds.
    #[inline]
    pub fn at_mut(&mut self, x: usize, y: usize) -> &mut f32 {
        &mut self.data[y * self.w + x]
    }

    /// Clamped sample: coordinates outside the plane replicate the edge
    /// (the `imfilter(..., 'replicate')` / `'symmetric'`-at-radius-1 border
    /// convention the ported references use).
    #[inline]
    pub fn at_clamped(&self, x: isize, y: isize) -> f32 {
        let xc = x.clamp(0, self.w as isize - 1) as usize;
        let yc = y.clamp(0, self.h as isize - 1) as usize;
        self.data[yc * self.w + xc]
    }
}

/// Interleaved-to-planar split for RGB triples (adapter-seam helper).
pub fn rgb_planes(w: usize, h: usize, interleaved: &[f32]) -> [Plane; 3] {
    assert_eq!(interleaved.len(), w * h * 3, "rgb buffer length mismatch");
    let mut r = Plane::new(w, h);
    let mut g = Plane::new(w, h);
    let mut b = Plane::new(w, h);
    for i in 0..w * h {
        r.data[i] = interleaved[i * 3];
        g.data[i] = interleaved[i * 3 + 1];
        b.data[i] = interleaved[i * 3 + 2];
    }
    [r, g, b]
}

/// FNV-1a (64-bit) over the bit patterns of an `f32` slice.
///
/// Bit-exact determinism oracle: two runs that produce the same hash produced
/// byte-identical rasters. Used by every engine's fixed-seed evidence test.
pub fn plane_hash(data: &[f32]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for v in data {
        for b in v.to_bits().to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

/// FNV-1a (64-bit) over the bit patterns of an `f64` slice.
pub fn plane_hash_f64(data: &[f64]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for v in data {
        for b in v.to_bits().to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

/// SplitMix64 — the crate's fixed-seed test/data generator.
///
/// Chosen because it is tiny, well-known public-domain math (Steele et al.,
/// "Fast splittable pseudorandom number generators", constants are the
/// standard SplitMix64 increments), and byte-stable across platforms.
pub struct SplitMix64(pub u64);

impl SplitMix64 {
    /// Next raw 64-bit draw.
    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform draw in [0, 1).
    #[inline]
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plane_indexing_and_clamping() {
        let mut p = Plane::new(3, 2);
        *p.at_mut(2, 1) = 5.0;
        assert_eq!(p.at(2, 1), 5.0);
        assert_eq!(p.at_clamped(9, 9), 5.0);
        assert_eq!(p.at_clamped(-1, -1), p.at(0, 0));
    }

    #[test]
    fn hash_is_bit_pattern_sensitive() {
        // -0.0 and 0.0 compare equal but differ in bits: the oracle must
        // distinguish them (bit-exact claim, not numeric-equality claim).
        assert_ne!(plane_hash(&[0.0f32]), plane_hash(&[-0.0f32]));
        assert_eq!(plane_hash(&[1.5, 2.5]), plane_hash(&[1.5, 2.5]));
    }

    #[test]
    fn splitmix_is_stable() {
        // Pinned first draws of seed 1 (cross-checked against the published
        // SplitMix64 reference sequence).
        let mut r = SplitMix64(1);
        assert_eq!(r.next_u64(), 0x910a_2dec_8902_5cc1);
        assert_eq!(r.next_u64(), 0xbeeb_8da1_658e_ec67);
    }
}
