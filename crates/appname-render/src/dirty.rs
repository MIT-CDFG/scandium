// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §3.7 (dirty-region tracking and invalidation — `RegionSet`
//! coalescing with 8 px slop and the 8-rect cap, `take()` swap semantics,
//! the [NORMATIVE] invalidation rules); DD ⑧; plan §2.2 "dirty-region
//! propagation from command receipts".
//!
//! Edits mark dirty rects; only dirty tiles recomposite. The types here are
//! the render-side half of the receipt contract:
//!
//! * [`DirtyRecord`] `{ layer, rect }` is the unit a command receipt
//!   produces — one document-space rect (lod 0) a command changed on one
//!   layer. O-CORE derives these from the §10.1 recorder (each touched
//!   `TileCoord::pixel_rect()`, or the node's full bounds for property/
//!   structural changes per the §3.7 invalidation rules) and carries them
//!   on the apply receipt; [`DirtyTracker::feed`] consumes them. The
//!   receipt-side field is next-wave O-CORE wiring (this lane's report
//!   names the exact shape); until then callers mark by hand.
//! * [`DirtyTracker`] accumulates marks per layer; [`DirtyTracker::take`]
//!   atomically swaps the accumulated map out (§3.7: marks landing during
//!   a render belong to the next frame — nothing lost, nothing rendered
//!   twice).
//! * [`DirtyMap`] is the taken set the compositor consumes. At CP-0.3 the
//!   composite output is flat (no per-node cache until the §4.6 DAG at
//!   CP-0.5), so consumption unions across layers — a dirty rect on ANY
//!   composited layer invalidates the flat result there. The per-layer
//!   granularity is kept in the API because the CP-0.5 DAG consumes it
//!   per node.
//!
//! Determinism: insertion-ordered `Vec`s throughout (no hash-order
//! iteration); identical mark sequences produce identical maps, rect lists,
//! and tile sets.

use appname_core::{IRect, LayerId, TILE_I32};

/// §3.7 `COALESCE_SLOP`: rects within 8 px merge (absorbs rect-per-dab
/// fragmentation without gluing distant edits).
pub const COALESCE_SLOP: i32 = 8;

/// §3.7 cap: a set exceeding 8 rects collapses to its bounding box (bounds
/// per-frame set algebra).
pub const REGION_SET_CAP: usize = 8;

/// One receipt-derived invalidation: the document-space rect (lod 0) a
/// command changed on one layer (ES §3.7; plan §2.2 receipt propagation).
///
/// **★ FEATHERED MASKS WIDEN THE RECT — a requirement §11.2 Sub-slice B
/// creates for the receipt wiring that is not yet in the tree.** A
/// `mask.edit` at `q` on a mask carrying `feather_px > 0` changes the
/// composited result throughout `q.inflate(Rmask)`, where
/// `Rmask = appname_core::feather_reach_px(feather_px) = ⌈feather_px⌉`
/// (the FI-10 Option B identity — the blur's support and the dirty margin
/// are the SAME number, which is the whole reason feather could be
/// served). A receipt that marked the bare edit rect would leave every
/// tile in the surrounding `Rmask` ring STALE on
/// [`crate::recomposite_dirty_with`] — silently wrong pixels, strictly
/// worse than the refusal Sub-slice B replaced.
///
/// This is NOT yet a live bug: the receipt-side derivation is still
/// next-wave O-CORE wiring (module doc above) and every current caller
/// marks by hand. The §4.6 DAG door is already safe by a different
/// mechanism — `dag::coverage_fold` folds the whole `Rmask`-inflated
/// coverage-tile neighbourhood into the cache key, pinned by
/// `dag::coverage_fold_tests`. It is `recomposite_dirty_with`, which
/// consumes whatever the caller marks, that depends on this note.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct DirtyRecord {
    pub layer: LayerId,
    pub rect: IRect,
}

/// A coalesced set of dirty rects (ES §3.7 `RegionSet`), ≤
/// [`REGION_SET_CAP`] entries.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct RegionSet {
    rects: Vec<IRect>,
}

impl RegionSet {
    pub fn new() -> RegionSet {
        RegionSet { rects: Vec::new() }
    }

    /// The §3.7 coalescing algorithm, literally:
    /// 1. ignore empty rects;
    /// 2. merge pass to fixpoint — while `r` inflated by the slop
    ///    intersects an existing rect, remove that rect and union it into
    ///    `r` (a grown `r` may touch others, so the scan restarts);
    /// 3. insert; over-cap collapses the whole set to its bounding box.
    pub fn add(&mut self, r: IRect) {
        if r.is_empty() {
            return;
        }
        let mut r = r;
        loop {
            let probe = r.inflate(COALESCE_SLOP, COALESCE_SLOP);
            match self
                .rects
                .iter()
                .position(|e| !probe.intersect(e).is_empty())
            {
                Some(i) => {
                    let e = self.rects.remove(i);
                    r = r.union(&e);
                }
                None => break,
            }
        }
        self.rects.push(r);
        if self.rects.len() > REGION_SET_CAP {
            let bb = self.bounding_box();
            self.rects.clear();
            self.rects.push(bb);
        }
    }

