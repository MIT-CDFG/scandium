// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §8.4 OpenRaster import/export [NORMATIVE: composite-op map,
//! nearest-op fallbacks + `appname:blend` precedence, unknown-op rule,
//! `stack.xml` attribute mapping, exporter entry order, mergedimage
//! mandatory, `appname:` extension]; interop truth
//! `bogos/tests/interop/ora-fixtures.yaml`; DD ⑫. ORA = ZIP + `mimetype`
//! (`image/openraster`) + `stack.xml` + `data/` layer PNGs +
//! `mergedimage.png`.
//!
//! **Interop v1 scope + registered deviations (slice-1 register pattern —
//! typed or in-artifact, never silent):**
//! * DEFLATED zip entries DECODE (the zip record's `electedFeatures`
//!   deflate election, O-INT ruling 2026-08-13, via the granular
//!   `deflate-flate2` feature) — foreign ORAs from other apps usually
//!   deflate. Our own exports remain Stored throughout, like the
//!   `.appname` package. Entries using any OTHER compression method
//!   (bzip2, zstd, …) still answer typed `UnsupportedFeature`.
//! * Layer-PNG `iCCP` is DETECTED and CARRIED raw (`ImportedLayer::icc`,
//!   still zlib-compressed as stored — the CARRIAGE format is
//!   deliberately as-stored; inflating belongs to whoever honors it) with
//!   a `CompatItem`; conversion/honoring rides the CMM seam (§5). Absent
//!   a chunk, layer data is assumed sRGB per the ORA spec — which is
//!   exactly the interim color assumption.
//! * Layer name/opacity/visibility/blend apply JOURNALED at engine
//!   landing (`import_into_engine`: `layer.rename` + `layer.set_props`,
//!   the wave-4 kinds). Group `isolation="auto"` (pass-through) and
//!   nesting are parsed and carried (`ImportedLayer::group_path` +
//!   `CompatItem`); group RECONSTRUCTION via `layer.group` is the one
//!   stack.xml surface still carried-not-applied at v1. Missing
//!   `isolation` reads as `isolate` per the ORA spec default.
//! * `appname-extra.json` round-trip: v1 writes
//!   `fillOpacity`/`clipToBelow` and a `warnings` array naming anything
//!   omitted (masks); on import, entries are UUID-matched against
//!   `appname:layerId` — orphans dropped with a `CompatItem` (fixture case
//!   `orphaned-extras-dropped`), matches carried with a `CompatItem` until
//!   the doc-model application surface lands. `parametric` payloads do not
//!   arise (no non-raster layers exist yet).
//! * The §8.1 trait `encode` writes the FLATTENED single-layer ORA (the
//!   never-silent fallback shape); the rich layered writer is
//!   [`export_ora_document`] (doc tree + established-path composite).
//! * `mergedimage.png` absent on import: the bottom layer stands in as the
//!   primary surface with a `CompatItem` (our exports always write it —
//!   fixture case `mergedimage-mandatory`).
//!
//! `stack.xml` is read/written by the in-crate bounded XML subset
//! ([`crate::xmlmini`] — no quick-xml deps record exists; flagged).

use std::io::Read;

use appname_core::{encode_composite_png, BlendMode, Document, IVec2};

use crate::error::CodecError;
use crate::meta::Metadata;
use crate::opts::{CodecOptions, DecodeOpts, EncodeOpts};
use crate::plane::RgbaPlane;
use crate::registry::{
    Codec, CodecCaps, CodecId, CompatAction, CompatReport, ExportImage, Imported, ImportedLayer,
    LicenseStatus,
};

pub struct OraCodec;

pub const ORA_MIMETYPE: &[u8] = b"image/openraster";
const APPNAME_NS: &str = "https://appname.org/ns/ora/1";

// ------------------------------------------------------------ blend maps ----

