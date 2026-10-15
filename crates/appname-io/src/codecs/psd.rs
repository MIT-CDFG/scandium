// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §8.5 "PSD import (clean-room) and the mapping table" — the
//! five-section byte-level structure, mode/depth tables, PackBits
//! [NORMATIVE] pseudocode, PSB widenings — plus Part 42 (§42.1
//! classification vocabulary + report rules, §42.2 matrix rows cited per
//! item, §42.4 report contract `contracts/schemas/compat-report.v1.json`);
//! DD ⑪. Corpus truth: `bogos/tests/interop/psd-corpus.yaml` (translated
//! into `tests/psd_import.rs` per the ORA pattern — the checked-in matrix
//! files do not exist in the spec repo; fixtures come from the in-crate
//! byte-level emitter the yaml itself sanctions for synthesized
//! constructs).
//!
//! CLEAN-ROOM PROVENANCE (preamble rule 2 / ES Part 47): this parser is
//! written against the byte-level structure NORMATIVELY fixed in
//! ENGINEERING-SPEC.md v5.5 §8.5 (offsets, tables, and decode pseudocode
//! reproduced there), whose own cited lineage is the publicly posted
//! "Adobe Photoshop File Formats Specification" (public web
//! documentation, 2022 revision) and independent clean-room documentation
//! — never Adobe source code, SDK headers, or decompiled material. No
//! Adobe artifact was consulted in writing this module.
//!
//! **Import v1 scope (CP-0.7 wave 1) — registered deviations, all typed
//! or reported, never silent (§42.1 rule 1):**
//! * All four §8.5 channel compressions are decoded: Raw, RLE, ZIP, and
//!   ZIP-with-prediction (flate2 per the zip record's `electedFeatures`,
//!   O-INT ruling 2026-08-13). A channel whose payload fails to decode
//!   (bad zlib stream, PackBits overrun) follows the composite-fallback
//!   rule with a `CompatItem` (CR-01 mechanic).
//! * All modes land in the interim RGBA working container (Gray/Indexed/
//!   CMYK/Lab/Multichannel/Duotone conversions are deterministic in-crate
//!   math, each reported per its §42.2 row; CMYK/Lab ride the CMM seam —
//!   real `Cmyk`/`Lab` documents arrive with §6.2 mode support).
//! * 32-bit channels quantize to 16-bit at the ingestion door
//!   (`Approximated`, reported — the ORA §8.4 precedent).
//! * Raster layer masks are RASTERIZED into the layer alpha (the §8.5
//!   mapping-table fallback; `MaskNode` application awaits the doc-model
//!   wave). Vector masks rasterize from the −3 channel when present, else
//!   drop reported.
//! * Effects (`lfx2`), unmapped adjustment keys, and unreadable fill
//!   descriptors take the §42.1 FallbackComposite mechanic (one fallback
//!   layer per cause, named "«source» (composite fallback)", top of
//!   stack, `bounds` recorded). Type (`TySh`) and smart-object
//!   (`SoLd`/`PlLd`) layers import their own stored raster (`Rasterized`).
//!   Groups: `lsct` walk → `group_path` carriage (reconstruction via
//!   `layer.group` pending), pass-through noted.
//! * The §42.4 report rides `Imported::compat_report` (schema-valid);
//!   `meta["compatReport"]` persistence is the document-meta wave.

use appname_core::BlendMode;

use crate::error::CodecError;
use crate::meta::Metadata;
use crate::opts::{DecodeOpts, EncodeOpts};
use crate::plane::RgbaPlane;
use crate::registry::{
    Codec, CodecCaps, CodecId, CompatAction, CompatReport, ExportImage, Imported, ImportedLayer,
    LayerLocks, LicenseStatus,
};

pub struct PsdCodec;

// ------------------------------------------------------------ cursor -------

struct Cur<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Cur<'a> {
    fn new(b: &'a [u8]) -> Cur<'a> {
        Cur { b, i: 0 }
    }

    fn trunc(&self) -> CodecError {
        CodecError::Truncated {
            offset: self.i as u64,
        }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], CodecError> {
        let end = self.i.checked_add(n).ok_or_else(|| self.trunc())?;
        let s = self.b.get(self.i..end).ok_or_else(|| self.trunc())?;
        self.i = end;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8, CodecError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, CodecError> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn i16(&mut self) -> Result<i16, CodecError> {
        Ok(i16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn u32(&mut self) -> Result<u32, CodecError> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn i32(&mut self) -> Result<i32, CodecError> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64, CodecError> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }

    /// u32 in PSD, u64 in PSB (§8.5 PSB differences).
    fn plen(&mut self, psb: bool) -> Result<u64, CodecError> {
        if psb {
            self.u64()
        } else {
            Ok(u64::from(self.u32()?))
        }
    }

    fn remaining(&self) -> usize {
        self.b.len() - self.i
    }
}

// ------------------------------------------------------------ header -------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Bitmap,
    Gray,
    Indexed,
    Rgb,
    Cmyk,
    Multichannel,
    Duotone,
    Lab,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Mode::Bitmap => "bitmap",
            Mode::Gray => "grayscale",
            Mode::Indexed => "indexed",
            Mode::Rgb => "rgb",
            Mode::Cmyk => "cmyk",
            Mode::Multichannel => "multichannel",
            Mode::Duotone => "duotone",
            Mode::Lab => "lab",
        }
    }

    /// Component channels (before alpha) in mode order.
    fn components(self) -> usize {
        match self {
            Mode::Bitmap | Mode::Gray | Mode::Duotone | Mode::Multichannel => 1,
            Mode::Indexed => 1,
            Mode::Rgb | Mode::Lab => 3,
            Mode::Cmyk => 4,
        }
    }
}

struct Header {
    psb: bool,
    channels: u16,
    height: u32,
    width: u32,
    depth: u16,
    mode: Mode,
}

fn corrupt(detail: impl std::fmt::Display) -> CodecError {
    CodecError::corrupt(format!("psd: {detail}"))
}

fn parse_header(cur: &mut Cur<'_>, opts: &DecodeOpts) -> Result<Header, CodecError> {
    if cur.take(4)? != b"8BPS" {
        return Err(CodecError::UnrecognizedFormat);
    }
    let version = cur.u16()?;
    let psb = match version {
        1 => false,
        2 => true,
        v => return Err(corrupt(format!("version {v}"))),
    };
    cur.take(6)?; // reserved: ignored on read (§8.5)
    let channels = cur.u16()?;
    if !(1..=56).contains(&channels) {
        return Err(corrupt(format!("channel count {channels}")));
    }
    let height = cur.u32()?;
    let width = cur.u32()?;
    let cap = if psb { 300_000 } else { 30_000 };
    if height == 0 || width == 0 || height > cap || width > cap {
        return Err(corrupt(format!(
            "dimensions {width}x{height} outside 1..={cap}"
        )));
    }
    opts.check_limits(width, height)?;
    let depth = cur.u16()?;
    let mode_v = cur.u16()?;
    let mode = match mode_v {
        0 => Mode::Bitmap,
        1 => Mode::Gray,
        2 => Mode::Indexed,
        3 => Mode::Rgb,
        4 => Mode::Cmyk,
        7 => Mode::Multichannel,
        8 => Mode::Duotone,
        9 => Mode::Lab,
        // Mode/depth-pairing violations carry the registry row
        // `error.codec.psd.header_invalid` via the stable token below
        // (§42.2-A / CR-05); other header failures stay on generic arms.
        v => {
            return Err(CodecError::Corrupt {
                detail: format!("psd.header_invalid: unknown color mode {v}"),
            })
        }
    };
    // H-09: any other (mode, depth) pair rejects.
    let ok = match mode {
        Mode::Bitmap => depth == 1,
        Mode::Gray | Mode::Rgb => matches!(depth, 8 | 16 | 32),
        Mode::Indexed => depth == 8,
        Mode::Cmyk | Mode::Multichannel | Mode::Duotone | Mode::Lab => matches!(depth, 8 | 16),
    };
    if !ok {
        return Err(CodecError::Corrupt {
            detail: format!(
                "psd.header_invalid: mode {}/depth {depth} pair (H-09)",
                mode.name()
            ),
        });
    }
    Ok(Header {
        psb,
        channels,
        height,
        width,
        depth,
        mode,
    })
}