    /// The coalesced rects (each non-empty; pairwise farther than the slop).
    pub fn rects(&self) -> &[IRect] {
        &self.rects
    }

    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }

    /// Union bounding box ([`IRect::EMPTY`] when empty).
    pub fn bounding_box(&self) -> IRect {
        self.rects.iter().fold(IRect::EMPTY, |acc, r| acc.union(r))
    }

    /// Does any rect intersect `r`?
    pub fn intersects(&self, r: &IRect) -> bool {
        self.rects.iter().any(|e| !e.intersect(r).is_empty())
    }

    /// Deduplicated tile coords (size [`TILE_I32`]) covered by the set, in
    /// deterministic row-major order — the §3.7 note "consumers derive tile
    /// sets via `IRect::tiles`".
    pub fn tiles(&self) -> Vec<(i32, i32)> {
        let mut out: Vec<(i32, i32)> = Vec::new();
        for r in &self.rects {
            out.extend(r.tiles(TILE_I32));
        }
        out.sort_by_key(|&(tx, ty)| (ty, tx));
        out.dedup();
        out
    }

    /// Merge another set into this one (re-coalescing).
    pub fn union_with(&mut self, other: &RegionSet) {
        for r in other.rects() {
            self.add(*r);
        }
    }
}

/// Per-layer dirty accumulation (ES §3.7 `DirtyTracker`).
///
/// Insertion-ordered by first mark (deterministic; `LayerId` carries no
/// order of its own worth imposing here).
#[derive(Default, Debug)]
pub struct DirtyTracker {
    per_layer: Vec<(LayerId, RegionSet)>,
}

impl DirtyTracker {
    pub fn new() -> DirtyTracker {
        DirtyTracker {
            per_layer: Vec::new(),
        }
    }

    /// Mark `r` dirty on `layer` (pixel edits mark the edited rect;
    /// property changes mark the node's full bounds — §3.7 invalidation
    /// rules; mask/op edits mark the OWNING layer).
    pub fn mark(&mut self, layer: LayerId, r: IRect) {
        if r.is_empty() {
            return;
        }
        match self.per_layer.iter_mut().find(|(l, _)| *l == layer) {
            Some((_, set)) => set.add(r),
            None => {
                let mut set = RegionSet::new();
                set.add(r);
                self.per_layer.push((layer, set));
            }
        }
    }

    /// Consume receipt-derived records (plan §2.2: "dirty-region
    /// propagation from command receipts" — the feed door those receipts
    /// call).
    pub fn feed<'a>(&mut self, records: impl IntoIterator<Item = &'a DirtyRecord>) {
        for rec in records {
            self.mark(rec.layer, rec.rect);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.per_layer.is_empty()
    }

    /// §3.7 `take()` semantics: atomically swap the accumulated map out,
    /// leaving an empty tracker — marks arriving after the take belong to
    /// the next render tick.
    pub fn take(&mut self) -> DirtyMap {
        DirtyMap {
            per_layer: std::mem::take(&mut self.per_layer),
        }
    }
}

/// The taken per-layer dirty map a render tick consumes (ES §3.7
/// `take()` return).
#[derive(Default, Debug)]
pub struct DirtyMap {
    per_layer: Vec<(LayerId, RegionSet)>,
}

