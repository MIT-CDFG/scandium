// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Fuzz target: `raw_probe` (§34.2) — the cargo-fuzz lift of the
//! in-crate deterministic floor `appname-io/tests/raw_fuzz_floor.rs`
//! (O-IO handoff at the RAW-binding slice; psd_fuzz_floor →
//! fuzz_psd_import precedent). Bar, identical to the floor's: every
//! input through `RawCodec::decode` — with the module ABSENT and with
//! the conformance-stub module PRESENT — ends in `Ok` or a §8.1-mapped
//! typed error. Never a panic, never an abort. Both module states run
//! on every input so mutations exercise the sniff/refusal arms (absent)
//! AND the post-open stub-spec arms (present).
//!
//! Seeds: `corpus/fuzz_raw_probe/` emitted by `seed_gen` from the
//! floor's pinned corpus builders (§43.7 vendor-magic heads + stub-spec
//! bytes from the production `libraw_sys::stub` emitters).

#![no_main]

use appname_io::raw::codec::RawCodec;
use appname_io::registry::Codec;
use appname_io::DecodeOpts;
use libfuzzer_sys::fuzz_target;
use libraw_sys::stub::stub_lib;

fuzz_target!(|data: &[u8]| {
    let absent = RawCodec::default();
    let _ = absent.decode(data, &DecodeOpts::default());
    let stubbed = RawCodec::with_module(stub_lib());
    let _ = stubbed.decode(data, &DecodeOpts::default());
});