/// The §8.4 16-op map, import direction column order (NORMATIVE both ways).
const ORA_OP_MAP: [(&str, BlendMode); 16] = [
    ("svg:src-over", BlendMode::Normal),
    ("svg:multiply", BlendMode::Multiply),
    ("svg:screen", BlendMode::Screen),
    ("svg:overlay", BlendMode::Overlay),
    ("svg:darken", BlendMode::Darken),
    ("svg:lighten", BlendMode::Lighten),
    ("svg:color-dodge", BlendMode::ColorDodge),
    ("svg:plus", BlendMode::LinearDodge),
    ("svg:color-burn", BlendMode::ColorBurn),
    ("svg:hard-light", BlendMode::HardLight),
    ("svg:soft-light", BlendMode::SoftLight),
    ("svg:difference", BlendMode::Difference),
    ("svg:hue", BlendMode::Hue),
    ("svg:saturation", BlendMode::Saturation),
    ("svg:color", BlendMode::Color),
    ("svg:luminosity", BlendMode::Luminosity),
];

/// §8.4 nearest-op fallbacks for the 11 Appname-only modes.
const NEAREST_OP: [(BlendMode, &str); 11] = [
    (BlendMode::Dissolve, "svg:src-over"),
    (BlendMode::LinearBurn, "svg:color-burn"),
    (BlendMode::DarkerColor, "svg:darken"),
    (BlendMode::LighterColor, "svg:lighten"),
    (BlendMode::VividLight, "svg:hard-light"),
    (BlendMode::LinearLight, "svg:hard-light"),
    (BlendMode::PinLight, "svg:hard-light"),
    (BlendMode::HardMix, "svg:hard-light"),
    (BlendMode::Exclusion, "svg:difference"),
    (BlendMode::Subtract, "svg:difference"),
    (BlendMode::Divide, "svg:color-dodge"),
];

/// Import: `composite-op` → BlendMode (§8.4 column 1 → column 2).
pub fn blend_from_ora_op(op: &str) -> Option<BlendMode> {
    ORA_OP_MAP.iter().find(|(o, _)| *o == op).map(|(_, b)| *b)
}

/// The canonical §7.3 blend string (the schema's camelCase publication).
pub fn canonical_blend_string(b: BlendMode) -> String {
    match serde_json::to_value(b) {
        Ok(serde_json::Value::String(s)) => s,
        _ => "normal".to_string(), // unreachable: the enum serializes to strings
    }
}

/// Parse a canonical §7.3 blend string (`appname:blend` values).
pub fn blend_from_canonical(s: &str) -> Option<BlendMode> {
    serde_json::from_value(serde_json::Value::String(s.to_string())).ok()
}

/// Export: BlendMode → (`composite-op`, optional `appname:blend`) — exact
/// inverse for the 16, pinned nearest + canonical string for the 11.
pub fn ora_op_from_blend(b: BlendMode) -> (&'static str, Option<String>) {
    if let Some((op, _)) = ORA_OP_MAP.iter().find(|(_, m)| *m == b) {
        return (op, None);
    }
    let (_, nearest) = NEAREST_OP
        .iter()
        .find(|(m, _)| *m == b)
        .expect("all 27 modes are 16 exact + 11 nearest");
    (nearest, Some(canonical_blend_string(b)))
}

// ------------------------------------------------------------- zip glue ----

fn map_zip_err(context: &str, e: zip::result::ZipError) -> CodecError {
    use zip::result::ZipError as Z;
    match e {
        Z::FileNotFound => CodecError::corrupt(format!("{context}: entry missing")),
        Z::UnsupportedArchive(detail) => CodecError::unsupported(format!(
            "ora.zip: {detail} (entry compression method outside the elected set: Stored + Deflated)"
        )),
        Z::Io(e) => CodecError::io_read(e),
        other => CodecError::corrupt(format!("{context}: {other}")),
    }
}

fn read_entry(
    za: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>,
    name: &str,
) -> Result<Vec<u8>, CodecError> {
    let mut f = za.by_name(name).map_err(|e| map_zip_err(name, e))?;
    let mut out = Vec::new();
    // Deflated entries inflate here (deflate election); a read failure on
    // an entry the archive layer accepted is corrupt entry data.
    f.read_to_end(&mut out)
        .map_err(|e| CodecError::corrupt(format!("ora.zip entry `{name}`: {e}")))?;
    Ok(out)
}

