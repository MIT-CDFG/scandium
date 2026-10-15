// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §7.2/§7.3 (manifest/document JSON shapes), §7.4 (tile chunk
//! binary format v1), §7.6 (ZIP container, STORED entries), §7.8-cited
//! CRC-32 + §7.4 xxHash64 via core's public `io::integrity`; plan §2.2
//! (cross-check + document-level test rows).
//!
//! Integration-test fixture factory for the CP-0.3 compositor suite.
//!
//! The render lane can only READ core (no mutation doors are public — the
//! D-48 spine), so document fixtures are built exclusively through core's
//! PUBLIC ingestion doors:
//!
//! * [`appname_core::Engine::apply_json`] with a `doc.new` envelope
//!   (white/black/transparent backgrounds), and
//! * [`appname_core::load_package_bytes`] over `.appname` packages this
//!   module assembles byte-by-byte from the PUBLISHED formats — ZIP with
//!   STORED entries (ES §7.6 container; PKWARE APPNOTE layout, the same
//!   container the spec pins), `manifest.json`/`document.json` (ES
//!   §7.2/§7.3 shapes as core's own writer emits them), and §7.4 tile
//!   chunks (48-byte header + raw LE payload).
//!
//! Checksums come from core's public [`appname_core::io::integrity`]
//! (`crc32` for ZIP entries — the §7.8-cited IEEE polynomial — and `xxh64`
//! for chunk payloads), and the chunk magic/version constants from
//! [`appname_core::io::tile_chunk`]. Nothing here bypasses the loader's
//! validation: every fixture is a legitimate package the product itself
//! could open.

use appname_core::io::integrity::{crc32, xxh64};
use appname_core::io::package::{DOCUMENT_SCHEMA, MANIFEST_SCHEMA, PACKAGE_MIMETYPE};
use appname_core::io::tile_chunk::{FMT_MAJOR, FMT_MINOR, MAGIC};
use appname_core::{load_package_bytes, DocumentId, Engine};

/// One uniform-valued tile of a fixture layer (values are
/// storage-normalized, like the wire payload: for `f32` fixtures linear
/// premultiplied working values; for `u8` fixtures the interim sRGB-encoded
/// convention core documents at `io::srgb`).
pub struct TileSpec {
    pub x: i32,
    pub y: i32,
    pub value: [f32; 4],
}

/// What a fixture layer is.
pub enum NodeKind {
    Raster { tiles: Vec<TileSpec> },
    Group { children: Vec<&'static str> },
}

/// One layer entry of a fixture document (§7.3 fields core's loader reads).
pub struct LayerSpec {
    /// Hyphenated UUID string (deterministic fixture ids).
    pub id: &'static str,
    pub name: &'static str,
    pub kind: NodeKind,
    /// §4.5 JSON string, camelCase (`"normal"`, `"multiply"`, …).
    pub blend: &'static str,
    pub opacity: f64,
    pub fill_opacity: f64,
    pub visible: bool,
    pub clip_to_below: bool,
    /// Column-major 9-array; `None` = identity.
    pub transform: Option<[f64; 9]>,
}

impl LayerSpec {
    /// A raster layer with the §2.6 defaults.
    pub fn raster(id: &'static str, tiles: Vec<TileSpec>) -> LayerSpec {
        LayerSpec {
            id,
            name: "layer",
            kind: NodeKind::Raster { tiles },
            blend: "normal",
            opacity: 1.0,
            fill_opacity: 1.0,
            visible: true,
            clip_to_below: false,
            transform: None,
        }
    }

