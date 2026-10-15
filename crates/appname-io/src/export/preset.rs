// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §29.1 `ExportPreset` + the [NORMATIVE] per-format export
//! option table (camelCase `options` keys) + the web-default column +
//! the reserved `quick` preset; §29.2 slice formats; wire shape per
//! `contracts/commands/slice.add.yaml` `preset` (open object, §7.3).
//!
//! Wire notes, each an interpretation recorded in the lane log:
//! * `profilePolicy` is a plain string in the contract schema; the
//!   `ConvertTo(ProfileId)` arm is carried as `"convertTo:<builtin>"`
//!   over the §5.5 builtin working-space names (`srgb`, `display-p3`,
//!   `prophoto-rgb`, `gray-gamma-22`) — an ICC-bytes destination joins
//!   when a profile-reference wire shape is pinned (v5.6 fold-in
//!   candidate).
//! * `metadataPolicy` strings: `preserve` | `stripAll` | `selective`
//!   (§8.6 arms camelCased per §2.8); `selective` takes the §29.4
//!   dialog defaults ([`crate::opts::SelectiveMeta::default`]) — the
//!   per-dial wire shape (`selective:{...}` object) joins with the
//!   dialog wave.
//! * AVIF is the §29.1 optional-module row and SVG the §29.2
//!   vector-slice arm: both answer typed `UnsupportedFeature` naming
//!   their trigger (no codec election / no vector-source plumbing yet).

use serde_json::Value;

use crate::error::CodecError;
use crate::opts::{
    BmpOptions, ChromaSubsampling, CodecOptions, Dither, GifOptions, JpegOptions, MetadataPolicy,
    PngDepth, PngOptions, SelectiveMeta, TiffCompression, TiffOptions, WebpOptions,
};

/// §29.1 `ProfilePolicy`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfilePolicy {
    /// keep + embed the working/document profile (stage-3 no-op)
    Embed,
    /// convert with the document rendering intent + BPC (§5.2) to a
    /// builtin working space, embed the destination profile
    ConvertTo(appname_color::BuiltinSpace),
    /// convert to sRGB and OMIT the profile — the one policy that may
    /// omit, precisely because sRGB is the documented default
    /// assumption (§29.4 [NORMATIVE] ICC row)
    ConvertToSrgbStrip,
}

/// §29.1 `ResizeSpec` — the `doc.resize` wire triple (one-of), aspect
/// preserved from the missing dimension(s).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeSpec {
    Width(u32),
    Height(u32),
    LongestEdge(u32),
}

impl ResizeSpec {
    /// Target (w, h) for a source extent, aspect-preserving, each axis
    /// clamped to ≥ 1 (§26.2/§19.3 discipline: never a zero axis).
    pub fn target(&self, sw: u32, sh: u32) -> (u32, u32) {
        let scale = |num: u32, den: u32, other: u32| -> u32 {
            ((other as u64 * num as u64 + den as u64 / 2) / den as u64).max(1) as u32
        };
        match *self {
            ResizeSpec::Width(w) => (w.max(1), scale(w.max(1), sw, sh)),
            ResizeSpec::Height(h) => (scale(h.max(1), sh, sw), h.max(1)),
            ResizeSpec::LongestEdge(e) => {
                let e = e.max(1);
                if sw >= sh {
                    (e, scale(e, sw, sh))
                } else {
                    (scale(e, sh, sw), e)
                }
            }
        }
    }
}

/// §29.1 `ExportPreset` (parsed, validated form).
#[derive(Debug, Clone)]
pub struct ExportPreset {
    /// Typed per-format §8.2 options (the §29.1 table mapped onto the
    /// in-crate option structs).
    pub options: CodecOptions,
    pub resize: Option<ResizeSpec>,
    pub profile_policy: ProfilePolicy,
    pub metadata_policy: MetadataPolicy,
    /// §29.2 scale variants; empty = plain 1× (normalized: `[1.0]`).
    pub scale_variants: Vec<f32>,
}

