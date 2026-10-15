// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §4.4 (working space: linear light, premultiplied alpha), §4.8
//! (`TileRgba` — the CPU reference working tile: `TILE·TILE·4` interleaved
//! f32), §6.4 (premultiplied rules; straight→working conversion; HDR
//! super-bright premult is valid at f32), §2.3 (tile layout: row-major,
//! channel-interleaved); DD ⑤.
//!
//! The working tile of the CPU reference compositor: one 256×256 RGBA tile
//! as a flat `Box<[f32]>` of `TILE·TILE·4` scalars, row-major and
//! channel-interleaved — the ES §4.8 layout, chosen so the later explicit
//! SIMD pass (plan §2.2: "SIMD-friendly layout now, explicit SIMD later")
//! can gather 8-lane channel planes without a data-structure change. All
//! values are **linear-light premultiplied** (§4.4); HDR values above 1.0
//! pass through unclamped (§6.4).
//!
//! Storage→working conversion honors the CP-0.1 interim color convention
//! documented at [`appname_core::io::srgb`] (module doc) so this crate and
//! core's export composite read identical working values from identical
//! storage:
//!
//! * `F32` storage is linear; `premultiplied` flag honored (premult passes
//!   through; straight multiplies by alpha per the §6.4 straight→working
//!   row).
//! * `U8`/`U16` (unsigned-normalized) storage carries sRGB-encoded
//!   components + linear alpha, decoded `srgb_decode(c) · a` — exactly
//!   core's `layer_pixel_linear` rule, premultiplied state exercised at
//!   `a ∈ {0, 1}` (interim until the §5 color lane; divergence here would
//!   break the cross-check suite, which pins it).

use std::sync::LazyLock;

use appname_core::io::srgb::srgb_decode;
use appname_core::{ChannelKind, TilePixels, TileStorage, TILE};

/// Scalars in one working tile (`TILE² · 4`).
pub const TILE_SCALARS: usize = TILE * TILE * 4;

/// 256-entry sRGB→linear decode table for the u8 storage domain
/// (O-WATCH review #1 MAJOR-3a: the scalar decode's `powf(2.4)` ran per
/// non-toe component inside per-pixel conversion loops — ~196k `powf` per
/// dense u8 tile — squarely in the CP-0.3 brush-echo latency path).
///
/// Built at first use FROM the scalar [`srgb_decode`] itself, so every
/// entry is **bit-exact by construction**: `LUT[k] ≡ srgb_decode(k/255)`
/// — the identical f32 the per-pixel call produced. Exhaustively proven
/// for all 256 bytes by `lut_is_bitexact_to_scalar_decode_exhaustive`;
/// the cross-check suite (render vs core's export, which decodes through
/// the same scalar function) is the document-level goldens-unchanged
/// proof. std-only (`LazyLock`, stable), zero unsafe.
static SRGB_U8_DECODE: LazyLock<[f32; 256]> = LazyLock::new(|| {
    let mut lut = [0.0f32; 256];
    for (k, slot) in lut.iter_mut().enumerate() {
        *slot = srgb_decode(k as f32 / 255.0);
    }
    lut
});

/// One 256×256 linear premultiplied f32 RGBA working tile (ES §4.8).
///
/// This is transient render state, not document state: writing it never
/// touches a document, so exposing constructors/writers here does not open
/// a D-48 side door (documents mutate only through the core command engine).
#[derive(Clone, PartialEq)]
pub struct TileRgba {
    px: Box<[f32]>,
}

impl TileRgba {
    /// Fully transparent tile (all scalars 0.0 — the §4.3 defined read for
    /// absent content).
    pub fn transparent() -> TileRgba {
        TileRgba {
            px: vec![0.0f32; TILE_SCALARS].into_boxed_slice(),
        }
    }

    /// Every pixel set to `v` (working-space linear premultiplied RGBA).
    pub fn splat(v: [f32; 4]) -> TileRgba {
        let mut px = vec![0.0f32; TILE_SCALARS].into_boxed_slice();
        for p in px.chunks_exact_mut(4) {
            p.copy_from_slice(&v);
        }
        TileRgba { px }
    }

