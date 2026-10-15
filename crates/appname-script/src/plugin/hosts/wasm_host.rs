// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! The wasmtime binding of [`SandboxHost`] — the §25 sandboxed WASM
//! tier.
//!
//! Spec: ES §25.3 (capability-limited: "WASI with no preopened dirs by
//! default" — this v1 host goes further and links NO WASI at all: the
//! guest's entire outside world is the `appname` import module, every
//! entry one brokered petition; wasmtime-wasi consumption for
//! preopen-scoped grants is a later optimization, recorded in the lane
//! log); deps/wasmtime.yaml (=47.0.3, default-features off, elected
//! features runtime+cranelift+std+wat). Feature `wasm-plugins`
//! (ES §33.1 row; default on, disabled => tier dropped). macOS
//! packaging: Cranelift JIT needs `com.apple.security.cs.allow-jit`
//! under the hardened runtime — O-BUILD rider (plugin module docs).
//!
//! Sandbox mapping:
//! * **fuel** — `Config::consume_fuel` + `Store::set_fuel`; exhaustion
//!   traps `Trap::OutOfFuel` → [`PluginTrap::FuelExhausted`].
//! * **deadline** — `Config::epoch_interruption`; the petition loop on
//!   the invoking thread doubles as the epoch ticker: when the
//!   broker's deadline passes it bumps the engine epoch and the guest
//!   traps `Trap::Interrupt` → [`PluginTrap::DeadlineExceeded`].
//! * **memory** — `StoreLimitsBuilder::memory_size` +
//!   `trap_on_grow_failure` → [`PluginTrap::MemoryLimit`].
//! * **threading** — the guest runs on a scoped thread; host imports
//!   send [`HostCall`]s over a channel to the invoking thread, which
//!   owns the [`HostBroker`] (gate + engine door) — so the capability
//!   gate runs on exactly one thread with no `unsafe`, and the W08
//!   corpus crosses the same broker as every host.
//!
//! Guest ABI v1 (import module `"appname"`; strings are (ptr, len)
//! into the guest's exported linear `memory`; results are i32 status
//! codes — denials are ERROR VALUES in the guest, recorded typed
//! host-side; bulk readback rides the invoke-door ABI block):
//!
//! ```wat
//! (import "appname" "fs_read"   (func (param i32 i32) (result i32)))          ;; >=0 len | -1 denied | -2 absent
//! (import "appname" "fs_write"  (func (param i32 i32 i32 i32) (result i32)))  ;; 0 | -1 denied | -3 io
//! (import "appname" "net"       (func (param i32 i32) (result i32)))          ;; -1 (v1: always denied)
//! (import "appname" "clipboard" (func (result i32)))                          ;; -1 (v1: always denied)
//! (import "appname" "register"  (func (param i32 i32 i32 i32) (result i32)))  ;; 0 | -1 denied
//! (import "appname" "apply"     (func (param i32 i32) (result i32)))          ;; 0 ok | 1 engine-rejected | -1 refused
//! ```
//!
//! The invoke result is the invocation REPORT (petition outcome array,
//! one spelling across hosts — see `stub::render_reply`).

use std::sync::mpsc;
use std::time::Duration;

use serde_json::Value;
use wasmtime::{
    Caller, Config, Engine as WasmEngine, Linker, Module, ResourceLimiter, Store, StoreLimits,
    StoreLimitsBuilder, Trap,
};

use crate::plugin::gate::PermissionError;
use crate::plugin::host::{HostBroker, HostCall, HostError, HostReply, PluginTrap, SandboxHost};
use crate::plugin::limits::ResourceLimits;
use crate::plugin::permissions::Capability;

/// Petition-loop tick: how often the invoking thread checks the
/// deadline while the guest computes (v1 pin).
const TICK_MS: u64 = 5;

/// See module docs.
pub struct WasmHost {
    engine: WasmEngine,
    module: Option<Module>,
    limits: ResourceLimits,
}

/// A [`ResourceLimiter`] that wraps [`StoreLimits`] and RECORDS a memory
/// denial structurally — so a memory-cap trap is classified as
/// [`PluginTrap::MemoryLimit`] by fact, not by matching wasmtime's
/// backtrace string (which carries no memory-limit token).
struct LimiterState {
    inner: StoreLimits,
    memory_denied: bool,
}

