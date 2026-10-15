// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Fuzz target: the PSD parser (ES §8.5 continuous-fuzz mandate).
//! Bar (the fuzz-floor D-series discipline, promoted): every input
//! either imports Ok or returns a TYPED CodecError — never a panic,
//! never an abort, and the §8.1 bomb gates hold (decode allocations
//! bounded by the header caps the codec enforces).

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let reg = appname_io::CodecRegistry::builtin();
    if let Some(codec) = reg.by_id("psd") {
        let _ = codec.decode(data, &appname_io::DecodeOpts::default());
    }
});