/// §8.1 bomb gate for member PNGs: IHDR dims from the fixed layout.
fn png_dims(bytes: &[u8]) -> Result<(u32, u32), CodecError> {
    let d = |o: usize| -> Option<u32> {
        Some(u32::from_be_bytes(bytes.get(o..o + 4)?.try_into().ok()?))
    };
    match (bytes.get(12..16), d(16), d(20)) {
        (Some(b"IHDR"), Some(w), Some(h)) => Ok((w, h)),
        _ => Err(CodecError::corrupt("layer PNG missing IHDR")),
    }
}

/// Raw `iCCP` chunk payload (profile name + method + zlib-compressed ICC),
/// carried as stored — see module doc.
fn png_iccp_raw(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut i = 8usize;
    while i + 8 <= bytes.len() {
        let len = u32::from_be_bytes(bytes.get(i..i + 4)?.try_into().ok()?) as usize;
        let ty = bytes.get(i + 4..i + 8)?;
        if ty == b"iCCP" {
            return bytes
                .get(i + 8..i.checked_add(8)?.checked_add(len)?)
                .map(<[u8]>::to_vec);
        }
        if ty == b"IDAT" || ty == b"IEND" {
            return None; // iCCP must precede IDAT
        }
        i = i.checked_add(12)?.checked_add(len)?;
    }
    None
}

// --------------------------------------------------------------- import ----

struct ParsedLayer {
    name: String,
    src: String,
    x: i32,
    y: i32,
    opacity: f32,
    visible: bool,
    blend: BlendMode,
    layer_uuid: Option<String>,
    group_path: Vec<String>,
}

fn parse_f32_attr(
    elem: &crate::xmlmini::Elem,
    name: &str,
    default: f32,
    report: &mut CompatReport,
) -> f32 {
    match elem.attr(name) {
        None => default,
        Some(v) => match v.parse::<f32>() {
            Ok(f) if f.is_finite() => f,
            _ => {
                report.push(
                    format!("ora.stack_xml.{name}"),
                    CompatAction::Approximated,
                    format!("unparseable `{name}=\"{v}\"` defaulted to {default}"),
                );
                default
            }
        },
    }
}

fn parse_i32_attr(elem: &crate::xmlmini::Elem, name: &str, report: &mut CompatReport) -> i32 {
    match elem.attr(name) {
        None => 0,
        Some(v) => v.parse::<i32>().unwrap_or_else(|_| {
            report.push(
                format!("ora.stack_xml.{name}"),
                CompatAction::Approximated,
                format!("unparseable `{name}=\"{v}\"` defaulted to 0"),
            );
            0
        }),
    }
}