// ------------------------------------------------------- blend key map -----

/// Appendix A key map: the 27 recognized keys, in §4.5 discriminant order
/// (B-01 lists them in exactly `BlendMode::ALL` order).
const BLEND_KEYS: [&[u8; 4]; 27] = [
    b"norm", b"diss", b"dark", b"mul ", b"idiv", b"lbrn", b"dkCl", b"lite", b"scrn", b"div ",
    b"lddg", b"lgCl", b"over", b"sLit", b"hLit", b"vLit", b"lLit", b"pLit", b"hMix", b"diff",
    b"smud", b"fsub", b"fdiv", b"hue ", b"sat ", b"colr", b"lum ",
];

pub fn blend_from_psd_key(key: &[u8; 4]) -> Option<BlendMode> {
    BLEND_KEYS
        .iter()
        .position(|k| *k == key)
        .map(|i| BlendMode::ALL[i])
}

// ----------------------------------------------------------- reporting -----

/// Extended §42.4 action set (the crate-level [`CompatAction`] carries the
/// shared three; PSD adds rasterized/fellBackToComposite as strings).
#[derive(Clone, Copy, PartialEq, Eq)]
enum PsdAction {
    Imported,
    Approximated,
    Rasterized,
    FellBackToComposite,
    Dropped,
}

impl PsdAction {
    fn wire(self) -> &'static str {
        match self {
            PsdAction::Imported => "imported",
            PsdAction::Approximated => "approximated",
            PsdAction::Rasterized => "rasterized",
            PsdAction::FellBackToComposite => "fellBackToComposite",
            PsdAction::Dropped => "dropped",
        }
    }

    /// §42.1: severity derives from action — `info` only for Imported.
    fn severity(self) -> &'static str {
        if self == PsdAction::Imported {
            "info"
        } else {
            "warning"
        }
    }

    fn crate_action(self) -> CompatAction {
        match self {
            PsdAction::Imported => CompatAction::Info,
            PsdAction::Approximated | PsdAction::Rasterized | PsdAction::FellBackToComposite => {
                CompatAction::Approximated
            }
            PsdAction::Dropped => CompatAction::Dropped,
        }
    }
}

/// A key-driven composite-fallback cause: (feature, §42.6 matrix row,
/// PSD-order rect (top, left, bottom, right), note).
type FallbackItem = (&'static str, &'static str, (i32, i32, i32, i32), String);

/// One report row: (layer name, layer id, feature, action, §42.6 matrix
/// row, bounds, note).
type ReportItem = (
    Option<String>,
    Option<u32>,
    String,
    PsdAction,
    Option<&'static str>,
    Option<(i32, i32, u32, u32)>,
    String,
);

struct Report {
    items: Vec<ReportItem>,
    skipped_resources: Vec<(u16, u64)>,
}

impl Report {
    fn new() -> Report {
        Report {
            items: Vec::new(),
            skipped_resources: Vec::new(),
        }
    }

    #[allow(clippy::too_many_arguments)] // mirrors the ReportItem tuple arity
    fn push(
        &mut self,
        layer: Option<&str>,
        lyid: Option<u32>,
        feature: impl Into<String>,
        action: PsdAction,
        row: Option<&'static str>,
        bounds: Option<(i32, i32, u32, u32)>,
        note: impl Into<String>,
    ) {
        self.items.push((
            layer.map(str::to_string),
            lyid,
            feature.into(),
            action,
            row,
            bounds,
            note.into(),
        ));
    }

    /// The §42.4 document (schema `appname.compatReport.v1`), plus the
    /// crate-level mirror items.
    fn finish(
        &self,
        hdr: &Header,
        outcome: &str,
        crate_report: &mut CompatReport,
    ) -> serde_json::Value {
        let mut counts = std::collections::BTreeMap::from([
            ("imported", 0u32),
            ("approximated", 0),
            ("rasterized", 0),
            ("fellBackToComposite", 0),
            ("dropped", 0),
        ]);
        let mut warnings = 0u32;
        let items: Vec<serde_json::Value> = self
            .items
            .iter()
            .map(|(layer, lyid, feature, action, row, bounds, note)| {
                *counts.get_mut(action.wire()).expect("closed set") += 1;
                if action.severity() == "warning" {
                    warnings += 1;
                }
                crate_report.push(feature.clone(), action.crate_action(), note.clone());
                let mut o = serde_json::Map::new();
                if let Some(l) = layer {
                    o.insert("layerPath".into(), serde_json::json!(l));
                }
                if let Some(id) = lyid {
                    o.insert("sourceLayerId".into(), serde_json::json!(id));
                }
                o.insert("feature".into(), serde_json::json!(feature));
                o.insert("action".into(), serde_json::json!(action.wire()));
                o.insert("severity".into(), serde_json::json!(action.severity()));
                if let Some(r) = row {
                    o.insert("matrixRow".into(), serde_json::json!(r));
                }
                if let Some((x, y, w, h)) = bounds {
                    o.insert(
                        "bounds".into(),
                        serde_json::json!({ "x": x, "y": y, "w": w, "h": h }),
                    );
                }
                o.insert("note".into(), serde_json::json!(note));
                serde_json::Value::Object(o)
            })
            .collect();
        let mut summary = serde_json::Map::new();
        for (k, v) in &counts {
            summary.insert((*k).into(), serde_json::json!(v));
        }
        summary.insert("warnings".into(), serde_json::json!(warnings));
        let skipped: Vec<serde_json::Value> = self
            .skipped_resources
            .iter()
            .map(|(id, bytes)| serde_json::json!({ "id": id, "bytes": bytes }))
            .collect();
        serde_json::json!({
            "schema": "appname.compatReport.v1",
            "producer": { "app": "Appname", "version": env!("CARGO_PKG_VERSION") },
            "source": {
                "path": "",
                "format": if hdr.psb { "psb" } else { "psd" },
                "colorMode": hdr.mode.name(),
                "depth": hdr.depth,
                "size": [hdr.width, hdr.height],
                "channels": hdr.channels,
            },
            "outcome": outcome,
            "items": items,
            "skippedResources": skipped,
            "summary": serde_json::Value::Object(summary),
        })
    }
}

/// §42.4: headless rejects still emit a report with `outcome:"rejected"`.
/// Built best-effort from whatever header bytes parse; callers (CLI
/// wiring) attach it beside the typed error.
pub fn rejected_report(bytes: &[u8], err: &CodecError) -> serde_json::Value {
    let mut cur = Cur::new(bytes);
    let hdr = parse_header(&mut cur, &DecodeOpts::default()).ok();
    let source = match &hdr {
        Some(h) => serde_json::json!({
            "path": "", "format": if h.psb { "psb" } else { "psd" },
            "colorMode": h.mode.name(), "depth": h.depth,
            "size": [h.width, h.height], "channels": h.channels,
        }),
        None => serde_json::json!({ "path": "", "format": "psd" }),
    };
    serde_json::json!({
        "schema": "appname.compatReport.v1",
        "producer": { "app": "Appname", "version": env!("CARGO_PKG_VERSION") },
        "source": source,
        "outcome": "rejected",
        "error": { "messageId": err.message_id(), "detail": err.to_string() },
        "items": [],
        "summary": { "imported": 0, "approximated": 0, "rasterized": 0,
                     "fellBackToComposite": 0, "dropped": 0, "warnings": 0 },
    })
}

// ------------------------------------------------------ channel decode -----

/// PackBits decode [NORMATIVE §8.5]: exactly `row_bytes` per row.
fn packbits_row(cur: &mut Cur<'_>, out: &mut Vec<u8>, row_bytes: usize) -> Result<(), CodecError> {
    let start = out.len();
    while out.len() - start < row_bytes {
        let n = cur.u8()? as i8;
        if n >= 0 {
            let lit = cur.take(n as usize + 1)?;
            if out.len() - start + lit.len() > row_bytes {
                return Err(corrupt("PackBits literal overruns the row"));
            }
            out.extend_from_slice(lit);
        } else if n >= -127 {
            let b = cur.u8()?;
            let count = 1 - n as isize;
            if out.len() - start + count as usize > row_bytes {
                return Err(corrupt("PackBits run overruns the row"));
            }
            out.extend(std::iter::repeat_n(b, count as usize));
        }
        // n == -128 → no-op
    }
    Ok(())
}

