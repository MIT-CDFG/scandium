// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! The `appname.action.v1` document model: parse, validate, serialize.
//!
//! Spec: ES §26.1 (action format; the mirrored artifact is
//! `contracts/schemas/action.v1.json` — read as law here), §10.2 (the
//! `appname.command.v1` command object each command step carries).
//!
//! Validation is field-by-field with the engine's structured error
//! taxonomy (`error.schema.*` / `error.api.invalid_arg`), the same
//! discipline as `appname-core`'s envelope parser: rejections carry typed
//! codes, not serde messages.
//!
//! # Shape (contract, verbatim §26.1)
//!
//! Exactly TWO step types — `command` and `stop`; actions are linear
//! (conditionals deliberately excluded from v1). Every step carries
//! `enabled` (default true); disabled steps are skipped without error.
//! `parameterized` paths MUST be RFC 6901 JSON Pointers; a pointer that
//! does not resolve at replay is a step error. A stop's `message` is
//! author-typed text shown verbatim (user content, not a MessageId).
//!
//! # Lane extensions (schema-open keys; fold-in divergences SCRIPT-D2/D3)
//!
//! The §26.1 step schema names no carrier for two things replay
//! normatively requires:
//!
//! * **rule 3** target rewriting needs "the name captured at record time"
//!   and a tree-index fallback — carried as the step-level
//!   `recordedTarget` object `{"layerName","treeIndex"}`;
//! * **rule 4** stroke replay "from `samples` assets" needs the asset
//!   BYTES to travel with the action (batch replays onto documents that
//!   have never seen them) — carried as the top-level `assets` array
//!   `[{"id","kind","base64"}]`. Asset ids are content-addressed
//!   (BLAKE3-derived UUIDs, `appname_core::engine::assets::asset_id_for`),
//!   so re-ingestion at replay converges on the recorded id.
//!
//! Both keys are valid under the mirrored schema (no
//! `additionalProperties: false` at those levels) and are recorded as
//! typed divergences in the lane log for the v5.6 fold-in.

use serde_json::{json, Map, Value};
use uuid::Uuid;

use appname_core::engine::assets::AssetKind;
use appname_core::{EngineError, ErrorCode};

/// The `schema` const every action document must carry.
pub const ACTION_SCHEMA: &str = "appname.action.v1";

/// §9.6-consistent cap: action documents above this size are rejected up
/// front (16 MiB — actions embed asset bytes, so the envelope's 1 MiB cap
/// would be too tight; the number is a lane election, noted in the log).
pub const MAX_ACTION_BYTES: usize = 16 * 1024 * 1024;

/// Rule-3 capture: the replay-resolution hints recorded alongside a
/// command step's `target.layerId` (lane extension, module docs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedTarget {
    /// Layer name at record time — the FIRST resolution key (rule 3:
    /// depth-first name match).
    pub layer_name: Option<String>,
    /// Tree index at record time (the info-row index: position in the
    /// document's z-order walk) — the SECOND resolution key.
    pub tree_index: Option<usize>,
}

/// One `command` step.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandStep {
    /// The recorded `appname.command.v1` object, kept VERBATIM as parsed
    /// (values captured, D-48 — the recorded envelope IS the action).
    /// Replay mints a fresh command id (ids are identity, not captured
    /// values; the journal rejects duplicates) and re-validates through
    /// the ordinary envelope door.
    pub command: Value,
    pub enabled: bool,
    /// RFC 6901 pointers into the COMMAND object (`/params/points` per
    /// the §26.1 example) that replay may substitute from batch
    /// variables. Syntax is validated at parse; resolution is answered at
    /// replay (a substituted pointer that does not resolve is a step
    /// error).
    pub parameterized: Vec<String>,
    /// Rule-3 capture (lane extension).
    pub recorded_target: Option<RecordedTarget>,
}

/// One `stop` step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopStep {
    /// Author-typed, shown/reported verbatim.
    pub message: String,
    pub allow_continue: bool,
    pub enabled: bool,
}

/// A step: exactly two types, §26.1.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Command(CommandStep),
    Stop(StopStep),
}

