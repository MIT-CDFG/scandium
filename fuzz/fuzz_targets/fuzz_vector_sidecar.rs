// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Fuzz target: the `appname.vector.v1` sidecar surface (ES §7.5 worked
//! example, §20.1 validate/normalize; TEXTVEC Door 1).
//!
//! UPGRADED (the recorded one-line, O-QA re-pin round): the Door-1
//! `VectorGeometry` registration MERGED (TEXTVEC wiring, main e8f5f9b) —
//! a registered `validate_sidecar` now sits on both production
//! assemblies' package doors. Three surfaces per input:
//!
//! 1. `appname_vector::path_from_sidecar_json` — the merged production
//!    wire parser (lenient reader: schema stamp, re-weld ≤ 1e-6,
//!    degenerate removal, non-finite rejection) that the service impl
//!    wraps. On accepted inputs the target also asserts the
//!    lenient-reader → strict-writer round trip: a normalized non-empty
//!    path MUST re-emit (`path_to_sidecar_json`) and the re-emission MUST
//!    re-parse (wire fixpoint). A violation is a real invariant break and
//!    is WANTED as a crash finding. (Paths normalized to zero subpaths
//!    are the known lenient/strict asymmetry — skipped, not asserted.)
//! 2. `appname_core::load_package_bytes_with` over an assembly carrying
//!    the REAL service (`appname_vector::register`) — the production
//!    posture: load now runs the registered `validate_sidecar` on the
//!    fuzzed entry (the named upgrade's surface).
//! 3. `appname_core::load_package_bytes` — the bare-mirror door KEPT
//!    (§7.7 lenient-reader posture: opaque byte carry, persisted
//!    bounds, content hash recomputed): it remains a public API and the
//!    service-less carry path is distinct code. The rest of the package
//!    is a fixed valid template so coverage concentrates on the sidecar
//!    seam, not on re-fuzzing the zip container (fuzz_codec_import
//!    already carries a whole-package seed).
//!
//! Bar: every input parses Ok or returns a typed error — never a panic,
//! never an abort; round-trip asserts as above.

#![no_main]

use std::io::{Cursor, Read, Write};
use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;

use appname_core::{EngineAssembly, PathId};

/// Fixed path id: the package entry name carries the id (§7.5 — the
/// sidecar body carries none), so a constant keeps the template static.
const PID: &str = "0f7a5e11-2bce-4d10-9b6f-3a4d5e6f7081";

/// Ordered entries (name, bytes) of a minimal valid `.appname` package
/// whose document.json carries one `paths` inventory row pointing at
/// `vector/{PID}.json` — everything except that sidecar entry itself.
static TEMPLATE: OnceLock<Vec<(String, Vec<u8>)>> = OnceLock::new();

fn template() -> &'static [(String, Vec<u8>)] {
    TEMPLATE.get_or_init(|| {
        // A real engine-saved package (bare core assembly, doc.new only)
        // so the template tracks the writer, not a hand-copied shape.
        let mut engine = appname_core::engine::kinds::register(EngineAssembly::new()).build();
        engine
            .apply_json(
                &serde_json::json!({
                    "schema": "appname.command.v1",
                    "id": "0f7a5e11-2bce-4d10-9b6f-3a4d5e6f7080",
                    "kind": "doc.new",
                    "params": {"width": 16, "height": 16, "background": "white"},
                })
                .to_string(),
            )
            .expect("template doc.new applies");
        let pkg = appname_core::save_package_bytes(&engine).expect("template package saves");
        let mut za = zip::ZipArchive::new(Cursor::new(pkg)).expect("template package reopens");
        let mut entries = Vec::new();
        for i in 0..za.len() {
            let mut f = za.by_index(i).expect("template entry");
            let name = f.name().to_string();
            let mut buf = Vec::new();
            f.read_to_end(&mut buf).expect("template entry bytes");
            if name == "document.json" {
                // Splice the §20 root `paths` inventory row (the flagged
                // SCHEMA-GAP wire form package.rs reads).
                let mut v: serde_json::Value =
                    serde_json::from_slice(&buf).expect("template document.json parses");
                v["paths"] = serde_json::json!([{
                    "id": PID,
                    "name": "fuzz-seam",
                    "bounds": {"x": 0, "y": 0, "w": 4, "h": 4},
                }]);
                buf = serde_json::to_vec(&v).expect("spliced document.json serializes");
            }
            entries.push((name, buf));
        }
        entries
    })
}

/// Rebuild the template package with `sidecar` as `vector/{PID}.json`
/// (Stored throughout, mimetype-first order preserved — §7.1).
fn package_with_sidecar(sidecar: &[u8]) -> Vec<u8> {
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut zw = zip::ZipWriter::new(&mut cursor);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, bytes) in template() {
            zw.start_file(name.clone(), opts).expect("zip entry");
            zw.write_all(bytes).expect("zip entry bytes");
        }
        zw.start_file(format!("vector/{PID}.json"), opts)
            .expect("sidecar entry");
        zw.write_all(sidecar).expect("sidecar bytes");
        zw.finish().expect("zip finish");
    }
    cursor.into_inner()
}

/// The service-registered assembly for surface 2 (built once; the
/// register chain is construction-only and reused across execs).
static SERVICE_ASM: OnceLock<EngineAssembly> = OnceLock::new();

fn service_asm() -> &'static EngineAssembly {
    SERVICE_ASM.get_or_init(|| {
        appname_vector::register(appname_core::engine::kinds::register(EngineAssembly::new()))
    })
}

fuzz_target!(|data: &[u8]| {
    // Surface 1: the production wire parser (+ round-trip invariants).
    if let Ok((path, _notes)) = appname_vector::path_from_sidecar_json(PathId::nil(), data) {
        if !path.subpaths.is_empty() {
            let re = appname_vector::path_to_sidecar_json(&path)
                .expect("lenient-reader output must satisfy the strict writer (§20.1)");
            let _ = appname_vector::path_from_sidecar_json(PathId::nil(), &re)
                .expect("re-emitted sidecar must re-parse (wire fixpoint)");
        }
    }
    let pkg = package_with_sidecar(data);
    // Surface 2 (the named upgrade): the package door with the REAL
    // VectorGeometry registered — load runs validate_sidecar on the
    // fuzzed entry, the production posture since the Door-1 merge.
    let _ = appname_core::load_package_bytes_with(service_asm(), &pkg);
    // Surface 3: the bare-mirror door, kept (§7.7 opaque carry).
    let _ = appname_core::load_package_bytes(&pkg);
});
