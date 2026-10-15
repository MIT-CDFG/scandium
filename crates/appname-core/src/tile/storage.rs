// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §2.3 (`TileStorage`, uniform tiles, edge-clipping zero rule),
//! §3.8 (content addressing).
//!
//! `TileStorage` unifies the storage depths behind one runtime type so a
//! buffer is not generic over `T` at the document level (ES §2.4). Pixel
//! variants hold one dense payload plus the `ColorModelKind` tag; the
//! `Uniform` variant is the in-memory solid-tile optimization (§2.3): 16 B +
//! tag, no pixel allocation, **no on-disk representation**.
//!
//! [NORMATIVE, ES §3.8] A tile's content identity is the BLAKE3-256 hash of
//! its **uncompressed payload bytes** (little-endian scalar encoding, §0.3),
//! including for `Uniform` tiles, which hash the materialized bytes streamed
//! as a repeated pattern without allocating — a `Uniform` tile and its
//! materialized twin have the same identity. Hash equality **is** identity.
//!
//! Payloads are plain boxed slices at CP-0.1; the §3.2 pool allocator
//! (`PooledBox`) replaces the backing when the memory-management work lands
//! (CP-0.5) without changing this module's API.

use super::channel::{Channel, ChannelKind};
use super::model::ColorModelKind;
use super::tile::Tile;
use super::TILE;
use crate::hash::ContentHash;
use crate::tile::model::ColorModel;

/// One dense payload: `TILE*TILE*channels` scalars, row-major,
/// channel-interleaved (ES §2.3), plus the model tag.
#[derive(Clone)]
pub struct TilePixels<T: Channel> {
    data: Box<[T]>,
    model: ColorModelKind,
}

impl<T: Channel> TilePixels<T> {
    fn new(data: Box<[T]>, model: ColorModelKind) -> TilePixels<T> {
        assert_eq!(
            data.len(),
            TILE * TILE * model.channels(),
            "payload length must match model"
        );
        TilePixels { data, model }
    }

    pub fn model(&self) -> ColorModelKind {
        self.model
    }

    pub fn as_slice(&self) -> &[T] {
        &self.data
    }
}

/// Depth-erased tile payload (ES §2.3). The `U16` variant exists behind the
/// `depth-u16` feature; `F16` is paused pending the `half` crate record (see
/// [`super::channel`]).
#[derive(Clone)]
pub enum TileStorage {
    U8(TilePixels<u8>),
    #[cfg(feature = "depth-u16")]
    U16(TilePixels<u16>),
    F32(TilePixels<f32>),
    /// Solid tile: every pixel identical; no pixel allocation (ES §2.3).
    /// `value[c]` is the storage-normalized channel value, canonicalized
    /// through the owning map's depth; entries at index >= CHANNELS MUST
    /// be 0. Only models with CHANNELS <= 4 form a non-zero `Uniform`
    /// (`CmykA` always materializes); the all-zero `Uniform` is the universal
    /// transparent representation for every model.
    Uniform {
        value: [f32; 4],
    },
}

/// Canonicalize a raw storage-normalized value through a depth (ES §2.3
/// uniform rule): `value[c] = T::to_f32(T::from_f32(raw[c]))` for the map's
/// `ChannelKind`, entries at index >= CHANNELS forced to 0. The stored f32
/// then equals exactly what materialized pixels re-encode to, making
/// materialization, hashing, and equality consistent.
pub fn canonical_uniform(raw: [f32; 4], kind: ChannelKind, model: ColorModelKind) -> [f32; 4] {
    fn canon<T: Channel>(raw: [f32; 4], channels: usize) -> [f32; 4] {
        let mut out = [0.0f32; 4];
        for (c, slot) in out.iter_mut().enumerate().take(channels.min(4)) {
            *slot = T::from_f32(raw[c]).to_f32();
        }
        out
    }
    let ch = model.channels();
    match kind {
        ChannelKind::U8 => canon::<u8>(raw, ch),
        #[cfg(feature = "depth-u16")]
        ChannelKind::U16 => canon::<u16>(raw, ch),
        ChannelKind::F32 => canon::<f32>(raw, ch),
        _ => unreachable!("depth {kind:?} is not storage-backed in this build"),
    }
}

// ---------------------------------------------------------------------------
// Generic payload helpers (monomorphized per depth, dispatched once at the
// storage boundary — the §2.4 pattern).
// ---------------------------------------------------------------------------

