// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! `raw_probe` FLOOR — the deterministic in-crate tier of the §34.2
//! `raw_probe` fuzz obligation at the LibRaw wrapper boundary (the
//! psd_fuzz_floor precedent: pinned-seed mutation corpus per-PR;
//! the cargo-fuzz target lift into `fuzz/` is O-QA's fence and is
//! flagged in the lane log).
//!
//! Bar: every mutated input through `RawCodec::decode` — with the
//! module ABSENT and with the conformance-stub module PRESENT — ends in
//! `Ok` or a §8.1-mapped typed error. Never a panic. Seeds follow the
//! §43.7 `truncated_*.bin` shape (vendor-magic heads) plus stub-spec
//! bytes so the mutations also reach the post-open arms.

#![cfg(feature = "raw")]

use appname_io::raw::codec::RawCodec;
use appname_io::registry::Codec;
use appname_io::DecodeOpts;
use libraw_sys::stub::{stub_bytes, stub_lib, with_prefix, StubSpec};

/// xorshift64* — the house deterministic mutation PRNG (pinned seeds).
struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

fn seeds() -> Vec<Vec<u8>> {
    let cr2 = {
        let mut v = vec![0x49, 0x49, 0x2A, 0x00, 0x10, 0x00, 0x00, 0x00, 0x43, 0x52];
        v.resize(24, 0x11);
        v
    };
    let raf = b"FUJIFILMCCD-RAW 0201FF129502".to_vec();
    let cr3 = {
        let mut v = vec![0, 0, 0, 0x18];
        v.extend_from_slice(b"ftypcrx ");
        v.resize(32, 0);
        v
    };
    let stub_ok = with_prefix(&cr2, &stub_bytes(&StubSpec::bayer_rggb(16, 12)));
    let stub_degenerate = with_prefix(
        &cr2,
        &stub_bytes(&StubSpec {
            color_maximum: 0,
            ..StubSpec::bayer_rggb(8, 8)
        }),
    );
    vec![cr2, raf, cr3, stub_ok, stub_degenerate]
}

fn assert_typed_or_ok(codec: &RawCodec, bytes: &[u8], what: &str) {
    match codec.decode(bytes, &DecodeOpts::default()) {
        Ok(_) => {}
        Err(e) => {
            // Typed = it maps to a registered §8.1 MessageId (the
            // accessor is total; reaching it without panicking is the
            // floor's whole assertion).
            let id = e.message_id();
            assert!(id.starts_with("error."), "{what}: odd id {id}");
        }
    }
}

/// ~1,900 deterministic mutations per module state: byte flips,
/// truncations, and aligned 4-byte scribbles over every seed.
#[test]
fn raw_probe_floor_never_panics() {
    let absent = RawCodec::default();
    let stubbed = RawCodec::with_module(stub_lib());
    let mut rng = XorShift(0x00DD_BA11_5EED_C1DE);
    for (si, seed) in seeds().iter().enumerate() {
        // Byte flips.
        for k in 0..120 {
            let mut m = seed.clone();
            let pos = (rng.next() as usize) % m.len();
            m[pos] ^= (rng.next() as u8) | 1;
            assert_typed_or_ok(&absent, &m, &format!("flip s{si}k{k} absent"));
            assert_typed_or_ok(&stubbed, &m, &format!("flip s{si}k{k} stub"));
        }
        // Truncations (§43.7 truncated_* shape).
        for k in 0..seed.len().min(48) {
            let m = &seed[..k];
            assert_typed_or_ok(&absent, m, &format!("trunc s{si}k{k} absent"));
            assert_typed_or_ok(&stubbed, m, &format!("trunc s{si}k{k} stub"));
        }
        // Aligned 4-byte scribbles (length/dimension fields).
        for k in 0..60 {
            let mut m = seed.clone();
            if m.len() >= 4 {
                let pos = ((rng.next() as usize) % (m.len() / 4)) * 4;
                let word = (rng.next() as u32).to_le_bytes();
                m[pos..pos + 4].copy_from_slice(&word);
            }
            assert_typed_or_ok(&absent, &m, &format!("scribble s{si}k{k} absent"));
            assert_typed_or_ok(&stubbed, &m, &format!("scribble s{si}k{k} stub"));
        }
    }
    // Empty + oversized-junk edge cases.
    assert_typed_or_ok(&stubbed, &[], "empty");
    assert_typed_or_ok(&stubbed, &vec![0xAB; 1 << 16], "junk64k");
}
