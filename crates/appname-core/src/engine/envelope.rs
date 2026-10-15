// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §10.2 (`appname.command.v1` envelope; the mirror artifact is
//! `contracts/schemas/command-envelope.v1.json`), §9.6 (command-as-JSON
//! boundary — 1 MiB cap). Ported from `appname-proto` `envelope.rs` (same
//! authorship; the shape survived the REPORT-A 41-case suite and the
//! CP-0.1 adversarial corpus is generated against it).
//!
//! Input is validated field-by-field (not via serde derive) so rejections
//! carry the taxonomy's structured error codes. Required: `schema` (const),
//! `id` (uuid, non-nil), `kind`. Everything else is optional on the wire.

use serde::Serialize;
use serde_json::{Map, Value};
use uuid::Uuid;

use super::error::{EngineError, ErrorCode};

/// The `schema` const every envelope must carry.
pub const ENVELOPE_SCHEMA: &str = "appname.command.v1";

/// ES §9.6: command documents above this size are rejected up front.
pub const MAX_ENVELOPE_BYTES: usize = 1024 * 1024;

/// The envelope `target` block. At CP-0.1 no implemented kind resolves a
/// target (`doc.new` ignores it per its contract); the block is validated
/// and carried for the kinds that will.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub layer_id: Option<Uuid>,
    pub mask_id: Option<Uuid>,
    pub channel_id: Option<Uuid>,
}

/// A validated `appname.command.v1` envelope.
///
/// Serialization is the journal/package wire form; field order and
/// `serde_json`'s object handling make it deterministic.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Envelope {
    pub schema: String,
    pub id: Uuid,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<Target>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    /// AssetId of binary stroke samples (never inline, ES §10.2). Stage 1
    /// carries any string value; consumers answer for it at dispatch and
    /// later stages (review blocker 1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub samples: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge_key: Option<String>,
    pub reversible: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
}

impl Envelope {
    /// Convenience constructor for callers building envelopes in code.
    /// `params` may be `Value::Null` for zero-param kinds.
    pub fn new(id: Uuid, kind: &str, params: Value) -> Self {
        Envelope {
            schema: ENVELOPE_SCHEMA.to_string(),
            id,
            kind: kind.to_string(),
            target: None,
            params: if params.is_null() { None } else { Some(params) },
            samples: None,
            merge_key: None,
            reversible: true,
            timestamp: None,
        }
    }

    /// Parse and validate an envelope from raw JSON text (stage 0 + stage 1
    /// of the pinned validation order — see `engine::apply_json`).
    pub fn from_json(text: &str) -> Result<Envelope, EngineError> {
        if text.len() > MAX_ENVELOPE_BYTES {
            return Err(EngineError::new(
                ErrorCode::InvalidArg,
                format!(
                    "command document is {} bytes; the cap is 1 MiB (ES 9.6)",
                    text.len()
                ),
            ));
        }
        let value: Value = serde_json::from_str(text)
            .map_err(|e| EngineError::new(ErrorCode::InvalidArg, format!("invalid JSON: {e}")))?;
        Self::from_value(value)
    }

