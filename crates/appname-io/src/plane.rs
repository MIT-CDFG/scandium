// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §8.6 "Orientation → transform table" [NORMATIVE], §6.4
//! (premultiplied boundary math), §8.1 (decode → `Imported.buffer`);
//! DD ⑥⑫; plan §2.3 row "Codecs v2".
//!
//! The codec interchange surface: every decoder produces an [`RgbaPlane`]
//! (straight-alpha, transfer-encoded, row-major RGBA at 8 or 16 bits) and
//! every encoder consumes the linear premultiplied f32 composite plane the
//! established export path yields (core `export_png_bytes` / render
//! `region_rgba_f32` — the wave-2 arbitration-3 pair).
//!
//! **The ingestion door (journaled since main a857cfb).** Core's mutation
//! surface is command-spine-gated; with the MAJOR-2 reconciliation LANDED,
//! imports are journaled `edit.paste` envelopes over content-addressed
//! raster assets, and core's asset ingress (`Engine::add_raster_asset`)
//! accepts encoded PNG. [`RgbaPlane::to_png_and_buffer`] therefore
//! synthesizes ONE deterministic PNG transcode of the decoded plane (8- or
//! 16-bit, lossless by construction) that serves BOTH roles: the asset
//! bytes the journaled envelope references, and — via
//! `appname_core::decode_png_to_buffer`, the same boundary core's asset
//! re-decode uses, with the same documented interim sRGB→linear
//! premultiplied conversion (`appname_core::io::srgb` module doc) — the
//! working [`Buffer`] handed back in `Imported`. One byte source, one
//! color-math authority: what the caller inspects and what replay
//! reproduces are the same pixels by construction.
//!
//! Determinism: all conversions are pure byte-driven functions; the PNG
//! hop uses a fixed encoder configuration; no wall clock, no threads.

use appname_core::io::srgb::{srgb_encode, unpremul};
use appname_core::{decode_png_to_buffer, Buffer, ProfileId};

use crate::error::CodecError;

/// Straight-alpha, transfer-encoded (sRGB-assumed under the CP-0.5 interim
/// color assumption), row-major RGBA pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanePixels {
    B8(Vec<u8>),
    B16(Vec<u16>),
}

/// The decoder-side interchange image (see module doc).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaPlane {
    pub width: u32,
    pub height: u32,
    pub pixels: PlanePixels,
}

impl RgbaPlane {
    pub fn new8(width: u32, height: u32, rgba: Vec<u8>) -> Result<RgbaPlane, CodecError> {
        if rgba.len() != width as usize * height as usize * 4 {
            return Err(CodecError::internal("plane length must be w*h*4"));
        }
        Ok(RgbaPlane {
            width,
            height,
            pixels: PlanePixels::B8(rgba),
        })
    }

    pub fn new16(width: u32, height: u32, rgba: Vec<u16>) -> Result<RgbaPlane, CodecError> {
        if rgba.len() != width as usize * height as usize * 4 {
            return Err(CodecError::internal("plane length must be w*h*4"));
        }
        Ok(RgbaPlane {
            width,
            height,
            pixels: PlanePixels::B16(rgba),
        })
    }

