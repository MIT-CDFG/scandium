// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §6.3 (promotion paths — exact formulas and the f16 caveat;
//! demotion dithering [NORMATIVE]), §6.1 (storage vs working precision);
//! contract `contracts/commands/doc.depth_convert.yaml` (promotion MUST be
//! lossless; demotion dithers when `dither: true`; converting to the
//! current depth is a no-op push).
//!
//! Depth-conversion scaffolding for the u8/u16/f16/f32 roster. f16 paths
//! are PAUSED on the `half` crate's `deps/` record (same discipline as
//! core's `ChannelKind::F16`): the *vocabulary* and *policy* for f16 are
//! complete here — including the §6.3 never-implicit `u16 → f16` rule —
//! but every executable f16 arm returns [`ColorError::DepthPaused`].
//!
//! Dither source: §6.3 mandates blue-noise of ±0.5 LSB from the shared
//! `BN64` texture (§4.5, the Dissolve table — owned by the render/QA
//! corpus so GPU and CPU dither identically). This crate takes the noise
//! through the [`DitherSource`] trait; [`NoDither`] gives the exact
//! round-to-nearest fast path. The BN64-backed impl arrives with wiring
//! (core/render own the table; see the lane log's core-needs note).

use crate::error::ColorError;
use crate::types::Depth;

/// §6.3 exact promotion: `u8 → u16` is `v · 257` (255 → 65535, lossless).
#[inline]
pub const fn promote_u8_to_u16(v: u8) -> u16 {
    v as u16 * 257
}

/// §6.3 exact promotion: `u8 → f32` is `v / 255`.
#[inline]
pub fn promote_u8_to_f32(v: u8) -> f32 {
    f32::from(v) / 255.0
}

/// §6.3 exact promotion: `u16 → f32` is `v / 65535` — "the sanctioned
/// promotion" (65535 < 2²⁴ fits f32 exactly).
#[inline]
pub fn promote_u16_to_f32(v: u16) -> f32 {
    f32::from(v) / 65535.0
}

/// §6.3 undithered integer demotion fast path: `v8 = (v16 + 128) / 257`.
#[inline]
pub const fn demote_u16_to_u8(v: u16) -> u8 {
    ((v as u32 + 128) / 257) as u8
}

/// A dither noise source in the §6.3 shape: `n(x, y) ∈ (0, 1)`, indexed by
/// document coordinates (deterministic, seam-free per spec wording; the
/// production impl reads the shared `BN64` table).
pub trait DitherSource {
    /// Noise in the open interval (0, 1) at document pixel (x, y).
    fn noise01(&self, x: u32, y: u32) -> f32;
}

/// No dithering: constant 0.5, which reduces `floor(v·MAX + n)` to exact
/// round-to-nearest (the §6.3 `"dither": false` path).
#[derive(Clone, Copy, Default, Debug)]
pub struct NoDither;

impl DitherSource for NoDither {
    #[inline]
    fn noise01(&self, _x: u32, _y: u32) -> f32 {
        0.5
    }
}

/// §6.3 [NORMATIVE] float → integer demotion:
/// `out = clamp( floor( clamp(v, 0, 1) · MAX + n(x,y) ), 0, MAX )`.
/// `MAX` = 255 or 65535. NaN clamps to 0 (NaN never reaches storage —
/// core's §2.2 rule; `clamp` here maps NaN → 0 because every comparison
/// with NaN is false and the first clamp arm wins).
#[inline]
pub fn demote_f32_dithered(v: f32, max: u32, noise: f32) -> u32 {
    let c = if v > 0.0 {
        if v < 1.0 {
            v
        } else {
            1.0
        }
    } else {
        0.0
    };
    let raw = (f64::from(c) * f64::from(max) + f64::from(noise)).floor();
    if raw < 0.0 {
        0
    } else if raw > f64::from(max) {
        max
    } else {
        // representable exactly: 0 ≤ raw ≤ 65535
        raw as u32
    }
}

