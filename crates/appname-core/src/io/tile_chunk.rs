// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §7.4 (tile chunk binary format v1) — the pinned 48-byte
//! little-endian header + raw pixel payload.
//!
//! [NORMATIVE §7.4] magic `"BGTILE"` + format 1.0 (the magic byte constant
//! is fixed by the published format — flagged in the lane log as a
//! spec-fixed constant); checksum = xxHash64 of the *uncompressed* payload
//! (integrity-only, §0.3); readers run every validation before the
//! allocation it would make unnecessary, and per-tile failures resolve
//! *gracefully* (transparent + warning), never a crash.
//!
//! CP-0.1 writer emits `compression = 0` (none): §3.8 sanctions
//! "zstd (or none)" and the zstd/lz4 crates have no deps records yet;
//! readers refuse compressed chunks gracefully per the §7.4 unknown-value
//! rule (lane-log note).

use crate::tile::channel::ChannelKind;
use crate::tile::map::TileCoord;
use crate::tile::model::ColorModelKind;
use crate::tile::storage::TileStorage;
use crate::tile::TILE;

/// §7.4 magic + version (normative format constants).
pub const MAGIC: &[u8; 6] = b"BGTILE";
pub const FMT_MAJOR: u8 = 1;
pub const FMT_MINOR: u8 = 0;

/// §7.4 rule 6: the decompression-bomb gate — largest legal payload
/// (f32 CmykA is not legal on the wire? §7.4 fixes the cap at RGBA-f32) +
/// fixed slack.
pub const TILE_MAX_RAW: u64 = 256 * 256 * 4 * 4 + 4096;

/// One graceful per-tile failure (ES §7.4 rule 10): the tile reads as
/// transparent and loading continues; the open report collects these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileWarning {
    pub entry: String,
    pub detail: String,
}

/// Encode one tile as a §7.4 chunk (compression = none). `kind`/`model`
/// come from the owning buffer (`Uniform` payloads materialize into the
/// scratch encode — §2.3: no on-disk Uniform representation).
pub(crate) fn write_tile_chunk(
    coord: TileCoord,
    tile: &TileStorage,
    kind: ChannelKind,
    model: ColorModelKind,
    premultiplied: bool,
) -> Vec<u8> {
    let channels = model.channels();
    let bytes_per = kind.bytes_per_channel();
    let payload_len = TILE * TILE * channels * bytes_per;
    let mut payload = vec![0u8; payload_len];
    encode_payload_le(tile, kind, model, &mut payload);

    let mut out = Vec::with_capacity(48 + payload.len());
    out.extend_from_slice(MAGIC);
    out.push(FMT_MAJOR);
    out.push(FMT_MINOR);
    out.extend_from_slice(&(TILE as u32).to_le_bytes()); // width
    out.extend_from_slice(&(TILE as u32).to_le_bytes()); // height
    out.extend_from_slice(&coord.x.to_le_bytes());
    out.extend_from_slice(&coord.y.to_le_bytes());
    out.push(coord.lod);
    out.push(kind.as_u8()); // pixel_format: same 0..3 coding as §7.4
    out.push(model.as_u8()); // color_model: same 0..7 coding as §7.4
    out.push(if premultiplied { 1 } else { 0 }); // flags: bit0 premultiplied
    out.push(0); // compression = none
    out.extend_from_slice(&[0, 0, 0]); // reserved
    out.extend_from_slice(&(payload_len as u64).to_le_bytes());
    out.extend_from_slice(&super::integrity::xxh64(&payload).to_le_bytes());
    out.extend_from_slice(&payload);
    out
}