/// A captured asset traveling with the action (lane extension; rule 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionAsset {
    /// Content-addressed id (must equal `asset_id_for(&bytes)`; verified
    /// at parse so a corrupt action answers typed here, not at replay).
    pub id: Uuid,
    pub kind: AssetKind,
    pub bytes: Vec<u8>,
}

/// A parsed, validated `appname.action.v1` document.
#[derive(Debug, Clone, PartialEq)]
pub struct Action {
    pub id: Option<Uuid>,
    pub name: String,
    pub steps: Vec<Step>,
    /// Lane extension (module docs); empty for actions that reference no
    /// assets.
    pub assets: Vec<ActionAsset>,
}

fn schema_err(msg: impl Into<String>) -> EngineError {
    EngineError::new(ErrorCode::WrongSchema, msg)
}

fn missing(field: &str) -> EngineError {
    EngineError::with_field(
        ErrorCode::MissingField,
        field,
        format!("action document is missing required field `{field}`"),
    )
}

impl Action {
    /// Parse and validate an action document from JSON text.
    pub fn from_json(text: &str) -> Result<Action, EngineError> {
        if text.len() > MAX_ACTION_BYTES {
            return Err(EngineError::new(
                ErrorCode::InvalidArg,
                format!("action document is {} bytes; the cap is 16 MiB", text.len()),
            ));
        }
        let value: Value = serde_json::from_str(text)
            .map_err(|e| EngineError::new(ErrorCode::InvalidArg, format!("invalid JSON: {e}")))?;
        Self::from_value(value)
    }

    /// Validate an action from an already-parsed JSON value.
    pub fn from_value(value: Value) -> Result<Action, EngineError> {
        let Value::Object(obj) = value else {
            return Err(schema_err("action document must be a JSON object"));
        };
        match obj.get("schema") {
            None => return Err(missing("schema")),
            Some(Value::String(s)) if s == ACTION_SCHEMA => {}
            Some(other) => {
                return Err(schema_err(format!(
                    "schema must be the constant \"{ACTION_SCHEMA}\", got {other}"
                )))
            }
        }
        let id = match obj.get("id") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(Uuid::parse_str(s).map_err(|_| {
                EngineError::with_field(
                    ErrorCode::WrongSchema,
                    "id",
                    format!("id is not a UUID: {s:?}"),
                )
            })?),
            Some(other) => {
                return Err(schema_err(format!("id must be a UUID string, got {other}")))
            }
        };
        let name = match obj.get("name") {
            None => return Err(missing("name")),
            Some(Value::String(s)) => s.clone(),
            Some(other) => return Err(schema_err(format!("name must be a string, got {other}"))),
        };
        let steps_raw = match obj.get("steps") {
            None => return Err(missing("steps")),
            Some(Value::Array(a)) => a,
            Some(other) => return Err(schema_err(format!("steps must be an array, got {other}"))),
        };
        let mut steps = Vec::with_capacity(steps_raw.len());
        for (i, raw) in steps_raw.iter().enumerate() {
            steps.push(parse_step(i, raw)?);
        }
        let assets = match obj.get("assets") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(a)) => {
                let mut out = Vec::with_capacity(a.len());
                for (i, raw) in a.iter().enumerate() {
                    out.push(parse_asset(i, raw)?);
                }
                out
            }
            Some(other) => return Err(schema_err(format!("assets must be an array, got {other}"))),
        };
        Ok(Action {
            id,
            name,
            steps,
            assets,
        })
    }

    /// Serialize to the wire form (deterministic key order).
    pub fn to_json(&self) -> String {
        self.to_value().to_string()
    }

    /// Serialize to a JSON value (deterministic key order — `json!` object
    /// literals preserve the written order under this workspace's
    /// `serde_json` configuration only for structs; maps sort by key,
    /// which is itself deterministic).
    pub fn to_value(&self) -> Value {
        let mut obj = Map::new();
        obj.insert("schema".into(), json!(ACTION_SCHEMA));
        if let Some(id) = self.id {
            obj.insert("id".into(), json!(id.to_string()));
        }
        obj.insert("name".into(), json!(self.name));
        obj.insert(
            "steps".into(),
            Value::Array(self.steps.iter().map(step_value).collect()),
        );
        if !self.assets.is_empty() {
            obj.insert(
                "assets".into(),
                Value::Array(
                    self.assets
                        .iter()
                        .map(|a| {
                            json!({
                                "id": a.id.to_string(),
                                "kind": kind_tag(a.kind),
                                "base64": base64_encode(&a.bytes),
                            })
                        })
                        .collect(),
                ),
            );
        }
        Value::Object(obj)
    }

    /// The command steps' count (reporting convenience).
    pub fn command_step_count(&self) -> usize {
        self.steps
            .iter()
            .filter(|s| matches!(s, Step::Command(_)))
            .count()
    }
}

