// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §7.1 (package layout — mimetype FIRST and STORED), §7.2
//! (manifest), §7.3 (document.json — `contracts/schemas/document.v1.json`),
//! §7.4 (tile trees), §7.6 (atomic saves), §7.8 (journal entry); DD ⑩.
//!
//! The `.appname` package (OQ-5: ES §7's title, mimetype, and §7.6 temp
//! naming all fix the `.appname` extension). CP-0.1 decisions, recorded in
//! the lane log:
//! * every entry STORED (the recorded zip crate runs default-features off —
//!   no deflate backend; §7.1 permits store per entry), tiles uncompressed
//!   (§3.8 sanctions "zstd (or none)"; zstd awaits its record);
//! * full-rewrite saves (the §7.6 incremental `PackageIndex` diff is an
//!   optimization for the editing loop; atomicity — temp + fsync + rename,
//!   same directory — is implemented exactly);
//! * the §7.8 journal entry carries the FULL session command stream
//!   (BASE + one COMMAND record per envelope + a CHECKPOINT encoding the
//!   cursor as the last in-effect command id); no wall-clock values are
//!   written anywhere (fixed epoch timestamps) so package bytes are a
//!   deterministic function of document state — the §7.8 autosave cadence
//!   arrives with the scratch-disk journal (0.7);
//! * history is NOT persisted (§10.4): a reopened engine undoes/redoes via
//!   the CLI replay path, not in-process deltas.

use std::io::{Read, Write};
use std::path::Path;

use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::buffer::Buffer;
use crate::engine::envelope::Envelope;
use crate::engine::error::EngineError;
use crate::engine::Engine;
use crate::graph::document::{ColorState, Document, DocumentGeometry};
use crate::graph::id::{DocumentId, LayerId};
use crate::graph::node::{BlendMode, ColorLabel, LayerNode, LockFlags};
use crate::io::tile_chunk::{self, ChunkOutcome, TileWarning};
use crate::io::{integrity, read_err, write_err};
use crate::serial::Maybe;

pub const PACKAGE_MIMETYPE: &str = "application/x-appname";
pub const MANIFEST_SCHEMA: &str = "appname.manifest.v1";
pub const DOCUMENT_SCHEMA: &str = "appname.document.v1";
/// This build's reader capability (manifest `requires.minReader` gate).
pub const READER_VERSION: (u32, u32, u32) = (0, 1, 0);
/// Fixed timestamp: no wall clock in packages (byte determinism; lane log).
pub const EPOCH_TS: &str = "1970-01-01T00:00:00Z";

/// A loaded package: the engine plus the §7.4/§7.8 graceful warnings.
pub struct LoadedPackage {
    pub engine: Engine,
    pub warnings: Vec<TileWarning>,
}

impl core::fmt::Debug for LoadedPackage {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LoadedPackage")
            .field("warnings", &self.warnings)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Save.
// ---------------------------------------------------------------------------

/// §7.6 atomic save, steps 4–6: temp file in the same directory (same
/// volume), fsync the file, fsync the parent directory (step 5: "so the
/// rename that follows is durable"), atomic rename, then fsync the parent
/// again so the rename itself is durable (review minor 2).
pub fn save_package(engine: &Engine, path: &Path) -> Result<(), EngineError> {
    let bytes = save_package_bytes(engine)?;
    let tmp = path.with_extension("appname.tmp");
    {
        let mut f = std::fs::File::create(&tmp)
            .map_err(|e| write_err(&format!("creating {}", tmp.display()), e))?;
        f.write_all(&bytes)
            .map_err(|e| write_err(&format!("writing {}", tmp.display()), e))?;
        f.sync_all()
            .map_err(|e| write_err(&format!("syncing {}", tmp.display()), e))?;
    }
    let parent = tmp
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf);
    sync_dir(parent.as_deref())?; // step 5: temp's directory entry durable
    std::fs::rename(&tmp, path)
        .map_err(|e| write_err(&format!("renaming into {}", path.display()), e))?;
    sync_dir(parent.as_deref()) // step 6 durability: the rename persists
}

/// Directory metadata flush (§7.6 step 5). `None` is a documented no-op;
/// failures are typed write errors.
///
/// W-B-010 (operator ruling under FI-31, required-where-supported): the doc line
/// previously read "`None`/unsupported platforms are a documented no-op" and the
/// body had NO PLATFORM ARM — it always called `File::open` on a directory. On
/// Windows that fails with **os error 5**, so File>New failed there and 46 tests
/// with it. THE DOC DESCRIBED A BEHAVIOUR THE CODE DID NOT IMPLEMENT.
///
/// Windows DOES support this: a directory handle requires
/// `FILE_FLAG_BACKUP_SEMANTICS`, after which `FlushFileBuffers` (what `sync_all`
/// calls) is accepted. So the no-op sentence applies to `None` and to platforms
/// genuinely without the capability — NOT to Windows.
fn sync_dir(dir: Option<&Path>) -> Result<(), EngineError> {
    let Some(dir) = dir else { return Ok(()) };
    let f = open_dir_for_sync(dir)
        .map_err(|e| write_err(&format!("opening directory {} for fsync", dir.display()), e))?;
    f.sync_all()
        .map_err(|e| write_err(&format!("fsyncing directory {}", dir.display()), e))
}

/// Unix arm: byte-for-byte the call `sync_dir` made before W-B-010. The split
/// exists so the Windows arm can differ; this arm is unchanged by construction.
#[cfg(not(windows))]
fn open_dir_for_sync(dir: &Path) -> std::io::Result<std::fs::File> {
    std::fs::File::open(dir)
}

/// Windows arm. `CreateFile` refuses a directory without
/// `FILE_FLAG_BACKUP_SEMANTICS` (0x0200_0000) and returns ERROR_ACCESS_DENIED,
/// which surfaces as os error 5 — the reported failure. The constant is spelled
/// here rather than pulled from a crate: this is the first platform-conditional
/// code in appname-core and the row is not a dependency decision.
#[cfg(windows)]
fn open_dir_for_sync(dir: &Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    // W-07 (O-WIN): the open alone succeeds read-only, but the
    // FlushFileBuffers that `sync_all` issues needs GENERIC_WRITE on the
    // handle — read-only it fails with os error 5. Measured on Windows 11.
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(dir)
}

pub fn save_package_bytes(engine: &Engine) -> Result<Vec<u8>, EngineError> {
    // Smart-object sources serialize as §7.4 content hashes (schema
    // `assetHash`); pre-compute blake3 per store asset for the writer.
    ASSET_HASHES.with(|m| {
        let mut m = m.borrow_mut();
        m.clear();
        for id in engine.assets().ids_sorted() {
            if let Some(bytes) = engine.assets().bytes(id) {
                m.insert(id, blake3::hash(bytes).to_hex().to_string());
            }
        }
    });
    let doc = engine
        .document()
        .ok_or_else(|| EngineError::bad_target("an uninitialized document has no package form"))?;

    let manifest = json!({
        "schema": MANIFEST_SCHEMA,
        "documentId": doc.id().as_uuid(),
        "createdWith": { "app": "appname", "version": env!("CARGO_PKG_VERSION") },
        "requires": { "minReader": "0.1.0", "features": ["tiles.v1", "layers.v1"] },
        "document": "document.json",
    });

    let mut layers = Vec::new();
    layers.push(layer_value(doc, doc.root())?);
    for id in doc.z_order_walk() {
        layers.push(layer_value(doc, id)?);
    }
    let mut document = json!({
        "schema": DOCUMENT_SCHEMA,
        "geometry": serde_json::to_value(doc.geometry()).expect("geometry serializes"),
        "color": serde_json::to_value(doc.color()).expect("color serializes"),
        "root": doc.root().as_uuid(),
        "layers": layers,
    });
    // §7.5 op sidecars: root "ops" = the OpId inventory; each op's body
    // lives at ops/{opId}.json (appname.op.v1). Emitted only when ops
    // exist (byte-compat with every op-free package).
    let mut op_ids: Vec<Uuid> = Vec::new();
    {
        let mut walk_ops: Vec<crate::graph::id::OpId> = Vec::new();
        for lid in doc.z_order_walk() {
            let n = doc.layer(lid).expect("walk resolves");
            if let Some(op) = n.adjustment_op() {
                walk_ops.push(op);
            }
            if let Some(so) = n.smart_object() {
                walk_ops.extend(so.smart_filters().iter().copied());
            }
        }
        for oid in walk_ops {
            op_ids.push(oid.as_uuid());
        }
    }
    if !op_ids.is_empty() {
        document["ops"] = Value::Array(op_ids.iter().map(|o| json!(o.to_string())).collect());
    }

    // §20 doc-path inventory — emitted only when paths exist (byte-compat
    // with every vector-free package). SCHEMA-GAP (flagged, never silent —
    // crop_rect precedent): document.v1.json carries per-layer/mask
    // `pathRef` but NO doc-level paths list; the paths-panel arena needs
    // one. Written as root "paths": [{id, name?, pathRef, bounds}] with
    // sidecars at vector/{pathId}.json (the pathRef convention). `bounds`
    // rides so a geometry-less reader can carry paths opaquely with dirty
    // hulls intact; a geometry-carrying loader revalidates. Fold-in
    // candidate recorded in the lane log.
    if doc.path_count() > 0 {
        let mut path_rows = Vec::new();
        for pid in doc.path_ids_sorted() {
            let p = doc.path(pid).expect("sorted ids resolve");
            let mut row = Map::new();
            row.insert("id".into(), json!(pid.as_uuid()));
            if let Some(n) = p.name() {
                row.insert("name".into(), json!(n));
            }
            row.insert("pathRef".into(), json!(pid.as_uuid()));
            let b = p.bounds();
            row.insert(
                "bounds".into(),
                json!({"x": b.min.x, "y": b.min.y,
                       "w": b.max.x - b.min.x, "h": b.max.y - b.min.y}),
            );
            path_rows.push(Value::Object(row));
        }
        document["paths"] = Value::Array(path_rows);
    }

    // §14.1 named-channel inventory — emitted only when channels exist
    // (byte-compat with every channel-free package). SCHEMA-GAP (flagged,
    // never silent — the paths precedent): document.v1.json carries no
    // doc-level channels list; §14.2 persistence needs one. Root
    // "channels": [{id, name, tilesRef, spot?}] in §14.1 LIST ORDER, tile
    // trees at channels/{channelId}/tiles. Fold-in candidate recorded in
    // the lane log.
    if doc.channel_count() > 0 {
        let mut chan_rows = Vec::new();
        for cid in doc.channel_ids_ordered() {
            let c = doc.channel(*cid).expect("ordered ids resolve");
            let mut row = Map::new();
            row.insert("id".into(), json!(cid.as_uuid()));
            row.insert("name".into(), json!(c.name()));
            row.insert(
                "tilesRef".into(),
                json!(format!("channels/{}/tiles", cid.as_uuid())),
            );
            if let Some(spot) = c.spot() {
                // §14.3/§7.7 round-trip room: carried verbatim.
                row.insert("spot".into(), spot.clone());
            }
            chan_rows.push(Value::Object(row));
        }
        document["channels"] = Value::Array(chan_rows);
    }

    // §29.2 slices — emitted only when present (byte-compat; guides
    // precedent). SCHEMA-GAP flagged with the channels row above:
    // document.v1.json has no slices key.
    if !doc.slices().is_empty() {
        document["slices"] = serde_json::to_value(doc.slices()).expect("slice rows serialize");
    }

    // §7.3 guides object — emitted only when non-default (byte-compat with
    // every guide-free package; the grid block rides inside — schema-gap
    // note, the $def permits additional properties).
    if !doc.guides().is_default() {
        document["guides"] = serde_json::to_value(doc.guides()).expect("guides serialize");
    }
    // §7.3 open meta object — the §42.4 import-provenance carriage
    // (`meta["compatReport"]` "survives the save to `.appname`").
    // Emitted only when non-empty (byte-compat with every meta-free
    // package). serde_json::Map is key-sorted (BTreeMap default), so
    // emission is deterministic and a save→load→save cycle byte-stable.
    if !doc.meta().is_empty() {
        document["meta"] = Value::Object(doc.meta().clone());
    }
    // Mask nodes (schema $defs.mask; emitted only when present so
    // mask-free documents stay byte-identical to their CP-0.3 form).
    if doc.mask_count() > 0 {
        let mut mask_values = Vec::new();
        // Op masks first? No — keep layer masks in walk order, then op
        // masks in op order (deterministic; loader attaches by reference).
        for lid in doc.z_order_walk() {
            let node = doc.layer(lid).expect("walk resolves");
            let ops: Vec<crate::graph::id::OpId> = node
                .adjustment_op()
                .into_iter()
                .chain(
                    node.smart_object()
                        .into_iter()
                        .flat_map(|so| so.smart_filters().iter().copied()),
                )
                .collect();
            for oid in &ops {
                if let Some(mid) = doc.op(*oid).and_then(|o| o.mask()) {
                    let m = doc.mask(mid).expect("validated tree");
                    mask_values.push(json!({
                        "id": m.id().as_uuid(),
                        "kind": "raster",
                        "enabled": m.enabled(),
                        "inverted": m.inverted(),
                        "density": m.density(),
                        "feather": m.feather(),
                        "linked": m.linked(),
                        "tilesRef": format!("ops/{}/mask/{}/tiles", oid.as_uuid(), m.id().as_uuid()),
                    }));
                }
            }
            for mid in node.masks() {
                let m = doc.mask(*mid).expect("validated tree");
                mask_values.push(json!({
                    "id": m.id().as_uuid(),
                    "kind": "raster",
                    "enabled": m.enabled(),
                    "inverted": m.inverted(),
                    "density": m.density(),
                    "feather": m.feather(),
                    "linked": m.linked(),
                    "tilesRef": format!("layers/{}/masks/{}/tiles", lid.as_uuid(), m.id().as_uuid()),
                }));
            }
        }
        document["masks"] = Value::Array(mask_values);
    }

    // Tail entries (everything after document.json), then sorted into the
    // §7.6 step-4 [NORMATIVE] total order: ascending byte-lexicographic
    // entry name — "this total order is the reproducibility contract"
    // (review minor 1; normative wins over the earlier (lod, y, x)
    // grouping, which remains embedded in the names themselves).
    let mut tail: Vec<(String, Vec<u8>)> = Vec::new();
    for id in doc.z_order_walk() {
        let node = doc.layer(id).expect("walk resolves");
        // Raster pixels OR the smart object's persisted render cache
        // (§11.5: cached_composite persists as the layer's tiles).
        let tile_source = node
            .pixels()
            .or_else(|| node.smart_object().map(|so| so.cached_composite()));
        if let Some(buf) = tile_source {
            for (c, tile) in buf.tiles().iter() {
                let chunk = tile_chunk::write_tile_chunk(
                    c,
                    tile,
                    buf.kind(),
                    buf.model(),
                    buf.premultiplied(),
                );
                let name = format!(
                    "layers/{}/tiles/{}/{}_{}.tile",
                    id.as_uuid(),
                    c.lod,
                    c.x,
                    c.y
                );
                tail.push((name, chunk));
            }
        }
        // Mask coverage trees (§7.1: layers/{layerId}/masks/{maskId}/tiles).
        for mid in node.masks() {
            let m = doc.mask(*mid).expect("validated tree");
            let buf = m.coverage();
            for (c, tile) in buf.tiles().iter() {
                let chunk = tile_chunk::write_tile_chunk(
                    c,
                    tile,
                    buf.kind(),
                    buf.model(),
                    buf.premultiplied(),
                );
                let name = format!(
                    "layers/{}/masks/{}/tiles/{}/{}_{}.tile",
                    id.as_uuid(),
                    mid.as_uuid(),
                    c.lod,
                    c.x,
                    c.y
                );
                tail.push((name, chunk));
            }
        }
    }
    // §14.2 channel coverage trees (channels/{channelId}/tiles): 8-bit
    // encoding ONLY when the tile's coverage plane is u8-exact
    // (`appname_color::plane_is_u8_exact` — the lossless-or-not-at-all
    // predicate), else f32 verbatim.
    for cid in doc.channel_ids_ordered() {
        let c = doc.channel(*cid).expect("ordered ids resolve");
        let buf = c.buffer();
        for (coord, tile) in buf.tiles().iter() {
            let mut plane = vec![0.0f32; crate::tile::TILE * crate::tile::TILE];
            for y in 0..crate::tile::TILE {
                for x in 0..crate::tile::TILE {
                    plane[y * crate::tile::TILE + x] = tile.read_pixel_f32(x, y, buf.model())[0];
                }
            }
            let kind = if appname_color::plane_is_u8_exact(&plane) {
                crate::tile::channel::ChannelKind::U8
            } else {
                crate::tile::channel::ChannelKind::F32
            };
            let chunk =
                tile_chunk::write_tile_chunk(coord, tile, kind, buf.model(), buf.premultiplied());
            let name = format!(
                "channels/{}/tiles/{}/{}_{}.tile",
                cid.as_uuid(),
                coord.lod,
                coord.x,
                coord.y
            );
            tail.push((name, chunk));
        }
    }
    // §20 vector sidecars: vector/{pathId}.json (the document.v1 pathRef
    // convention), normalized bytes verbatim (content-hash identity is the
    // reproducibility contract; the byte-lex tail order covers them).
    for pid in doc.path_ids_sorted() {
        let p = doc.path(pid).expect("sorted ids resolve");
        tail.push((
            format!("vector/{}.json", pid.as_uuid()),
            p.sidecar().as_ref().clone(),
        ));
    }
    // §23.1 text sidecars: text/{layerId}.json (the document.v1 textRef
    // convention), normalized bytes verbatim (content-hash identity; the
    // byte-lex tail order covers them).
    for lid in doc.z_order_walk() {
        if let Some(td) = doc.layer(lid).expect("walk resolves").text_data() {
            tail.push((
                format!("text/{}.json", lid.as_uuid()),
                td.sidecar().as_ref().clone(),
            ));
        }
    }
    tail.push((
        "journal/autosave.log".to_string(),
        write_journal(doc.id(), engine.journal(), engine.journal_cursor()),
    ));
    // D-48 vector value-captures ride the journal (bound posture:
    // `vector.shape_add` replay never re-runs constructor trig). One
    // sidecar entry (bytes VERBATIM — capture identity is byte identity)
    // + one meta entry (bounds) per captured envelope still in the
    // journal. Absent when no captures exist (byte-compat with every
    // capture-free package; §7.7 additive rules — package-surface
    // addition flagged in the lane log).
    {
        let journal_ids: std::collections::HashSet<Uuid> =
            engine.journal().iter().map(|e| e.id).collect();
        let mut cap_ids: Vec<Uuid> = engine
            .vector_captures()
            .keys()
            .filter(|id| journal_ids.contains(id))
            .copied()
            .collect();
        cap_ids.sort();
        for id in cap_ids {
            let g = &engine.vector_captures()[&id];
            tail.push((
                format!("journal/captures/{id}.sidecar.json"),
                g.sidecar.clone(),
            ));
            let b = g.bounds;
            tail.push((
                format!("journal/captures/{id}.meta.json"),
                canonical_json(&json!({
                    "schema": "appname.vector.capture.v1",
                    "bounds": {"x": b.min.x, "y": b.min.y,
                                "w": b.max.x - b.min.x, "h": b.max.y - b.min.y},
                })),
            ));
        }
    }
    // Asset entries (wave-2 arbitration 4): persist exactly the blobs the
    // journal OR the document references (save-time GC — an asset lives
    // with its referencing history/state), content-addressed uuid names
    // under assets/ (§7.1 layout in uuid form; the byte-lex tail order
    // covers them). Referenced set (fix-round B2 — the old walk missed
    // document-referenced smart-object sources and the nested
    // `params.source.assetId` shape, silently GC-ing live source assets on
    // the save-after-reopen):
    //   1. envelope-level `samples`;
    //   2. params-level `assetId` (edit.paste family) + `assetBefore`/
    //      `assetAfter` (smartobject.edit_contents — replay needs both
    //      sides of the swap);
    //   3. the nested `params.source.assetId` (smartobject.replace);
    //   4. every smart-object source the DOCUMENT references
    //      (document.json `assetHash` refs, §11.5).
    let mut referenced: std::collections::HashSet<Uuid> = engine
        .journal()
        .iter()
        .filter_map(|e| e.samples.as_deref())
        .filter_map(|s| Uuid::parse_str(s).ok())
        .collect();
    for e in engine.journal() {
        let Some(m) = e.params_map() else { continue };
        for key in ["assetId", "assetBefore", "assetAfter"] {
            if let Some(id) = m
                .get(key)
                .and_then(Value::as_str)
                .and_then(|s| Uuid::parse_str(s).ok())
            {
                referenced.insert(id);
            }
        }
        if let Some(id) = m
            .get("source")
            .and_then(Value::as_object)
            .and_then(|s| s.get("assetId"))
            .and_then(Value::as_str)
            .and_then(|s| Uuid::parse_str(s).ok())
        {
            referenced.insert(id);
        }
    }
    for lid in doc.z_order_walk() {
        if let Some(so) = doc.layer(lid).and_then(|n| n.smart_object()) {
            let crate::graph::node::SmartSource::Embedded(asset) = so.source();
            referenced.insert(*asset);
        }
    }
    for id in engine.assets().ids_sorted() {
        if referenced.contains(&id) || engine.assets().is_pending(id) {
            let bytes = engine
                .assets()
                .bytes(id)
                .expect("listed id resolves")
                .to_vec();
            // Entry naming: samples stay FLAT `assets/{uuid}` (byte-compat
            // with every CP-0.3 package incl. the pinned selftest
            // reference); raster images carry the `.image` suffix.
            let name = match engine.assets().kind_of(id).expect("listed id resolves") {
                crate::engine::assets::AssetKind::Samples => format!("assets/{id}"),
                crate::engine::assets::AssetKind::RasterImage => format!("assets/{id}.image"),
                // §23.5 embedded faces (Door 10): `.font` suffix — a
                // package-surface addition under the §7.7 additive rules
                // (flagged in the lane log, `.image` precedent).
                crate::engine::assets::AssetKind::FontFace => format!("assets/{id}.font"),
                // D-ML-1 mask planes: `.mask` suffix (same flagged rule).
                crate::engine::assets::AssetKind::MaskPlane => format!("assets/{id}.mask"),
            };
            tail.push((name, bytes));
        }
    }
    // §7.5 op sidecars (appname.op.v1) + op-mask tiles.
    for lid in doc.z_order_walk() {
        let node = doc.layer(lid).expect("walk resolves");
        let ops: Vec<crate::graph::id::OpId> = node
            .adjustment_op()
            .into_iter()
            .chain(
                node.smart_object()
                    .into_iter()
                    .flat_map(|so| so.smart_filters().iter().copied()),
            )
            .collect();
        for oid in ops {
            let op = doc.op(oid).expect("validated tree");
            let body = json!({
                "schema": "appname.op.v1",
                "kind": op.op(),
                "params": op.params(),
                "mask": op.mask().map(|m| Value::String(m.as_uuid().to_string())).unwrap_or(Value::Null),
                "blend": serde_json::to_value(op.blend()).expect("blend serializes"),
                "opacity": op.opacity(),
                "enabled": op.enabled(),
            });
            tail.push((format!("ops/{}.json", oid.as_uuid()), canonical_json(&body)));
            if let Some(mid) = op.mask() {
                let m = doc.mask(mid).expect("validated tree");
                let buf = m.coverage();
                for (c, tile) in buf.tiles().iter() {
                    let chunk = tile_chunk::write_tile_chunk(
                        c,
                        tile,
                        buf.kind(),
                        buf.model(),
                        buf.premultiplied(),
                    );
                    tail.push((
                        format!(
                            "ops/{}/mask/{}/tiles/{}/{}_{}.tile",
                            oid.as_uuid(),
                            mid.as_uuid(),
                            c.lod,
                            c.x,
                            c.y
                        ),
                        chunk,
                    ));
                }
            }
        }
    }

    // §7.5 embedded ICC profiles: raw bytes at profiles/{id}.icc —
    // re-embedded on every save (round-trip fidelity; builtin/untagged
    // entries carry no bytes and no entry).
    for (pid, entry) in doc.profiles() {
        if let crate::graph::document::DocProfileSource::Icc(bytes) = &entry.source {
            tail.push((format!("profiles/{}.icc", pid.as_uuid()), (**bytes).clone()));
        }
    }
    tail.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));

    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut zw = zip::ZipWriter::new(&mut cursor);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);

        // [NORMATIVE §7.1/§7.6 step 4] fixed head: mimetype (STORED, exact
        // payload), manifest.json, document.json…
        zw.start_file("mimetype", opts)
            .map_err(|e| write_err("starting mimetype", e))?;
        zw.write_all(PACKAGE_MIMETYPE.as_bytes())
            .map_err(|e| write_err("writing mimetype", e))?;

        zw.start_file("manifest.json", opts)
            .map_err(|e| write_err("starting manifest.json", e))?;
        zw.write_all(&canonical_json(&manifest))
            .map_err(|e| write_err("writing manifest.json", e))?;

        zw.start_file("document.json", opts)
            .map_err(|e| write_err("starting document.json", e))?;
        zw.write_all(&canonical_json(&document))
            .map_err(|e| write_err("writing document.json", e))?;

        // …then every remaining entry in the byte-lex total order.
        for (name, bytes) in &tail {
            zw.start_file(name.clone(), opts)
                .map_err(|e| write_err("starting tail entry", e))?;
            zw.write_all(bytes)
                .map_err(|e| write_err("writing tail entry", e))?;
        }

        zw.finish()
            .map_err(|e| write_err("finalizing package", e))?;
    }
    Ok(cursor.into_inner())
}

