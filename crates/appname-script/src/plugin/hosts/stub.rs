// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! The scripted STUB host — a deterministic [`SandboxHost`] whose
//! "entry bytes" are a JSON action script, so the hostile-plugin corpus
//! (`tests/plugin_safety.rs`) exercises the REAL broker, gate, session
//! store, and budget paths with no VM dependency — it is the host that
//! keeps that stack tested even in a `--no-default-features` build.
//!
//! Not a toy in the load-bearing sense: everything OUTSIDE the VM —
//! which is exactly what the sandbox contract governs — is the
//! production code path. The VM-level behaviors it cannot reach
//! (guest-code escape surface, real fuel/epoch/memory traps) are
//! covered against the live wasm/lua bindings in `tests/plugin_hosts.rs`.
//!
//! Script form (`appname.stubscript.v1`):
//!
//! ```json
//! { "schema": "appname.stubscript.v1",
//!   "steps": [
//!     { "op": "fs_read",  "path": "presets/a.json" },
//!     { "op": "fs_write", "path": "cache/out.bin", "bytes": "aGk=" },
//!     { "op": "net",      "detail": "https://evil.example" },
//!     { "op": "clipboard" },
//!     { "op": "register", "id": "org.example.t.f", "capability": "filter.register" },
//!     { "op": "apply",    "envelope": { "schema": "appname.command.v1", "...": "..." } },
//!     { "op": "spin",     "fuel": 1000000 },
//!     { "op": "sleep_ms", "ms": 50 },
//!     { "op": "trap_on_denial" }
//!   ] }
//! ```
//!
//! Default denial behavior mirrors the real guests: a denied petition
//! surfaces to the script as a recorded denial and execution CONTINUES
//! (the guest saw a host error and may proceed) — unless
//! `trap_on_denial` ran earlier, after which the next denial traps the
//! invocation typed ([`PluginTrap::Denied`]).

use serde_json::Value;

use crate::plugin::host::{HostBroker, HostCall, HostError, HostReply, PluginTrap, SandboxHost};
use crate::plugin::limits::ResourceLimits;
use crate::plugin::permissions::Capability;

/// See module docs.
#[derive(Debug, Default)]
pub struct ScriptedStubHost {
    steps: Vec<Value>,
}

/// One observed outcome, echoed in the invocation report the stub
/// returns (a JSON array — tests assert on it).
#[derive(Debug)]
enum Outcome {
    Reply(String),
    Denied(String, Option<&'static str>),
}

impl ScriptedStubHost {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Render a sanctioned reply for the invocation report — ONE spelling
/// across every host (stub/wasm/lua), so the W08 corpus asserts the
/// same strings whichever binding ran.
pub(crate) fn render_reply(reply: &HostReply) -> String {
    match reply {
        HostReply::FileBytes(b) => format!("file:{}B", b.len()),
        HostReply::NoSuchFile => "no-such-file".into(),
        HostReply::Written => "written".into(),
        HostReply::WriteFailed(e) => format!("write-failed:{e}"),
        HostReply::Registered(id) => format!("registered:{id}"),
        HostReply::Applied(r) => format!("applied:{r}"),
    }
}

/// Base64 decode for the `bytes` field — minimal, strict, stdlib-only
/// (no new dependency for a test double).
fn b64_decode(s: &str) -> Result<Vec<u8>, HostError> {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let val = |c: u8| -> Option<u32> { ALPHABET.iter().position(|&a| a == c).map(|p| p as u32) };
    let s = s.trim_end_matches('=');
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let bytes = s.as_bytes();
    for chunk in bytes.chunks(4) {
        let mut acc: u32 = 0;
        for (i, &c) in chunk.iter().enumerate() {
            let v = val(c).ok_or_else(|| {
                HostError::BadEntry(format!("stub script: bad base64 byte {c:#x}"))
            })?;
            acc |= v << (18 - 6 * i);
        }
        let n = chunk.len();
        if n < 2 {
            return Err(HostError::BadEntry("stub script: truncated base64".into()));
        }
        out.push((acc >> 16) as u8);
        if n > 2 {
            out.push((acc >> 8) as u8);
        }
        if n > 3 {
            out.push(acc as u8);
        }
    }
    Ok(out)
}

impl SandboxHost for ScriptedStubHost {
    fn instantiate(&mut self, entry: &[u8], _limits: &ResourceLimits) -> Result<(), HostError> {
        let v: Value = serde_json::from_slice(entry)
            .map_err(|e| HostError::BadEntry(format!("stub script is not JSON: {e}")))?;
        if v.get("schema").and_then(Value::as_str) != Some("appname.stubscript.v1") {
            return Err(HostError::BadEntry(
                "stub script schema is not appname.stubscript.v1".into(),
            ));
        }
        let steps = v
            .get("steps")
            .and_then(Value::as_array)
            .ok_or_else(|| HostError::BadEntry("stub script has no steps array".into()))?;
        self.steps = steps.clone();
        Ok(())
    }