/// Inflate one §8.5 zlib stream (codes 2/3) to exactly `expect` bytes.
/// Bounded read: `expect` derives from the already-gated dimensions, and
/// the decoder never produces more than `expect + 1` bytes before the
/// length check rejects. Short, overlong, or malformed streams are
/// `Corrupt`.
fn inflate_zlib(data: &[u8], expect: usize) -> Result<Vec<u8>, CodecError> {
    use std::io::Read as _;
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(data)
        .take(expect as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|e| corrupt(format!("zlib stream: {e}")))?;
    if out.len() != expect {
        return Err(corrupt(format!(
            "zlib stream inflated to {} bytes, expected {expect}",
            out.len()
        )));
    }
    Ok(out)
}

/// Undo §8.5 code-3 per-row delta prediction [NORMATIVE depth specifics]:
/// 8-bit `b[i] += b[i−1]` on bytes; 16-bit `s[i] += s[i−1]` on big-endian
/// `u16`; 32-bit delta-then-deshuffle — byte-wise delta per row, then
/// de-interleave byte planes (output sample `j`, byte `k` =
/// `row[k·width + j]`). `rows.len()` is a whole number of rows.
fn unpredict_rows(rows: &mut [u8], width: usize, depth: u16) -> Result<(), CodecError> {
    match depth {
        8 => {
            for row in rows.chunks_exact_mut(width) {
                for i in 1..row.len() {
                    row[i] = row[i].wrapping_add(row[i - 1]);
                }
            }
        }
        16 => {
            for row in rows.chunks_exact_mut(2 * width) {
                let mut prev = 0u16;
                for i in 0..width {
                    let v = u16::from_be_bytes([row[2 * i], row[2 * i + 1]]).wrapping_add(prev);
                    row[2 * i..2 * i + 2].copy_from_slice(&v.to_be_bytes());
                    prev = v;
                }
            }
        }
        32 => {
            let mut shuffled = vec![0u8; 4 * width];
            for row in rows.chunks_exact_mut(4 * width) {
                for i in 1..row.len() {
                    row[i] = row[i].wrapping_add(row[i - 1]);
                }
                shuffled.copy_from_slice(row);
                for j in 0..width {
                    for k in 0..4 {
                        row[4 * j + k] = shuffled[k * width + j];
                    }
                }
            }
        }
        d => return Err(corrupt(format!("ZIP prediction at depth {d}"))),
    }
    Ok(())
}

/// Decode one channel's data block (compression code + payload) to raw
/// big-endian rows (`row_bytes = bytes_per_sample * width`).
fn decode_channel(
    data: &[u8],
    width: usize,
    height: usize,
    depth: u16,
    psb: bool,
) -> Result<Vec<u8>, CodecError> {
    let bps = match depth {
        1 => return Err(corrupt("1-bit layer channels (composite only)")),
        8 => 1usize,
        16 => 2,
        32 => 4,
        d => return Err(corrupt(format!("depth {d}"))),
    };
    let row_bytes = bps
        .checked_mul(width)
        .ok_or_else(|| corrupt("row size overflow"))?;
    let total = row_bytes
        .checked_mul(height)
        .ok_or_else(|| corrupt("channel size overflow"))?;
    let mut cur = Cur::new(data);
    match cur.u16()? {
        0 => {
            let raw = cur.take(total)?;
            Ok(raw.to_vec())
        }
        1 => {
            // Per-row packed byte counts for ALL rows first.
            let mut counts = Vec::with_capacity(height);
            for _ in 0..height {
                counts.push(if psb {
                    cur.u32()? as usize
                } else {
                    cur.u16()? as usize
                });
            }
            let mut out = Vec::with_capacity(total);
            for c in counts {
                let row = cur.take(c)?;
                let mut rc = Cur::new(row);
                packbits_row(&mut rc, &mut out, row_bytes)?;
                if rc.remaining() != 0 {
                    return Err(corrupt("PackBits row has trailing packed bytes"));
                }
            }
            Ok(out)
        }
        code @ (2 | 3) => {
            let rest = cur.take(cur.remaining())?;
            let mut rows = inflate_zlib(rest, total)?;
            if code == 3 {
                unpredict_rows(&mut rows, width, depth)?;
            }
            Ok(rows)
        }
        c => Err(corrupt(format!("compression code {c}"))),
    }
}

/// One decoded sample as u16 (8-bit widens ×257; 32-bit f32 clamps and
/// quantizes — the registered deviation).
fn sample_u16(bytes: &[u8], index: usize, depth: u16) -> u16 {
    match depth {
        8 => u16::from(bytes[index]) * 257,
        16 => u16::from_be_bytes([bytes[index * 2], bytes[index * 2 + 1]]),
        32 => {
            let f = f32::from_be_bytes([
                bytes[index * 4],
                bytes[index * 4 + 1],
                bytes[index * 4 + 2],
                bytes[index * 4 + 3],
            ]);
            (f.clamp(0.0, 1.0) * 65535.0).round() as u16
        }
        _ => 0,
    }
}

// ------------------------------------------------------ mode conversion ----

/// naive CMYK(ink, post-inversion)→RGB — CMM seam, reported per H-05.
fn cmyk_to_rgb(c: f32, m: f32, y: f32, k: f32) -> [f32; 3] {
    [
        (1.0 - c) * (1.0 - k),
        (1.0 - m) * (1.0 - k),
        (1.0 - y) * (1.0 - k),
    ]
}

/// CIELAB (D50) → sRGB (D65, Bradford) — deterministic in-crate math,
/// CMM seam, reported per H-08.
fn lab_to_srgb(l: f32, a: f32, b: f32) -> [f32; 3] {
    let fy = (l + 16.0) / 116.0;
    let fx = fy + a / 500.0;
    let fz = fy - b / 200.0;
    let f_inv = |t: f32| {
        if t > 6.0 / 29.0 {
            t * t * t
        } else {
            3.0 * (6.0f32 / 29.0).powi(2) * (t - 4.0 / 29.0)
        }
    };
    // D50 white.
    let (xn, yn, zn) = (0.964_22_f32, 1.0_f32, 0.825_21_f32);
    let (x, y, z) = (xn * f_inv(fx), yn * f_inv(fy), zn * f_inv(fz));
    // Bradford-adapted D50→D65, composed with the sRGB matrix
    // (linear-light output).
    let rl = 3.134_136 * x - 1.617_386 * y - 0.490_662 * z;
    let gl = -0.978_795 * x + 1.916_254 * y + 0.033_442 * z;
    let bl = 0.071_955 * x - 0.228_977 * y + 1.405_386 * z;
    [rl.clamp(0.0, 1.0), gl.clamp(0.0, 1.0), bl.clamp(0.0, 1.0)]
}

// ---------------------------------------------------------- resources ------

#[derive(Default)]
struct Resources {
    icc: Option<Vec<u8>>,
    exif: Option<Vec<u8>>,
    xmp: Option<Vec<u8>>,
    transparency_index: Option<u16>,
    resolution_ppi: Option<f64>,
}

