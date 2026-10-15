// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Fuzz target: the whole codec matrix through the sniff-dispatch door
//! (registry resolve on magic bytes, then that codec's decode) — the
//! path every import takes (PNG/JPEG/TIFF/WebP/GIF/BMP/ORA/PSD).
//! Same bar as fuzz_psd_import: Ok or typed CodecError, never a panic.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let reg = appname_io::CodecRegistry::builtin();
    if let Ok(codec) = reg.resolve(data, None) {
        let _ = codec.decode(data, &appname_io::DecodeOpts::default());
    }
});
