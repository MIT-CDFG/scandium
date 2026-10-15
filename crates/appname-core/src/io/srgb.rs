// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §4.4/§6.4 boundary math; plan §2.1 row "PNG import/export —
//! sRGB↔linear f32 at boundaries"; ported from the proto's `color.rs`
//! (same authorship — the IEC 61966-2-1 piecewise pair).
//!
//! INTERIM CP-0.1 COLOR ASSUMPTION (documented per the plan row; the §5
//! CMM replaces this at CP-0.5):
//! * u8/u16 storage carries **sRGB-transfer-encoded** component values whose
//!   premultiplied state is only exercised at alpha ∈ {0, 1} (doc.new
//!   backgrounds are opaque or absent), so decode-then-premultiply is exact
//!   for every reachable pixel;
//! * f32 storage carries **linear-light premultiplied** working values
//!   (imported layers) — no transfer curve;
//! * the PNG boundary does all conversion in linear f32: decode sRGB →
//!   linear, premultiply in linear on import; composite in linear premult,
//!   unpremultiply, encode linear → sRGB on export (§6.4 halo rule).
//! * profiles are the slice-2 placeholder ids; no ICC data is read or
//!   embedded yet.

/// sRGB-encoded component (0..1) → linear light (IEC 61966-2-1).
#[inline]
pub fn srgb_decode(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// 256-entry decode LUT for 8-bit components (O-WATCH review #1 MAJOR-3,
/// core half): built ONCE from the scalar function at first use, so every
/// entry is bit-identical to `srgb_decode(v as f32 / 255.0)` by
/// construction — hot paths (PNG import decode, the export conversion
/// loop) index instead of calling `powf` per pixel. 16-bit components keep
/// the scalar path (a 65536-entry table is not this fix).
static SRGB_DECODE_U8: std::sync::LazyLock<[f32; 256]> = std::sync::LazyLock::new(|| {
    let mut lut = [0.0f32; 256];
    for (i, slot) in lut.iter_mut().enumerate() {
        *slot = srgb_decode(i as f32 / 255.0);
    }
    lut
});

/// LUT-backed decode of an 8-bit sRGB component.
#[inline]
pub fn srgb_decode_u8(v: u8) -> f32 {
    SRGB_DECODE_U8[v as usize]
}

/// Linear light (0..1) → sRGB-encoded component.
#[inline]
pub fn srgb_encode(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// Linear premultiplied → linear straight; `a = 0` maps to zeros (§6.4:
/// premultiplied storage keeps no hidden color).
#[inline]
pub fn unpremul(p: [f32; 4]) -> [f32; 4] {
    let a = p[3];
    if a > 0.0 {
        [p[0] / a, p[1] / a, p[2] / a, a]
    } else {
        [0.0, 0.0, 0.0, 0.0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_pair_round_trips_within_f32_tolerance() {
        for i in 0..=255u32 {
            let c = i as f32 / 255.0;
            let back = srgb_encode(srgb_decode(c));
            assert!((back - c).abs() < 1e-6, "c={c} back={back}");
        }
        assert_eq!(srgb_decode(0.0), 0.0);
        assert!((srgb_decode(1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn u8_srgb_round_trip_is_exact_after_requantization() {
        for i in 0..=255u32 {
            let c = i as f32 / 255.0;
            let re = (srgb_encode(srgb_decode(c)) * 255.0).round() as u32;
            assert_eq!(re, i);
        }
    }

    #[test]
    fn decode_lut_is_bit_exact_vs_the_scalar_path_for_all_256_values() {
        // MAJOR-3 acceptance: exhaustive 0..=255, to_bits equality.
        for v in 0..=255u8 {
            let lut = srgb_decode_u8(v);
            let scalar = srgb_decode(v as f32 / 255.0);
            assert_eq!(
                lut.to_bits(),
                scalar.to_bits(),
                "v={v}: lut {lut} != scalar {scalar}"
            );
        }
    }

    #[test]
    fn unpremul_guards_zero_alpha() {
        assert_eq!(unpremul([0.25, 0.0, 0.1, 0.0]), [0.0, 0.0, 0.0, 0.0]);
        let s = unpremul([0.25, 0.5, 0.125, 0.5]);
        assert_eq!(s, [0.5, 1.0, 0.25, 0.5]);
    }
}