fn parse_resources(cur: &mut Cur<'_>, report: &mut Report) -> Result<Resources, CodecError> {
    let total = cur.u32()? as usize;
    let block = Cur::new(cur.take(total)?);
    let mut cur = block;
    let mut res = Resources::default();
    while cur.remaining() >= 4 {
        if cur.take(4)? != b"8BIM" {
            return Err(corrupt("image resource signature"));
        }
        let id = cur.u16()?;
        // Pascal name, padded to even TOTAL (length byte + bytes).
        let name_len = cur.u8()? as usize;
        let padded = if (name_len + 1) % 2 == 1 {
            name_len + 1
        } else {
            name_len
        };
        cur.take(padded)?;
        let size = cur.u32()? as usize;
        let data = cur.take(size)?;
        if size % 2 == 1 {
            cur.take(1)?; // data padded to even
        }
        match id {
            1005 => {
                // ResolutionInfo: hRes as 16.16 fixed (px/inch when unit=1).
                if data.len() >= 16 {
                    let fixed = u32::from_be_bytes(data[0..4].try_into().unwrap());
                    let h_unit = u16::from_be_bytes(data[4..6].try_into().unwrap());
                    let ppi = f64::from(fixed) / 65536.0;
                    // Unit 1 = px/inch; 2 = px/cm → convert.
                    let ppi = if h_unit == 2 { ppi * 2.54 } else { ppi };
                    if ppi.is_finite() && ppi > 0.0 {
                        res.resolution_ppi = Some(ppi);
                        report.push(
                            None,
                            None,
                            "resource.resolution",
                            PsdAction::Imported,
                            Some("RS-01"),
                            None,
                            format!(
                                "resolution {ppi:.2} ppi (applied at CLI wiring — doc geometry)"
                            ),
                        );
                    }
                } else {
                    report.push(
                        None,
                        None,
                        "resource.resolution",
                        PsdAction::Dropped,
                        Some("RS-01"),
                        None,
                        "short ResolutionInfo block dropped",
                    );
                }
            }
            1036 => report.push(
                None,
                None,
                "resource.thumbnail",
                PsdAction::Dropped,
                Some("RS-04"),
                None,
                "thumbnail skipped — previews regenerate (§8.5)",
            ),
            1039 => res.icc = Some(data.to_vec()),
            1047 => {
                if data.len() >= 2 {
                    res.transparency_index =
                        Some(u16::from_be_bytes(data[0..2].try_into().unwrap()));
                }
            }
            1058 => res.exif = Some(data.to_vec()),
            1060 => res.xmp = Some(data.to_vec()),
            1064 => {
                // Pixel aspect ratio: u32 version + f64 ratio.
                if data.len() >= 12 {
                    let ratio = f64::from_be_bytes(data[4..12].try_into().unwrap());
                    if (ratio - 1.0).abs() > 1e-9 {
                        report.push(
                            None,
                            None,
                            "resource.pixel_aspect_ratio",
                            PsdAction::Approximated,
                            Some("RS-07"),
                            None,
                            format!(
                                "non-square pixel ratio {ratio} — Appname is square-pixel (§8.5)"
                            ),
                        );
                    }
                }
            }
            other => report.skipped_resources.push((other, size as u64)),
        }
    }
    Ok(res)
}

// ----------------------------------------------------- layer records -------

struct MaskData {
    rect: (i32, i32, i32, i32), // top,left,bottom,right
    default_color: u8,
    disabled: bool,
}

struct LayerRec {
    rect: (i32, i32, i32, i32), // top,left,bottom,right
    channels: Vec<(i16, u64)>,
    blend: BlendMode,
    unknown_blend: Option<String>,
    opacity: f32,
    clipping: bool,
    hidden: bool,
    lock_transparency_flag: bool,
    name: String,
    lyid: Option<u32>,
    lsct: Option<(u32, Option<[u8; 4]>)>,
    fill_opacity: f32,
    lspf: u32,
    mask: Option<MaskData>,
    blending_nondefault: bool,
    /// Fallback-triggering / rasterize-triggering info keys seen.
    keys: Vec<[u8; 4]>,
}

/// Additional-info big-key subset whose length widens to u64 in PSB.
const PSB_BIG_KEYS: [&[u8; 4]; 13] = [
    b"Lr16", b"Lr32", b"LMsk", b"Layr", b"Mt16", b"Mt32", b"Mtrn", b"Alph", b"FMsk", b"lnk2",
    b"FEid", b"FXid", b"PxSD",
];

const ADJUSTMENT_KEYS: [&[u8; 4]; 15] = [
    b"levl", b"curv", b"brit", b"blnc", b"hue2", b"mixr", b"grdm", b"phfl", b"blwh", b"expA",
    b"vibA", b"thrs", b"post", b"nvrt", b"selc",
];

fn parse_layer_record(cur: &mut Cur<'_>, psb: bool) -> Result<LayerRec, CodecError> {
    let rect = (cur.i32()?, cur.i32()?, cur.i32()?, cur.i32()?);
    let nch = cur.u16()? as usize;
    if nch > 56 {
        return Err(corrupt(format!("layer channel count {nch}")));
    }
    let mut channels = Vec::with_capacity(nch);
    for _ in 0..nch {
        let id = cur.i16()?;
        let len = cur.plen(psb)?;
        channels.push((id, len));
    }
    if cur.take(4)? != b"8BIM" {
        return Err(corrupt("layer blend signature"));
    }
    let key: [u8; 4] = cur.take(4)?.try_into().unwrap();
    let (blend, unknown_blend) = match blend_from_psd_key(&key) {
        Some(b) => (b, None),
        None => (
            BlendMode::Normal,
            Some(String::from_utf8_lossy(&key).into_owned()),
        ),
    };
    let opacity = f32::from(cur.u8()?) / 255.0;
    let clipping = cur.u8()? == 1;
    let flags = cur.u8()?;
    cur.u8()?; // filler
    let extra_len = cur.u32()? as usize;
    let mut extra = Cur::new(cur.take(extra_len)?);

    // Mask data.
    let mask_len = extra.u32()? as usize;
    let mut mask = None;
    if mask_len > 0 {
        let mut mc = Cur::new(extra.take(mask_len)?);
        if mask_len >= 20 {
            let rect = (mc.i32()?, mc.i32()?, mc.i32()?, mc.i32()?);
            let default_color = mc.u8()?;
            let mflags = mc.u8()?;
            mask = Some(MaskData {
                rect,
                default_color,
                disabled: mflags & 0b10 != 0,
            });
        }
    }
    // Blending ranges: default composite+per-channel pairs are
    // [0,0,255,255]; anything else records the §8.5 CompatItem.
    let ranges_len = extra.u32()? as usize;
    let ranges = extra.take(ranges_len)?;
    let blending_nondefault =
        !ranges.is_empty() && !ranges.chunks_exact(4).all(|c| c == [0, 0, 255, 255]);

    // Pascal name, padded so the name field totals a multiple of 4.
    let pn_len = extra.u8()? as usize;
    let name_bytes = extra.take(pn_len)?;
    let mut name = String::from_utf8_lossy(name_bytes).into_owned();
    let pad = (4 - ((pn_len + 1) % 4)) % 4;
    extra.take(pad)?;

    let mut rec = LayerRec {
        rect,
        channels,
        blend,
        unknown_blend,
        opacity,
        clipping,
        hidden: flags & 0b10 != 0,
        lock_transparency_flag: flags & 0b1 != 0,
        name: String::new(),
        lyid: None,
        lsct: None,
        fill_opacity: 1.0,
        lspf: 0,
        mask,
        blending_nondefault,
        keys: Vec::new(),
    };

    // Additional-info keys until the record's extra block ends.
    while extra.remaining() >= 12 {
        let sig = extra.take(4)?;
        if sig != b"8BIM" && sig != b"8B64" {
            return Err(corrupt("additional-info signature"));
        }
        let key: [u8; 4] = extra.take(4)?.try_into().unwrap();
        let big = psb && PSB_BIG_KEYS.contains(&&key);
        let len = if big {
            extra.u64()?
        } else {
            u64::from(extra.u32()?)
        } as usize;
        let padded = len + (len % 2); // blocks pad to even
        let data = extra.take(padded.min(extra.remaining()).max(len))?;
        let data = &data[..len];
        match &key {
            b"lyid" if len >= 4 => {
                rec.lyid = Some(u32::from_be_bytes(data[0..4].try_into().unwrap()));
            }
            b"lsct" if len >= 4 => {
                let ty = u32::from_be_bytes(data[0..4].try_into().unwrap());
                let bkey = if len >= 12 && &data[4..8] == b"8BIM" {
                    Some(data[8..12].try_into().unwrap())
                } else {
                    None
                };
                rec.lsct = Some((ty, bkey));
            }
            b"luni" if len >= 4 => {
                let n = u32::from_be_bytes(data[0..4].try_into().unwrap()) as usize;
                let units: Vec<u16> = data[4..]
                    .chunks_exact(2)
                    .take(n)
                    .map(|c| u16::from_be_bytes([c[0], c[1]]))
                    .collect();
                name = String::from_utf16_lossy(&units);
            }
            b"lspf" if len >= 4 => {
                rec.lspf = u32::from_be_bytes(data[0..4].try_into().unwrap());
            }
            b"iOpa" if len >= 1 => {
                rec.fill_opacity = f32::from(data[0]) / 255.0;
            }
            _ => rec.keys.push(key),
        }
    }
    rec.name = name;
    Ok(rec)
}