impl ExportPreset {
    /// The §29.1 per-format DEFAULT preset (options column defaults,
    /// `Embed`, `Preserve`, no resize, 1×).
    pub fn default_for(format: &str) -> Result<ExportPreset, CodecError> {
        Ok(ExportPreset {
            options: default_options(format)?,
            resize: None,
            profile_policy: ProfilePolicy::Embed,
            metadata_policy: MetadataPolicy::Preserve,
            scale_variants: vec![1.0],
        })
    }

    /// The §29.1 "Web default" column: per-format web preset.
    pub fn web_default(format: &str) -> Result<ExportPreset, CodecError> {
        let mut p = ExportPreset::default_for(format)?;
        match &mut p.options {
            CodecOptions::Png(o) => {
                // 8-bit, ConvertToSrgbStrip
                o.bit_depth = PngDepth::B8;
                p.profile_policy = ProfilePolicy::ConvertToSrgbStrip;
            }
            CodecOptions::Jpeg(_) => {
                // quality 82 (the default), strip metadata
                p.metadata_policy = MetadataPolicy::StripAll;
                p.profile_policy = ProfilePolicy::ConvertToSrgbStrip;
            }
            CodecOptions::Webp(_) => {
                // lossy 80 — the elected encoder is lossless-only
                // (opts.rs deviation register); web preset keeps the
                // recorded lossless posture until the lossy election
                p.profile_policy = ProfilePolicy::ConvertToSrgbStrip;
            }
            _ => {
                return Err(CodecError::unsupported(format!(
                    "export.web_default.{format} (§29.1: not a web target)"
                )))
            }
        }
        Ok(p)
    }

    /// The reserved `quick` preset (§29.1 Quick export [NORMATIVE]
    /// defaults): PNG 8-bit, `ConvertToSrgbStrip`, `StripAll`, no
    /// resize.
    pub fn quick() -> ExportPreset {
        ExportPreset {
            options: CodecOptions::Png(PngOptions {
                bit_depth: PngDepth::B8,
                ..PngOptions::default()
            }),
            resize: None,
            profile_policy: ProfilePolicy::ConvertToSrgbStrip,
            metadata_policy: MetadataPolicy::StripAll,
            scale_variants: vec![1.0],
        }
    }

    /// Parse the contract wire object (`format` + camelCase `options` +
    /// `resize` + `profilePolicy` + `metadataPolicy` + `scaleVariants`;
    /// `contracts/commands/slice.add.yaml` preset shape). Missing fields
    /// take the §29.1 defaults; out-of-range/unknown values answer
    /// typed.
    pub fn from_wire(v: &Value) -> Result<ExportPreset, CodecError> {
        let obj = v
            .as_object()
            .ok_or_else(|| CodecError::corrupt("preset: not an object"))?;
        let format = obj
            .get("format")
            .and_then(Value::as_str)
            .ok_or_else(|| CodecError::corrupt("preset: missing `format`"))?;
        let mut preset = ExportPreset::default_for(format)?;
        if let Some(opts) = obj.get("options") {
            preset.options = parse_options(format, opts)?;
        }
        if let Some(r) = obj.get("resize") {
            preset.resize = Some(parse_resize(r)?);
        }
        if let Some(p) = obj.get("profilePolicy") {
            let s = p
                .as_str()
                .ok_or_else(|| CodecError::corrupt("preset: profilePolicy must be a string"))?;
            preset.profile_policy = parse_profile_policy(s)?;
        }
        if let Some(m) = obj.get("metadataPolicy") {
            let s = m
                .as_str()
                .ok_or_else(|| CodecError::corrupt("preset: metadataPolicy must be a string"))?;
            preset.metadata_policy = parse_metadata_policy(s)?;
        }
        if let Some(sv) = obj.get("scaleVariants") {
            let arr = sv
                .as_array()
                .ok_or_else(|| CodecError::corrupt("preset: scaleVariants must be an array"))?;
            let mut variants = Vec::with_capacity(arr.len());
            for e in arr {
                let f = e.as_f64().unwrap_or(0.0) as f32;
                if !(0.01..=16.0).contains(&f) {
                    return Err(CodecError::OptionOutOfRange {
                        field: "preset.scaleVariants",
                        detail: format!("{e} not in 0.01..=16"),
                    });
                }
                variants.push(f);
            }
            if !variants.is_empty() {
                preset.scale_variants = variants;
            }
        }
        Ok(preset)
    }

