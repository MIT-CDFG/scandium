// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §2.8 (serialization conventions) — the unknown-preserving enum
//! wrapper shared by every persisted enum-valued field.
//!
//! Conventions this crate follows for persisted JSON (ES §2.8, all
//! [NORMATIVE]): camelCase keys; `"schema": "appname.<thing>.vN"`
//! discriminators on top-level objects; UUID strings for ids; no binary inside
//! JSON (pixels ride package entries, ES §7); readers ignore unknown fields;
//! published-schema value sets win over the default snake_case enum rule
//! (`document.v1.json` pins camelCase blend/kind values). The canonical-output
//! writer (sorted keys, LF, 2-space indent) lands with the package writer
//! (slice 3, ES §7.6).

use serde::{Deserialize, Serialize};

/// Unknown-preserving wrapper (ES §2.8): a newer file's enum values survive an
/// older app. Deserialization attempts `Known` first (serde `untagged` = first
/// success wins); any failure captures the raw JSON value losslessly, and
/// serializing `Unknown` re-emits it verbatim. Code MUST treat `Unknown` as
/// inert: never dropped on save, never interpreted; runtime behavior falls
/// back per field (e.g. an unknown blend composites as `Normal`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Maybe<T> {
    Known(T),
    Unknown(serde_json::Value),
}

impl<T> Maybe<T> {
    pub fn known(&self) -> Option<&T> {
        match self {
            Maybe::Known(t) => Some(t),
            Maybe::Unknown(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::node::BlendMode;

    #[test]
    fn known_value_round_trips() {
        let m: Maybe<BlendMode> = serde_json::from_str("\"multiply\"").unwrap();
        assert!(matches!(m.known(), Some(BlendMode::Multiply)));
        assert_eq!(serde_json::to_string(&m).unwrap(), "\"multiply\"");
    }

    #[test]
    fn unknown_value_is_preserved_verbatim() {
        // A blend mode from a hypothetical newer schema version.
        let m: Maybe<BlendMode> = serde_json::from_str("\"quantumDodge\"").unwrap();
        assert!(m.known().is_none());
        assert_eq!(serde_json::to_string(&m).unwrap(), "\"quantumDodge\"");
    }

    #[test]
    fn unknown_non_string_shapes_survive_too() {
        let m: Maybe<BlendMode> = serde_json::from_str("{\"vendor\":\"x\",\"id\":9}").unwrap();
        assert!(m.known().is_none());
        let back = serde_json::to_value(&m).unwrap();
        assert_eq!(back, serde_json::json!({"vendor": "x", "id": 9}));
    }
}
