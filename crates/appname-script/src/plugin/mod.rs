// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! The §25 SANDBOXED plugin tier v1 — manifest, permissions, capability
//! gates, lifecycle, and the sandbox-host seam.
//!
//! Spec: ES §25.1 (tiers), §25.2 (the `appname.plugin.v1` manifest),
//! §25.3 (permissions and sandbox — THE governing section for this
//! module), §25.4 (the sandboxed API subset), §25.5 (signing; the
//! out-of-process protocol is Post-1.0), §25.6 (lifecycle, directories,
//! `appname.pluginstate.v1`); §10.2 stage-5 capability check; plan §2.5
//! row "Plugins sandboxed tier v1" (lane O-SCRIPT).
//!
//! # Sandbox posture, honestly (ES §25.1)
//!
//! The Python tier (`appname-py`) is TRUSTED BY DESIGN — §25.1 says so
//! and its docs say so. **The sandbox story is THIS tier**: untrusted
//! community plugins run capability-limited under a WASM (`wasmtime`) or
//! Lua (`mlua`) host with
//!
//! * **no ambient filesystem** — a plugin sees its own per-plugin
//!   SESSION STORE plus explicitly granted directories, nothing else
//!   ([`SessionStore`], [`Gatekeeper`]);
//! * **no network** — `net` is DENIED at this tier in v1 regardless of
//!   manifest requests or stored grants ([`NetPolicy`] pins the §30.1
//!   `[privacy] network_features = false` hard-gate as the v1 constant);
//! * **typed refusals** — every denial is a [`PermissionError`] mapping
//!   to `APPNAME_ERR_PERMISSION` with the registered wire ids
//!   `error.permission.filesystem_denied` / `error.permission.network_denied`
//!   (contracts/message_ids.yaml, shipped-v1) — "the plugin cannot touch
//!   fs/net (tested, §34)";
//! * **bounded execution** — fuel, epoch deadline, and memory caps
//!   ([`ResourceLimits`]) so a runaway loop traps typed instead of
//!   hanging the app (W08 corpus).
//!
//! # One door (D-48; §3.11 hard rule)
//!
//! Every document mutation a plugin performs is an `appname.command.v1`
//! envelope through the ordinary apply entrance — hosts route every
//! capability petition through [`host::HostBroker`], and the broker's
//! only mutation reply is "submit this envelope through the door the
//! embedder gave us" (the C ABI for real embeddings, [`replay::ApplyDoor`]
//! in-process). If a front-end needs a new capability it becomes an ABI
//! addition with conformance tests — never a side door.
//!
//! # Record status (plan preamble rule 6)
//!
//! `deps/wasmtime.yaml` and `deps/mlua.yaml` were drafted by O-BUILD
//! (lane/build @ 4bf82d8) and this consumption is each record's named
//! `preAuthorizedFor` slice — record-first satisfied same-batch (O-INT
//! ruling on record). Pins per the records: `wasmtime =47.0.3`
//! (default features OFF; elected: runtime, cranelift, std, wat) and
//! `mlua =0.12.0` (features exactly `["lua54", "vendored"]` — vendored
//! PUC Lua 5.4.8). The bindings live at [`hosts::wasm_host`] /
//! [`hosts::lua_host`] behind the ES §33.1 features `wasm-plugins` /
//! `lua-plugins` (desktop default on; off = tier dropped, §48.4).
//! `wasmtime-wasi` is covered by the record but NOT consumed in v1:
//! this host links no WASI at all — the guest's entire outside world
//! is the `appname` import module (wasm_host docs).
//!
//! # macOS `allow-jit` entitlement (packaging note, flagged to O-BUILD)
//!
//! `wasmtime`'s default execution strategy is Cranelift JIT: it maps
//! W^X pages at runtime. Under the hardened runtime (required for
//! notarization, `scripts/build_dmg.sh`) that needs the
//! `com.apple.security.cs.allow-jit` entitlement on the app bundle —
//! without it, `mmap(MAP_JIT)` fails and the WASM host cannot start.
//! Lua (`mlua`) is an interpreter and needs no entitlement. Options for
//! O-BUILD's packaging call, recorded here and in the lane log: (a) add
//! `allow-jit` to the entitlement set (scoped, Apple-sanctioned for
//! exactly this use); (b) build wasmtime with the Pulley interpreter
//! backend (no JIT, slower); (c) ship the Lua host only on macOS. The
//! lane's recommendation is (a).

pub mod gate;
pub mod host;
pub mod hosts;
pub mod lifecycle;
pub mod limits;
pub mod manifest;
pub mod permissions;
pub mod session;
pub mod state;

pub use gate::{Gatekeeper, NetPolicy, PermissionError};
pub use host::{HostBroker, HostCall, HostError, HostReply, SandboxHost};
pub use lifecycle::{PluginRecord, PluginRegistry, PluginStatus, ScanDirs};
pub use limits::ResourceLimits;
pub use manifest::{Manifest, ManifestError, ProvidedKind, Provides};
pub use permissions::{Capability, Grant, GrantSet};
pub use session::SessionStore;
pub use state::{PluginState, StateFile};