    /// Canonical lowercase output extension, no dot (§26.2 `{ext}`).
    pub fn extension(&self) -> &'static str {
        match self.options {
            CodecOptions::Png(_) => "png",
            CodecOptions::Jpeg(_) => "jpg",
            CodecOptions::Tiff(_) => "tif",
            CodecOptions::Webp(_) => "webp",
            CodecOptions::Gif(_) => "gif",
            CodecOptions::Bmp(_) => "bmp",
            CodecOptions::Ora(_) => "ora",
        }
    }

    /// Registry id of the elected codec (§8.1).
    pub fn codec_id(&self) -> &'static str {
        match self.options {
            CodecOptions::Png(_) => "png",
            CodecOptions::Jpeg(_) => "jpeg",
            CodecOptions::Tiff(_) => "tiff",
            CodecOptions::Webp(_) => "webp",
            CodecOptions::Gif(_) => "gif",
            CodecOptions::Bmp(_) => "bmp",
            CodecOptions::Ora(_) => "ora",
        }
    }
}

fn default_options(format: &str) -> Result<CodecOptions, CodecError> {
    Ok(match format {
        "png" => CodecOptions::Png(PngOptions::default()),
        "jpeg" | "jpg" => CodecOptions::Jpeg(JpegOptions::default()),
        "tiff" | "tif" => CodecOptions::Tiff(TiffOptions::default()),
        "webp" => CodecOptions::Webp(WebpOptions::default()),
        "gif" => CodecOptions::Gif(GifOptions::default()),
        "bmp" => CodecOptions::Bmp(BmpOptions::default()),
        // §29.1 optional-module / §29.2 vector rows — typed with the
        // trigger named (module doc).
        "avif" => {
            return Err(CodecError::unsupported(
                "export.format.avif (§29.1 optional module — no codec election/deps record)",
            ))
        }
        "svg" => {
            return Err(CodecError::unsupported(
                "export.format.svg (§29.2 vector slices — needs the vector-layer source plumbing)",
            ))
        }
        // ORA rides the §8.4 document save-as door (export_ora_document
        // needs the layer stack, not a flattened plane).
        "ora" => {
            return Err(CodecError::unsupported(
                "export.format.ora (§8.4 interop rides the document save-as door, not Export-As)",
            ))
        }
        other => {
            return Err(CodecError::OptionOutOfRange {
                field: "preset.format",
                detail: format!("unknown export format `{other}` (§29.1 table)"),
            })
        }
    })
}

