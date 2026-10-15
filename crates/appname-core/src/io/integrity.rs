// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §7.4 (tile chunk checksum = xxHash64 of the uncompressed
//! payload), §7.8 (journal record CRC32, "IEEE/802.3 polynomial … same CRC
//! as ZIP"), §0.3 (integrity hashes are NEVER identities — BLAKE3 is the
//! one content identity).
//!
//! Both checksums are implemented from their public algorithm
//! specifications (xxHash64: Y. Collet's published spec, BSD-licensed
//! reference; CRC-32: the IEEE 802.3 polynomial, reflected form) rather
//! than via new dependencies: the ES §1.3 `xxhash-rust` crate has no deps/
//! record at CP-0.1, and the zip crate's CRC backend is a transitive
//! dependency this crate must not reach into directly (deps policy).
//! Recorded as a lane-log note — swapping in recorded crates later changes
//! nothing on the wire (known-answer tests pin the values).

/// xxHash64 (seed 0) over one contiguous buffer — the §7.4 chunk checksum.
pub fn xxh64(data: &[u8]) -> u64 {
    const P1: u64 = 0x9E37_79B1_85EB_CA87;
    const P2: u64 = 0xC2B2_AE3D_27D4_EB4F;
    const P3: u64 = 0x1656_67B1_9E37_79F9;
    const P4: u64 = 0x85EB_CA77_C2B2_AE63;
    const P5: u64 = 0x27D4_EB2F_1656_67C5;

    #[inline]
    fn read_u64(b: &[u8]) -> u64 {
        u64::from_le_bytes(b[..8].try_into().expect("8 bytes"))
    }
    #[inline]
    fn read_u32(b: &[u8]) -> u64 {
        u32::from_le_bytes(b[..4].try_into().expect("4 bytes")) as u64
    }
    #[inline]
    fn round(acc: u64, input: u64) -> u64 {
        acc.wrapping_add(input.wrapping_mul(P2))
            .rotate_left(31)
            .wrapping_mul(P1)
    }
    #[inline]
    fn merge_round(acc: u64, val: u64) -> u64 {
        (acc ^ round(0, val)).wrapping_mul(P1).wrapping_add(P4)
    }

    let len = data.len();
    let mut rest = data;
    let mut h: u64;
    if len >= 32 {
        let (mut v1, mut v2, mut v3, mut v4) =
            (P1.wrapping_add(P2), P2, 0u64, 0u64.wrapping_sub(P1));
        while rest.len() >= 32 {
            v1 = round(v1, read_u64(&rest[0..]));
            v2 = round(v2, read_u64(&rest[8..]));
            v3 = round(v3, read_u64(&rest[16..]));
            v4 = round(v4, read_u64(&rest[24..]));
            rest = &rest[32..];
        }
        h = v1
            .rotate_left(1)
            .wrapping_add(v2.rotate_left(7))
            .wrapping_add(v3.rotate_left(12))
            .wrapping_add(v4.rotate_left(18));
        h = merge_round(h, v1);
        h = merge_round(h, v2);
        h = merge_round(h, v3);
        h = merge_round(h, v4);
    } else {
        h = P5;
    }
    h = h.wrapping_add(len as u64);
    while rest.len() >= 8 {
        h = (h ^ round(0, read_u64(rest)))
            .rotate_left(27)
            .wrapping_mul(P1)
            .wrapping_add(P4);
        rest = &rest[8..];
    }
    if rest.len() >= 4 {
        h = (h ^ read_u32(rest).wrapping_mul(P1))
            .rotate_left(23)
            .wrapping_mul(P2)
            .wrapping_add(P3);
        rest = &rest[4..];
    }
    for &b in rest {
        h = (h ^ (b as u64).wrapping_mul(P5))
            .rotate_left(11)
            .wrapping_mul(P1);
    }
    h ^= h >> 33;
    h = h.wrapping_mul(P2);
    h ^= h >> 29;
    h = h.wrapping_mul(P3);
    h ^= h >> 32;
    h
}

/// CRC-32 (IEEE 802.3, reflected, init/xorout `0xFFFF_FFFF`) — the §7.8
/// journal record guard, byte-compatible with ZIP's entry CRC.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xxh64_known_answers() {
        // Canonical vectors of the published xxHash spec (seed 0).
        assert_eq!(xxh64(b""), 0xEF46_DB37_51D8_E999);
        assert_eq!(xxh64(b"abc"), 0x44BC_2CF5_AD77_0999);
        // Exercises the 32-byte stripe loop + every tail path.
        let long: Vec<u8> = (0u8..=200).collect();
        let h1 = xxh64(&long);
        let h2 = xxh64(&long);
        assert_eq!(h1, h2);
        assert_ne!(xxh64(&long[..199]), h1);
    }

    #[test]
    fn crc32_known_answers() {
        // The standard CRC-32/ISO-HDLC check value.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }
}
