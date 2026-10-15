// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §2.2 (pixel and color-model traits) — the storable channel scalar.
//!
//! [NORMATIVE, ES §2.2] Every pixel operation is written once, generic over
//! channel type `T` and color model `C`. **NaN never reaches storage**:
//! `from_f32` maps NaN → 0 (integer types eliminate it via `clamp`, where NaN
//! compares false and clamps to 0). Integer rounding is `f32::round` — half
//! away from zero — fixed so CPU, SIMD, and GPU readback quantization agree
//! bit-for-bit (§4.8).
//!
//! Depth roster at CP-0.1: `u8` and `f32` always; `u16` behind the
//! `depth-u16` cargo feature. `f16` is PAUSED — its impl needs the `half`
//! crate (ES §1.3), which has no deps/ record yet; [`ChannelKind::F16`]
//! exists as a tag so serialized depth vocabulary (`document.v1.json`
//! `depth` enum) is complete, but no storage variant backs it yet.

use serde::{Deserialize, Serialize};

/// A storable channel scalar (ES §2.2).
pub trait Channel: Copy + Send + Sync + 'static {
    const ZERO: Self;
    /// "White"/opaque reference value.
    const ONE: Self;
    /// → normalized working float.
    fn to_f32(self) -> f32;
    /// ← clamp/round as appropriate.
    fn from_f32(v: f32) -> Self;
    const KIND: ChannelKind;

    /// Bytes per scalar in the wire/payload encoding.
    const BYTES: usize;
    /// Write this scalar little-endian into `out` (exactly `BYTES` bytes).
    /// [NORMATIVE, ES §0.3] persisted numbers are little-endian; content
    /// hashes (§3.8) run over this same LE payload encoding so identity is
    /// platform-independent.
    fn put_le(self, out: &mut [u8]);
    /// Raw bit pattern widened to u32 — the "bitwise, float-semantics-free"
    /// comparison scans of ES §2.4 (−0.0 is content; NaN never stored).
    fn bits(self) -> u32;
}

impl Channel for u8 {
    const ZERO: Self = 0;
    const ONE: Self = 255;
    const KIND: ChannelKind = ChannelKind::U8;
    const BYTES: usize = 1;
    #[inline]
    fn to_f32(self) -> f32 {
        self as f32 / 255.0
    }
    #[inline]
    fn from_f32(v: f32) -> Self {
        (v.clamp(0.0, 1.0) * 255.0).round() as u8
    }
    #[inline]
    fn put_le(self, out: &mut [u8]) {
        out[0] = self;
    }
    #[inline]
    fn bits(self) -> u32 {
        self as u32
    }
}

#[cfg(feature = "depth-u16")]
impl Channel for u16 {
    const ZERO: Self = 0;
    const ONE: Self = 65535;
    const KIND: ChannelKind = ChannelKind::U16;
    const BYTES: usize = 2;
    #[inline]
    fn to_f32(self) -> f32 {
        self as f32 / 65535.0
    }
    #[inline]
    fn from_f32(v: f32) -> Self {
        (v.clamp(0.0, 1.0) * 65535.0).round() as u16
    }
    #[inline]
    fn put_le(self, out: &mut [u8]) {
        out[..2].copy_from_slice(&self.to_le_bytes());
    }
    #[inline]
    fn bits(self) -> u32 {
        self as u32
    }
}

impl Channel for f32 {
    const ZERO: Self = 0.0;
    /// Unbounded above (HDR/scene-linear) — ONE is the reference white.
    const ONE: Self = 1.0;
    const KIND: ChannelKind = ChannelKind::F32;
    const BYTES: usize = 4;
    #[inline]
    fn to_f32(self) -> f32 {
        self
    }
    #[inline]
    fn from_f32(v: f32) -> Self {
        // NaN scrub [NORMATIVE, ES §2.2]: a stored NaN would poison blends,
        // defeat the uniformity scan (NaN != NaN), and break content-hash
        // determinism (many bit patterns).
        if v.is_nan() {
            0.0
        } else {
            v
        }
    }
    #[inline]
    fn put_le(self, out: &mut [u8]) {
        out[..4].copy_from_slice(&self.to_le_bytes());
    }
    #[inline]
    fn bits(self) -> u32 {
        self.to_bits()
    }
}

