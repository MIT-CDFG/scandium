// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Shared fixture builders for the codec-matrix suites (deterministic:
//! every fixture is a pure function of its arguments — plan §2.3
//! determinism row).

#![allow(dead_code)] // shared across suites; not every suite uses every helper

use appname_core::io::srgb::srgb_decode_u8;
use appname_core::{Buffer, IVec2};

/// Deterministic straight sRGB RGBA bytes (position-derived pattern).
pub fn srgb_bytes(w: u32, h: u32, opaque: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let a = if opaque {
                255
            } else {
                match i % 4 {
                    0 => 255,
                    1 => 0,
                    2 => 128,
                    _ => 200,
                }
            };
            out.extend_from_slice(&[
                (i * 7 % 256) as u8,
                (i * 13 % 256) as u8,
                (i * 29 % 256) as u8,
                a as u8,
            ]);
        }
    }
    out
}

/// The linear premultiplied f32 plane equivalent of [`srgb_bytes`] — the
/// same §6.4 straight→working math core's boundary applies.
pub fn linear_plane(bytes: &[u8]) -> Vec<f32> {
    let mut plane = Vec::with_capacity(bytes.len());
    for px in bytes.chunks_exact(4) {
        let a = px[3] as f32 / 255.0;
        plane.push(srgb_decode_u8(px[0]) * a);
        plane.push(srgb_decode_u8(px[1]) * a);
        plane.push(srgb_decode_u8(px[2]) * a);
        plane.push(a);
    }
    plane
}

/// Read a decoded buffer back to straight sRGB bytes (the inverse §6.4
/// math), for round-trip comparison.
pub fn buffer_to_srgb_bytes(buf: &Buffer, w: u32, h: u32) -> Vec<u8> {
    use appname_core::io::srgb::{srgb_encode, unpremul};
    let mut out = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let px = buf.read_pixel_f32(IVec2::new(x, y));
            let s = unpremul(px);
            out.push((srgb_encode(s[0].clamp(0.0, 1.0)) * 255.0).round() as u8);
            out.push((srgb_encode(s[1].clamp(0.0, 1.0)) * 255.0).round() as u8);
            out.push((srgb_encode(s[2].clamp(0.0, 1.0)) * 255.0).round() as u8);
            out.push((s[3].clamp(0.0, 1.0) * 255.0).round() as u8);
        }
    }
    out
}

/// Byte-compare with a per-channel tolerance (float unpremul/premul round
/// trips can wobble one quantization step at fractional alphas; a=0
/// pixels legitimately lose their hidden color, §6.4).
pub fn assert_bytes_close(got: &[u8], want: &[u8], tol: u8, ctx: &str) {
    assert_eq!(got.len(), want.len(), "{ctx}: length");
    for (i, (g, w)) in got.iter().zip(want.iter()).enumerate() {
        let a_index = i / 4 * 4 + 3;
        if a_index < want.len() && want[a_index] == 0 {
            // fully transparent: premultiplied storage keeps no hidden
            // color; only alpha must agree
            if i % 4 == 3 {
                assert_eq!(g, w, "{ctx}: alpha at byte {i}");
            }
            continue;
        }
        assert!(
            g.abs_diff(*w) <= tol,
            "{ctx}: byte {i} differs beyond {tol}: got {g} want {w}"
        );
    }
}
