// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §2.2–§2.4 (pixel traits, `Tile<T,C>`, `TileMap`), §3.1 (tile
//! geometry), §3.3 (copy-on-write), §3.8 (content addressing); DD areas ⑥⑲.
//!
//! The tile substrate: fixed 256×256, channel-interleaved, row-major,
//! premultiplied-alpha pixel blocks; a sparse copy-on-write [`map::TileMap`]
//! of `Arc<TileStorage>` payloads; BLAKE3-256 content identity per tile.
//!
//! Mutation-door rule (plan preamble rule 5 / D-48): every mutating entry
//! point in this module is `pub(crate)` — [`map::TileMap::make_mut`] is the
//! single COW door — so the command engine (slice 2) becomes the only public
//! way document state changes. Read access is public.
//!
//! CP-0.1 scope fences: no tile pool (§3.2 — payloads are plain boxed slices
//! until the pool lands), no cache tiers / spill (§3.4, CP-0.5), no mips
//! (§3.5), no SIMD kernels (§4.8 — the layout is SIMD-friendly per §3.1: one
//! contiguous allocation, rows contiguous, no padding).

pub mod channel;
pub mod compress;
pub mod dedup;
pub mod map;
pub mod model;
pub mod storage;
#[allow(clippy::module_inception)]
pub mod tile;

/// [NORMATIVE, ES §2.3] Tile edge in pixels. A single compile-time constant —
/// never per-document.
pub const TILE: usize = 256;

/// `TILE` as the i32 the geometry layer works in (grid math, `IRect::tiles`).
pub const TILE_I32: i32 = TILE as i32;