/// The §29.1 options table, camelCase keys, unknown keys rejected (the
/// table IS the dialog contract; a typo must not silently ride).
fn parse_options(format: &str, v: &Value) -> Result<CodecOptions, CodecError> {
    let obj = v
        .as_object()
        .ok_or_else(|| CodecError::corrupt("preset: options must be an object"))?;
    let known: &[&str] = match format {
        "png" => &["depth", "interlace", "compression", "palette", "dither"],
        "jpeg" | "jpg" => &["quality", "progressive", "chroma"],
        "webp" => &["mode", "quality", "effort"],
        "tiff" | "tif" => &["depth", "compression"],
        "gif" => &["colors", "dither", "loop"],
        "bmp" => &[],
        _ => &[],
    };
    for k in obj.keys() {
        if !known.contains(&k.as_str()) {
            return Err(CodecError::OptionOutOfRange {
                field: "preset.options",
                detail: format!("unknown key `{k}` for format `{format}` (§29.1 table)"),
            });
        }
    }
    let get_u = |key: &str| obj.get(key).and_then(Value::as_u64);
    let get_b = |key: &str| obj.get(key).and_then(Value::as_bool);
    let get_s = |key: &str| obj.get(key).and_then(Value::as_str);

    Ok(match format {
        "png" => {
            let mut o = PngOptions::default();
            match get_u("depth") {
                None => {}
                Some(8) => o.bit_depth = PngDepth::B8,
                Some(16) => o.bit_depth = PngDepth::B16,
                Some(d) => {
                    return Err(CodecError::OptionOutOfRange {
                        field: "png.depth",
                        detail: format!("{d} not in {{8, 16}}"),
                    })
                }
            }
            if let Some(b) = get_b("interlace") {
                o.interlace = b;
            }
            if let Some(c) = get_u("compression") {
                if c > 9 {
                    return Err(CodecError::OptionOutOfRange {
                        field: "png.compression",
                        detail: format!("{c} not in 0..=9"),
                    });
                }
                o.compression = c as u8;
            }
            if get_b("palette") == Some(true) {
                // §29.1 PNG-8 palette export: the §8.2 quantizer is
                // in-crate (GIF path) but the PNG palette encode arm is
                // unwired — typed, deferral inventory.
                return Err(CodecError::unsupported(
                    "png.palette export (quantizer present; PNG palette encode arm is a recorded deferral)",
                ));
            }
            CodecOptions::Png(o)
        }
        "jpeg" | "jpg" => {
            let mut o = JpegOptions::default();
            if let Some(q) = get_u("quality") {
                if q > 100 {
                    return Err(CodecError::OptionOutOfRange {
                        field: "jpeg.quality",
                        detail: format!("{q} not in 0..=100"),
                    });
                }
                o.quality = q as u8;
            }
            if let Some(p) = get_b("progressive") {
                o.progressive = p; // true answers typed at encode (opts.rs register)
            }
            match get_s("chroma") {
                None | Some("auto") => o.chroma = None,
                Some("444") => o.chroma = Some(ChromaSubsampling::S444),
                Some("422") => o.chroma = Some(ChromaSubsampling::S422),
                Some("420") => o.chroma = Some(ChromaSubsampling::S420),
                Some(other) => {
                    return Err(CodecError::OptionOutOfRange {
                        field: "jpeg.chroma",
                        detail: format!("`{other}` not in {{auto, 444, 422, 420}}"),
                    })
                }
            }
            CodecOptions::Jpeg(o)
        }
        "webp" => {
            let mut o = WebpOptions::default();
            match get_s("mode") {
                None => {}
                Some("lossless") => o.lossless = true,
                // the elected encoder is lossless-only; the arm stays
                // typed at encode (opts.rs deviation register)
                Some("lossy") => o.lossless = false,
                Some(other) => {
                    return Err(CodecError::OptionOutOfRange {
                        field: "webp.mode",
                        detail: format!("`{other}` not in {{lossy, lossless}}"),
                    })
                }
            }
            if let Some(q) = get_u("quality") {
                if q > 100 {
                    return Err(CodecError::OptionOutOfRange {
                        field: "webp.quality",
                        detail: format!("{q} not in 0..=100"),
                    });
                }
                o.quality = q as f32;
            }
            if let Some(e) = get_u("effort") {
                if e > 6 {
                    return Err(CodecError::OptionOutOfRange {
                        field: "webp.effort",
                        detail: format!("{e} not in 0..=6"),
                    });
                }
                o.effort = e as u8;
            }
            CodecOptions::Webp(o)
        }
        "tiff" | "tif" => {
            let mut o = TiffOptions::default();
            match get_u("depth") {
                // §29.1: 8|16|32, default 16. The elected `tiff` encoder
                // ships 8/16; 32 (f32) is the §6.1 storage-precision arm
                // — typed until the encoder row grows it.
                None | Some(16) | Some(8) => {}
                Some(32) => {
                    return Err(CodecError::unsupported(
                        "tiff.depth 32 export (f32 TIFF arrives with the §6.1 precision row)",
                    ))
                }
                Some(d) => {
                    return Err(CodecError::OptionOutOfRange {
                        field: "tiff.depth",
                        detail: format!("{d} not in {{8, 16, 32}}"),
                    })
                }
            }
            match get_s("compression") {
                None => {}
                Some("none") => o.compression = TiffCompression::None,
                Some("lzw") => o.compression = TiffCompression::Lzw,
                Some("zip") => o.compression = TiffCompression::Zip,
                Some(other) => {
                    return Err(CodecError::OptionOutOfRange {
                        field: "tiff.compression",
                        detail: format!("`{other}` not in {{none, lzw, zip}}"),
                    })
                }
            }
            CodecOptions::Tiff(o)
        }
        "gif" => {
            let mut o = GifOptions::default();
            if let Some(c) = get_u("colors") {
                if !(2..=256).contains(&c) {
                    return Err(CodecError::OptionOutOfRange {
                        field: "gif.colors",
                        detail: format!("{c} not in 2..=256"),
                    });
                }
                o.max_colors = c as u16;
            }
            if let Some(d) = get_b("dither") {
                o.dither = if d {
                    Dither::FloydSteinberg
                } else {
                    Dither::None
                };
            }
            if let Some(l) = get_u("loop") {
                o.loop_count = u16::try_from(l).map_err(|_| CodecError::OptionOutOfRange {
                    field: "gif.loop",
                    detail: format!("{l} not in 0..=65535"),
                })?;
            }
            CodecOptions::Gif(o)
        }
        "bmp" => CodecOptions::Bmp(BmpOptions::default()),
        other => {
            return Err(CodecError::OptionOutOfRange {
                field: "preset.format",
                detail: format!("unknown export format `{other}` (§29.1 table)"),
            })
        }
    })
}