    /// Read one pixel (tile-local coordinates).
    #[inline]
    pub fn get(&self, x: usize, y: usize) -> [f32; 4] {
        debug_assert!(x < TILE && y < TILE);
        let i = (y * TILE + x) * 4;
        [self.px[i], self.px[i + 1], self.px[i + 2], self.px[i + 3]]
    }

    /// Write one pixel (tile-local coordinates) — working-space value,
    /// linear premultiplied.
    #[inline]
    pub fn set(&mut self, x: usize, y: usize, v: [f32; 4]) {
        debug_assert!(x < TILE && y < TILE);
        let i = (y * TILE + x) * 4;
        self.px[i..i + 4].copy_from_slice(&v);
    }

    /// Flat scalar view (`TILE·TILE·4`, row-major RGBA interleaved).
    #[inline]
    pub fn as_slice(&self) -> &[f32] {
        &self.px
    }

    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [f32] {
        &mut self.px
    }

    /// One row's span `[x0, x1)` as a flat RGBA slice.
    #[inline]
    pub fn row_span(&self, y: usize, x0: usize, x1: usize) -> &[f32] {
        debug_assert!(y < TILE && x0 <= x1 && x1 <= TILE);
        &self.px[(y * TILE + x0) * 4..(y * TILE + x1) * 4]
    }

    /// Convert one storage tile (RGBA model) to the working representation.
    ///
    /// `kind`/`premultiplied` come from the owning [`appname_core::Buffer`].
    /// Conversion rules per the module doc; the storage-kind dispatch runs
    /// **once per tile**, never per pixel (the §4.8 "match once per tile"
    /// pattern). The wildcard arm covers depths this build may compile in
    /// behind core features (`depth-u16`) via the generic per-pixel reader —
    /// same rule, undispatched.
    pub fn from_storage(storage: &TileStorage, kind: ChannelKind, premultiplied: bool) -> TileRgba {
        debug_assert_eq!(
            storage.model().map(|m| m.channels()).unwrap_or(4),
            4,
            "from_storage is RGBA-only at CP-0.3 (flatten refuses other models)"
        );
        // The wildcard arm is unreachable unless core's feature-gated
        // depth variants (`depth-u16`) are compiled in by feature
        // unification — kept so a CI feature build changes nothing here.
        #[allow(unreachable_patterns)]
        match storage {
            TileStorage::Uniform { value } => {
                TileRgba::splat(convert_one(*value, kind, premultiplied))
            }
            TileStorage::U8(p) => TileRgba::from_unsigned_dense(p),
            TileStorage::F32(p) => TileRgba::from_f32_dense(p, premultiplied),
            // Feature-gated depths (core `depth-u16`) and future variants:
            // per-pixel generic read, identical conversion rule.
            other => {
                let model = other.model().expect("dense storage carries a model");
                let mut out = TileRgba::transparent();
                for y in 0..TILE {
                    for x in 0..TILE {
                        let v = other.read_pixel_f32(x, y, model);
                        out.set(x, y, convert_one(v, kind, premultiplied));
                    }
                }
                out
            }
        }
    }

    /// U8 dense: interim sRGB convention (module doc) — decode components,
    /// multiply by alpha in linear. The decode is the [`SRGB_U8_DECODE`]
    /// table indexed by the raw byte — bit-exact to the scalar call it
    /// replaced (`LUT[k] ≡ srgb_decode(k.to_f32())` since
    /// `k.to_f32() = k/255`); alpha stays the linear `to_f32` (no TRC).
    fn from_unsigned_dense(p: &TilePixels<u8>) -> TileRgba {
        use appname_core::Channel;
        let lut = &*SRGB_U8_DECODE;
        let src = p.as_slice();
        let mut px = vec![0.0f32; TILE_SCALARS].into_boxed_slice();
        for (o, s) in px.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
            let a = s[3].to_f32();
            o[0] = lut[s[0] as usize] * a;
            o[1] = lut[s[1] as usize] * a;
            o[2] = lut[s[2] as usize] * a;
            o[3] = a;
        }
        TileRgba { px }
    }

    /// F32 dense: linear values; premult passes through, straight
    /// multiplies by alpha (§6.4 straight→working row).
    fn from_f32_dense(p: &TilePixels<f32>, premultiplied: bool) -> TileRgba {
        let src = p.as_slice();
        let mut px = vec![0.0f32; TILE_SCALARS].into_boxed_slice();
        if premultiplied {
            px.copy_from_slice(src);
        } else {
            for (o, s) in px.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                let a = s[3];
                o[0] = s[0] * a;
                o[1] = s[1] * a;
                o[2] = s[2] * a;
                o[3] = a;
            }
        }
        TileRgba { px }
    }
}