fn step_value(step: &Step) -> Value {
    match step {
        Step::Command(c) => {
            let mut obj = Map::new();
            obj.insert("command".into(), c.command.clone());
            if !c.enabled {
                obj.insert("enabled".into(), json!(false));
            }
            if !c.parameterized.is_empty() {
                obj.insert("parameterized".into(), json!(c.parameterized));
            }
            if let Some(rt) = &c.recorded_target {
                let mut t = Map::new();
                if let Some(n) = &rt.layer_name {
                    t.insert("layerName".into(), json!(n));
                }
                if let Some(i) = rt.tree_index {
                    t.insert("treeIndex".into(), json!(i));
                }
                obj.insert("recordedTarget".into(), Value::Object(t));
            }
            Value::Object(obj)
        }
        Step::Stop(s) => {
            let mut stop = Map::new();
            stop.insert("message".into(), json!(s.message));
            if !s.allow_continue {
                stop.insert("allowContinue".into(), json!(false));
            }
            let mut obj = Map::new();
            obj.insert("stop".into(), Value::Object(stop));
            if !s.enabled {
                obj.insert("enabled".into(), json!(false));
            }
            Value::Object(obj)
        }
    }
}

fn parse_step(index: usize, raw: &Value) -> Result<Step, EngineError> {
    let Value::Object(obj) = raw else {
        return Err(schema_err(format!("steps[{index}] must be a JSON object")));
    };
    let has_command = obj.contains_key("command");
    let has_stop = obj.contains_key("stop");
    if has_command == has_stop {
        return Err(schema_err(format!(
            "steps[{index}] must carry exactly one of `command` | `stop` (§26.1: two step types)"
        )));
    }
    let enabled = match obj.get("enabled") {
        None => true,
        Some(Value::Bool(b)) => *b,
        Some(other) => {
            return Err(schema_err(format!(
                "steps[{index}].enabled must be a boolean, got {other}"
            )))
        }
    };
    if has_stop {
        let Some(Value::Object(stop)) = obj.get("stop") else {
            return Err(schema_err(format!(
                "steps[{index}].stop must be a JSON object"
            )));
        };
        let message = match stop.get("message") {
            Some(Value::String(s)) => s.clone(),
            Some(other) => {
                return Err(schema_err(format!(
                    "steps[{index}].stop.message must be a string, got {other}"
                )))
            }
            None => return Err(missing("stop.message")),
        };
        let allow_continue = match stop.get("allowContinue") {
            None => true,
            Some(Value::Bool(b)) => *b,
            Some(other) => {
                return Err(schema_err(format!(
                    "steps[{index}].stop.allowContinue must be a boolean, got {other}"
                )))
            }
        };
        return Ok(Step::Stop(StopStep {
            message,
            allow_continue,
            enabled,
        }));
    }
    let command = obj.get("command").expect("has_command checked").clone();
    {
        // Structural pre-checks only — full envelope validation happens at
        // replay through the ordinary door (stages 0-7). The §26.1 example
        // records commands WITHOUT an `id` (replay mints one), so `id` is
        // NOT required here — a noted softening of the `$ref` to
        // command.v1 (divergence SCRIPT-D4 in the lane log).
        let Value::Object(cmd) = &command else {
            return Err(schema_err(format!(
                "steps[{index}].command must be a JSON object"
            )));
        };
        match cmd.get("kind") {
            Some(Value::String(_)) => {}
            Some(other) => {
                return Err(schema_err(format!(
                    "steps[{index}].command.kind must be a string, got {other}"
                )))
            }
            None => return Err(missing("command.kind")),
        }
        if let Some(s) = cmd.get("schema") {
            if s != "appname.command.v1" {
                return Err(schema_err(format!(
                    "steps[{index}].command.schema must be \"appname.command.v1\", got {s}"
                )));
            }
        }
    }
    let parameterized = match obj.get("parameterized") {
        None => Vec::new(),
        Some(Value::Array(a)) => {
            let mut out = Vec::with_capacity(a.len());
            for p in a {
                let Value::String(p) = p else {
                    return Err(schema_err(format!(
                        "steps[{index}].parameterized entries must be strings, got {p}"
                    )));
                };
                // RFC 6901 syntax: "" (whole doc) or starts with '/'.
                if !p.is_empty() && !p.starts_with('/') {
                    return Err(EngineError::with_field(
                        ErrorCode::OutOfRange,
                        "parameterized",
                        format!(
                            "steps[{index}].parameterized entry {p:?} is not an RFC 6901 \
                             JSON Pointer (must be \"\" or start with '/')"
                        ),
                    ));
                }
                out.push(p.clone());
            }
            out
        }
        Some(other) => {
            return Err(schema_err(format!(
                "steps[{index}].parameterized must be an array, got {other}"
            )))
        }
    };
    let recorded_target = match obj.get("recordedTarget") {
        None | Some(Value::Null) => None,
        Some(Value::Object(t)) => {
            let layer_name = match t.get("layerName") {
                None | Some(Value::Null) => None,
                Some(Value::String(s)) => Some(s.clone()),
                Some(other) => {
                    return Err(schema_err(format!(
                        "steps[{index}].recordedTarget.layerName must be a string, got {other}"
                    )))
                }
            };
            let tree_index = match t.get("treeIndex") {
                None | Some(Value::Null) => None,
                Some(v) => Some(v.as_u64().ok_or_else(|| {
                    schema_err(format!(
                        "steps[{index}].recordedTarget.treeIndex must be a non-negative \
                         integer, got {v}"
                    ))
                })? as usize),
            };
            Some(RecordedTarget {
                layer_name,
                tree_index,
            })
        }
        Some(other) => {
            return Err(schema_err(format!(
                "steps[{index}].recordedTarget must be an object, got {other}"
            )))
        }
    };
    Ok(Step::Command(CommandStep {
        command,
        enabled,
        parameterized,
        recorded_target,
    }))
}