    /// §8.6 orientation bake [NORMATIVE]: source pixel `(x, y)` maps to
    /// upright `(x', y')` per the transform table; values 5–8 swap output
    /// dimensions; a lossless permutation, no resampling. Values outside
    /// 1–8 are the CALLER's business (treated-as-1 + `CompatItem` per the
    /// spec rule) — this function accepts 1..=8 only.
    pub fn apply_orientation(self, v: u16) -> Result<RgbaPlane, CodecError> {
        if !(1..=8).contains(&v) {
            return Err(CodecError::internal(format!(
                "orientation {v} outside 1..=8"
            )));
        }
        if v == 1 {
            return Ok(self);
        }
        let (w, h) = (self.width as usize, self.height as usize);
        let swap = v >= 5;
        let (ow, oh) = if swap { (h, w) } else { (w, h) };
        // Upright coordinates of source (x, y), per the table.
        let map = |x: usize, y: usize| -> (usize, usize) {
            match v {
                2 => (w - 1 - x, y),
                3 => (w - 1 - x, h - 1 - y),
                4 => (x, h - 1 - y),
                5 => (y, x),
                6 => (h - 1 - y, x),
                7 => (h - 1 - y, w - 1 - x),
                8 => (y, w - 1 - x),
                _ => unreachable!(),
            }
        };
        fn permute<T: Copy + Default>(
            src: &[T],
            w: usize,
            h: usize,
            ow: usize,
            map: impl Fn(usize, usize) -> (usize, usize),
        ) -> Vec<T> {
            let mut out = vec![T::default(); src.len()];
            for y in 0..h {
                for x in 0..w {
                    let (nx, ny) = map(x, y);
                    let s = (y * w + x) * 4;
                    let d = (ny * ow + nx) * 4;
                    out[d..d + 4].copy_from_slice(&src[s..s + 4]);
                }
            }
            out
        }
        let pixels = match &self.pixels {
            PlanePixels::B8(p) => PlanePixels::B8(permute(p, w, h, ow, map)),
            PlanePixels::B16(p) => PlanePixels::B16(permute(p, w, h, ow, map)),
        };
        Ok(RgbaPlane {
            width: ow as u32,
            height: oh as u32,
            pixels,
        })
    }

    /// The ingestion door (module doc): plane → linear premultiplied f32
    /// [`Buffer`] through core's public PNG boundary. Lossless by
    /// construction (PNG is; the 8/16-bit depth is preserved into the
    /// transcode).
    pub fn to_buffer(&self, working: ProfileId) -> Result<Buffer, CodecError> {
        Ok(self.to_png_and_buffer(working)?.1)
    }

    /// The journaled-ingestion pair: the SAME synthesized PNG bytes both
    /// as the `Engine::add_raster_asset` carrier (the MAJOR-2
    /// reconciliation landed: imports are journaled `edit.paste` envelopes
    /// over content-addressed raster assets, and core's asset ingress
    /// accepts PNG) and decoded through core's boundary into the working
    /// [`Buffer`] — byte-coupled, so `Imported.buffer` and what a replayed
    /// `edit.paste` reproduces are the same pixels by construction.
    pub fn to_png_and_buffer(&self, working: ProfileId) -> Result<(Vec<u8>, Buffer), CodecError> {
        let png_bytes = self.to_png_bytes()?;
        let (buffer, _, _) = decode_png_to_buffer(&png_bytes, working)
            .map_err(|e| CodecError::internal(format!("core ingestion door: {e}")))?;
        Ok((png_bytes, buffer))
    }

    /// Synthesize the deterministic PNG transcode of this plane (fixed,
    /// version-pinned encoder configuration).
    pub fn to_png_bytes(&self) -> Result<Vec<u8>, CodecError> {
        let mut png_bytes = Vec::new();
        {
            let mut enc = png::Encoder::new(
                std::io::Cursor::new(&mut png_bytes),
                self.width,
                self.height,
            );
            enc.set_color(png::ColorType::Rgba);
            // Fixed, version-pinned encoder configuration (determinism
            // note: the hop is decode-side only, so even a future png-crate
            // compression change cannot alter decoded pixel values).
            enc.set_compression(png::Compression::Fast);
            match &self.pixels {
                PlanePixels::B8(p) => {
                    enc.set_depth(png::BitDepth::Eight);
                    let mut w = enc.write_header().map_err(CodecError::internal)?;
                    w.write_image_data(p).map_err(CodecError::internal)?;
                }
                PlanePixels::B16(p) => {
                    enc.set_depth(png::BitDepth::Sixteen);
                    let mut be = Vec::with_capacity(p.len() * 2);
                    for v in p {
                        be.extend_from_slice(&v.to_be_bytes());
                    }
                    let mut w = enc.write_header().map_err(CodecError::internal)?;
                    w.write_image_data(&be).map_err(CodecError::internal)?;
                }
            }
        }
        Ok(png_bytes)
    }
}