fn walk_stack(
    elem: &crate::xmlmini::Elem,
    path: &mut Vec<String>,
    out: &mut Vec<ParsedLayer>,
    report: &mut CompatReport,
) {
    // Document order in stack.xml = TOPMOST first (ORA spec).
    for child in &elem.children {
        match child.name.as_str() {
            "layer" => {
                let name = child.attr("name").unwrap_or("Layer").to_string();
                let src = match child.attr("src") {
                    Some(s) => s.to_string(),
                    None => {
                        report.push(
                            "ora.stack_xml.layer",
                            CompatAction::Dropped,
                            format!("layer `{name}` has no src attribute"),
                        );
                        continue;
                    }
                };
                let opacity = parse_f32_attr(child, "opacity", 1.0, report).clamp(0.0, 1.0);
                let visible = match child.attr("visibility") {
                    None | Some("visible") => true,
                    Some("hidden") => false,
                    Some(other) => {
                        report.push(
                            "ora.stack_xml.visibility",
                            CompatAction::Approximated,
                            format!("unknown visibility `{other}` read as visible"),
                        );
                        true
                    }
                };
                // §8.4 precedence: appname:blend (canonical) wins over
                // composite-op; unknown composite-op → Normal + CompatItem.
                let blend = match child.attr("appname:blend").and_then(blend_from_canonical) {
                    Some(b) => b,
                    None => {
                        if let Some(bad) = child.attr("appname:blend") {
                            report.push(
                                "ora.appname_blend",
                                CompatAction::Approximated,
                                format!("unknown appname:blend `{bad}`; composite-op used"),
                            );
                        }
                        let op = child.attr("composite-op").unwrap_or("svg:src-over");
                        match blend_from_ora_op(op) {
                            Some(b) => b,
                            None => {
                                report.push(
                                    "ora.composite_op",
                                    CompatAction::Approximated,
                                    format!("unknown composite-op `{op}` mapped to Normal"),
                                );
                                BlendMode::Normal
                            }
                        }
                    }
                };
                out.push(ParsedLayer {
                    name,
                    src,
                    x: parse_i32_attr(child, "x", report),
                    y: parse_i32_attr(child, "y", report),
                    opacity,
                    visible,
                    blend,
                    layer_uuid: child.attr("appname:layerId").map(str::to_string),
                    group_path: path.clone(),
                });
            }
            "stack" => {
                let name = child.attr("name").unwrap_or("Group").to_string();
                // Missing isolation reads as isolate (ORA spec default,
                // fixture check); `auto` = pass-through (§4.7), carried.
                match child.attr("isolation") {
                    None | Some("isolate") => {}
                    Some("auto") => report.push(
                        "ora.stack_isolation",
                        CompatAction::Info,
                        format!(
                            "group `{name}` is pass-through (isolation=auto); carried on group_path until group reconstruction lands"
                        ),
                    ),
                    Some(other) => report.push(
                        "ora.stack_isolation",
                        CompatAction::Approximated,
                        format!("unknown isolation `{other}` on `{name}` read as isolate"),
                    ),
                }
                path.push(name);
                walk_stack(child, path, out, report);
                path.pop();
            }
            other => report.push(
                "ora.stack_xml.element",
                CompatAction::Dropped,
                format!("unknown element <{other}> skipped"),
            ),
        }
    }
}

/// Reject entry names that could escape the archive namespace.
fn sane_entry_name(src: &str) -> Result<(), CodecError> {
    if src.starts_with('/') || src.split('/').any(|c| c == "..") || src.contains('\\') {
        return Err(CodecError::corrupt(format!(
            "suspicious entry path `{src}`"
        )));
    }
    Ok(())
}

impl Codec for OraCodec {
    fn id(&self) -> CodecId {
        CodecId("ora")
    }

    fn capabilities(&self) -> CodecCaps {
        CodecCaps {
            read: true,
            write: true,
            layered: true,
            max_depth: appname_core::ChannelKind::U16,
            alpha: true,
            animation: false,
            license: LicenseStatus::Builtin,
        }
    }

    /// §8.1 magic: `PK\x03\x04` @0 + `mimetype` @30 + `image/openraster`
    /// @38, effective length 54.
    fn sniff(&self, head: &[u8]) -> bool {
        head.len() >= 54
            && head.starts_with(b"PK\x03\x04")
            && &head[30..38] == b"mimetype"
            && &head[38..54] == ORA_MIMETYPE
    }

    fn magic_len(&self) -> u32 {
        54
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["ora"]
    }

