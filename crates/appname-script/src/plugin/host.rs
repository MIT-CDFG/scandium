// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! The sandbox-host seam: the trait every binding implements and the
//! broker every petition crosses (ES §25.3/§25.4).
//!
//! Shape: a [`SandboxHost`] executes untrusted code; whenever that code
//! wants ANYTHING from the outside world it issues a [`HostCall`]
//! through the [`HostBroker`] the invoker supplied. The broker owns the
//! [`Gatekeeper`] and the engine door — so the capability gate is
//! enforced in EXACTLY ONE place regardless of which VM is running, so
//! the W08 corpus asserts the same outcomes whether it drives the stub
//! host or the live wasm/lua bindings.
//!
//! Mutations: the broker's [`HostCall::ApplyCommand`] reply is produced
//! by the [`crate::replay::ApplyDoor`] the embedder supplied — the C
//! ABI in real embeddings. One door (D-48); no other mutation call
//! exists on this surface, deliberately.

use std::fmt;
use std::time::Instant;

use appname_core::engine::envelope::Envelope;

use crate::plugin::gate::{Gatekeeper, PermissionError};
use crate::plugin::limits::{FuelAccount, ResourceLimits};
use crate::plugin::manifest::Manifest;
use crate::plugin::permissions::Capability;
use crate::replay::ApplyDoor;

/// A capability petition from sandboxed code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostCall {
    /// Read a file. Relative = session store; absolute = needs a grant.
    FsRead { path: String },
    /// Write a file (same addressing).
    FsWrite { path: String, bytes: Vec<u8> },
    /// Any network access (v1: denied).
    Net { detail: String },
    /// Clipboard access (v1 sandboxed tier: denied).
    Clipboard,
    /// Register a provider (filter/tool/codec/panel/action/adjustment).
    Register {
        provider_id: String,
        capability: Capability,
    },
    /// Submit an `appname.command.v1` envelope through the one door.
    ApplyCommand { envelope_json: String },
}

/// The broker's reply to a sanctioned call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostReply {
    FileBytes(Vec<u8>),
    /// The petition was SANCTIONED but the file does not exist (or is
    /// unreadable). Distinct from a denial: the guest sees "no such
    /// file", never a permission surface, so probing for existence
    /// outside the sandbox stays impossible (denials fire first).
    NoSuchFile,
    Written,
    /// Sanctioned write failed at the IO layer (disk full, …) — guest-
    /// visible failure, not a permission surface.
    WriteFailed(String),
    /// The `x.`-prefixed registry id the provider was recorded under.
    Registered(String),
    /// The engine's receipt JSON for an applied command.
    Applied(String),
}

/// Why an invocation stopped abnormally (typed — the W08 contract).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginTrap {
    /// The fuel budget ran out (runaway loop class).
    FuelExhausted,
    /// The wall-clock deadline passed.
    DeadlineExceeded,
    /// The memory cap was hit.
    MemoryLimit,
    /// A capability petition was denied and the plugin chose (or is
    /// defined) to trap on it rather than handle it.
    Denied(PermissionError),
    /// The guest faulted (VM trap, unhandled guest error, bad entry).
    Faulted(String),
}

impl fmt::Display for PluginTrap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FuelExhausted => write!(f, "plugin exhausted its fuel budget"),
            Self::DeadlineExceeded => write!(f, "plugin exceeded its deadline"),
            Self::MemoryLimit => write!(f, "plugin exceeded its memory limit"),
            Self::Denied(e) => write!(f, "plugin trapped on a denied petition: {e}"),
            Self::Faulted(m) => write!(f, "plugin faulted: {m}"),
        }
    }
}

/// Host-level failure (constructing or running the VM).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostError {
    /// Entry bytes rejected before execution (bad wasm module, lua
    /// parse error, stub script parse error).
    BadEntry(String),
    /// The invocation trapped (typed).
    Trap(PluginTrap),
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadEntry(m) => write!(f, "plugin entry rejected: {m}"),
            Self::Trap(t) => t.fmt(f),
        }
    }
}

impl std::error::Error for HostError {}

/// What a binding calls when its guest petitions the outside world.
pub trait HostBroker {
    fn petition(&mut self, call: HostCall) -> Result<HostReply, PermissionError>;
    /// Consume fuel on behalf of the guest; `false` = exhausted, the
    /// binding must trap with [`PluginTrap::FuelExhausted`].
    #[must_use]
    fn consume_fuel(&mut self, n: u64) -> bool;
    /// `true` once the wall-clock deadline passed; the binding must
    /// trap with [`PluginTrap::DeadlineExceeded`].
    fn deadline_passed(&self) -> bool;
}

/// One sandbox binding (wasmtime, mlua, or the test stub).
///
/// Contract: `invoke` runs the named entry point with `args_json`,
/// routing EVERY external effect through `broker`; it returns the
/// guest's result JSON or a typed error. A binding MUST NOT touch the
/// filesystem, network, or engine except through the broker.
pub trait SandboxHost {
    /// Load and validate the entry payload (lazy per §25.6: this runs
    /// on first invocation, not at scan time).
    fn instantiate(&mut self, entry: &[u8], limits: &ResourceLimits) -> Result<(), HostError>;
    /// Run an exported entry point.
    fn invoke(
        &mut self,
        entry_point: &str,
        args_json: &str,
        broker: &mut dyn HostBroker,
    ) -> Result<String, HostError>;
}