fn parse_resize(v: &Value) -> Result<ResizeSpec, CodecError> {
    let obj = v
        .as_object()
        .ok_or_else(|| CodecError::corrupt("preset: resize must be an object"))?;
    let axis = |key: &'static str| -> Result<Option<u32>, CodecError> {
        match obj.get(key) {
            None => Ok(None),
            Some(x) => {
                let n = x.as_u64().unwrap_or(0);
                if !(1..=1_000_000).contains(&n) {
                    return Err(CodecError::OptionOutOfRange {
                        field: "preset.resize",
                        detail: format!("{key} = {x} not in 1..=1000000 (§19.3)"),
                    });
                }
                Ok(Some(n as u32))
            }
        }
    };
    match (axis("width")?, axis("height")?, axis("longestEdge")?) {
        (Some(w), None, None) => Ok(ResizeSpec::Width(w)),
        (None, Some(h), None) => Ok(ResizeSpec::Height(h)),
        (None, None, Some(e)) => Ok(ResizeSpec::LongestEdge(e)),
        _ => Err(CodecError::corrupt(
            "preset: resize needs exactly one of width | height | longestEdge (§19.3 one-of)",
        )),
    }
}

fn parse_profile_policy(s: &str) -> Result<ProfilePolicy, CodecError> {
    use appname_color::BuiltinSpace;
    Ok(match s {
        "embed" => ProfilePolicy::Embed,
        "convertToSrgbStrip" => ProfilePolicy::ConvertToSrgbStrip,
        other => match other.strip_prefix("convertTo:") {
            Some("srgb") => ProfilePolicy::ConvertTo(BuiltinSpace::Srgb),
            Some("display-p3") => ProfilePolicy::ConvertTo(BuiltinSpace::DisplayP3),
            Some("prophoto-rgb") => ProfilePolicy::ConvertTo(BuiltinSpace::ProPhotoRgb),
            Some("gray-gamma-22") => ProfilePolicy::ConvertTo(BuiltinSpace::GrayGamma22),
            _ => {
                return Err(CodecError::OptionOutOfRange {
                    field: "preset.profilePolicy",
                    detail: format!(
                        "`{other}` not in {{embed, convertToSrgbStrip, convertTo:<space>}}"
                    ),
                })
            }
        },
    })
}