impl ResourceLimiter for LimiterState {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        // StoreLimits with `trap_on_grow_failure` signals a denied grow
        // as Err (a trap), otherwise as Ok(false). Record the denial on
        // BOTH paths so the caller can classify the resulting trap as
        // MemoryLimit structurally, never by backtrace-string matching.
        match self.inner.memory_growing(current, desired, maximum) {
            Ok(true) => Ok(true),
            Ok(false) => {
                self.memory_denied = true;
                Ok(false)
            }
            Err(e) => {
                self.memory_denied = true;
                Err(e)
            }
        }
    }

    fn memory_grow_failed(&mut self, error: wasmtime::Error) -> wasmtime::Result<()> {
        // Delegate so `trap_on_grow_failure` (set on `inner`) still turns
        // a denied grow into a trap; `memory_denied` was already recorded
        // in `memory_growing` above.
        self.inner.memory_grow_failed(error)
    }

    fn table_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        self.inner.table_growing(current, desired, maximum)
    }

    fn table_grow_failed(&mut self, error: wasmtime::Error) -> wasmtime::Result<()> {
        self.inner.table_grow_failed(error)
    }

    fn instances(&self) -> usize {
        self.inner.instances()
    }

    fn tables(&self) -> usize {
        self.inner.tables()
    }

    fn memories(&self) -> usize {
        self.inner.memories()
    }
}

/// Store data for one invocation: the petition channel pair + limiter.
struct GuestCtx {
    limiter: LimiterState,
    calls: mpsc::Sender<HostCall>,
    replies: mpsc::Receiver<Result<HostReply, PermissionError>>,
}

impl WasmHost {
    pub fn new() -> Result<Self, HostError> {
        let mut config = Config::new();
        config.consume_fuel(true);
        config.epoch_interruption(true);
        let engine = WasmEngine::new(&config)
            .map_err(|e| HostError::BadEntry(format!("wasmtime engine: {e}")))?;
        Ok(Self {
            engine,
            module: None,
            limits: ResourceLimits::V1_DEFAULT,
        })
    }
}

/// Read a guest string given (ptr, len) against the exported memory.
fn guest_str(caller: &mut Caller<'_, GuestCtx>, ptr: u32, len: u32) -> wasmtime::Result<String> {
    let mem = caller
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .ok_or_else(|| wasmtime::Error::msg("guest exports no memory"))?;
    let mut buf = vec![0u8; len as usize];
    mem.read(&mut *caller, ptr as usize, &mut buf)
        .map_err(|e| wasmtime::Error::msg(format!("guest pointer out of bounds: {e}")))?;
    String::from_utf8(buf).map_err(|_| wasmtime::Error::msg("guest string is not UTF-8"))
}

fn guest_bytes(caller: &mut Caller<'_, GuestCtx>, ptr: u32, len: u32) -> wasmtime::Result<Vec<u8>> {
    let mem = caller
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .ok_or_else(|| wasmtime::Error::msg("guest exports no memory"))?;
    let mut buf = vec![0u8; len as usize];
    mem.read(&mut *caller, ptr as usize, &mut buf)
        .map_err(|e| wasmtime::Error::msg(format!("guest pointer out of bounds: {e}")))?;
    Ok(buf)
}

/// Send one petition to the broker thread; block for the verdict.
fn petition(
    caller: &mut Caller<'_, GuestCtx>,
    call: HostCall,
) -> wasmtime::Result<Result<HostReply, PermissionError>> {
    let ctx = caller.data_mut();
    ctx.calls
        .send(call)
        .map_err(|_| wasmtime::Error::msg("petition channel closed"))?;
    ctx.replies
        .recv()
        .map_err(|_| wasmtime::Error::msg("petition reply channel closed"))
}

/// Map a trap/error from the guest call into the typed contract.
/// Classify a guest error into the typed contract. Memory-limit traps
/// are NOT handled here — the caller detects those structurally via
/// [`LimiterState::memory_denied`] before falling through to this.
fn classify(err: wasmtime::Error) -> HostError {
    if let Some(trap) = err.downcast_ref::<Trap>() {
        return match trap {
            Trap::OutOfFuel => HostError::Trap(PluginTrap::FuelExhausted),
            Trap::Interrupt => HostError::Trap(PluginTrap::DeadlineExceeded),
            other => HostError::Trap(PluginTrap::Faulted(other.to_string())),
        };
    }
    HostError::Trap(PluginTrap::Faulted(err.to_string()))
}

