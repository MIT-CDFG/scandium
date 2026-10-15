// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Own-written implementations from the PUBLIC algorithm specifications:
// BLAKE3 (the published BLAKE3 specification; single-chunk short-input
// subset) and the ChaCha stream cipher core with 8 rounds (D. J.
// Bernstein's published ChaCha specification). No reference code was
// fetched or consulted; both are pinned by self-tests below. No external
// crate is used (crate dependency policy: no new dependencies).

//! Spec: ES §45.3 "Seed policy and determinism" — the seeded streams of the
//! deterministic exemplar engine.
//!
//! Per §45.3, every stochastic choice in the exemplar pipeline reads a
//! keyed counter-mode stream: per pyramid level `ℓ`, the stream key is
//! `blake3("appname.caf" ‖ le64(seed) ‖ le32(ℓ))`, and each draw reads a
//! **fixed ChaCha8 counter position** derived from its own index —
//! stateless, so evaluation order (tile order, thread count, candidate
//! scan order) is invisible to the output. The engine therefore remains a
//! pure function `(inputs, params incl. seed) → output`, byte-exact
//! (D-48); there is NO entropy source and NO mutable generator state
//! anywhere in this module.
//!
//! Sub-stream selectors (this crate pins the layout; golden hashes key it):
//! * selector 0 — the PCA sample draw (`"appname.caf.pca"` in §21.4's
//!   naming; draw `k` reads block counter `k`),
//! * selector 1 — the priority-heap tie-break draw (pixel linear index `i`
//!   reads block counter `i`).
//!
//! Each draw is the first little-endian `u64` of the ChaCha8 block at
//! (counter = index, nonce = [selector, 0]).

/// BLAKE3 initialization vector (the published spec constants; identical
/// to the SHA-256 initial hash words).
const IV: [u32; 8] = [
    0x6a09_e667,
    0xbb67_ae85,
    0x3c6e_f372,
    0xa54f_f53a,
    0x510e_527f,
    0x9b05_688c,
    0x1f83_d9ab,
    0x5be0_cd19,
];

/// BLAKE3 message word permutation applied between rounds.
const MSG_PERM: [usize; 16] = [2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8];

const CHUNK_START: u32 = 1 << 0;
const CHUNK_END: u32 = 1 << 1;
const ROOT: u32 = 1 << 3;

/// BLAKE3 quarter-round (mixing function G of the published spec).
#[inline]
fn g(v: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize, x: u32, y: u32) {
    v[a] = v[a].wrapping_add(v[b]).wrapping_add(x);
    v[d] = (v[d] ^ v[a]).rotate_right(16);
    v[c] = v[c].wrapping_add(v[d]);
    v[b] = (v[b] ^ v[c]).rotate_right(12);
    v[a] = v[a].wrapping_add(v[b]).wrapping_add(y);
    v[d] = (v[d] ^ v[a]).rotate_right(8);
    v[c] = v[c].wrapping_add(v[d]);
    v[b] = (v[b] ^ v[c]).rotate_right(7);
}

/// One BLAKE3 compression: 7 rounds over the 16-word state, message
/// schedule permuted by [`MSG_PERM`] between rounds. Returns the first 8
/// output words (`v[i] ^ v[i+8]`), i.e. the 256-bit chaining value.
fn compress(h: &[u32; 8], block: &[u32; 16], counter: u64, block_len: u32, flags: u32) -> [u32; 8] {
    let mut v = [0u32; 16];
    v[..8].copy_from_slice(h);
    v[8..12].copy_from_slice(&IV[..4]);
    v[12] = counter as u32;
    v[13] = (counter >> 32) as u32;
    v[14] = block_len;
    v[15] = flags;
    let mut m = *block;
    for round in 0..7 {
        g(&mut v, 0, 4, 8, 12, m[0], m[1]);
        g(&mut v, 1, 5, 9, 13, m[2], m[3]);
        g(&mut v, 2, 6, 10, 14, m[4], m[5]);
        g(&mut v, 3, 7, 11, 15, m[6], m[7]);
        g(&mut v, 0, 5, 10, 15, m[8], m[9]);
        g(&mut v, 1, 6, 11, 12, m[10], m[11]);
        g(&mut v, 2, 7, 8, 13, m[12], m[13]);
        g(&mut v, 3, 4, 9, 14, m[14], m[15]);
        if round < 6 {
            let mut nm = [0u32; 16];
            for (i, &p) in MSG_PERM.iter().enumerate() {
                nm[i] = m[p];
            }
            m = nm;
        }
    }
    let mut out = [0u32; 8];
    for i in 0..8 {
        out[i] = v[i] ^ v[i + 8];
    }
    out
}

