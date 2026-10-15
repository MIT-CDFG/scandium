// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §2.4 (`TileMap` — sparse, copy-on-write tile storage), §3.3
//! (copy-on-write and reference counting); DD ⑲.
//!
//! [NORMATIVE, ES §2.4] An absent tile reads as **all-zero storage scalars**
//! in every channel (fully transparent for alpha-bearing models). Map
//! iteration is **unordered**; any consumer requiring determinism sorts
//! coordinates by `(y, x)` ascending within one lod (`region_hash`, the
//! package writer, golden dumps).
//!
//! [NORMATIVE, ES §3.3] Every tile is `Arc<TileStorage>`; undo states,
//! snapshots, the journal, and live state share tiles, and an edit clones a
//! tile only when it is about to mutate a shared one. [`TileMap::make_mut`]
//! is the **single COW entry point** — and it is `pub(crate)`, so the command
//! engine (slice 2) is the only public door to mutation (D-48). Slice 2's
//! `DeltaRecorder` builds ES §10.4 tile-delta undo directly on these Arcs
//! (`before`/`after` captures are cheap Arc clones — never whole-layer
//! copies).

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::channel::ChannelKind;
use super::model::ColorModelKind;
use super::storage::TileStorage;
use super::TILE_I32;
use crate::geom::IRect;

/// Tile grid coordinate (ES §2.4). `lod` is carried redundantly so the coord
/// stays self-describing in caches and deltas; the owning buffer level
/// debug-asserts the match (ES §2.4 invariant).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct TileCoord {
    pub lod: u8,
    pub x: i32,
    pub y: i32,
}

impl TileCoord {
    pub const fn new(lod: u8, x: i32, y: i32) -> TileCoord {
        TileCoord { lod, x, y }
    }

    /// The pixel rect this coord covers in its own lod's pixel space:
    /// `[x*256, (x+1)*256) × [y*256, (y+1)*256)` (ES §3.1; the `2^lod`
    /// document-space scaling is the mip layer's concern, §3.5).
    pub fn pixel_rect(&self) -> IRect {
        IRect::new(
            self.x * TILE_I32,
            self.y * TILE_I32,
            (self.x + 1) * TILE_I32,
            (self.y + 1) * TILE_I32,
        )
    }
}

/// Sparse map of tiles for one image surface at one LOD (ES §2.4). Empty
/// (absent) tiles read as fully transparent and cost nothing.
#[derive(Clone)]
pub struct TileMap {
    /// COW: Arcs shared across undo/snapshots (ES §3.3).
    tiles: HashMap<TileCoord, Arc<TileStorage>>,
    kind: ChannelKind,
    model: ColorModelKind,
}

impl TileMap {
    /// A new empty map. The depth must be storage-backed in this build
    /// (`u16` needs the `depth-u16` feature; `f16` is paused — see
    /// [`super::channel`]).
    ///
    /// # Panics
    ///
    /// Panics when `kind` is not storage-backed in this build. Slice-2
    /// design constraint (review minor 6, lane log): this constructor must
    /// never be reachable with an unbacked depth from the command surface
    /// or the C ABI — `Document::new` already refuses such depths with the
    /// typed `GraphError::DepthNotStorageBacked` before any map exists, and
    /// every future public door must do the same.
    pub fn new(kind: ChannelKind, model: ColorModelKind) -> TileMap {
        assert!(
            kind.is_storage_backed(),
            "depth {kind:?} is not storage-backed in this build (feature-gated or paused)"
        );
        TileMap {
            tiles: HashMap::new(),
            kind,
            model,
        }
    }

    pub fn kind(&self) -> ChannelKind {
        self.kind
    }

    pub fn model(&self) -> ColorModelKind {
        self.model
    }

    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    /// Read-only access; absent → `None` (transparent, ES §2.4).
    pub fn get(&self, c: TileCoord) -> Option<&Arc<TileStorage>> {
        self.tiles.get(&c)
    }

    /// Unordered iteration (ES §2.4) — determinism-requiring consumers sort
    /// by `(y, x)`.
    pub fn iter(&self) -> impl Iterator<Item = (TileCoord, &Arc<TileStorage>)> {
        self.tiles.iter().map(|(c, t)| (*c, t))
    }

