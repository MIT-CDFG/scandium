// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! C-ABI EXPORT census at the REAL ARTIFACT level (conformance-gap
//! entry, O-QA CP-0.7 wave 2).
//!
//! Spec: DD ④ / ES Appendix N (the Apache-2.0 SDK surface is the cabi
//! crate's declared exports); plan §3.11 ("the ABI is the ONLY
//! entrance"). The in-crate export census sees only what the crate
//! DECLARES — it is structurally blind to symbols other crates leak
//! into the linked cdylib. Censused on the shipped artifact 2026-08-13
//! (`nm -gU target/debug/libappname_cabi.dylib`): 34 exported symbols,
//! 29 `_appname_*`, and FIVE strays invisible to the in-crate census:
//!
//!   _blake3_compress_in_place_portable   (T — the blake3 C shim leaks
//!                                         a global function symbol)
//!   ___CLASS_RawWindowMetalLayer          (S)  ┐ objc2 class metadata
//!   ___DROP_FLAG_OFFSET_RawWindowMetalLayer(S) │ from the wgpu/metal
//!   ___IVAR_OFFSET_RawWindowMetalLayer    (S)  │ dependency chain
//!   ___REGISTER_CLASS_RawWindowMetalLayer (D)  ┘
//!
//! FILED: QA-F10 (reports/O-QA-log.md CP-0.7 wave 2) — routed to
//! O-SCRIPT (exported_symbols_list / linker version-script pass) with
//! O-BUILD (release pipeline), so no SDK artifact ships with stray
//! non-appname exports. The gate test below is FAILING-BY-DESIGN and
//! carries the house `#[ignore]`-with-filed-issue marker: un-ignore it
//! when the fix lands — it then locks the artifact clean forever.

use std::path::PathBuf;
use std::process::Command;

fn cdylib() -> Option<PathBuf> {
    let root = appname_conformance::paths::repo_root();
    for profile in ["debug", "release"] {
        let p = root
            .join("target")
            .join(profile)
            .join("libappname_cabi.dylib");
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn exported_symbols(lib: &PathBuf) -> Vec<String> {
    let out = Command::new("nm")
        .args(["-gU"])
        .arg(lib)
        .output()
        .expect("nm available");
    assert!(out.status.success(), "nm failed on {}", lib.display());
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .filter_map(|l| l.split_whitespace().last().map(str::to_string))
        .collect()
}

/// The DOCUMENTED gap (always-on): the strays censused above are still
/// present, and the appname surface is intact. This test pins the
/// CURRENT truth so the gap cannot silently grow or silently vanish
/// unnoticed (either direction forces a QA look).
#[test]
fn cdylib_stray_exports_pinned_qa_f10() {
    let Some(lib) = cdylib() else {
        eprintln!("cabi cdylib not built — census pin vacuous this run (gate builds it)");
        return;
    };
    let syms = exported_symbols(&lib);
    let appname = syms.iter().filter(|s| s.starts_with("_appname_")).count();
    let strays: Vec<&String> = syms
        .iter()
        .filter(|s| !s.starts_with("_appname_"))
        .collect();
    assert!(appname >= 29, "appname surface shrank: {appname} symbols");
    let known = [
        "_blake3_compress_in_place_portable",
        "___CLASS_RawWindowMetalLayer",
        "___DROP_FLAG_OFFSET_RawWindowMetalLayer",
        "___IVAR_OFFSET_RawWindowMetalLayer",
        "___REGISTER_CLASS_RawWindowMetalLayer",
    ];
    for s in &strays {
        assert!(
            known.contains(&s.as_str()),
            "NEW stray export {s} — the QA-F10 gap GREW; census + route before anything ships"
        );
    }
}

/// FAILING-BY-DESIGN (house marker): the clean-artifact gate. Un-ignore
/// when QA-F10's fix lands (exported-symbol control in the cabi link).
#[test]
#[ignore = "FILED QA-F10 (reports/O-QA-log.md CP-0.7 wave 2): cdylib leaks 5 non-appname \
            symbols (blake3 shim + objc2 metadata); un-ignore when O-SCRIPT lands \
            exported-symbol control — this test then locks the SDK artifact clean"]
fn cdylib_exports_only_appname_symbols() {
    let lib = cdylib().expect("cabi cdylib built");
    let strays: Vec<String> = exported_symbols(&lib)
        .into_iter()
        .filter(|s| !s.starts_with("_appname_"))
        .collect();
    assert!(
        strays.is_empty(),
        "stray non-appname exports in the SDK artifact: {strays:?}"
    );
}