    /// Validate an envelope from an already-parsed JSON value (stage 1).
    pub fn from_value(value: Value) -> Result<Envelope, EngineError> {
        let obj = match value {
            Value::Object(m) => m,
            other => {
                return Err(EngineError::new(
                    ErrorCode::InvalidArg,
                    format!("envelope must be a JSON object, got {}", type_name(&other)),
                ))
            }
        };

        const KNOWN: [&str; 8] = [
            "schema",
            "id",
            "kind",
            "target",
            "params",
            "samples",
            "mergeKey",
            "reversible",
        ];
        for key in obj.keys() {
            if !KNOWN.contains(&key.as_str()) && key != "timestamp" {
                return Err(EngineError::range(
                    key,
                    format!("unknown envelope field `{key}`"),
                ));
            }
        }

        // schema — required const.
        let schema = match obj.get("schema") {
            None => return Err(EngineError::missing("schema")),
            Some(Value::String(s)) => s.clone(),
            Some(v) => {
                return Err(EngineError::range(
                    "schema",
                    format!("`schema` must be a string, got {}", type_name(v)),
                ))
            }
        };
        if schema != ENVELOPE_SCHEMA {
            return Err(EngineError::with_field(
                ErrorCode::WrongSchema,
                "schema",
                format!("expected \"{ENVELOPE_SCHEMA}\", got \"{schema}\""),
            ));
        }

        // id — required uuid, non-nil.
        let id = match obj.get("id") {
            None => return Err(EngineError::missing("id")),
            Some(Value::String(s)) => Uuid::parse_str(s).map_err(|_| {
                EngineError::range("id", format!("`id` is not a valid UUID: \"{s}\""))
            })?,
            Some(v) => {
                return Err(EngineError::range(
                    "id",
                    format!("`id` must be a UUID string, got {}", type_name(v)),
                ))
            }
        };
        if id.is_nil() {
            return Err(EngineError::with_field(
                ErrorCode::DuplicateId,
                "id",
                "the nil UUID is not a valid command id",
            ));
        }

        // kind — required string.
        let kind = match obj.get("kind") {
            None => return Err(EngineError::missing("kind")),
            Some(Value::String(s)) if !s.is_empty() => s.clone(),
            Some(Value::String(_)) => {
                return Err(EngineError::range("kind", "`kind` must be non-empty"))
            }
            Some(v) => {
                return Err(EngineError::range(
                    "kind",
                    format!("`kind` must be a string, got {}", type_name(v)),
                ))
            }
        };

        // target — optional object of three nullable uuids.
        let target = match obj.get("target") {
            None | Some(Value::Null) => None,
            Some(Value::Object(t)) => Some(parse_target(t)?),
            Some(v) => {
                return Err(EngineError::range(
                    "target",
                    format!("`target` must be an object, got {}", type_name(v)),
                ))
            }
        };

        // params — optional object.
        let params = match obj.get("params") {
            None | Some(Value::Null) => None,
            Some(v @ Value::Object(_)) => Some(v.clone()),
            Some(v) => {
                return Err(EngineError::range(
                    "params",
                    format!("`params` must be an object, got {}", type_name(v)),
                ))
            }
        };

        // samples — nullable assetId string (§10.2 wire shape). Stage 1
        // validates the TYPE only and carries the value: the seven stroke
        // kinds' contract envelopes legitimately carry a samples assetId,
        // and dispatch (stage 4) must be the one to answer for them
        // (`not_implemented` while unimplemented; an implemented consumer
        // may refuse unsupported samples at its own stage — review
        // blocker 1 / P-A14 hedge).
        let samples = match obj.get("samples") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()),
            Some(v) => {
                return Err(EngineError::range(
                    "samples",
                    format!("`samples` must be a string or null, got {}", type_name(v)),
                ))
            }
        };

        let merge_key = match obj.get("mergeKey") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()),
            Some(v) => {
                return Err(EngineError::range(
                    "mergeKey",
                    format!("`mergeKey` must be a string or null, got {}", type_name(v)),
                ))
            }
        };

        let reversible = match obj.get("reversible") {
            None => true,
            Some(Value::Bool(b)) => *b,
            Some(v) => {
                return Err(EngineError::range(
                    "reversible",
                    format!("`reversible` must be a boolean, got {}", type_name(v)),
                ))
            }
        };

        let timestamp = match obj.get("timestamp") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()),
            Some(v) => {
                return Err(EngineError::range(
                    "timestamp",
                    format!("`timestamp` must be a string, got {}", type_name(v)),
                ))
            }
        };

        Ok(Envelope {
            schema,
            id,
            kind,
            target,
            params,
            samples,
            merge_key,
            reversible,
            timestamp,
        })
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("envelope serialization cannot fail")
    }

    /// Params as an object map, when present.
    pub fn params_map(&self) -> Option<&Map<String, Value>> {
        match &self.params {
            Some(Value::Object(m)) => Some(m),
            _ => None,
        }
    }
}

fn parse_target(t: &Map<String, Value>) -> Result<Target, EngineError> {
    for key in t.keys() {
        if !["layerId", "maskId", "channelId"].contains(&key.as_str()) {
            return Err(EngineError::range(
                key,
                format!("unknown target field `{key}`"),
            ));
        }
    }
    let get = |name: &str| -> Result<Option<Uuid>, EngineError> {
        match t.get(name) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) => Uuid::parse_str(s).map(Some).map_err(|_| {
                EngineError::range(name, format!("target.{name} is not a valid UUID: \"{s}\""))
            }),
            Some(v) => Err(EngineError::range(
                name,
                format!(
                    "target.{name} must be a UUID string or null, got {}",
                    type_name(v)
                ),
            )),
        }
    };
    Ok(Target {
        layer_id: get("layerId")?,
        mask_id: get("maskId")?,
        channel_id: get("channelId")?,
    })
}

