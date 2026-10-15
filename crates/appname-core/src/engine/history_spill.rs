// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §10.4 (byte accounting + enforcement order: "(1) compress +
//! spill the coldest (oldest) resident DeltaSets; (2) if still over
//! `bytes_limit` or `depth_limit`, drop whole steps from the bottom of the
//! undo stack; (3) the most recent 8 steps SHOULD never be dropped"), §3.8
//! (zstd-3 default / lz4 under backlog), §3.4 ("Historical snapshots and
//! undo deltas are never paged behind their backs — their Arcs pin RAM
//! until history spill compresses the delta itself").
//!
//! A spilled step keeps its SKELETON resident (coords, bounds pairs, tree
//! ops, after-Arcs — the afters are live-shared for below-cursor steps and
//! cost nothing) and moves the accounted `before` payloads to the scratch
//! disk. §10.4: a spilled delta re-accounts at its COMPRESSED size. Revive
//! is transparent at undo time: the engine rebuilds the `DeltaSet` from the
//! blobs (identity-stable — spill bytes are the §3.8 hash encoding) and
//! frees them.
//!
//! Divergences (lane log): the §3.8 adaptive-codec trigger is frame-clocked
//! ("backlog > tiles demoted during the last 2 frames"); headless core has
//! no frame clock, so the proxy is tiles-demoted-this-enforcement-pass >
//! [`BACKLOG_TILES`] — the remainder of the pass compresses lz4, and
//! idle-time recompression to zstd-3 arrives with the maintenance-tick
//! machinery. Accounting counts tile payload bytes (dense nominal ==
//! `byte_len`; `Uniform` befores are metadata-only and stay resident); the
//! spec's "+ serialized command JSON length" term is omitted as noise
//! (journal envelopes are engine-lifetime state, not per-step cost).

use std::sync::Arc;

use uuid::Uuid;

use crate::engine::delta::{DeltaSet, TileDelta, TreeOp};
use crate::geom::IRect;
use crate::io::scratch::{ScratchDisk, ScratchRef};
use crate::io::tile_chunk::{decode_payload_le, encode_payload_le};
use crate::tile::channel::ChannelKind;
use crate::tile::compress::{compress_payload, decompress_payload, CompressedTile, TileCodec};
use crate::tile::model::ColorModelKind;
use crate::tile::storage::TileStorage;
use crate::tile::TILE;

/// Headless proxy for the §3.8 two-frame demotion-backlog trigger.
pub(crate) const BACKLOG_TILES: usize = 512;

/// §10.4 rule 3: the most recent N steps are never dropped.
pub(crate) const NEVER_DROP_FLOOR: usize = 8;

/// Telemetry mirroring the §3.4 `CacheStats` families that apply to the
/// history tier (the render-facing stall/hit families arrive with the
/// live-tile cache).
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct HistoryStats {
    /// Resident accounted bytes across in-process steps (§10.4 formula).
    pub resident_bytes: usize,
    /// Compressed accounted bytes of spilled steps.
    pub spilled_bytes: usize,
    /// Blobs written to scratch over the engine's lifetime.
    pub spills: u64,
    /// Blobs revived (undo across a spilled step).
    pub revives: u64,
    /// Payload compressions performed.
    pub compressions: u64,
    /// Compressions that fell back to lz4 under the backlog proxy.
    pub lz4_under_backlog: u64,
    /// Whole steps dropped from the bottom of the undo stack (§10.4 rule 2).
    pub dropped_steps: u64,
    /// Current scratch-file footprint in bytes.
    pub disk_bytes: u64,
}

pub(crate) struct SpilledTileDelta {
    pub(crate) surface: Uuid,
    pub(crate) coord: crate::tile::map::TileCoord,
    pub(crate) before: SpilledSide,
    pub(crate) after: Arc<TileStorage>,
}

pub(crate) enum SpilledSide {
    /// Metadata-only befores (`Uniform`, incl. the shared transparent tile)
    /// stay resident — there is nothing to spill.
    Resident(Arc<TileStorage>),
    Blob {
        r: ScratchRef,
        comp_len: u32,
        kind: ChannelKind,
        model: ColorModelKind,
    },
}