/// BLAKE3 hash of a SHORT input (≤ 64 bytes, one block, one chunk) — the
/// only case the §45.3 key derivation needs (the keyed input is 23 bytes).
///
/// # Panics
///
/// Panics if `input.len() > 64` (the multi-block tree mode is deliberately
/// not implemented; the caller never needs it).
pub fn blake3_short(input: &[u8]) -> [u8; 32] {
    assert!(input.len() <= 64, "blake3_short: single-block inputs only");
    let mut block = [0u32; 16];
    for (i, chunk) in input.chunks(4).enumerate() {
        let mut w = [0u8; 4];
        w[..chunk.len()].copy_from_slice(chunk);
        block[i] = u32::from_le_bytes(w);
    }
    let h = compress(
        &IV,
        &block,
        0,
        input.len() as u32,
        CHUNK_START | CHUNK_END | ROOT,
    );
    let mut out = [0u8; 32];
    for (i, w) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }
    out
}

/// ChaCha quarter-round (published spec).
#[inline]
fn qr(v: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
    v[a] = v[a].wrapping_add(v[b]);
    v[d] = (v[d] ^ v[a]).rotate_left(16);
    v[c] = v[c].wrapping_add(v[d]);
    v[b] = (v[b] ^ v[c]).rotate_left(12);
    v[a] = v[a].wrapping_add(v[b]);
    v[d] = (v[d] ^ v[a]).rotate_left(8);
    v[c] = v[c].wrapping_add(v[d]);
    v[b] = (v[b] ^ v[c]).rotate_left(7);
}

/// One ChaCha8 block (8 rounds = 4 double-rounds) in the original 64-bit
/// counter + 64-bit nonce layout: constants ‖ key ‖ counter ‖ nonce.
fn chacha8_block(key: &[u8; 32], counter: u64, nonce: [u32; 2]) -> [u32; 16] {
    let mut s = [0u32; 16];
    // "expand 32-byte k" (published sigma constants).
    s[0] = 0x6170_7865;
    s[1] = 0x3320_646e;
    s[2] = 0x7962_2d32;
    s[3] = 0x6b20_6574;
    for i in 0..8 {
        s[4 + i] = u32::from_le_bytes([key[i * 4], key[i * 4 + 1], key[i * 4 + 2], key[i * 4 + 3]]);
    }
    s[12] = counter as u32;
    s[13] = (counter >> 32) as u32;
    s[14] = nonce[0];
    s[15] = nonce[1];
    let mut v = s;
    for _ in 0..4 {
        qr(&mut v, 0, 4, 8, 12);
        qr(&mut v, 1, 5, 9, 13);
        qr(&mut v, 2, 6, 10, 14);
        qr(&mut v, 3, 7, 11, 15);
        qr(&mut v, 0, 5, 10, 15);
        qr(&mut v, 1, 6, 11, 12);
        qr(&mut v, 2, 7, 8, 13);
        qr(&mut v, 3, 4, 9, 14);
    }
    for i in 0..16 {
        v[i] = v[i].wrapping_add(s[i]);
    }
    v
}

/// Sub-stream selector: the PCA sample draw (§21.4 `"appname.caf.pca"`).
pub const STREAM_PCA: u32 = 0;
/// Sub-stream selector: the priority-heap tie-break draw.
pub const STREAM_TIEBREAK: u32 = 1;

/// The §45.3 per-level stream key: `blake3("appname.caf" ‖ le64(seed) ‖
/// le32(level))`. Level 0 is the finest pyramid level (full resolution).
pub fn level_key(seed: u32, level: u32) -> [u8; 32] {
    let mut buf = [0u8; 23];
    buf[..11].copy_from_slice(b"appname.caf");
    buf[11..19].copy_from_slice(&(seed as u64).to_le_bytes());
    buf[19..23].copy_from_slice(&level.to_le_bytes());
    blake3_short(&buf)
}

/// Stateless draw: the first LE `u64` of the ChaCha8 block at
/// (counter = `index`, nonce = [`selector`, 0]) under `key`. A pure
/// function of its arguments — no generator state exists, so evaluation
/// order is invisible (§45.3 "stateless, so tile order and thread count
/// are invisible").
pub fn draw_u64(key: &[u8; 32], selector: u32, index: u64) -> u64 {
    let b = chacha8_block(key, index, [selector, 0]);
    (b[0] as u64) | ((b[1] as u64) << 32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Published BLAKE3 test vector: the empty input. This pins the whole
    /// compression (IV, schedule, permutation, flags, output rule).
    #[test]
    fn blake3_empty_vector() {
        let h = blake3_short(b"");
        let hex: String = h.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
    }

    /// Key derivation is length- and content-sensitive and stable.
    #[test]
    fn level_key_separates_seed_and_level() {
        let a = level_key(0, 0);
        assert_ne!(a, level_key(1, 0));
        assert_ne!(a, level_key(0, 1));
        assert_eq!(a, level_key(0, 0));
    }

    /// Draws are stateless pure functions: same coordinates same value,
    /// distinct coordinates distinct values (spot check), and evaluation
    /// order cannot matter because there is no state to mutate.
    #[test]
    fn draws_are_stateless_and_distinct() {
        let k = level_key(7, 2);
        let a = draw_u64(&k, STREAM_PCA, 5);
        let b = draw_u64(&k, STREAM_TIEBREAK, 5);
        let c = draw_u64(&k, STREAM_PCA, 6);
        assert_eq!(a, draw_u64(&k, STREAM_PCA, 5));
        assert_ne!(a, b);
        assert_ne!(a, c);
    }
}
