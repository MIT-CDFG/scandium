// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §2.2 (color-model traits) — the eight storage color models; DD ⑥.
//!
//! [NORMATIVE, ES §2.2] channel order/meaning per the eight-model table:
//! `Gray` is K₀ gray (masks/selections: 0 = hidden/unselected, 1 =
//! revealed/selected, §11.2/§13.1); `Rgb(a)` is encoded per the buffer
//! profile's transfer curve; `Cmyk(a)` is ink coverage; `Lab(a)` uses the
//! ICC.1-scaled encoding (§2.2). Semantics beyond naming (conversion, Lab
//! byte mapping) live with the color lane (CP-0.5, ES §5–§6).

use serde::{Deserialize, Serialize};

/// A color model: how many channels, whether alpha is present, channel
/// meaning (ES §2.2). Marker types live in [`model`] so they never clash with
/// working-pixel type names.
pub trait ColorModel: Copy + Send + Sync + 'static {
    /// Channel count, including alpha when `HAS_ALPHA`.
    const CHANNELS: usize;
    const HAS_ALPHA: bool;
    const KIND: ColorModelKind;
    /// Index of the alpha channel, if any.
    const ALPHA_INDEX: Option<usize>;
}

/// Runtime color-model tag. Serialized vocabulary is the `document.v1.json`
/// `mode` enum (camelCase: `gray`, `grayA`, `rgb`, `rgba`, `cmyk`, `cmykA`,
/// `lab`, `labA`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColorModelKind {
    Gray,
    GrayA,
    Rgb,
    Rgba,
    Cmyk,
    CmykA,
    Lab,
    LabA,
}

impl ColorModelKind {
    /// Stable numeric encoding used inside hash framings (ES §2.5
    /// `region_hash` feeds `model:u8`, §2.2 table order). Part of the
    /// identity definition — do not renumber.
    pub const fn as_u8(self) -> u8 {
        match self {
            ColorModelKind::Gray => 0,
            ColorModelKind::GrayA => 1,
            ColorModelKind::Rgb => 2,
            ColorModelKind::Rgba => 3,
            ColorModelKind::Cmyk => 4,
            ColorModelKind::CmykA => 5,
            ColorModelKind::Lab => 6,
            ColorModelKind::LabA => 7,
        }
    }

    pub const fn channels(self) -> usize {
        match self {
            ColorModelKind::Gray => 1,
            ColorModelKind::GrayA => 2,
            ColorModelKind::Rgb | ColorModelKind::Lab => 3,
            ColorModelKind::Rgba | ColorModelKind::Cmyk | ColorModelKind::LabA => 4,
            ColorModelKind::CmykA => 5,
        }
    }

    pub const fn has_alpha(self) -> bool {
        matches!(
            self,
            ColorModelKind::GrayA
                | ColorModelKind::Rgba
                | ColorModelKind::CmykA
                | ColorModelKind::LabA
        )
    }

    pub const fn alpha_index(self) -> Option<usize> {
        match self {
            ColorModelKind::GrayA => Some(1),
            ColorModelKind::Rgba | ColorModelKind::LabA => Some(3),
            ColorModelKind::CmykA => Some(4),
            _ => None,
        }
    }
}

/// Zero-sized marker types (ES §2.2): `Tile<u8, model::Rgba>`,
/// `Tile<f32, model::Gray>`, …
#[allow(clippy::module_inception)] // §2.2 layout: model::model mirrors channel::channel
pub mod model {
    #[derive(Clone, Copy, Debug)]
    pub struct Gray;
    #[derive(Clone, Copy, Debug)]
    pub struct GrayA;
    #[derive(Clone, Copy, Debug)]
    pub struct Rgb;
    #[derive(Clone, Copy, Debug)]
    pub struct Rgba;
    #[derive(Clone, Copy, Debug)]
    pub struct Cmyk;
    #[derive(Clone, Copy, Debug)]
    pub struct CmykA;
    #[derive(Clone, Copy, Debug)]
    pub struct Lab;
    #[derive(Clone, Copy, Debug)]
    pub struct LabA;
}

macro_rules! impl_color_model {
    ($marker:ty, $channels:expr, $has_alpha:expr, $kind:expr, $alpha:expr) => {
        impl ColorModel for $marker {
            const CHANNELS: usize = $channels;
            const HAS_ALPHA: bool = $has_alpha;
            const KIND: ColorModelKind = $kind;
            const ALPHA_INDEX: Option<usize> = $alpha;
        }
    };
}

impl_color_model!(model::Gray, 1, false, ColorModelKind::Gray, None);
impl_color_model!(model::GrayA, 2, true, ColorModelKind::GrayA, Some(1));
impl_color_model!(model::Rgb, 3, false, ColorModelKind::Rgb, None);
impl_color_model!(model::Rgba, 4, true, ColorModelKind::Rgba, Some(3));
impl_color_model!(model::Cmyk, 4, false, ColorModelKind::Cmyk, None);
impl_color_model!(model::CmykA, 5, true, ColorModelKind::CmykA, Some(4));
impl_color_model!(model::Lab, 3, false, ColorModelKind::Lab, None);
impl_color_model!(model::LabA, 4, true, ColorModelKind::LabA, Some(3));

#[cfg(test)]
mod tests {
    use super::*;

    /// The compile-time trait table and the runtime enum table are the same
    /// eight rows (single-source check on the §2.2 table).
    #[test]
    fn trait_consts_match_runtime_tables() {
        fn check<C: ColorModel>() {
            assert_eq!(C::CHANNELS, C::KIND.channels());
            assert_eq!(C::HAS_ALPHA, C::KIND.has_alpha());
            assert_eq!(C::ALPHA_INDEX, C::KIND.alpha_index());
        }
        check::<model::Gray>();
        check::<model::GrayA>();
        check::<model::Rgb>();
        check::<model::Rgba>();
        check::<model::Cmyk>();
        check::<model::CmykA>();
        check::<model::Lab>();
        check::<model::LabA>();
    }

    #[test]
    fn model_codes_are_stable() {
        let all = [
            ColorModelKind::Gray,
            ColorModelKind::GrayA,
            ColorModelKind::Rgb,
            ColorModelKind::Rgba,
            ColorModelKind::Cmyk,
            ColorModelKind::CmykA,
            ColorModelKind::Lab,
            ColorModelKind::LabA,
        ];
        for (i, m) in all.iter().enumerate() {
            assert_eq!(m.as_u8() as usize, i);
        }
    }

    #[test]
    fn model_serializes_to_schema_mode_vocabulary() {
        for (m, s) in [
            (ColorModelKind::Gray, "\"gray\""),
            (ColorModelKind::GrayA, "\"grayA\""),
            (ColorModelKind::Rgb, "\"rgb\""),
            (ColorModelKind::Rgba, "\"rgba\""),
            (ColorModelKind::Cmyk, "\"cmyk\""),
            (ColorModelKind::CmykA, "\"cmykA\""),
            (ColorModelKind::Lab, "\"lab\""),
            (ColorModelKind::LabA, "\"labA\""),
        ] {
            assert_eq!(serde_json::to_string(&m).unwrap(), s);
            let back: ColorModelKind = serde_json::from_str(s).unwrap();
            assert_eq!(back, m);
        }
    }
}