/// LE scalar encode of a full tile payload (row-major, channel-interleaved;
/// same encoding the BLAKE3 content hash runs over, §3.8).
pub(crate) fn encode_payload_le(
    tile: &TileStorage,
    kind: ChannelKind,
    model: ColorModelKind,
    out: &mut [u8],
) {
    let channels = model.channels();
    let bytes_per = kind.bytes_per_channel();
    for y in 0..TILE {
        for x in 0..TILE {
            let px = tile.read_pixel_f32(x, y, model);
            for c in 0..channels {
                let raw = px.get(c).copied().unwrap_or(0.0);
                let off = ((y * TILE + x) * channels + c) * bytes_per;
                match kind {
                    ChannelKind::U8 => {
                        out[off] = (raw.clamp(0.0, 1.0) * 255.0).round() as u8;
                    }
                    ChannelKind::U16 => {
                        let v = (raw.clamp(0.0, 1.0) * 65535.0).round() as u16;
                        out[off..off + 2].copy_from_slice(&v.to_le_bytes());
                    }
                    ChannelKind::F32 => {
                        out[off..off + 4].copy_from_slice(&raw.to_le_bytes());
                    }
                    ChannelKind::F16 => unreachable!("paused depth never reaches the writer"),
                }
            }
        }
    }
}

/// Parsed chunk: coordinate + a dense [`TileStorage`] payload.
pub(crate) struct ParsedChunk {
    pub coord: TileCoord,
    pub kind: ChannelKind,
    pub model: ColorModelKind,
    pub premultiplied: bool,
    pub tile: TileStorage,
}

/// Per-tile outcome per the §7.4 graceful rule.
pub(crate) enum ChunkOutcome {
    Ok(Box<ParsedChunk>),
    /// Tile reads as transparent; loading continues (§7.4 rule 10).
    Transparent(TileWarning),
}

/// §7.4 `read_tile_chunk` with the normative validation order (each check
/// before the allocation it would have made unnecessary).
pub(crate) fn read_tile_chunk(entry: &str, bytes: &[u8]) -> ChunkOutcome {
    let warn = |detail: String| {
        ChunkOutcome::Transparent(TileWarning {
            entry: entry.to_string(),
            detail,
        })
    };
    // 1. Fixed header present.
    if bytes.len() < 48 {
        return warn(format!("truncated chunk: {} bytes < 48", bytes.len()));
    }
    // 2. Magic.
    if &bytes[0..6] != MAGIC {
        return warn("not a tile chunk (bad magic)".into());
    }
    // 3. Major version (per-tile graceful; the file-level gate is
    //    manifest.requires.minReader). Minor is never checked.
    if bytes[6] != FMT_MAJOR {
        return warn(format!("unsupported chunk format major {}", bytes[6]));
    }
    let width = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    let height = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
    let tile_x = i32::from_le_bytes(bytes[16..20].try_into().unwrap());
    let tile_y = i32::from_le_bytes(bytes[20..24].try_into().unwrap());
    let lod = bytes[24];
    let pixel_format = bytes[25];
    let color_model = bytes[26];
    let flags = bytes[27];
    let compression = bytes[28];
    let uncompressed_len = u64::from_le_bytes(bytes[32..40].try_into().unwrap());
    let checksum = u64::from_le_bytes(bytes[40..48].try_into().unwrap());

    // 4. Enum ranges (reserved flag bits ignored on read).
    if pixel_format > 3 || color_model > 7 || compression > 2 {
        return warn(format!(
            "out-of-range enums (pixel_format {pixel_format}, color_model {color_model}, compression {compression})"
        ));
    }
    let kind = match pixel_format {
        0 => ChannelKind::U8,
        1 => ChannelKind::U16,
        2 => ChannelKind::F16,
        _ => ChannelKind::F32,
    };
    let model = match color_model {
        0 => ColorModelKind::Gray,
        1 => ColorModelKind::GrayA,
        2 => ColorModelKind::Rgb,
        3 => ColorModelKind::Rgba,
        4 => ColorModelKind::Cmyk,
        5 => ColorModelKind::CmykA,
        6 => ColorModelKind::Lab,
        _ => ColorModelKind::LabA,
    };
    // Build-level reality: paused depths cannot materialize — graceful.
    if !kind.is_storage_backed() {
        return warn(format!(
            "depth {kind:?} is not storage-backed in this build (paused)"
        ));
    }
    // CP-0.1 reader: compressed chunks await the zstd/lz4 records —
    // graceful per the §7.4 unknown-value spirit (lane-log note).
    if compression != 0 {
        return warn(format!(
            "compression {compression} is not readable in this build"
        ));
    }
    // 5. Geometry (CP-0.1 writer always emits full 256² tiles; clipped-edge
    //    chunks materialize zero-filled per §2.3 — accepted on read).
    if width == 0 || width > 256 || height == 0 || height > 256 {
        return warn(format!("bad geometry {width}x{height}"));
    }
    // 6. Size gate BEFORE any payload allocation.
    if uncompressed_len > TILE_MAX_RAW {
        return warn(format!(
            "uncompressed_len {uncompressed_len} exceeds the §7.4 cap"
        ));
    }
    // 7. Exact-size check.
    let expect =
        width as u64 * height as u64 * model.channels() as u64 * kind.bytes_per_channel() as u64;
    if uncompressed_len != expect {
        return warn(format!(
            "size mismatch: header {uncompressed_len}, computed {expect}"
        ));
    }
    // 8. Payload present in full (compression = none: stored verbatim).
    let payload = &bytes[48..];
    if payload.len() as u64 != uncompressed_len {
        return warn(format!(
            "payload length {} does not match uncompressed_len {uncompressed_len}",
            payload.len()
        ));
    }
    // 9. Checksum.
    if super::integrity::xxh64(payload) != checksum {
        return warn("checksum mismatch".into());
    }

    // Materialize into a full 256² tile, zero-filling any clipped remainder
    // (§2.3 document-edge rule).
    let mut tile = TileStorage::materialize([0.0; 4], kind, model);
    decode_payload_le(
        &mut tile,
        payload,
        width as usize,
        height as usize,
        kind,
        model,
    );
    ChunkOutcome::Ok(Box::new(ParsedChunk {
        coord: TileCoord::new(lod, tile_x, tile_y),
        kind,
        model,
        premultiplied: flags & 1 != 0,
        tile,
    }))
}

