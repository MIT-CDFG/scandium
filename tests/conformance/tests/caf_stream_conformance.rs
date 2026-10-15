// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// O-QA CAF seeded-stream conformance (ES §45.3 micro-layout, ratified
// 2026-08-16, see reports/O-QA-log.md ratification entry). Staged pre-merge;
// ACTIVATED at the Row A merge (main 27d0bee) against the live
// `appname_algo::exemplar` engine.
//
// The pinned values are `fixtures/caf_stream/vectors.json`, derived TWICE
// independently before pinning (O-QA rule): (1) the independent Python
// reference `fixtures/caf_stream/derive_reference.py` (ChaCha core
// anchored to the RFC 7539/8439 §2.3.2 published 20-round block vector,
// BLAKE3 core anchored to the published empty-input vector), and (2) the
// lane/algo implementation at commit 094ba40 — all 12 values byte-equal.
// Any future drift here is an investigation, never a re-pin.

use appname_algo::exemplar::stream::{draw_u64, level_key};

/// Raw ChaCha8 draw vectors under the classical 000102..1f test key:
/// pins the 8-round core, the DJB 64/64 layout (words 12..13 = le64
/// counter, words 14..15 = nonce = [selector, 0]), and the
/// first-LE-u64 draw extraction. The 2^32+5 index pins the counter's
/// high-word split; selector 5 pins unassigned-selector behavior.
#[test]
fn caf_stream_raw_chacha8_vectors() {
    let mut key = [0u8; 32];
    for (i, b) in key.iter_mut().enumerate() {
        *b = i as u8;
    }
    const VECTORS: [(u32, u64, u64); 6] = [
        (0, 0, 7686257455104398656),
        (0, 1, 6465113837412424310),
        (1, 0, 13623057893742724943),
        (1, 1, 2109579612481664528),
        (0, (1 << 32) + 5, 13119105391643301176),
        (5, 7, 3639555453512898399),
    ];
    for (sel, idx, expect) in VECTORS {
        assert_eq!(
            draw_u64(&key, sel, idx),
            expect,
            "raw ChaCha8 vector drifted at selector {sel} index {idx}"
        );
    }
}

/// End-to-end vectors through the §45.3 key derivation
/// blake3("appname.caf" || le64(seed) || le32(level)): pins the label,
/// the LE field encodings, the single-chunk BLAKE3 subset, and the
/// key-to-stream hand-off.
#[test]
fn caf_stream_end_to_end_vectors() {
    const KEY00: &str = "df85aa632ea816b14b6d4468c19a577eb025402d696bda942714f49a3cdb3d45";
    const KEY71: &str = "837d8131159db85594cdc3e77fa597113dc0342f2cc51fe5d4eb7de47492a027";
    let hex = |k: [u8; 32]| k.iter().map(|b| format!("{b:02x}")).collect::<String>();

    let k00 = level_key(0, 0);
    assert_eq!(hex(k00), KEY00, "level_key(0,0) drifted");
    assert_eq!(draw_u64(&k00, 0, 0), 5970617666140389662);
    assert_eq!(draw_u64(&k00, 1, 0), 2528273638894136006);

    let k71 = level_key(7, 1);
    assert_eq!(hex(k71), KEY71, "level_key(7,1) drifted");
    assert_eq!(draw_u64(&k71, 0, 3), 6882283125405427572);
    assert_eq!(draw_u64(&k71, 1, 3), 5353841464653045594);
}
