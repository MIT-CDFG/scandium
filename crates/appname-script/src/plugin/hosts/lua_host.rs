// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! The mlua binding of [`SandboxHost`] — the §25 sandboxed Lua tier.
//!
//! Spec: ES §25.3 ("Lua runs with a restricted standard library");
//! deps/mlua.yaml (=0.12.0, features exactly ["lua54","vendored"] —
//! vendored PUC Lua 5.4.8, an interpreter: no JIT, no macOS
//! entitlement implication from this host). Feature `lua-plugins`
//! (ES §33.1 row; default on, disabled => tier dropped).
//!
//! Sandbox mapping:
//! * **stdlib** — `TABLE | STRING | MATH` only: no `io`, no `os`, no
//!   `package`/`require`, no `debug`, no `coroutine`. The guest's whole
//!   outside world is the `appname` table, and every entry in it is one
//!   [`HostBroker::petition`] — the same gate the stub and wasm hosts
//!   cross, so the W08 corpus asserts identical behavior per host.
//! * **fuel** — an instruction-count hook every [`STRIDE`] VM
//!   instructions decrements a budget seeded from
//!   [`ResourceLimits::fuel`]; exhaustion raises a Lua error the
//!   invoke maps to [`PluginTrap::FuelExhausted`]. The same hook
//!   checks the wall-clock deadline.
//! * **memory** — `Lua::set_memory_limit`; allocation failure maps to
//!   [`PluginTrap::MemoryLimit`].
//! * **isolation** — a FRESH `Lua` state per invocation: no state
//!   leaks between invocations or plugins.
//!
//! Guest surface v1 (each returns `value` or `nil, errmsg` — denials
//! are error VALUES in the guest, recorded typed host-side):
//!
//! ```lua
//! appname.fs_read(path)          -- -> bytes|nil, err
//! appname.fs_write(path, bytes)  -- -> true|nil, err
//! appname.net(detail)            -- -> nil, err   (v1: always denied)
//! appname.clipboard()            -- -> nil, err   (v1: always denied)
//! appname.register(id, cap)     -- -> registry_id|nil, err
//! appname.apply(envelope_json)   -- -> receipt_json|nil, err
//! ```
//!
//! The invoke result is the invocation REPORT (the petition outcome
//! array, stub-host shape) — the real filter-return plumbing rides the
//! invoke-door ABI block.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use mlua::{HookTriggers, Lua, LuaOptions, StdLib, VmState};
use serde_json::Value;

use crate::plugin::host::{HostBroker, HostCall, HostError, HostReply, PluginTrap, SandboxHost};
use crate::plugin::limits::ResourceLimits;
use crate::plugin::permissions::Capability;

/// Instruction-hook stride: coarse enough to be cheap, fine enough
/// that a tight loop traps promptly (v1 pin, lane log).
const STRIDE: u32 = 1024;

/// Sentinel messages the budget hook raises (module-private contract
/// between the hook and the trap mapping).
const FUEL_MSG: &str = "appname:fuel-exhausted";
const DEADLINE_MSG: &str = "appname:deadline-exceeded";

/// See module docs.
#[derive(Debug, Default)]
pub struct LuaHost {
    chunk: Vec<u8>,
    limits: ResourceLimits,
    ready: bool,
}

impl LuaHost {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Shared petition context for the scoped `appname` table functions.
struct Ctx<'b> {
    broker: &'b mut dyn HostBroker,
    outcomes: Vec<Value>,
    /// Set when the broker's own budget refuses mid-petition.
    budget_trap: Option<PluginTrap>,
}

impl Ctx<'_> {
    /// Route one petition; record the outcome (stub-report shape);
    /// yield the guest-visible (reply | error-message) pair.
    fn petition(&mut self, call: HostCall) -> Result<HostReply, String> {
        if !self.broker.consume_fuel(1_000) {
            self.budget_trap = Some(PluginTrap::FuelExhausted);
            return Err(FUEL_MSG.into());
        }
        if self.broker.deadline_passed() {
            self.budget_trap = Some(PluginTrap::DeadlineExceeded);
            return Err(DEADLINE_MSG.into());
        }
        match self.broker.petition(call) {
            Ok(reply) => {
                self.outcomes.push(serde_json::json!({
                    "ok": super::stub::render_reply(&reply)
                }));
                Ok(reply)
            }
            Err(denial) => {
                let msg = denial.to_string();
                self.outcomes.push(serde_json::json!({
                    "denied": msg,
                    "wireId": denial.wire_id(),
                }));
                Err(msg)
            }
        }
    }
}

fn classify(err: &mlua::Error, ctx_trap: Option<PluginTrap>) -> HostError {
    // A budget refusal recorded by the ctx wins (it caused the error).
    if let Some(t) = ctx_trap {
        return HostError::Trap(t);
    }
    match err {
        mlua::Error::MemoryError(_) => HostError::Trap(PluginTrap::MemoryLimit),
        mlua::Error::CallbackError { cause, .. } => classify(cause, None),
        mlua::Error::RuntimeError(m) if m.contains(FUEL_MSG) => {
            HostError::Trap(PluginTrap::FuelExhausted)
        }
        mlua::Error::RuntimeError(m) if m.contains(DEADLINE_MSG) => {
            HostError::Trap(PluginTrap::DeadlineExceeded)
        }
        other => HostError::Trap(PluginTrap::Faulted(other.to_string())),
    }
}