/// The standard broker: gate + engine door + budgets. Bindings receive
/// it as `&mut dyn HostBroker`; the W08 suite drives it through the
/// stub host.
pub struct EngineBroker<'a> {
    gate: &'a Gatekeeper,
    door: &'a mut dyn ApplyDoor,
    fuel: FuelAccount,
    started: Instant,
    limits: ResourceLimits,
    /// Providers registered during this invocation (`x.`-prefixed ids).
    pub registered: Vec<String>,
}

impl<'a> EngineBroker<'a> {
    pub fn new(gate: &'a Gatekeeper, door: &'a mut dyn ApplyDoor, limits: ResourceLimits) -> Self {
        Self {
            gate,
            door,
            fuel: FuelAccount::new(&limits),
            started: Instant::now(),
            limits,
            registered: Vec::new(),
        }
    }

    pub fn manifest_gate(&self) -> &Gatekeeper {
        self.gate
    }
}

impl HostBroker for EngineBroker<'_> {
    fn petition(&mut self, call: HostCall) -> Result<HostReply, PermissionError> {
        match call {
            HostCall::FsRead { path } => {
                let real = self.gate.petition_fs(&path, false)?;
                // Sanctioned; an IO failure is guest-visible "no such
                // file", NOT a permission surface (variant docs).
                match std::fs::read(&real) {
                    Ok(b) => Ok(HostReply::FileBytes(b)),
                    Err(_) => Ok(HostReply::NoSuchFile),
                }
            }
            HostCall::FsWrite { path, bytes } => {
                let real = self.gate.petition_fs(&path, true)?;
                if let Some(parent) = real.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                match std::fs::write(&real, bytes) {
                    Ok(()) => Ok(HostReply::Written),
                    Err(e) => Ok(HostReply::WriteFailed(e.to_string())),
                }
            }
            HostCall::Net { .. } => {
                self.gate.petition_net()?;
                unreachable!("NetPolicy::Deny is total in v1")
            }
            HostCall::Clipboard => {
                self.gate.petition_clipboard()?;
                unreachable!("no sandboxed clipboard in v1")
            }
            HostCall::Register {
                provider_id,
                capability,
            } => {
                let registered_id = self.gate.petition_registration(&provider_id, capability)?;
                self.registered.push(registered_id.clone());
                Ok(HostReply::Registered(registered_id))
            }
            HostCall::ApplyCommand { envelope_json } => {
                let sanctioned = self.gate.petition_apply(&envelope_json)?;
                // The engine's own pipeline from stage 0 on — envelope
                // parse rejections come back as its ordinary typed error
                // JSON, exactly what any other caller of the door gets.
                let receipt = Envelope::from_json(sanctioned).and_then(|env| self.door.apply(env));
                let json = match receipt {
                    Ok(r) => serde_json::json!({ "ok": true, "receipt": r }),
                    Err(e) => serde_json::json!({ "ok": false, "error": e }),
                };
                Ok(HostReply::Applied(json.to_string()))
            }
        }
    }

    fn consume_fuel(&mut self, n: u64) -> bool {
        self.fuel.consume(n)
    }

    fn deadline_passed(&self) -> bool {
        self.started.elapsed().as_millis() as u64 > self.limits.deadline_ms
    }
}

/// Convenience: which host a manifest's entry wants, as a factory seam
/// for the lifecycle. Both VM bindings are LIVE behind their cargo
/// features (`wasm-plugins`/`lua-plugins`, default on); when a feature
/// is disabled its tier is [`HostAvailability::Dropped`] (§48.4) and
/// callers surface an honest "requires feature X" message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostAvailability {
    /// Compiled and constructible in this build.
    Available,
    /// The tier's cargo feature is off — DROPPED per §48.4 (graceful
    /// fallback: the Plugin Manager lists the plugin grayed with the
    /// named feature; nothing crashes).
    Dropped(&'static str),
}

/// Availability of the binding for a manifest (see [`HostAvailability`]).
pub fn host_availability(manifest: &Manifest) -> HostAvailability {
    use crate::plugin::manifest::EntryKind;
    match manifest.entry_kind() {
        EntryKind::Wasm => {
            #[cfg(feature = "wasm-plugins")]
            {
                HostAvailability::Available
            }
            #[cfg(not(feature = "wasm-plugins"))]
            {
                HostAvailability::Dropped("wasm-plugins")
            }
        }
        EntryKind::Lua => {
            #[cfg(feature = "lua-plugins")]
            {
                HostAvailability::Available
            }
            #[cfg(not(feature = "lua-plugins"))]
            {
                HostAvailability::Dropped("lua-plugins")
            }
        }
        // Python is the trusted tier (§25.1) — not served by this
        // module's hosts at all; the lifecycle lists it, appname-py runs it.
        EntryKind::Python => HostAvailability::Available,
    }
}
