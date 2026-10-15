// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Corpus-shared helpers: the un-premultiply discipline several §16.3
//! blocks demand ("sharpening premultiplied data would sharpen the alpha
//! ramp into halos"), and the [NORMATIVE] §16.3 stochastic-filter RNG rule
//! `rng = Pcg32::new(state: seed ^ blake3_64(tile.x, tile.y, lod),
//! stream: pass_index)` — deterministic, replayable, tile-order
//! independent (BLAKE3 via appname-core's ContentHash; the first 8 LE
//! bytes are the 64-bit fold, pinned).

use appname_core::{ContentHash, IRect};
use appname_tools::brush::rng::Pcg32;
use appname_tools::resample::{EdgePolicy, Plane};

/// Un-premultiplied view of one pixel (`c/a`, alpha kept; `a = 0` → zeros)
/// — core's §6.4 helper semantics.
#[inline]
pub fn unpremul(p: [f32; 4]) -> [f32; 4] {
    appname_core::io::srgb::unpremul(p)
}

/// Re-premultiply after an un-premultiplied edit.
#[inline]
pub fn premul(p: [f32; 4]) -> [f32; 4] {
    [p[0] * p[3], p[1] * p[3], p[2] * p[3], p[3]]
}

/// §16.3 [NORMATIVE] stochastic seeding.
pub fn tile_rng(seed: u64, tile_x: i32, tile_y: i32, lod: u8, pass: u64) -> Pcg32 {
    let h = ContentHash::from_stream(|hh| {
        hh.update(&tile_x.to_le_bytes());
        hh.update(&tile_y.to_le_bytes());
        hh.update(&[lod]);
    });
    let fold = u64::from_le_bytes(h.as_bytes()[..8].try_into().expect("8 bytes"));
    Pcg32::new(seed ^ fold, pass)
}

/// Copy the overlap of `input` into a fresh plane over `rect` with clamp
/// reads (the §16.3 "clamp" edge policy at the plane boundary).
pub fn clamped_copy(input: &Plane, rect: IRect) -> Plane {
    let mut out = Plane::new(rect);
    for y in rect.min.y..rect.max.y {
        for x in rect.min.x..rect.max.x {
            out.set(x, y, input.get(x, y, EdgePolicy::Clamp));
        }
    }
    out
}