/// Base-library globals that are STRIPPED before any plugin code runs.
///
/// [CRITICAL SANDBOX INVARIANT — §25.3] `StdLib::TABLE|STRING|MATH`
/// selects which *optional* libraries load, but PUC Lua's BASE library
/// (`luaopen_base`) is opened unconditionally by mlua, and it ships four
/// escape hatches that read/execute arbitrary paths, bypassing BOTH the
/// session-store confinement and the capability gate:
///
/// * `load` / `loadstring` — compile a string OR a bytecode blob
///   (`string.dump` output) into a callable chunk: arbitrary code load.
/// * `loadfile` — read+compile any absolute path: arbitrary fs read +
///   code load.
/// * `dofile` — read+compile+EXECUTE any absolute path: proven live
///   escape (`dofile("/abs/evil.lua")` ran and returned 42).
/// * `collectgarbage` — GC control is host policy, not the plugin's
///   (it can force collections that defeat the memory cap); no
///   legitimate sandboxed-plugin use.
///
/// `require`/`module`/`package` come with `StdLib::PACKAGE`, which we do
/// NOT load — they are absent — but we nil them defensively so a future
/// flag change can't silently reopen the path, and so the W08 corpus can
/// assert their absence uniformly.
const STRIPPED_BASE_GLOBALS: &[&str] = &[
    "load",
    "loadstring",
    "loadfile",
    "dofile",
    "collectgarbage",
    "require",
    "module",
    "package",
];

/// Build the restricted `Lua` state: stdlib subset, base-library
/// hardening (the CRITICAL step — see [`STRIPPED_BASE_GLOBALS`]), and
/// the memory cap.
///
/// SURVIVING base globals, each justified as non-fs/non-code-load:
/// `assert`/`error`/`pcall`/`xpcall` (error handling), `select`/`type`/
/// `tostring`/`tonumber` (value ops), `pairs`/`ipairs`/`next` (iteration),
/// `rawget`/`rawset`/`rawequal`/`rawlen` (raw table ops),
/// `setmetatable`/`getmetatable` (metatable ops — not an fs/code vector;
/// the string metatable's `__index` is just the already-present string
/// table), `print`/`warn` (write to the HOST's stdout/stderr, never the
/// filesystem or network), `_G` (globals self-reference — after this
/// hardening it contains nothing dangerous), `_VERSION` (a constant).
/// The `os`/`io`/`debug`/`coroutine`/`utf8` libraries are never opened
/// (their `StdLib` flags are absent) — the corpus asserts they are nil.
/// `string.dump` (bytecode serialization) is also stripped: with `load`
/// gone it cannot reload, but removing it closes the surface entirely.
fn restricted_lua(limits: &ResourceLimits) -> Result<Lua, HostError> {
    let lua = Lua::new_with(
        StdLib::TABLE | StdLib::STRING | StdLib::MATH,
        LuaOptions::default(),
    )
    .map_err(|e| HostError::BadEntry(format!("lua init: {e}")))?;
    harden_base(&lua)?;
    lua.set_memory_limit(limits.memory_max_bytes)
        .map_err(|e| HostError::BadEntry(format!("lua memory limit: {e}")))?;
    Ok(lua)
}

/// Strip the escape-hatch globals (and `string.dump`) from a freshly
/// constructed state. Idempotent; runs before any plugin bytecode.
fn harden_base(lua: &Lua) -> Result<(), HostError> {
    let globals = lua.globals();
    for name in STRIPPED_BASE_GLOBALS {
        globals
            .set(*name, mlua::Value::Nil)
            .map_err(|e| HostError::BadEntry(format!("lua harden {name}: {e}")))?;
    }
    // `string.dump` lives in the string TABLE, not globals.
    if let Ok(string_tbl) = globals.get::<mlua::Table>("string") {
        string_tbl
            .set("dump", mlua::Value::Nil)
            .map_err(|e| HostError::BadEntry(format!("lua harden string.dump: {e}")))?;
    }
    Ok(())
}

impl SandboxHost for LuaHost {
    fn instantiate(&mut self, entry: &[u8], limits: &ResourceLimits) -> Result<(), HostError> {
        self.limits = *limits;
        // Parse-check now (§25.6 lazy init still holds — this runs at
        // first invocation, not scan): a syntactically hostile chunk is
        // a typed BadEntry before any code runs.
        let lua = restricted_lua(limits)?;
        lua.load(entry)
            .set_name("plugin.lua")
            .into_function()
            .map_err(|e| HostError::BadEntry(format!("lua chunk rejected: {e}")))?;
        self.chunk = entry.to_vec();
        self.ready = true;
        Ok(())
    }