// ------------------------------------------------------ plane assembly -----

/// Assemble interleaved RGBA (as straight sRGB-encoded u16 samples) from
/// planar per-channel data under the header's mode.
#[allow(clippy::too_many_arguments)]
fn assemble_rgba16(
    mode: Mode,
    depth: u16,
    w: usize,
    h: usize,
    comp: &[Vec<u8>],        // component planes in mode order
    alpha: Option<&Vec<u8>>, // −1 / merged alpha plane
    palette: Option<&[u8]>,  // indexed: 768 bytes
    transparency_index: Option<u16>,
) -> Result<Vec<u16>, CodecError> {
    use appname_core::io::srgb::srgb_encode;
    let n = w.checked_mul(h).ok_or_else(|| corrupt("plane overflow"))?;
    // Bounds discipline BEFORE any indexing (the fuzz-floor bar): a
    // mutated channel count / short plane must answer typed, never panic.
    let bps = usize::from(depth.max(8)) / 8;
    let need = n
        .checked_mul(bps)
        .ok_or_else(|| corrupt("plane overflow"))?;
    if comp.len() < mode.components() {
        return Err(corrupt(format!(
            "{} component plane(s) for mode {}",
            comp.len(),
            mode.name()
        )));
    }
    for (ci, p) in comp.iter().take(mode.components()).enumerate() {
        if p.len() < need {
            return Err(corrupt(format!("short component plane {ci}")));
        }
    }
    if let Some(p) = alpha {
        if p.len() < need {
            return Err(corrupt("short alpha plane"));
        }
    }
    if mode == Mode::Indexed && palette.is_none_or(|p| p.len() < 768) {
        return Err(corrupt("indexed file without a full palette"));
    }
    let mut out = vec![0u16; n * 4];
    for i in 0..n {
        let a = alpha.map_or(u16::MAX, |p| sample_u16(p, i, depth));
        let (r, g, b, a) = match mode {
            Mode::Gray | Mode::Duotone | Mode::Multichannel => {
                let v = sample_u16(&comp[0], i, depth);
                (v, v, v, a)
            }
            Mode::Rgb => (
                sample_u16(&comp[0], i, depth),
                sample_u16(&comp[1], i, depth),
                sample_u16(&comp[2], i, depth),
                a,
            ),
            Mode::Indexed => {
                let idx = comp[0][i] as usize;
                let pal = palette.ok_or_else(|| corrupt("indexed file without palette"))?;
                let r = u16::from(pal[idx]) * 257;
                let g = u16::from(pal[256 + idx]) * 257;
                let b = u16::from(pal[512 + idx]) * 257;
                let a = if transparency_index == Some(idx as u16) {
                    0
                } else {
                    a
                };
                (r, g, b, a)
            }
            Mode::Cmyk => {
                // §8.5: stored inverted; v → 1 − v, then the naive CMM-seam
                // conversion.
                let f = |ch: usize| 1.0 - f32::from(sample_u16(&comp[ch], i, depth)) / 65535.0;
                let [r, g, b] = cmyk_to_rgb(f(0), f(1), f(2), f(3));
                let q = |v: f32| (v * 65535.0).round() as u16;
                (q(r), q(g), q(b), a)
            }
            Mode::Lab => {
                let l = f32::from(sample_u16(&comp[0], i, depth)) / 65535.0 * 100.0;
                let ab = |ch: usize| f32::from(sample_u16(&comp[ch], i, depth)) / 257.0 - 128.0;
                let [rl, gl, bl] = lab_to_srgb(l, ab(1), ab(2));
                let q = |v: f32| (srgb_encode(v) * 65535.0).round() as u16;
                (q(rl), q(gl), q(bl), a)
            }
            Mode::Bitmap => return Err(corrupt("bitmap layers (composite only)")),
        };
        out[i * 4] = r;
        out[i * 4 + 1] = g;
        out[i * 4 + 2] = b;
        out[i * 4 + 3] = a;
    }
    Ok(out)
}

/// Narrow u16 samples to a plane at the source depth class (8-bit sources
/// stay 8-bit; 16/32-bit stay 16-bit — module-doc deviation for 32).
fn plane_from_rgba16(w: u32, h: u32, px: Vec<u16>, depth: u16) -> Result<RgbaPlane, CodecError> {
    if depth == 8 || depth == 1 {
        let bytes: Vec<u8> = px.iter().map(|v| (v / 257) as u8).collect();
        RgbaPlane::new8(w, h, bytes)
    } else {
        RgbaPlane::new16(w, h, px)
    }
}

// ---------------------------------------------------- merged image data ----

/// Decode the final section: `u16` compression code, then all composite
/// channels planar in mode order (§8.5).
fn decode_merged(cur: &mut Cur<'_>, hdr: &Header) -> Result<Vec<Vec<u8>>, CodecError> {
    let (w, h) = (hdr.width as usize, hdr.height as usize);
    let nch = hdr.channels as usize;
    let row_bytes = match hdr.depth {
        1 => w.div_ceil(8),
        8 => w,
        16 => 2 * w,
        32 => 4 * w,
        d => return Err(corrupt(format!("depth {d}"))),
    };
    let code = cur.u16()?;
    let mut planes = Vec::with_capacity(nch);
    match code {
        0 => {
            for _ in 0..nch {
                planes.push(cur.take(row_bytes * h)?.to_vec());
            }
        }
        1 => {
            // One count table for ALL rows of ALL channels, then rows.
            let mut counts = Vec::with_capacity(nch * h);
            for _ in 0..nch * h {
                counts.push(if hdr.psb {
                    cur.u32()? as usize
                } else {
                    cur.u16()? as usize
                });
            }
            for ch in 0..nch {
                let mut plane = Vec::with_capacity(row_bytes * h);
                for r in 0..h {
                    let packed = cur.take(counts[ch * h + r])?;
                    let mut rc = Cur::new(packed);
                    packbits_row(&mut rc, &mut plane, row_bytes)?;
                }
                planes.push(plane);
            }
        }
        code @ (2 | 3) => {
            // One zlib stream of all composite channels, planar in mode
            // order (§8.5 merged-data section); prediction per row per
            // plane. Prediction is undefined for 1-bit rows (§8.5 depth
            // specifics cover 8/16/32 only) — `unpredict_rows` rejects it.
            let total = row_bytes
                .checked_mul(h)
                .and_then(|v| v.checked_mul(nch))
                .ok_or_else(|| corrupt("merged size overflow"))?;
            let rest = cur.take(cur.remaining())?;
            let all = inflate_zlib(rest, total)?;
            for ch in 0..nch {
                let mut plane = all[ch * row_bytes * h..(ch + 1) * row_bytes * h].to_vec();
                if code == 3 {
                    unpredict_rows(&mut plane, w, hdr.depth)?;
                }
                planes.push(plane);
            }
        }
        c => return Err(corrupt(format!("merged compression code {c}"))),
    }
    // Bitmap: unpack 1-bit rows (MSB-first, 1 = black) to 8-bit gray.
    if hdr.depth == 1 {
        for plane in &mut planes {
            let mut unpacked = Vec::with_capacity(w * h);
            for row in plane.chunks_exact(row_bytes) {
                for x in 0..w {
                    let bit = (row[x / 8] >> (7 - (x % 8))) & 1;
                    unpacked.push(if bit == 1 { 0 } else { 255 });
                }
            }
            *plane = std::mem::take(&mut unpacked);
        }
    }
    Ok(planes)
}