fn parse_asset(index: usize, raw: &Value) -> Result<ActionAsset, EngineError> {
    let Value::Object(obj) = raw else {
        return Err(schema_err(format!("assets[{index}] must be a JSON object")));
    };
    let id = match obj.get("id") {
        Some(Value::String(s)) => Uuid::parse_str(s)
            .map_err(|_| schema_err(format!("assets[{index}].id is not a UUID: {s:?}")))?,
        _ => return Err(missing("assets[].id")),
    };
    let kind = match obj.get("kind") {
        Some(Value::String(s)) => kind_from_tag(s).ok_or_else(|| {
            // [Ratification rider, return leg: the diagnostic had gone
            // stale against the door-slice-2 carriage arms — it now
            // enumerates the full tag set kind_from_tag accepts.]
            schema_err(format!(
                "assets[{index}].kind must be \"samples\" | \"rasterImage\" | \"fontFace\" \
                 | \"maskPlane\", got {s:?}"
            ))
        })?,
        _ => return Err(missing("assets[].kind")),
    };
    let bytes = match obj.get("base64") {
        Some(Value::String(s)) => base64_decode(s)
            .ok_or_else(|| schema_err(format!("assets[{index}].base64 is not valid base64")))?,
        _ => return Err(missing("assets[].base64")),
    };
    let expect = appname_core::engine::assets::asset_id_for(&bytes);
    if expect != id {
        return Err(schema_err(format!(
            "assets[{index}] bytes do not hash to their recorded id (recorded {id}, \
             content-addressed {expect}) — the action file is corrupt"
        )));
    }
    Ok(ActionAsset { id, kind, bytes })
}