/// Storage depth tag (ES §2.2). Serialized vocabulary is the
/// `document.v1.json` `depth` enum: `u8` | `u16` | `f16` | `f32`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChannelKind {
    U8,
    U16,
    F16,
    F32,
}

impl ChannelKind {
    /// Stable numeric encoding used inside hash framings (ES §2.5
    /// `region_hash` feeds `kind:u8`). Changing these values changes every
    /// buffer's `region_hash` — they are part of the identity definition.
    pub const fn as_u8(self) -> u8 {
        match self {
            ChannelKind::U8 => 0,
            ChannelKind::U16 => 1,
            ChannelKind::F16 => 2,
            ChannelKind::F32 => 3,
        }
    }

    pub const fn bytes_per_channel(self) -> usize {
        match self {
            ChannelKind::U8 => 1,
            ChannelKind::U16 | ChannelKind::F16 => 2,
            ChannelKind::F32 => 4,
        }
    }

    /// True for the unsigned-normalized depths, where premultiplied storage
    /// additionally implies `rgb <= a` per channel (ES §6.4 rule 1).
    pub const fn is_unsigned_normalized(self) -> bool {
        matches!(self, ChannelKind::U8 | ChannelKind::U16)
    }

    /// Depths this build can materialize pixel payloads for.
    pub const fn is_storage_backed(self) -> bool {
        match self {
            ChannelKind::U8 | ChannelKind::F32 => true,
            #[cfg(feature = "depth-u16")]
            ChannelKind::U16 => true,
            #[cfg(not(feature = "depth-u16"))]
            ChannelKind::U16 => false,
            ChannelKind::F16 => false, // PAUSED: needs the `half` crate record.
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u8_round_trip_is_exact_for_all_values() {
        for v in 0..=255u8 {
            assert_eq!(u8::from_f32(v.to_f32()), v);
        }
    }

    #[cfg(feature = "depth-u16")]
    #[test]
    fn u16_round_trip_is_exact_for_all_values() {
        for v in 0..=65535u16 {
            assert_eq!(u16::from_f32(v.to_f32()), v);
        }
    }

    #[test]
    fn nan_never_reaches_storage() {
        assert_eq!(f32::from_f32(f32::NAN), 0.0);
        assert_eq!(u8::from_f32(f32::NAN), 0); // NaN compares false, clamps to 0
        assert_eq!(u8::from_f32(-1.0), 0);
        assert_eq!(u8::from_f32(2.0), 255);
    }

    #[test]
    fn integer_rounding_is_half_away_from_zero() {
        // 0.5/255 * 255 = 0.5 exactly → rounds to 1 (away from zero), the
        // fixed §2.2 choice.
        assert_eq!(u8::from_f32(0.5 / 255.0), 1);
    }

    #[test]
    fn f32_keeps_hdr_values_and_negative_zero_bits() {
        assert_eq!(f32::from_f32(42.5), 42.5);
        assert_eq!(f32::from_f32(-3.0), -3.0);
        // −0.0 survives storage; the bitwise scans treat it as content.
        assert_ne!(f32::from_f32(-0.0).bits(), 0);
    }

    #[test]
    fn le_encoding_shapes() {
        let mut b = [0u8; 4];
        0xABu8.put_le(&mut b[..1]);
        assert_eq!(b[0], 0xAB);
        1.0f32.put_le(&mut b);
        assert_eq!(b, 1.0f32.to_le_bytes());
    }

    #[test]
    fn kind_codes_are_stable() {
        // Part of the region_hash identity definition — do not renumber.
        assert_eq!(ChannelKind::U8.as_u8(), 0);
        assert_eq!(ChannelKind::U16.as_u8(), 1);
        assert_eq!(ChannelKind::F16.as_u8(), 2);
        assert_eq!(ChannelKind::F32.as_u8(), 3);
    }

    #[test]
    fn kind_serializes_to_schema_depth_vocabulary() {
        for (k, s) in [
            (ChannelKind::U8, "\"u8\""),
            (ChannelKind::U16, "\"u16\""),
            (ChannelKind::F16, "\"f16\""),
            (ChannelKind::F32, "\"f32\""),
        ] {
            assert_eq!(serde_json::to_string(&k).unwrap(), s);
        }
    }
}