/// Canonical persisted JSON (§2.8): sorted keys (serde_json's default map
/// is ordered), 2-space indent, LF, UTF-8, trailing newline.
fn canonical_json(v: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(v).expect("JSON serialization cannot fail");
    bytes.push(b'\n');
    bytes
}

fn layer_value(doc: &Document, id: LayerId) -> Result<Value, EngineError> {
    let node = doc
        .layer(id)
        .ok_or_else(|| write_err("serializing layers", format!("layer {id} does not resolve")))?;
    let mut m = Map::new();
    m.insert("id".into(), json!(node.id().as_uuid()));
    m.insert("name".into(), json!(node.name()));
    m.insert("kind".into(), json!(node.kind().kind_str()));
    m.insert(
        "blend".into(),
        serde_json::to_value(node.blend()).expect("blend serializes"),
    );
    m.insert("opacity".into(), json!(node.opacity()));
    m.insert("fillOpacity".into(), json!(node.fill_opacity()));
    m.insert("visible".into(), json!(node.visible()));
    m.insert("clipToBelow".into(), json!(node.clip_to_below()));
    m.insert(
        "transform".into(),
        serde_json::to_value(node.transform()).expect("mat3 serializes"),
    );
    m.insert(
        "masks".into(),
        Value::Array(node.masks().iter().map(|mm| json!(mm.as_uuid())).collect()),
    );
    if !node.locks().is_empty() {
        m.insert(
            "locks".into(),
            serde_json::to_value(node.locks()).expect("locks serialize"),
        );
    }
    if let Some(label) = node.color_label() {
        m.insert(
            "colorLabel".into(),
            serde_json::to_value(label).expect("label serializes"),
        );
    }
    if node.is_group() {
        m.insert(
            "children".into(),
            Value::Array(node.children().iter().map(|c| json!(c.as_uuid())).collect()),
        );
        m.insert("isolated".into(), json!(false));
    } else if let Some(sd) = node.shape_data() {
        // §20.2 shape layer: pathRef (schema-declared, uuid) + fill/stroke
        // payloads verbatim. `boolOps` is the §20.3 non-destructive stack —
        // SCHEMA-GAP flagged (no declared slot in document.v1.json OR the
        // vector sidecar schema; explorer-verified), written absent-when-
        // empty per the crop_rect precedent, fold-in candidate recorded.
        m.insert("pathRef".into(), json!(sd.path().as_uuid()));
        m.insert("fill".into(), sd.fill().clone());
        m.insert("stroke".into(), sd.stroke().clone());
        if !sd.bool_ops().is_empty() {
            m.insert(
                "boolOps".into(),
                Value::Array(
                    sd.bool_ops()
                        .iter()
                        .map(|(op, pid)| json!({"op": op.as_wire(), "pathId": pid.as_uuid()}))
                        .collect(),
                ),
            );
        }
    } else if let Some(td) = node.text_data() {
        // §23.1 text layer: textRef per the document.v1 $defs.layer row
        // ("text layers: text/{layerId}.json (§7.5)" — schema-declared,
        // path-string form). `textBounds` carries the LAID-OUT hull so a
        // text-less reader keeps dirty hulls and the state-hash fold
        // intact across an opaque round-trip — SCHEMA-GAP flagged (no
        // declared slot; the root-"paths" bounds precedent), fold-in
        // candidate recorded in the lane log.
        m.insert(
            "textRef".into(),
            json!(format!("text/{}.json", node.id().as_uuid())),
        );
        let b = td.bounds();
        m.insert(
            "textBounds".into(),
            json!({"x": b.min.x, "y": b.min.y,
                   "w": b.max.x - b.min.x, "h": b.max.y - b.min.y}),
        );
    } else if let Some(fd) = node.fill_data() {
        // §22 fill layer: document.v1 `$defs.layer` makes `fill` REQUIRED
        // when `kind == "fill"` (the allOf row), so omitting it would emit
        // a document our own reader is right to refuse. The WIRE object is
        // written verbatim rather than re-serialized from the decoded
        // content: it is what came in, it round-trips the §7.7 unknown
        // fields a v5.6 reader may need, and re-encoding would have to
        // invent an sRGB round trip back out of linear light. Same posture
        // as the shape layer's fill/stroke above.
        m.insert("fill".into(), fd.wire().clone());
    } else if let Some(op) = node.adjustment_op() {
        // §7.5: adjustment layers reference ops/{opId}.json via opRef.
        m.insert("opRef".into(), json!(op.as_uuid()));
    } else if let Some(so) = node.smart_object() {
        let crate::graph::node::SmartSource::Embedded(asset) = so.source();
        m.insert(
            "smartObject".into(),
            json!({
                "source": {
                    "kind": "embedded",
                    // Schema wants the content hash (assets are §7.4
                    // content-addressed); the loader re-derives the store
                    // uuid from the bytes.
                    "assetHash": format!("blake3:{}", doc_asset_blake3(*asset)?),
                },
                "placeTransform": serde_json::to_value(so.place_transform()).expect("mat3"),
                "smartFilters": so
                    .smart_filters()
                    .iter()
                    .map(|o| json!(o.as_uuid()))
                    .collect::<Vec<_>>(),
            }),
        );
        // The persisted per-instance render cache (§11.5).
        m.insert(
            "tilesRef".into(),
            json!(format!("layers/{}/tiles", node.id().as_uuid())),
        );
    } else {
        m.insert(
            "tilesRef".into(),
            json!(format!("layers/{}/tiles", node.id().as_uuid())),
        );
    }
    Ok(Value::Object(m))
}

// The writer resolves smart-object asset hashes through a thread-local set
// by save_package_bytes before serialization (assets live on the ENGINE,
// not the Document — the §7.4 content-address is a pure function of the
// bytes).
thread_local! {
    static ASSET_HASHES: std::cell::RefCell<std::collections::HashMap<Uuid, String>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

fn doc_asset_blake3(asset: Uuid) -> Result<String, EngineError> {
    ASSET_HASHES
        .with(|m| m.borrow().get(&asset).cloned())
        .ok_or_else(|| {
            write_err(
                "serializing smart object",
                format!("embedded asset {asset} is not registered in the engine store"),
            )
        })
}

// ---------------------------------------------------------------------------
// §7.8 journal entry (in-package form).
// ---------------------------------------------------------------------------

pub(crate) fn record(out: &mut Vec<u8>, rtype: u8, body: &[u8]) {
    let mut payload = Vec::with_capacity(1 + body.len());
    payload.push(rtype);
    payload.extend_from_slice(body);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&integrity::crc32(&payload).to_le_bytes());
    out.extend_from_slice(&payload);
}

fn write_journal(doc_id: DocumentId, journal: &[Envelope], cursor: usize) -> Vec<u8> {
    let mut out = Vec::new();
    // BASE: documentId + savedAt (fixed epoch — determinism, module doc).
    let mut base = Vec::with_capacity(16 + EPOCH_TS.len());
    base.extend_from_slice(doc_id.as_uuid().as_bytes());
    base.extend_from_slice(EPOCH_TS.as_bytes());
    record(&mut out, 0x00, &base);
    // COMMAND per envelope (full stream incl. the redoable tail, so
    // cross-invocation redo survives the round trip).
    for env in journal {
        record(&mut out, 0x01, env.to_json().as_bytes());
    }
    // CHECKPOINT: timestamp + last in-effect CommandId (nil ⇒ cursor 0).
    let last = if cursor > 0 {
        journal[cursor - 1].id
    } else {
        Uuid::nil()
    };
    let mut cp = Vec::with_capacity(EPOCH_TS.len() + 16);
    cp.extend_from_slice(EPOCH_TS.as_bytes());
    cp.extend_from_slice(last.as_bytes());
    record(&mut out, 0x03, &cp);
    out
}

/// Parse the §7.8 journal entry: envelopes + cursor + corrupt-tail
/// truncation warnings (never fatal — the §7.8 rule: a torn tail costs the
/// tail, never the document).
fn read_journal(
    bytes: &[u8],
    manifest_doc_id: Uuid,
    warnings: &mut Vec<TileWarning>,
) -> Result<(Vec<Envelope>, usize), EngineError> {
    let entry = "journal/autosave.log";
    let warn = |warnings: &mut Vec<TileWarning>, detail: String| {
        warnings.push(TileWarning {
            entry: entry.to_string(),
            detail,
        });
    };
    let mut envelopes: Vec<Envelope> = Vec::new();
    let mut cursor_from_checkpoint: Option<usize> = None;
    let mut pos = 0usize;
    loop {
        if pos == bytes.len() {
            break; // clean end
        }
        if bytes.len() - pos < 8 {
            warn(
                &mut *warnings,
                "torn frame header at journal tail (truncated)".into(),
            );
            break;
        }
        let len = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
        let crc = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap());
        if bytes.len() - pos - 8 < len || len == 0 {
            warn(
                &mut *warnings,
                "frame length exceeds remaining bytes (truncated tail)".into(),
            );
            break;
        }
        let payload = &bytes[pos + 8..pos + 8 + len];
        if integrity::crc32(payload) != crc {
            warn(
                &mut *warnings,
                "frame CRC mismatch (truncated at corrupt record)".into(),
            );
            break;
        }
        pos += 8 + len;
        match payload[0] {
            0x00 => {
                // BASE anchors replay to this package's document.
                if payload.len() < 17 {
                    warn(&mut *warnings, "short BASE record (truncated)".into());
                    break;
                }
                let base_id = Uuid::from_bytes(payload[1..17].try_into().unwrap());
                if base_id != manifest_doc_id {
                    return Err(read_err(
                        "validating journal",
                        format!("BASE document id {base_id} does not match the manifest"),
                    ));
                }
            }
            0x01 => {
                let text = std::str::from_utf8(&payload[1..])
                    .map_err(|_| ())
                    .and_then(|t| Envelope::from_json(t).map_err(|_| ()));
                match text {
                    Ok(env) => envelopes.push(env),
                    Err(()) => {
                        warn(
                            &mut *warnings,
                            "unparseable COMMAND record (truncated here)".into(),
                        );
                        break;
                    }
                }
            }
            0x02 => {
                // TILEREF points into scratch extents that a package cannot
                // resolve — §7.8 truncation rule applies.
                warn(
                    &mut *warnings,
                    "TILEREF record in a package journal (truncated here)".into(),
                );
                break;
            }
            0x03 => {
                if payload.len() < 17 {
                    warn(&mut *warnings, "short CHECKPOINT record (ignored)".into());
                    continue;
                }
                let last = Uuid::from_bytes(payload[payload.len() - 16..].try_into().unwrap());
                if last.is_nil() {
                    cursor_from_checkpoint = Some(0);
                } else {
                    match envelopes.iter().position(|e| e.id == last) {
                        Some(ix) => cursor_from_checkpoint = Some(ix + 1),
                        None => warn(
                            &mut *warnings,
                            "CHECKPOINT names an unknown command id (cursor defaults to end)"
                                .into(),
                        ),
                    }
                }
            }
            t @ 0x04..=0x7F => {
                // Reserved-skippable: framing is self-describing.
                warn(
                    &mut *warnings,
                    format!("skipped reserved record type 0x{t:02x}"),
                );
            }
            t => {
                // Reserved-critical: replay stops here.
                warn(
                    &mut *warnings,
                    format!("critical reserved record type 0x{t:02x} ends replay"),
                );
                break;
            }
        }
    }
    let cursor = cursor_from_checkpoint.unwrap_or(envelopes.len());
    Ok((envelopes, cursor))
}

