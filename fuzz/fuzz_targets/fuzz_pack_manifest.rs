// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Fuzz target: the pack-manifest JSON path — `appname-ml`'s
//! `PackRegistry::parse` (serde shape with `deny_unknown_fields`
//! mirroring every schema `additionalProperties: false`, then the
//! discipline validation pass: id grammar, duplicate ids, revocable
//! refusal, hash-pin hex checks, lawful-fallback requirement).
//!
//! Spec: `contracts/schemas/ai-model-pack.v1.json` (appname.aiModelPack.v1,
//! ES Part 53 §53.9); D-45/D-49. The strict-unknown-fields posture is
//! exactly the fuzz-worthy edge: every rejection must be a typed
//! `ManifestError`, never a panic — including the panic-prone classes
//! (deep nesting, huge numbers, non-UTF-8 mutations, duplicate keys).
//!
//! On accepted registries the target also walks the post-parse read
//! surfaces (`license_display`, `pack`) so accessor code shares the soak.
//!
//! Bar: `parse` returns Ok or typed `ManifestError` — never a panic,
//! never an abort.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Lossy widening keeps non-UTF-8 mutations in play (they become
    // replacement chars and still exercise the parser).
    let text = String::from_utf8_lossy(data);
    if let Ok(reg) = appname_ml::manifest::PackRegistry::parse(&text) {
        for p in &reg.packs {
            let _ = p.license_display();
            let _ = reg.pack(&p.id);
        }
    }
});
