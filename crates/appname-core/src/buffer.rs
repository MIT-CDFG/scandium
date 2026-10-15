// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §2.5 (`Buffer` — a depth/model-agnostic image surface), §2.3
//! (edge zero rule), §2.4 (bounds maintenance), §6.4 (premultiplied-alpha
//! rules); DD ⑥㉙.
//!
//! A `Buffer` is the unit a layer, mask, or channel stores pixels in: a tile
//! map per level, a content bounding box, depth/model tags, a profile
//! handle, and the premultiplied flag.
//!
//! **Premultiplied convention** [NORMATIVE, ES §2.3/§6.4]: working tiles are
//! always premultiplied; storage declares itself via [`Buffer::premultiplied`]
//! (importers of straight-alpha data set `false`). For premultiplied storage,
//! wherever `a = 0` stored color MUST be `(0,…,0)`, and at the
//! unsigned-normalized depths additionally `color <= a` per channel (at f32,
//! `color > a` with `a > 0` is valid super-bright HDR premult). These rules
//! are asserted at the debug boundaries of every crate-internal write door
//! ([`Buffer::fill_rect`], [`Buffer::write_pixel_f32`]) and checkable
//! surface-wide via [`Buffer::validate_premultiplied`].
//!
//! CP-0.1 scope: lod 0 only (mip levels append lazily with §3.5),
//! `region_hash` recomputes on demand (the per-coord dirty cache arrives
//! with the §10.1 recorder), and depth promotion / model conversion (§6.2,
//! §6.3) belong to the color lane.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::geom::{IRect, IVec2};
use crate::graph::id::ProfileId;
use crate::hash::ContentHash;
use crate::tile::channel::ChannelKind;
use crate::tile::map::{TileCoord, TileMap};
use crate::tile::model::ColorModelKind;
use crate::tile::storage::{canonical_uniform, TileStorage};
use crate::tile::{TILE, TILE_I32};

/// Process-unique identity of one `Buffer` for cache keying (ES §2.1, §3.4).
/// Monotonic counter assigned in `Buffer::new`; never serialized, never
/// reused in-process.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct BufferUid(pub u64);

static NEXT_BUFFER_UID: AtomicU64 = AtomicU64::new(1);

fn fresh_uid() -> BufferUid {
    BufferUid(NEXT_BUFFER_UID.fetch_add(1, Ordering::Relaxed))
}

/// A premultiplied-convention violation found by validation (ES §6.4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PremulViolation {
    /// Document-space pixel where the violation sits (best effort; for value
    /// validation this is the write target or `None`).
    pub at: Option<IVec2>,
    /// Offending channel index.
    pub channel: usize,
    pub value: f32,
    pub alpha: f32,
}

/// A depth/model-agnostic tiled image surface (ES §2.5).
#[derive(Clone)]
pub struct Buffer {
    /// `levels[0]` = full res; higher levels are the mip pyramid (§3.5,
    /// appended lazily — CP-0.1 carries lod 0 only).
    levels: Vec<TileMap>,
    /// Content bounds in document space (may exceed the canvas, ES §19.1).
    bounds: IRect,
    kind: ChannelKind,
    model: ColorModelKind,
    /// ICC profile this data is encoded in (ES §5; resolution lands with the
    /// color lane).
    profile: ProfileId,
    /// Storage alpha state (working tiles are always premult, ES §6.4).
    premultiplied: bool,
    uid: BufferUid,
}

impl Buffer {
    /// ES §2.5 `new` contract: one empty lod-0 map, `bounds = EMPTY`,
    /// `premultiplied = true` (importers of straight-alpha data set `false`
    /// via their own door when that lane lands), fresh `uid`.
    ///
    /// # Panics
    ///
    /// Panics (via [`TileMap::new`]) when `kind` is not storage-backed in
    /// this build (`u16` without the `depth-u16` feature; `f16` is paused).
    /// Slice-2 design constraint (review minor 6, lane log): unreachable
    /// from the command surface — `Document::new` refuses such depths with
    /// `GraphError::DepthNotStorageBacked` first, and every future public
    /// door (commands, ABI) must guard the same way.
    pub fn new(kind: ChannelKind, model: ColorModelKind, profile: ProfileId) -> Buffer {
        Buffer {
            levels: vec![TileMap::new(kind, model)],
            bounds: IRect::EMPTY,
            kind,
            model,
            profile,
            premultiplied: true,
            uid: fresh_uid(),
        }
    }

    pub fn kind(&self) -> ChannelKind {
        self.kind
    }

    pub fn model(&self) -> ColorModelKind {
        self.model
    }

    pub fn profile(&self) -> ProfileId {
        self.profile
    }

    pub fn premultiplied(&self) -> bool {
        self.premultiplied
    }

    pub fn uid(&self) -> BufferUid {
        self.uid
    }

