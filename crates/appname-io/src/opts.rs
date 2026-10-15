// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §8.1 (`DecodeOpts`/`EncodeOpts`), §8.2 "Per-codec option
//! structs" [NORMATIVE] — defaults and ranges below are the contract;
//! out-of-range values fail validation with `CodecError::OptionOutOfRange`
//! BEFORE any encoding starts. DD ⑫㉞.
//!
//! Interim deviations from the frozen §8.2 defaults, each typed (never
//! silent) and recorded in the lane log for the CMM / encoder-election
//! handoffs:
//! * `JpegOptions::progressive` defaults FALSE here (spec: true). The
//!   elected pure-Rust encoder (`image`, per the jpeg-codecs record — the
//!   mozjpeg arm is non-viable under the IJG licensing finding) emits
//!   baseline streams only; `progressive: true` answers
//!   `UnsupportedFeature("jpeg.progressive")`.
//! * `WebpOptions::lossless` defaults TRUE here (spec: false). The elected
//!   pure-Rust `image-webp` encoder is VP8L lossless-only; `lossless:
//!   false` answers `UnsupportedFeature("webp.lossy_encode")`.
//! * `JpegOptions::chroma: None` (auto) resolves to **S444 at every
//!   quality** (spec's [NORMATIVE] auto rule: S420 below q90, else S444).
//!   The elected encoder is fixed 4:4:4, so the sub-q90 arm of the auto
//!   rule cannot be honored; explicit `Some(S420)`/`Some(S422)` answer
//!   typed `UnsupportedFeature`. No export-report channel exists at this
//!   slice to surface the auto-arm substitution per encode — that channel
//!   arrives with the §29.1 Export-As report; until then this register
//!   entry and the lane log ARE the record (review round 1 minor 3).
//! * `DecodeOpts::working_profile` is a crate-interim field: core's PNG
//!   ingestion door tags buffers with a working `ProfileId`. When O-COLOR's
//!   CMM lands, embedded-ICC → `ProfileId` resolution replaces it and the
//!   field folds into the §5.3 assign/convert semantics.

use appname_core::{ChannelKind, ProfileId};

use crate::error::CodecError;

/// §8.1 `DecodeOpts` (defaults per spec; see module doc for the interim
/// `working_profile` seam).
#[derive(Debug, Clone)]
pub struct DecodeOpts {
    /// default true: bake EXIF orientation, tag reset to 1 (§8.6)
    pub apply_orientation: bool,
    /// None (default) = smallest ChannelKind holding the source losslessly.
    /// `Some(_)` is refused typed until the §5/§6 depth pipeline lands.
    pub target_depth: Option<ChannelKind>,
    /// default true: animation/multipage → layers (§8.2); false = first
    /// frame only
    pub frames_as_layers: bool,
    /// default 4_294_967_296 (2^32); checked BEFORE any pixel allocation
    pub max_pixels: u64,
    /// default 1_048_576 per axis — decompression-bomb gate
    pub max_dimension: u32,
    /// default true: unrecognized EXIF/XMP blocks preserved byte-wise (§8.6)
    pub keep_unknown_metadata: bool,
    /// ASSIGN this profile instead of the embedded one (§5.3), never
    /// convert. Refused typed until the CMM's profile registry lands.
    pub icc_override: Option<ProfileId>,
    /// §8.1: consulted only by the RAW codec (§8.3); `None` = the §43.3
    /// defaults. Validated per the §43.3 table before any pixel work.
    pub raw: Option<crate::raw::params::RawDevelopParams>,
    /// INTERIM (see module doc): the working-space id core's ingestion door
    /// tags decoded buffers with.
    pub working_profile: ProfileId,
}

impl Default for DecodeOpts {
    fn default() -> DecodeOpts {
        DecodeOpts {
            apply_orientation: true,
            target_depth: None,
            frames_as_layers: true,
            max_pixels: 4_294_967_296,
            max_dimension: 1_048_576,
            keep_unknown_metadata: true,
            icc_override: None,
            raw: None,
            working_profile: ProfileId::nil(),
        }
    }
}

impl DecodeOpts {
    /// §8.1 decompression-bomb gate: called by every codec after the header
    /// parse and BEFORE any pixel allocation.
    pub fn check_limits(&self, width: u32, height: u32) -> Result<(), CodecError> {
        if width == 0 || height == 0 {
            return Err(CodecError::corrupt("zero-sized image"));
        }
        if width > self.max_dimension
            || height > self.max_dimension
            || (width as u64) * (height as u64) > self.max_pixels
        {
            return Err(CodecError::LimitExceeded {
                width,
                height,
                max_pixels: self.max_pixels,
            });
        }
        Ok(())
    }

