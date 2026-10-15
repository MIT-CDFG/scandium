# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# O-QA independent reference derivation for the §45.3 CAF seeded-stream
# conformance vectors. Written from the PUBLIC specifications only:
# ChaCha core per RFC 7539/8439's structure (round count changed to 8,
# state words 12..15 laid out as DJB's original 64-bit counter + 64-bit
# nonce), BLAKE3 single-chunk short-input subset per the published spec.
# INDEPENDENT of the lane/algo Rust implementation: no Rust code was
# consulted while writing this file's core functions; both cores are
# anchored below against PUBLISHED vectors before any derivation runs.

import json
import struct
import sys

MASK = 0xFFFFFFFF


# ---------------------------------------------------------------- ChaCha
def rotl(x, n):
    return ((x << n) | (x >> (32 - n))) & MASK


def qr(s, a, b, c, d):
    s[a] = (s[a] + s[b]) & MASK
    s[d] = rotl(s[d] ^ s[a], 16)
    s[c] = (s[c] + s[d]) & MASK
    s[b] = rotl(s[b] ^ s[c], 12)
    s[a] = (s[a] + s[b]) & MASK
    s[d] = rotl(s[d] ^ s[a], 8)
    s[c] = (s[c] + s[d]) & MASK
    s[b] = rotl(s[b] ^ s[c], 7)