impl DirtyMap {
    /// Construct directly from records (one-shot callers that skip the
    /// tracker).
    pub fn from_records<'a>(records: impl IntoIterator<Item = &'a DirtyRecord>) -> DirtyMap {
        let mut t = DirtyTracker::new();
        t.feed(records);
        t.take()
    }

    pub fn is_empty(&self) -> bool {
        self.per_layer.is_empty()
    }

    /// Per-layer view (CP-0.5 DAG consumption granularity).
    pub fn layers(&self) -> impl Iterator<Item = (LayerId, &RegionSet)> {
        self.per_layer.iter().map(|(l, s)| (*l, s))
    }

    /// Union across layers — the CP-0.3 flat-composite consumption (module
    /// doc): any composited layer's dirt invalidates the flat result there.
    pub fn union(&self) -> RegionSet {
        let mut out = RegionSet::new();
        for (_, set) in &self.per_layer {
            out.union_with(set);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Layer ids in these tests: `LayerId::nil()` plus `LayerId::new()`
    // (random v4) where a DISTINCT id is all that matters — tracker
    // behavior keys on identity and insertion order, never on id value,
    // so the tests stay deterministic.

    #[test]
    fn slop_merges_and_distance_keeps_separate() {
        let mut s = RegionSet::new();
        s.add(IRect::new(0, 0, 10, 10));
        // Within 8 px (gap of 5): merges.
        s.add(IRect::new(15, 0, 25, 10));
        assert_eq!(s.rects(), &[IRect::new(0, 0, 25, 10)]);
        // Farther than 8 px (gap of 20): stays separate.
        s.add(IRect::new(45, 0, 55, 10));
        assert_eq!(s.rects().len(), 2);
    }

    #[test]
    fn merge_runs_to_fixpoint_chain() {
        let mut s = RegionSet::new();
        s.add(IRect::new(0, 0, 10, 10));
        s.add(IRect::new(40, 0, 50, 10));
        // The bridge rect touches both (within slop of each): all three
        // collapse into one — the §3.7 restart-scan fixpoint.
        s.add(IRect::new(15, 0, 35, 10));
        assert_eq!(s.rects(), &[IRect::new(0, 0, 50, 10)]);
    }

    #[test]
    fn cap_collapses_to_bounding_box() {
        let mut s = RegionSet::new();
        // 9 far-apart rects (spacing 100 > slop): the 9th insert trips the
        // cap and the set becomes one bounding box.
        for i in 0..9 {
            s.add(IRect::new(i * 100, 0, i * 100 + 10, 10));
        }
        assert_eq!(s.rects(), &[IRect::new(0, 0, 810, 10)]);
    }

    #[test]
    fn empty_rects_are_ignored() {
        let mut s = RegionSet::new();
        s.add(IRect::EMPTY);
        s.add(IRect::new(5, 5, 5, 9)); // zero width ⇒ empty
        assert!(s.is_empty());
    }

    #[test]
    fn tiles_are_deduplicated_row_major() {
        let mut s = RegionSet::new();
        s.add(IRect::new(-10, 0, 10, 10)); // tiles (-1,0) and (0,0)
        s.add(IRect::new(300, 300, 310, 310)); // tile (1,1)
        let t = s.tiles();
        assert_eq!(t, vec![(-1, 0), (0, 0), (1, 1)]);
    }

    #[test]
    fn intersects_and_bounding_box() {
        let mut s = RegionSet::new();
        s.add(IRect::new(0, 0, 10, 10));
        s.add(IRect::new(100, 100, 110, 110));
        assert!(s.intersects(&IRect::new(5, 5, 6, 6)));
        assert!(!s.intersects(&IRect::new(50, 50, 60, 60)));
        assert_eq!(s.bounding_box(), IRect::new(0, 0, 110, 110));
    }

    #[test]
    fn tracker_take_swaps_and_resets() {
        let mut t = DirtyTracker::new();
        let a = LayerId::nil();
        t.mark(a, IRect::new(0, 0, 4, 4));
        assert!(!t.is_empty());
        let taken = t.take();
        assert!(t.is_empty(), "take leaves an empty tracker (§3.7)");
        assert!(!taken.is_empty());
        // Marks after the take land in the fresh map.
        t.mark(a, IRect::new(9, 9, 12, 12));
        assert_eq!(t.take().union().rects(), &[IRect::new(9, 9, 12, 12)]);
    }

    #[test]
    fn feed_records_and_union_across_layers() {
        let la = LayerId::nil();
        let lb = LayerId::new(); // distinct random id
        let recs = [
            DirtyRecord {
                layer: la,
                rect: IRect::new(0, 0, 10, 10),
            },
            DirtyRecord {
                layer: lb,
                rect: IRect::new(0, 0, 12, 12),
            },
            DirtyRecord {
                layer: lb,
                rect: IRect::new(500, 0, 510, 10),
            },
        ];
        let map = DirtyMap::from_records(&recs);
        assert_eq!(map.layers().count(), 2);
        let u = map.union();
        // Overlapping cross-layer rects coalesce in the union view.
        assert_eq!(u.rects().len(), 2);
        assert!(u.intersects(&IRect::new(11, 11, 12, 12)));
        assert!(u.intersects(&IRect::new(505, 5, 506, 6)));
    }

    #[test]
    fn mark_ignores_empty_and_keeps_layer_granularity() {
        let mut t = DirtyTracker::new();
        let a = LayerId::nil();
        t.mark(a, IRect::EMPTY);
        assert!(t.is_empty());
        t.mark(a, IRect::new(0, 0, 1, 1));
        let map = t.take();
        let per: Vec<_> = map.layers().collect();
        assert_eq!(per.len(), 1);
        assert_eq!(per[0].0, a);
        assert_eq!(per[0].1.rects(), &[IRect::new(0, 0, 1, 1)]);
    }
}