    fn decode(&self, bytes: &[u8], opts: &DecodeOpts) -> Result<Imported, CodecError> {
        opts.check_interim_unsupported()?;
        let mut report = CompatReport::default();
        let mut za = zip::ZipArchive::new(std::io::Cursor::new(bytes))
            .map_err(|e| map_zip_err("archive", e))?;
        if read_entry(&mut za, "mimetype")? != ORA_MIMETYPE {
            return Err(CodecError::corrupt(
                "mimetype entry is not image/openraster",
            ));
        }
        let stack_xml = read_entry(&mut za, "stack.xml")?;
        let root = crate::xmlmini::parse(&stack_xml)?;
        if root.name != "image" {
            return Err(CodecError::corrupt(format!("root element <{}>", root.name)));
        }
        let dim = |n: &str| -> Result<u32, CodecError> {
            root.attr(n)
                .and_then(|v| v.parse::<u32>().ok())
                .ok_or_else(|| CodecError::corrupt(format!("image {n} attribute")))
        };
        let (w, h) = (dim("w")?, dim("h")?);
        opts.check_limits(w, h)?;

        // Walk every top-level stack, topmost-first, then flip to
        // bottom-to-top (§8.1 layer order).
        let mut parsed = Vec::new();
        let mut path = Vec::new();
        for stack in root.children_named("stack") {
            walk_stack(stack, &mut path, &mut parsed, &mut report);
        }
        parsed.reverse();
        if parsed.is_empty() {
            return Err(CodecError::corrupt("stack.xml holds no layers"));
        }
        if !opts.frames_as_layers {
            // The nearest analog of first-frame-only: keep the bottom layer.
            parsed.truncate(1);
        }

        let mut layers = Vec::with_capacity(parsed.len());
        let mut known_uuids = Vec::new();
        for p in &parsed {
            sane_entry_name(&p.src)?;
            let png = read_entry(&mut za, &p.src)?;
            let (lw, lh) = png_dims(&png)?;
            opts.check_limits(lw, lh)?;
            let icc = png_iccp_raw(&png);
            if icc.is_some() {
                report.push(
                    "ora.layer_iccp",
                    CompatAction::Info,
                    format!(
                        "layer `{}` carries an iCCP profile (carried compressed; honored at the CMM seam)",
                        p.name
                    ),
                );
            }
            let (buffer, _, _) = appname_core::decode_png_to_buffer(&png, opts.working_profile)
                .map_err(|e| CodecError::corrupt(format!("layer `{}`: {e}", p.name)))?;
            if let Some(u) = &p.layer_uuid {
                known_uuids.push(u.clone());
            }
            layers.push(ImportedLayer {
                name: p.name.clone(),
                buffer,
                ingress_png: png,
                offset: IVec2::new(p.x, p.y),
                opacity: p.opacity,
                fill_opacity: 1.0,
                blend: p.blend,
                visible: p.visible,
                clip_to_below: false,
                locks: crate::registry::LayerLocks::default(),
                group_path: p.group_path.clone(),
                icc,
            });
        }

        // appname-extra.json: UUID-match against appname:layerId; orphans
        // dropped with a CompatItem (§8.4 / fixture orphaned-extras-dropped).
        if let Ok(extra) = read_entry(&mut za, "appname-extra.json") {
            match serde_json::from_slice::<serde_json::Value>(&extra) {
                Ok(v)
                    if v.get("schema").and_then(|s| s.as_str()) == Some("appname.oraExtra.v1") =>
                {
                    if let Some(map) = v.get("layers").and_then(|l| l.as_object()) {
                        for key in map.keys() {
                            if known_uuids.iter().any(|u| u == key) {
                                report.push(
                                    "ora.appname_extra",
                                    CompatAction::Info,
                                    format!(
                                        "extension data for layer {key} carried (applied when the doc-model surface lands)"
                                    ),
                                );
                            } else {
                                report.push(
                                    "ora.appname_extra",
                                    CompatAction::Dropped,
                                    format!("extras entry {key} matches no appname:layerId"),
                                );
                            }
                        }
                    }
                }
                Ok(_) => report.push(
                    "ora.appname_extra",
                    CompatAction::Dropped,
                    "appname-extra.json has an unknown schema",
                ),
                Err(e) => report.push(
                    "ora.appname_extra",
                    CompatAction::Dropped,
                    format!("appname-extra.json is malformed: {e}"),
                ),
            }
        }

        // Primary surface: mergedimage.png (mandatory in OUR exports);
        // absent in a foreign file → bottom layer stands in, reported.
        let (ingress_png, buffer) = match read_entry(&mut za, "mergedimage.png") {
            Ok(png) => {
                let (mw, mh) = png_dims(&png)?;
                opts.check_limits(mw, mh)?;
                let (buffer, _, _) = appname_core::decode_png_to_buffer(&png, opts.working_profile)
                    .map_err(|e| CodecError::corrupt(format!("mergedimage: {e}")))?;
                (png, buffer)
            }
            Err(_) => {
                report.push(
                    "ora.mergedimage",
                    CompatAction::Info,
                    "mergedimage.png absent; bottom layer stands in as the primary surface",
                );
                (layers[0].ingress_png.clone(), layers[0].buffer.clone())
            }
        };

        Ok(Imported {
            buffer,
            ingress_png,
            size: (w, h),
            profile: None, // ORA carries no document-level ICC; per-layer iCCP rides ImportedLayer::icc
            meta: Metadata::default(),
            layers: Some(layers),
            orientation_applied: None, // ORA carries no EXIF
            report,
            compat_report: None,
        })
    }