    fn invoke(
        &mut self,
        entry_point: &str,
        args_json: &str,
        broker: &mut dyn HostBroker,
    ) -> Result<String, HostError> {
        if !self.ready {
            return Err(HostError::BadEntry("invoke before instantiate".into()));
        }
        let lua = restricted_lua(&self.limits)?;

        // Budget hook: fuel + deadline ('static state only — Rc'd).
        let fuel = Rc::new(RefCell::new(self.limits.fuel));
        let started = Instant::now();
        let deadline_ms = self.limits.deadline_ms;
        {
            let fuel = Rc::clone(&fuel);
            lua.set_hook(
                HookTriggers::new().every_nth_instruction(STRIDE),
                move |_lua, _dbg| {
                    if let Some(rem) = fuel.borrow_mut().as_mut() {
                        *rem = rem.saturating_sub(u64::from(STRIDE));
                        if *rem == 0 {
                            return Err(mlua::Error::RuntimeError(FUEL_MSG.into()));
                        }
                    }
                    if started.elapsed().as_millis() as u64 > deadline_ms {
                        return Err(mlua::Error::RuntimeError(DEADLINE_MSG.into()));
                    }
                    Ok(VmState::Continue)
                },
            )
            .map_err(|e| HostError::BadEntry(format!("lua hook: {e}")))?;
        }

        let ctx = RefCell::new(Ctx {
            broker,
            outcomes: Vec::new(),
            budget_trap: None,
        });

        let run = lua.scope(|scope| {
            let appname = lua.create_table()?;
            // Each entry: unpack args -> one ctx.petition -> guest pair.
            let fs_read = scope.create_function(|lua, path: String| {
                match ctx.borrow_mut().petition(HostCall::FsRead { path }) {
                    Ok(HostReply::FileBytes(b)) => {
                        Ok((Some(lua.create_string(&b)?), None::<String>))
                    }
                    Ok(HostReply::NoSuchFile) => Ok((None, Some("no such file".into()))),
                    Ok(_) => Ok((None, Some("unexpected reply".into()))),
                    Err(msg) => Ok((None, Some(msg))),
                }
            })?;
            appname.set("fs_read", fs_read)?;

            let fs_write =
                scope.create_function(|_, (path, bytes): (String, mlua::LuaString)| {
                    let call = HostCall::FsWrite {
                        path,
                        bytes: bytes.as_bytes().to_vec(),
                    };
                    match ctx.borrow_mut().petition(call) {
                        Ok(HostReply::Written) => Ok((Some(true), None::<String>)),
                        Ok(HostReply::WriteFailed(e)) => Ok((None, Some(e))),
                        Ok(_) => Ok((None, Some("unexpected reply".into()))),
                        Err(msg) => Ok((None, Some(msg))),
                    }
                })?;
            appname.set("fs_write", fs_write)?;

            let net = scope.create_function(|_, detail: String| {
                match ctx.borrow_mut().petition(HostCall::Net { detail }) {
                    Ok(_) => Ok((None::<bool>, Some("unexpected reply".to_string()))),
                    Err(msg) => Ok((None, Some(msg))),
                }
            })?;
            appname.set("net", net)?;

            let clipboard = scope.create_function(|_, ()| {
                match ctx.borrow_mut().petition(HostCall::Clipboard) {
                    Ok(_) => Ok((None::<bool>, Some("unexpected reply".to_string()))),
                    Err(msg) => Ok((None, Some(msg))),
                }
            })?;
            appname.set("clipboard", clipboard)?;

            let register = scope.create_function(|_, (id, cap): (String, String)| {
                let Some(capability) = Capability::parse(&cap) else {
                    return Ok((None, Some(format!("unknown capability {cap:?}"))));
                };
                let call = HostCall::Register {
                    provider_id: id,
                    capability,
                };
                match ctx.borrow_mut().petition(call) {
                    Ok(HostReply::Registered(rid)) => Ok((Some(rid), None::<String>)),
                    Ok(_) => Ok((None, Some("unexpected reply".into()))),
                    Err(msg) => Ok((None, Some(msg))),
                }
            })?;
            appname.set("register", register)?;

            let apply = scope.create_function(|_, envelope_json: String| {
                match ctx
                    .borrow_mut()
                    .petition(HostCall::ApplyCommand { envelope_json })
                {
                    Ok(HostReply::Applied(receipt)) => Ok((Some(receipt), None::<String>)),
                    Ok(_) => Ok((None, Some("unexpected reply".into()))),
                    Err(msg) => Ok((None, Some(msg))),
                }
            })?;
            appname.set("apply", apply)?;

            lua.globals().set("appname", appname)?;

            // Run the chunk; then, when the chunk defined the entry
            // point as a global function, call it with args_json.
            lua.load(&self.chunk[..]).set_name("plugin.lua").exec()?;
            if let Ok(f) = lua.globals().get::<mlua::Function>(entry_point) {
                f.call::<()>(args_json.to_string())?;
            }
            Ok(())
        });

        let ctx = ctx.into_inner();
        match run {
            Ok(()) => Ok(Value::Array(ctx.outcomes).to_string()),
            Err(e) => Err(classify(&e, ctx.budget_trap)),
        }
    }
}