// ---------------------------------------------------------------------------
// Load.
// ---------------------------------------------------------------------------

pub fn load_package(path: &Path) -> Result<LoadedPackage, EngineError> {
    load_package_with(&crate::engine::kind_impl::EngineAssembly::new(), path)
}

/// Load and re-assemble over `assembly` (wave-2 arbitration 1: reopened
/// engines need the same kind assembly to apply/replay assembled kinds).
pub fn load_package_with(
    assembly: &crate::engine::kind_impl::EngineAssembly,
    path: &Path,
) -> Result<LoadedPackage, EngineError> {
    let bytes =
        std::fs::read(path).map_err(|e| read_err(&format!("reading {}", path.display()), e))?;
    load_package_bytes_with(assembly, &bytes)
}

pub fn load_package_bytes(bytes: &[u8]) -> Result<LoadedPackage, EngineError> {
    load_package_bytes_with(&crate::engine::kind_impl::EngineAssembly::new(), bytes)
}

pub fn load_package_bytes_with(
    assembly: &crate::engine::kind_impl::EngineAssembly,
    bytes: &[u8],
) -> Result<LoadedPackage, EngineError> {
    let mut za = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| read_err("opening package", e))?;
    let mut warnings: Vec<TileWarning> = Vec::new();

    // [NORMATIVE §7.1] entry 0 is the mimetype with the exact payload.
    {
        let mut first = za
            .by_index(0)
            .map_err(|e| read_err("reading first entry", e))?;
        if first.name() != "mimetype" {
            return Err(read_err(
                "validating package",
                "first entry is not `mimetype`",
            ));
        }
        let mut mt = String::new();
        first
            .read_to_string(&mut mt)
            .map_err(|e| read_err("reading mimetype", e))?;
        if mt != PACKAGE_MIMETYPE {
            return Err(read_err(
                "validating package",
                format!("unexpected mimetype \"{mt}\""),
            ));
        }
    }

    // manifest.json (§7.2): schema const, uuid, minReader gate, document const.
    let manifest: Value = parse_json_entry(&mut za, "manifest.json")?;
    if manifest.get("schema").and_then(Value::as_str) != Some(MANIFEST_SCHEMA) {
        return Err(read_err(
            "validating manifest.json",
            format!("schema must be \"{MANIFEST_SCHEMA}\""),
        ));
    }
    let doc_id = manifest
        .get("documentId")
        .and_then(Value::as_str)
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| read_err("validating manifest.json", "documentId must be a UUID"))?;
    let min_reader = manifest
        .get("requires")
        .and_then(|r| r.get("minReader"))
        .and_then(Value::as_str)
        .ok_or_else(|| read_err("validating manifest.json", "requires.minReader missing"))?;
    let need = parse_semver(min_reader).ok_or_else(|| {
        read_err(
            "validating manifest.json",
            "requires.minReader is not semver",
        )
    })?;
    if need > READER_VERSION {
        // QA-F9 closure: the §7.7 minReader hard-refusal carries its
        // REGISTERED typed id (`error.unsupported.newer_document`, §51.4 /
        // message_ids.yaml), not the generic read-failure id. Clean
        // refusal: no crash, bytes untouched, versions named.
        return Err(EngineError::new(
            crate::engine::error::ErrorCode::UnsupportedNewerDocument,
            format!(
                "package requires reader {min_reader}; this build reads {}.{}.{} \
                 (§7.7 clean refusal)",
                READER_VERSION.0, READER_VERSION.1, READER_VERSION.2
            ),
        ));
    }
    if manifest.get("document").and_then(Value::as_str) != Some("document.json") {
        return Err(read_err(
            "validating manifest.json",
            "manifest.document must be \"document.json\"",
        ));
    }

    // document.json (§7.3 / document.v1.json).
    let document: Value = parse_json_entry(&mut za, "document.json")?;
    let obj = document
        .as_object()
        .ok_or_else(|| read_err("parsing document.json", "not an object"))?;
    if obj.get("schema").and_then(Value::as_str) != Some(DOCUMENT_SCHEMA) {
        return Err(read_err(
            "validating document.json",
            format!("schema must be \"{DOCUMENT_SCHEMA}\""),
        ));
    }
    let geometry: DocumentGeometry = field_from(obj, "geometry")?;
    let color: ColorState = field_from(obj, "color")?;
    let root = obj
        .get("root")
        .and_then(Value::as_str)
        .and_then(|s| Uuid::parse_str(s).ok())
        .map(LayerId::from_uuid)
        .ok_or_else(|| read_err("parsing document.json", "`root` must be a UUID string"))?;
    let layer_objs = obj
        .get("layers")
        .and_then(Value::as_array)
        .ok_or_else(|| read_err("parsing document.json", "`layers` must be an array"))?;

    // Index layer objects by id; the root entry must exist and be a group.
    let mut by_id: Map<String, Value> = Map::new();
    for item in layer_objs {
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| read_err("parsing document.json", "layer `id` must be a UUID string"))?;
        by_id.insert(id.to_string(), item.clone());
    }
    let root_obj = by_id
        .get(&root.as_uuid().to_string())
        .cloned()
        .ok_or_else(|| {
            read_err(
                "parsing document.json",
                "`root` does not resolve in `layers`",
            )
        })?;
    if root_obj.get("kind").and_then(Value::as_str) != Some("group") {
        return Err(read_err(
            "parsing document.json",
            "root layer must be kind \"group\"",
        ));
    }

    let mut doc = Document::new(DocumentId(doc_id), root, geometry, color)
        .map_err(|e| read_err("rebuilding document", format!("{e:?}")))?;
    // §7.3 guides object (optional; absent = defaults).
    if let Some(g) = obj.get("guides") {
        let parsed: crate::graph::document::Guides = serde_json::from_value(g.clone())
            .map_err(|e| read_err("parsing document.json guides", e))?;
        doc.set_guides(parsed);
    }
    // §7.3 open meta object (optional; absent = empty). Restored
    // verbatim — the §42.4 report and §42.2 `psd.*` carriage blobs are
    // opaque to the loader.
    match obj.get("meta") {
        None => {}
        Some(Value::Object(m)) => {
            *doc.meta_mut() = m.clone();
        }
        Some(_) => {
            return Err(read_err(
                "parsing document.json",
                "`meta` must be an object",
            ));
        }
    }
    // Deterministic profile seeding (doc.new parity) + §7.5 embedded ICC
    // re-registration (content-derived uid; §5.2 validation happens at
    // first CMM use, not load — load stays parse-cheap).
    crate::engine::color_glue::seed_builtin_profiles(&mut doc);
    {
        let icc_names: Vec<String> = (0..za.len())
            .filter_map(|i| za.by_index(i).ok().map(|e| e.name().to_string()))
            .filter(|n| n.starts_with("profiles/") && n.ends_with(".icc"))
            .collect();
        for name in icc_names {
            let id_part = &name["profiles/".len()..name.len() - ".icc".len()];
            let Ok(pid) = Uuid::parse_str(id_part) else {
                warnings.push(TileWarning {
                    entry: name.clone(),
                    detail: "profile entry name is not a uuid (skipped)".into(),
                });
                continue;
            };
            let mut bytes = Vec::new();
            {
                use std::io::Read;
                let mut e = za
                    .by_name(&name)
                    .map_err(|e| read_err("reading profile entry", e))?;
                e.read_to_end(&mut bytes)
                    .map_err(|e| read_err("reading profile entry", e))?;
            }
            let uid = appname_color::ProfileUid::of_icc_bytes(&bytes);
            doc.register_profile(
                crate::graph::id::ProfileId(pid),
                crate::graph::document::DocProfile {
                    uid,
                    source: crate::graph::document::DocProfileSource::Icc(std::sync::Arc::new(
                        bytes,
                    )),
                },
            );
        }
    }

    let mask_objs: Map<String, Value> = match obj.get("masks") {
        None => Map::new(),
        Some(Value::Array(items)) => {
            let mut by_id = Map::new();
            for item in items {
                let id = item.get("id").and_then(Value::as_str).ok_or_else(|| {
                    read_err("parsing document.json", "mask `id` must be a UUID string")
                })?;
                by_id.insert(id.to_string(), item.clone());
            }
            by_id
        }
        Some(_) => {
            return Err(read_err(
                "parsing document.json",
                "`masks` must be an array",
            ))
        }
    };

    // §7.5 op sidecars: root "ops" inventory → ops/{opId}.json bodies
    // (+ per-op mask coverage from the masks array's ops/ tilesRef).
    if let Some(op_list) = obj.get("ops").and_then(Value::as_array) {
        for ov in op_list {
            let oid_s = ov.as_str().ok_or_else(|| {
                read_err(
                    "parsing document.json",
                    "`ops` entries must be UUID strings",
                )
            })?;
            let oid = crate::graph::id::OpId::from_uuid(
                Uuid::parse_str(oid_s)
                    .map_err(|_| read_err("parsing document.json", "op id not a UUID"))?,
            );
            let body_bytes = read_entry(&mut za, &format!("ops/{oid_s}.json"))?;
            let body: Value = serde_json::from_slice(&body_bytes)
                .map_err(|e| read_err("parsing op sidecar", e))?;
            if body.get("schema").and_then(Value::as_str) != Some("appname.op.v1") {
                return Err(read_err(
                    "parsing op sidecar",
                    "schema must be appname.op.v1",
                ));
            }
            let kind = body
                .get("kind")
                .and_then(Value::as_str)
                .ok_or_else(|| read_err("parsing op sidecar", "`kind` required"))?;
            let mut op = crate::graph::op_node::OpNode::new(
                oid,
                kind,
                body.get("params")
                    .cloned()
                    .unwrap_or(Value::Object(Map::new())),
            );
            if let Some(v) = body.get("blend") {
                if let Ok(b) = serde_json::from_value::<BlendMode>(v.clone()) {
                    op.blend = b;
                }
            }
            if let Some(v) = body.get("opacity").and_then(Value::as_f64) {
                op.opacity = v.clamp(0.0, 1.0) as f32;
            }
            if let Some(v) = body.get("enabled").and_then(Value::as_bool) {
                op.enabled = v;
            }
            if let Some(mid_s) = body.get("mask").and_then(Value::as_str) {
                let mid = crate::graph::id::MaskId::from_uuid(
                    Uuid::parse_str(mid_s)
                        .map_err(|_| read_err("parsing op sidecar", "mask id not a UUID"))?,
                );
                let mobj = mask_objs.get(mid_s).ok_or_else(|| {
                    read_err(
                        "parsing op sidecar",
                        format!("op mask {mid_s} missing from `masks`"),
                    )
                })?;
                let tiles_ref = mobj
                    .get("tilesRef")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        read_err("parsing op mask", "raster masks require `tilesRef`")
                    })?;
                let coverage = load_mask_tiles(&mut za, tiles_ref, &color, &mut warnings)?;
                let mask = crate::graph::mask_node::MaskNode::new_raster(mid, coverage);
                doc.attach_op_mask(oid, mask)
                    .map_err(|e| read_err("rebuilding op masks", format!("{e:?}")))?;
                op.mask = Some(mid);
            }
            doc.insert_op(op)
                .map_err(|e| read_err("rebuilding ops", format!("{e:?}")))?;
        }
    }

    // §20 doc-path pass: root "paths" inventory (flagged SCHEMA-GAP wire
    // form) → vector/{pathId}.json sidecars. Load-validate via the
    // assembly's geometry service when one is registered (Door-1
    // validation; typed errors surface); a geometry-less assembly carries
    // sidecars OPAQUELY — bytes verbatim, persisted bounds, content hash
    // recomputed from the bytes (§7.7 lenient-reader posture; every
    // geometry arm on such an engine answers typed NI anyway).
    if let Some(path_list) = obj.get("paths").and_then(Value::as_array) {
        for row in path_list {
            let pid_s = row
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| read_err("parsing document.json", "`paths[].id` required"))?;
            let pid = crate::graph::id::PathId::from_uuid(
                Uuid::parse_str(pid_s)
                    .map_err(|_| read_err("parsing document.json", "path id not a UUID"))?,
            );
            let name = row.get("name").and_then(Value::as_str).map(str::to_string);
            let sidecar = read_entry(&mut za, &format!("vector/{pid_s}.json"))?;
            let (sidecar, bounds) = match assembly.vector_geometry() {
                Some(svc) => {
                    let g = svc.validate_sidecar(&sidecar)?;
                    (g.sidecar, g.bounds)
                }
                None => {
                    let b = row.get("bounds").and_then(Value::as_object);
                    let get = |k: &str| -> i32 {
                        b.and_then(|m| m.get(k))
                            .and_then(Value::as_i64)
                            .unwrap_or(0) as i32
                    };
                    (
                        sidecar,
                        crate::geom::IRect::new(
                            get("x"),
                            get("y"),
                            get("x") + get("w"),
                            get("y") + get("h"),
                        ),
                    )
                }
            };
            let hash = crate::engine::vector_ops::sidecar_content_hash(&sidecar);
            let node = crate::graph::path_node::PathNode::new(
                pid,
                name,
                std::sync::Arc::new(sidecar),
                hash,
                bounds,
            );
            doc.insert_path(node)
                .map_err(|e| read_err("rebuilding paths", format!("{e:?}")))?;
        }
    }

    // §14.1 named-channel pass (door C1; SCHEMA-GAP wire form, the paths
    // precedent): root "channels" inventory in §14.1 list order →
    // channels/{channelId}/tiles trees. §14.2 persistence rule: tiles
    // arrive Gray at U8 (u8-exact planes) or F32; both rebuild into the
    // one §13.1 Gray/f32 coverage representation. The §14.3 `spot` slot
    // round-trips verbatim (§7.7).
    if let Some(chan_list) = obj.get("channels").and_then(Value::as_array) {
        for row in chan_list {
            let cid_s = row
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| read_err("parsing document.json", "`channels[].id` required"))?;
            let cid = crate::graph::id::ChannelId::from_uuid(
                Uuid::parse_str(cid_s)
                    .map_err(|_| read_err("parsing document.json", "channel id not a UUID"))?,
            );
            let name = row
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| read_err("parsing document.json", "`channels[].name` required"))?
                .to_string();
            let tiles_ref = row
                .get("tilesRef")
                .and_then(Value::as_str)
                .ok_or_else(|| read_err("parsing document.json", "`channels[].tilesRef` required"))?
                .to_string();
            let buffer = load_channel_tiles(&mut za, &tiles_ref, &mut warnings)?;
            let node = crate::graph::channel_node::NamedChannel::from_loaded(
                cid,
                name,
                buffer,
                row.get("spot").cloned(),
            );
            doc.insert_channel(node)
                .map_err(|e| read_err("rebuilding channels", format!("{e:?}")))?;
        }
    }

    // §29.2 slice pass (export door 1; SCHEMA-GAP wire form): root
    // "slices" array, order-preserving, presets carried verbatim (§7.3
    // OPEN object).
    if let Some(slice_list) = obj.get("slices").and_then(Value::as_array) {
        let mut rows = Vec::with_capacity(slice_list.len());
        for row in slice_list {
            let parsed: crate::graph::document::SliceRow = serde_json::from_value(row.clone())
                .map_err(|e| read_err("parsing document.json slices", e))?;
            rows.push(parsed);
        }
        *doc.slices_mut() = rows;
    }

    // Depth-first insertion honoring z-order.
    let mut stack: Vec<(LayerId, Vec<String>)> = vec![(root, children_of(&root_obj)?)];
    while let Some((parent, children)) = stack.pop() {
        for (index, child_id) in children.iter().enumerate() {
            let child_obj = by_id
                .get(child_id)
                .ok_or_else(|| {
                    read_err(
                        "parsing document.json",
                        format!("child {child_id} does not resolve"),
                    )
                })?
                .clone();
            let node = build_node(&mut za, &child_obj, &color, &mut warnings)?;
            let id = node.id();
            doc.insert_child(parent, index, node)
                .map_err(|e| read_err("rebuilding document", format!("{e:?}")))?;
            if child_obj.get("kind").and_then(Value::as_str) == Some("group") {
                stack.push((id, children_of(&child_obj)?));
            }
        }
    }
    // Mask nodes: resolve each layer's mask list against document.masks
    // and load the coverage trees (Gray/f32 per §13.1; graceful per-tile
    // §7.4 rules apply through the same chunk path).
    for item in layer_objs {
        let Some(lid) = item
            .get("id")
            .and_then(Value::as_str)
            .and_then(|s| Uuid::parse_str(s).ok())
            .map(LayerId::from_uuid)
        else {
            continue;
        };
        let Some(mask_ids) = item.get("masks").and_then(Value::as_array) else {
            continue;
        };
        for mid_v in mask_ids {
            let mid_s = mid_v.as_str().ok_or_else(|| {
                read_err(
                    "parsing document.json",
                    "layer `masks` entries must be UUID strings",
                )
            })?;
            let mobj = mask_objs.get(mid_s).ok_or_else(|| {
                read_err(
                    "parsing document.json",
                    format!("mask {mid_s} does not resolve in `masks`"),
                )
            })?;
            if mobj.get("kind").and_then(Value::as_str) != Some("raster") {
                return Err(read_err(
                    "parsing layer mask",
                    "mask kinds beyond \"raster\" are not loadable at CP-0.5 (§20 path work)",
                ));
            }
            let mid = crate::graph::id::MaskId::from_uuid(
                Uuid::parse_str(mid_s)
                    .map_err(|_| read_err("parsing document.json", "mask id not a UUID"))?,
            );
            let tiles_ref = mobj
                .get("tilesRef")
                .and_then(Value::as_str)
                .ok_or_else(|| read_err("parsing layer mask", "raster masks require `tilesRef`"))?;
            let expected = format!("layers/{}/masks/{}/tiles", lid.as_uuid(), mid.as_uuid());
            if tiles_ref != expected {
                return Err(read_err(
                    "parsing layer mask",
                    format!("mask tilesRef \"{tiles_ref}\" must be \"{expected}\""),
                ));
            }
            let coverage = load_mask_tiles(&mut za, &expected, &color, &mut warnings)?;
            let mut mask = crate::graph::mask_node::MaskNode::new_raster(mid, coverage);
            if let Some(v) = mobj.get("enabled").and_then(Value::as_bool) {
                mask.enabled = v;
            }
            if let Some(v) = mobj.get("inverted").and_then(Value::as_bool) {
                mask.inverted = v;
            }
            if let Some(v) = mobj.get("density").and_then(Value::as_f64) {
                mask.density = v.clamp(0.0, 1.0) as f32;
            }
            if let Some(v) = mobj.get("feather").and_then(Value::as_f64) {
                mask.feather = v.max(0.0) as f32;
            }
            if let Some(v) = mobj.get("linked").and_then(Value::as_bool) {
                mask.linked = v;
            }
            doc.attach_mask(lid, mask)
                .map_err(|e| read_err("rebuilding masks", format!("{e:?}")))?;
        }
    }
    doc.validate_tree()
        .map_err(|e| read_err("validating document tree", format!("{e:?}")))?;
    // Schema semantic note (§1517): every pathRef MUST resolve within the
    // package — shape base paths and §20.3 bool_ops operands both.
    for lid in doc.z_order_walk() {
        if let Some(sd) = doc.layer(lid).and_then(|n| n.shape_data()) {
            let mut refs = vec![sd.path()];
            refs.extend(sd.bool_ops().iter().map(|(_, p)| *p));
            for pid in refs {
                if doc.path(pid).is_none() {
                    return Err(read_err(
                        "validating document tree",
                        format!(
                            "shape layer {} references path {pid} absent from the package \
                             (document.v1 pathRef rule)",
                            lid.as_uuid()
                        ),
                    ));
                }
            }
        }
    }

    // §7.8 journal entry.
    let journal_bytes = read_entry(&mut za, "journal/autosave.log").unwrap_or_default();
    let (journal, cursor) = read_journal(&journal_bytes, doc_id, &mut warnings)?;

    // Asset entries: content-address-verified on load (a tampered blob
    // degrades gracefully to a warning + skip; a stroke replaying against
    // it then answers typed at its own stage).
    let mut assets = crate::engine::assets::AssetStore::new();
    let asset_names: Vec<String> = (0..za.len())
        .filter_map(|i| za.by_index(i).ok().map(|e| e.name().to_string()))
        .filter(|n| n.starts_with("assets/"))
        .collect();
    for name in asset_names {
        let rest = name.strip_prefix("assets/").expect("prefix-filtered");
        let (id_str, kind) = if let Some(base) = rest.strip_suffix(".image") {
            (base, crate::engine::assets::AssetKind::RasterImage)
        } else if let Some(base) = rest.strip_suffix(".font") {
            (base, crate::engine::assets::AssetKind::FontFace)
        } else if let Some(base) = rest.strip_suffix(".mask") {
            (base, crate::engine::assets::AssetKind::MaskPlane)
        } else {
            (rest, crate::engine::assets::AssetKind::Samples)
        };
        let Some(id) = Uuid::parse_str(id_str).ok() else {
            warnings.push(TileWarning {
                entry: name.clone(),
                detail: "asset entry name is not a uuid (skipped)".into(),
            });
            continue;
        };
        let blob = read_entry(&mut za, &name)?;
        if assets.insert_loaded_kind(id, blob, kind).is_err() {
            warnings.push(TileWarning {
                entry: name.clone(),
                detail: "asset bytes do not match their content-derived id (skipped)".into(),
            });
        }
    }

    // Smart-object sources: map the recorded blake3 hashes onto the loaded
    // store ids (content-addressed both ways — the hash is a pure function
    // of the bytes).
    let pending: std::collections::HashMap<Uuid, String> =
        SMART_SOURCE_HASHES.with(|m| m.borrow_mut().drain().collect());
    if !pending.is_empty() {
        let mut by_hash: std::collections::HashMap<String, Uuid> = std::collections::HashMap::new();
        for id in assets.ids_sorted() {
            if let Some(bytes) = assets.bytes(id) {
                by_hash.insert(blake3::hash(bytes).to_hex().to_string(), id);
            }
        }
        for (layer_uuid, hash) in pending {
            let Some(asset) = by_hash.get(&hash).copied() else {
                return Err(read_err(
                    "resolving smartObject source",
                    format!("no asset entry matches recorded hash blake3:{hash}"),
                ));
            };
            let lid = LayerId::from_uuid(layer_uuid);
            if let Ok(node) = doc.layer_mut(lid) {
                if let Some(so) = node.smart_object_mut() {
                    so.source = crate::graph::node::SmartSource::Embedded(asset);
                }
            }
        }
    }

    // TEXTVEC Door 10 (§44.2 step 2): register document-embedded faces
    // into the text service's FontDb BEFORE any layout validation — fonts
    // are layout input. §44.1 [NORMATIVE] "opening never blocks on fonts":
    // a face the service refuses degrades to a warning, never an error.
    if let Some(svc) = assembly.text_layout() {
        for id in assets.ids_sorted() {
            if assets.kind_of(id) == Some(crate::engine::assets::AssetKind::FontFace) {
                let bytes = assets.bytes(id).expect("sorted ids resolve");
                if let Err(e) = svc.register_embedded_face(id, bytes) {
                    warnings.push(TileWarning {
                        entry: format!("assets/{id}.font"),
                        detail: format!(
                            "embedded face refused by the text service ({e}) — \
                             §44.2 fallback covers affected runs (§44.1 open flow)"
                        ),
                    });
                }
            }
        }
        // §23.1 text pass (the vector Door-1 loader mirror): re-validate
        // every text sidecar through the Door-6 service; normalized bytes
        // + service-derived laid-out bounds become authoritative. Typed
        // errors surface (structurally invalid sidecars are load errors —
        // missing FONTS are not errors, §23.5, and never reach here as
        // such).
        for lid in doc.z_order_walk() {
            let Some(td) = doc.layer(lid).and_then(|n| n.text_data()).cloned() else {
                continue;
            };
            let r = svc.validate_sidecar(td.sidecar())?;
            let hash = crate::engine::text_ops::text_content_hash(&r.sidecar);
            doc.apply_text_data_block(
                LayerId::from_uuid(lid.as_uuid()),
                &crate::graph::node::TextLayerData::new(
                    std::sync::Arc::new(r.sidecar),
                    hash,
                    r.bounds,
                ),
            )
            .map_err(|e| read_err("validating text layers", format!("{e:?}")))?;
        }
    }

    let mut engine = Engine::from_loaded_parts(assembly.clone(), doc, journal, cursor, assets);
    // D-48 vector value-captures: re-enter journal captures so replay
    // consumes them (never re-runs constructor trig).
    let cap_names: Vec<String> = (0..za.len())
        .filter_map(|i| za.by_index(i).ok().map(|e| e.name().to_string()))
        .filter(|n| n.starts_with("journal/captures/") && n.ends_with(".meta.json"))
        .collect();
    for meta_name in cap_names {
        let id_part = meta_name
            .trim_start_matches("journal/captures/")
            .trim_end_matches(".meta.json");
        let Ok(env_id) = Uuid::parse_str(id_part) else {
            warnings.push(TileWarning {
                entry: meta_name.clone(),
                detail: "capture entry name is not a UUID — skipped (§7.7 lenient)".into(),
            });
            continue;
        };
        let meta_bytes = read_entry(&mut za, &meta_name)?;
        let meta: Value =
            serde_json::from_slice(&meta_bytes).map_err(|e| read_err("parsing capture meta", e))?;
        if meta.get("schema").and_then(Value::as_str) != Some("appname.vector.capture.v1") {
            return Err(read_err(
                "parsing capture meta",
                "schema must be appname.vector.capture.v1",
            ));
        }
        let b = meta.get("bounds").and_then(Value::as_object);
        let get = |k: &str| -> i32 {
            b.and_then(|m| m.get(k))
                .and_then(Value::as_i64)
                .unwrap_or(0) as i32
        };
        let bounds =
            crate::geom::IRect::new(get("x"), get("y"), get("x") + get("w"), get("y") + get("h"));
        let sidecar = read_entry(&mut za, &format!("journal/captures/{id_part}.sidecar.json"))?;
        engine.insert_loaded_vector_capture(
            env_id,
            crate::engine::vector_ops::GeometryResult { sidecar, bounds },
        );
    }
    Ok(LoadedPackage { engine, warnings })
}