/// A cold step's delta with its `before` payloads on scratch (§10.4).
pub(crate) struct SpilledDeltaSet {
    pub(crate) tiles: Vec<SpilledTileDelta>,
    pub(crate) tree_after: Vec<TreeOp>,
    pub(crate) bounds: Vec<(Uuid, IRect, IRect)>,
    /// §10.4: re-accounted at compressed size.
    pub(crate) accounted: usize,
}

/// Resident accounted bytes of a live `DeltaSet` (§10.4 formula over the
/// `before` side; dense `byte_len` == nominal, `Uniform` == 0).
pub(crate) fn resident_accounted(deltas: &DeltaSet) -> usize {
    deltas.tiles.iter().map(|d| d.before.byte_len()).sum()
}

/// Compress + spill one step's befores. Caller guarantees the step is not
/// in its undone state (`tree_stash` all `None`) — only below-cursor steps
/// are eligible, which the engine's enforcement upholds.
pub(crate) fn spill_step(
    deltas: DeltaSet,
    scratch: &mut ScratchDisk,
    demoted_so_far: &mut usize,
    stats: &mut HistoryStats,
) -> std::io::Result<SpilledDeltaSet> {
    debug_assert!(
        deltas.tree_stash.iter().all(Option::is_none),
        "spill targets below-cursor steps only (stash empty)"
    );
    let mut tiles = Vec::with_capacity(deltas.tiles.len());
    let mut accounted = 0usize;
    for d in deltas.tiles {
        let TileDelta {
            surface,
            coord,
            before,
            after,
        } = d;
        let before = match (before.kind(), before.model()) {
            (Some(kind), Some(model)) => {
                let mut raw = vec![0u8; before.byte_len()];
                encode_payload_le(&before, kind, model, &mut raw);
                let codec = if *demoted_so_far > BACKLOG_TILES {
                    stats.lz4_under_backlog += 1;
                    TileCodec::Lz4
                } else {
                    TileCodec::Zstd
                };
                let compressed = compress_payload(&raw, codec);
                stats.compressions += 1;
                *demoted_so_far += 1;
                let comp_len = compressed.bytes.len() as u32;
                let r = scratch.alloc(&compressed.bytes, raw.len() as u32, codec)?;
                stats.spills += 1;
                accounted += comp_len as usize;
                SpilledSide::Blob {
                    r,
                    comp_len,
                    kind,
                    model,
                }
            }
            // Uniform (incl. transparent): metadata-only, stays resident.
            _ => SpilledSide::Resident(before),
        };
        tiles.push(SpilledTileDelta {
            surface,
            coord,
            before,
            after,
        });
    }
    Ok(SpilledDeltaSet {
        tiles,
        tree_after: deltas.tree_after,
        bounds: deltas.bounds,
        accounted,
    })
}

/// Rebuild the resident `DeltaSet` from scratch blobs (undo revive) and
/// free them. Identity-stable: blob bytes are the §3.8 LE payload encoding.
pub(crate) fn revive_step(
    spilled: SpilledDeltaSet,
    scratch: &mut ScratchDisk,
    stats: &mut HistoryStats,
) -> Result<DeltaSet, String> {
    let mut tiles = Vec::with_capacity(spilled.tiles.len());
    for t in spilled.tiles {
        let before = match t.before {
            SpilledSide::Resident(arc) => arc,
            SpilledSide::Blob {
                r,
                comp_len,
                kind,
                model,
            } => {
                let bytes = scratch
                    .read(&r, comp_len as usize)
                    .map_err(|e| format!("scratch read failed: {e}"))?;
                let raw = decompress_payload(&CompressedTile {
                    algo: r.codec,
                    bytes: bytes.into(),
                    raw_len: r.raw_len as usize,
                })?;
                let mut tile = TileStorage::materialize([0.0; 4], kind, model);
                decode_payload_le(&mut tile, &raw, TILE, TILE, kind, model);
                scratch.free(&r);
                stats.revives += 1;
                Arc::new(tile)
            }
        };
        tiles.push(TileDelta {
            surface: t.surface,
            coord: t.coord,
            before,
            after: t.after,
        });
    }
    let stash = spilled.tree_after.iter().map(|_| None).collect();
    Ok(DeltaSet {
        tiles,
        tree_after: spilled.tree_after,
        tree_stash: stash,
        bounds: spilled.bounds,
    })
}

