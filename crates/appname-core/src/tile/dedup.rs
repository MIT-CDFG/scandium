// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §3.8 (content addressing — the dedup index).
//!
//! Identical tile content is stored once: at command commit, changed tiles
//! are hashed on the worker pool and interned here; a hash hit swaps the
//! caller's Arc for the existing payload and releases the duplicate.
//! [NORMATIVE, ES §3.8] BLAKE3-256 equality **is** identity — no byte-compare
//! on hash hit. Weak references keep the index from pinning payloads alive.
//!
//! The commit-pipeline caller arrives with the command engine (slice 2); the
//! structure and its contract are part of the content-addressing substrate.

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use super::storage::TileStorage;
use crate::hash::ContentHash;

/// Dedup index: content hash → weak ref to a live payload (ES §3.8).
///
/// **Scoping requirement (review minor 2, slice-2 design input):** one
/// `DedupTable` MUST be scoped to a single `(ChannelKind, ColorModelKind)`
/// encoding context. Tile content hashes cover payload bytes only, and
/// byte-identical payloads exist across contexts (e.g. a u8×Rgba tile and
/// an f32×Gray tile are both 256 KiB and can carry the same bytes), so a
/// cross-context table would alias payloads with different meaning. The
/// §2.5 `region_hash` framing adds the kind/model tags precisely because
/// tile hashes alone do not. `intern` carries a debug-time guard; the
/// structural per-context scoping (table-per-surface-encoding in the
/// commit pipeline) is a named slice-2 requirement in the lane log.
#[derive(Default)]
pub struct DedupTable {
    by_hash: HashMap<ContentHash, Weak<TileStorage>>,
}

impl DedupTable {
    pub fn new() -> DedupTable {
        DedupTable::default()
    }

    /// At command commit, per changed tile: if a live payload with this hash
    /// exists, return it (the caller swaps its Arc and releases the duplicate
    /// to the pool); else register the new payload's Weak (ES §3.8).
    ///
    /// Debug guard (review minor 2): a hash hit whose stored payload carries
    /// different depth/model tags than the incoming tile is a cross-context
    /// aliasing bug — the table was not scoped per `(kind, model)` (see the
    /// struct doc). `Uniform` payloads carry no tags, so only pixel-variant
    /// pairs are checkable here; the structural fix is slice 2's.
    #[cfg_attr(not(test), allow(dead_code))] // slice-2 command-engine door (in-crate tests exercise it today)
    pub(crate) fn intern(
        &mut self,
        hash: ContentHash,
        tile: &Arc<TileStorage>,
    ) -> Option<Arc<TileStorage>> {
        if let Some(existing) = self.by_hash.get(&hash).and_then(Weak::upgrade) {
            if let (Some(ek), Some(ik)) = (existing.kind(), tile.kind()) {
                debug_assert!(
                    ek == ik,
                    "DedupTable hash hit across storage depths ({ek:?} vs {ik:?}) — tables must \
                     be scoped per (kind, model) [slice-2 requirement]"
                );
            }
            if let (Some(em), Some(im)) = (existing.model(), tile.model()) {
                debug_assert!(
                    em == im,
                    "DedupTable hash hit across color models ({em:?} vs {im:?}) — tables must \
                     be scoped per (kind, model) [slice-2 requirement]"
                );
            }
            return Some(existing);
        }
        self.by_hash.insert(hash, Arc::downgrade(tile));
        None
    }

    /// Drop entries whose payloads died (maintenance sweep).
    #[cfg_attr(not(test), allow(dead_code))] // slice-2 command-engine door (in-crate tests exercise it today)
    pub(crate) fn sweep(&mut self) {
        self.by_hash.retain(|_, w| w.strong_count() > 0);
    }

    pub fn len(&self) -> usize {
        self.by_hash.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_hash.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tile::channel::ChannelKind;
    use crate::tile::model::ColorModelKind;

    const K: ChannelKind = ChannelKind::U8;
    const M: ColorModelKind = ColorModelKind::Rgba;

    #[test]
    fn intern_returns_existing_live_payload() {
        let mut table = DedupTable::new();
        let a = Arc::new(TileStorage::materialize([0.5, 0.5, 0.5, 1.0], K, M));
        let h = a.content_hash(K, M);
        assert!(table.intern(h, &a).is_none(), "first sighting registers");

        // An identical payload arriving later dedups to the first Arc.
        let b = Arc::new(TileStorage::materialize([0.5, 0.5, 0.5, 1.0], K, M));
        let hit = table.intern(b.content_hash(K, M), &b).expect("hash hit");
        assert!(Arc::ptr_eq(&hit, &a));
    }

    #[test]
    fn dead_payloads_do_not_resurrect() {
        let mut table = DedupTable::new();
        let a = Arc::new(TileStorage::materialize([0.25, 0.0, 0.0, 1.0], K, M));
        let h = a.content_hash(K, M);
        table.intern(h, &a);
        drop(a);

        // The weak ref cannot upgrade; the newcomer re-registers.
        let b = Arc::new(TileStorage::materialize([0.25, 0.0, 0.0, 1.0], K, M));
        assert!(table.intern(h, &b).is_none());
        let hit = table.intern(h, &b).expect("b is live now");
        assert!(Arc::ptr_eq(&hit, &b));
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "scoped per (kind, model)")]
    fn cross_context_byte_collision_is_flagged_in_debug() {
        use crate::tile::model::model;
        use crate::tile::tile::Tile;

        // The reviewer-proved premise: byte-identical payloads exist across
        // (kind, model) contexts. u8×Rgba = 256·256·4 scalars × 1 B and
        // f32×Gray = 256·256 scalars × 4 B are both 262144 payload bytes;
        // fill both with the byte pattern 0x3F.
        let u8_rgba: Arc<TileStorage> = Arc::new(Tile::<u8, model::Rgba>::filled([0x3F; 4]).into());
        let f32_gray: Arc<TileStorage> = Arc::new(
            Tile::<f32, model::Gray>::filled([f32::from_bits(0x3F3F_3F3F), 0.0, 0.0, 0.0]).into(),
        );
        let h1 = u8_rgba.content_hash(ChannelKind::U8, ColorModelKind::Rgba);
        let h2 = f32_gray.content_hash(ChannelKind::F32, ColorModelKind::Gray);
        assert_eq!(h1, h2, "collision premise: identical bytes, identical hash");

        // A table shared across contexts aliases them; the debug guard on
        // the hash hit names the scoping requirement.
        let mut table = DedupTable::new();
        assert!(table.intern(h1, &u8_rgba).is_none());
        let _ = table.intern(h2, &f32_gray);
    }

    #[test]
    fn sweep_drops_dead_entries() {
        let mut table = DedupTable::new();
        let a = Arc::new(TileStorage::materialize([0.1, 0.0, 0.0, 1.0], K, M));
        table.intern(a.content_hash(K, M), &a);
        assert_eq!(table.len(), 1);
        drop(a);
        table.sweep();
        assert!(table.is_empty());
    }
}