// ------------------------------------------------------------- import ------

struct ParsedLayers {
    merged_has_alpha: bool,
    records: Vec<LayerRec>,
    /// Per record, per channel: the raw data block (compression + payload).
    channel_data: Vec<Vec<Vec<u8>>>,
}

fn parse_layer_info(cur: &mut Cur<'_>, psb: bool) -> Result<Option<ParsedLayers>, CodecError> {
    let count = cur.i16()?;
    if count == 0 {
        return Ok(None);
    }
    let merged_has_alpha = count < 0;
    let n = count.unsigned_abs() as usize;
    if n > 4096 {
        return Err(corrupt(format!("layer count {n}")));
    }
    let mut records = Vec::with_capacity(n);
    for _ in 0..n {
        records.push(parse_layer_record(cur, psb)?);
    }
    let mut channel_data = Vec::with_capacity(n);
    for rec in &records {
        let mut per = Vec::with_capacity(rec.channels.len());
        for (_, len) in &rec.channels {
            let len = usize::try_from(*len).map_err(|_| corrupt("channel length"))?;
            per.push(cur.take(len)?.to_vec());
        }
        channel_data.push(per);
    }
    Ok(Some(ParsedLayers {
        merged_has_alpha,
        records,
        channel_data,
    }))
}

/// Parse the whole layer-and-mask section (already sliced). 16/32-bit
/// files carry their real records under global `Lr16`/`Lr32` (§8.5).
fn parse_layer_section(cur: &mut Cur<'_>, psb: bool) -> Result<Option<ParsedLayers>, CodecError> {
    let info_len = cur.plen(psb)? as usize;
    let mut info = Cur::new(cur.take(info_len)?);
    let mut parsed = if info_len >= 2 {
        parse_layer_info(&mut info, psb)?
    } else {
        None
    };
    // Global layer-mask info: u32 length + payload, skipped.
    if cur.remaining() >= 4 {
        let gl = cur.u32()? as usize;
        cur.take(gl.min(cur.remaining()))?;
    }
    // Global additional info: Lr16/Lr32 carry the real records.
    while cur.remaining() >= 12 {
        let sig = cur.take(4)?;
        if sig != b"8BIM" && sig != b"8B64" {
            break; // padding / unknown tail — not fatal at the global tier
        }
        let key: [u8; 4] = cur.take(4)?.try_into().unwrap();
        let big = psb && PSB_BIG_KEYS.contains(&&key);
        let len = if big {
            cur.u64()?
        } else {
            u64::from(cur.u32()?)
        } as usize;
        let padded = len + (len % 2);
        let data = cur.take(padded.min(cur.remaining()).max(len))?;
        let data = &data[..len];
        if (&key == b"Lr16" || &key == b"Lr32") && parsed.is_none() {
            let mut lr = Cur::new(data);
            parsed = parse_layer_info(&mut lr, psb)?;
        }
    }
    Ok(parsed)
}

/// Crop the merged composite (interleaved RGBA u16 at canvas size) to a
/// region, producing a fallback plane.
fn crop_composite(
    comp: &[u16],
    cw: usize,
    ch: usize,
    region: (i32, i32, i32, i32), // top,left,bottom,right
) -> Option<(RgbaPlane, (i32, i32))> {
    let top = region.0.clamp(0, ch as i32);
    let left = region.1.clamp(0, cw as i32);
    let bottom = region.2.clamp(0, ch as i32);
    let right = region.3.clamp(0, cw as i32);
    let (w, h) = ((right - left) as usize, (bottom - top) as usize);
    if w == 0 || h == 0 {
        return None;
    }
    let mut px = Vec::with_capacity(w * h * 4);
    for y in top..bottom {
        for x in left..right {
            let i = (y as usize * cw + x as usize) * 4;
            px.extend_from_slice(&comp[i..i + 4]);
        }
    }
    let plane = RgbaPlane::new16(w as u32, h as u32, px).ok()?;
    Some((plane, (left, top)))
}

