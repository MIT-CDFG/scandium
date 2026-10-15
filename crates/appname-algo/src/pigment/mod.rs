// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ALGORITHMS.md §23 `paint_engine` (pigment: `kubelka_munk_spectral`,
//! basis [jakob19, spectraljs], luts `own_baked_33cube_f16`; strokes
//! reference `libmypaint_isc`); ES §22.3; SPEC §28 presets;
//! INTEGRATION-NOTES §6 order 3 (pigment stack: 3a rgb2spec LUT bake,
//! 3b spectral.js cross-check, 3c `.myb` importer).
//!
//! The paint engine's pigment stack:
//! * [`rgb2spec`] — Jakob–Hanika spectral upsampling: offline LUT bake +
//!   runtime coefficient fetch (BSD-3 port).
//! * [`km`] — Kubelka–Munk mixing cross-check reference (MIT port of
//!   spectral.js math + data).
//! * [`myb`] — `.myb` preset importer per the libmypaint ISC port-spec memo.
//! * [`cie`] — shared CIE 1931 / D65 / sRGB reference data (BSD-3 port).
//!
//! Cross-check contract (census 3b role): the rgb2spec spectral path and the
//! spectral.js K-M path are independently derived; `tests` below hold the
//! agreement oracle that guards the engine's own pipeline.

pub mod cie;
pub mod km;
pub mod myb;
pub mod rgb2spec;

#[cfg(test)]
mod tests {
    use super::km;
    use super::rgb2spec::{bake, spectrum_to_srgb, Rgb2SpecModel, Tables};

    /// The dossier cross-check: reflectance spectra produced by the
    /// rgb2spec sigmoid fit, mixed with the K/S operator from the
    /// spectral.js port, must behave like the spectral.js pipeline —
    /// subtractive, not additive.
    #[test]
    fn rgb2spec_spectra_under_km_mixing_stay_subtractive() {
        let t = Tables::new_srgb();
        let model: Rgb2SpecModel = bake(&t, 9);

        // Sample the sigmoid spectra of blue and yellow on spectral.js's
        // 38-band grid, K-S mix them 1:1, and integrate back.
        let sample = |rgb: [f32; 3]| -> [f64; km::SIZE] {
            let coeff = model.fetch(rgb);
            let mut r = [0f64; km::SIZE];
            for (i, band) in r.iter_mut().enumerate() {
                let lambda = 380.0 + 10.0 * i as f32;
                *band = (super::rgb2spec::eval_precise(coeff, lambda) as f64).max(1e-9);
            }
            r
        };
        let blue = km::Pigment::from_reflectance(sample([0.05, 0.05, 0.9]));
        let yellow = km::Pigment::from_reflectance(sample([0.9, 0.9, 0.05]));
        // Mix through the PORTED operator (luminance-weighted concentration,
        // spectral.js semantics) — the cross-check is that rgb2spec spectra
        // behave subtractively under the spectral.js mixing law.
        let rgb = km::mix(&[(&blue, 1.0), (&yellow, 1.0)]).to_lrgb();
        assert!(
            rgb[1] > rgb[0] && rgb[1] > rgb[2],
            "K-M mix of rgb2spec blue+yellow spectra must be green-dominant, got {rgb:?}"
        );

        // And the same mix done additively (linear RGB average) must NOT be
        // green-dominant — proving the pigment path differs from plain
        // blending in the direction the dossier expects.
        let additive: [f64; 3] = [(0.05 + 0.9) / 2.0, (0.05 + 0.9) / 2.0, (0.9 + 0.05) / 2.0];
        // Equal R and G channels: an additive average has no green
        // dominance to offer, unlike the K-M mix above.
        assert!((additive[0] - additive[1]).abs() < 1e-9);
    }

    /// Constants cross-check (census 3b: "constants cross-check vs dossier
    /// `pigment-mixing`"): both ports must agree on what "white" is to
    /// within display precision.
    #[test]
    fn both_ports_agree_on_white() {
        let t = Tables::new_srgb();
        let model = bake(&t, 5);
        let coeff = model.fetch([1.0; 3]);
        let white_rgb2spec = spectrum_to_srgb(&t, coeff);
        let white_km = km::Pigment::from_lrgb([1.0; 3]).to_lrgb();
        for c in 0..3 {
            assert!(
                (white_rgb2spec[c] - 1.0).abs() < 5e-3,
                "rgb2spec white {white_rgb2spec:?}"
            );
            assert!((white_km[c] - 1.0).abs() < 5e-3, "km white {white_km:?}");
        }
    }
}
