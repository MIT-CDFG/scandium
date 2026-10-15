// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §5.5 (sRGB transfer pair — the normative piecewise constants),
//! Appendix B "Transfer functions" (sRGB, Gamma 2.2; Rec.709 and PQ/HLG are
//! out of this checkpoint's scope), §5.5 "Gray working spaces" (Gray Gamma
//! 2.2 and sGray TRCs).
//!
//! Tone response curves for the builtin matrix/TRC working spaces. All math
//! is f64 (derivation precision); pixel paths narrow to f32 after the full
//! decode→matrix→encode chain, which keeps intermediate rounding out of the
//! stored result.
//!
//! Domain note: out-of-range inputs occur transiently (filter ringing,
//! wide-gamut → narrow-gamut matrices). Both curves extend **odd-symmetric**
//! (`f(-x) = -f(x)`) above/below the unit interval so they stay monotone and
//! invertible everywhere; the transform layer decides when to clamp (encode
//! side, matching the §5.4 LUT domain `clamp(c, 0, 1)`).

/// A tone response curve of a builtin working space.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum TransferFn {
    /// No curve (linear light).
    Linear,
    /// IEC 61966-2-1 piecewise sRGB pair (ES §5.5 constants). Also the sGray
    /// TRC (§5.5 "Gray working spaces": the sRGB transfer as a gray TRC).
    SrgbPiecewise,
    /// Pure power 2.2 (Appendix B "Gamma 2.2"; the Gray Gamma 2.2 TRC).
    Gamma22,
    /// ProPhoto RGB TRC (ROMM, ISO 22028-2): power 1.8 with the standard
    /// linear toe, slope 16, break at Et = 1/512 (linear-side).
    ProPhoto,
}

impl TransferFn {
    /// Encoded value → linear light. Odd-symmetric outside [0, 1].
    pub fn decode(self, v: f64) -> f64 {
        if v < 0.0 {
            return -self.decode(-v);
        }
        match self {
            TransferFn::Linear => v,
            TransferFn::SrgbPiecewise => {
                // ES §5.5: L = (V ≤ 0.04045) ? V/12.92 : ((V+0.055)/1.055)^2.4
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            }
            TransferFn::Gamma22 => v.powf(2.2),
            TransferFn::ProPhoto => {
                // ROMM: E ≤ 16·Et ⇒ L = E/16, else L = E^1.8; Et = 1/512.
                if v <= 16.0 / 512.0 {
                    v / 16.0
                } else {
                    v.powf(1.8)
                }
            }
        }
    }

    /// Linear light → encoded value. Odd-symmetric outside [0, 1].
    pub fn encode(self, l: f64) -> f64 {
        if l < 0.0 {
            return -self.encode(-l);
        }
        match self {
            TransferFn::Linear => l,
            TransferFn::SrgbPiecewise => {
                // ES §5.5: V = (L ≤ 0.0031308) ? 12.92·L : 1.055·L^(1/2.4) − 0.055
                if l <= 0.003_130_8 {
                    12.92 * l
                } else {
                    1.055 * l.powf(1.0 / 2.4) - 0.055
                }
            }
            TransferFn::Gamma22 => l.powf(1.0 / 2.2),
            TransferFn::ProPhoto => {
                if l <= 1.0 / 512.0 {
                    l * 16.0
                } else {
                    l.powf(1.0 / 1.8)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CURVES: [TransferFn; 4] = [
        TransferFn::Linear,
        TransferFn::SrgbPiecewise,
        TransferFn::Gamma22,
        TransferFn::ProPhoto,
    ];

    #[test]
    fn round_trips_across_the_unit_interval() {
        for tf in CURVES {
            for i in 0..=4096u32 {
                let v = f64::from(i) / 4096.0;
                let rt = tf.encode(tf.decode(v));
                assert!((rt - v).abs() < 1e-12, "{tf:?}: encode(decode({v})) = {rt}");
            }
        }
    }

    #[test]
    fn srgb_piecewise_is_continuous_at_the_breaks() {
        // Decode side around V = 0.04045; encode side around L = 0.0031308.
        let tf = TransferFn::SrgbPiecewise;
        let below = tf.decode(0.04045);
        let above = tf.decode(0.04045 + 1e-9);
        assert!((below - above).abs() < 1e-6);
        let e_below = tf.encode(0.003_130_8);
        let e_above = tf.encode(0.003_130_8 + 1e-9);
        assert!((e_below - e_above).abs() < 1e-6);
    }

    #[test]
    fn prophoto_is_continuous_at_the_toe() {
        let tf = TransferFn::ProPhoto;
        let lo = tf.decode(16.0 / 512.0);
        let hi = tf.decode(16.0 / 512.0 + 1e-9);
        assert!((lo - hi).abs() < 1e-6, "{lo} vs {hi}");
    }

    #[test]
    fn srgb_known_values_match_spec_constants() {
        let tf = TransferFn::SrgbPiecewise;
        // Linear-segment sample: V = 0.02 → L = 0.02/12.92.
        assert!((tf.decode(0.02) - 0.02 / 12.92).abs() < 1e-15);
        // Endpoint identities.
        assert_eq!(tf.decode(0.0), 0.0);
        assert!((tf.decode(1.0) - 1.0).abs() < 1e-12);
        assert!((tf.encode(1.0) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn curves_are_monotone_including_extension() {
        for tf in CURVES {
            let mut prev = f64::NEG_INFINITY;
            for i in -100..=1200i32 {
                let v = f64::from(i) / 1000.0; // spans [-0.1, 1.2]
                let l = tf.decode(v);
                assert!(l > prev, "{tf:?} not strictly monotone at {v}");
                prev = l;
            }
        }
    }

    #[test]
    fn odd_extension_is_symmetric() {
        for tf in CURVES {
            for v in [0.005, 0.05, 0.5, 1.1] {
                assert_eq!(tf.decode(-v).to_bits(), (-tf.decode(v)).to_bits());
                assert_eq!(tf.encode(-v).to_bits(), (-tf.encode(v)).to_bits());
            }
        }
    }
}