/// Streaming LE hash of a dense payload (chunked; no allocation beyond the
/// stack scratch buffer).
fn hash_dense<T: Channel>(data: &[T]) -> ContentHash {
    ContentHash::from_stream(|h| {
        let mut buf = [0u8; 4096];
        let per = T::BYTES;
        let cap = buf.len() / per;
        for chunk in data.chunks(cap) {
            let mut n = 0;
            for &s in chunk {
                s.put_le(&mut buf[n..n + per]);
                n += per;
            }
            h.update(&buf[..n]);
        }
    })
}

/// Streaming LE hash of the materialized form of a uniform value — identical
/// to hashing the dense twin, with no pixel allocation (ES §2.3/§3.8).
fn hash_uniform<T: Channel>(value: &[f32; 4], channels: usize) -> ContentHash {
    // Encode one pixel's pattern (<= 5 channels * 4 B).
    let mut pattern = [0u8; 20];
    let per = T::BYTES;
    for c in 0..channels {
        let raw = value.get(c).copied().unwrap_or(0.0);
        T::from_f32(raw).put_le(&mut pattern[c * per..c * per + per]);
    }
    let pat_len = channels * per;
    ContentHash::from_stream(|h| {
        // Tile the pattern into a stack buffer holding a whole number of
        // pixels, then stream TILE*TILE pixels worth of bytes.
        let mut buf = [0u8; 4080]; // 4080 = lcm-friendly for 1..=20-byte patterns
        let px_per_buf = buf.len() / pat_len;
        for i in 0..px_per_buf {
            buf[i * pat_len..(i + 1) * pat_len].copy_from_slice(&pattern[..pat_len]);
        }
        let mut remaining = TILE * TILE;
        while remaining > 0 {
            let px = remaining.min(px_per_buf);
            h.update(&buf[..px * pat_len]);
            remaining -= px;
        }
    })
}

#[cfg_attr(not(test), allow(dead_code))] // slice-2 command-engine door (in-crate tests exercise it today)
fn splat_dense<T: Channel>(value: [f32; 4], model: ColorModelKind) -> TilePixels<T> {
    let channels = model.channels();
    let mut data = vec![T::ZERO; TILE * TILE * channels].into_boxed_slice();
    let mut px = [T::ZERO; 5];
    for (c, slot) in px.iter_mut().enumerate().take(channels) {
        *slot = T::from_f32(value.get(c).copied().unwrap_or(0.0));
    }
    for chunk in data.chunks_exact_mut(channels) {
        chunk.copy_from_slice(&px[..channels]);
    }
    TilePixels::new(data, model)
}

/// Bitwise-zero scan (ES §2.4 transparency scan): true iff every scalar's bit
/// pattern is zero. Float-semantics-free: an f32 −0.0 is content.
fn is_zero_dense<T: Channel>(data: &[T]) -> bool {
    data.iter().all(|s| s.bits() == 0)
}

/// Uniformity scan (ES §2.4): splat the first pixel and compare-scan the
/// rest (bit-pattern compare). On full match returns the normalized channel
/// values, zero-padded to 4. Models with CHANNELS > 4 never collapse (§2.3
/// model limit) — the caller guards.
fn scan_uniform_dense<T: Channel>(data: &[T], channels: usize) -> Option<[f32; 4]> {
    let first = &data[..channels];
    for px in data.chunks_exact(channels).skip(1) {
        for c in 0..channels {
            if px[c].bits() != first[c].bits() {
                return None;
            }
        }
    }
    let mut out = [0.0f32; 4];
    for (c, slot) in out.iter_mut().enumerate().take(channels.min(4)) {
        *slot = first[c].to_f32();
    }
    Some(out)
}

fn read_px_dense<T: Channel>(data: &[T], channels: usize, x: usize, y: usize) -> [f32; 4] {
    let i = (y * TILE + x) * channels;
    let mut out = [0.0f32; 4];
    for (c, slot) in out.iter_mut().enumerate().take(channels.min(4)) {
        *slot = data[i + c].to_f32();
    }
    out
}

#[cfg_attr(not(test), allow(dead_code))] // slice-2 command-engine door (in-crate tests exercise it today)
fn write_px_dense<T: Channel>(data: &mut [T], channels: usize, x: usize, y: usize, v: [f32; 4]) {
    let i = (y * TILE + x) * channels;
    for c in 0..channels.min(4) {
        data[i + c] = T::from_f32(v[c]);
    }
    // Channels beyond the 4-slot value (CmykA's 5th) are zeroed — the write
    // API grows to full-width values with the CP-1.0 channels work (ES §14).
    for c in 4..channels {
        data[i + c] = T::ZERO;
    }
}