def chacha_block(key, words12_15, rounds):
    """Generic ChaCha block: constants | key | caller-supplied words 12..15.
    The counter/nonce LAYOUT is entirely the caller's choice of words
    12..15, which is exactly the degree of freedom under ratification."""
    assert len(key) == 32 and rounds % 2 == 0
    s = [0x61707865, 0x3320646E, 0x79622D32, 0x6B206574]
    s += list(struct.unpack("<8I", key))
    s += list(words12_15)
    v = s[:]
    for _ in range(rounds // 2):
        qr(v, 0, 4, 8, 12)
        qr(v, 1, 5, 9, 13)
        qr(v, 2, 6, 10, 14)
        qr(v, 3, 7, 11, 15)
        qr(v, 0, 5, 10, 15)
        qr(v, 1, 6, 11, 12)
        qr(v, 2, 7, 8, 13)
        qr(v, 3, 4, 9, 14)
    return [(v[i] + s[i]) & MASK for i in range(16)]


def chacha8_djb_block(key, counter, nonce2):
    """The layout under ratification: 8 rounds, words 12..13 = le64
    counter, words 14..15 = the two u32 nonce words [selector, 0]."""
    w = [counter & MASK, (counter >> 32) & MASK, nonce2[0], nonce2[1]]
    return chacha_block(key, w, 8)


def draw_u64(key, selector, index):
    """First little-endian u64 of the block at (counter=index,
    nonce=[selector, 0]) — the §45.3 draw as ratified."""
    b = chacha8_djb_block(key, index, (selector, 0))
    return b[0] | (b[1] << 32)


# ---------------------------------------------------------------- BLAKE3
IV = [
    0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A,
    0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19,
]
MSG_PERM = [2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8]
CHUNK_START, CHUNK_END, ROOT = 1, 2, 8


def b3_g(v, a, b, c, d, x, y):
    v[a] = (v[a] + v[b] + x) & MASK
    v[d] = rotl(v[d] ^ v[a], 32 - 16)
    v[c] = (v[c] + v[d]) & MASK
    v[b] = rotl(v[b] ^ v[c], 32 - 12)
    v[a] = (v[a] + v[b] + y) & MASK
    v[d] = rotl(v[d] ^ v[a], 32 - 8)
    v[c] = (v[c] + v[d]) & MASK
    v[b] = rotl(v[b] ^ v[c], 32 - 7)


def b3_compress(h, block_words, counter, block_len, flags):
    v = h[:8] + IV[:4] + [
        counter & MASK, (counter >> 32) & MASK, block_len, flags,
    ]
    m = block_words[:]
    for rnd in range(7):
        b3_g(v, 0, 4, 8, 12, m[0], m[1])
        b3_g(v, 1, 5, 9, 13, m[2], m[3])
        b3_g(v, 2, 6, 10, 14, m[4], m[5])
        b3_g(v, 3, 7, 11, 15, m[6], m[7])
        b3_g(v, 0, 5, 10, 15, m[8], m[9])
        b3_g(v, 1, 6, 11, 12, m[10], m[11])
        b3_g(v, 2, 7, 8, 13, m[12], m[13])
        b3_g(v, 3, 4, 9, 14, m[14], m[15])
        if rnd < 6:
            m = [m[p] for p in MSG_PERM]
    return [v[i] ^ v[i + 8] for i in range(8)]


def blake3_short(data):
    """BLAKE3 of a <=64-byte input: one block, one chunk, root output."""
    assert len(data) <= 64
    padded = data + b"\x00" * (64 - len(data))
    words = list(struct.unpack("<16I", padded))
    out = b3_compress(IV, words, 0, len(data), CHUNK_START | CHUNK_END | ROOT)
    return struct.pack("<8I", *out)


def level_key(seed, level):
    """§45.3: blake3("appname.caf" || le64(seed) || le32(level))."""
    return blake3_short(
        b"appname.caf" + struct.pack("<Q", seed) + struct.pack("<I", level)
    )


# ------------------------------------------------- published-vector anchors
def anchor_chacha_rfc7539():
    """RFC 7539/8439 sec 2.3.2 block test vector: 20 rounds, IETF layout
    (word 12 = 32-bit counter, words 13..15 = 96-bit nonce). Anchors the
    sigma constants, quarter-round, column/diagonal order, key/word
    serialization — everything except the round count and the 12..15
    layout, which are exactly the two knobs under ratification."""
    key = bytes(range(32))
    nonce = bytes.fromhex("000000090000004a00000000")
    n_words = struct.unpack("<3I", nonce)
    block = chacha_block(key, [1, *n_words], 20)
    keystream = struct.pack("<16I", *block)
    expected = bytes.fromhex(
        "10f1e7e4d13b5915500fdd1fa32071c4"
        "c7d1f4c733c068030422aa9ac3d46c4e"
        "d2826446079faa0914c2d705d98b02a2"
        "b5129cd1de164eb9cbd083e8a2503c4e"
    )
    assert keystream == expected, "RFC 7539 ChaCha20 block anchor FAILED"


def anchor_blake3_empty():
    """Published BLAKE3 empty-input vector (official test_vectors.json)."""
    expected = "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
    got = blake3_short(b"").hex()
    assert got == expected, f"BLAKE3 empty anchor FAILED: {got}"


# ------------------------------------------------------------- derivation
def main():
    anchor_chacha_rfc7539()
    anchor_blake3_empty()

    fixed_key = bytes(range(32))  # 000102...1f, the classical test key
    raw_points = [
        # (selector, index) — 0/1 are the ratified sub-streams; the large
        # index exercises the high counter word (le64 split); selector 5
        # exercises a not-yet-assigned selector slot.
        (0, 0), (0, 1), (1, 0), (1, 1), (0, (1 << 32) + 5), (5, 7),
    ]
    raw = [
        {
            "selector": sel,
            "index": idx,
            "draw_u64": str(draw_u64(fixed_key, sel, idx)),
        }
        for sel, idx in raw_points
    ]

    lk00 = level_key(0, 0)
    lk71 = level_key(7, 1)
    e2e = [
        {"seed": 0, "level": 0, "level_key_hex": lk00.hex(),
         "draws": [{"selector": s, "index": 0,
                    "draw_u64": str(draw_u64(lk00, s, 0))} for s in (0, 1)]},
        {"seed": 7, "level": 1, "level_key_hex": lk71.hex(),
         "draws": [{"selector": s, "index": 3,
                    "draw_u64": str(draw_u64(lk71, s, 3))} for s in (0, 1)]},
    ]

    out = {
        "contract": "ES §45.3 seeded-stream micro-layout (O-QA ratified)",
        "layout": {
            "key": 'blake3("appname.caf" || le64(seed) || le32(level))  (single-chunk BLAKE3, 32-byte key)',
            "block": "ChaCha8 (8 rounds), DJB layout: words 12..13 = le64 block counter, words 14..15 = nonce = [selector, 0]",
            "draw": "first little-endian u64 of the block (out words 0,1): w0 | (w1 << 32)",
            "selectors": {"0": "appname.caf.pca (PCA sample draw, counter = draw index k)",
                          "1": "tie-break (counter = pixel linear index y*w+x at the level)"},
        },
        "raw_chacha8_vectors_key_000102..1f": raw,
        "end_to_end_vectors": e2e,
        "anchors": [
            "ChaCha core: RFC 7539/8439 sec 2.3.2 20-round block vector (PASS at derivation time)",
            "BLAKE3 core: published empty-input vector af1349b9... (PASS at derivation time)",
        ],
        "derivation": "independent Python reference (this script); cross-checked against lane/algo appname_algo::exemplar::stream output — see reports/O-QA-log.md ratification entry",
    }
    json.dump(out, sys.stdout, indent=2)
    print()


if __name__ == "__main__":
    main()