    /// The §8.1 trait writer: the FLATTENED single-layer ORA (module doc;
    /// the rich layered writer is [`export_ora_document`]).
    fn encode(&self, img: &ExportImage<'_>, opts: &EncodeOpts) -> Result<Vec<u8>, CodecError> {
        let CodecOptions::Ora(_) = &opts.format else {
            return Err(CodecError::internal("ora codec handed non-ora options"));
        };
        opts.check_interim_unsupported()?;
        let png =
            encode_composite_png(img.width, img.height, img.plane).map_err(CodecError::internal)?;
        let id = crate::derive_uuid(&format!(
            "appname-io/ora/flat-layer/{}",
            appname_core::ContentHash::of_bytes(&png).to_hex()
        ));
        let layer = OraLayerEntry {
            xml: format!(
                "    <layer name=\"Composite\" src=\"data/{id}.png\" x=\"0\" y=\"0\" opacity=\"1\" visibility=\"visible\" composite-op=\"svg:src-over\" appname:layerId=\"{id}\"/>\n"
            ),
            path: format!("data/{id}.png"),
            png: png.clone(),
        };
        write_ora(
            img.width,
            img.height,
            &[layer],
            &png,
            img.plane,
            serde_json::Map::new(),
            Vec::new(),
        )
    }
}

// --------------------------------------------------------------- export ----

struct OraLayerEntry {
    xml: String,
    path: String,
    png: Vec<u8>,
}

/// Convert one layer buffer's bounds rect to a straight-sRGB plane (8-bit
/// for `U8` storage, 16-bit otherwise — §8.4 depth rule; f32 working
/// buffers carry ≥8-bit content, quantized to 16).
fn buffer_rect_plane(
    buf: &appname_core::Buffer,
    rect: appname_core::IRect,
) -> Result<RgbaPlane, CodecError> {
    use appname_core::io::srgb::{srgb_encode, unpremul};
    let (w, h) = (rect.width() as u32, rect.height() as u32);
    let sixteen = buf.kind() != appname_core::ChannelKind::U8;
    if sixteen {
        let mut px16 = Vec::with_capacity(w as usize * h as usize * 4);
        for y in rect.min.y..rect.max.y {
            for x in rect.min.x..rect.max.x {
                let s = unpremul(buf.read_pixel_f32(IVec2::new(x, y)));
                px16.push((srgb_encode(s[0].clamp(0.0, 1.0)) * 65535.0).round() as u16);
                px16.push((srgb_encode(s[1].clamp(0.0, 1.0)) * 65535.0).round() as u16);
                px16.push((srgb_encode(s[2].clamp(0.0, 1.0)) * 65535.0).round() as u16);
                px16.push((s[3].clamp(0.0, 1.0) * 65535.0).round() as u16);
            }
        }
        RgbaPlane::new16(w, h, px16)
    } else {
        let mut px8 = Vec::with_capacity(w as usize * h as usize * 4);
        for y in rect.min.y..rect.max.y {
            for x in rect.min.x..rect.max.x {
                let s = unpremul(buf.read_pixel_f32(IVec2::new(x, y)));
                px8.push((srgb_encode(s[0].clamp(0.0, 1.0)) * 255.0).round() as u8);
                px8.push((srgb_encode(s[1].clamp(0.0, 1.0)) * 255.0).round() as u8);
                px8.push((srgb_encode(s[2].clamp(0.0, 1.0)) * 255.0).round() as u8);
                px8.push((s[3].clamp(0.0, 1.0) * 255.0).round() as u8);
            }
        }
        RgbaPlane::new8(w, h, px8)
    }
}