impl TileStorage {
    /// The universal absent/transparent representation: every storage scalar
    /// zero, allocation-free (ES §2.3). The `kind`/`model` arguments document
    /// the owning context (a transparent tile encodes identically at every
    /// depth, so they do not affect the value).
    pub fn transparent(_kind: ChannelKind, _model: ColorModelKind) -> TileStorage {
        TileStorage::Uniform { value: [0.0; 4] }
    }

    /// Dense pool allocation splat-filled with the encoded value (ES §2.3
    /// materialization; `make_mut` is the caller).
    #[cfg_attr(not(test), allow(dead_code))] // slice-2 command-engine door (in-crate tests exercise it today)
    pub(crate) fn materialize(
        value: [f32; 4],
        kind: ChannelKind,
        model: ColorModelKind,
    ) -> TileStorage {
        match kind {
            ChannelKind::U8 => TileStorage::U8(splat_dense(value, model)),
            #[cfg(feature = "depth-u16")]
            ChannelKind::U16 => TileStorage::U16(splat_dense(value, model)),
            ChannelKind::F32 => TileStorage::F32(splat_dense(value, model)),
            _ => unreachable!("depth {kind:?} is not storage-backed in this build"),
        }
    }

    /// Non-zero solid tile (ES §2.3 formation rules: fill commands, prune
    /// collapse). Canonicalizes through the depth; refuses (debug) models
    /// with more than four channels.
    #[cfg_attr(not(test), allow(dead_code))] // slice-2 command-engine door (in-crate tests exercise it today)
    pub(crate) fn uniform(raw: [f32; 4], kind: ChannelKind, model: ColorModelKind) -> TileStorage {
        debug_assert!(
            model.channels() <= 4,
            "Uniform applies to models with CHANNELS <= 4 (ES §2.3)"
        );
        TileStorage::Uniform {
            value: canonical_uniform(raw, kind, model),
        }
    }

    /// Storage depth of a pixel payload; `None` for `Uniform` (the owning map
    /// carries the context, ES §2.3).
    pub fn kind(&self) -> Option<ChannelKind> {
        match self {
            TileStorage::U8(_) => Some(ChannelKind::U8),
            #[cfg(feature = "depth-u16")]
            TileStorage::U16(_) => Some(ChannelKind::U16),
            TileStorage::F32(_) => Some(ChannelKind::F32),
            TileStorage::Uniform { .. } => None,
        }
    }

    pub fn model(&self) -> Option<ColorModelKind> {
        match self {
            TileStorage::U8(p) => Some(p.model()),
            #[cfg(feature = "depth-u16")]
            TileStorage::U16(p) => Some(p.model()),
            TileStorage::F32(p) => Some(p.model()),
            TileStorage::Uniform { .. } => None,
        }
    }

    pub fn is_uniform(&self) -> bool {
        matches!(self, TileStorage::Uniform { .. })
    }

    pub fn uniform_value(&self) -> Option<[f32; 4]> {
        match self {
            TileStorage::Uniform { value } => Some(*value),
            _ => None,
        }
    }

    /// Actual heap payload bytes (`Uniform` holds none and is accounted as
    /// metadata, ES §2.3).
    pub fn byte_len(&self) -> usize {
        match self {
            TileStorage::U8(p) => p.data.len(),
            #[cfg(feature = "depth-u16")]
            TileStorage::U16(p) => p.data.len() * 2,
            TileStorage::F32(p) => p.data.len() * 4,
            TileStorage::Uniform { .. } => 0,
        }
    }

    /// Nominal storage size of one tile at a depth/model —
    /// `TILE*TILE*channels*bytes` (the §10.4 undo byte-accounting unit).
    pub const fn nominal_byte_len(kind: ChannelKind, model: ColorModelKind) -> usize {
        TILE * TILE * model.channels() * kind.bytes_per_channel()
    }

