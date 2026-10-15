// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §5.1 (`Intent`), §5.2 (pixel-format mapping table — the
//! `(ColorModel, Depth)` pairs that name a CMS marshalling layout), §6.1
//! (storage depths); contract `contracts/commands/doc.convert_profile.yaml`
//! (wire spellings of the intent enum).
//!
//! Local vocabulary types. This crate sits BELOW `appname-core` in the
//! dependency DAG (ES §1.4 rule 4: core → domain crates → leaf), so it
//! cannot see core's `ColorModelKind`/`ChannelKind`/`RenderingIntent`;
//! core maps its enums onto these at the wiring boundary (the mapping is
//! mechanical — variant names line up by construction).

use core::fmt;

/// Rendering intent (ES §5.1). Wire spellings per
/// `doc.convert_profile.yaml` `paramsSchema.intent`:
/// `perceptual | relative | saturation | absolute`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Intent {
    Perceptual,
    RelativeColorimetric,
    Saturation,
    AbsoluteColorimetric,
}

impl Intent {
    /// The contract wire spelling.
    pub const fn as_wire(self) -> &'static str {
        match self {
            Intent::Perceptual => "perceptual",
            Intent::RelativeColorimetric => "relative",
            Intent::Saturation => "saturation",
            Intent::AbsoluteColorimetric => "absolute",
        }
    }

    /// Parse the contract wire spelling. Unknown literals are the caller's
    /// `error.schema.out_of_range` (this crate does not own envelope
    /// validation; it just refuses to guess).
    pub fn from_wire(s: &str) -> Option<Intent> {
        Some(match s {
            "perceptual" => Intent::Perceptual,
            "relative" => Intent::RelativeColorimetric,
            "saturation" => Intent::Saturation,
            "absolute" => Intent::AbsoluteColorimetric,
            _ => return None,
        })
    }
}

/// Color model at the CMS boundary (rows of the ES §5.2 pixel-format
/// mapping table). Alpha-bearing variants exist because the CMS marshaller
/// must know where the alpha channel sits — alpha itself is **never**
/// color-managed (ES §5.2 / §6.2: it copies through untouched).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum ColorModel {
    Gray,
    GrayA,
    Rgb,
    Rgba,
    Cmyk,
    CmykA,
    Lab,
    LabA,
}

impl ColorModel {
    /// Total channels including alpha.
    pub const fn channels(self) -> usize {
        match self {
            ColorModel::Gray => 1,
            ColorModel::GrayA => 2,
            ColorModel::Rgb | ColorModel::Lab => 3,
            ColorModel::Rgba | ColorModel::LabA | ColorModel::Cmyk => 4,
            ColorModel::CmykA => 5,
        }
    }

    /// Color channels (channels minus alpha).
    pub const fn color_channels(self) -> usize {
        match self {
            ColorModel::Gray | ColorModel::GrayA => 1,
            ColorModel::Rgb | ColorModel::Rgba | ColorModel::Lab | ColorModel::LabA => 3,
            ColorModel::Cmyk | ColorModel::CmykA => 4,
        }
    }

    /// Whether the model carries an alpha channel (always the LAST channel,
    /// matching the lcms2 `EXTRA` convention of the §5.2 table).
    pub const fn has_alpha(self) -> bool {
        matches!(
            self,
            ColorModel::GrayA | ColorModel::Rgba | ColorModel::CmykA | ColorModel::LabA
        )
    }

    /// The alpha-less companion model (identity for alpha-less models).
    pub const fn without_alpha(self) -> ColorModel {
        match self {
            ColorModel::GrayA => ColorModel::Gray,
            ColorModel::Rgba => ColorModel::Rgb,
            ColorModel::CmykA => ColorModel::Cmyk,
            ColorModel::LabA => ColorModel::Lab,
            other => other,
        }
    }
}

/// Storage depth vocabulary (ES §6.1). `F16` exists as a tag so the depth
/// vocabulary is complete, but every f16 conversion path in this crate is
/// PAUSED pending the `half` crate's `deps/` record — the same paused-depth
/// discipline `appname-core` applies to `ChannelKind::F16` (its channel
/// module documents the pause; ours returns
/// [`ColorError::DepthPaused`](crate::error::ColorError::DepthPaused)).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Depth {
    U8,
    U16,
    F16,
    F32,
}

impl Depth {
    /// Bytes per channel.
    pub const fn bytes(self) -> usize {
        match self {
            Depth::U8 => 1,
            Depth::U16 | Depth::F16 => 2,
            Depth::F32 => 4,
        }
    }

    /// Whether this crate can move pixels at this depth today. `false` only
    /// for [`Depth::F16`] (paused on the `half` record).
    pub const fn is_active(self) -> bool {
        !matches!(self, Depth::F16)
    }

    /// Wire spelling per `doc.depth_convert.yaml` `paramsSchema.to`.
    pub const fn as_wire(self) -> &'static str {
        match self {
            Depth::U8 => "u8",
            Depth::U16 => "u16",
            Depth::F16 => "f16",
            Depth::F32 => "f32",
        }
    }
}

/// A CMS marshalling layout: model × depth (one cell of the ES §5.2
/// pixel-format table). The lcms2 `TYPE_*`/descriptor mapping for each cell
/// attaches when the ICC backend lands; the pure slice only moves `F32`
/// (working precision is always float, ES §6.1).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct PixelFormat {
    pub model: ColorModel,
    pub depth: Depth,
}

impl PixelFormat {
    pub const fn new(model: ColorModel, depth: Depth) -> PixelFormat {
        PixelFormat { model, depth }
    }

    /// Floats per pixel at this layout (channel count; depth-independent
    /// because the slice-1 apply path is f32-only).
    pub const fn channels(self) -> usize {
        self.model.channels()
    }
}

impl fmt::Display for PixelFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}/{}", self.model, self.depth.as_wire())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intent_wire_round_trips() {
        for i in [
            Intent::Perceptual,
            Intent::RelativeColorimetric,
            Intent::Saturation,
            Intent::AbsoluteColorimetric,
        ] {
            assert_eq!(Intent::from_wire(i.as_wire()), Some(i));
        }
        // The contract's own invalid-case literal must NOT parse
        // (doc.convert_profile.yaml invalid case "bad-enum").
        assert_eq!(Intent::from_wire("relative_colorimetric"), None);
    }

    #[test]
    fn model_channel_arithmetic() {
        for m in [
            ColorModel::Gray,
            ColorModel::GrayA,
            ColorModel::Rgb,
            ColorModel::Rgba,
            ColorModel::Cmyk,
            ColorModel::CmykA,
            ColorModel::Lab,
            ColorModel::LabA,
        ] {
            let alpha = usize::from(m.has_alpha());
            assert_eq!(m.color_channels() + alpha, m.channels());
            assert_eq!(m.without_alpha().color_channels(), m.color_channels());
            assert!(!m.without_alpha().has_alpha());
        }
    }

    #[test]
    fn depth_vocabulary_matches_contract() {
        assert_eq!(Depth::U8.as_wire(), "u8");
        assert_eq!(Depth::U16.as_wire(), "u16");
        assert_eq!(Depth::F16.as_wire(), "f16");
        assert_eq!(Depth::F32.as_wire(), "f32");
        assert!(
            !Depth::F16.is_active(),
            "f16 stays paused until the half record"
        );
        assert!(Depth::U8.is_active() && Depth::U16.is_active() && Depth::F32.is_active());
    }
}