fn children_of(obj: &Value) -> Result<Vec<String>, EngineError> {
    match obj.get("children") {
        None => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| {
                v.as_str().map(str::to_string).ok_or_else(|| {
                    read_err(
                        "parsing document.json",
                        "`children` entries must be UUID strings",
                    )
                })
            })
            .collect(),
        Some(_) => Err(read_err(
            "parsing document.json",
            "`children` must be an array",
        )),
    }
}

thread_local! {
    /// Loader side-channel: smartObject layer id → recorded blake3 hex
    /// (resolved to a store AssetId after the asset pass).
    static SMART_SOURCE_HASHES: std::cell::RefCell<std::collections::HashMap<Uuid, String>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

fn build_node(
    za: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>,
    obj: &Value,
    color: &ColorState,
    warnings: &mut Vec<TileWarning>,
) -> Result<LayerNode, EngineError> {
    let id = obj
        .get("id")
        .and_then(Value::as_str)
        .and_then(|s| Uuid::parse_str(s).ok())
        .map(LayerId::from_uuid)
        .ok_or_else(|| read_err("parsing layer", "`id` must be a UUID string"))?;
    let name = obj
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let kind = obj
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| read_err("parsing layer", "`kind` must be a string"))?;

    let mut node = match kind {
        "group" => LayerNode::new_group(id, name, false),
        "raster" => {
            let tiles_ref = obj
                .get("tilesRef")
                .and_then(Value::as_str)
                .ok_or_else(|| read_err("parsing layer", "raster layers require `tilesRef`"))?;
            let buffer = load_layer_tiles(za, id, tiles_ref, color, warnings)?;
            LayerNode::new_raster(id, name, buffer)
        }
        "adjustment" => {
            // §7.5: the op body lives at ops/{opId}.json; the layer carries
            // only the reference. The ops pass (below) fills the arena.
            let op = obj
                .get("opRef")
                .and_then(Value::as_str)
                .and_then(|u| Uuid::parse_str(u).ok())
                .ok_or_else(|| read_err("parsing layer", "adjustment layers require `opRef`"))?;
            LayerNode::new_adjustment(id, name, crate::graph::id::OpId::from_uuid(op))
        }
        "smartObject" => {
            let so = obj
                .get("smartObject")
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    read_err("parsing layer", "smartObject layers require `smartObject`")
                })?;
            let source = so
                .get("source")
                .and_then(Value::as_object)
                .ok_or_else(|| read_err("parsing smartObject", "`source` required"))?;
            match source.get("kind").and_then(Value::as_str) {
                Some("embedded") => {}
                Some("linked") => {
                    return Err(read_err(
                        "parsing smartObject",
                        "linked sources are not loadable at CP-0.7 wave 1 (drift detection is                          shell-side; typed-pending)",
                    ))
                }
                _ => return Err(read_err("parsing smartObject", "unknown `source.kind`")),
            }
            let hash = source
                .get("assetHash")
                .and_then(Value::as_str)
                .and_then(|h| h.strip_prefix("blake3:"))
                .ok_or_else(|| read_err("parsing smartObject", "`assetHash` must be blake3:<hex>"))?
                .to_string();
            SMART_SOURCE_HASHES.with(|m| m.borrow_mut().insert(id.as_uuid(), hash));
            let place: crate::geom::Mat3 = so
                .get("placeTransform")
                .map(|v| serde_json::from_value(v.clone()))
                .transpose()
                .map_err(|e| read_err("parsing smartObject", format!("placeTransform: {e}")))?
                .unwrap_or(crate::geom::Mat3::IDENTITY);
            let filters: Vec<crate::graph::id::OpId> = so
                .get("smartFilters")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .filter_map(|u| Uuid::parse_str(u).ok())
                        .map(crate::graph::id::OpId::from_uuid)
                        .collect()
                })
                .unwrap_or_default();
            let tiles_ref = obj.get("tilesRef").and_then(Value::as_str).ok_or_else(|| {
                read_err("parsing layer", "smartObject layers persist `tilesRef`")
            })?;
            let cached = load_layer_tiles(za, id, tiles_ref, color, warnings)?;
            LayerNode::new_smart_object(
                id,
                name,
                crate::graph::node::SmartObjectData {
                    // Placeholder until the asset pass maps hash → store id.
                    source: crate::graph::node::SmartSource::Embedded(Uuid::nil()),
                    cached_composite: cached,
                    smart_filters: filters,
                    place_transform: place,
                },
            )
        }
        "fill" => {
            // §22 fill layer. `fill` is schema-REQUIRED for this kind, so
            // its absence is a read error, not a defaultable field. The
            // payload re-enters through the SAME registry the command door
            // uses (`decode_fill_content`) so a hand-edited or
            // foreign-writer package cannot install content the command
            // path would have refused — one validation gate, two doors.
            let raw = obj
                .get("fill")
                .cloned()
                .ok_or_else(|| read_err("parsing layer", "fill layers require `fill`"))?;
            let content = crate::engine::fill_reg::decode_fill_content(&raw, "fill")
                .map_err(|e| read_err("parsing layer", format!("`fill`: {}", e.message)))?;
            LayerNode::new_fill(
                id,
                name,
                crate::graph::node::FillLayerData::new(content, raw),
            )
        }
        "shape" => {
            // §20.2 shape layer: pathRef + fill/stroke carried verbatim
            // (engine-opaque payloads); boolOps per the flagged SCHEMA-GAP
            // wire form. The path itself loads in the paths pass — the
            // reference is validated there (schema §1517: every pathRef
            // MUST resolve within the package).
            let path = obj
                .get("pathRef")
                .and_then(Value::as_str)
                .and_then(|u| Uuid::parse_str(u).ok())
                .map(crate::graph::id::PathId::from_uuid)
                .ok_or_else(|| read_err("parsing layer", "shape layers require `pathRef`"))?;
            let fill = obj.get("fill").cloned().unwrap_or(Value::Null);
            let stroke = obj.get("stroke").cloned().unwrap_or(Value::Null);
            let mut bool_ops = Vec::new();
            if let Some(rows) = obj.get("boolOps").and_then(Value::as_array) {
                for row in rows {
                    let op = row
                        .get("op")
                        .and_then(Value::as_str)
                        .and_then(crate::engine::vector_ops::VectorBoolOp::from_wire)
                        .ok_or_else(|| {
                            read_err("parsing layer", "`boolOps[].op` must be a §20.3 operator")
                        })?;
                    let pid = row
                        .get("pathId")
                        .and_then(Value::as_str)
                        .and_then(|u| Uuid::parse_str(u).ok())
                        .map(crate::graph::id::PathId::from_uuid)
                        .ok_or_else(|| {
                            read_err("parsing layer", "`boolOps[].pathId` must be a UUID")
                        })?;
                    bool_ops.push((op, pid));
                }
            }
            LayerNode::new_shape(
                id,
                name,
                crate::graph::node::ShapeLayerData {
                    path,
                    fill,
                    stroke,
                    bool_ops,
                },
            )
        }
        "text" => {
            // §23.1 text layer (TEXTVEC Door 7): textRef per the schema
            // row, sidecar carried OPAQUE at parse time — the text pass
            // after tree build re-validates via the Door-6 service when
            // one is registered (mirrors the vector Door-1 loader rule).
            let text_ref = obj
                .get("textRef")
                .and_then(Value::as_str)
                .ok_or_else(|| read_err("parsing layer", "text layers require `textRef`"))?;
            let expected = format!("text/{}.json", id.as_uuid());
            if text_ref != expected {
                return Err(read_err(
                    "parsing layer",
                    format!("text textRef \"{text_ref}\" must be \"{expected}\""),
                ));
            }
            let sidecar = read_entry(za, text_ref)?;
            let b = obj.get("textBounds").and_then(Value::as_object);
            let get = |k: &str| -> i32 {
                b.and_then(|m| m.get(k))
                    .and_then(Value::as_i64)
                    .unwrap_or(0) as i32
            };
            let bounds = crate::geom::IRect::new(
                get("x"),
                get("y"),
                get("x") + get("w"),
                get("y") + get("h"),
            );
            let hash = crate::engine::text_ops::text_content_hash(&sidecar);
            LayerNode::new_text(
                id,
                name,
                crate::graph::node::TextLayerData::new(std::sync::Arc::new(sidecar), hash, bounds),
            )
        }
        other => {
            // §7.7 worked scenario 3 [NORMATIVE] wants unknown kinds
            // preserved and rendered from their `tilesRef` composite, with
            // byte-identical re-emission of the raw JSON + entries. That
            // fallback needs the unknown-preserving LayerKind variant AND
            // the raw-copy `PackageIndex` path (§7.6), neither of which is
            // cheap on the CP-0.1 structures — kept as a TYPED refusal and
            // recorded in the lane-log fold-in table with the §7.7 citation
            // (resolution point: the CP-0.5 document-format wave, where the
            // §7.7 compat report lands; review minor 4a).
            return Err(read_err(
                "parsing layer",
                format!(
                    "layer kind \"{other}\" is not loadable at CP-0.1 (the §7.7 \
                     unknown-kind fallback arrives with the compat-report work)"
                ),
            ));
        }
    };

    // Optional §2.6 fields (defaults per the table).
    if let Some(v) = obj.get("blend") {
        let maybe: Maybe<BlendMode> = serde_json::from_value(v.clone())
            .map_err(|e| read_err("parsing layer", format!("`blend`: {e}")))?;
        // Unknown blend composites as Normal (§2.8); the raw value would
        // round-trip via the Maybe wrapper when layer props persist —
        // CP-0.1 keeps the known value or the default.
        if let Some(b) = maybe.known() {
            node.blend = *b;
        }
    }
    // Out-of-range numerics: readers are LENIENT (review minor 4 reading —
    // document.v1.json x-spec-note: "Validation is strict for WRITERS;
    // readers apply the lenient rules of §2.8/§7.7", and §6.4's readers
    // "sanitize violations on load"): clamp into range with a recorded
    // warning, uniformly for opacity and fillOpacity.
    let clamped_opacity = |name: &str, raw: f64, warnings: &mut Vec<TileWarning>| -> f32 {
        let v = raw.clamp(0.0, 1.0);
        if v != raw {
            warnings.push(TileWarning {
                entry: format!("document.json layer {}", id.as_uuid()),
                detail: format!(
                    "`{name}` {raw} out of [0,1] — clamped to {v} (§7.7 lenient reader)"
                ),
            });
        }
        v as f32
    };
    if let Some(v) = obj.get("opacity").and_then(Value::as_f64) {
        node.opacity = clamped_opacity("opacity", v, warnings);
    }
    if let Some(v) = obj.get("fillOpacity").and_then(Value::as_f64) {
        node.fill_opacity = clamped_opacity("fillOpacity", v, warnings);
    }
    if let Some(v) = obj.get("visible").and_then(Value::as_bool) {
        node.visible = v;
    }
    if let Some(v) = obj.get("clipToBelow").and_then(Value::as_bool) {
        node.clip_to_below = v;
    }
    if let Some(v) = obj.get("transform") {
        node.transform = serde_json::from_value(v.clone())
            .map_err(|e| read_err("parsing layer", format!("`transform`: {e}")))?;
    }
    if let Some(v) = obj.get("locks") {
        node.locks = serde_json::from_value::<LockFlags>(v.clone())
            .map_err(|e| read_err("parsing layer", format!("`locks`: {e}")))?;
    }
    // q-tutorial/Q-9 (MAJOR, reproduced by O-WATCH #906 and again here):
    // the writer above emits `colorLabel` and this reader did not take it
    // back, so one load+save DROPPED it while `visible` survived. It is a
    // KNOWN, contract-declared (layer.set_props), ABI-published field, and
    // the engine was preserving UNKNOWN fields verbatim while losing this
    // one — the asymmetry #906 named. Read beside `locks` because the
    // contract treats the pair as siblings (both emit T only, §40.3).
    //
    // A malformed value is a §7.7 lenient-reader warning, NOT a refusal:
    // dropping the label again is the very defect, and refusing the whole
    // document over one cosmetic enum would be worse than either.
    if let Some(v) = obj.get("colorLabel") {
        match serde_json::from_value::<ColorLabel>(v.clone()) {
            Ok(label) => node.color_label = Some(label),
            Err(e) => warnings.push(TileWarning {
                entry: format!("document.json layer {}", id.as_uuid()),
                detail: format!("`colorLabel` {v} is not a declared label ({e}) — dropped"),
            }),
        }
    }
    Ok(node)
}