    /// BLAKE3-256 content identity over the uncompressed LE payload bytes
    /// (ES §3.8). The `kind`/`model` arguments exist for the `Uniform`
    /// variant and are debug-asserted against pixel variants (ES §2.3).
    pub fn content_hash(&self, kind: ChannelKind, model: ColorModelKind) -> ContentHash {
        match self {
            TileStorage::U8(p) => {
                debug_assert_eq!((Some(kind), Some(model)), (self.kind(), self.model()));
                hash_dense(&p.data)
            }
            #[cfg(feature = "depth-u16")]
            TileStorage::U16(p) => {
                debug_assert_eq!((Some(kind), Some(model)), (self.kind(), self.model()));
                hash_dense(&p.data)
            }
            TileStorage::F32(p) => {
                debug_assert_eq!((Some(kind), Some(model)), (self.kind(), self.model()));
                hash_dense(&p.data)
            }
            TileStorage::Uniform { value } => {
                let ch = model.channels();
                match kind {
                    ChannelKind::U8 => hash_uniform::<u8>(value, ch),
                    #[cfg(feature = "depth-u16")]
                    ChannelKind::U16 => hash_uniform::<u16>(value, ch),
                    ChannelKind::F32 => hash_uniform::<f32>(value, ch),
                    _ => unreachable!("depth {kind:?} is not storage-backed in this build"),
                }
            }
        }
    }

    /// ES §2.4 transparency scan: true iff every stored scalar's bit pattern
    /// is zero (for `Uniform`, iff the encoded pattern is zero at this
    /// depth). Absence-equivalent tiles are pruned from the map.
    pub fn is_bitwise_zero(&self, _kind: ChannelKind, _model: ColorModelKind) -> bool {
        match self {
            TileStorage::U8(p) => is_zero_dense(&p.data),
            #[cfg(feature = "depth-u16")]
            TileStorage::U16(p) => is_zero_dense(&p.data),
            TileStorage::F32(p) => is_zero_dense(&p.data),
            // Canonicalization already ran at formation, so the value bits
            // are exactly what materialized pixels would hold at the owning
            // depth: +0.0 encodes to zero bits everywhere; a canonical −0.0
            // (possible only at f32 depth) is content, matching the dense
            // scan's float-semantics-free rule.
            TileStorage::Uniform { value } => value.iter().all(|v| v.bits() == 0),
        }
    }

    /// ES §2.4 uniformity scan for the prune pass. Returns the canonical
    /// uniform value when every pixel matches the first (bit-compare); `None`
    /// for models with CHANNELS > 4 (never collapse, §2.3) and for `Uniform`
    /// (already collapsed).
    pub fn scan_uniform(&self) -> Option<[f32; 4]> {
        let model = self.model()?;
        if model.channels() > 4 {
            return None;
        }
        match self {
            TileStorage::U8(p) => scan_uniform_dense(&p.data, model.channels()),
            #[cfg(feature = "depth-u16")]
            TileStorage::U16(p) => scan_uniform_dense(&p.data, model.channels()),
            TileStorage::F32(p) => scan_uniform_dense(&p.data, model.channels()),
            TileStorage::Uniform { .. } => None,
        }
    }

    /// Read one pixel as normalized f32 components, zero-padded to 4.
    /// `model` supplies the channel count for `Uniform` (debug-asserted
    /// against pixel variants).
    pub fn read_pixel_f32(&self, x: usize, y: usize, model: ColorModelKind) -> [f32; 4] {
        debug_assert!(x < TILE && y < TILE);
        match self {
            TileStorage::U8(p) => {
                debug_assert_eq!(p.model(), model);
                read_px_dense(&p.data, model.channels(), x, y)
            }
            #[cfg(feature = "depth-u16")]
            TileStorage::U16(p) => {
                debug_assert_eq!(p.model(), model);
                read_px_dense(&p.data, model.channels(), x, y)
            }
            TileStorage::F32(p) => {
                debug_assert_eq!(p.model(), model);
                read_px_dense(&p.data, model.channels(), x, y)
            }
            TileStorage::Uniform { value } => *value,
        }
    }

    /// Write one pixel from normalized f32 components. Dense variants only —
    /// the owning map's `make_mut` materializes `Uniform` before handing out
    /// mutable access (ES §2.3), so hitting `Uniform` here is a logic error.
    #[cfg_attr(not(test), allow(dead_code))] // slice-2 command-engine door (in-crate tests exercise it today)
    pub(crate) fn write_pixel_f32(&mut self, x: usize, y: usize, v: [f32; 4]) {
        debug_assert!(x < TILE && y < TILE);
        match self {
            TileStorage::U8(p) => write_px_dense(&mut p.data, p.model.channels(), x, y, v),
            #[cfg(feature = "depth-u16")]
            TileStorage::U16(p) => write_px_dense(&mut p.data, p.model.channels(), x, y, v),
            TileStorage::F32(p) => write_px_dense(&mut p.data, p.model.channels(), x, y, v),
            TileStorage::Uniform { .. } => {
                unreachable!("write_pixel_f32 on Uniform — make_mut materializes first (ES §2.3)")
            }
        }
    }
}