    fn invoke(
        &mut self,
        _entry_point: &str,
        _args_json: &str,
        broker: &mut dyn HostBroker,
    ) -> Result<String, HostError> {
        let mut outcomes: Vec<Outcome> = Vec::new();
        let mut trap_on_denial = false;
        for step in &self.steps {
            // Per-step overhead: every guest step costs fuel, so even a
            // long petition script is bounded.
            if !broker.consume_fuel(1_000) {
                return Err(HostError::Trap(PluginTrap::FuelExhausted));
            }
            if broker.deadline_passed() {
                return Err(HostError::Trap(PluginTrap::DeadlineExceeded));
            }
            let op = step
                .get("op")
                .and_then(Value::as_str)
                .ok_or_else(|| HostError::BadEntry("stub script step without op".into()))?;
            let str_field = |k: &str| -> Result<String, HostError> {
                step.get(k)
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .ok_or_else(|| HostError::BadEntry(format!("step {op} missing {k}")))
            };
            let call = match op {
                "spin" => {
                    let fuel = step.get("fuel").and_then(Value::as_u64).unwrap_or(u64::MAX);
                    if !broker.consume_fuel(fuel) {
                        return Err(HostError::Trap(PluginTrap::FuelExhausted));
                    }
                    continue;
                }
                "sleep_ms" => {
                    let ms = step.get("ms").and_then(Value::as_u64).unwrap_or(0);
                    std::thread::sleep(std::time::Duration::from_millis(ms));
                    if broker.deadline_passed() {
                        return Err(HostError::Trap(PluginTrap::DeadlineExceeded));
                    }
                    continue;
                }
                "trap_on_denial" => {
                    trap_on_denial = true;
                    continue;
                }
                "fs_read" => HostCall::FsRead {
                    path: str_field("path")?,
                },
                "fs_write" => HostCall::FsWrite {
                    path: str_field("path")?,
                    bytes: b64_decode(&str_field("bytes")?)?,
                },
                "net" => HostCall::Net {
                    detail: str_field("detail")?,
                },
                "clipboard" => HostCall::Clipboard,
                "register" => HostCall::Register {
                    provider_id: str_field("id")?,
                    capability: Capability::parse(&str_field("capability")?).ok_or_else(|| {
                        HostError::BadEntry("step register: unknown capability".into())
                    })?,
                },
                "apply" => HostCall::ApplyCommand {
                    envelope_json: step
                        .get("envelope")
                        .ok_or_else(|| HostError::BadEntry("step apply missing envelope".into()))?
                        .to_string(),
                },
                other => return Err(HostError::BadEntry(format!("unknown stub op {other:?}"))),
            };
            match broker.petition(call) {
                Ok(reply) => outcomes.push(Outcome::Reply(render_reply(&reply))),
                Err(denial) => {
                    if trap_on_denial {
                        return Err(HostError::Trap(PluginTrap::Denied(denial)));
                    }
                    outcomes.push(Outcome::Denied(denial.to_string(), denial.wire_id()));
                }
            }
        }
        let report: Vec<Value> = outcomes
            .iter()
            .map(|o| match o {
                Outcome::Reply(s) => serde_json::json!({ "ok": s }),
                Outcome::Denied(msg, wire) => {
                    serde_json::json!({ "denied": msg, "wireId": wire })
                }
            })
            .collect();
        Ok(Value::Array(report).to_string())
    }
}
