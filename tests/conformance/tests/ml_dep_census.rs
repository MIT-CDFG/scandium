// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! appname-ml dependency-graph census — the ML slice-2 reviewer advisory,
//! adopted into the O-QA CP-1.0 accumulated ratification round (O-INT
//! dispatch, 2026-08-17): ENFORCE, don't narrate, the ratified dev-dep
//! ruling. The shipped appname-ml LIBRARY depends on serde/serde_json
//! ONLY (deps/serde.yaml, usedBy extension flagged in the O-ML log); the
//! cross-crate D-ML-1 pipeline proof lives in [dev-dependencies]
//! (appname-core, appname-assembly, uuid — test binaries, shipped
//! nowhere). D-48's structural claim — replay can never reach a model —
//! rests on the NORMAL edge set staying exactly this small, so a silent
//! promotion of any dev-dep (or any new normal edge, e.g. the `ort`
//! record's consumption slice landing without its own companion here)
//! must turn this census red.
//!
//! Mechanism: `cargo metadata --no-deps` over the workspace manifest —
//! declared dependency records with their `kind` (normal = null in the
//! cargo-metadata wire form), so the pin covers every feature
//! combination and needs no resolution of the full graph.
//!
//! Spec: D-46/D-48/D-49; plan §3.13 (O-ML lane rules); the dev-dep
//! posture entry in reports/O-ML-log.md (slice 2) and its ratification
//! in reports/INTEGRATION-log.md (ML slice 2 merge).

use appname_conformance::json::{parse, Value};
use appname_conformance::paths;
use std::process::Command;

#[test]
fn appname_ml_normal_dependency_set_is_serde_only() {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let out = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .arg("--manifest-path")
        .arg(paths::repo_root().join("Cargo.toml"))
        .output()
        .expect("spawn cargo metadata");
    assert!(
        out.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).expect("cargo metadata utf-8");
    let meta = parse(stdout.trim()).expect("cargo metadata JSON");

    let packages = meta
        .get("packages")
        .and_then(Value::as_arr)
        .expect("packages array");
    let ml = packages
        .iter()
        .find(|p| p.get("name").and_then(Value::as_str) == Some("appname-ml"))
        .expect("appname-ml is a workspace member");
    let deps = ml
        .get("dependencies")
        .and_then(Value::as_arr)
        .expect("dependencies array");

    // kind: null = normal, "dev" = dev-dependencies, "build" = build
    // scripts (cargo-metadata wire form). Only the NORMAL set ships.
    let mut normal: Vec<&str> = deps
        .iter()
        .filter(|d| d.get("kind").is_none_or(Value::is_null))
        .map(|d| d.get("name").and_then(Value::as_str).expect("dep name"))
        .collect();
    normal.sort_unstable();
    assert_eq!(
        normal,
        ["serde", "serde_json"],
        "appname-ml's shipped (normal) dependency set moved. The ratified \
         slice-2 posture is serde/serde_json ONLY (deps/serde.yaml): the \
         D-48 structural proof that replay cannot reach a model rests on \
         this edge set. A dev-dep promoted to [dependencies], or a new \
         normal edge (e.g. the ort record's consumption slice), needs its \
         own recorded ruling + a companion edit here — not a silent land."
    );

    // The ratified ruling's other half, pinned from the same record: the
    // pipeline-proof crates stay DEV-scoped (test binaries only). If one
    // vanishes entirely that is fine (test moved/retired); present in any
    // non-dev scope is the alarm above; present in dev scope is the
    // recorded posture.
    for name in ["appname-core", "appname-assembly", "uuid"] {
        for d in deps
            .iter()
            .filter(|d| d.get("name").and_then(Value::as_str) == Some(name))
        {
            assert_eq!(
                d.get("kind").and_then(Value::as_str),
                Some("dev"),
                "{name}: appname-ml may carry this crate in dev scope only \
                 (ML slice-2 ratified ruling)"
            );
        }
    }
}
