// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §2.3 (`Tile<T, C>` — the atomic storage unit), §3.1 (geometry).
//!
//! [NORMATIVE, ES §2.3/§3.1] A tile is a fixed 256×256 dense block:
//! channel-interleaved, row-major, premultiplied alpha, always `TILE*TILE`
//! pixels. Scalar index of `(x, y, c)` is `(y·256 + x)·CHANNELS + c`; rows
//! are contiguous with no padding anywhere, so the payload is one allocation
//! that SIMD kernels and GPU row-major uploads consume as-is.
//!
//! A tile carries **no** coordinates — its position is the key in the owning
//! [`super::map::TileMap`]. The runtime, depth-erased twin of this generic
//! type is [`super::storage::TileStorage`]; `Tile<T, C>` is the type generic
//! kernels are written against (one implementation per operation, ES §2.2).
//!
//! Working-space conversion (`to_rgba_linear`/`from_rgba_linear`) requires
//! the color pipeline's `ColorContext` (ES §5) and lands with the color lane
//! at CP-0.5.

use std::marker::PhantomData;

use super::channel::Channel;
use super::model::ColorModel;
use super::TILE;

/// A dense 256×256 pixel block (ES §2.3).
#[derive(Clone)]
pub struct Tile<T: Channel, C: ColorModel> {
    /// `TILE*TILE*C::CHANNELS` scalars, row-major, channel-interleaved.
    data: Box<[T]>,
    _model: PhantomData<C>,
}

impl<T: Channel, C: ColorModel> Tile<T, C> {
    /// Scalar count of one full tile payload.
    pub const SCALARS: usize = TILE * TILE * C::CHANNELS;

    /// All-zero tile (transparent black — every storage scalar zero, the
    /// §2.4 absence value).
    pub fn zeroed() -> Self {
        Tile {
            data: vec![T::ZERO; Self::SCALARS].into_boxed_slice(),
            _model: PhantomData,
        }
    }

    /// Solid fill from up to four channel scalars (`v[..CHANNELS]` used).
    /// Models with more than four channels (`CmykA`) cannot use this
    /// constructor (ES §2.3 signature; debug-asserted).
    pub fn filled(v: [T; 4]) -> Self {
        debug_assert!(
            C::CHANNELS <= 4,
            "Tile::filled is defined for models with <= 4 channels"
        );
        let mut t = Self::zeroed();
        for px in t.data.chunks_exact_mut(C::CHANNELS) {
            px.copy_from_slice(&v[..C::CHANNELS]);
        }
        t
    }

    #[inline]
    pub fn pixel(&self, x: usize, y: usize) -> &[T] {
        // In-tile coordinates are < TILE on both axes; a wrapped x would
        // silently alias into the next row (debug-checked, like every other
        // in-tile addressing boundary).
        debug_assert!(x < TILE && y < TILE, "in-tile coordinate out of range");
        let i = (y * TILE + x) * C::CHANNELS;
        &self.data[i..i + C::CHANNELS]
    }

    #[inline]
    pub fn pixel_mut(&mut self, x: usize, y: usize) -> &mut [T] {
        debug_assert!(x < TILE && y < TILE, "in-tile coordinate out of range");
        let i = (y * TILE + x) * C::CHANNELS;
        &mut self.data[i..i + C::CHANNELS]
    }

    pub fn as_slice(&self) -> &[T] {
        &self.data
    }

    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.data
    }

    /// Consume into the raw payload (used to hand a generic-kernel result to
    /// the depth-erased storage layer).
    pub fn into_payload(self) -> Box<[T]> {
        self.data
    }
}

#[cfg(test)]
mod tests {
    use super::super::model::model;
    use super::*;

    #[test]
    fn addressing_is_row_major_channel_interleaved() {
        let mut t: Tile<u8, model::Rgba> = Tile::zeroed();
        t.pixel_mut(3, 2).copy_from_slice(&[10, 20, 30, 40]);
        // Scalar index of (x=3, y=2, c=0) = (2*256 + 3) * 4 (ES §2.3 worked
        // layout example).
        let i = (2 * TILE + 3) * 4;
        assert_eq!(&t.as_slice()[i..i + 4], &[10, 20, 30, 40]);
        assert_eq!(t.pixel(3, 2), &[10, 20, 30, 40]);
        // Neighbors untouched.
        assert_eq!(t.pixel(2, 2), &[0, 0, 0, 0]);
        assert_eq!(t.pixel(3, 1), &[0, 0, 0, 0]);
    }

    #[test]
    fn payload_length_per_model() {
        assert_eq!(Tile::<u8, model::Rgba>::SCALARS, 256 * 256 * 4);
        assert_eq!(Tile::<f32, model::Gray>::SCALARS, 256 * 256);
        assert_eq!(Tile::<u8, model::CmykA>::SCALARS, 256 * 256 * 5);
        let t: Tile<f32, model::Gray> = Tile::zeroed();
        assert_eq!(t.as_slice().len(), 256 * 256);
    }

    #[test]
    fn corner_pixels_are_addressable() {
        let mut t: Tile<f32, model::Gray> = Tile::zeroed();
        t.pixel_mut(0, 0)[0] = 0.25;
        t.pixel_mut(TILE - 1, TILE - 1)[0] = 0.75;
        assert_eq!(t.pixel(0, 0)[0], 0.25);
        assert_eq!(t.pixel(TILE - 1, TILE - 1)[0], 0.75);
        // Last scalar of the payload is the last pixel (rows contiguous, no
        // padding).
        assert_eq!(*t.as_slice().last().unwrap(), 0.75);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "in-tile coordinate out of range")]
    fn out_of_bounds_pixel_panics() {
        // x = TILE would otherwise alias into row y+1 (row-major layout) —
        // the debug boundary refuses it.
        let t: Tile<u8, model::Rgba> = Tile::zeroed();
        let _ = t.pixel(TILE, 0);
    }

    #[test]
    fn filled_replicates_the_first_channels() {
        let t: Tile<u8, model::GrayA> = Tile::filled([9, 200, 0, 0]);
        assert_eq!(t.pixel(0, 0), &[9, 200]);
        assert_eq!(t.pixel(255, 255), &[9, 200]);
    }
}