fn parse_metadata_policy(s: &str) -> Result<MetadataPolicy, CodecError> {
    Ok(match s {
        "preserve" => MetadataPolicy::Preserve,
        "stripAll" => MetadataPolicy::StripAll,
        "selective" => MetadataPolicy::Selective(SelectiveMeta::default()),
        other => {
            return Err(CodecError::OptionOutOfRange {
                field: "preset.metadataPolicy",
                detail: format!("`{other}` not in {{preserve, stripAll, selective}}"),
            })
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn per_format_defaults_match_the_291_table() {
        // PNG: depth 8 (Auto holds source depth; table default 8 rides
        // the codec's Auto with an 8-bit source), compression 6,
        // interlace false.
        match ExportPreset::default_for("png").unwrap().options {
            CodecOptions::Png(o) => {
                assert_eq!(o.compression, 6);
                assert!(!o.interlace);
            }
            _ => panic!("png"),
        }
        // JPEG: quality 82.
        match ExportPreset::default_for("jpeg").unwrap().options {
            CodecOptions::Jpeg(o) => assert_eq!(o.quality, 82),
            _ => panic!("jpeg"),
        }
        // TIFF: compression zip.
        match ExportPreset::default_for("tiff").unwrap().options {
            CodecOptions::Tiff(o) => assert_eq!(o.compression, TiffCompression::Zip),
            _ => panic!("tiff"),
        }
        // GIF: 256 colors, dither on, infinite loop.
        match ExportPreset::default_for("gif").unwrap().options {
            CodecOptions::Gif(o) => {
                assert_eq!(o.max_colors, 256);
                assert_eq!(o.dither, Dither::FloydSteinberg);
                assert_eq!(o.loop_count, 0);
            }
            _ => panic!("gif"),
        }
    }

    #[test]
    fn quick_preset_is_the_normative_quartet() {
        let q = ExportPreset::quick();
        assert!(matches!(
            q.options,
            CodecOptions::Png(PngOptions {
                bit_depth: PngDepth::B8,
                ..
            })
        ));
        assert_eq!(q.profile_policy, ProfilePolicy::ConvertToSrgbStrip);
        assert_eq!(q.metadata_policy, MetadataPolicy::StripAll);
        assert!(q.resize.is_none());
    }

    #[test]
    fn wire_parse_full_object() {
        let p = ExportPreset::from_wire(&json!({
            "format": "webp",
            "options": { "mode": "lossless", "quality": 80, "effort": 6 },
            "profilePolicy": "convertToSrgbStrip",
            "metadataPolicy": "stripAll",
            "scaleVariants": [1, 2, 3],
            "resize": { "longestEdge": 2048 }
        }))
        .unwrap();
        assert_eq!(p.scale_variants, vec![1.0, 2.0, 3.0]);
        assert_eq!(p.resize, Some(ResizeSpec::LongestEdge(2048)));
        assert_eq!(p.profile_policy, ProfilePolicy::ConvertToSrgbStrip);
        assert!(matches!(p.metadata_policy, MetadataPolicy::StripAll));
    }

    #[test]
    fn unknown_option_key_rejects_typed() {
        let e = ExportPreset::from_wire(&json!({
            "format": "png", "options": { "qualitty": 80 }
        }))
        .unwrap_err();
        assert!(matches!(e, CodecError::OptionOutOfRange { .. }), "{e}");
    }

    #[test]
    fn out_of_range_values_reject_typed() {
        for (fmt, opts) in [
            ("jpeg", json!({ "quality": 101 })),
            ("png", json!({ "compression": 10 })),
            ("webp", json!({ "effort": 7 })),
            ("gif", json!({ "colors": 1 })),
        ] {
            let e =
                ExportPreset::from_wire(&json!({ "format": fmt, "options": opts })).unwrap_err();
            assert!(
                matches!(e, CodecError::OptionOutOfRange { .. }),
                "{fmt}: {e}"
            );
        }
    }

    #[test]
    fn optional_module_formats_answer_typed() {
        for fmt in ["avif", "svg"] {
            let e = ExportPreset::default_for(fmt).unwrap_err();
            assert!(matches!(e, CodecError::UnsupportedFeature { .. }), "{fmt}");
        }
    }

    #[test]
    fn resize_targets_preserve_aspect() {
        assert_eq!(ResizeSpec::Width(200).target(400, 300), (200, 150));
        assert_eq!(ResizeSpec::Height(150).target(400, 300), (200, 150));
        assert_eq!(ResizeSpec::LongestEdge(200).target(400, 300), (200, 150));
        assert_eq!(ResizeSpec::LongestEdge(200).target(300, 400), (150, 200));
        // never a zero axis
        assert_eq!(ResizeSpec::Width(1).target(10000, 10), (1, 1));
    }

    #[test]
    fn profile_policy_wire_forms() {
        assert_eq!(parse_profile_policy("embed").unwrap(), ProfilePolicy::Embed);
        assert!(matches!(
            parse_profile_policy("convertTo:display-p3").unwrap(),
            ProfilePolicy::ConvertTo(_)
        ));
        assert!(parse_profile_policy("convertTo:cmyk").is_err());
    }
}