    /// Reject the option arms whose machinery arrives with the CMM/depth
    /// pipeline (typed, up front — shared by every codec's decode).
    pub(crate) fn check_interim_unsupported(&self) -> Result<(), CodecError> {
        if let Some(kind) = self.target_depth {
            let _ = kind;
            return Err(CodecError::unsupported(
                "decode.target_depth (arrives with the §5/§6 depth pipeline)",
            ));
        }
        if self.icc_override.is_some() {
            return Err(CodecError::unsupported(
                "decode.icc_override (arrives with the CMM profile registry, §5.3)",
            ));
        }
        Ok(())
    }
}

/// §8.1 `IccPolicy`. The two ConvertToSrgb* arms need the CMM (§5) and are
/// refused typed until it lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IccPolicy {
    #[default]
    Embed,
    ConvertToSrgbThenTag,
    ConvertToSrgbUntagged,
    Omit,
}

/// §8.6 `SelectiveMeta` — the four dials of `MetadataPolicy::Selective`,
/// consumed by the §29.4 policy matrix (`crate::export::policy`).
///
/// Defaults: the spec pins exactly ONE dial's dialog default —
/// `strip_gps` is on (§29.4 matrix row: "dialog default: on"). The other
/// three are unpinned at v5.5; `Default` models the dialog's opening
/// state as PRESERVE-leaning (Selective ≠ StripAll: a user electing
/// Selective starts from "keep, minus GPS") — interpretation recorded in
/// the lane log. [`SelectiveMeta::privacy_one_click`] is the §29.4
/// "stripping GPS and personal fields is a one-click option" preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectiveMeta {
    /// keep IPTC creator+rights / `dc:creator`+`dc:rights` / EXIF
    /// Artist+Copyright (§29.4 author/copyright row)
    pub keep_copyright: bool,
    /// keep camera + lens EXIF (make/model/exposure/ISO/program; lens
    /// model/focal length/aperture) — serial numbers stay governed by
    /// `strip_personal` EVEN when this is set (§29.4 [NORMATIVE])
    pub keep_camera: bool,
    /// drop the whole GPS IFD + `exif:GPS*` XMP properties (§29.4;
    /// dialog default: on)
    pub strip_gps: bool,
    /// drop original date/time, serial numbers / owner name, IPTC
    /// creator+location, XMP creator-contact block (§8.6, §29.4)
    pub strip_personal: bool,
}

impl Default for SelectiveMeta {
    fn default() -> SelectiveMeta {
        SelectiveMeta {
            keep_copyright: true,
            keep_camera: true,
            strip_gps: true, // the ONE spec-pinned default (§29.4)
            strip_personal: false,
        }
    }
}

impl SelectiveMeta {
    /// §29.4: "stripping GPS and personal fields is a one-click option".
    pub fn privacy_one_click() -> SelectiveMeta {
        SelectiveMeta {
            keep_copyright: true,
            keep_camera: true,
            strip_gps: true,
            strip_personal: true,
        }
    }
}

/// §8.6 `MetadataPolicy`.
///
/// Codec-level `encode` consumes only the Preserve/not-Preserve split
/// (each codec embeds carried blocks iff `Preserve`); the §29.4 field-
/// group matrix is applied UPSTREAM by `crate::export::policy`, which
/// pre-filters [`crate::meta::Metadata`] and hands the codec `Preserve`
/// over the filtered blocks. A `Selective` value reaching a raw codec
/// encode therefore strips everything — fail-closed in the privacy
/// direction, never a leak.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MetadataPolicy {
    #[default]
    Preserve,
    StripAll,
    /// §29.4 selective stripping (the field-group matrix).
    Selective(SelectiveMeta),
}

/// §8.1 `EncodeOpts`.
#[derive(Debug, Clone)]
pub struct EncodeOpts {
    /// per-codec parameters with the §8.2 defaults
    pub format: CodecOptions,
    /// Preserve | StripAll (§8.6, §29.4); default Preserve
    pub metadata: MetadataPolicy,
    /// default Embed
    pub icc: IccPolicy,
    /// composited under content for alpha-less targets; default opaque
    /// white (LINEAR premultiplied working values, [r, g, b], alpha 1)
    pub matte: [f32; 3],
    /// None = document resolution (§19.1)
    pub ppi: Option<f64>,
}

impl EncodeOpts {
    pub fn new(format: CodecOptions) -> EncodeOpts {
        EncodeOpts {
            format,
            metadata: MetadataPolicy::Preserve,
            icc: IccPolicy::Embed,
            matte: [1.0, 1.0, 1.0],
            ppi: None,
        }
    }

    pub(crate) fn check_interim_unsupported(&self) -> Result<(), CodecError> {
        match self.icc {
            IccPolicy::Embed | IccPolicy::Omit => Ok(()),
            IccPolicy::ConvertToSrgbThenTag | IccPolicy::ConvertToSrgbUntagged => {
                Err(CodecError::unsupported(
                    "encode.icc convert-to-sRGB policies (arrive with the CMM, §5)",
                ))
            }
        }
    }
}