/// The `appname.command.v1` schema const (minted into replayed commands
/// that omitted it, SCRIPT-D4).
pub(crate) fn envelope_schema() -> &'static str {
    appname_core::engine::envelope::ENVELOPE_SCHEMA
}

pub(crate) fn kind_tag(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::Samples => "samples",
        AssetKind::RasterImage => "rasterImage",
        // MECHANICAL companion arms (core door slice 2, Doors 10 + D-ML-1
        // — RATIFIED by O-SCRIPT, return leg, reports/O-SCRIPT-log.md):
        // embedded font faces (§23.5) and mask coverage planes
        // (appname.mask.v1); tags pinned by the round-trip test below.
        AssetKind::FontFace => "fontFace",
        AssetKind::MaskPlane => "maskPlane",
    }
}

fn kind_from_tag(tag: &str) -> Option<AssetKind> {
    match tag {
        "samples" => Some(AssetKind::Samples),
        "rasterImage" => Some(AssetKind::RasterImage),
        "fontFace" => Some(AssetKind::FontFace),
        "maskPlane" => Some(AssetKind::MaskPlane),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// RFC 6901 JSON Pointer (the §26.1 `parameterized` mechanism) — resolve
// and write. Dependency-free by design (no new externals, preamble rule 6).
// ---------------------------------------------------------------------------

/// Split an RFC 6901 pointer into decoded reference tokens
/// (`~1` -> `/`, `~0` -> `~`, applied in that order).
fn pointer_tokens(pointer: &str) -> Option<Vec<String>> {
    if pointer.is_empty() {
        return Some(Vec::new());
    }
    let rest = pointer.strip_prefix('/')?;
    Some(
        rest.split('/')
            .map(|t| t.replace("~1", "/").replace("~0", "~"))
            .collect(),
    )
}

/// Set `value` at `pointer` inside `doc`. Every token must resolve to an
/// EXISTING location (§26.1: "a pointer that does not resolve at replay is
/// a step error") — substitution replaces recorded values, it never grows
/// the document.
pub(crate) fn pointer_set(doc: &mut Value, pointer: &str, value: Value) -> Result<(), String> {
    let tokens = pointer_tokens(pointer)
        .ok_or_else(|| format!("{pointer:?} is not an RFC 6901 JSON Pointer"))?;
    if tokens.is_empty() {
        return Err("the whole-document pointer \"\" cannot be substituted".into());
    }
    let mut cur = doc;
    for (i, token) in tokens.iter().enumerate() {
        let last = i + 1 == tokens.len();
        match cur {
            Value::Object(map) => {
                let slot = map.get_mut(token).ok_or_else(|| {
                    format!("pointer {pointer:?} does not resolve (no member {token:?})")
                })?;
                if last {
                    *slot = value;
                    return Ok(());
                }
                cur = slot;
            }
            Value::Array(arr) => {
                let idx: usize = token.parse().map_err(|_| {
                    format!(
                        "pointer {pointer:?} does not resolve ({token:?} is not an array index)"
                    )
                })?;
                let len = arr.len();
                let slot = arr.get_mut(idx).ok_or_else(|| {
                    format!("pointer {pointer:?} does not resolve (index {idx} out of bounds, len {len})")
                })?;
                if last {
                    *slot = value;
                    return Ok(());
                }
                cur = slot;
            }
            other => {
                return Err(format!(
                    "pointer {pointer:?} does not resolve ({token:?} into non-container {other})"
                ))
            }
        }
    }
    unreachable!("loop returns on the last token");
}

// ---------------------------------------------------------------------------
// Minimal RFC 4648 base64 (standard alphabet, padded) — asset carriage.
// Hand-rolled to keep the crate free of new external dependencies
// (preamble rule 6); pinned by round-trip + vector tests below.
// ---------------------------------------------------------------------------

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub(crate) fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            B64[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

pub(crate) fn base64_decode(text: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some(u32::from(c - b'A')),
            b'a'..=b'z' => Some(u32::from(c - b'a') + 26),
            b'0'..=b'9' => Some(u32::from(c - b'0') + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let raw = text.as_bytes();
    if !raw.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(raw.len() / 4 * 3);
    for (i, chunk) in raw.chunks(4).enumerate() {
        let last = (i + 1) * 4 == raw.len();
        let pads = chunk.iter().filter(|&&c| c == b'=').count();
        if pads > 2 || (pads > 0 && !last) {
            return None;
        }
        // '=' only as trailing padding.
        if chunk[..4 - pads].contains(&b'=') {
            return None;
        }
        let mut n: u32 = 0;
        for &c in &chunk[..4 - pads] {
            n = (n << 6) | val(c)?;
        }
        n <<= 6 * pads as u32;
        out.push((n >> 16) as u8);
        if pads < 2 {
            out.push((n >> 8) as u8);
        }
        if pads < 1 {
            out.push(n as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal(steps: Value) -> Value {
        json!({ "schema": ACTION_SCHEMA, "name": "t", "steps": steps })
    }

    #[test]
    fn parses_the_spec_example_shape() {
        // The §26.1 worked example, verbatim structure (commands carry no
        // id — replay mints one; SCRIPT-D4).
        let a = Action::from_value(json!({
            "schema": "appname.action.v1",
            "name": "Web export 1200px",
            "steps": [
                { "command": { "schema": "appname.command.v1", "kind": "doc.resize",
                               "params": { "longestEdge": 1200, "resample": "lanczos" } } },
                { "stop": { "message": "Check the crop", "allowContinue": true } },
                { "command": { "schema": "appname.command.v1", "kind": "adjustment.add",
                               "params": { "opKind": "adjustment.curves", "points": [[0,0],[1,1]] } },
                  "enabled": true, "parameterized": ["/params/points"] }
            ]
        }))
        .expect("spec example parses");
        assert_eq!(a.name, "Web export 1200px");
        assert_eq!(a.steps.len(), 3);
        assert_eq!(a.command_step_count(), 2);
        assert!(
            matches!(&a.steps[1], Step::Stop(s) if s.message == "Check the crop" && s.allow_continue)
        );
        let Step::Command(c) = &a.steps[2] else {
            panic!("step 2 is a command")
        };
        assert_eq!(c.parameterized, vec!["/params/points"]);
        assert!(c.enabled);
    }

    #[test]
    fn round_trips_through_to_json() {
        let a = Action::from_value(json!({
            "schema": ACTION_SCHEMA,
            "id": "3fa5be3e-7292-4a10-9c2b-000000000001",
            "name": "rt",
            "steps": [
                { "command": { "kind": "fill", "params": { "content": "color" } },
                  "recordedTarget": { "layerName": "Background", "treeIndex": 1 } },
                { "stop": { "message": "look", "allowContinue": false }, "enabled": false }
            ],
            "assets": []
        }))
        .expect("parses");
        let b = Action::from_json(&a.to_json()).expect("re-parses");
        assert_eq!(a, b);
    }

    /// The action-wire asset-kind tags, pinned (ratification pin for the
    /// door-slice-2 carriage arms): total round-trip over every
    /// `AssetKind`, camelCase per the `rasterImage` precedent. A new
    /// variant refuses to compile past `kind_tag`'s exhaustive match
    /// until it gets a wire tag — and this pin then locks the spelling.
    #[test]
    fn asset_kind_tags_round_trip_totally() {
        for kind in [
            AssetKind::Samples,
            AssetKind::RasterImage,
            AssetKind::FontFace,
            AssetKind::MaskPlane,
        ] {
            assert_eq!(kind_from_tag(kind_tag(kind)), Some(kind), "{kind:?}");
        }
        assert_eq!(kind_tag(AssetKind::FontFace), "fontFace");
        assert_eq!(kind_tag(AssetKind::MaskPlane), "maskPlane");
        assert_eq!(kind_from_tag("FontFace"), None, "tags are exact");
    }

    #[test]
    fn exactly_two_step_types() {
        for bad in [
            json!([{}]),                                                            // neither
            json!([{ "command": { "kind": "fill" }, "stop": { "message": "x" } }]), // both
        ] {
            let e = Action::from_value(minimal(bad)).unwrap_err();
            assert_eq!(e.code, ErrorCode::WrongSchema);
            assert!(e.message.contains("exactly one of"), "{}", e.message);
        }
    }

    #[test]
    fn typed_rejects() {
        // Missing name.
        let e = Action::from_value(json!({ "schema": ACTION_SCHEMA, "steps": [] })).unwrap_err();
        assert_eq!(e.code, ErrorCode::MissingField);
        // Wrong schema const.
        let e =
            Action::from_value(json!({ "schema": "appname.action.v2", "name": "x", "steps": [] }))
                .unwrap_err();
        assert_eq!(e.code, ErrorCode::WrongSchema);
        // Bad pointer syntax.
        let e = Action::from_value(minimal(json!([
            { "command": { "kind": "fill" }, "parameterized": ["params/points"] }
        ])))
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::OutOfRange);
        // Stop without message.
        let e = Action::from_value(minimal(json!([{ "stop": {} }]))).unwrap_err();
        assert_eq!(e.code, ErrorCode::MissingField);
        // Command without kind.
        let e = Action::from_value(minimal(json!([{ "command": { "params": {} } }]))).unwrap_err();
        assert_eq!(e.code, ErrorCode::MissingField);
    }

    #[test]
    fn asset_bytes_must_hash_to_their_recorded_id() {
        let bytes = b"hello asset";
        let good = appname_core::engine::assets::asset_id_for(bytes);
        let doc = json!({
            "schema": ACTION_SCHEMA, "name": "a", "steps": [],
            "assets": [ { "id": good.to_string(), "kind": "samples",
                          "base64": base64_encode(bytes) } ]
        });
        assert_eq!(
            Action::from_value(doc)
                .expect("good id parses")
                .assets
                .len(),
            1
        );
        let doc = json!({
            "schema": ACTION_SCHEMA, "name": "a", "steps": [],
            "assets": [ { "id": Uuid::nil().to_string(), "kind": "samples",
                          "base64": base64_encode(bytes) } ]
        });
        let e = Action::from_value(doc).unwrap_err();
        assert!(e.message.contains("do not hash"), "{}", e.message);
    }

    #[test]
    fn base64_vectors_and_round_trip() {
        // RFC 4648 §10 vectors.
        for (plain, enc) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64_encode(plain.as_bytes()), enc);
            assert_eq!(base64_decode(enc).unwrap(), plain.as_bytes());
        }
        let all: Vec<u8> = (0u8..=255).collect();
        assert_eq!(base64_decode(&base64_encode(&all)).unwrap(), all);
        for bad in ["A", "AB=C", "====", "Zg==Zg=="] {
            assert!(base64_decode(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn pointer_set_replaces_existing_locations_only() {
        let mut doc = json!({ "params": { "points": [[0,0],[1,1]], "n": 3 } });
        pointer_set(&mut doc, "/params/n", json!(9)).expect("object member");
        pointer_set(&mut doc, "/params/points/1", json!([2, 2])).expect("array index");
        assert_eq!(
            doc,
            json!({ "params": { "points": [[0,0],[2,2]], "n": 9 } })
        );
        assert!(pointer_set(&mut doc, "/params/missing", json!(1)).is_err());
        assert!(pointer_set(&mut doc, "/params/points/7", json!(1)).is_err());
        assert!(pointer_set(&mut doc, "", json!(1)).is_err());
        assert!(pointer_set(&mut doc, "params/n", json!(1)).is_err());
        // ~-escapes decode per RFC 6901.
        let mut esc = json!({ "a/b": { "c~d": 1 } });
        pointer_set(&mut esc, "/a~1b/c~0d", json!(2)).expect("escaped tokens");
        assert_eq!(esc, json!({ "a/b": { "c~d": 2 } }));
    }
}