/// Summary Debug (ES §1.6: diagnostics never dump pixel payloads).
impl core::fmt::Debug for TileStorage {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            TileStorage::Uniform { value } => {
                f.debug_struct("Uniform").field("value", value).finish()
            }
            other => f
                .debug_struct("TilePixels")
                .field("kind", &other.kind())
                .field("model", &other.model())
                .field("bytes", &other.byte_len())
                .finish(),
        }
    }
}

impl<C: ColorModel> From<Tile<u8, C>> for TileStorage {
    fn from(t: Tile<u8, C>) -> TileStorage {
        TileStorage::U8(TilePixels::new(t.into_payload(), C::KIND))
    }
}

#[cfg(feature = "depth-u16")]
impl<C: ColorModel> From<Tile<u16, C>> for TileStorage {
    fn from(t: Tile<u16, C>) -> TileStorage {
        TileStorage::U16(TilePixels::new(t.into_payload(), C::KIND))
    }
}

impl<C: ColorModel> From<Tile<f32, C>> for TileStorage {
    fn from(t: Tile<f32, C>) -> TileStorage {
        TileStorage::F32(TilePixels::new(t.into_payload(), C::KIND))
    }
}

#[cfg(test)]
mod tests {
    use super::super::model::model;
    use super::*;

    const RGBA8: (ChannelKind, ColorModelKind) = (ChannelKind::U8, ColorModelKind::Rgba);

    #[test]
    fn uniform_and_materialized_twin_share_identity() {
        // The §2.3/§3.8 rule that makes Uniform invisible to identity.
        let raw = [0.5, 0.25, 0.125, 1.0];
        let canonical = canonical_uniform(raw, RGBA8.0, RGBA8.1);
        let uniform = TileStorage::uniform(raw, RGBA8.0, RGBA8.1);
        let dense = TileStorage::materialize(canonical, RGBA8.0, RGBA8.1);
        assert_eq!(
            uniform.content_hash(RGBA8.0, RGBA8.1),
            dense.content_hash(RGBA8.0, RGBA8.1)
        );
    }

    #[test]
    fn uniform_hash_matches_generic_tile_fill() {
        // Cross-check against the generic Tile<T,C> path.
        let raw = [0.5, 0.25, 0.125, 1.0];
        let v8 = raw.map(u8::from_f32);
        let tile: Tile<u8, model::Rgba> = Tile::filled(v8);
        let via_tile = TileStorage::from(tile);
        let uniform = TileStorage::uniform(raw, RGBA8.0, RGBA8.1);
        assert_eq!(
            via_tile.content_hash(RGBA8.0, RGBA8.1),
            uniform.content_hash(RGBA8.0, RGBA8.1)
        );
    }

    #[test]
    fn transparent_uniform_hashes_like_zeroed_dense() {
        let t = TileStorage::transparent(RGBA8.0, RGBA8.1);
        let z = TileStorage::from(Tile::<u8, model::Rgba>::zeroed());
        assert_eq!(
            t.content_hash(RGBA8.0, RGBA8.1),
            z.content_hash(RGBA8.0, RGBA8.1)
        );
    }

    #[test]
    fn identical_content_same_key_different_content_different_key() {
        let mut a: Tile<u8, model::Rgba> = Tile::zeroed();
        let mut b: Tile<u8, model::Rgba> = Tile::zeroed();
        a.pixel_mut(7, 9).copy_from_slice(&[1, 2, 3, 4]);
        b.pixel_mut(7, 9).copy_from_slice(&[1, 2, 3, 4]);
        let (sa, sb) = (TileStorage::from(a), TileStorage::from(b));
        assert_eq!(
            sa.content_hash(RGBA8.0, RGBA8.1),
            sb.content_hash(RGBA8.0, RGBA8.1)
        );

        let mut c: Tile<u8, model::Rgba> = Tile::zeroed();
        c.pixel_mut(7, 9).copy_from_slice(&[1, 2, 3, 5]);
        let sc = TileStorage::from(c);
        assert_ne!(
            sa.content_hash(RGBA8.0, RGBA8.1),
            sc.content_hash(RGBA8.0, RGBA8.1)
        );
    }

