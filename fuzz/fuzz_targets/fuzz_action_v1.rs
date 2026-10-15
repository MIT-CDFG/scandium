// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Fuzz target: the `appname.action.v1` file parser —
//! `appname_script::Action::from_json` (ES §26.1;
//! `contracts/schemas/action.v1.json` read as law; the SCRIPT-D2/D3 lane
//! extensions `recordedTarget` + `assets`).
//!
//! Action files are user-loadable artifacts (batch runner, action
//! palette), so the parser is a first-class untrusted-bytes surface:
//! field-by-field validation with typed `error.schema.*` codes, the
//! 16 MiB `MAX_ACTION_BYTES` bomb gate, RFC 6901 pointer syntax checks on
//! `parameterized`, base64 asset decoding with content-addressed id
//! verification (`asset_id_for(bytes)` must equal the declared id — a
//! corrupt action answers typed at parse, not at replay).
//!
//! On accepted inputs the target asserts the wire fixpoint the
//! record → file → replay spine depends on: `to_json` of a parsed action
//! MUST re-parse. A violation is a real invariant break and is WANTED as
//! a crash finding.
//!
//! Bar: every input parses Ok or returns a typed `EngineError` — never a
//! panic, never an abort.

#![no_main]

use libfuzzer_sys::fuzz_target;

use appname_script::Action;

fuzz_target!(|data: &[u8]| {
    // Lossy widening keeps non-UTF-8 mutations in play.
    let text = String::from_utf8_lossy(data);
    if let Ok(action) = Action::from_json(&text) {
        let re = action.to_json();
        let _ = Action::from_json(&re)
            .expect("re-emitted action must re-parse (record→file→replay wire fixpoint)");
    }
});