fn load_layer_tiles(
    za: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>,
    layer: LayerId,
    tiles_ref: &str,
    color: &ColorState,
    warnings: &mut Vec<TileWarning>,
) -> Result<Buffer, EngineError> {
    // §7.1 entry-name hygiene: the prefix must be the canonical layer path
    // (zip-slip defense: fixed alphabet, no traversal).
    let expected = format!("layers/{}/tiles", layer.as_uuid());
    if tiles_ref != expected {
        return Err(read_err(
            "parsing layer",
            format!("tilesRef \"{tiles_ref}\" must be \"{expected}\""),
        ));
    }
    let prefix = format!("{expected}/");
    let names: Vec<String> = (0..za.len())
        .filter_map(|i| za.by_index(i).ok().map(|e| e.name().to_string()))
        .filter(|n| n.starts_with(&prefix))
        .collect();

    let mut parsed = Vec::new();
    for name in names {
        let bytes = read_entry(za, &name)?;
        match tile_chunk::read_tile_chunk(&name, &bytes) {
            ChunkOutcome::Ok(chunk) => {
                // §7.4 rule 5: path coords vs header — header authoritative,
                // mismatch recorded.
                if let Some((px, py, plod)) = parse_tile_path(&prefix, &name) {
                    if (chunk.coord.x, chunk.coord.y, chunk.coord.lod) != (px, py, plod) {
                        warnings.push(TileWarning {
                            entry: name.clone(),
                            detail: "path coordinates disagree with the chunk header (header wins)"
                                .into(),
                        });
                    }
                }
                parsed.push(chunk);
            }
            ChunkOutcome::Transparent(w) => warnings.push(w),
        }
    }

    // Buffer format: from the chunks (per-layer depth may differ from the
    // document default — imported layers are f32); mixed formats within one
    // layer degrade gracefully per tile.
    let (kind, model, premult) = match parsed.first() {
        Some(c) => (c.kind, c.model, c.premultiplied),
        None => (color.depth, color.mode, true),
    };
    let mut buffer = Buffer::new(kind, model, color.working);
    if !premult {
        buffer.set_premultiplied_loaded(false);
    }
    // §6.4 [NORMATIVE] reader-sanitize (CP-0.5 scheduled item): for
    // premultiplied storage, wherever a = 0 stored color MUST be zero, and
    // unsigned-normalized depths additionally clamp rgb ≤ a per channel.
    // Violations are fixed on load with a recorded warning; at f32 only the
    // a = 0 rule applies (rgb > a is legal HDR premult). CmykA's alpha sits
    // beyond the 4-slot window and defers with the validators (§14).
    if premult && model.has_alpha() && model.alpha_index().map(|i| i < 4).unwrap_or(false) {
        let alpha_ix = model.alpha_index().expect("checked");
        let unorm = kind.is_unsigned_normalized();
        for chunk in parsed.iter_mut() {
            let mut fixed = 0usize;
            for y in 0..crate::tile::TILE {
                for x in 0..crate::tile::TILE {
                    let mut px = chunk.tile.read_pixel_f32(x, y, model);
                    let a = px[alpha_ix];
                    let mut dirty = false;
                    for (c, slot) in px.iter_mut().enumerate().take(model.channels().min(4)) {
                        if c == alpha_ix {
                            continue;
                        }
                        if a == 0.0 && *slot != 0.0 {
                            *slot = 0.0;
                            dirty = true;
                        } else if unorm && *slot > a {
                            *slot = a;
                            dirty = true;
                        }
                    }
                    if dirty {
                        chunk.tile.write_pixel_f32(x, y, px);
                        fixed += 1;
                    }
                }
            }
            if fixed > 0 {
                warnings.push(TileWarning {
                    entry: format!(
                        "layers/{}/tiles/{}/{}_{}.tile",
                        layer.as_uuid(),
                        chunk.coord.lod,
                        chunk.coord.x,
                        chunk.coord.y
                    ),
                    detail: format!(
                        "{fixed} pixel(s) violated the §6.4 premultiplied rules — sanitized on                          load (a=0 ⇒ rgb=0; unorm rgb ≤ a)"
                    ),
                });
            }
        }
    }
    for chunk in parsed {
        if (chunk.kind, chunk.model) != (kind, model) {
            warnings.push(TileWarning {
                entry: format!(
                    "layers/{}/tiles ({:?},{:?})",
                    layer.as_uuid(),
                    chunk.coord.x,
                    chunk.coord.y
                ),
                detail: "chunk format differs from the layer's established format (tile skipped)"
                    .into(),
            });
            continue;
        }
        buffer.install_tile(chunk.coord, std::sync::Arc::new(chunk.tile));
    }
    // Bounds are derived state (§2.4): recompute pixel-exact.
    buffer.recompute_bounds_pixel_exact();
    Ok(buffer)
}

/// Mask coverage loader: same chunk pipeline as layers, Gray/f32 expected
/// (§13.1); other chunk formats degrade gracefully per tile.
fn load_mask_tiles(
    za: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>,
    tiles_ref: &str,
    color: &ColorState,
    warnings: &mut Vec<TileWarning>,
) -> Result<Buffer, EngineError> {
    let prefix = format!("{tiles_ref}/");
    let names: Vec<String> = (0..za.len())
        .filter_map(|i| za.by_index(i).ok().map(|e| e.name().to_string()))
        .filter(|n| n.starts_with(&prefix))
        .collect();
    let mut buffer = crate::mask::Selection::new_mask_buffer();
    let _ = color;
    for name in names {
        let bytes = read_entry(za, &name)?;
        match tile_chunk::read_tile_chunk(&name, &bytes) {
            ChunkOutcome::Ok(chunk) => {
                if (chunk.kind, chunk.model)
                    != (
                        crate::tile::channel::ChannelKind::F32,
                        crate::tile::model::ColorModelKind::Gray,
                    )
                {
                    warnings.push(TileWarning {
                        entry: name.clone(),
                        detail: "mask chunk is not Gray/f32 (§13.1) — tile skipped".into(),
                    });
                    continue;
                }
                buffer.install_tile(chunk.coord, std::sync::Arc::new(chunk.tile));
            }
            ChunkOutcome::Transparent(w) => warnings.push(w),
        }
    }
    buffer.recompute_bounds_pixel_exact();
    Ok(buffer)
}

/// §14.2 channel coverage loader: Gray chunks at U8 (u8-exact planes,
/// the persistence election) or F32 — both rebuild into the §13.1
/// Gray/f32 representation; other formats degrade gracefully per tile.
fn load_channel_tiles(
    za: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>,
    tiles_ref: &str,
    warnings: &mut Vec<TileWarning>,
) -> Result<Buffer, EngineError> {
    let prefix = format!("{tiles_ref}/");
    let names: Vec<String> = (0..za.len())
        .filter_map(|i| za.by_index(i).ok().map(|e| e.name().to_string()))
        .filter(|n| n.starts_with(&prefix))
        .collect();
    let mut buffer = crate::mask::Selection::new_mask_buffer();
    for name in names {
        let bytes = read_entry(za, &name)?;
        match tile_chunk::read_tile_chunk(&name, &bytes) {
            ChunkOutcome::Ok(chunk) => {
                if chunk.model != crate::tile::model::ColorModelKind::Gray {
                    warnings.push(TileWarning {
                        entry: name.clone(),
                        detail: "channel chunk is not Gray (§14.2) — tile skipped".into(),
                    });
                    continue;
                }
                match chunk.kind {
                    crate::tile::channel::ChannelKind::F32 => {
                        buffer.install_tile(chunk.coord, std::sync::Arc::new(chunk.tile));
                    }
                    crate::tile::channel::ChannelKind::U8 => {
                        // u8-exact plane: decode into the f32 buffer
                        // (values are exact k/255 by construction).
                        let rect = chunk.coord.pixel_rect();
                        for y in 0..crate::tile::TILE {
                            for x in 0..crate::tile::TILE {
                                let v = chunk.tile.read_pixel_f32(
                                    x,
                                    y,
                                    crate::tile::model::ColorModelKind::Gray,
                                )[0];
                                if v != 0.0 {
                                    buffer.write_pixel_f32(
                                        crate::geom::IVec2::new(
                                            rect.min.x + x as i32,
                                            rect.min.y + y as i32,
                                        ),
                                        [v, 0.0, 0.0, 0.0],
                                    );
                                }
                            }
                        }
                    }
                    _ => {
                        warnings.push(TileWarning {
                            entry: name.clone(),
                            detail: "channel chunk depth is not U8/F32 (§14.2) — tile skipped"
                                .into(),
                        });
                        continue;
                    }
                }
            }
            ChunkOutcome::Transparent(w) => warnings.push(w),
        }
    }
    buffer.recompute_bounds_pixel_exact();
    Ok(buffer)
}

fn parse_tile_path(prefix: &str, name: &str) -> Option<(i32, i32, u8)> {
    let rest = name.strip_prefix(prefix)?; // "{lod}/{x}_{y}.tile"
    let (lod_s, xy) = rest.split_once('/')?;
    let xy = xy.strip_suffix(".tile")?;
    let (x_s, y_s) = xy.split_once('_')?;
    Some((x_s.parse().ok()?, y_s.parse().ok()?, lod_s.parse().ok()?))
}

fn parse_semver(s: &str) -> Option<(u32, u32, u32)> {
    let mut it = s.split('.');
    let maj = it.next()?.parse().ok()?;
    let min = it.next()?.parse().ok()?;
    let pat = it.next()?.parse().ok()?;
    if it.next().is_some() {
        return None;
    }
    Some((maj, min, pat))
}

fn field_from<T: serde::de::DeserializeOwned>(
    obj: &Map<String, Value>,
    key: &str,
) -> Result<T, EngineError> {
    let v = obj
        .get(key)
        .ok_or_else(|| read_err("parsing document.json", format!("`{key}` is required")))?;
    serde_json::from_value(v.clone())
        .map_err(|e| read_err("parsing document.json", format!("`{key}`: {e}")))
}

fn read_entry(
    za: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>,
    name: &str,
) -> Result<Vec<u8>, EngineError> {
    let mut entry = za
        .by_name(name)
        .map_err(|e| read_err(&format!("opening package entry {name}"), e))?;
    let mut out = Vec::new();
    entry
        .read_to_end(&mut out)
        .map_err(|e| read_err(&format!("reading package entry {name}"), e))?;
    Ok(out)
}