/// The five-section import (§8.5). Returns the Imported plus the §42.4
/// report attached on `Imported::compat_report`.
pub fn import_psd(bytes: &[u8], opts: &DecodeOpts) -> Result<Imported, CodecError> {
    opts.check_interim_unsupported()?;
    let mut report = Report::new();
    let mut cur = Cur::new(bytes);
    let hdr = parse_header(&mut cur, opts)?;
    let (cw, chh) = (hdr.width as usize, hdr.height as usize);

    // Section 2: color-mode data.
    let cmd_len = cur.u32()? as usize;
    let cmd = cur.take(cmd_len)?;
    let palette: Option<&[u8]> = match hdr.mode {
        Mode::Indexed => {
            if cmd.len() < 768 {
                return Err(corrupt("indexed file without a 768-byte palette"));
            }
            report.push(
                None,
                None,
                "header.indexed",
                PsdAction::Approximated,
                Some("H-03"),
                None,
                "mode converted (Rgb via the 768-byte palette; palette identity not retained)",
            );
            Some(&cmd[..768])
        }
        Mode::Duotone => {
            report.push(
                None, None, "header.duotone", PsdAction::Approximated, Some("H-07"), None,
                "rendered as grayscale; duotone color data not carried at import v1 (registered deviation)",
            );
            None
        }
        _ => None,
    };
    match hdr.mode {
        Mode::Bitmap => report.push(
            None, None, "header.bitmap", PsdAction::Imported, Some("H-01"), None,
            "1-bit → 8-bit container (rows unpacked MSB-first, 1 = black; composite only)",
        ),
        Mode::Cmyk => report.push(
            None, None, "header.cmyk", PsdAction::Approximated, Some("H-05"), None,
            "components inverted per §8.5 then converted to the interim RGBA container without the CMM (§5 seam; a Cmyk document arrives with §6.2)",
        ),
        Mode::Lab => report.push(
            None, None, "header.lab", PsdAction::Approximated, Some("H-08"), None,
            "Lab converted to the interim RGBA container (D50→D65 Bradford, in-crate math; a Lab document arrives with §6.2)",
        ),
        Mode::Multichannel => report.push(
            None, None, "header.multichannel", PsdAction::Approximated, Some("H-06"), None,
            "first channel imported as gray; remaining channels dropped at v1 (§14 ChannelSet pending)",
        ),
        Mode::Gray | Mode::Duotone
            if hdr.mode == Mode::Gray => {
                report.push(
                    None, None, "header.gray", PsdAction::Imported, Some("H-02"), None,
                    "grayscale replicated into the interim RGBA container",
                );
            }
        _ => {}
    }
    if hdr.depth == 32 {
        report.push(
            None, None, "depth.32", PsdAction::Approximated, Some("H-02"), None,
            "32-bit channels quantize to 16-bit at the interim ingestion door (registered deviation)",
        );
    }

    // Section 3: image resources.
    let res = parse_resources(&mut cur, &mut report)?;

    // Section 4: layer-and-mask info. Errors INSIDE the section degrade to
    // the CR-03 flattened fallback; a broken section LENGTH is fatal.
    let section_len = cur.plen(hdr.psb)? as usize;
    let section = cur.take(section_len)?;
    let parsed = if section_len == 0 {
        None
    } else {
        let mut sc = Cur::new(section);
        match parse_layer_section(&mut sc, hdr.psb) {
            Ok(p) => p,
            Err(e) => {
                report.push(
                    None,
                    None,
                    "layers.section",
                    PsdAction::FellBackToComposite,
                    Some("CR-03"),
                    Some((0, 0, hdr.width, hdr.height)),
                    format!("layer-and-mask section unusable ({e}); flattened single-layer import"),
                );
                None
            }
        }
    };

    // Section 5: merged image data → the composite (primary surface and
    // the FallbackComposite source).
    let merged_planes = decode_merged(&mut cur, &hdr)?;
    let comps = hdr.mode.components();
    let merged_alpha_plane =
        if hdr.channels as usize > comps && parsed.as_ref().is_some_and(|p| p.merged_has_alpha) {
            Some(&merged_planes[comps])
        } else {
            None
        };
    if hdr.channels as usize > comps {
        let extras = hdr.channels as usize - comps - usize::from(merged_alpha_plane.is_some());
        if extras > 0 {
            report.push(
                None,
                None,
                "channels.extra",
                PsdAction::Dropped,
                Some("CH-04"),
                None,
                format!(
                    "{extras} extra alpha/spot channel(s) dropped at v1 (§14 ChannelSet pending)"
                ),
            );
        }
    }
    let composite16 = assemble_rgba16(
        if hdr.mode == Mode::Bitmap {
            Mode::Gray
        } else {
            hdr.mode
        },
        if hdr.depth == 1 { 8 } else { hdr.depth },
        cw,
        chh,
        &merged_planes[..comps.min(merged_planes.len())],
        merged_alpha_plane,
        palette,
        res.transparency_index,
    )?;
    let composite_plane = plane_from_rgba16(hdr.width, hdr.height, composite16.clone(), hdr.depth)?;

    // Build layers.
    let mut layers: Vec<ImportedLayer> = Vec::new();
    let mut fallbacks: Vec<ImportedLayer> = Vec::new();
    if let Some(p) = &parsed {
        // Group walk (bottom-to-top): type 3 opens, type 1/2 closes+names.
        let mut path: Vec<String> = Vec::new();
        let mut balanced = true;
        for rec in &p.records {
            match rec.lsct {
                Some((3, _)) => path.push(String::new()),
                Some((1 | 2, _)) if path.pop().is_none() => {
                    balanced = false;
                }
                _ => {}
            }
        }
        if !path.is_empty() {
            balanced = false;
        }
        if !balanced {
            report.push(
                None,
                None,
                "layers.groups",
                PsdAction::Approximated,
                Some("G-02"),
                None,
                "unbalanced lsct dividers; layers import flat (§8.5 flatten-group fallback)",
            );
        }

        let mut group_stack: Vec<String> = Vec::new();
        for (ri, rec) in p.records.iter().enumerate() {
            let path_str = if group_stack.is_empty() || !balanced {
                rec.name.clone()
            } else {
                format!("{}/{}", group_stack.join("/"), rec.name)
            };
            // Dividers shape the walk, they are not pixel layers.
            match rec.lsct {
                Some((3, _)) => {
                    if balanced {
                        group_stack.push(String::new());
                    }
                    continue;
                }
                Some((1 | 2, bkey)) => {
                    if balanced {
                        group_stack.pop();
                        group_stack.push(rec.name.clone());
                        // The just-closed group's name applies to layers
                        // already collected? No: records are bottom-to-top,
                        // the folder record CLOSES the group from above —
                        // we assign eagerly below instead.
                        group_stack.pop();
                    }
                    report.push(
                        Some(&rec.name),
                        rec.lyid,
                        "group",
                        PsdAction::Imported,
                        Some(if matches!(bkey, Some(k) if &k == b"pass") {
                            "B-02"
                        } else {
                            "G-01"
                        }),
                        None,
                        format!(
                            "group carried on group_path; reconstruction via layer.group pending{}",
                            if matches!(bkey, Some(k) if &k == b"pass") {
                                "; pass-through"
                            } else {
                                ""
                            }
                        ),
                    );
                    continue;
                }
                _ => {}
            }
            if let Some(unknown) = &rec.unknown_blend {
                report.push(
                    Some(&path_str),
                    rec.lyid,
                    "blend.unknown_key",
                    PsdAction::Approximated,
                    Some("B-04"),
                    None,
                    format!("unknown blend key `{unknown}` mapped to Normal"),
                );
            }
            if rec.blending_nondefault {
                report.push(
                    Some(&path_str),
                    rec.lyid,
                    "blend.ranges",
                    PsdAction::Approximated,
                    None,
                    None,
                    "non-default blending ranges skipped (§8.5)",
                );
            }

            let (top, left, bottom, right) = rec.rect;
            let (lw, lh) = (
                (right - left).max(0) as usize,
                (bottom - top).max(0) as usize,
            );
            if lw > 0 && lh > 0 {
                opts.check_limits(lw as u32, lh as u32)?;
            }

            // Decode this record's channels.
            let mut comp_planes: Vec<Vec<u8>> = Vec::new();
            let mut alpha_plane: Option<Vec<u8>> = None;
            let mut mask_plane: Option<Vec<u8>> = None;
            let mut real_mask_plane: Option<Vec<u8>> = None;
            let mut failed: Option<CodecError> = None;
            if lw > 0 && lh > 0 {
                let mut comp_by_id: Vec<(i16, Vec<u8>)> = Vec::new();
                for (ci, (cid, _)) in rec.channels.iter().enumerate() {
                    let data = &p.channel_data[ri][ci];
                    // Mask channels decode over the MASK rect.
                    let (dw, dh) = if *cid == -2 || *cid == -3 {
                        match &rec.mask {
                            Some(m) => (
                                (m.rect.3 - m.rect.1).max(0) as usize,
                                (m.rect.2 - m.rect.0).max(0) as usize,
                            ),
                            None => (0, 0),
                        }
                    } else {
                        (lw, lh)
                    };
                    if dw == 0 || dh == 0 {
                        continue;
                    }
                    match decode_channel(data, dw, dh, hdr.depth, hdr.psb) {
                        Ok(plane) => match *cid {
                            -1 => alpha_plane = Some(plane),
                            -2 => mask_plane = Some(plane),
                            -3 => real_mask_plane = Some(plane),
                            id if id >= 0 => comp_by_id.push((id, plane)),
                            _ => {}
                        },
                        Err(e) => {
                            failed = Some(e);
                            break;
                        }
                    }
                }
                comp_by_id.sort_by_key(|(id, _)| *id);
                comp_planes = comp_by_id.into_iter().map(|(_, p)| p).collect();
            }

            if let Some(e) = failed {
                // CR-01: the layer region follows the composite-fallback
                // rule. (ZIP channels decode for real since the deflate
                // election — a failed zlib stream is just Corrupt now.)
                let feature = "channel.corrupt";
                if let Some((plane, (fx, fy))) = crop_composite(&composite16, cw, chh, rec.rect) {
                    let (png, buffer) = plane.to_png_and_buffer(opts.working_profile)?;
                    fallbacks.push(ImportedLayer {
                        name: format!("{} (composite fallback)", rec.name),
                        buffer,
                        ingress_png: png,
                        offset: appname_core::IVec2::new(fx, fy),
                        opacity: 1.0,
                        fill_opacity: 1.0,
                        blend: BlendMode::Normal,
                        visible: true,
                        clip_to_below: false,
                        locks: LayerLocks::default(),
                        group_path: Vec::new(),
                        icc: None,
                    });
                }
                report.push(
                    Some(&path_str),
                    rec.lyid,
                    feature,
                    PsdAction::FellBackToComposite,
                    Some("CR-01"),
                    Some((
                        rec.rect.1,
                        rec.rect.0,
                        (rec.rect.3 - rec.rect.1).max(0) as u32,
                        (rec.rect.2 - rec.rect.0).max(0) as u32,
                    )),
                    format!(
                        "channel decode failed ({e}); region reproduced from the merged composite"
                    ),
                );
                continue;
            }

            // Key-driven items and fallback causes.
            let mut extra_fallback: Option<FallbackItem> = None;
            for key in &rec.keys {
                match key {
                    b"lfx2" => {
                        // FX at v1: appearance preserved via the §42.1
                        // mechanic (inflated rect).
                        let (t, l, b, r) = rec.rect;
                        let dw = (r - l).max(0) as f64;
                        let dh = (b - t).max(0) as f64;
                        let inflate = ((dw * dw + dh * dh).sqrt() * 0.5).ceil() as i32;
                        extra_fallback = Some((
                            "effect.lfx2", "FX-15",
                            (t - inflate, l - inflate, b + inflate, r + inflate),
                            "layer effects not mapped at import v1; appearance preserved from the composite over the inflated rect".into(),
                        ));
                    }
                    b"SoCo" | b"GdFl" | b"PtFl" => {
                        extra_fallback = Some((
                            "fill.descriptor", "AD-16",
                            (0, 0, chh as i32, cw as i32),
                            "fill layer parameters not mapped at import v1; appearance preserved from the composite".into(),
                        ));
                    }
                    k if ADJUSTMENT_KEYS.contains(&k) => {
                        extra_fallback = Some((
                            "adjustment", "AD-17",
                            (0, 0, chh as i32, cw as i32),
                            format!(
                                "adjustment `{}` not mapped at import v1; appearance preserved from the composite",
                                String::from_utf8_lossy(k)
                            ),
                        ));
                    }
                    b"TySh" => report.push(
                        Some(&path_str),
                        rec.lyid,
                        "text",
                        PsdAction::Rasterized,
                        Some("TX-05"),
                        None,
                        "type layer imports its stored raster (§23 text import pending)",
                    ),
                    b"SoLd" | b"PlLd" => report.push(
                        Some(&path_str),
                        rec.lyid,
                        "smart_object",
                        PsdAction::Rasterized,
                        Some("SO-04"),
                        None,
                        "smart object imports its stored raster (§11.5 pending)",
                    ),
                    b"vmsk" | b"vsms" => {
                        if real_mask_plane.is_none() {
                            report.push(
                                Some(&path_str),
                                rec.lyid,
                                "mask.vector",
                                PsdAction::Dropped,
                                Some("MK-02"),
                                None,
                                "vector mask without a −3 raster channel dropped at v1",
                            );
                        }
                    }
                    b"clbl" => report.push(
                        Some(&path_str),
                        rec.lyid,
                        "blend.clipped_flag",
                        PsdAction::Approximated,
                        None,
                        None,
                        "clipped-element blending flag noted (§8.5)",
                    ),
                    other => report.push(
                        Some(&path_str),
                        rec.lyid,
                        format!("psd.info.{}", String::from_utf8_lossy(other).trim()),
                        PsdAction::Dropped,
                        None,
                        None,
                        "unrecognized additional-info key skipped by length (§8.5)",
                    ),
                }
            }
            if let Some((feature, row, region, note)) = extra_fallback {
                if let Some((plane, (fx, fy))) = crop_composite(&composite16, cw, chh, region) {
                    let (png, buffer) = plane.to_png_and_buffer(opts.working_profile)?;
                    fallbacks.push(ImportedLayer {
                        name: format!("{} (composite fallback)", rec.name),
                        buffer,
                        ingress_png: png,
                        offset: appname_core::IVec2::new(fx, fy),
                        opacity: 1.0,
                        fill_opacity: 1.0,
                        blend: BlendMode::Normal,
                        visible: true,
                        clip_to_below: false,
                        locks: LayerLocks::default(),
                        group_path: Vec::new(),
                        icc: None,
                    });
                    report.push(
                        Some(&path_str),
                        rec.lyid,
                        feature,
                        PsdAction::FellBackToComposite,
                        Some(row),
                        Some((
                            region.1,
                            region.0,
                            (region.3 - region.1).max(0) as u32,
                            (region.2 - region.0).max(0) as u32,
                        )),
                        note,
                    );
                } else {
                    report.push(
                        Some(&path_str),
                        rec.lyid,
                        feature,
                        PsdAction::Dropped,
                        Some(row),
                        None,
                        format!("{note} (region empty after canvas clamp)"),
                    );
                }
            }

            // Live pixel layer.
            if lw == 0
                || lh == 0
                || comp_planes.len() < hdr.mode.components().min(rec.channels.len())
            {
                continue; // no own pixels (adjustment/fill markers)
            }
            let mut rgba16 = assemble_rgba16(
                hdr.mode,
                hdr.depth,
                lw,
                lh,
                &comp_planes,
                alpha_plane.as_ref(),
                palette,
                res.transparency_index,
            )?;
            // Raster mask → rasterized into alpha (registered fallback).
            let effective_mask = mask_plane.as_ref().or(real_mask_plane.as_ref());
            if let (Some(mplane), Some(m)) = (effective_mask, &rec.mask) {
                if !m.disabled {
                    let (mt, ml, mb, mr) = m.rect;
                    let (mw, mh) = ((mr - ml).max(0) as usize, (mb - mt).max(0) as usize);
                    for y in 0..lh {
                        for x in 0..lw {
                            let gx = left + x as i32;
                            let gy = top + y as i32;
                            let mv =
                                if gx >= ml && gx < mr && gy >= mt && gy < mb && mw > 0 && mh > 0 {
                                    sample_u16(
                                        mplane,
                                        (gy - mt) as usize * mw + (gx - ml) as usize,
                                        hdr.depth,
                                    )
                                } else {
                                    u16::from(m.default_color) * 257
                                };
                            let ai = (y * lw + x) * 4 + 3;
                            rgba16[ai] = ((u32::from(rgba16[ai]) * u32::from(mv)) / 65535) as u16;
                        }
                    }
                    report.push(
                        Some(&path_str), rec.lyid, "mask.raster", PsdAction::Rasterized,
                        Some("MK-01"), None,
                        "layer mask rasterized into the alpha channel (MaskNode application pending)",
                    );
                }
            }
            let plane = plane_from_rgba16(lw as u32, lh as u32, rgba16, hdr.depth)?;
            let (png, buffer) = plane.to_png_and_buffer(opts.working_profile)?;
            layers.push(ImportedLayer {
                name: rec.name.clone(),
                buffer,
                ingress_png: png,
                offset: appname_core::IVec2::new(left, top),
                opacity: rec.opacity,
                fill_opacity: rec.fill_opacity,
                blend: rec.blend,
                visible: !rec.hidden,
                clip_to_below: rec.clipping,
                locks: LayerLocks {
                    transparency: rec.lock_transparency_flag || rec.lspf & 0b1 != 0,
                    pixels: rec.lspf & 0b10 != 0,
                    position: rec.lspf & 0b100 != 0,
                },
                group_path: if balanced {
                    group_stack.clone()
                } else {
                    Vec::new()
                },
                icc: None,
            });
        }
    }
    // §42.1: fallback layers go ABOVE all other layers (top of stack).
    layers.extend(fallbacks);

    let (ingress_png, buffer) = composite_plane.to_png_and_buffer(opts.working_profile)?;
    let meta = Metadata {
        icc: res.icc.clone(),
        exif: if opts.keep_unknown_metadata {
            res.exif.clone()
        } else {
            None
        },
        xmp: if opts.keep_unknown_metadata {
            res.xmp.clone()
        } else {
            None
        },
        iptc: None,
        animation_loop: None,
    };
    let mut crate_report = CompatReport::default();
    let compat = report.finish(&hdr, "imported", &mut crate_report);
    Ok(Imported {
        buffer,
        ingress_png,
        size: (hdr.width, hdr.height),
        profile: res.icc,
        meta,
        layers: if layers.is_empty() {
            None
        } else {
            Some(layers)
        },
        orientation_applied: None, // resource-1058 EXIF is carried, not baked: PSD pixels are upright
        report: crate_report,
        compat_report: Some(compat),
    })
}

impl Codec for PsdCodec {
    fn id(&self) -> CodecId {
        CodecId("psd")
    }

    fn capabilities(&self) -> CodecCaps {
        CodecCaps {
            read: true,
            write: false, // §8.5 is import-only; PSD write is out of contract
            layered: true,
            max_depth: appname_core::ChannelKind::F32,
            alpha: true,
            animation: false,
            license: LicenseStatus::Builtin,
        }
    }

    fn sniff(&self, head: &[u8]) -> bool {
        head.starts_with(b"8BPS")
    }

    fn magic_len(&self) -> u32 {
        4
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["psd", "psb"]
    }

    fn decode(&self, bytes: &[u8], opts: &DecodeOpts) -> Result<Imported, CodecError> {
        import_psd(bytes, opts)
    }

    fn encode(&self, _img: &ExportImage<'_>, _opts: &EncodeOpts) -> Result<Vec<u8>, CodecError> {
        Err(CodecError::WriteUnsupported)
    }
}