/// Summary Debug (ES §1.6: diagnostics never dump pixel payloads).
impl core::fmt::Debug for TileRgba {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TileRgba")
            .field("scalars", &TILE_SCALARS)
            .finish()
    }
}

/// One storage-normalized RGBA value → working linear premultiplied RGBA
/// (per-depth rule from the module doc). `pub(crate)`: the compositor's
/// Uniform fast path converts the tile's single value through exactly
/// this function (MINOR-5), so uniform and dense sources share one rule.
///
/// U8 routes through [`SRGB_U8_DECODE`] by recovering the byte index:
/// [NORMATIVE ES §2.3] a `Uniform` value in a u8 map is canonicalized
/// through the depth (`value[c] = u8::to_f32(u8::from_f32(raw))`, i.e.
/// exactly `k/255`), so `round(v·255)` recovers `k` losslessly and
/// `LUT[k] = srgb_decode(k/255) = srgb_decode(v)` bit-exactly — proven
/// exhaustively for all 256 canonical values in
/// `convert_one_u8_lut_matches_scalar_for_all_canonical_bytes`. The
/// saturating `.min(255)` guard only engages on non-canonical inputs,
/// which the §2.3 invariant excludes (they would snap to the nearest
/// canonical byte instead of decoding a value no u8 storage can hold).
/// Other unsigned depths (u16, feature-gated) keep the scalar decode —
/// the 256-entry table covers only the u8 domain (MAJOR-3a scope).
#[inline]
pub(crate) fn convert_one(v: [f32; 4], kind: ChannelKind, premultiplied: bool) -> [f32; 4] {
    if kind == ChannelKind::U8 {
        let lut = &*SRGB_U8_DECODE;
        let a = v[3];
        let idx = |c: f32| (((c * 255.0).round()) as usize).min(255);
        [
            lut[idx(v[0])] * a,
            lut[idx(v[1])] * a,
            lut[idx(v[2])] * a,
            a,
        ]
    } else if kind.is_unsigned_normalized() {
        let a = v[3];
        [
            srgb_decode(v[0]) * a,
            srgb_decode(v[1]) * a,
            srgb_decode(v[2]) * a,
            a,
        ]
    } else if premultiplied {
        v
    } else {
        let a = v[3];
        [v[0] * a, v[1] * a, v[2] * a, a]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use appname_core::io::srgb::srgb_decode;

    #[test]
    fn transparent_is_all_zero_and_splat_fills() {
        let t = TileRgba::transparent();
        assert!(t.as_slice().iter().all(|v| *v == 0.0));
        let s = TileRgba::splat([0.25, 0.5, 0.75, 1.0]);
        assert_eq!(s.get(0, 0), [0.25, 0.5, 0.75, 1.0]);
        assert_eq!(s.get(255, 255), [0.25, 0.5, 0.75, 1.0]);
    }

    #[test]
    fn set_get_round_trips_hdr_values_unclamped() {
        // §6.4: super-bright HDR premult (color > alpha, alpha > 0) is
        // valid at f32; the working tile must not clamp.
        let mut t = TileRgba::transparent();
        t.set(3, 7, [3.5, 0.0, 1.25, 0.5]);
        assert_eq!(t.get(3, 7), [3.5, 0.0, 1.25, 0.5]);
    }

    #[test]
    fn row_span_is_the_flat_interleaved_window() {
        let mut t = TileRgba::transparent();
        t.set(10, 4, [0.1, 0.2, 0.3, 0.4]);
        t.set(11, 4, [0.5, 0.6, 0.7, 0.8]);
        let span = t.row_span(4, 10, 12);
        assert_eq!(span, &[0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8]);
    }

    #[test]
    fn uniform_storage_converts_by_depth_rule() {
        // f32 premult: pass-through (linear premultiplied already).
        let u = TileStorage::Uniform {
            value: [0.25, 0.5, 0.125, 0.5],
        };
        let t = TileRgba::from_storage(&u, ChannelKind::F32, true);
        assert_eq!(t.get(0, 0), [0.25, 0.5, 0.125, 0.5]);

        // f32 straight: ×a (§6.4 straight→working row).
        let t = TileRgba::from_storage(&u, ChannelKind::F32, false);
        assert_eq!(t.get(0, 0), [0.125, 0.25, 0.0625, 0.5]);

        // u8 interim convention: srgb_decode(c)·a, alpha linear — the exact
        // core `layer_pixel_linear` rule (cross-check anchor). Values are
        // CANONICAL u8 (`k/255`, the §2.3 Uniform invariant a real u8 map
        // guarantees; `convert_one`'s LUT routing snaps only these).
        use appname_core::Channel;
        let (mid, half) = ((128u8).to_f32(), (127u8).to_f32());
        let u8v = TileStorage::Uniform {
            value: [1.0, mid, 0.0, half],
        };
        let t = TileRgba::from_storage(&u8v, ChannelKind::U8, true);
        let got = t.get(128, 200);
        assert_eq!(got[0], srgb_decode(1.0) * half);
        assert_eq!(got[1], srgb_decode(mid) * half);
        assert_eq!(got[2], 0.0);
        assert_eq!(got[3], half);
    }

    #[test]
    fn f32_uniform_hdr_passes_through() {
        let u = TileStorage::Uniform {
            value: [2.0, 3.5, 0.0, 1.0],
        };
        let t = TileRgba::from_storage(&u, ChannelKind::F32, true);
        assert_eq!(t.get(0, 0), [2.0, 3.5, 0.0, 1.0]);
    }

    /// MAJOR-3a bit-exactness, exhaustive: every LUT entry equals the
    /// scalar decode of the same byte — `to_bits` equality, all 256.
    #[test]
    fn lut_is_bitexact_to_scalar_decode_exhaustive() {
        for k in 0..=255usize {
            let lut = super::SRGB_U8_DECODE[k];
            let scalar = srgb_decode(k as f32 / 255.0);
            assert_eq!(
                lut.to_bits(),
                scalar.to_bits(),
                "byte {k}: LUT {lut} != scalar {scalar}"
            );
        }
    }

    /// MAJOR-3a byte recovery, exhaustive: for every canonical u8 value
    /// `v = k/255` (the §2.3 Uniform invariant), the LUT-routed
    /// `convert_one` is bit-identical to the pre-LUT scalar formula
    /// `srgb_decode(v)·a`, across alphas 0 / fractional / 1.
    #[test]
    fn convert_one_u8_lut_matches_scalar_for_all_canonical_bytes() {
        use appname_core::Channel;
        for k in 0..=255u32 {
            let v = (k as u8).to_f32(); // exactly k/255 — the canonical value
            for a in [0.0f32, 0.5, 128.0 / 255.0, 1.0] {
                let got = super::convert_one([v, v, v, a], ChannelKind::U8, true);
                let want = srgb_decode(v) * a;
                for (c, &g) in got.iter().enumerate().take(3) {
                    assert_eq!(
                        g.to_bits(),
                        want.to_bits(),
                        "byte {k} alpha {a}: channel {c} {g} != {want}"
                    );
                }
                assert_eq!(got[3].to_bits(), a.to_bits());
            }
        }
    }

    /// The recovery index is exact for every canonical byte (no
    /// neighbor-bucket drift), and the saturation guard holds at the
    /// domain edges.
    #[test]
    fn u8_byte_recovery_is_lossless_and_guarded() {
        use appname_core::Channel;
        for k in 0..=255u32 {
            let v = (k as u8).to_f32();
            assert_eq!(
                ((v * 255.0).round()) as usize,
                k as usize,
                "byte {k} round-trips"
            );
        }
        // Out-of-domain inputs (excluded by §2.3, guarded anyway): the
        // saturating cast pins them to the domain edges, no panic.
        let hi = super::convert_one([2.0, 2.0, 2.0, 1.0], ChannelKind::U8, true);
        assert_eq!(hi[0].to_bits(), srgb_decode(1.0).to_bits());
        let lo = super::convert_one([-1.0, -1.0, -1.0, 1.0], ChannelKind::U8, true);
        assert_eq!(lo[0].to_bits(), srgb_decode(0.0).to_bits());
    }
}