    pub fn bounds(&self) -> IRect {
        self.bounds
    }

    pub fn level_count(&self) -> usize {
        self.levels.len()
    }

    /// The lod-0 tile map (read-only).
    pub fn tiles(&self) -> &TileMap {
        &self.levels[0]
    }

    pub fn get_tile(&self, c: TileCoord) -> Option<&Arc<TileStorage>> {
        debug_assert!((c.lod as usize) < self.levels.len());
        debug_assert_eq!(c.lod, 0, "mip levels arrive with ES §3.5");
        self.levels[c.lod as usize].get(c)
    }

    /// ES §2.5 `tiles_in` contract: **dense over the grid** — yields one item
    /// for every grid cell of `roi` (row-major `(y, x)` order) with `None`
    /// for absent tiles, so callers treat `None` as all-zero without
    /// branching on sparsity. `roi` is used as given, not clipped to
    /// `bounds`. Allocates nothing; never triggers COW.
    pub fn tiles_in<'a>(
        &'a self,
        lod: u8,
        roi: IRect,
    ) -> impl Iterator<Item = (TileCoord, Option<&'a Arc<TileStorage>>)> + 'a {
        assert!(
            (lod as usize) < self.levels.len(),
            "lod out of range (ES §2.5 precondition)"
        );
        let map = &self.levels[lod as usize];
        roi.tiles(TILE_I32).map(move |(tx, ty)| {
            let c = TileCoord::new(lod, tx, ty);
            (c, map.get(c))
        })
    }

    /// Read one pixel (document space, lod 0) as normalized f32 components,
    /// zero-padded to 4. Absent tiles read as all-zero storage scalars
    /// (ES §2.4).
    pub fn read_pixel_f32(&self, p: IVec2) -> [f32; 4] {
        let (c, px, py) = Self::locate(p);
        match self.levels[0].get(c) {
            Some(t) => t.read_pixel_f32(px, py, self.model),
            None => [0.0; 4],
        }
    }

    /// Pixel-content identity over `bounds` (ES §2.5), independent of
    /// tile-map iteration order, residency tier, codec, and `Uniform`
    /// representation:
    ///
    /// ```text
    /// H = BLAKE3( kind:u8 ‖ model:u8 ‖ premultiplied:u8 ‖
    ///             bounds.min.x:i32LE ‖ bounds.min.y:i32LE ‖
    ///             bounds.max.x:i32LE ‖ bounds.max.y:i32LE ‖
    ///             for each OCCUPIED lod-0 tile intersecting bounds,
    ///                 sorted by (y, x) ascending:
    ///               coord.y:i32LE ‖ coord.x:i32LE ‖ tile content hash : 32 B )
    /// ```
    ///
    /// Recomputed on demand at CP-0.1; the per-changed-coord cache
    /// invalidation arrives with the §10.1 recorder.
    pub fn region_hash(&self) -> ContentHash {
        let map = &self.levels[0];
        let mut occupied: Vec<TileCoord> = map
            .iter()
            .map(|(c, _)| c)
            .filter(|c| !c.pixel_rect().intersect(&self.bounds).is_empty())
            .collect();
        occupied.sort_by_key(|c| (c.y, c.x));
        ContentHash::from_stream(|h| {
            h.update(&[
                self.kind.as_u8(),
                self.model.as_u8(),
                self.premultiplied as u8,
            ]);
            for v in [
                self.bounds.min.x,
                self.bounds.min.y,
                self.bounds.max.x,
                self.bounds.max.y,
            ] {
                h.update(&v.to_le_bytes());
            }
            for c in occupied {
                h.update(&c.y.to_le_bytes());
                h.update(&c.x.to_le_bytes());
                h.update(map.content_hash_at(c).as_bytes());
            }
        })
    }

    /// Surface-wide premultiplied-convention check (ES §6.4). `Ok` for
    /// straight-alpha storage and alpha-free models by definition.
    ///
    /// **CmykA is a documented no-op skip** (review minor 3): its alpha
    /// lives at channel index 4, outside the 4-slot f32 access window, so
    /// this validator cannot observe the real alpha — comparing against
    /// slot 3 would silently test ink K as if it were alpha. Validation for
    /// 5-channel models arrives with the §14 channels work (full-width
    /// values); until then skipping is honest, wrong math is not.
    pub fn validate_premultiplied(&self) -> Result<(), PremulViolation> {
        if !self.premultiplied || !self.model.has_alpha() {
            return Ok(());
        }
        let alpha_ix = self.model.alpha_index().expect("alpha-bearing model");
        if alpha_ix >= 4 {
            return Ok(()); // CmykA: deferred to §14 (see doc comment)
        }
        let unsigned = self.kind.is_unsigned_normalized();
        for (c, tile) in self.levels[0].iter() {
            let rect = c.pixel_rect();
            for py in 0..TILE {
                for px in 0..TILE {
                    let v = tile.read_pixel_f32(px, py, self.model);
                    let a = v[alpha_ix];
                    for (ch, &vc) in v.iter().enumerate().take(self.model.channels().min(4)) {
                        if ch == alpha_ix {
                            continue;
                        }
                        let bad = if unsigned {
                            vc > a
                        } else {
                            a == 0.0 && vc != 0.0
                        };
                        if bad {
                            return Err(PremulViolation {
                                at: Some(IVec2::new(
                                    rect.min.x + px as i32,
                                    rect.min.y + py as i32,
                                )),
                                channel: ch,
                                value: v[ch],
                                alpha: a,
                            });
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Validate one storage-normalized value against the premultiplied
    /// convention for this buffer (the per-write debug-boundary check).
    /// CmykA skips — same reasoning as [`Buffer::validate_premultiplied`].
    #[cfg_attr(not(test), allow(dead_code))] // slice-2 command-engine door (in-crate tests exercise it today)
    fn validate_premul_value(&self, v: [f32; 4]) -> Result<(), PremulViolation> {
        if !self.premultiplied || !self.model.has_alpha() {
            return Ok(());
        }
        let alpha_ix = self.model.alpha_index().expect("alpha-bearing model");
        if alpha_ix >= 4 {
            return Ok(()); // CmykA: deferred to §14 (see validate_premultiplied)
        }
        let a = v[alpha_ix];
        let unsigned = self.kind.is_unsigned_normalized();
        for (ch, &vc) in v.iter().enumerate().take(self.model.channels().min(4)) {
            if ch == alpha_ix {
                continue;
            }
            let bad = if unsigned {
                vc > a
            } else {
                a == 0.0 && vc != 0.0
            };
            if bad {
                return Err(PremulViolation {
                    at: None,
                    channel: ch,
                    value: v[ch],
                    alpha: a,
                });
            }
        }
        Ok(())
    }

    // -----------------------------------------------------------------
    // Crate-internal mutation doors (plan rule 5 / D-48): the command
    // engine (slice 2) is the only public path to these.
    // -----------------------------------------------------------------

    /// The COW door at buffer level (delegates to `TileMap::make_mut`,
    /// ES §2.4). Callers must maintain `bounds` themselves: the write doors
    /// [`Buffer::write_pixel_f32`]/[`Buffer::fill_rect`] union eagerly, and
    /// erasing edits re-tighten via [`Buffer::recompute_bounds`] — a caller
    /// writing through this raw door owes the same discipline (the
    /// `recompute_bounds` debug guard catches the omission).
    #[cfg_attr(not(test), allow(dead_code))] // slice-2 command-engine door (in-crate tests exercise it today)
    pub(crate) fn make_mut_tile(&mut self, c: TileCoord) -> &mut TileStorage {
        debug_assert_eq!(c.lod, 0, "mip levels arrive with ES §3.5");
        self.levels[0].make_mut(c)
    }

    /// Write one pixel (document space, lod 0), growing bounds eagerly
    /// (ES §2.4). Debug-asserts the §6.4 premultiplied rules.
    #[cfg_attr(not(test), allow(dead_code))] // slice-2 command-engine door (in-crate tests exercise it today)
    pub(crate) fn write_pixel_f32(&mut self, p: IVec2, v: [f32; 4]) {
        debug_assert!(
            self.validate_premul_value(v).is_ok(),
            "premultiplied violation (ES §6.4): {:?} into {:?}/{:?}",
            v,
            self.kind,
            self.model
        );
        let (c, px, py) = Self::locate(p);
        self.levels[0].make_mut(c).write_pixel_f32(px, py, v);
        self.bounds = self.bounds.union(&IRect::new(p.x, p.y, p.x + 1, p.y + 1));
    }

    /// Fill `rect` with one storage-normalized value (ES §2.3 uniform
    /// formation rule 1: fully-covered tiles become `Uniform` without
    /// materializing; an all-zero fill removes them — absence reads the
    /// same). Bounds grow eagerly for content fills and recompute
    /// tile-granularly for erasing fills (ES §2.4).
    #[cfg_attr(not(test), allow(dead_code))] // slice-2 command-engine door (in-crate tests exercise it today)
    pub(crate) fn fill_rect(&mut self, rect: IRect, raw: [f32; 4]) {
        if rect.is_empty() {
            return;
        }
        let value = canonical_uniform(raw, self.kind, self.model);
        debug_assert!(
            self.validate_premul_value(value).is_ok(),
            "premultiplied violation (ES §6.4): fill {value:?}"
        );
        let zero_fill = TileStorage::Uniform { value }.is_bitwise_zero(self.kind, self.model);
        let uniform_ok = self.model.channels() <= 4;
        for (tx, ty) in rect.tiles(TILE_I32) {
            let c = TileCoord::new(0, tx, ty);
            let tile_rect = c.pixel_rect();
            if rect.contains_rect(&tile_rect) {
                // Fully covered: representation fast paths, no pixel loop.
                if zero_fill {
                    self.levels[0].remove(c);
                } else if uniform_ok {
                    self.levels[0].insert_arc(c, Arc::new(TileStorage::Uniform { value }));
                } else {
                    // CmykA: always materializes (ES §2.3 model limit).
                    self.levels[0].insert_arc(
                        c,
                        Arc::new(TileStorage::materialize(value, self.kind, self.model)),
                    );
                }
            } else {
                let span = rect.intersect(&tile_rect);
                if span.is_empty() {
                    continue;
                }
                // Partial cover of an absent tile with zeros is a no-op
                // (absent already reads all-zero).
                if zero_fill && self.levels[0].get(c).is_none() {
                    continue;
                }
                let tile = self.levels[0].make_mut(c);
                for y in span.min.y..span.max.y {
                    for x in span.min.x..span.max.x {
                        tile.write_pixel_f32(
                            (x - tile_rect.min.x) as usize,
                            (y - tile_rect.min.y) as usize,
                            value,
                        );
                    }
                }
            }
        }
        if zero_fill {
            // Deletion-type edit: shrink lazily, tile-granular (ES §2.4).
            self.recompute_bounds();
        } else {
            self.bounds = self.bounds.union(&rect);
        }
    }

    /// ES §2.4 bounds recompute: prune, then exactly the NORMATIVE
    /// `occupied_bounds() ∩ old bounds` — O(occupied tiles), no pixel reads
    /// beyond the prune scans (review minor 4: the formula is applied
    /// as written, with no special case for empty old bounds — `bounds`
    /// grows eagerly on every write door, so "occupied tiles under EMPTY
    /// bounds" is unreachable except through in-crate misuse of the raw
    /// `make_mut_tile` door, which the debug assert below names).
    #[cfg_attr(not(test), allow(dead_code))] // slice-2 command-engine door (in-crate tests exercise it today)
    pub(crate) fn recompute_bounds(&mut self) {
        self.levels[0].prune_transparent();
        let occupied = self.levels[0].occupied_bounds();
        debug_assert!(
            occupied.is_empty() || !self.bounds.is_empty(),
            "recompute_bounds: occupied tiles under EMPTY bounds — a writer skipped eager \
             bounds growth (raw make_mut_tile misuse; ES §2.4 write doors union eagerly)"
        );
        let clipped = occupied.intersect(&self.bounds);
        self.bounds = if clipped.is_empty() {
            IRect::EMPTY
        } else {
            clipped
        };
    }

    /// Commit-time sparsity maintenance door (prune + collapse; ES §2.4).
    #[cfg_attr(not(test), allow(dead_code))] // slice-2 command-engine door (in-crate tests exercise it today)
    pub(crate) fn prune(&mut self) {
        self.levels[0].prune_transparent();
    }

    // ----- §10.1 delta-restore doors (engine commit/undo path only) -----

    /// Prune/collapse one touched coord at command commit (§2.4: restricted
    /// to the recorder's coords — never a full-map sweep during editing).
    pub(crate) fn prune_tile(&mut self, c: TileCoord) {
        self.levels[0].prune_at(c);
    }

    /// Install a recorded payload Arc (delta restore / dedup swap, §3.3 —
    /// an O(1) Arc swap, never a pixel copy).
    pub(crate) fn install_tile(&mut self, c: TileCoord, tile: Arc<TileStorage>) {
        self.levels[0].insert_arc(c, tile);
    }

    /// Remove a tile (restoring a created tile back to absent, §2.4).
    #[cfg_attr(not(test), allow(dead_code))] // engine door for later slices (in-crate tests exercise it today)
    pub(crate) fn remove_tile(&mut self, c: TileCoord) {
        self.levels[0].remove(c);
    }

    /// Set bounds to a recorder-captured value (§10.1 restore path). The
    /// value was read from this buffer under the same invariants, so this
    /// trusted door bypasses the recompute misuse guard by design.
    #[cfg_attr(not(test), allow(dead_code))] // engine door for later slices (in-crate tests exercise it today)
    pub(crate) fn set_bounds_recorded(&mut self, bounds: IRect) {
        self.bounds = if bounds.is_empty() {
            IRect::EMPTY
        } else {
            bounds
        };
    }

    /// Package-loader door: storage alpha state read from chunk flags
    /// (§7.4 bit0; importers of straight-alpha data per §2.5 `new` note).
    pub(crate) fn set_premultiplied_loaded(&mut self, premultiplied: bool) {
        self.premultiplied = premultiplied;
    }

    /// Eager bounds growth by a caller-known content rect (§2.4 "every
    /// write unions the affected rect") — the KindContext door for
    /// implementations that install whole tile planes and know the true
    /// content extent (e.g. a stroke commit's bounds).
    pub(crate) fn grow_bounds_recorded(&mut self, rect: IRect) {
        self.bounds = self.bounds.union(&rect);
    }

    /// Pixel-exact content bounds (§2.4: "computed on demand by scanning
    /// … tiles' pixels"): prune, then union the tight rect of pixels with
    /// any nonzero storage scalar. Used by the package loader and importer
    /// so persisted/derived bounds agree with the in-memory eager-grown
    /// bounds for every content-only surface.
    pub(crate) fn recompute_bounds_pixel_exact(&mut self) {
        self.levels[0].prune_transparent();
        let mut bounds = IRect::EMPTY;
        let coords: Vec<TileCoord> = self.levels[0].iter().map(|(c, _)| c).collect();
        for c in coords {
            let tile = self.levels[0].get(c).expect("occupied").clone();
            let origin = c.pixel_rect().min;
            let mut tile_bounds = IRect::EMPTY;
            for y in 0..TILE {
                for x in 0..TILE {
                    let px = tile.read_pixel_f32(x, y, self.model);
                    if px.iter().any(|v| v.to_bits() != 0) {
                        let gx = origin.x + x as i32;
                        let gy = origin.y + y as i32;
                        tile_bounds = tile_bounds.union(&IRect::new(gx, gy, gx + 1, gy + 1));
                    }
                }
            }
            bounds = bounds.union(&tile_bounds);
        }
        self.bounds = bounds;
    }

    /// Pixel-exact eager growth SCOPED to a just-written region (Q-1
    /// determinism class). Unions into `bounds` the tight rect of pixels
    /// carrying any nonzero storage scalar in the tiles that intersect
    /// `written` — the SAME per-pixel predicate the package loader's
    /// [`Self::recompute_bounds_pixel_exact`] applies, but restricted to the
    /// written footprint so an alpha-PRESERVING stroke pays only the cost of
    /// its own dabs, never a full-layer rescan (guarding the F1 fill_apply
    /// +17% class of recompute regression). Because the write only ADDS
    /// coverage and the prior extent is already loader-tight, the result
    /// equals the loader's full recompute; a coverage-REDUCING write must use
    /// `recompute_bounds_pixel_exact` instead, which can shrink sub-tile.
    pub(crate) fn grow_bounds_pixel_exact_within(&mut self, written: IRect) {
        if written.is_empty() {
            return;
        }
        // Slack can only arise at the GROWTH FRONTIER. If the write lands
        // wholly inside the current (loader-tight) extent, the extent does
        // not move and no fringe is exposed, so the pixel scan is pure cost —
        // skip it. This keeps painting on an already-established layer O(1)
        // (the paint hot path: `stroke_apply` on a full-canvas layer), and
        // pays the scan only on the rare dab that pushes the extent outward.
        if self.bounds.union(&written) == self.bounds {
            return;
        }
        let mut grown = IRect::EMPTY;
        for (c, tile) in self.levels[0].iter() {
            if c.pixel_rect().intersect(&written).is_empty() {
                continue;
            }
            let origin = c.pixel_rect().min;
            for y in 0..TILE {
                for x in 0..TILE {
                    let px = tile.read_pixel_f32(x, y, self.model);
                    if px.iter().any(|v| v.to_bits() != 0) {
                        let gx = origin.x + x as i32;
                        let gy = origin.y + y as i32;
                        grown = grown.union(&IRect::new(gx, gy, gx + 1, gy + 1));
                    }
                }
            }
        }
        self.bounds = self.bounds.union(&grown);
    }

    fn locate(p: IVec2) -> (TileCoord, usize, usize) {
        let c = TileCoord::new(0, p.x.div_euclid(TILE_I32), p.y.div_euclid(TILE_I32));
        (
            c,
            p.x.rem_euclid(TILE_I32) as usize,
            p.y.rem_euclid(TILE_I32) as usize,
        )
    }
}

/// Summary Debug (ES §1.6: diagnostics never dump pixel payloads).
impl core::fmt::Debug for Buffer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Buffer")
            .field("kind", &self.kind)
            .field("model", &self.model)
            .field("bounds", &self.bounds)
            .field("tiles", &self.levels[0].len())
            .field("premultiplied", &self.premultiplied)
            .field("uid", &self.uid)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tile::channel::Channel;
    use uuid::Uuid;

    fn rgba8() -> Buffer {
        Buffer::new(
            ChannelKind::U8,
            ColorModelKind::Rgba,
            ProfileId(Uuid::nil()),
        )
    }

    fn gray_f32() -> Buffer {
        Buffer::new(
            ChannelKind::F32,
            ColorModelKind::Gray,
            ProfileId(Uuid::nil()),
        )
    }

    #[test]
    fn new_contract() {
        let b = rgba8();
        assert_eq!(b.level_count(), 1);
        assert_eq!(b.bounds(), IRect::EMPTY);
        assert!(b.premultiplied());
        assert!(b.tiles().is_empty());
    }

    #[test]
    fn buffer_uids_are_unique_and_monotonic() {
        let a = rgba8().uid();
        let b = rgba8().uid();
        assert_ne!(a, b);
        assert!(b.0 > a.0);
    }

    #[test]
    fn pixel_addressing_spans_negative_document_space() {
        let mut b = rgba8();
        b.write_pixel_f32(IVec2::new(-1, -1), [0.0, 0.0, 0.0, 1.0]);
        b.write_pixel_f32(IVec2::new(0, 0), [1.0, 1.0, 1.0, 1.0]);
        // (-1,-1) lives in tile (-1,-1) at in-tile offset (255,255).
        assert_eq!(b.read_pixel_f32(IVec2::new(-1, -1)), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(b.read_pixel_f32(IVec2::new(0, 0)), [1.0, 1.0, 1.0, 1.0]);
        assert!(b.get_tile(TileCoord::new(0, -1, -1)).is_some());
        assert_eq!(b.bounds(), IRect::new(-1, -1, 1, 1));
        // Absent space reads zero.
        assert_eq!(b.read_pixel_f32(IVec2::new(999, 999)), [0.0; 4]);
    }

    #[test]
    fn fill_rect_full_tiles_form_uniform_partial_tiles_write_pixels() {
        let mut b = rgba8();
        // Cover tile (0,0) fully and tile (1,0) partially.
        b.fill_rect(IRect::new(0, 0, 300, 256), [0.5, 0.25, 0.125, 1.0]);
        assert!(b.get_tile(TileCoord::new(0, 0, 0)).unwrap().is_uniform());
        let partial = b.get_tile(TileCoord::new(0, 1, 0)).unwrap();
        assert!(!partial.is_uniform());
        // Inside vs outside the filled span of the partial tile.
        let v = b.read_pixel_f32(IVec2::new(299, 10));
        assert!(v[3] == 1.0 && v[0] > 0.0);
        assert_eq!(b.read_pixel_f32(IVec2::new(300, 10)), [0.0; 4]);
        assert_eq!(b.bounds(), IRect::new(0, 0, 300, 256));
    }

    #[test]
    fn zero_fill_prunes_and_shrinks_bounds_tile_granularly() {
        let mut b = rgba8();
        b.fill_rect(IRect::new(0, 0, 512, 256), [0.5, 0.5, 0.5, 1.0]);
        assert_eq!(b.tiles().len(), 2);
        // Erase the right tile entirely.
        b.fill_rect(IRect::new(256, 0, 512, 256), [0.0; 4]);
        assert_eq!(
            b.tiles().len(),
            1,
            "zero-filled full tile is removed, not stored"
        );
        assert_eq!(b.bounds(), IRect::new(0, 0, 256, 256));
        // Erase everything.
        b.fill_rect(IRect::new(0, 0, 256, 256), [0.0; 4]);
        assert!(b.tiles().is_empty());
        assert_eq!(b.bounds(), IRect::EMPTY);
    }

    #[test]
    fn zero_fill_partial_over_absent_is_noop() {
        let mut b = rgba8();
        b.fill_rect(IRect::new(10, 10, 20, 20), [0.0; 4]);
        assert!(b.tiles().is_empty());
        assert_eq!(b.bounds(), IRect::EMPTY);
    }

    #[test]
    fn tiles_in_is_dense_over_the_grid() {
        let mut b = rgba8();
        b.fill_rect(IRect::new(0, 0, 256, 256), [0.1, 0.1, 0.1, 1.0]);
        let items: Vec<_> = b.tiles_in(0, IRect::new(-10, 0, 300, 256)).collect();
        // Grid columns -1, 0, 1 × row 0.
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].0, TileCoord::new(0, -1, 0));
        assert!(items[0].1.is_none(), "absent cell yields None");
        assert!(items[1].1.is_some());
        assert!(items[2].1.is_none());
    }

    #[test]
    fn region_hash_is_order_independent_and_content_sensitive() {
        let mut a = rgba8();
        a.fill_rect(IRect::new(0, 0, 256, 256), [0.2, 0.2, 0.2, 1.0]);
        a.fill_rect(IRect::new(256, 0, 512, 256), [0.4, 0.4, 0.4, 1.0]);

        let mut b = rgba8();
        // Same content, opposite insertion order.
        b.fill_rect(IRect::new(256, 0, 512, 256), [0.4, 0.4, 0.4, 1.0]);
        b.fill_rect(IRect::new(0, 0, 256, 256), [0.2, 0.2, 0.2, 1.0]);
        assert_eq!(a.region_hash(), b.region_hash());

        // A single pixel change changes identity.
        b.write_pixel_f32(IVec2::new(5, 5), [0.9, 0.9, 0.9, 1.0]);
        assert_ne!(a.region_hash(), b.region_hash());
    }

    #[test]
    fn region_hash_sees_uniform_and_dense_identically() {
        let mut a = rgba8();
        a.fill_rect(IRect::new(0, 0, 256, 256), [0.5, 0.25, 0.125, 1.0]);
        assert!(a.get_tile(TileCoord::new(0, 0, 0)).unwrap().is_uniform());

        let mut b = rgba8();
        b.fill_rect(IRect::new(0, 0, 256, 256), [0.5, 0.25, 0.125, 1.0]);
        // Force materialization on b's tile without changing content.
        b.make_mut_tile(TileCoord::new(0, 0, 0));
        assert!(!b.get_tile(TileCoord::new(0, 0, 0)).unwrap().is_uniform());
        assert_eq!(a.region_hash(), b.region_hash());
    }

    #[test]
    fn region_hash_frames_encoding_tags() {
        // Same normalized content at different depths → different identity
        // (kind byte + payload bytes both differ).
        let mut a = rgba8();
        a.fill_rect(IRect::new(0, 0, 10, 10), [1.0, 0.0, 0.0, 1.0]);
        let mut b = Buffer::new(
            ChannelKind::F32,
            ColorModelKind::Rgba,
            ProfileId(Uuid::nil()),
        );
        b.fill_rect(IRect::new(0, 0, 10, 10), [1.0, 0.0, 0.0, 1.0]);
        assert_ne!(a.region_hash(), b.region_hash());
        // Empty buffers of different models also differ (prefix bytes).
        let e1 = rgba8().region_hash();
        let e2 = gray_f32().region_hash();
        assert_ne!(e1, e2);
    }

    #[test]
    fn premultiplied_validation_catches_unsigned_color_over_alpha() {
        let mut b = rgba8();
        // Valid premult writes pass.
        b.fill_rect(IRect::new(0, 0, 16, 16), [0.5, 0.5, 0.5, 0.5]);
        assert!(b.validate_premultiplied().is_ok());
        // Hand-corrupt a tile below the guarded doors (test-only path).
        b.make_mut_tile(TileCoord::new(0, 0, 0))
            .write_pixel_f32(3, 3, [0.8, 0.0, 0.0, 0.5]);
        let err = b.validate_premultiplied().unwrap_err();
        assert_eq!(err.channel, 0);
        assert_eq!(err.at, Some(IVec2::new(3, 3)));
    }

    #[test]
    fn premultiplied_validation_f32_allows_hdr_but_not_hidden_color() {
        let mut b = Buffer::new(
            ChannelKind::F32,
            ColorModelKind::Rgba,
            ProfileId(Uuid::nil()),
        );
        // Super-bright HDR premult (color > alpha, alpha > 0) is valid at f32
        // (ES §6.4 rule 1).
        b.write_pixel_f32(IVec2::new(0, 0), [3.5, 0.0, 0.0, 0.5]);
        assert!(b.validate_premultiplied().is_ok());
        // a = 0 with color is a violation.
        b.make_mut_tile(TileCoord::new(0, 0, 0))
            .write_pixel_f32(1, 0, [0.25, 0.0, 0.0, 0.0]);
        assert!(b.validate_premultiplied().is_err());
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "premultiplied violation")]
    fn write_door_debug_asserts_premultiplied() {
        let mut b = rgba8();
        b.write_pixel_f32(IVec2::new(0, 0), [0.9, 0.0, 0.0, 0.1]);
    }

    #[test]
    fn alpha_free_and_straight_storage_skip_premul_rules() {
        let mut b = gray_f32();
        b.fill_rect(IRect::new(0, 0, 8, 8), [0.7, 0.0, 0.0, 0.0]);
        assert!(b.validate_premultiplied().is_ok());
    }

    #[test]
    fn cmyka_premul_validation_is_deferred_not_wrong() {
        // Review minor 3: CmykA's alpha is channel 4, outside the 4-slot
        // window; the old `.min(3)` path compared C/M/Y against ink K as if
        // it were alpha. C = 0.8 > slot-3 K = 0.2 would have tripped both
        // the surface validator and the write door's debug_assert
        // spuriously. Now the validators skip CmykA explicitly.
        let mut b = Buffer::new(
            ChannelKind::U8,
            ColorModelKind::CmykA,
            ProfileId(Uuid::nil()),
        );
        assert!(
            b.premultiplied(),
            "default storage flag — makes the skip path the live path"
        );
        b.fill_rect(IRect::new(0, 0, 300, 40), [0.8, 0.1, 0.1, 0.2]); // must not panic
        b.write_pixel_f32(IVec2::new(3, 3), [0.9, 0.0, 0.0, 0.1]); // must not panic
        assert!(
            b.validate_premultiplied().is_ok(),
            "documented no-op skip until §14"
        );
        let v = b.read_pixel_f32(IVec2::new(3, 3));
        assert_eq!(v[0], u8::from_f32(0.9).to_f32());
    }

    #[test]
    fn recompute_bounds_applies_the_normative_intersection() {
        // ES §2.4 [NORMATIVE]: new bounds = occupied_bounds() ∩ old bounds —
        // the old bounds clip even when occupied is larger.
        let mut b = rgba8();
        b.fill_rect(IRect::new(0, 0, 512, 256), [0.5, 0.5, 0.5, 1.0]);
        // Simulate a deletion-type command that narrowed the meaningful
        // region before re-tightening (fields are module-visible to tests).
        b.bounds = IRect::new(0, 0, 100, 100);
        b.recompute_bounds();
        assert_eq!(
            b.bounds(),
            IRect::new(0, 0, 100, 100),
            "occupied ∩ old, not occupied"
        );
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "occupied tiles under EMPTY bounds")]
    fn recompute_bounds_flags_raw_door_misuse_in_debug() {
        // Writing through the raw COW door without growing bounds is the
        // only way to reach "occupied under EMPTY bounds"; the debug guard
        // names it (review minor 4 decision: NORMATIVE formula + guard).
        let mut b = rgba8();
        b.make_mut_tile(TileCoord::new(0, 0, 0))
            .write_pixel_f32(1, 1, [0.5, 0.5, 0.5, 1.0]);
        b.recompute_bounds();
    }

    /// Property-style COW safety at buffer level: a snapshot's identity never
    /// moves while the live buffer keeps editing (fixed-seed pseudo-random
    /// edit script; plan test row).
    #[test]
    fn snapshot_identity_is_stable_under_live_edits() {
        // SplitMix64 — tiny deterministic generator for test scripts.
        struct Rng(u64);
        impl Rng {
            fn next(&mut self) -> u64 {
                self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
                let mut z = self.0;
                z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
                z ^ (z >> 31)
            }
            fn pick(&mut self, n: u64) -> i32 {
                (self.next() % n) as i32
            }
        }

        let mut rng = Rng(0xC0FFEE);
        let mut live = rgba8();
        live.fill_rect(IRect::new(0, 0, 700, 700), [0.25, 0.25, 0.25, 1.0]);

        let snapshot = Buffer {
            levels: live.levels.clone(),
            bounds: live.bounds,
            kind: live.kind,
            model: live.model,
            profile: live.profile,
            premultiplied: live.premultiplied,
            uid: live.uid,
        };
        let frozen = snapshot.region_hash();
        assert_eq!(live.region_hash(), frozen);

        for _ in 0..40 {
            let x = rng.pick(700);
            let y = rng.pick(700);
            let w = 1 + rng.pick(300);
            let h = 1 + rng.pick(300);
            let a = (rng.pick(256) as f32) / 255.0;
            let c = ((rng.pick(256) as f32) / 255.0) * a; // valid premult
            live.fill_rect(IRect::new(x, y, x + w, y + h), [c, c, c, a]);
        }
        live.prune();

        assert_eq!(
            snapshot.region_hash(),
            frozen,
            "shared tiles must never mutate under COW"
        );
        assert!(snapshot.validate_premultiplied().is_ok());
        assert!(live.validate_premultiplied().is_ok());
    }

    /// Determinism property: the same edit script replays to the same
    /// region_hash (fixed seed).
    #[test]
    fn identical_edit_scripts_reach_identical_identity() {
        fn run(seed: u64) -> ContentHash {
            let mut s = seed;
            let mut next = move || {
                s = s.wrapping_add(0x9E3779B97F4A7C15);
                let mut z = s;
                z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
                z ^ (z >> 31)
            };
            let mut b = Buffer::new(
                ChannelKind::U8,
                ColorModelKind::Rgba,
                ProfileId(Uuid::nil()),
            );
            for _ in 0..25 {
                let x = (next() % 600) as i32 - 100;
                let y = (next() % 600) as i32 - 100;
                let w = 1 + (next() % 400) as i32;
                let h = 1 + (next() % 400) as i32;
                let a = ((next() % 256) as f32) / 255.0;
                let c = (((next() % 256) as f32) / 255.0) * a;
                b.fill_rect(IRect::new(x, y, x + w, y + h), [c, 0.0, c, a]);
            }
            b.prune();
            b.region_hash()
        }
        assert_eq!(run(42), run(42));
        assert_ne!(run(42), run(43));
    }
}