    /// A group with the §2.6 defaults (pass-through, §4.7).
    pub fn group(id: &'static str, children: Vec<&'static str>) -> LayerSpec {
        LayerSpec {
            id,
            name: "group",
            kind: NodeKind::Group { children },
            blend: "normal",
            opacity: 1.0,
            fill_opacity: 1.0,
            visible: true,
            clip_to_below: false,
            transform: None,
        }
    }
}

/// A whole fixture package.
pub struct PkgSpec {
    pub width: i32,
    pub height: i32,
    /// `"u8"` or `"f32"` — document depth AND every chunk's depth.
    pub depth: &'static str,
    /// Chunk header flag bit0 (§7.4): fixtures are premultiplied unless a
    /// test exercises the straight-alpha working conversion.
    pub premultiplied: bool,
    /// All non-root layers, any order; `root_children` wires the tree.
    pub layers: Vec<LayerSpec>,
    /// Bottom-to-top ids of the root stack (§2.6: index 0 composites
    /// first).
    pub root_children: Vec<&'static str>,
}

pub const ROOT_ID: &str = "aaaaaaaa-0000-4000-8000-000000000000";
const DOC_ID: &str = "dddddddd-0000-4000-8000-000000000001";
/// The deterministic RGBA working-profile placeholder id
/// (`appname_core::engine::doc_new::builtin_working_profile`).
fn working_profile_uuid() -> String {
    appname_core::engine::doc_new::builtin_working_profile(appname_core::ColorModelKind::Rgba)
        .as_uuid()
        .to_string()
}

/// Assemble the package bytes for `spec` (module doc: published formats,
/// STORED zip, §7.4 chunks).
pub fn build_package(spec: &PkgSpec) -> Vec<u8> {
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    entries.push(("mimetype".into(), PACKAGE_MIMETYPE.as_bytes().to_vec()));
    entries.push((
        "manifest.json".into(),
        format!(
            "{{\n  \"schema\": \"{MANIFEST_SCHEMA}\",\n  \"documentId\": \"{DOC_ID}\",\n  \
             \"requires\": {{ \"minReader\": \"0.1.0\" }},\n  \"document\": \"document.json\"\n}}\n"
        )
        .into_bytes(),
    ));
    entries.push(("document.json".into(), document_json(spec).into_bytes()));
    for layer in &spec.layers {
        if let NodeKind::Raster { tiles } = &layer.kind {
            for t in tiles {
                let name = format!("layers/{}/tiles/0/{}_{}.tile", layer.id, t.x, t.y);
                entries.push((name, tile_chunk_uniform(t, spec.depth, spec.premultiplied)));
            }
        }
    }
    zip_stored(&entries)
}

/// Build and load in one step; loader warnings must be empty (fixtures are
/// clean packages).
pub fn load_fixture(spec: &PkgSpec) -> Engine {
    let bytes = build_package(spec);
    let loaded = load_package_bytes(&bytes).expect("fixture package loads");
    assert!(
        loaded.warnings.is_empty(),
        "fixture produced loader warnings: {:?}",
        loaded.warnings
    );
    loaded.engine
}

/// ES §7.3 `document.json` in the exact field shape core's writer emits
/// (§2.8 spellings: `imageSize`/`canvasSize` as `[w, h]`,
/// `resolutionPPI` published spelling, camelCase blend strings).
fn document_json(spec: &PkgSpec) -> String {
    let mut layers_json: Vec<String> = Vec::new();
    // Root group entry: id + kind + children (the loader builds the root
    // from `Document::new` and reads only `children` here; §2.6 requires
    // it undecorated).
    layers_json.push(format!(
        "{{ \"id\": \"{ROOT_ID}\", \"name\": \"root\", \"kind\": \"group\", \"children\": [{}] }}",
        spec.root_children
            .iter()
            .map(|c| format!("\"{c}\""))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    for l in &spec.layers {
        let transform = l
            .transform
            .unwrap_or([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0])
            .map(|v| format!("{v:?}"))
            .join(", ");
        let kind_fields = match &l.kind {
            NodeKind::Raster { .. } => {
                format!(
                    "\"kind\": \"raster\", \"tilesRef\": \"layers/{}/tiles\"",
                    l.id
                )
            }
            NodeKind::Group { children } => format!(
                "\"kind\": \"group\", \"children\": [{}]",
                children
                    .iter()
                    .map(|c| format!("\"{c}\""))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        layers_json.push(format!(
            "{{ \"id\": \"{}\", \"name\": \"{}\", {kind_fields}, \"blend\": \"{}\", \
             \"opacity\": {:?}, \"fillOpacity\": {:?}, \"visible\": {}, \
             \"clipToBelow\": {}, \"transform\": [{transform}], \"masks\": [] }}",
            l.id, l.name, l.blend, l.opacity, l.fill_opacity, l.visible, l.clip_to_below
        ));
    }
    format!(
        "{{\n  \"schema\": \"{DOCUMENT_SCHEMA}\",\n  \"geometry\": {{ \"imageSize\": [{w}, {h}], \
         \"canvasSize\": [{w}, {h}], \"resolutionPPI\": 72.0 }},\n  \"color\": {{ \"working\": \
         \"{profile}\", \"mode\": \"rgba\", \"depth\": \"{depth}\" }},\n  \"root\": \"{ROOT_ID}\",\n  \
         \"layers\": [\n    {layers}\n  ]\n}}\n",
        w = spec.width,
        h = spec.height,
        profile = working_profile_uuid(),
        depth = spec.depth,
        layers = layers_json.join(",\n    ")
    )
}

/// One §7.4 tile chunk (uniform value, compression = none): the pinned
/// 48-byte LE header + raw payload; checksum = xxHash64 of the payload
/// (core's public `integrity::xxh64`).
fn tile_chunk_uniform(t: &TileSpec, depth: &str, premultiplied: bool) -> Vec<u8> {
    let (pixel_format, bytes_per): (u8, usize) = match depth {
        "u8" => (0, 1),
        "f32" => (3, 4),
        other => panic!("fixture depth {other} unsupported"),
    };
    let mut one_px: Vec<u8> = Vec::new();
    for c in 0..4 {
        match depth {
            "u8" => one_px.push((t.value[c].clamp(0.0, 1.0) * 255.0).round() as u8),
            _ => one_px.extend_from_slice(&t.value[c].to_le_bytes()),
        }
    }
    let payload_len = 256 * 256 * 4 * bytes_per;
    let mut payload = Vec::with_capacity(payload_len);
    for _ in 0..256 * 256 {
        payload.extend_from_slice(&one_px);
    }
    debug_assert_eq!(payload.len(), payload_len);

    let mut out = Vec::with_capacity(48 + payload_len);
    out.extend_from_slice(MAGIC);
    out.push(FMT_MAJOR);
    out.push(FMT_MINOR);
    out.extend_from_slice(&256u32.to_le_bytes()); // width
    out.extend_from_slice(&256u32.to_le_bytes()); // height
    out.extend_from_slice(&t.x.to_le_bytes());
    out.extend_from_slice(&t.y.to_le_bytes());
    out.push(0); // lod
    out.push(pixel_format); // §7.4 coding: 0 = u8, 3 = f32
    out.push(3); // color_model 3 = RGBA
    out.push(premultiplied as u8); // flags bit0
    out.push(0); // compression = none
    out.extend_from_slice(&[0, 0, 0]); // reserved
    out.extend_from_slice(&(payload_len as u64).to_le_bytes());
    out.extend_from_slice(&xxh64(&payload).to_le_bytes());
    out.extend_from_slice(&payload);
    out
}

/// Minimal STORED-only ZIP writer (PKWARE APPNOTE layout: local headers +
/// central directory + EOCD; CRC-32 via core's public `integrity::crc32`).
/// Entry 0 must be the mimetype (§7.1) — callers put it first.
fn zip_stored(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();
    for (name, data) in entries {
        let offset = out.len() as u32;
        let crc = crc32(data);
        let n = name.as_bytes();
        let (nlen, dlen) = (n.len() as u16, data.len() as u32);
        // Local file header.
        out.extend_from_slice(&0x04034b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0u16.to_le_bytes()); // flags (no descriptor)
        out.extend_from_slice(&0u16.to_le_bytes()); // method = STORED
        out.extend_from_slice(&0u16.to_le_bytes()); // mod time
        out.extend_from_slice(&0x0021u16.to_le_bytes()); // mod date 1980-01-01
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&dlen.to_le_bytes()); // compressed
        out.extend_from_slice(&dlen.to_le_bytes()); // uncompressed
        out.extend_from_slice(&nlen.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra len
        out.extend_from_slice(n);
        out.extend_from_slice(data);
        // Central directory record.
        central.extend_from_slice(&0x02014b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes()); // made by
        central.extend_from_slice(&20u16.to_le_bytes()); // needed
        central.extend_from_slice(&0u16.to_le_bytes()); // flags
        central.extend_from_slice(&0u16.to_le_bytes()); // method
        central.extend_from_slice(&0u16.to_le_bytes()); // time
        central.extend_from_slice(&0x0021u16.to_le_bytes()); // date
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&dlen.to_le_bytes());
        central.extend_from_slice(&dlen.to_le_bytes());
        central.extend_from_slice(&nlen.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // extra
        central.extend_from_slice(&0u16.to_le_bytes()); // comment
        central.extend_from_slice(&0u16.to_le_bytes()); // disk start
        central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
        central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(n);
    }
    let cd_offset = out.len() as u32;
    out.extend_from_slice(&central);
    let count = entries.len() as u16;
    // End of central directory.
    out.extend_from_slice(&0x06054b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // this disk
    out.extend_from_slice(&0u16.to_le_bytes()); // cd disk
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // comment len
    out
}

/// A fresh engine with one applied `doc.new` (core's one public command
/// door at CP-0.3's start).
// Shared across test binaries; not every binary uses every helper.
#[allow(dead_code)]
pub fn engine_doc_new(width: i32, height: i32, depth: &str, background: &str) -> Engine {
    let mut engine = Engine::new();
    let id = DocumentId::new().as_uuid(); // fresh envelope id
    let env = format!(
        "{{\"schema\":\"appname.command.v1\",\"id\":\"{id}\",\"kind\":\"doc.new\",\
         \"params\":{{\"width\":{width},\"height\":{height},\"resolutionPpi\":72.0,\
         \"mode\":\"rgba\",\"depth\":\"{depth}\",\"background\":\"{background}\"}}}}"
    );
    engine.apply_json(&env).expect("doc.new applies");
    engine
}