fn parse_json_entry(
    za: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>,
    name: &str,
) -> Result<Value, EngineError> {
    let bytes = read_entry(za, name)?;
    serde_json::from_slice(&bytes).map_err(|e| read_err(&format!("parsing {name}"), e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::error::ErrorCode;
    use crate::tile::model::ColorModelKind;
    use serde_json::json;

    fn doc_new_engine(id_tail: u32, params: Value) -> Engine {
        let mut e = Engine::new();
        let text = json!({
            "schema": "appname.command.v1",
            "id": format!("c0000000-0000-4000-8000-0000000{id_tail:05x}"),
            "kind": "doc.new",
            "params": params
        })
        .to_string();
        e.apply_json(&text).unwrap();
        e
    }

    /// Rebuild a package with one entry's bytes transformed (test harness
    /// for malformed-package cases).
    fn repackage(bytes: &[u8], target: &str, f: impl Fn(Vec<u8>) -> Option<Vec<u8>>) -> Vec<u8> {
        let mut za = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
        for i in 0..za.len() {
            let mut e = za.by_index(i).unwrap();
            let mut data = Vec::new();
            e.read_to_end(&mut data).unwrap();
            entries.push((e.name().to_string(), data));
        }
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut zw = zip::ZipWriter::new(&mut cursor);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            for (name, data) in entries {
                let data = if name == target {
                    match f(data) {
                        Some(d) => d,
                        None => continue, // drop the entry
                    }
                } else {
                    data
                };
                zw.start_file(name, opts).unwrap();
                zw.write_all(&data).unwrap();
            }
            zw.finish().unwrap();
        }
        cursor.into_inner()
    }

    #[test]
    fn round_trip_reproduces_state_hash_and_is_byte_stable() {
        let engine = doc_new_engine(
            1,
            json!({"width": 700, "height": 300, "background": "black"}),
        );
        let b1 = save_package_bytes(&engine).unwrap();
        let loaded = load_package_bytes(&b1).unwrap();
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        assert_eq!(
            loaded.engine.state_hash(),
            engine.state_hash(),
            "save→load identity"
        );
        // save→load→save is byte-stable (deterministic writer, §7.6 spirit).
        let b2 = save_package_bytes(&loaded.engine).unwrap();
        assert_eq!(b1, b2, "byte-stable round trip");
    }

    #[test]
    fn import_meta_survives_the_package_round_trip() {
        // §42.4 [NORMATIVE]: the report persists under `meta["compatReport"]`,
        // "survives the save to `.appname`, travels with the document".
        let mut engine = doc_new_engine(21, json!({"width": 16, "height": 16}));
        let report = json!({
            "schema": "appname.compatReport.v1",
            "outcome": "imported",
            "items": [],
            "summary": {}
        });
        engine
            .set_import_meta("compatReport", report.clone())
            .unwrap();
        engine
            .set_import_meta("psd.duotone", json!("b64:AAEC"))
            .unwrap();

        let b1 = save_package_bytes(&engine).unwrap();
        // The wire carries the §7.3 open meta object verbatim.
        let mut za = zip::ZipArchive::new(std::io::Cursor::new(b1.as_slice())).unwrap();
        let document: Value = parse_json_entry(&mut za, "document.json").unwrap();
        assert_eq!(document["meta"]["compatReport"], report);

        // Survives the reopen, and the reopened save is byte-stable.
        let loaded = load_package_bytes(&b1).unwrap();
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        let meta = loaded.engine.document().unwrap().meta();
        assert_eq!(meta, engine.document().unwrap().meta());
        assert_eq!(meta.get("compatReport"), Some(&report));
        let b2 = save_package_bytes(&loaded.engine).unwrap();
        assert_eq!(b1, b2, "byte-stable round trip with meta");

        // Travels with the document (§42.4): the † duplicate carries it.
        let dup = engine.duplicate().unwrap();
        assert_eq!(
            dup.document().unwrap().meta(),
            engine.document().unwrap().meta()
        );

        // The RECORDED posture (lane log, CP-0.7 boundary round): import
        // provenance is boundary state — a journal replay reconstructs
        // edits, not provenance, and state_hash does not fold meta.
        let replayed = engine.replay_assembled(engine.journal()).unwrap();
        assert!(replayed.document().unwrap().meta().is_empty());
        assert_eq!(replayed.state_hash(), engine.state_hash());
    }

    #[test]
    fn meta_free_packages_stay_byte_identical() {
        // The open meta object is emitted only when non-empty — every
        // pre-existing package keeps its bytes.
        let engine = doc_new_engine(22, json!({"width": 16, "height": 16}));
        let bytes = save_package_bytes(&engine).unwrap();
        let mut za = zip::ZipArchive::new(std::io::Cursor::new(bytes.as_slice())).unwrap();
        let document: Value = parse_json_entry(&mut za, "document.json").unwrap();
        assert!(document.get("meta").is_none(), "no meta key when empty");
    }

    #[test]
    fn import_meta_door_refuses_registered_keys_and_uninitialized_docs() {
        // §10.3 registered keys are `meta.set` envelope territory (D-48).
        let mut engine = doc_new_engine(23, json!({"width": 8, "height": 8}));
        let err = engine.set_import_meta("title", json!("x")).unwrap_err();
        assert!(
            err.to_string().contains("meta.set"),
            "refusal names the envelope route: {err}"
        );
        // No document, no meta (add_icc_profile posture).
        let mut bare = Engine::new();
        assert!(bare.set_import_meta("compatReport", json!({})).is_err());
    }

    #[test]
    fn non_object_meta_is_a_typed_load_error() {
        let engine = doc_new_engine(24, json!({"width": 8, "height": 8}));
        let bytes = save_package_bytes(&engine).unwrap();
        let bad = repackage(&bytes, "document.json", |data| {
            let mut v: Value = serde_json::from_slice(&data).unwrap();
            v["meta"] = json!("not an object");
            Some(v.to_string().into_bytes())
        });
        let err = load_package_bytes(&bad).unwrap_err();
        assert!(
            err.to_string().contains("`meta` must be an object"),
            "typed refusal: {err}"
        );
    }

    #[test]
    fn replay_from_a_reopened_package_reproduces_state_hash() {
        // The dispatch's §7.8 requirement, at the core layer.
        let engine = doc_new_engine(2, json!({"width": 64, "height": 64}));
        let bytes = save_package_bytes(&engine).unwrap();
        let loaded = load_package_bytes(&bytes).unwrap();
        let replayed = loaded
            .engine
            .replay_assembled(loaded.engine.journal())
            .unwrap();
        assert_eq!(replayed.state_hash(), loaded.engine.state_hash());
        assert_eq!(loaded.engine.journal_cursor(), 1);
    }

    #[test]
    fn mimetype_is_first_and_stored_in_the_raw_zip_bytes() {
        let engine = doc_new_engine(3, json!({"width": 8, "height": 8}));
        let bytes = save_package_bytes(&engine).unwrap();
        // Raw ZIP local-file-header check at offset 0 (ES §7.1: detectable
        // from the first bytes, like ODF/ORA): signature, compression
        // method 0 (stored), name "mimetype", then the exact payload.
        assert_eq!(&bytes[0..4], b"PK\x03\x04");
        let method = u16::from_le_bytes(bytes[8..10].try_into().unwrap());
        assert_eq!(method, 0, "mimetype entry must be STORED");
        let name_len = u16::from_le_bytes(bytes[26..28].try_into().unwrap()) as usize;
        let extra_len = u16::from_le_bytes(bytes[28..30].try_into().unwrap()) as usize;
        assert_eq!(&bytes[30..30 + name_len], b"mimetype");
        assert_eq!(extra_len, 0, "§7.6: mimetype stored with no extra field");
        let payload_at = 30 + name_len + extra_len;
        assert_eq!(
            payload_at, 38,
            "§7.6: mimetype content begins at byte offset 38 (sniffable)"
        );
        assert_eq!(
            &bytes[payload_at..payload_at + PACKAGE_MIMETYPE.len()],
            PACKAGE_MIMETYPE.as_bytes()
        );
    }

    #[test]
    fn entry_order_is_the_7_6_step_4_total_order() {
        // Review minor 1: mimetype, manifest.json, document.json, then ALL
        // remaining entries in ascending byte-lexicographic name order
        // [NORMATIVE §7.6 step 4 — the reproducibility contract].
        let engine = doc_new_engine(12, json!({"width": 300, "height": 300}));
        let bytes = save_package_bytes(&engine).unwrap();
        let mut za = zip::ZipArchive::new(std::io::Cursor::new(bytes.as_slice())).unwrap();
        let names: Vec<String> = (0..za.len())
            .map(|i| za.by_index(i).unwrap().name().to_string())
            .collect();
        assert_eq!(names[0], "mimetype");
        assert_eq!(names[1], "manifest.json");
        assert_eq!(names[2], "document.json");
        let tail = &names[3..];
        let mut sorted = tail.to_vec();
        sorted.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        assert_eq!(
            tail,
            sorted.as_slice(),
            "post-document entries in byte-lex order"
        );
        // journal/… sorts before layers/… ('j' < 'l') — the visible layout
        // change from the pre-review (lod, y, x)-grouped writer.
        assert_eq!(tail[0], "journal/autosave.log");
        assert!(tail[1..].iter().all(|n| n.starts_with("layers/")));
        assert_eq!(tail.len(), 1 + 4, "journal + 2x2 tile grid");
    }

    #[test]
    fn journal_framing_round_trips_cursor_and_tail() {
        // §7.8 framing unit round trip, including a redoable tail encoded
        // via the CHECKPOINT's last-in-effect id (nil ⇒ cursor 0).
        let doc_id = DocumentId(Uuid::from_u128(7));
        let e1 = Envelope::new(
            Uuid::from_u128(0x11),
            "doc.new",
            json!({"width": 8, "height": 8}),
        );
        let e2 = Envelope::new(Uuid::from_u128(0x22), "layer.add", json!({}));
        for cursor in [0usize, 1, 2] {
            let bytes = write_journal(doc_id, &[e1.clone(), e2.clone()], cursor);
            let mut warnings = Vec::new();
            let (envs, got) = read_journal(&bytes, doc_id.as_uuid(), &mut warnings).unwrap();
            assert!(warnings.is_empty(), "{warnings:?}");
            assert_eq!(envs.len(), 2);
            assert_eq!(envs[0], e1);
            assert_eq!(envs[1], e2);
            assert_eq!(got, cursor, "cursor {cursor} round-trips");
        }
        // BASE doc-id mismatch is a typed package-consistency error.
        let bytes = write_journal(doc_id, std::slice::from_ref(&e1), 1);
        let err = read_journal(&bytes, Uuid::from_u128(9), &mut Vec::new()).unwrap_err();
        assert_eq!(err.code, ErrorCode::IoReadFailed);
    }

    #[test]
    fn reopened_engine_guards_in_process_undo_redo() {
        let engine = doc_new_engine(4, json!({"width": 8, "height": 8}));
        let bytes = save_package_bytes(&engine).unwrap();
        let mut e = load_package_bytes(&bytes).unwrap().engine;
        // History is not persisted (§10.4): both directions answer with the
        // replay-path message.
        let err = e.undo().unwrap_err();
        assert_eq!(err.code, ErrorCode::NothingToUndo);
        assert!(err.message.contains("replay path"));
        assert_eq!(e.redo().unwrap_err().code, ErrorCode::NothingToRedo);
    }

    #[test]
    fn saving_an_uninitialized_engine_refuses_typed() {
        // Undoing doc.new returns the empty pre-init handle (§40.2), and an
        // uninitialized document has no package form.
        let mut engine = doc_new_engine(5, json!({"width": 8, "height": 8}));
        engine.undo().unwrap();
        assert!(engine.document().is_none());
        let err = save_package_bytes(&engine).unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);
    }

    #[test]
    fn malformed_packages_reject_typed_never_panic() {
        let engine = doc_new_engine(6, json!({"width": 8, "height": 8}));
        let good = save_package_bytes(&engine).unwrap();

        // Not a ZIP at all.
        let err = load_package_bytes(b"definitely not a zip").unwrap_err();
        assert_eq!(err.code, ErrorCode::IoReadFailed);

        // Wrong mimetype payload.
        let bad = repackage(&good, "mimetype", |_| Some(b"application/x-other".to_vec()));
        let err = load_package_bytes(&bad).unwrap_err();
        assert_eq!(err.code, ErrorCode::IoReadFailed);
        assert!(err.message.contains("mimetype"));

        // Manifest schema const wrong.
        let bad = repackage(&good, "manifest.json", |b| {
            Some(
                String::from_utf8(b)
                    .unwrap()
                    .replace(MANIFEST_SCHEMA, "appname.manifest.v9")
                    .into_bytes(),
            )
        });
        let err = load_package_bytes(&bad).unwrap_err();
        assert_eq!(err.code, ErrorCode::IoReadFailed);

        // minReader newer than this build (§7.7 clean refusal).
        let bad = repackage(&good, "manifest.json", |b| {
            Some(
                String::from_utf8(b)
                    .unwrap()
                    .replace("\"minReader\": \"0.1.0\"", "\"minReader\": \"9.9.9\"")
                    .into_bytes(),
            )
        });
        let err = load_package_bytes(&bad).unwrap_err();
        assert!(err.message.contains("requires reader 9.9.9"));
        // QA-F9 closure pin: the §7.7 refusal is the REGISTERED typed id.
        assert_eq!(err.code, ErrorCode::UnsupportedNewerDocument);
        assert_eq!(err.code.as_str(), "error.unsupported.newer_document");

        // document.json schema const wrong.
        let bad = repackage(&good, "document.json", |b| {
            Some(
                String::from_utf8(b)
                    .unwrap()
                    .replace(DOCUMENT_SCHEMA, "appname.document.v9")
                    .into_bytes(),
            )
        });
        let err = load_package_bytes(&bad).unwrap_err();
        assert_eq!(err.code, ErrorCode::IoReadFailed);

        // document.json not JSON.
        let bad = repackage(&good, "document.json", |_| Some(b"{nope".to_vec()));
        let err = load_package_bytes(&bad).unwrap_err();
        assert_eq!(err.code, ErrorCode::IoReadFailed);

        // Missing manifest entirely.
        let bad = repackage(&good, "manifest.json", |_| None);
        let err = load_package_bytes(&bad).unwrap_err();
        assert_eq!(err.code, ErrorCode::IoReadFailed);
    }

    #[test]
    fn corrupt_tile_chunk_degrades_gracefully_with_warning() {
        let engine = doc_new_engine(7, json!({"width": 300, "height": 8}));
        let good = save_package_bytes(&engine).unwrap();
        // Flip a payload byte of the first tile chunk (checksum mismatch →
        // transparent + warning, §7.4 rule 10 — not a hard failure).
        let doc = engine.document().unwrap();
        let bg = doc.z_order_walk()[0];
        let tile_entry = format!("layers/{}/tiles/0/0_0.tile", bg.as_uuid());
        let bad = repackage(&good, &tile_entry, |mut b| {
            let last = b.len() - 1;
            b[last] ^= 0xFF;
            Some(b)
        });
        let loaded = load_package_bytes(&bad).unwrap();
        assert_eq!(loaded.warnings.len(), 1);
        assert!(loaded.warnings[0].detail.contains("checksum"));
        // The damaged tile reads transparent; the neighbor tile survives.
        assert_ne!(loaded.engine.state_hash(), engine.state_hash());
    }

    #[test]
    fn corrupt_journal_tail_truncates_with_warning_never_fatal() {
        let engine = doc_new_engine(8, json!({"width": 8, "height": 8}));
        let good = save_package_bytes(&engine).unwrap();
        let bad = repackage(&good, "journal/autosave.log", |mut b| {
            // Flip a byte inside the LAST record's payload (the CHECKPOINT):
            // its CRC mismatches → truncation there; the COMMAND before it
            // survives, cursor falls back to journal end.
            let last = b.len() - 1;
            b[last] ^= 0xFF;
            Some(b)
        });
        let loaded = load_package_bytes(&bad).unwrap();
        assert!(
            loaded.warnings.iter().any(|w| w.detail.contains("CRC")),
            "{:?}",
            loaded.warnings
        );
        assert_eq!(
            loaded.engine.journal().len(),
            1,
            "commands before the tear survive"
        );
        assert_eq!(
            loaded.engine.journal_cursor(),
            1,
            "cursor defaults to journal end"
        );
        assert_eq!(loaded.engine.state_hash(), engine.state_hash());
    }

    #[test]
    fn atomic_save_writes_the_file_and_removes_the_temp() {
        let engine = doc_new_engine(9, json!({"width": 8, "height": 8}));
        let dir = std::env::temp_dir().join(format!("appname-core-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("atomic-save-test.appname");
        save_package(&engine, &path).unwrap();
        assert!(path.exists());
        assert!(
            !path.with_extension("appname.tmp").exists(),
            "§7.6 temp renamed away"
        );
        let loaded = load_package(&path).unwrap();
        assert_eq!(loaded.engine.state_hash(), engine.state_hash());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn document_json_is_canonical_sorted_keys() {
        let engine = doc_new_engine(10, json!({"width": 8, "height": 8}));
        let bytes = save_package_bytes(&engine).unwrap();
        let mut za = zip::ZipArchive::new(std::io::Cursor::new(bytes.as_slice())).unwrap();
        let doc_bytes = read_entry(&mut za, "document.json").unwrap();
        let text = String::from_utf8(doc_bytes).unwrap();
        // §2.8 canonical output: sorted keys (serde_json's default map is
        // ordered), LF, trailing newline, 2-space indent.
        assert!(text.ends_with('\n'));
        assert!(!text.contains('\r'));
        let color_at = text.find("\"color\"").unwrap();
        let geometry_at = text.find("\"geometry\"").unwrap();
        let layers_at = text.find("\"layers\"").unwrap();
        let root_at = text.find("\"root\"").unwrap();
        let schema_at = text.find("\"schema\"").unwrap();
        assert!(
            color_at < geometry_at
                && geometry_at < layers_at
                && layers_at < root_at
                && root_at < schema_at
        );
        // And import of a package written by this writer validates fully.
        let parsed: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["schema"], DOCUMENT_SCHEMA);
        assert_eq!(parsed["geometry"]["resolutionPPI"], 300.0);
    }

    #[test]
    fn out_of_range_opacity_fields_clamp_leniently_with_warnings() {
        // Review minor 4b: readers are lenient uniformly — BOTH opacity and
        // fillOpacity clamp into [0,1] with a recorded warning (writer-side
        // strictness stays with the schema; reader rule per the
        // document.v1.json x-spec-note + §6.4 sanitize precedent).
        let engine = doc_new_engine(13, json!({"width": 8, "height": 8}));
        let good = save_package_bytes(&engine).unwrap();
        let bad = repackage(&good, "document.json", |b| {
            let mut v: Value = serde_json::from_slice(&b).unwrap();
            for layer in v["layers"].as_array_mut().unwrap() {
                if layer["kind"] == "raster" {
                    layer["opacity"] = json!(1.5);
                    layer["fillOpacity"] = json!(-0.25);
                }
            }
            Some(canonical_json(&v))
        });
        let loaded = load_package_bytes(&bad).unwrap();
        assert_eq!(loaded.warnings.len(), 2, "{:?}", loaded.warnings);
        assert!(loaded.warnings.iter().all(|w| w.detail.contains("clamped")));
        let doc = loaded.engine.document().unwrap();
        let bg = doc.z_order_walk()[0];
        let layer = doc.layer(bg).unwrap();
        assert_eq!(layer.opacity(), 1.0);
        assert_eq!(layer.fill_opacity(), 0.0);
    }

    #[test]
    fn unknown_layer_kind_refuses_typed_with_the_7_7_citation() {
        // Review minor 4a: the §7.7 scenario-3 fallback is deferred (lane
        // log fold-in); until then the refusal is typed and names it.
        let engine = doc_new_engine(14, json!({"width": 8, "height": 8}));
        let good = save_package_bytes(&engine).unwrap();
        let bad = repackage(&good, "document.json", |b| {
            let mut v: Value = serde_json::from_slice(&b).unwrap();
            for layer in v["layers"].as_array_mut().unwrap() {
                if layer["kind"] == "raster" {
                    layer["kind"] = json!("flow");
                }
            }
            Some(canonical_json(&v))
        });
        let err = load_package_bytes(&bad).unwrap_err();
        assert_eq!(err.code, ErrorCode::IoReadFailed);
        assert!(err.message.contains("§7.7"), "{}", err.message);
    }

    #[test]
    fn assets_round_trip_with_gc_and_tamper_detection() {
        // Wave-2 arbitration 4: referenced assets persist byte-stable;
        // unreferenced blobs are GC'd at save; tampered blobs degrade to a
        // warning + skip. The referencing envelope is the journaled doc.new
        // itself — `samples` is an envelope-level field (§10.2), carried at
        // stage 1 regardless of kind, and content-derived ids are
        // precomputable before the blob exists.
        let blob = b"APNSAMPL-not-a-real-recording".to_vec();
        let referenced = crate::engine::assets::asset_id_for(&blob);

        let mut engine = Engine::new();
        let text = json!({
            "schema": "appname.command.v1",
            "id": "c0000000-0000-4000-8000-0000000000aa",
            "kind": "doc.new",
            "params": {"width": 8, "height": 8},
            "samples": referenced.to_string(),
        })
        .to_string();
        engine.apply_json(&text).unwrap();
        assert_eq!(engine.add_samples_asset(blob.clone()), referenced);
        let _orphan = engine.add_samples_asset(b"unreferenced-blob".to_vec());

        let bytes = save_package_bytes(&engine).unwrap();
        let reloaded = load_package_bytes(&bytes).unwrap();
        assert!(reloaded.warnings.is_empty(), "{:?}", reloaded.warnings);
        assert!(
            reloaded.engine.assets().contains(referenced),
            "referenced asset persists"
        );
        assert_eq!(
            reloaded.engine.assets().len(),
            2,
            "session-pending orphan survives its OWN save (asset-add-then-stroke pipeline)"
        );
        assert_eq!(
            reloaded.engine.assets().bytes(referenced).unwrap(),
            blob.as_slice()
        );
        // After a reopen the orphan is no longer pending: the NEXT save
        // collects it (bounded orphan lifetime).
        let second = save_package_bytes(&reloaded.engine).unwrap();
        let re2 = load_package_bytes(&second).unwrap();
        assert_eq!(
            re2.engine.assets().len(),
            1,
            "unreferenced non-pending blob GC'd"
        );
        assert!(re2.engine.assets().contains(referenced));
        // Byte-stable round trip once the asset set is settled.
        let third = save_package_bytes(&re2.engine).unwrap();
        assert_eq!(second, third);

        // Tampered asset entry: warning + skip, never fatal (the settled
        // second save carries only the referenced asset).
        let entry = format!("assets/{referenced}");
        let bad = repackage(&second, &entry, |mut b| {
            b[0] ^= 0xFF;
            Some(b)
        });
        let damaged = load_package_bytes(&bad).unwrap();
        assert_eq!(damaged.warnings.len(), 1, "{:?}", damaged.warnings);
        assert!(damaged.warnings[0].detail.contains("content-derived"));
        assert!(
            !damaged.engine.assets().contains(referenced),
            "tampered blob skipped"
        );
        assert!(damaged.engine.assets().is_empty());
    }

    #[test]
    fn masks_round_trip_with_state_hash_identity() {
        // CP-0.5 mask doc-model: attach a raster mask via the doors, round
        // trip the package, prove state-hash identity and byte stability.
        let mut engine = doc_new_engine(50, json!({"width": 300, "height": 300}));
        {
            let doc = engine.doc_mut_for_test();
            let bg = doc.z_order_walk()[0];
            let mut coverage = crate::mask::Selection::new_mask_buffer();
            coverage.fill_rect(
                crate::geom::IRect::new(10, 10, 200, 150),
                [0.75, 0.0, 0.0, 0.0],
            );
            let mask = crate::graph::mask_node::MaskNode::new_raster(
                crate::graph::id::MaskId::new(),
                coverage,
            );
            doc.attach_mask(bg, mask).unwrap();
        }
        let h = engine.state_hash();
        let bytes = save_package_bytes(&engine).unwrap();
        let loaded = load_package_bytes(&bytes).unwrap();
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        assert_eq!(
            loaded.engine.state_hash(),
            h,
            "mask round trip is hash-identical"
        );
        let doc = loaded.engine.document().unwrap();
        assert_eq!(doc.mask_count(), 1);
        let bg = doc.z_order_walk()[0];
        let m = doc.mask(doc.layer(bg).unwrap().masks()[0]).unwrap();
        assert!(m.enabled() && m.linked() && !m.inverted());
        assert!(m.coverage().bounds() == crate::geom::IRect::new(10, 10, 200, 150));
        assert_eq!(
            save_package_bytes(&loaded.engine).unwrap(),
            bytes,
            "byte-stable"
        );
    }

    // -- §22 fill layers ---------------------------------------------------

    /// A document with one fill layer of each frozen `$defs/fill` arm,
    /// created through the REAL `layer.add` door so the registry is the
    /// thing under test and not a hand-built node.
    fn engine_with_fill_layers() -> Engine {
        let mut e = crate::engine::kinds::register(crate::EngineAssembly::new()).build();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "f111a000-0000-4000-8000-000000000001",
                "kind": "doc.new",
                "params": {"width": 32, "height": 32, "background": "white"},
            })
            .to_string(),
        )
        .unwrap();
        let arms = [
            json!({"type": "none"}),
            json!({"type": "solid", "color": {"model": "rgba", "comps": [0.2, 0.4, 0.6, 0.8]}}),
            json!({"type": "pattern", "patternRef": "assets/paper", "scale": 2.5,
                   "phase": [3.0, -4.0]}),
            json!({
                "type": "gradient", "angleDeg": 45, "scale": 2, "reverse": true,
                // §7.7: an unknown field must survive the round trip.
                "futureV56Field": {"kept": true},
                "gradient": {
                    "kind": "radial", "space": "perceptual", "dither": false,
                    "stops": [
                        {"pos": 0, "color": {"model": "rgb", "comps": [0, 0, 0, 0]},
                         "midpoint": 0.25, "opacity": 0.5},
                        {"pos": 1, "color": {"model": "rgb", "comps": [1, 1, 1, 0]}}
                    ]
                }
            }),
        ];
        for (i, arm) in arms.iter().enumerate() {
            e.apply_json(
                &json!({
                    "schema": "appname.command.v1",
                    "id": format!("f111a000-0000-4000-8000-00000000001{i}"),
                    "kind": "layer.add",
                    "params": {
                        "kind": "fill",
                        "name": format!("Fill {i}"),
                        "newLayerId": format!("f111a000-0000-4000-8000-00000000002{i}"),
                        "kindParams": arm,
                    },
                })
                .to_string(),
            )
            .unwrap();
        }
        e
    }

    #[test]
    fn fill_layers_round_trip_with_state_hash_identity() {
        // The seam the registry would otherwise dangle on: a fill layer
        // that cannot be saved and re-opened is not served, however well
        // it decodes. document.v1 `$defs.layer` makes `fill` REQUIRED when
        // `kind == "fill"`, so a writer that omits it emits a document its
        // own reader is right to refuse.
        let engine = engine_with_fill_layers();
        let h = engine.state_hash();
        let bytes = save_package_bytes(&engine).unwrap();

        // The written JSON must actually carry the schema-required field.
        let mut za = zip::ZipArchive::new(std::io::Cursor::new(bytes.clone())).unwrap();
        let doc_json: Value = {
            let mut s = String::new();
            za.by_name("document.json")
                .unwrap()
                .read_to_string(&mut s)
                .unwrap();
            serde_json::from_str(&s).unwrap()
        };
        let fills: Vec<&Value> = doc_json["layers"]
            .as_array()
            .expect("layers array")
            .iter()
            .filter(|l| l["kind"] == json!("fill"))
            .collect();
        assert_eq!(fills.len(), 4, "all four arms must be written");
        for l in &fills {
            assert!(
                l.get("fill").is_some(),
                "document.v1 requires `fill` on kind:\"fill\"; wrote {l}"
            );
        }

        let loaded = load_package_bytes(&bytes).unwrap();
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        assert_eq!(
            loaded.engine.state_hash(),
            h,
            "fill-layer round trip must be hash-identical"
        );
        // The decoded content survives, and so does the §7.7 unknown field.
        let doc = loaded.engine.document().unwrap();
        let mut seen = Vec::new();
        for id in doc.z_order_walk() {
            if let Some(fd) = doc.layer(id).unwrap().fill_data() {
                seen.push(fd.content().wire_type());
                if fd.content().wire_type() == "gradient" {
                    assert_eq!(
                        fd.wire()["futureV56Field"],
                        json!({"kept": true}),
                        "§7.7 unknown fields must survive the round trip"
                    );
                    let crate::engine::fill_reg::FillContent::Gradient {
                        gradient,
                        angle_deg,
                        scale,
                        reverse,
                    } = fd.content()
                    else {
                        panic!("expected gradient")
                    };
                    assert_eq!((*angle_deg, *scale, *reverse), (45.0, 2.0, true));
                    assert_eq!(gradient.stops()[0].midpoint, 0.25);
                    assert_eq!(gradient.stops()[0].alpha, 0.5);
                    assert!(!gradient.dither());
                }
            }
        }
        assert_eq!(seen, vec!["none", "solid", "pattern", "gradient"]);
        assert_eq!(
            save_package_bytes(&loaded.engine).unwrap(),
            bytes,
            "byte-stable"
        );
    }

    #[test]
    fn a_fill_layer_missing_its_payload_is_refused_on_load() {
        // The reader must not default a schema-REQUIRED field into
        // existence, and must not accept content the command door would
        // have refused — one validation gate, two doors.
        let bytes = save_package_bytes(&engine_with_fill_layers()).unwrap();
        for (label, mutate) in [
            ("dropped", None::<Value>),
            ("unknown arm", Some(json!({"type": "plaid"}))),
            (
                "gradient with one stop",
                Some(json!({
                    "type": "gradient",
                    "gradient": {"kind": "linear", "stops": [
                        {"pos": 0, "color": {"model": "rgb", "comps": [0, 0, 0, 0]}}]}
                })),
            ),
        ] {
            let tampered = repackage(&bytes, "document.json", |data| {
                let mut v: Value = serde_json::from_slice(&data).unwrap();
                for l in v["layers"].as_array_mut().unwrap() {
                    if l["kind"] == json!("fill") {
                        match &mutate {
                            None => {
                                l.as_object_mut().unwrap().remove("fill");
                            }
                            Some(m) => l["fill"] = m.clone(),
                        }
                    }
                }
                Some(serde_json::to_vec(&v).unwrap())
            });
            let err = load_package_bytes(&tampered).unwrap_err();
            assert_eq!(
                err.code,
                ErrorCode::IoReadFailed,
                "{label}: expected a typed read refusal, got {err:?}"
            );
        }
    }

    #[test]
    fn fill_content_folds_into_the_state_hash() {
        // Without the fold, two documents differing ONLY in fill content
        // hash EQUAL — which would make a content change invisible to
        // autosave/dirty detection and let §34.4 property 9 pass on a
        // document that never actually returned.
        let mk = |comps: Value| {
            let mut e = crate::engine::kinds::register(crate::EngineAssembly::new()).build();
            e.apply_json(
                &json!({
                    "schema": "appname.command.v1",
                    "id": "f111b000-0000-4000-8000-000000000001",
                    "kind": "doc.new",
                    "params": {"width": 16, "height": 16, "background": "white"},
                })
                .to_string(),
            )
            .unwrap();
            e.apply_json(
                &json!({
                    "schema": "appname.command.v1",
                    "id": "f111b000-0000-4000-8000-000000000002",
                    "kind": "layer.add",
                    "params": {
                        "kind": "fill", "name": "Wash",
                        "newLayerId": "f111b000-0000-4000-8000-000000000003",
                        "kindParams": {"type": "solid",
                                       "color": {"model": "rgb", "comps": comps}},
                    },
                })
                .to_string(),
            )
            .unwrap();
            e
        };
        let red = mk(json!([1.0, 0.0, 0.0, 0.0]));
        let blue = mk(json!([0.0, 0.0, 1.0, 0.0]));
        assert_ne!(
            red.state_hash(),
            blue.state_hash(),
            "a red fill and a blue fill must not share a state hash"
        );
        // Identical content still agrees — the fold is a function of the
        // payload, not of allocation order.
        assert_eq!(
            mk(json!([1.0, 0.0, 0.0, 0.0])).state_hash(),
            red.state_hash()
        );
    }

    #[test]
    fn loader_sanitizes_premultiplied_violations_with_warnings() {
        // §6.4 reader-sanitize: craft a PREMULTIPLIED-flagged u8 chunk whose
        // payload violates both rules (rgb > a; color under a = 0).
        let engine = doc_new_engine(
            40,
            json!({"width": 8, "height": 8, "background": "transparent"}),
        );
        let doc = engine.document().unwrap();
        let bg = doc.z_order_walk()[0];
        let mut tile = crate::tile::storage::TileStorage::materialize(
            [0.0; 4],
            crate::tile::channel::ChannelKind::U8,
            ColorModelKind::Rgba,
        );
        tile.write_pixel_f32(0, 0, [0.8, 0.1, 0.1, 0.5]); // rgb > a
        tile.write_pixel_f32(1, 0, [0.25, 0.0, 0.0, 0.0]); // color under a=0
        tile.write_pixel_f32(2, 0, [0.25, 0.25, 0.25, 0.5]); // valid premult
        let chunk = crate::io::tile_chunk::write_tile_chunk(
            crate::tile::map::TileCoord::new(0, 0, 0),
            &tile,
            crate::tile::channel::ChannelKind::U8,
            ColorModelKind::Rgba,
            true, // claims premultiplied
        );
        let good = save_package_bytes(&engine).unwrap();
        let entry = format!("layers/{}/tiles/0/0_0.tile", bg.as_uuid());
        let bad = {
            let mut za = zip::ZipArchive::new(std::io::Cursor::new(good.as_slice())).unwrap();
            let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
            for i in 0..za.len() {
                let mut e = za.by_index(i).unwrap();
                let mut data = Vec::new();
                e.read_to_end(&mut data).unwrap();
                entries.push((e.name().to_string(), data));
            }
            entries.push((entry.clone(), chunk));
            let mut cursor = std::io::Cursor::new(Vec::new());
            {
                let mut zw = zip::ZipWriter::new(&mut cursor);
                let opts = zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored);
                for (name, data) in entries {
                    zw.start_file(name, opts).unwrap();
                    zw.write_all(&data).unwrap();
                }
                zw.finish().unwrap();
            }
            cursor.into_inner()
        };
        let loaded = load_package_bytes(&bad).unwrap();
        assert_eq!(loaded.warnings.len(), 1, "{:?}", loaded.warnings);
        assert!(loaded.warnings[0].detail.contains("sanitized"));
        assert!(loaded.warnings[0].detail.contains("2 pixel"));
        let buf = loaded
            .engine
            .document()
            .unwrap()
            .layer(bg)
            .unwrap()
            .pixels()
            .unwrap();
        assert!(
            buf.validate_premultiplied().is_ok(),
            "sanitized surface passes §6.4"
        );
        use crate::geom::IVec2 as V;
        let px = buf.read_pixel_f32(V::new(0, 0));
        assert_eq!(px[0], px[3], "rgb clamped to a");
        assert_eq!(
            buf.read_pixel_f32(V::new(1, 0)),
            [0.0; 4],
            "hidden color zeroed"
        );
        let keep = buf.read_pixel_f32(V::new(2, 0));
        assert!(keep[0] > 0.0, "valid pixels untouched");
        // Re-save emits the SANITIZED truth (differs from the corrupt
        // input, byte-stable thereafter).
        let s1 = save_package_bytes(&loaded.engine).unwrap();
        let re = load_package_bytes(&s1).unwrap();
        assert!(
            re.warnings.is_empty(),
            "sanitized package loads without warnings"
        );
        assert_eq!(save_package_bytes(&re.engine).unwrap(), s1);
    }

    #[test]
    fn export_honors_the_straight_f32_premultiplied_flag() {
        // Wave-2 arbitration 3 / review-verified divergence: a straight-
        // alpha f32 layer (chunk flags bit0 = 0) must premultiply at the
        // export boundary. Crafted via public doors: save a doc.new f32
        // package, splice in a straight-content chunk with the flag off.
        let engine = doc_new_engine(
            30,
            json!({"width": 8, "height": 8, "depth": "f32", "background": "transparent"}),
        );
        let doc = engine.document().unwrap();
        let bg = doc.z_order_walk()[0];
        // Straight content: color 0.5 at alpha 0.5 (a value where straight
        // and premultiplied interpretations visibly differ).
        let mut tile = crate::tile::storage::TileStorage::materialize(
            [0.0; 4],
            crate::tile::channel::ChannelKind::F32,
            ColorModelKind::Rgba,
        );
        tile.write_pixel_f32(0, 0, [0.5, 0.0, 0.0, 0.5]);
        let chunk = crate::io::tile_chunk::write_tile_chunk(
            crate::tile::map::TileCoord::new(0, 0, 0),
            &tile,
            crate::tile::channel::ChannelKind::F32,
            ColorModelKind::Rgba,
            false, // STRAIGHT storage
        );
        let good = save_package_bytes(&engine).unwrap();
        let entry = format!("layers/{}/tiles/0/0_0.tile", bg.as_uuid());
        // The transparent background wrote no tiles: append the crafted one.
        let bad = {
            let mut za = zip::ZipArchive::new(std::io::Cursor::new(good.as_slice())).unwrap();
            let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
            for i in 0..za.len() {
                let mut e = za.by_index(i).unwrap();
                let mut data = Vec::new();
                e.read_to_end(&mut data).unwrap();
                entries.push((e.name().to_string(), data));
            }
            entries.push((entry.clone(), chunk));
            let mut cursor = std::io::Cursor::new(Vec::new());
            {
                let mut zw = zip::ZipWriter::new(&mut cursor);
                let opts = zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored);
                for (name, data) in entries {
                    zw.start_file(name, opts).unwrap();
                    zw.write_all(&data).unwrap();
                }
                zw.finish().unwrap();
            }
            cursor.into_inner()
        };
        let loaded = load_package_bytes(&bad).unwrap();
        let buf = loaded
            .engine
            .document()
            .unwrap()
            .layer(bg)
            .unwrap()
            .pixels()
            .unwrap();
        assert!(
            !buf.premultiplied(),
            "chunk flag bit0=0 loads as straight storage"
        );

        let png = crate::io::png_io::export_png_bytes(&loaded.engine).unwrap();
        let (decoded, _, _) =
            crate::io::png_io::decode_png_to_buffer(&png, crate::graph::id::ProfileId::nil())
                .unwrap();
        let px = decoded.read_pixel_f32(crate::geom::IVec2::new(0, 0));
        // Straight (0.5, a 0.5) → premult (0.25, 0.5) → unpremul 0.5 →
        // sRGB 188/255; the pre-fix passthrough would have unpremultiplied
        // 0.5/0.5 = 1.0 → 255.
        let expect = crate::io::srgb::srgb_decode(188.0 / 255.0) * 0.5;
        assert!(
            (px[0] - expect).abs() < 2.0 / 255.0,
            "straight-f32 premultiplies at export: got {px:?}, want r≈{expect}"
        );
        assert!((px[3] - 0.5).abs() < 1.0 / 255.0);
    }

    /// The MAJOR-2 reconciliation, proven end-to-end: imports are now a
    /// JOURNALED `edit.paste` over an ingested raster asset, so journal
    /// replay reproduces the imported document byte-identically — the
    /// inverse of the retired `import_breaks_replay_equality_as_documented`
    /// (whose IO-call door is gone). The raster asset persists in the
    /// package (`assets/{uuid}.image`, referenced via `params.assetId`) and
    /// the reopened engine replays to the same state hash.
    #[test]
    fn journaled_import_replay_equality_holds() {
        let assembly =
            crate::engine::kinds::register(crate::engine::kind_impl::EngineAssembly::new());
        let mut engine = assembly.build();
        engine
            .apply_json(
                &json!({
                    "schema": "appname.command.v1",
                    "id": "d0000000-0000-4000-8000-00000000000b",
                    "kind": "doc.new",
                    "params": {"width": 8, "height": 8},
                })
                .to_string(),
            )
            .unwrap();
        // A 2×2 opaque red PNG, ingested as a raster asset.
        let png = {
            let mut out = Vec::new();
            let mut enc = png::Encoder::new(&mut out, 2, 2);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut wr = enc.write_header().unwrap();
            wr.write_image_data(&[255u8, 0, 0, 255].repeat(4)).unwrap();
            drop(wr);
            out
        };
        let asset = engine.add_raster_asset(png).unwrap();
        let new_layer = "d0000000-0000-4000-8000-00000000001b";
        engine
            .apply_json(
                &json!({
                    "schema": "appname.command.v1",
                    "id": "d0000000-0000-4000-8000-00000000000c",
                    "kind": "edit.paste",
                    "params": {"assetId": asset, "at": [0, 0], "newLayerId": new_layer},
                })
                .to_string(),
            )
            .unwrap();
        assert_eq!(engine.document().unwrap().layer_count(), 3);

        // Replay equality HOLDS (the old IO-call door broke it by design).
        let replayed = engine.replay_assembled(engine.journal()).unwrap();
        assert_eq!(replayed.state_hash(), engine.state_hash());

        // Package round trip: the asset rides as assets/{uuid}.image, the
        // journal references it, and the REOPENED engine replays to the
        // same state (the CLI replay-recovery path).
        let bytes = save_package_bytes(&engine).unwrap();
        let mut za = zip::ZipArchive::new(std::io::Cursor::new(bytes.as_slice())).unwrap();
        let names: Vec<String> = (0..za.len())
            .map(|i| za.by_index(i).unwrap().name().to_string())
            .collect();
        assert!(
            names.contains(&format!("assets/{asset}.image")),
            "raster asset persists with its kind suffix: {names:?}"
        );
        let loaded = load_package_bytes_with(&assembly, &bytes).unwrap();
        assert_eq!(loaded.engine.state_hash(), engine.state_hash());
        let re_replayed = loaded
            .engine
            .replay_assembled(loaded.engine.journal())
            .unwrap();
        assert_eq!(re_replayed.state_hash(), engine.state_hash());
    }

    /// q-tutorial/Q-9 (MAJOR; reproduced by O-WATCH #906 and again by this
    /// desk before the fix): `colorLabel` was WRITTEN by the layer writer and
    /// never read back by `build_node`, so ONE load+save dropped it while
    /// `visible` survived the same trip. The asymmetry #906 named is what
    /// makes it more than cosmetic: this engine preserves UNKNOWN fields
    /// verbatim (the open `meta` object) and was losing a KNOWN,
    /// contract-declared (layer.set_props), ABI-published one.
    ///
    /// `visible` is asserted too, and deliberately set to a NON-default, so
    /// the case cannot pass on a reader that dropped both.
    #[test]
    fn color_label_survives_the_package_round_trip() {
        // The REAL registry, because `layer.set_props` is the door under
        // test and `doc_new_engine`'s minimal assembly does not carry it.
        let mut e = crate::engine::kinds::register(crate::EngineAssembly::new()).build();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "c1a00000-0000-4000-8000-00000000000d",
                "kind": "doc.new",
                "params": {"width": 16, "height": 16, "background": "white"},
            })
            .to_string(),
        )
        .unwrap();
        let layer = e.document().unwrap().z_order_walk()[0];
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "c1a00000-0000-4000-8000-000000000001",
                "kind": "layer.set_props",
                "target": {"layerId": layer.as_uuid()},
                "params": {"colorLabel": "green", "visible": false},
            })
            .to_string(),
        )
        .unwrap();
        assert_eq!(
            e.document().unwrap().layer(layer).unwrap().color_label(),
            Some(ColorLabel::Green)
        );

        // The wire carries it — the writer was never the defect.
        let b1 = save_package_bytes(&e).unwrap();
        let mut za = zip::ZipArchive::new(std::io::Cursor::new(b1.as_slice())).unwrap();
        let document: Value = parse_json_entry(&mut za, "document.json").unwrap();
        let on_wire = document["layers"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|l| l.get("colorLabel"))
            .count();
        assert_eq!(on_wire, 1, "exactly one layer carries a label on the wire");

        // THE FINDING: one load+save used to lose it here.
        let loaded = load_package_bytes(&b1).unwrap();
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        let n = loaded.engine.document().unwrap().layer(layer).unwrap();
        assert_eq!(n.color_label(), Some(ColorLabel::Green), "Q-9: colorLabel");
        assert!(!n.visible(), "Q-9 control: visible survived the same trip");

        // It must not decay on the SECOND trip either.
        let b2 = save_package_bytes(&loaded.engine).unwrap();
        let twice = load_package_bytes(&b2).unwrap();
        assert_eq!(
            twice
                .engine
                .document()
                .unwrap()
                .layer(layer)
                .unwrap()
                .color_label(),
            Some(ColorLabel::Green)
        );

        // NEGATIVE CONTROL: a layer that never carried a label must not
        // acquire one, so the arms above measure a RESTORED value, not a
        // default that happens to match.
        let mut plain = crate::engine::kinds::register(crate::EngineAssembly::new()).build();
        plain
            .apply_json(
                &json!({
                    "schema": "appname.command.v1",
                    "id": "c1a00000-0000-4000-8000-00000000000e",
                    "kind": "doc.new",
                    "params": {"width": 16, "height": 16, "background": "white"},
                })
                .to_string(),
            )
            .unwrap();
        let pb = save_package_bytes(&plain).unwrap();
        let pl = load_package_bytes(&pb).unwrap();
        let pd = pl.engine.document().unwrap();
        let p0 = pd.z_order_walk()[0];
        assert_eq!(pd.layer(p0).unwrap().color_label(), None);

        // LENIENT READER: an undeclared label is a WARNING, not a refusal.
        // Refusing a whole document over a cosmetic enum, and dropping the
        // field silently again, are both worse than recording it.
        let target_id = layer.as_uuid().to_string();
        let bad = repackage(&b1, "document.json", |bytes: Vec<u8>| {
            let mut d: Value = serde_json::from_slice(&bytes).unwrap();
            // BY ID on purpose: `layers[0]` is the ROOT group, and the loader
            // does not build the root through `build_node`, so a bad label
            // there is parsed by nobody and would make this arm vacuous.
            let ls = d["layers"].as_array_mut().unwrap();
            let slot = ls
                .iter_mut()
                .find(|l| l["id"] == json!(target_id))
                .expect("the labelled layer is on the wire");
            slot["colorLabel"] = json!("chartreuse");
            Some(serde_json::to_vec(&d).unwrap())
        });
        let lenient = load_package_bytes(&bad).unwrap();
        assert_eq!(
            lenient
                .engine
                .document()
                .unwrap()
                .layer(layer)
                .unwrap()
                .color_label(),
            None,
            "an undeclared label is dropped, never guessed into a neighbour"
        );
        assert!(
            lenient
                .warnings
                .iter()
                .any(|w| w.detail.contains("colorLabel")),
            "the drop is RECORDED, not silent: {:?}",
            lenient.warnings
        );
    }
}