pub(crate) fn decode_payload_le(
    tile: &mut TileStorage,
    payload: &[u8],
    width: usize,
    height: usize,
    kind: ChannelKind,
    model: ColorModelKind,
) {
    let channels = model.channels();
    let bytes_per = kind.bytes_per_channel();
    for y in 0..height {
        for x in 0..width {
            let mut px = [0.0f32; 4];
            for (c, slot) in px.iter_mut().enumerate().take(channels.min(4)) {
                let off = ((y * width + x) * channels + c) * bytes_per;
                *slot = match kind {
                    ChannelKind::U8 => payload[off] as f32 / 255.0,
                    ChannelKind::U16 => {
                        u16::from_le_bytes(payload[off..off + 2].try_into().unwrap()) as f32
                            / 65535.0
                    }
                    ChannelKind::F32 => {
                        f32::from_le_bytes(payload[off..off + 4].try_into().unwrap())
                    }
                    ChannelKind::F16 => unreachable!("graceful-refused before decode"),
                };
            }
            tile.write_pixel_f32(x, y, px);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::ContentHash;

    const K: ChannelKind = ChannelKind::U8;
    const M: ColorModelKind = ColorModelKind::Rgba;

    fn sample_tile() -> TileStorage {
        let mut t = TileStorage::materialize([0.0; 4], K, M);
        t.write_pixel_f32(3, 5, [0.5, 0.25, 0.0, 1.0]);
        t.write_pixel_f32(255, 255, [0.0, 0.0, 0.0, 1.0]);
        t
    }

    fn chunk() -> Vec<u8> {
        write_tile_chunk(TileCoord::new(0, 2, -1), &sample_tile(), K, M, true)
    }

    #[test]
    fn round_trip_preserves_identity_and_coord() {
        let bytes = chunk();
        assert_eq!(&bytes[0..6], b"BGTILE");
        match read_tile_chunk("layers/x/tiles/0/2_-1.tile", &bytes) {
            ChunkOutcome::Ok(parsed) => {
                assert_eq!(parsed.coord, TileCoord::new(0, 2, -1));
                assert_eq!(parsed.kind, K);
                assert_eq!(parsed.model, M);
                assert!(parsed.premultiplied);
                assert_eq!(
                    parsed.tile.content_hash(K, M),
                    sample_tile().content_hash(K, M),
                    "byte-lossless round trip"
                );
            }
            ChunkOutcome::Transparent(w) => panic!("unexpected warning: {w:?}"),
        }
    }

    #[test]
    fn uniform_tile_round_trips_via_materialized_payload() {
        let uniform = TileStorage::uniform([0.5, 0.25, 0.125, 1.0], K, M);
        let bytes = write_tile_chunk(TileCoord::new(0, 0, 0), &uniform, K, M, true);
        match read_tile_chunk("e", &bytes) {
            ChunkOutcome::Ok(parsed) => {
                assert_eq!(parsed.tile.content_hash(K, M), uniform.content_hash(K, M));
            }
            ChunkOutcome::Transparent(w) => panic!("unexpected warning: {w:?}"),
        }
    }

    #[test]
    fn f32_payload_round_trips_bitwise() {
        let mut t = TileStorage::materialize([0.0; 4], ChannelKind::F32, M);
        t.write_pixel_f32(9, 9, [3.5, 0.001, -0.25, 0.5]); // HDR premult (§6.4 f32)
        let bytes = write_tile_chunk(TileCoord::new(0, 0, 0), &t, ChannelKind::F32, M, true);
        match read_tile_chunk("e", &bytes) {
            ChunkOutcome::Ok(parsed) => {
                let hash_in = t.content_hash(ChannelKind::F32, M);
                let hash_out = parsed.tile.content_hash(ChannelKind::F32, M);
                assert_eq!(hash_in, hash_out);
                assert_eq!(
                    parsed.tile.read_pixel_f32(9, 9, M),
                    [3.5, 0.001, -0.25, 0.5]
                );
            }
            ChunkOutcome::Transparent(w) => panic!("unexpected warning: {w:?}"),
        }
    }

    #[test]
    fn writer_payload_matches_blake3_identity_encoding() {
        // §3.8/§7.4 coherence: the chunk payload bytes hash (BLAKE3) to the
        // tile's content identity.
        let t = sample_tile();
        let bytes = chunk();
        let payload = &bytes[48..];
        assert_eq!(ContentHash::of_bytes(payload), t.content_hash(K, M));
    }

    #[test]
    fn validation_order_failures_are_graceful() {
        let good = chunk();
        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("truncated", good[..30].to_vec()),
            ("bad magic", {
                let mut b = good.clone();
                b[0] = b'X';
                b
            }),
            ("bad major", {
                let mut b = good.clone();
                b[6] = 9;
                b
            }),
            ("enum range", {
                let mut b = good.clone();
                b[25] = 77;
                b
            }),
            ("compressed unsupported", {
                let mut b = good.clone();
                b[28] = 1;
                b
            }),
            ("geometry", {
                let mut b = good.clone();
                b[8..12].copy_from_slice(&0u32.to_le_bytes());
                b
            }),
            ("bomb gate", {
                let mut b = good.clone();
                b[32..40].copy_from_slice(&(TILE_MAX_RAW + 1).to_le_bytes());
                b
            }),
            ("size mismatch", {
                let mut b = good.clone();
                b[32..40].copy_from_slice(&12u64.to_le_bytes());
                b
            }),
            ("checksum", {
                let mut b = good.clone();
                let last = b.len() - 1;
                b[last] ^= 0xFF;
                b
            }),
        ];
        for (name, bytes) in cases {
            match read_tile_chunk("e", &bytes) {
                ChunkOutcome::Transparent(w) => {
                    assert!(!w.detail.is_empty(), "{name}: warning carries detail");
                }
                ChunkOutcome::Ok(_) => panic!("{name}: must not parse"),
            }
        }
    }

    #[test]
    fn reserved_bytes_are_zero_and_ignored_on_read() {
        let mut b = chunk();
        assert_eq!(&b[29..32], &[0, 0, 0], "writer emits zero reserved bytes");
        // Reserved flag bits + reserved bytes are ignored on read (§7.4).
        b[27] |= 0b1111_1100;
        b[29] = 7;
        match read_tile_chunk("e", &b) {
            ChunkOutcome::Ok(_) => {}
            ChunkOutcome::Transparent(w) => panic!("reserved data must be ignored: {w:?}"),
        }
    }
}