    /// Content identity of the tile at `c` (absent tiles are the transparent
    /// uniform, ES §2.4).
    pub fn content_hash_at(&self, c: TileCoord) -> crate::hash::ContentHash {
        match self.tiles.get(&c) {
            Some(t) => t.content_hash(self.kind, self.model),
            None => {
                TileStorage::transparent(self.kind, self.model).content_hash(self.kind, self.model)
            }
        }
    }

    /// Get a uniquely-owned, mutable tile, allocating or COW-cloning as
    /// needed. **The single COW entry point** (ES §2.4/§3.3): if the Arc is
    /// shared (held by an undo state or snapshot), it is cloned first so the
    /// edit never mutates history; a `Uniform` materializes to a dense
    /// payload before mutable access.
    #[cfg_attr(not(test), allow(dead_code))] // slice-2 command-engine door (in-crate tests exercise it today)
    pub(crate) fn make_mut(&mut self, c: TileCoord) -> &mut TileStorage {
        let kind = self.kind;
        let model = self.model;
        let arc = self
            .tiles
            .entry(c)
            .or_insert_with(|| Arc::new(TileStorage::transparent(kind, model)));
        let t = Arc::make_mut(arc); // clones iff strong_count > 1
        if let TileStorage::Uniform { value } = *t {
            *t = TileStorage::materialize(value, kind, model); // splat (§2.3)
        }
        t
    }

    /// Install a shared payload at `c` (fill fast paths writing `Uniform`
    /// directly, slice-2 delta restore swapping Arcs, ES §3.3).
    ///
    /// Debug guard (review minor 2): a pixel payload whose depth/model tags
    /// differ from this map's is a cross-context aliasing bug (tile content
    /// hashes cover bytes only — encoding context comes from the map).
    /// `Uniform` carries no tags and is exempt.
    #[cfg_attr(not(test), allow(dead_code))] // slice-2 command-engine door (in-crate tests exercise it today)
    pub(crate) fn insert_arc(&mut self, c: TileCoord, tile: Arc<TileStorage>) {
        debug_assert!(
            tile.kind().is_none_or(|k| k == self.kind),
            "insert_arc: payload depth {:?} does not match map depth {:?} — storage variants \
             must match their owning map's (kind, model)",
            tile.kind(),
            self.kind
        );
        debug_assert!(
            tile.model().is_none_or(|m| m == self.model),
            "insert_arc: payload model {:?} does not match map model {:?} — storage variants \
             must match their owning map's (kind, model)",
            tile.model(),
            self.model
        );
        self.tiles.insert(c, tile);
    }

    /// Remove the tile at `c` (back to absent/transparent).
    #[cfg_attr(not(test), allow(dead_code))] // slice-2 command-engine door (in-crate tests exercise it today)
    pub(crate) fn remove(&mut self, c: TileCoord) -> Option<Arc<TileStorage>> {
        self.tiles.remove(&c)
    }

    /// Drop tiles that became fully transparent and collapse solid tiles to
    /// `Uniform` (sparsity maintenance, ES §2.4). CP-0.1 exposes the
    /// full-sweep form (document load / explicit compaction); the
    /// commit-time form restricted to a command's touched coords arrives
    /// with the `DeltaRecorder` (slice 2, §10.1).
    #[cfg_attr(not(test), allow(dead_code))] // slice-2 command-engine door (in-crate tests exercise it today)
    pub(crate) fn prune_transparent(&mut self) {
        let coords: Vec<TileCoord> = self.tiles.keys().copied().collect();
        for c in coords {
            self.prune_at(c);
        }
    }

    /// Prune/collapse one coordinate (the restricted per-coord step).
    #[cfg_attr(not(test), allow(dead_code))] // slice-2 command-engine door (in-crate tests exercise it today)
    pub(crate) fn prune_at(&mut self, c: TileCoord) {
        let Some(arc) = self.tiles.get(&c) else {
            return;
        };
        // 1. Transparency scan: bitwise zero → remove (absent reads the same).
        if arc.is_bitwise_zero(self.kind, self.model) {
            self.tiles.remove(&c);
            return;
        }
        // 2. Uniformity scan: solid → replace payload with Uniform (all-zero
        //    was already handled above).
        if let Some(v) = arc.scan_uniform() {
            self.tiles
                .insert(c, Arc::new(TileStorage::uniform(v, self.kind, self.model)));
        }
    }

