// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Sandbox host bindings.
//!
//! * [`stub`] — the scripted test double; the W08 corpus's baseline
//!   host, always compiled (it is how the gate/broker stack stays
//!   tested even in a `--no-default-features` build).
//! * [`wasm_host`] — wasmtime, behind feature `wasm-plugins`
//!   (deps/wasmtime.yaml; ES §33.1 row, desktop default on).
//! * [`lua_host`] — mlua/vendored Lua 5.4, behind feature
//!   `lua-plugins` (deps/mlua.yaml; ES §33.1 row, desktop default on).
//!
//! With a feature disabled the tier is DROPPED per §48.4 (graceful:
//! everything else builds and works; `host_availability` reports it).
//! Every binding implements [`super::host::SandboxHost`] and routes
//! every external effect through the ONE [`super::host::HostBroker`] —
//! the capability gate is enforced in exactly one place regardless of
//! which VM is running, which is what lets the W08 corpus assert the
//! same outcomes per host.

#[cfg(feature = "lua-plugins")]
pub mod lua_host;
pub mod stub;
#[cfg(feature = "wasm-plugins")]
pub mod wasm_host;