    #[test]
    fn hash_covers_payload_bytes_so_depth_changes_identity() {
        // The same normalized values at different depths are different
        // payload bytes, hence different identities (dedup never crosses
        // depth; §2.5 region_hash additionally frames kind/model).
        let u8_tile =
            TileStorage::materialize([1.0, 0.0, 0.0, 1.0], ChannelKind::U8, ColorModelKind::Rgba);
        let f32_tile =
            TileStorage::materialize([1.0, 0.0, 0.0, 1.0], ChannelKind::F32, ColorModelKind::Rgba);
        assert_ne!(
            u8_tile.content_hash(ChannelKind::U8, ColorModelKind::Rgba),
            f32_tile.content_hash(ChannelKind::F32, ColorModelKind::Rgba)
        );
    }

    #[test]
    fn canonicalization_zero_pads_beyond_channels() {
        let v = canonical_uniform([0.4, 0.9, 0.9, 0.9], ChannelKind::F32, ColorModelKind::Gray);
        assert_eq!(v, [0.4, 0.0, 0.0, 0.0]);
        // u8 canonicalization snaps to representable steps.
        let v8 = canonical_uniform([0.5, 0.0, 0.0, 0.0], ChannelKind::U8, ColorModelKind::Gray);
        assert_eq!(v8[0], 128.0 / 255.0);
    }

    #[test]
    fn bitwise_zero_scan_treats_negative_zero_as_content() {
        let mut t: Tile<f32, model::Gray> = Tile::zeroed();
        let s0 = TileStorage::from(t.clone());
        assert!(s0.is_bitwise_zero(ChannelKind::F32, ColorModelKind::Gray));
        t.pixel_mut(0, 0)[0] = -0.0;
        let s1 = TileStorage::from(t);
        assert!(!s1.is_bitwise_zero(ChannelKind::F32, ColorModelKind::Gray));
    }

    #[test]
    fn uniformity_scan_finds_solid_and_rejects_mixed() {
        let solid = TileStorage::materialize([0.25, 0.5, 0.75, 1.0], RGBA8.0, RGBA8.1);
        let v = solid.scan_uniform().expect("solid tile scans uniform");
        let canonical = canonical_uniform([0.25, 0.5, 0.75, 1.0], RGBA8.0, RGBA8.1);
        assert_eq!(v, canonical);

        let mut mixed: Tile<u8, model::Rgba> = Tile::filled([9, 9, 9, 9]);
        mixed.pixel_mut(255, 255)[2] = 8;
        assert!(TileStorage::from(mixed).scan_uniform().is_none());
    }

    #[test]
    fn read_write_pixel_round_trip() {
        let mut s = TileStorage::materialize([0.0; 4], ChannelKind::F32, ColorModelKind::Rgba);
        s.write_pixel_f32(10, 20, [0.1, 0.2, 0.3, 0.4]);
        assert_eq!(
            s.read_pixel_f32(10, 20, ColorModelKind::Rgba),
            [0.1, 0.2, 0.3, 0.4]
        );
        assert_eq!(s.read_pixel_f32(11, 20, ColorModelKind::Rgba), [0.0; 4]);
        // Uniform reads its value everywhere.
        let u = TileStorage::uniform([0.5, 0.5, 0.5, 1.0], ChannelKind::F32, ColorModelKind::Rgba);
        assert_eq!(
            u.read_pixel_f32(200, 3, ColorModelKind::Rgba),
            [0.5, 0.5, 0.5, 1.0]
        );
    }

    #[test]
    fn byte_lengths() {
        let dense = TileStorage::materialize([0.0; 4], RGBA8.0, RGBA8.1);
        assert_eq!(dense.byte_len(), 256 * 256 * 4);
        assert_eq!(TileStorage::transparent(RGBA8.0, RGBA8.1).byte_len(), 0);
        assert_eq!(
            TileStorage::nominal_byte_len(ChannelKind::F32, ColorModelKind::CmykA),
            256 * 256 * 5 * 4
        );
    }

    #[test]
    fn transparent_works_for_five_channel_models() {
        // CmykA never forms a *non-zero* Uniform, but the all-zero absent
        // representation and its materialization are valid (ES §2.3).
        let t = TileStorage::transparent(ChannelKind::U8, ColorModelKind::CmykA);
        assert!(t.is_bitwise_zero(ChannelKind::U8, ColorModelKind::CmykA));
        let m = TileStorage::materialize([0.0; 4], ChannelKind::U8, ColorModelKind::CmykA);
        assert_eq!(m.byte_len(), 256 * 256 * 5);
        assert_eq!(
            t.content_hash(ChannelKind::U8, ColorModelKind::CmykA),
            m.content_hash(ChannelKind::U8, ColorModelKind::CmykA)
        );
    }
}