/// Convenience: §6.3 demotion `f32 → u8` with a dither source at (x, y).
#[inline]
pub fn demote_f32_to_u8<D: DitherSource>(v: f32, d: &D, x: u32, y: u32) -> u8 {
    demote_f32_dithered(v, 255, d.noise01(x, y)) as u8
}

/// Convenience: §6.3 demotion `f32 → u16` with a dither source at (x, y).
#[inline]
pub fn demote_f32_to_u16<D: DitherSource>(v: f32, d: &D, x: u32, y: u32) -> u16 {
    demote_f32_dithered(v, 65535, d.noise01(x, y)) as u16
}

/// Depth-conversion policy verdict for a `(from, to)` pair — the
/// `doc.depth_convert` groundwork core wires at the command layer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DepthConvertKind {
    /// Same depth: §40.1 no-op push (still recorded/undoable — core's job).
    NoOp,
    /// §6.3 lossless promotion.
    LosslessPromotion,
    /// Lossy demotion; dithers when requested; warns
    /// (`color.depth.demote_lossy_warning`).
    LossyDemotion,
    /// `u16 → f16`: never implicit, explicit path warns
    /// (`color.depth.f16_loss_warning`) — and is additionally paused today.
    ExplicitLossyF16,
}

/// Classify a depth conversion per §6.3. Errors: any f16 endpoint is
/// [`ColorError::DepthPaused`] until the `half` record lands (the
/// classification the pair WILL have is documented on each arm).
pub fn classify_depth_convert(from: Depth, to: Depth) -> Result<DepthConvertKind, ColorError> {
    if from == Depth::F16 || to == Depth::F16 {
        // Post-record classifications, for the record:
        //   u8  → f16 : LosslessPromotion (255 codes fit the 11-bit
        //               significand, §6.3 table)
        //   u16 → f16 : ExplicitLossyF16 (§6.3 [NORMATIVE], never implicit)
        //   f16 → f32 : LosslessPromotion (bit widen)
        //   f32 → f16 : LossyDemotion (plain IEEE round-to-nearest-even;
        //               dither off and meaningless, §6.3)
        return Err(ColorError::DepthPaused(Depth::F16));
    }
    Ok(match (from, to) {
        (a, b) if a == b => DepthConvertKind::NoOp,
        (Depth::U8, Depth::U16 | Depth::F32) | (Depth::U16, Depth::F32) => {
            DepthConvertKind::LosslessPromotion
        }
        _ => DepthConvertKind::LossyDemotion,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The spec's own test, verbatim (§6.3 `promotion_roundtrips`).
    #[test]
    fn promotion_roundtrips() {
        for v in 0..=255u8 {
            let up = u16::from(v) * 257; // 8→16
            assert_eq!((up / 257) as u8, v); // 16→8 recovers exactly
            assert_eq!(promote_u8_to_u16(v), up);
        }
    }

    /// §6.3 endpoints: 255 → 65535; u16 → f32 is exact for every code.
    #[test]
    fn promotion_formulas_are_exact() {
        assert_eq!(promote_u8_to_u16(255), 65535);
        assert_eq!(promote_u8_to_u16(0), 0);
        for v in [0u16, 1, 2048, 2049, 32768, 65519, 65535] {
            let f = promote_u16_to_f32(v);
            // exact recovery via round-to-nearest re-quantization
            let back = (f64::from(f) * 65535.0).round() as u16;
            assert_eq!(back, v);
        }
        assert_eq!(promote_u8_to_f32(255), 1.0);
        assert_eq!(promote_u8_to_f32(0), 0.0);
    }

    /// The u8 → u16 → u8 chain through BOTH formulas (promote then the
    /// §6.3 undithered fast path) is the identity.
    #[test]
    fn promote_then_fast_demote_is_identity() {
        for v in 0..=255u8 {
            assert_eq!(demote_u16_to_u8(promote_u8_to_u16(v)), v);
        }
    }

    /// Undithered float demotion is round-to-nearest and clamps.
    #[test]
    fn undithered_demotion_rounds_and_clamps() {
        let d = NoDither;
        assert_eq!(demote_f32_to_u8(0.0, &d, 0, 0), 0);
        assert_eq!(demote_f32_to_u8(1.0, &d, 0, 0), 255);
        assert_eq!(demote_f32_to_u8(1.5, &d, 0, 0), 255, "over-range clamps");
        assert_eq!(demote_f32_to_u8(-0.25, &d, 0, 0), 0, "under-range clamps");
        assert_eq!(
            demote_f32_to_u8(f32::NAN, &d, 0, 0),
            0,
            "NaN never reaches storage"
        );
        // 0.5/255 boundary: v = 100.49/255 rounds down, 100.51/255 rounds up.
        assert_eq!(demote_f32_to_u8(100.49 / 255.0, &d, 0, 0), 100);
        assert_eq!(demote_f32_to_u8(100.51 / 255.0, &d, 0, 0), 101);
        assert_eq!(demote_f32_to_u16(1.0, &d, 0, 0), 65535);
    }

    /// Full-range dither noise shifts a mid-code value by at most one LSB
    /// (±0.5 LSB around round-to-nearest, §6.3).
    #[test]
    fn dither_moves_at_most_one_lsb() {
        struct Fixed(f32);
        impl DitherSource for Fixed {
            fn noise01(&self, _x: u32, _y: u32) -> f32 {
                self.0
            }
        }
        let v = 100.5 / 255.0; // exactly between codes 100 and 101
        let lo = demote_f32_to_u8(v, &Fixed(1e-6), 0, 0);
        let hi = demote_f32_to_u8(v, &Fixed(1.0 - 1e-6), 0, 0);
        assert_eq!(lo, 100);
        assert_eq!(hi, 101);
    }

    /// Exhaustive u16 sweep: promote to f32 and re-demote (round-to-
    /// nearest) recovers every code — §6.3 losslessness at full width.
    #[test]
    fn u16_f32_round_trip_exhaustive() {
        let d = NoDither;
        for v in 0..=65535u16 {
            let f = promote_u16_to_f32(v);
            assert_eq!(demote_f32_to_u16(f, &d, 0, 0), v);
        }
    }

    /// Policy classification per §6.3, including the f16 pause.
    #[test]
    fn depth_convert_classification() {
        use DepthConvertKind::*;
        assert_eq!(classify_depth_convert(Depth::U8, Depth::U8).unwrap(), NoOp);
        assert_eq!(
            classify_depth_convert(Depth::U8, Depth::U16).unwrap(),
            LosslessPromotion
        );
        assert_eq!(
            classify_depth_convert(Depth::U8, Depth::F32).unwrap(),
            LosslessPromotion
        );
        assert_eq!(
            classify_depth_convert(Depth::U16, Depth::F32).unwrap(),
            LosslessPromotion
        );
        assert_eq!(
            classify_depth_convert(Depth::F32, Depth::U8).unwrap(),
            LossyDemotion
        );
        assert_eq!(
            classify_depth_convert(Depth::F32, Depth::U16).unwrap(),
            LossyDemotion
        );
        assert_eq!(
            classify_depth_convert(Depth::U16, Depth::U8).unwrap(),
            LossyDemotion
        );
        // f16 endpoints: paused until the half record.
        for (a, b) in [
            (Depth::U8, Depth::F16),
            (Depth::U16, Depth::F16),
            (Depth::F16, Depth::F32),
            (Depth::F32, Depth::F16),
            (Depth::F16, Depth::F16),
        ] {
            assert_eq!(
                classify_depth_convert(a, b),
                Err(ColorError::DepthPaused(Depth::F16))
            );
        }
    }
}