/// §8.1 `CodecOptions` (the built-in arms through slice 2; PSD/RAW and
/// plugin `External` arms arrive with their rows).
#[derive(Debug, Clone)]
pub enum CodecOptions {
    Png(PngOptions),
    Jpeg(JpegOptions),
    Tiff(TiffOptions),
    Webp(WebpOptions),
    Gif(GifOptions),
    Bmp(BmpOptions),
    Ora(OraOptions),
}

/// §8.1 `OraOptions` — no dials at interop v1 (the §8.4 writer rules are
/// all NORMATIVE); the struct anchors the options arm for later rows.
#[derive(Debug, Clone, Default)]
pub struct OraOptions {}

// ---------------------------------------------------------------- §8.2 ----

#[derive(Debug, Clone)]
pub struct PngOptions {
    /// 0..=9 flate level; default 6
    pub compression: u8,
    /// Adam7; default false
    pub interlace: bool,
    /// default Auto
    pub bit_depth: PngDepth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PngDepth {
    Auto,
    B8,
    B16,
}

impl Default for PngOptions {
    fn default() -> PngOptions {
        PngOptions {
            compression: 6,
            interlace: false,
            bit_depth: PngDepth::Auto,
        }
    }
}

#[derive(Debug, Clone)]
pub struct JpegOptions {
    /// 0..=100; default 82
    pub quality: u8,
    /// None = auto. DEVIATION (module-doc register): the §8.2 auto rule
    /// mandates S420 below q90; the elected fixed-4:4:4 encoder resolves
    /// auto to S444 at every quality. Explicit S420/S422 answer typed.
    pub chroma: Option<ChromaSubsampling>,
    /// default FALSE here (spec: true) — elected-encoder deviation, module
    /// doc.
    pub progressive: bool,
    /// MCU rows between RSTn markers; None = no restart markers
    pub restart_interval: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChromaSubsampling {
    S444,
    S422,
    S420,
}

impl Default for JpegOptions {
    fn default() -> JpegOptions {
        JpegOptions {
            quality: 82,
            chroma: None,
            progressive: false,
            restart_interval: None,
        }
    }
}

impl JpegOptions {
    pub(crate) fn validate(&self) -> Result<(), CodecError> {
        if self.quality > 100 {
            return Err(CodecError::OptionOutOfRange {
                field: "jpeg.quality",
                detail: format!("{} not in 0..=100", self.quality),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct TiffOptions {
    /// default Zip
    pub compression: TiffCompression,
    /// false = chunky/contiguous (default)
    pub planar: bool,
    /// default Auto
    pub bigtiff: BigTiff,
    /// None = strips (rows-per-strip auto); Some(256) = tiled TIFF
    pub tile_size: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TiffCompression {
    None,
    Lzw,
    #[default]
    Zip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BigTiff {
    /// emit BigTIFF iff any offset would exceed u32
    #[default]
    Auto,
    Force,
    Never,
}

#[derive(Debug, Clone)]
pub struct WebpOptions {
    /// default TRUE here (spec: false) — elected-encoder deviation, module
    /// doc.
    pub lossless: bool,
    /// 0.0..=100.0; default 82.0 (lossy distortion / lossless effort trade)
    pub quality: f32,
    /// 0..=6 encoder method; default 4
    pub effort: u8,
}

impl Default for WebpOptions {
    fn default() -> WebpOptions {
        WebpOptions {
            lossless: true,
            quality: 82.0,
            effort: 4,
        }
    }
}

impl WebpOptions {
    pub(crate) fn validate(&self) -> Result<(), CodecError> {
        if !(0.0..=100.0).contains(&self.quality) {
            return Err(CodecError::OptionOutOfRange {
                field: "webp.quality",
                detail: format!("{} not in 0.0..=100.0", self.quality),
            });
        }
        if self.effort > 6 {
            return Err(CodecError::OptionOutOfRange {
                field: "webp.effort",
                detail: format!("{} not in 0..=6", self.effort),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct GifOptions {
    /// 2..=256; default 256
    pub max_colors: u16,
    /// default FloydSteinberg
    pub dither: Dither,
    /// 0 = infinite; default 0
    pub loop_count: u16,
    /// for frame layers without "delay:"; default 100
    pub default_delay_ms: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dither {
    None,
    FloydSteinberg,
    Ordered8x8,
}

impl Default for GifOptions {
    fn default() -> GifOptions {
        GifOptions {
            max_colors: 256,
            dither: Dither::FloydSteinberg,
            loop_count: 0,
            default_delay_ms: 100,
        }
    }
}

impl GifOptions {
    pub(crate) fn validate(&self) -> Result<(), CodecError> {
        if !(2..=256).contains(&self.max_colors) {
            return Err(CodecError::OptionOutOfRange {
                field: "gif.max_colors",
                detail: format!("{} not in 2..=256", self.max_colors),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct BmpOptions {
    pub depth: BmpDepth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BmpDepth {
    /// B32 iff alpha present else B24
    #[default]
    Auto,
    B24,
    B32,
}