#[cfg(test)]
mod opnode_round_trip {
    use super::*;
    use crate::engine::kind_impl::EngineAssembly;
    use serde_json::json;

    /// §7.5 sidecar + inline-smartObject round trip: an adjustment layer
    /// (opRef → ops/{id}.json), a masked smart filter, and an embedded
    /// smart object (blake3 assetHash) all survive save→load with
    /// state-hash identity; op-free packages stay byte-identical.
    #[test]
    fn parametric_doc_round_trips_with_state_hash_identity() {
        let assembly = crate::engine::kinds::register(EngineAssembly::new());
        let mut e = assembly.build();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "0b70de00-0000-4000-8000-000000000001",
                "kind": "doc.new",
                "params": {"width": 64, "height": 64, "background": "white"},
            })
            .to_string(),
        )
        .unwrap();
        // Embedded raster asset for the smart object.
        let png = {
            let mut out = Vec::new();
            let mut enc = png::Encoder::new(&mut out, 4, 4);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut wr = enc.write_header().unwrap();
            wr.write_image_data(&[10u8, 200, 30, 255].repeat(16))
                .unwrap();
            drop(wr);
            out
        };
        let asset = e.add_raster_asset(png.clone()).unwrap();
        // Journal must reference the asset for save-time GC: a paste does.
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "0b70de00-0000-4000-8000-000000000002",
                "kind": "edit.paste",
                "params": {"assetId": asset.to_string(), "at": [0, 0],
                            "newLayerId": "0b70de00-0000-4000-8000-000000000012"},
            })
            .to_string(),
        )
        .unwrap();
        {
            // Crate-door fixtures (the kinds land in the next commits).
            let doc = e.doc_mut_for_test();
            let root = doc.root();
            let n = doc.children_of(root).unwrap().len();
            // Adjustment layer + its op.
            let oid = crate::graph::id::OpId::from_uuid(
                Uuid::parse_str("0b70de00-0000-4000-8000-00000000000a").unwrap(),
            );
            let lid = LayerId::from_uuid(
                Uuid::parse_str("0b70de00-0000-4000-8000-00000000000b").unwrap(),
            );
            doc.insert_op(crate::graph::op_node::OpNode::new(
                oid,
                "adjustment.curves",
                json!({"channels": {"master": {"points": [[0, 0], [1, 1]]}}}),
            ))
            .unwrap();
            assert!(doc.op(oid).is_some(), "DBG op not stored after insert_op");
            doc.insert_child(
                root,
                n,
                crate::graph::node::LayerNode::new_adjustment(lid, "Curves", oid),
            )
            .unwrap();
            // Smart object with one masked smart filter.
            let fid = crate::graph::id::OpId::from_uuid(
                Uuid::parse_str("0b70de00-0000-4000-8000-00000000000c").unwrap(),
            );
            let mid = crate::graph::id::MaskId::from_uuid(
                Uuid::parse_str("0b70de00-0000-4000-8000-00000000000d").unwrap(),
            );
            let mut fop = crate::graph::op_node::OpNode::new(
                fid,
                "filter.gaussian_blur",
                json!({"sigma": 2.5}),
            );
            let mut cov = crate::mask::Selection::new_mask_buffer();
            cov.fill_rect(crate::geom::IRect::new(0, 0, 4, 4), [0.6, 0.0, 0.0, 0.0]);
            cov.set_bounds_recorded(crate::geom::IRect::new(0, 0, 4, 4));
            doc.attach_op_mask(fid, crate::graph::mask_node::MaskNode::new_raster(mid, cov))
                .unwrap();
            fop.mask = Some(mid);
            doc.insert_op(fop).unwrap();
            let sid = LayerId::from_uuid(
                Uuid::parse_str("0b70de00-0000-4000-8000-00000000000e").unwrap(),
            );
            let working = doc.color().working;
            let mut cached = crate::buffer::Buffer::new(
                crate::tile::channel::ChannelKind::F32,
                crate::tile::model::ColorModelKind::Rgba,
                working,
            );
            cached.fill_rect(crate::geom::IRect::new(0, 0, 4, 4), [0.1, 0.7, 0.2, 1.0]);
            cached.set_bounds_recorded(crate::geom::IRect::new(0, 0, 4, 4));
            let n2 = doc.children_of(root).unwrap().len();
            doc.insert_child(
                root,
                n2,
                crate::graph::node::LayerNode::new_smart_object(
                    sid,
                    "Placed",
                    crate::graph::node::SmartObjectData {
                        source: crate::graph::node::SmartSource::Embedded(asset),
                        cached_composite: cached,
                        smart_filters: vec![fid],
                        place_transform: crate::geom::Mat3::IDENTITY,
                    },
                ),
            )
            .unwrap();
            doc.validate_tree().unwrap();
        }
        let bytes = save_package_bytes(&e).unwrap();
        // The op sidecars + smartObject block are present.
        {
            let mut za = zip::ZipArchive::new(std::io::Cursor::new(bytes.as_slice())).unwrap();
            let names: Vec<String> = (0..za.len())
                .map(|i| za.by_index(i).unwrap().name().to_string())
                .collect();
            assert!(names
                .iter()
                .any(|n| n == "ops/0b70de00-0000-4000-8000-00000000000a.json"));
            assert!(names
                .iter()
                .any(|n| n == "ops/0b70de00-0000-4000-8000-00000000000c.json"));
            assert!(names
                .iter()
                .any(|n| n.starts_with("ops/0b70de00-0000-4000-8000-00000000000c/mask/")));
        }
        let loaded = load_package_bytes_with(&assembly, &bytes).unwrap();
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        let ld = loaded.engine.document().unwrap();
        assert_eq!(ld.op_count(), 2);
        {
            let od = e.document().unwrap();
            let oid = crate::graph::id::OpId::from_uuid(
                Uuid::parse_str("0b70de00-0000-4000-8000-00000000000a").unwrap(),
            );
            let fid = crate::graph::id::OpId::from_uuid(
                Uuid::parse_str("0b70de00-0000-4000-8000-00000000000c").unwrap(),
            );
            assert_eq!(
                ld.op(oid).unwrap().params(),
                od.op(oid).unwrap().params(),
                "DBG curves params"
            );
            assert_eq!(
                ld.op(fid).unwrap().params(),
                od.op(fid).unwrap().params(),
                "DBG blur params"
            );
            let mid = crate::graph::id::MaskId::from_uuid(
                Uuid::parse_str("0b70de00-0000-4000-8000-00000000000d").unwrap(),
            );
            assert_eq!(
                ld.mask(mid).unwrap().coverage().bounds(),
                od.mask(mid).unwrap().coverage().bounds(),
                "DBG op-mask bounds"
            );
            assert_eq!(
                ld.mask(mid).unwrap().coverage().region_hash(),
                od.mask(mid).unwrap().coverage().region_hash(),
                "DBG op-mask region hash"
            );
            let sid = LayerId::from_uuid(
                Uuid::parse_str("0b70de00-0000-4000-8000-00000000000e").unwrap(),
            );
            let (lb, ob) = (
                ld.layer(sid)
                    .unwrap()
                    .smart_object()
                    .unwrap()
                    .cached_composite(),
                od.layer(sid)
                    .unwrap()
                    .smart_object()
                    .unwrap()
                    .cached_composite(),
            );
            assert_eq!(
                (
                    lb.bounds(),
                    lb.kind(),
                    lb.model(),
                    lb.premultiplied(),
                    lb.tiles().len()
                ),
                (
                    ob.bounds(),
                    ob.kind(),
                    ob.model(),
                    ob.premultiplied(),
                    ob.tiles().len()
                ),
                "DBG cached composite framing"
            );
            assert_eq!(lb.region_hash(), ob.region_hash(), "DBG cached composite");
            assert_eq!(
                ld.layer(sid).unwrap().smart_object().unwrap().source(),
                od.layer(sid).unwrap().smart_object().unwrap().source(),
                "DBG smart source"
            );
        }
        assert_eq!(
            loaded.engine.state_hash_persistent(),
            e.state_hash_persistent(),
            "parametric round trip is state-identical"
        );
        // Byte-stable second save.
        let again = save_package_bytes(&loaded.engine).unwrap();
        let third = load_package_bytes_with(&assembly, &again).unwrap();
        assert_eq!(
            third.engine.state_hash_persistent(),
            e.state_hash_persistent()
        );
    }

    /// Fix-round BLOCKER-2 regression: a smart-object SOURCE asset whose
    /// only references are the document itself (document.json `assetHash`)
    /// must survive the save-after-reopen GC. The failing sequence was
    /// save → load → save (no new commands) → load: the first save
    /// survived only via `is_pending`, the second dropped
    /// `assets/{uuid}.image`, and the third open died
    /// `error.io.read_failed` (no asset entry matches recorded hash).
    #[test]
    fn smart_object_source_survives_the_save_after_reopen_gc() {
        let assembly = crate::engine::kinds::register(EngineAssembly::new());
        let mut e = assembly.build();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "0b70de00-0000-4000-8000-0000000000b1",
                "kind": "doc.new",
                "params": {"width": 32, "height": 32, "background": "white"},
            })
            .to_string(),
        )
        .unwrap();
        let bg = e.document().unwrap().z_order_walk()[0];
        // Convert creates the source asset ENGINE-SIDE: no journal
        // `assetId`/`samples` reference exists — only document.json's
        // smartObject assetHash points at it (the shape the old GC missed).
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "0b70de00-0000-4000-8000-0000000000b2",
                "kind": "smartobject.convert",
                "params": {"layerIds": [bg.as_uuid().to_string()],
                            "newLayerId": "0b70de00-0000-4000-8000-0000000000b3"},
            })
            .to_string(),
        )
        .unwrap();
        let so =
            LayerId::from_uuid(Uuid::parse_str("0b70de00-0000-4000-8000-0000000000b3").unwrap());
        let crate::graph::node::SmartSource::Embedded(asset) = *e
            .document()
            .unwrap()
            .layer(so)
            .unwrap()
            .smart_object()
            .unwrap()
            .source();
        let entry = format!("assets/{asset}.image");
        let names = |bytes: &[u8]| -> Vec<String> {
            let mut za = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).unwrap();
            (0..za.len())
                .map(|i| za.by_index(i).unwrap().name().to_string())
                .collect()
        };

        // Save 1 (fresh session) → load 1.
        let bytes1 = save_package_bytes(&e).unwrap();
        assert!(
            names(&bytes1).contains(&entry),
            "first save carries the source"
        );
        let loaded = load_package_bytes_with(&assembly, &bytes1).unwrap();

        // Save 2 — the reviewer's empty-script leg: reopened engine, no
        // new commands, `is_pending` cleared. The old GC dropped the
        // entry here.
        let bytes2 = save_package_bytes(&loaded.engine).unwrap();
        assert!(
            names(&bytes2).contains(&entry),
            "B2: the document-referenced source asset survives the save-after-reopen GC"
        );

        // Load 2 — died `error.io.read_failed` before the fix — and one
        // more full cycle for good measure.
        let reloaded = load_package_bytes_with(&assembly, &bytes2).unwrap();
        assert_eq!(
            reloaded.engine.state_hash_persistent(),
            e.state_hash_persistent(),
            "third open reproduces the state"
        );
        let bytes3 = save_package_bytes(&reloaded.engine).unwrap();
        assert!(names(&bytes3).contains(&entry));
        let final_open = load_package_bytes_with(&assembly, &bytes3).unwrap();
        assert_eq!(
            final_open.engine.state_hash_persistent(),
            e.state_hash_persistent()
        );
    }
}