/// Export-side conversion: one LINEAR PREMULTIPLIED f32 RGBA plane
/// (row-major, the established composite output) → straight sRGB 8-bit
/// RGBA, the same §6.4 math (unpremultiply, then transfer-encode, then
/// quantize round-half-up) as core's `encode_composite_png`, so
/// single-layer paths stay byte-compatible across formats.
pub fn linear_plane_to_srgb8(w: u32, h: u32, plane: &[f32]) -> Result<Vec<u8>, CodecError> {
    if plane.len() != w as usize * h as usize * 4 {
        return Err(CodecError::internal("plane length must be w*h*4"));
    }
    let mut out = Vec::with_capacity(plane.len());
    for px in plane.chunks_exact(4) {
        let s = unpremul([px[0], px[1], px[2], px[3]]);
        out.push((srgb_encode(s[0].clamp(0.0, 1.0)) * 255.0).round() as u8);
        out.push((srgb_encode(s[1].clamp(0.0, 1.0)) * 255.0).round() as u8);
        out.push((srgb_encode(s[2].clamp(0.0, 1.0)) * 255.0).round() as u8);
        out.push((s[3].clamp(0.0, 1.0) * 255.0).round() as u8);
    }
    Ok(out)
}

/// Alpha-less-target conversion (§8.1 `EncodeOpts::matte`): source-over
/// the opaque matte in LINEAR light first (§4.4), then transfer-encode to
/// straight sRGB 8-bit RGB.
pub fn linear_plane_to_srgb8_over_matte(
    w: u32,
    h: u32,
    plane: &[f32],
    matte: [f32; 3],
) -> Result<Vec<u8>, CodecError> {
    if plane.len() != w as usize * h as usize * 4 {
        return Err(CodecError::internal("plane length must be w*h*4"));
    }
    let mut out = Vec::with_capacity(w as usize * h as usize * 3);
    for px in plane.chunks_exact(4) {
        let ia = 1.0 - px[3];
        for c in 0..3 {
            let lin = px[c] + matte[c] * ia; // premul source over opaque matte
            out.push((srgb_encode(lin.clamp(0.0, 1.0)) * 255.0).round() as u8);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A 3x2 plane with globally unique pixel values (r encodes the index).
    fn probe() -> RgbaPlane {
        let mut p = Vec::new();
        for i in 0..6u8 {
            p.extend_from_slice(&[i * 10, 1, 2, 255]);
        }
        RgbaPlane::new8(3, 2, p).unwrap()
    }

    fn r_at(pl: &RgbaPlane, x: u32, y: u32) -> u8 {
        match &pl.pixels {
            PlanePixels::B8(p) => p[(y as usize * pl.width as usize + x as usize) * 4],
            PlanePixels::B16(_) => unreachable!(),
        }
    }

    /// §8.6 matrix, hand-computed for the 3x2 probe:
    /// source r values laid out  0 10 20
    ///                          30 40 50
    #[test]
    fn orientation_matrix_matches_the_normative_table() {
        // (v, expected rows of r values after bake)
        let cases: [(u16, Vec<Vec<u8>>); 8] = [
            (1, vec![vec![0, 10, 20], vec![30, 40, 50]]),
            (2, vec![vec![20, 10, 0], vec![50, 40, 30]]),
            (3, vec![vec![50, 40, 30], vec![20, 10, 0]]),
            (4, vec![vec![30, 40, 50], vec![0, 10, 20]]),
            (5, vec![vec![0, 30], vec![10, 40], vec![20, 50]]),
            (6, vec![vec![30, 0], vec![40, 10], vec![50, 20]]),
            (7, vec![vec![50, 20], vec![40, 10], vec![30, 0]]),
            (8, vec![vec![20, 50], vec![10, 40], vec![0, 30]]),
        ];
        for (v, rows) in cases {
            let out = probe().apply_orientation(v).unwrap();
            let (ew, eh) = (rows[0].len() as u32, rows.len() as u32);
            assert_eq!((out.width, out.height), (ew, eh), "v={v} size");
            for (y, row) in rows.iter().enumerate() {
                for (x, want) in row.iter().enumerate() {
                    assert_eq!(r_at(&out, x as u32, y as u32), *want, "v={v} at ({x},{y})");
                }
            }
        }
    }

    #[test]
    fn orientation_bake_is_lossless_and_involutive_where_the_table_says() {
        // 3 (rot 180) and 2/4 (flips) are involutions; 6 then 8 round-trips.
        let p = probe();
        assert_eq!(
            p.clone()
                .apply_orientation(3)
                .unwrap()
                .apply_orientation(3)
                .unwrap(),
            p
        );
        assert_eq!(
            p.clone()
                .apply_orientation(2)
                .unwrap()
                .apply_orientation(2)
                .unwrap(),
            p
        );
        assert_eq!(
            p.clone()
                .apply_orientation(4)
                .unwrap()
                .apply_orientation(4)
                .unwrap(),
            p
        );
        assert_eq!(
            p.clone()
                .apply_orientation(6)
                .unwrap()
                .apply_orientation(8)
                .unwrap(),
            p
        );
        assert_eq!(
            p.clone()
                .apply_orientation(5)
                .unwrap()
                .apply_orientation(5)
                .unwrap(),
            p
        );
        assert_eq!(
            p.clone()
                .apply_orientation(7)
                .unwrap()
                .apply_orientation(7)
                .unwrap(),
            p
        );
    }

    #[test]
    fn to_buffer_rides_the_core_door_losslessly_for_8_bit() {
        use appname_core::io::srgb::srgb_decode_u8;
        use appname_core::IVec2;
        let plane = RgbaPlane::new8(2, 1, vec![188, 0, 255, 255, 10, 20, 30, 128]).unwrap();
        let buf = plane.to_buffer(ProfileId::nil()).unwrap();
        let px = buf.read_pixel_f32(IVec2::new(0, 0));
        assert_eq!(px[0], srgb_decode_u8(188));
        assert_eq!(px[2], 1.0);
        // Straight 128-alpha premultiplies in linear at the boundary.
        let px1 = buf.read_pixel_f32(IVec2::new(1, 0));
        let a = 128.0 / 255.0;
        assert!((px1[3] - a).abs() < 1e-6);
        assert!((px1[0] - srgb_decode_u8(10) * a).abs() < 1e-6);
    }

    #[test]
    fn to_buffer_preserves_16_bit_precision_through_the_door() {
        use appname_core::IVec2;
        // A value whose 16-bit precision an 8-bit hop would destroy.
        let plane = RgbaPlane::new16(1, 1, vec![0x8081, 0, 0, 0xFFFF]).unwrap();
        let buf = plane.to_buffer(ProfileId::nil()).unwrap();
        let px = buf.read_pixel_f32(IVec2::new(0, 0));
        let expect = {
            let c = 0x8081 as f32 / 65535.0;
            appname_core::io::srgb::srgb_decode(c)
        };
        assert_eq!(px[0].to_bits(), expect.to_bits());
    }

    #[test]
    fn export_conversions_agree_with_core_math() {
        // Opaque linear 0.5 encodes to sRGB 188 both with and without matte.
        let plane = [0.5f32, 0.0, 1.0, 1.0];
        let rgba = linear_plane_to_srgb8(1, 1, &plane).unwrap();
        assert_eq!(rgba, vec![188, 0, 255, 255]);
        let rgb = linear_plane_to_srgb8_over_matte(1, 1, &plane, [1.0; 3]).unwrap();
        assert_eq!(rgb, vec![188, 0, 255]);
        // Half-coverage premultiplied black over white matte = linear 0.5.
        let ghost = [0.0f32, 0.0, 0.0, 0.5];
        let rgb = linear_plane_to_srgb8_over_matte(1, 1, &ghost, [1.0; 3]).unwrap();
        assert_eq!(rgb, vec![188, 188, 188]);
    }
}