    /// Union of occupied tile rects, tile-granular, in this lod's own pixel
    /// space. O(len), no pixel reads (ES §2.4).
    pub fn occupied_bounds(&self) -> IRect {
        self.tiles
            .keys()
            .fold(IRect::EMPTY, |acc, c| acc.union(&c.pixel_rect()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tile::channel::Channel;

    const K: ChannelKind = ChannelKind::U8;
    const M: ColorModelKind = ColorModelKind::Rgba;

    fn coord(x: i32, y: i32) -> TileCoord {
        TileCoord::new(0, x, y)
    }

    #[test]
    fn absent_reads_none_and_zero_hash_is_transparent() {
        let map = TileMap::new(K, M);
        assert!(map.get(coord(3, -2)).is_none());
        let transparent = TileStorage::transparent(K, M).content_hash(K, M);
        assert_eq!(map.content_hash_at(coord(3, -2)), transparent);
    }

    #[test]
    fn cow_shared_tile_is_cloned_and_sibling_unaffected() {
        // The §3.3 worked example, steps 0-3.
        let mut live = TileMap::new(K, M);
        live.make_mut(coord(0, 0))
            .write_pixel_f32(1, 1, [0.2, 0.4, 0.6, 1.0]);

        // "History" holds the same Arc (capture = Arc clone, no pixel copy).
        let before: Arc<TileStorage> = live.get(coord(0, 0)).cloned().unwrap();
        assert_eq!(Arc::strong_count(&before), 2);

        // First touch: strong_count 2 → payload clones; map now holds B.
        live.make_mut(coord(0, 0))
            .write_pixel_f32(1, 1, [0.9, 0.9, 0.9, 1.0]);
        let after = live.get(coord(0, 0)).cloned().unwrap();
        assert!(
            !Arc::ptr_eq(&before, &after),
            "shared tile must be cloned, not mutated"
        );
        // The captured history payload kept its bytes.
        assert_eq!(
            before.read_pixel_f32(1, 1, M),
            [
                u8::from_f32(0.2).to_f32(),
                u8::from_f32(0.4).to_f32(),
                u8::from_f32(0.6).to_f32(),
                1.0
            ]
        );
        // A cloned sibling map (snapshot) is likewise unaffected by further
        // edits to the live map.
        let snapshot = live.clone();
        let snap_hash = snapshot.content_hash_at(coord(0, 0));
        live.make_mut(coord(0, 0))
            .write_pixel_f32(5, 5, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(snapshot.content_hash_at(coord(0, 0)), snap_hash);
        assert_ne!(live.content_hash_at(coord(0, 0)), snap_hash);
    }

    #[test]
    fn cow_unshared_tile_mutates_in_place() {
        // §3.3 step 3: later dabs in the same stroke pay no clone.
        let mut map = TileMap::new(K, M);
        map.make_mut(coord(0, 0))
            .write_pixel_f32(0, 0, [0.1, 0.1, 0.1, 1.0]);
        let p0 = Arc::as_ptr(map.get(coord(0, 0)).unwrap());
        map.make_mut(coord(0, 0))
            .write_pixel_f32(0, 1, [0.2, 0.2, 0.2, 1.0]);
        let p1 = Arc::as_ptr(map.get(coord(0, 0)).unwrap());
        assert_eq!(p0, p1, "unshared tile must mutate in place (no clone)");
    }

    #[test]
    fn content_keys_stable_under_identical_content() {
        let mut a = TileMap::new(K, M);
        let mut b = TileMap::new(K, M);
        for (x, y, px) in [
            (0, 0, [0.5, 0.0, 0.0, 1.0]),
            (300, 5, [0.0, 0.5, 0.0, 0.25]),
        ] {
            let c = coord(x / 256, y / 256);
            a.make_mut(c)
                .write_pixel_f32((x % 256) as usize, (y % 256) as usize, px);
            b.make_mut(c)
                .write_pixel_f32((x % 256) as usize, (y % 256) as usize, px);
        }
        for (c, _) in a.iter() {
            assert_eq!(a.content_hash_at(c), b.content_hash_at(c));
        }
        // COW cloning does not change identity either.
        let shared = a.clone();
        for (c, _) in shared.iter() {
            assert_eq!(shared.content_hash_at(c), a.content_hash_at(c));
        }
    }

    #[test]
    fn make_mut_materializes_uniform_with_same_identity() {
        let mut map = TileMap::new(K, M);
        map.insert_arc(
            coord(2, 2),
            Arc::new(TileStorage::uniform([0.5, 0.25, 0.0, 1.0], K, M)),
        );
        let h_uniform = map.content_hash_at(coord(2, 2));
        let t = map.make_mut(coord(2, 2));
        assert!(!t.is_uniform(), "make_mut must hand out a dense payload");
        assert_eq!(
            map.content_hash_at(coord(2, 2)),
            h_uniform,
            "materialization preserves identity"
        );
    }

    #[test]
    fn prune_removes_zero_and_collapses_solid() {
        let mut map = TileMap::new(K, M);
        // A tile written then erased back to zero.
        map.make_mut(coord(0, 0))
            .write_pixel_f32(3, 3, [0.5, 0.5, 0.5, 1.0]);
        map.make_mut(coord(0, 0))
            .write_pixel_f32(3, 3, [0.0, 0.0, 0.0, 0.0]);
        // A dense but solid tile.
        map.insert_arc(
            coord(1, 0),
            Arc::new(TileStorage::materialize([0.25, 0.5, 0.75, 1.0], K, M)),
        );
        // A genuinely mixed tile.
        map.make_mut(coord(2, 0))
            .write_pixel_f32(9, 9, [1.0, 0.0, 0.0, 1.0]);

        map.prune_transparent();

        assert!(
            map.get(coord(0, 0)).is_none(),
            "all-zero tile pruned to absent"
        );
        let collapsed = map.get(coord(1, 0)).unwrap();
        assert!(
            collapsed.is_uniform(),
            "solid dense tile collapses to Uniform"
        );
        assert!(!map.get(coord(2, 0)).unwrap().is_uniform());
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn prune_preserves_identity() {
        let mut map = TileMap::new(K, M);
        map.insert_arc(
            coord(4, 4),
            Arc::new(TileStorage::materialize([0.1, 0.2, 0.3, 1.0], K, M)),
        );
        let before = map.content_hash_at(coord(4, 4));
        map.prune_transparent();
        assert!(map.get(coord(4, 4)).unwrap().is_uniform());
        assert_eq!(map.content_hash_at(coord(4, 4)), before);
    }

    #[test]
    fn occupied_bounds_unions_tile_rects_including_negative() {
        let mut map = TileMap::new(K, M);
        assert_eq!(map.occupied_bounds(), IRect::EMPTY);
        map.make_mut(coord(0, 0))
            .write_pixel_f32(0, 0, [0.5, 0.0, 0.0, 1.0]);
        map.make_mut(coord(-1, 2))
            .write_pixel_f32(0, 0, [0.5, 0.0, 0.0, 1.0]);
        let b = map.occupied_bounds();
        assert_eq!(b, IRect::new(-256, 0, 256, 768));
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "must match their owning map's (kind, model)")]
    fn insert_arc_rejects_mismatched_storage_variants_in_debug() {
        // Review minor 2: encoding context lives on the map; a payload of a
        // different depth aliased into this map would carry the wrong
        // meaning for every read.
        let mut map = TileMap::new(K, M);
        let alien = Arc::new(TileStorage::materialize([0.5; 4], ChannelKind::F32, M));
        map.insert_arc(coord(0, 0), alien);
    }

    #[test]
    fn coord_pixel_rect() {
        assert_eq!(
            TileCoord::new(0, 0, 0).pixel_rect(),
            IRect::new(0, 0, 256, 256)
        );
        assert_eq!(
            TileCoord::new(0, -1, 1).pixel_rect(),
            IRect::new(-256, 256, 0, 512)
        );
    }
}