pub(crate) fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> serde_json::Value {
        serde_json::json!({
            "schema": "appname.command.v1",
            "id": "b0000000-0000-4000-8000-00000000aaaa",
            "kind": "doc.new",
            "params": {"width": 64, "height": 64}
        })
    }

    #[test]
    fn valid_minimal_parses() {
        let e = Envelope::from_value(base()).unwrap();
        assert_eq!(e.kind, "doc.new");
        assert!(e.reversible);
        assert!(e.target.is_none());
    }

    // The envelope-stage corpus expectations
    // (fixtures/adversarial/generated/envelope_rejects.json), asserted at
    // the parser layer; the full-apply layer re-asserts through Engine.

    #[test]
    fn missing_schema_key_is_missing_field() {
        let mut v = base();
        v.as_object_mut().unwrap().remove("schema");
        let e = Envelope::from_value(v).unwrap_err();
        assert_eq!(e.code, ErrorCode::MissingField); // G-ENV-missing-schema-key
    }

    #[test]
    fn wrong_schema_const_is_wrong_schema() {
        let mut v = base();
        v["schema"] = "appname.command.v2".into();
        let e = Envelope::from_value(v).unwrap_err();
        assert_eq!(e.code, ErrorCode::WrongSchema); // G-ENV-schema-wrong-const
    }

    #[test]
    fn missing_id_and_kind_are_missing_field() {
        let mut v = base();
        v.as_object_mut().unwrap().remove("id");
        assert_eq!(
            Envelope::from_value(v).unwrap_err().code,
            ErrorCode::MissingField
        ); // G-ENV-missing-id-key
        let mut v = base();
        v.as_object_mut().unwrap().remove("kind");
        assert_eq!(
            Envelope::from_value(v).unwrap_err().code,
            ErrorCode::MissingField
        ); // G-ENV-missing-kind-key
    }

    #[test]
    fn id_not_uuid_is_out_of_range() {
        let mut v = base();
        v["id"] = "not-a-uuid".into();
        assert_eq!(
            Envelope::from_value(v).unwrap_err().code,
            ErrorCode::OutOfRange
        ); // G-ENV-id-not-uuid
    }

    #[test]
    fn nil_id_is_duplicate_id() {
        let mut v = base();
        v["id"] = "00000000-0000-0000-0000-000000000000".into();
        assert_eq!(
            Envelope::from_value(v).unwrap_err().code,
            ErrorCode::DuplicateId
        );
    }

    #[test]
    fn kind_not_string_is_out_of_range() {
        let mut v = base();
        v["kind"] = 7.into();
        assert_eq!(
            Envelope::from_value(v).unwrap_err().code,
            ErrorCode::OutOfRange
        ); // G-ENV-kind-not-string
    }

    #[test]
    fn params_not_object_is_out_of_range() {
        let mut v = base();
        v["params"] = 7.into();
        assert_eq!(
            Envelope::from_value(v).unwrap_err().code,
            ErrorCode::OutOfRange
        ); // G-ENV-params-not-object
    }

    #[test]
    fn unknown_top_level_key_is_out_of_range() {
        let mut v = base();
        v["surprise"] = true.into();
        assert_eq!(
            Envelope::from_value(v).unwrap_err().code,
            ErrorCode::OutOfRange
        ); // G-ENV-unknown-top-level-key
    }

    #[test]
    fn oversized_document_is_invalid_arg() {
        let big = format!(
            "{{\"schema\":\"appname.command.v1\",\"id\":\"b0000000-0000-4000-8000-00000000aaaa\",\"kind\":\"doc.new\",\"params\":{{\"width\":1,\"height\":1,\"pad\":\"{}\"}}}}",
            "x".repeat(MAX_ENVELOPE_BYTES)
        );
        let e = Envelope::from_json(&big).unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidArg); // ES §9.6 cap
    }

    #[test]
    fn invalid_json_is_invalid_arg() {
        assert_eq!(
            Envelope::from_json("{not json").unwrap_err().code,
            ErrorCode::InvalidArg
        );
        assert_eq!(
            Envelope::from_json("[1,2,3]").unwrap_err().code,
            ErrorCode::InvalidArg
        );
    }

    #[test]
    fn target_block_validates_uuids_and_keys() {
        let mut v = base();
        v["target"] = serde_json::json!({"layerId": "b0000000-0000-4000-8000-00000000bbbb"});
        let e = Envelope::from_value(v).unwrap();
        assert!(e.target.unwrap().layer_id.is_some());

        let mut v = base();
        v["target"] = serde_json::json!({"layerId": "nope"});
        assert_eq!(
            Envelope::from_value(v).unwrap_err().code,
            ErrorCode::OutOfRange
        );

        let mut v = base();
        v["target"] = serde_json::json!({"strayId": null});
        assert_eq!(
            Envelope::from_value(v).unwrap_err().code,
            ErrorCode::OutOfRange
        );
    }

    #[test]
    fn samples_string_is_carried_and_wrong_type_rejects() {
        // Review blocker 1: §10.2 declares samples string|null valid wire
        // shape — stage 1 carries the value instead of preempting dispatch.
        let mut v = base();
        v["samples"] = "00000000-0001-4000-8000-000000000006".into();
        let e = Envelope::from_value(v).unwrap();
        assert_eq!(
            e.samples.as_deref(),
            Some("00000000-0001-4000-8000-000000000006")
        );

        // null stays None.
        let mut v = base();
        v["samples"] = serde_json::Value::Null;
        assert!(Envelope::from_value(v).unwrap().samples.is_none());

        // Wrong TYPE still rejects at stage 1.
        let mut v = base();
        v["samples"] = 123.into();
        let err = Envelope::from_value(v).unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);
        assert_eq!(err.field.as_deref(), Some("samples"));
    }

    #[test]
    fn wire_round_trip_is_stable() {
        let e = Envelope::from_value(base()).unwrap();
        let text = e.to_json();
        let back = Envelope::from_json(&text).unwrap();
        assert_eq!(back, e);
    }
}
