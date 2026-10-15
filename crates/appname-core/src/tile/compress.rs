// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §3.8 (compression and content addressing), §3.4 (residency
//! tiers); deps/zstd.yaml + deps/lz4.yaml (both pre-authorized, first
//! consumer O-CORE).
//!
//! Cold tile payloads compress with **zstd level 3** (default,
//! precision-exact, lossless) or **lz4** (the §3.8 adaptive under-backlog
//! codec — faster, weaker ratio). Compression operates on the SAME
//! little-endian payload encoding the §3.8 content hash and the §7.4 chunk
//! writer use, so `compressed → decompressed → hash` is identity-stable by
//! construction. [NORMATIVE, §3.8] content identity is computed over the
//! UNCOMPRESSED payload bytes — codec choice is a residency detail and MUST
//! be invisible to identity.
//!
//! First consumer: §10.4 history spill ("cold undo states compress (zstd)
//! and spill to scratch"). The §3.4 live-tile page-out tier consumes the
//! same types when the residency/snapshot machinery (arc-swap publication)
//! lands with the render-integration wave.

use std::io::Read;

/// §3.8 codec tag. `None` is the raw-payload passthrough (also the §7.4
/// chunk value for uncompressed tiles).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TileCodec {
    None,
    Zstd,
    Lz4,
}

/// One compressed payload (§3.8 shape): codec, compressed bytes, and the
/// uncompressed length for allocation on revive.
pub struct CompressedTile {
    pub algo: TileCodec,
    pub bytes: Box<[u8]>,
    pub raw_len: usize,
}

/// zstd level 3 — the §3.8 default (precision-exact, lossless).
pub(crate) const ZSTD_SPILL_LEVEL: i32 = 3;

/// Compress a raw LE payload with the given codec.
pub(crate) fn compress_payload(raw: &[u8], algo: TileCodec) -> CompressedTile {
    let bytes: Box<[u8]> = match algo {
        TileCodec::None => raw.into(),
        TileCodec::Zstd => zstd::bulk::compress(raw, ZSTD_SPILL_LEVEL)
            .expect("zstd in-memory compression cannot fail on valid input")
            .into(),
        TileCodec::Lz4 => {
            let mut enc = lz4::EncoderBuilder::new()
                .build(Vec::new())
                .expect("lz4 in-memory encoder");
            std::io::Write::write_all(&mut enc, raw).expect("lz4 in-memory write");
            let (out, result) = enc.finish();
            result.expect("lz4 in-memory finish");
            out.into()
        }
    };
    CompressedTile {
        algo,
        bytes,
        raw_len: raw.len(),
    }
}

/// Decompress back to the exact raw payload. Length mismatches are refused
/// (a spill blob is trusted only for what it was recorded as).
pub(crate) fn decompress_payload(c: &CompressedTile) -> Result<Vec<u8>, String> {
    let raw = match c.algo {
        TileCodec::None => c.bytes.to_vec(),
        TileCodec::Zstd => zstd::bulk::decompress(&c.bytes, c.raw_len)
            .map_err(|e| format!("zstd revive failed: {e}"))?,
        TileCodec::Lz4 => {
            let mut dec = lz4::Decoder::new(std::io::Cursor::new(&c.bytes[..]))
                .map_err(|e| format!("lz4 revive failed: {e}"))?;
            let mut out = Vec::with_capacity(c.raw_len);
            dec.read_to_end(&mut out)
                .map_err(|e| format!("lz4 revive failed: {e}"))?;
            out
        }
    };
    if raw.len() != c.raw_len {
        return Err(format!(
            "revived payload length {} != recorded raw_len {}",
            raw.len(),
            c.raw_len
        ));
    }
    Ok(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_payload() -> Vec<u8> {
        // Mixed-entropy payload: a compressible ramp plus an f32-ish tail
        // (exercises both codecs' paths; content mirrors LE tile bytes).
        let mut v = Vec::with_capacity(64 * 1024);
        for i in 0..12_000u32 {
            v.push((i % 251) as u8);
        }
        for i in 0..4_000u32 {
            v.extend_from_slice(&(i as f32 * 0.37).to_le_bytes());
        }
        v
    }

    #[test]
    fn zstd_round_trip_is_bit_exact() {
        let raw = sample_payload();
        let c = compress_payload(&raw, TileCodec::Zstd);
        assert_eq!(c.algo, TileCodec::Zstd);
        assert!(c.bytes.len() < raw.len(), "ramp payload must compress");
        assert_eq!(decompress_payload(&c).unwrap(), raw);
    }

    #[test]
    fn lz4_round_trip_is_bit_exact() {
        let raw = sample_payload();
        let c = compress_payload(&raw, TileCodec::Lz4);
        assert_eq!(decompress_payload(&c).unwrap(), raw);
    }

    #[test]
    fn none_codec_passes_through() {
        let raw = sample_payload();
        let c = compress_payload(&raw, TileCodec::None);
        assert_eq!(&*c.bytes, &raw[..]);
        assert_eq!(decompress_payload(&c).unwrap(), raw);
    }

    #[test]
    fn truncated_blob_refuses_typed() {
        let raw = sample_payload();
        let mut c = compress_payload(&raw, TileCodec::Zstd);
        c.bytes = c.bytes[..c.bytes.len() / 2].into();
        assert!(decompress_payload(&c).is_err());
    }
}