/// Nearest-neighbor thumbnail plane (≤256/axis, aspect kept, §8.4).
fn thumbnail_plane(w: u32, h: u32, plane: &[f32]) -> (u32, u32, Vec<f32>) {
    let long = w.max(h);
    if long <= 256 {
        return (w, h, plane.to_vec());
    }
    let tw = ((w as u64 * 256 / long as u64).max(1)) as u32;
    let th = ((h as u64 * 256 / long as u64).max(1)) as u32;
    let mut out = Vec::with_capacity(tw as usize * th as usize * 4);
    for y in 0..th {
        let sy = ((y as u64 * 2 + 1) * h as u64 / (th as u64 * 2)) as usize;
        for x in 0..tw {
            let sx = ((x as u64 * 2 + 1) * w as u64 / (tw as u64 * 2)) as usize;
            let i = (sy * w as usize + sx) * 4;
            out.extend_from_slice(&plane[i..i + 4]);
        }
    }
    (tw, th, out)
}

/// Assemble the container in the NORMATIVE §8.4 entry order (everything
/// Stored — deterministic, and PNG payloads are already compressed).
fn write_ora(
    w: u32,
    h: u32,
    layers_top_first: &[OraLayerEntry],
    merged_png: &[u8],
    composite: &[f32],
    extra_layers: serde_json::Map<String, serde_json::Value>,
    extra_warnings: Vec<String>,
) -> Result<Vec<u8>, CodecError> {
    let mut stack_xml = String::new();
    stack_xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    stack_xml.push_str(&format!(
        "<image w=\"{w}\" h=\"{h}\" version=\"0.0.3\" xmlns:appname=\"{APPNAME_NS}\">\n  <stack>\n"
    ));
    for l in layers_top_first {
        stack_xml.push_str(&l.xml);
    }
    stack_xml.push_str("  </stack>\n</image>\n");

    let (tw, th, tplane) = thumbnail_plane(w, h, composite);
    let thumb_png = encode_composite_png(tw, th, &tplane).map_err(CodecError::internal)?;

    let mut extra = serde_json::Map::new();
    extra.insert("schema".into(), "appname.oraExtra.v1".into());
    extra.insert(
        "producer".into(),
        serde_json::json!({ "app": "Appname", "version": env!("CARGO_PKG_VERSION") }),
    );
    extra.insert("layers".into(), serde_json::Value::Object(extra_layers));
    if !extra_warnings.is_empty() {
        extra.insert(
            "warnings".into(),
            serde_json::Value::Array(
                extra_warnings
                    .into_iter()
                    .map(serde_json::Value::String)
                    .collect(),
            ),
        );
    }
    let extra_json =
        serde_json::to_vec(&serde_json::Value::Object(extra)).map_err(CodecError::internal)?;

    fn put(
        zw: &mut zip::ZipWriter<&mut std::io::Cursor<Vec<u8>>>,
        name: &str,
        bytes: &[u8],
    ) -> Result<(), CodecError> {
        use std::io::Write;
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zw.start_file(name, opts)
            .map_err(|e| CodecError::internal(format!("zip entry {name}: {e}")))?;
        zw.write_all(bytes).map_err(CodecError::io_write)
    }
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut zw = zip::ZipWriter::new(&mut cursor);
        put(&mut zw, "mimetype", ORA_MIMETYPE)?;
        put(&mut zw, "stack.xml", stack_xml.as_bytes())?;
        for l in layers_top_first {
            // Group markers carry XML only — no zip entry of their own.
            if !l.path.is_empty() {
                put(&mut zw, &l.path, &l.png)?;
            }
        }
        put(&mut zw, "mergedimage.png", merged_png)?;
        put(&mut zw, "Thumbnails/thumbnail.png", &thumb_png)?;
        put(&mut zw, "appname-extra.json", &extra_json)?;
        zw.finish().map_err(CodecError::internal)?;
    }
    Ok(cursor.into_inner())
}

