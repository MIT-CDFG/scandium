// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §14.1 (`NamedChannel` — the struct shape verbatim), §14.2
//! (alpha channels: stored coverage, selection round-trip), §14.3 (spot
//! reserved room, §7.7 unknown-entry round-trip), §7.4/§14.2 (persistence:
//! 8-bit encoding only when `appname_color::plane_is_u8_exact`); door C1
//! of reports/O-COLOR-log.md (the channels row's core-side arena ask).
//!
//! One arena row per NAMED channel (alpha now, spot post-1.0).
//! `ChannelSet::component` is a DERIVED view of the color buffer (§14.1)
//! — component channels have NO storage and NO independent identity here,
//! which is exactly why every "component channel targeted" contract case
//! is structurally unreachable wire-side (recorded divergence: core never
//! mints component ids; the target_kind_mismatch rows for typed selectors
//! live in the apply-image/calculations grammar instead).

use uuid::Uuid;

use crate::buffer::Buffer;
use crate::graph::arena::HasId;
use crate::graph::id::ChannelId;

/// §14.1 `NamedChannel { id, name, buffer }` — the buffer is Gray/f32
/// coverage exactly like masks and the selection (§13.1 one coverage
/// representation).
#[derive(Clone)]
pub struct NamedChannel {
    id: ChannelId,
    name: String,
    buffer: Buffer,
    /// §14.3 spot room, reserved: v1 kinds refuse `ink`/`solidity`
    /// (`error.unsupported.spot_channels`), but a reader encountering a
    /// spot entry round-trips it verbatim (§7.7). `None` on every channel
    /// a v1 kind creates.
    spot: Option<serde_json::Value>,
}

impl NamedChannel {
    pub(crate) fn new(id: ChannelId, name: impl Into<String>, buffer: Buffer) -> NamedChannel {
        NamedChannel {
            id,
            name: name.into(),
            buffer,
            spot: None,
        }
    }

    /// Loader constructor: restores a persisted row including the §7.7
    /// spot round-trip slot.
    pub(crate) fn from_loaded(
        id: ChannelId,
        name: String,
        buffer: Buffer,
        spot: Option<serde_json::Value>,
    ) -> NamedChannel {
        NamedChannel {
            id,
            name,
            buffer,
            spot,
        }
    }

    pub fn id(&self) -> ChannelId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn set_name(&mut self, name: String) {
        self.name = name;
    }

    /// The Gray/f32 coverage buffer (§14.2).
    pub fn buffer(&self) -> &Buffer {
        &self.buffer
    }

    pub(crate) fn buffer_mut(&mut self) -> &mut Buffer {
        &mut self.buffer
    }

    /// The §7.7 spot round-trip slot (always `None` for v1-created rows).
    pub fn spot(&self) -> Option<&serde_json::Value> {
        self.spot.as_ref()
    }
}

impl HasId for NamedChannel {
    fn uuid(&self) -> Uuid {
        self.id.as_uuid()
    }
}

/// Summary Debug (ES §1.6: never dump coverage payloads).
impl core::fmt::Debug for NamedChannel {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NamedChannel")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("tiles", &self.buffer.tiles().len())
            .field("spot", &self.spot.is_some())
            .finish()
    }
}