impl SandboxHost for WasmHost {
    fn instantiate(&mut self, entry: &[u8], limits: &ResourceLimits) -> Result<(), HostError> {
        self.limits = *limits;
        // `Module::new` accepts binary wasm and (via the elected `wat`
        // feature) text-format sources — the W08 fixtures are WAT.
        self.module = Some(
            Module::new(&self.engine, entry)
                .map_err(|e| HostError::BadEntry(format!("wasm module rejected: {e}")))?,
        );
        Ok(())
    }

    fn invoke(
        &mut self,
        entry_point: &str,
        _args_json: &str,
        broker: &mut dyn HostBroker,
    ) -> Result<String, HostError> {
        let module = self
            .module
            .as_ref()
            .ok_or_else(|| HostError::BadEntry("invoke before instantiate".into()))?;

        let (call_tx, call_rx) = mpsc::channel::<HostCall>();
        let (reply_tx, reply_rx) = mpsc::channel::<Result<HostReply, PermissionError>>();

        let engine = &self.engine;
        let limits = self.limits;
        let mut outcomes: Vec<Value> = Vec::new();

        let guest_result: Result<(), HostError> = std::thread::scope(|s| {
            let guest = s.spawn(move || -> Result<(), HostError> {
                let ctx = GuestCtx {
                    limiter: LimiterState {
                        inner: StoreLimitsBuilder::new()
                            .memory_size(limits.memory_max_bytes)
                            .instances(2)
                            .trap_on_grow_failure(true)
                            .build(),
                        memory_denied: false,
                    },
                    calls: call_tx,
                    replies: reply_rx,
                };
                let mut store = Store::new(engine, ctx);
                store.limiter(|c| &mut c.limiter);
                if let Some(fuel) = limits.fuel {
                    store
                        .set_fuel(fuel)
                        .map_err(|e| HostError::BadEntry(format!("fuel: {e}")))?;
                }
                store.set_epoch_deadline(1);

                let mut linker: Linker<GuestCtx> = Linker::new(engine);
                linker
                    .func_wrap(
                        "appname",
                        "fs_read",
                        |mut caller: Caller<'_, GuestCtx>,
                         ptr: u32,
                         len: u32|
                         -> wasmtime::Result<i32> {
                            let path = guest_str(&mut caller, ptr, len)?;
                            Ok(match petition(&mut caller, HostCall::FsRead { path })? {
                                Ok(HostReply::FileBytes(b)) => {
                                    i32::try_from(b.len()).unwrap_or(i32::MAX)
                                }
                                Ok(HostReply::NoSuchFile) => -2,
                                Ok(_) => -9,
                                Err(_) => -1,
                            })
                        },
                    )
                    .and_then(|l| {
                        l.func_wrap(
                            "appname",
                            "fs_write",
                            |mut caller: Caller<'_, GuestCtx>,
                             pptr: u32,
                             plen: u32,
                             dptr: u32,
                             dlen: u32|
                             -> wasmtime::Result<i32> {
                                let path = guest_str(&mut caller, pptr, plen)?;
                                let bytes = guest_bytes(&mut caller, dptr, dlen)?;
                                Ok(
                                    match petition(&mut caller, HostCall::FsWrite { path, bytes })?
                                    {
                                        Ok(HostReply::Written) => 0,
                                        Ok(HostReply::WriteFailed(_)) => -3,
                                        Ok(_) => -9,
                                        Err(_) => -1,
                                    },
                                )
                            },
                        )
                    })
                    .and_then(|l| {
                        l.func_wrap(
                            "appname",
                            "net",
                            |mut caller: Caller<'_, GuestCtx>,
                             ptr: u32,
                             len: u32|
                             -> wasmtime::Result<i32> {
                                let detail = guest_str(&mut caller, ptr, len)?;
                                Ok(match petition(&mut caller, HostCall::Net { detail })? {
                                    Ok(_) => -9,
                                    Err(_) => -1,
                                })
                            },
                        )
                    })
                    .and_then(|l| {
                        l.func_wrap(
                            "appname",
                            "clipboard",
                            |mut caller: Caller<'_, GuestCtx>| -> wasmtime::Result<i32> {
                                Ok(match petition(&mut caller, HostCall::Clipboard)? {
                                    Ok(_) => -9,
                                    Err(_) => -1,
                                })
                            },
                        )
                    })
                    .and_then(|l| {
                        l.func_wrap(
                            "appname",
                            "register",
                            |mut caller: Caller<'_, GuestCtx>,
                             idp: u32,
                             idl: u32,
                             capp: u32,
                             capl: u32|
                             -> wasmtime::Result<i32> {
                                let id = guest_str(&mut caller, idp, idl)?;
                                let cap = guest_str(&mut caller, capp, capl)?;
                                let Some(capability) = Capability::parse(&cap) else {
                                    return Ok(-1);
                                };
                                let call = HostCall::Register {
                                    provider_id: id,
                                    capability,
                                };
                                Ok(match petition(&mut caller, call)? {
                                    Ok(HostReply::Registered(_)) => 0,
                                    Ok(_) => -9,
                                    Err(_) => -1,
                                })
                            },
                        )
                    })
                    .and_then(|l| {
                        l.func_wrap(
                            "appname",
                            "apply",
                            |mut caller: Caller<'_, GuestCtx>,
                             ptr: u32,
                             len: u32|
                             -> wasmtime::Result<i32> {
                                let envelope_json = guest_str(&mut caller, ptr, len)?;
                                Ok(
                                    match petition(
                                        &mut caller,
                                        HostCall::ApplyCommand { envelope_json },
                                    )? {
                                        Ok(HostReply::Applied(receipt)) => {
                                            let ok = serde_json::from_str::<Value>(&receipt)
                                                .map(|v| v["ok"] == Value::Bool(true))
                                                .unwrap_or(false);
                                            if ok {
                                                0
                                            } else {
                                                1
                                            }
                                        }
                                        Ok(_) => -9,
                                        Err(_) => -1,
                                    },
                                )
                            },
                        )
                    })
                    .map_err(|e| HostError::BadEntry(format!("linker: {e}")))?;

                let instance = linker.instantiate(&mut store, module).map_err(classify)?;
                let func = instance
                    .get_typed_func::<(), ()>(&mut store, entry_point)
                    .map_err(|e| HostError::BadEntry(format!("entry {entry_point:?}: {e}")))?;
                match func.call(&mut store, ()) {
                    Ok(()) => Ok(()),
                    // A memory-limit denial is recorded structurally by
                    // the limiter — map the resulting trap to MemoryLimit
                    // by fact, not by backtrace-string matching.
                    Err(e) if store.data().limiter.memory_denied => {
                        let _ = e;
                        Err(HostError::Trap(PluginTrap::MemoryLimit))
                    }
                    Err(e) => Err(classify(e)),
                }
            });

            // The petition loop: broker verdicts + outcome log + epoch
            // ticking, all on the invoking thread. A broker-budget
            // refusal (fuel/deadline) stops the guest via the epoch and
            // OVERRIDES the guest-side classification below — the typed
            // cause is the budget, not the induced interrupt.
            let mut budget_trap: Option<PluginTrap> = None;
            loop {
                match call_rx.recv_timeout(Duration::from_millis(TICK_MS)) {
                    Ok(call) => {
                        if !broker.consume_fuel(1_000) {
                            budget_trap = Some(PluginTrap::FuelExhausted);
                            engine.increment_epoch();
                            break; // no reply: the guest traps on epoch/channel
                        }
                        let verdict = broker.petition(call);
                        match &verdict {
                            Ok(reply) => outcomes.push(serde_json::json!({
                                "ok": super::stub::render_reply(reply)
                            })),
                            Err(denial) => outcomes.push(serde_json::json!({
                                "denied": denial.to_string(),
                                "wireId": denial.wire_id(),
                            })),
                        }
                        if reply_tx.send(verdict).is_err() {
                            break; // guest gone
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if budget_trap.is_none() && broker.deadline_passed() {
                            budget_trap = Some(PluginTrap::DeadlineExceeded);
                            engine.increment_epoch();
                        }
                        if guest.is_finished() {
                            break;
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            // Keep replies alive until here so a mid-petition guest sees
            // channel-closed only after the epoch already fired.
            drop(reply_tx);
            let joined = guest.join().unwrap_or_else(|_| {
                Err(HostError::Trap(PluginTrap::Faulted(
                    "guest thread panicked".into(),
                )))
            });
            match (joined, budget_trap) {
                (_, Some(t)) => Err(HostError::Trap(t)),
                (r, None) => r,
            }
        });

        guest_result?;
        Ok(Value::Array(outcomes).to_string())
    }
}