/// Free a spilled step's blobs without reviving (step dropped, §10.4
/// rule 2).
pub(crate) fn free_step(spilled: &SpilledDeltaSet, scratch: &mut ScratchDisk) {
    for t in &spilled.tiles {
        if let SpilledSide::Blob { r, .. } = &t.before {
            scratch.free(r);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::kind_impl::EngineAssembly;
    use crate::engine::Engine;
    use serde_json::json;

    fn engine_with_fill() -> Engine {
        // Post-fill-swap: production fill lives tools-side; this fixture
        // registers core's reference FillKind explicitly (in-crate door).
        crate::engine::kinds::register(EngineAssembly::new())
            .with_kind(std::sync::Arc::new(crate::engine::kinds::FillKind))
            .build()
    }

    fn doc_new(e: &mut Engine, w: u32, h: u32) {
        let text = json!({
            "schema": "appname.command.v1",
            "id": "5b111000-0000-4000-8000-000000000001",
            "kind": "doc.new",
            "params": {"width": w, "height": h, "background": "white"},
        })
        .to_string();
        e.apply_json(&text).unwrap();
    }

    /// Full-`rect` selection whose coverage differs at one pixel per tile:
    /// fills through it leave DENSE tiles (a solid fill would collapse to
    /// `Uniform` under the §2.4 uniformity scan and account 0 bytes).
    fn variegate_selection(e: &mut Engine, rect: crate::geom::IRect) {
        let doc = e.doc_mut_for_test();
        let sel = doc.selection_mut();
        sel.mask_mut().fill_rect(rect, [1.0, 0.0, 0.0, 0.0]);
        for (tx, ty) in rect.tiles(crate::tile::TILE_I32) {
            let p = crate::geom::IVec2::new(
                (tx * crate::tile::TILE_I32).max(rect.min.x),
                (ty * crate::tile::TILE_I32).max(rect.min.y),
            );
            sel.mask_mut().write_pixel_f32(p, [0.5, 0.0, 0.0, 0.0]);
        }
        sel.set_bounds(rect);
    }

    fn fill(e: &mut Engine, n: u32, gray: f64) {
        let layer = e.document().unwrap().z_order_walk()[0];
        let text = json!({
            "schema": "appname.command.v1",
            "id": format!("5b111000-0000-4000-8000-0001{n:08x}"),
            "kind": "fill",
            "target": {"layerId": layer.as_uuid().to_string()},
            "params": {"content": "color",
                        "color": {"model": "rgb", "comps": [gray, gray * 0.5, 1.0 - gray, 0.0]}},
        })
        .to_string();
        e.apply_json(&text).unwrap();
    }

    #[test]
    fn spill_keeps_history_bounded_and_undo_redo_byte_identical() {
        let mut e = engine_with_fill();
        e.set_scratch_dir(
            std::env::temp_dir().join(format!("appname-spill-{}", uuid::Uuid::new_v4())),
        );
        doc_new(&mut e, 512, 512); // 2x2 tiles, 1 MiB nominal per dense fill delta
                                   // One variegated selection up front: fills stay dense (accounted)
                                   // and the selection is CONSTANT across every hashed state (the
                                   // selection folds into state_hash; test-door edits are unjournaled).
        let canvas = e.document().unwrap().geometry().canvas_rect();
        variegate_selection(&mut e, canvas);
        let mut hashes = vec![e.state_hash()];
        // Budget: ~1.5 dense deltas resident; 6 fills force spills.
        e.set_history_limits(1_600_000, usize::MAX);
        for i in 0..6 {
            fill(&mut e, i, (i as f64 + 1.0) / 7.0);
            hashes.push(e.state_hash());
        }
        let stats = e.history_stats();
        assert!(stats.spills > 0, "budget forced spills: {stats:?}");
        assert!(
            stats.resident_bytes <= 1_600_000,
            "resident stays under budget once spillable steps exist: {stats:?}"
        );
        assert!(stats.disk_bytes > 0);

        // Undo across the spilled steps: byte-identical restore per step.
        for i in (0..6).rev() {
            e.undo().unwrap();
            assert_eq!(
                e.state_hash(),
                hashes[i],
                "undo to state {i} revives byte-identically"
            );
        }
        assert!(
            e.history_stats().revives > 0,
            "spilled steps revived on undo"
        );
        // Redo forward: byte-identical again.
        for (i, want) in hashes.iter().enumerate().skip(1) {
            e.redo().unwrap();
            assert_eq!(e.state_hash(), *want, "redo to state {i}");
        }
    }

    #[test]
    fn depth_limit_drops_from_the_bottom_respecting_the_floor() {
        let mut e = engine_with_fill();
        doc_new(&mut e, 256, 256);
        let canvas = e.document().unwrap().geometry().canvas_rect();
        variegate_selection(&mut e, canvas);
        for i in 0..11 {
            fill(&mut e, 100 + i, (i as f64 + 1.0) / 12.0);
        }
        assert_eq!(e.history().len(), 12, "doc.new + 11 fills in-process");
        e.set_history_limits(usize::MAX, 5);
        // Floor: never below NEVER_DROP_FLOOR resident steps even though the
        // depth limit asks for 5.
        assert_eq!(e.history().len(), NEVER_DROP_FLOOR);
        assert_eq!(
            e.history_stats().dropped_steps as usize,
            12 - NEVER_DROP_FLOOR
        );
        // The dropped prefix behaves exactly like a persisted prefix
        // (suffix-aligned history): undo works down to the base, then
        // answers typed.
        for _ in 0..NEVER_DROP_FLOOR {
            e.undo().unwrap();
        }
        let err = e.undo().unwrap_err();
        assert_eq!(err.code, crate::engine::error::ErrorCode::NothingToUndo);
        // Redo the whole suffix back.
        for _ in 0..NEVER_DROP_FLOOR {
            e.redo().unwrap();
        }
    }

    #[test]
    fn unbounded_engines_never_touch_scratch() {
        let mut e = engine_with_fill();
        doc_new(&mut e, 256, 256);
        let canvas = e.document().unwrap().geometry().canvas_rect();
        variegate_selection(&mut e, canvas);
        for i in 0..4 {
            fill(&mut e, 200 + i, (i as f64 + 1.0) / 5.0);
        }
        let stats = e.history_stats();
        assert_eq!(
            (stats.spills, stats.dropped_steps, stats.disk_bytes),
            (0, 0, 0)
        );
    }

    /// W3-adjacent gigapixel smoke (the formal W3 gate is O-QA's): a
    /// synthetic 12k x 8k document edited under a history budget keeps
    /// accounted history bounded; run manually with
    /// `cargo test --release -p appname-core w03 -- --ignored --nocapture`
    /// and record the process RSS bound in the lane log.
    #[test]
    #[ignore = "manual W3 smoke; run --release and record RSS in the lane log"]
    fn w03_gigapixel_smoke_bounded_history() {
        let mut e = engine_with_fill();
        doc_new(&mut e, 12_000, 8_000);
        e.set_history_limits(128 * 1024 * 1024, usize::MAX);
        let layer = e.document().unwrap().z_order_walk()[0];
        // Region control: one variegated 4096x4096 selection keeps result
        // tiles dense (the accounted case).
        variegate_selection(&mut e, crate::geom::IRect::new(0, 0, 4096, 4096));
        // 10 fills over a 4096x4096 region: 64 MiB nominal history each,
        // 640 MiB unbounded -> must stay near the 128 MiB budget.
        for i in 0..10u32 {
            let sel_gray = (i as f64 + 1.0) / 11.0;
            let text = json!({
                "schema": "appname.command.v1",
                "id": format!("5b111000-0000-4000-8000-0002{i:08x}"),
                "kind": "fill",
                "target": {"layerId": layer.as_uuid().to_string()},
                "params": {"content": "color",
                            "color": {"model": "rgb", "comps": [sel_gray, 0.3, 0.7, 0.0]}},
            })
            .to_string();
            e.apply_json(&text).unwrap();
            let s = e.history_stats();
            println!(
                "fill {i}: resident {} MiB, spilled {} MiB, disk {} MiB, spills {}, lz4 {}",
                s.resident_bytes / (1024 * 1024),
                s.spilled_bytes / (1024 * 1024),
                s.disk_bytes / (1024 * 1024),
                s.spills,
                s.lz4_under_backlog,
            );
        }
        let s = e.history_stats();
        assert!(s.spills > 0);
        assert!(
            s.resident_bytes <= 128 * 1024 * 1024,
            "resident bounded by budget: {s:?}"
        );
        // Undo the last three across the spill boundary as a correctness probe.
        let h = e.state_hash();
        e.undo().unwrap();
        e.undo().unwrap();
        e.redo().unwrap();
        e.redo().unwrap();
        assert_eq!(e.state_hash(), h);
    }
}
