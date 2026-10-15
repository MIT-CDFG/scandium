// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Deliberately a no-op: this crate emits NO `cargo:rustc-link-lib`
//! directive, ever.
//!
//! The record posture (`deps/libraw.yaml`, linking + bindingCensus):
//! LibRaw is an OPTIONAL, REPLACEABLE dynamic module (LGPL-2.1-elected,
//! §48.2 rule 1) — a build-time dynamic link would make the whole binary
//! fail to load whenever the dylib is absent, contradicting the §48.4
//! graceful fallback ("feature off / library absent ⇒ RAW disabled,
//! clean typed error"). The module is therefore resolved at RUNTIME via
//! the platform dynamic loader (src/lib.rs), from a path the caller
//! supplies — the packaging channel decides the shipped dylib's source
//! and location (the record's packagingChannel gap, dmg wave; the
//! §1.4-rule-2 vs Part-43-rule-1 tension is O-INT's register, not
//! resolved here).
//!
//! The build script exists because `links = "raw"` (the ES §1.4 / ES:934
//! native-library identity claim) requires one.

fn main() {}