/// The rich §8.4 writer: document layer tree + the established-path
/// composite (render `region_rgba_f32` / core reference math — the same
/// plane every other encoder consumes; keeping the compositor out of this
/// crate preserves the §1.4 dependency direction).
pub fn export_ora_document(doc: &Document, composite: &[f32]) -> Result<Vec<u8>, CodecError> {
    let canvas = doc.geometry().canvas_rect();
    let (w, h) = (canvas.width() as u32, canvas.height() as u32);
    if composite.len() != w as usize * h as usize * 4 {
        return Err(CodecError::internal(
            "composite plane length must be canvas w*h*4",
        ));
    }

    let mut layers: Vec<OraLayerEntry> = Vec::new();
    let mut extra_layers = serde_json::Map::new();
    let mut warnings = Vec::new();
    collect_layers(
        doc,
        doc.root(),
        2,
        &mut layers,
        &mut extra_layers,
        &mut warnings,
    )?;
    if layers.is_empty() {
        return Err(CodecError::unsupported(
            "ora.empty_document (a document with no raster layers has no ORA form)",
        ));
    }
    let merged = encode_composite_png(w, h, composite).map_err(CodecError::internal)?;
    write_ora(w, h, &layers, &merged, composite, extra_layers, warnings)
}

/// Walk children TOPMOST-first (doc order is bottom-to-top → reversed),
/// emitting nested `<stack>` for groups.
fn collect_layers(
    doc: &Document,
    parent: appname_core::LayerId,
    indent: usize,
    out: &mut Vec<OraLayerEntry>,
    extra_layers: &mut serde_json::Map<String, serde_json::Value>,
    warnings: &mut Vec<String>,
) -> Result<(), CodecError> {
    let children = doc.children_of(parent).unwrap_or(&[]);
    let pad = " ".repeat(indent * 2);
    for id in children.iter().rev() {
        let Some(node) = doc.layer(*id) else { continue };
        let uuid = node.id().as_uuid();
        if node.is_group() {
            out.push(OraLayerEntry {
                xml: format!(
                    "{pad}<stack name=\"{}\" isolation=\"isolate\" appname:layerId=\"{uuid}\">\n",
                    crate::xmlmini::escape_attr(node.name())
                ),
                path: String::new(),
                png: Vec::new(),
            });
            collect_layers(doc, *id, indent + 1, out, extra_layers, warnings)?;
            out.push(OraLayerEntry {
                xml: format!("{pad}</stack>\n"),
                path: String::new(),
                png: Vec::new(),
            });
            continue;
        }
        let Some(buf) = node.pixels() else {
            warnings.push(format!(
                "layer {uuid} ({}) is not raster; skipped at interop v1",
                node.name()
            ));
            continue;
        };
        let bounds = buf.bounds();
        let (rect, plane) = if bounds.is_empty() {
            // An empty layer still needs a src (PNG has no 0-dim form).
            (
                appname_core::IRect::new(0, 0, 1, 1),
                RgbaPlane::new8(1, 1, vec![0, 0, 0, 0])?,
            )
        } else {
            (bounds, buffer_rect_plane(buf, bounds)?)
        };
        let png = plane.to_png_bytes()?;
        let (op, appname_blend) = ora_op_from_blend(node.blend());
        let blend_attr = appname_blend
            .map(|s| format!(" appname:blend=\"{s}\""))
            .unwrap_or_default();
        let src = format!("data/{uuid}.png");
        out.push(OraLayerEntry {
            xml: format!(
                "{pad}<layer name=\"{}\" src=\"{src}\" x=\"{}\" y=\"{}\" opacity=\"{}\" visibility=\"{}\" composite-op=\"{op}\"{blend_attr} appname:layerId=\"{uuid}\"/>\n",
                crate::xmlmini::escape_attr(node.name()),
                rect.min.x,
                rect.min.y,
                node.opacity(),
                if node.visible() { "visible" } else { "hidden" },
            ),
            path: src,
            png,
        });
        // appname-extra.json entry when the layer carries ORA-inexpressible
        // state (v1 subset — module doc).
        let mut entry = serde_json::Map::new();
        if node.fill_opacity() < 1.0 {
            entry.insert("fillOpacity".into(), serde_json::json!(node.fill_opacity()));
        }
        if node.clip_to_below() {
            entry.insert("clipToBelow".into(), serde_json::json!(true));
        }
        if !node.masks().is_empty() {
            warnings.push(format!(
                "layer {uuid} ({}): {} mask(s) not exported at interop v1",
                node.name(),
                node.masks().len()
            ));
        }
        if !entry.is_empty() {
            extra_layers.insert(uuid.to_string(), serde_json::Value::Object(entry));
        }
    }
    Ok(())
}
