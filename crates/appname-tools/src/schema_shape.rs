// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §16.1 ("params are camelCase JSON validated against the
//! filter's schema before execution — §10.2 discipline") + §17.1 (the
//! adjustment param schemas); the O-INT 2026-08-17 strictness ruling
//! (O-UI probe filing, reports/O-UI-log.md @ 48eacd8): **the registered
//! schema is the contract surface — validate strictly per its letter;
//! unknown keys refuse typed per `additionalProperties: false`.** Silent
//! tolerance breaks the adversarial discipline (a typo'd param must
//! never no-op with ok=true).
//!
//! This module is the ONE shape checker both registries drive their
//! strictness from (`appname-filters` and `appname-adjust` both depend
//! on this crate; each maps [`ShapeViolation`] onto its own typed error
//! and runs the check BEFORE its hand validator via the trait-provided
//! `validate_strict`). It enforces the STRUCTURAL letter of the
//! registered schema:
//!
//! * `additionalProperties: false` — unknown keys reject, recursively
//!   (nested objects like the levels tuples and hue/sat bands declare
//!   it too);
//! * `required` — missing keys reject;
//! * leaf `type` (`object` / `array` / `integer` / `number` /
//!   `boolean` / `string`) — a typo'd TYPE on a known key is the same
//!   silent-no-op class as an unknown key (e.g. `monochromatic: "yes"`
//!   previously defaulted to `false` with ok=true) and rejects here;
//! * `enum` membership;
//! * `$ref: "#/$defs/…"` resolution against the schema ROOT (the
//!   registry self-check test asserts every registered `$ref`
//!   resolves).
//!
//! What it deliberately does NOT re-implement (stays in the per-op hand
//! validators, which the audit verified against each schema): numeric
//! RANGES (minimum/maximum — incl. the recorded §40.1 non-finite
//! policies, unreachable from JSON text but live for programmatic
//! `Value`s), array length rows, and every cross-field rule (the
//! liquify disp-length product, the custom-kernel shape). This is not a
//! JSON-Schema engine election (deps/jsonschema.yaml stays CANDIDATE,
//! O-CORE's call); it is the minimal own checker that closes the ruled
//! drift class mechanically for every current and future registrant.
//!
//! Pin (recorded): `integer` here means a JSON value serde reads as
//! i64/u64 — a float-typed `2.0` rejects, matching every existing hand
//! validator (`as_i64`), and diverging (strictly) from the JSON-Schema
//! spec's multiple-of-one reading.

use serde_json::Value;

/// One structural violation, with the JSON-path of the offending spot.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeViolation {
    /// Dotted instance path (`params`, `params.composite`, …).
    pub path: String,
    /// Human-readable detail naming the schema rule.
    pub detail: String,
}

impl core::fmt::Display for ShapeViolation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{} at {}", self.detail, self.path)
    }
}

fn violation(path: &str, detail: impl Into<String>) -> ShapeViolation {
    ShapeViolation {
        path: path.to_string(),
        detail: detail.into(),
    }
}

/// Resolve `#/$defs/NAME` references against the schema root,
/// ITERATIVELY: a `$defs` entry that is itself a `$ref` follows through
/// (reviewer minor 3 — a single-strip resolve handed the bare ref
/// object to the walker, which then validated ANYTHING silently). A
/// chain longer than the `$defs` table (a cycle) fails loudly like the
/// unsupported-form arm.
fn resolve<'a>(root: &'a Value, node: &'a Value) -> Result<&'a Value, ShapeViolation> {
    let mut cur = node;
    // Cycle guard: any acyclic chain visits each $defs entry at most
    // once, so defs-table-size + 1 hops means a loop.
    let max_hops = root
        .get("$defs")
        .and_then(Value::as_object)
        .map_or(1, |d| d.len() + 1);
    for _ in 0..max_hops {
        let Some(r) = cur.get("$ref").and_then(Value::as_str) else {
            return Ok(cur);
        };
        let name = r.strip_prefix("#/$defs/").ok_or_else(|| {
            violation(
                "$schema",
                format!("unsupported $ref form {r:?} (only #/$defs/… is used)"),
            )
        })?;
        cur = root.get("$defs").and_then(|d| d.get(name)).ok_or_else(|| {
            violation(
                "$schema",
                format!("$ref {r:?} does not resolve against the schema root"),
            )
        })?;
    }
    Err(violation(
        "$schema",
        "$ref chain does not terminate (cycle in $defs)",
    ))
}

fn check_node(
    root: &Value,
    schema: &Value,
    value: &Value,
    path: &str,
) -> Result<(), ShapeViolation> {
    let schema = resolve(root, schema)?;
    let ty = schema.get("type").and_then(Value::as_str);

    // Enum membership first (enums may omit "type").
    if let Some(allowed) = schema.get("enum").and_then(Value::as_array) {
        if !allowed.contains(value) {
            return Err(violation(
                path,
                format!("value {value} outside the schema enum (letter of the registered schema)"),
            ));
        }
        return Ok(());
    }

    let is_object_schema = ty == Some("object") || schema.get("properties").is_some();
    if is_object_schema {
        let Some(obj) = value.as_object() else {
            return Err(violation(path, "must be an object (schema type)"));
        };
        let props = schema.get("properties").and_then(Value::as_object);
        // additionalProperties: false — the ruled default surface.
        if schema.get("additionalProperties") == Some(&Value::Bool(false)) {
            for k in obj.keys() {
                if props.is_none_or(|p| !p.contains_key(k)) {
                    return Err(violation(
                        path,
                        format!(
                            "unknown key {k:?} (additionalProperties: false — the registered \
                             schema is the contract surface; strictness ruling 2026-08-17)"
                        ),
                    ));
                }
            }
        }
        if let Some(req) = schema.get("required").and_then(Value::as_array) {
            for r in req.iter().filter_map(Value::as_str) {
                if !obj.contains_key(r) {
                    return Err(violation(path, format!("required key {r:?} missing")));
                }
            }
        }
        if let Some(props) = props {
            for (k, sub) in props {
                if let Some(v) = obj.get(k) {
                    check_node(root, sub, v, &format!("{path}.{k}"))?;
                }
            }
        }
        return Ok(());
    }

    match ty {
        Some("array") => {
            let Some(arr) = value.as_array() else {
                return Err(violation(path, "must be an array (schema type)"));
            };
            if let Some(items) = schema.get("items") {
                for (i, v) in arr.iter().enumerate() {
                    check_node(root, items, v, &format!("{path}[{i}]"))?;
                }
            }
        }
        Some("integer") if value.as_i64().is_none() && value.as_u64().is_none() => {
            return Err(violation(path, "must be an integer (schema type)"));
        }
        Some("number") if !value.is_number() => {
            return Err(violation(path, "must be a number (schema type)"));
        }
        Some("boolean") if !value.is_boolean() => {
            return Err(violation(path, "must be a boolean (schema type)"));
        }
        Some("string") if !value.is_string() => {
            return Err(violation(path, "must be a string (schema type)"));
        }
        _ => {} // typed leaf holds, or untyped: nothing structural
    }
    Ok(())
}

/// Check `params` against the registered schema's structural letter
/// (module doc). The first violation is returned (deterministic:
/// serde_json maps iterate in sorted key order and the walk is
/// depth-first).
pub fn check_shape(schema_text: &str, params: &Value) -> Result<(), ShapeViolation> {
    let schema: Value = serde_json::from_str(schema_text).map_err(|e| {
        violation(
            "$schema",
            format!("registered param_schema is not valid JSON: {e}"),
        )
    })?;
    check_node(&schema, &schema, params, "params")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const S: &str = r##"{"type":"object","additionalProperties":false,"required":["a"],
        "properties":{
            "a":{"type":"integer"},
            "b":{"type":"boolean"},
            "e":{"enum":["x","y"]},
            "nest":{"$ref":"#/$defs/t"},
            "list":{"type":"array","items":{"type":"number"}}},
        "$defs":{"t":{"type":"object","additionalProperties":false,
            "properties":{"inner":{"type":"number"}}}}}"##;

    #[test]
    fn accepts_the_letter() {
        let p = json!({"a": 3, "b": true, "e": "x", "nest": {"inner": 0.5}, "list": [1, 2.5]});
        assert_eq!(check_shape(S, &p), Ok(()));
        assert_eq!(check_shape(S, &json!({"a": 1})), Ok(()));
    }

    #[test]
    fn unknown_keys_reject_at_every_depth() {
        let err = check_shape(S, &json!({"a": 1, "bogus": 1})).unwrap_err();
        assert_eq!(err.path, "params");
        assert!(err.detail.contains("bogus"));
        let err = check_shape(S, &json!({"a": 1, "nest": {"typo": 1}})).unwrap_err();
        assert_eq!(err.path, "params.nest");
        assert!(err.detail.contains("typo"));
    }

    #[test]
    fn required_types_and_enums_hold() {
        assert!(check_shape(S, &json!({}))
            .unwrap_err()
            .detail
            .contains("required"));
        assert!(check_shape(S, &json!({"a": 1.5}))
            .unwrap_err()
            .detail
            .contains("integer"));
        assert!(check_shape(S, &json!({"a": 1, "b": "yes"}))
            .unwrap_err()
            .detail
            .contains("boolean"));
        assert!(check_shape(S, &json!({"a": 1, "e": "z"}))
            .unwrap_err()
            .detail
            .contains("enum"));
        let err = check_shape(S, &json!({"a": 1, "list": [1, "x"]})).unwrap_err();
        assert_eq!(err.path, "params.list[1]");
    }

    #[test]
    fn chained_refs_resolve_and_enforce_the_target() {
        // Reviewer probe (minor 3): a $defs entry that is itself a $ref
        // must follow through to the real target — a wrong-typed value
        // against integer-via-chained-ref REFUSES (the single-strip
        // resolve silently accepted anything here).
        let chained = r##"{"type":"object","additionalProperties":false,
            "properties":{"n":{"$ref":"#/$defs/alias"}},
            "$defs":{"alias":{"$ref":"#/$defs/real"},
                      "real":{"type":"integer"}}}"##;
        assert_eq!(check_shape(chained, &json!({"n": 3})), Ok(()));
        let err = check_shape(chained, &json!({"n": "not an int"})).unwrap_err();
        assert_eq!(err.path, "params.n");
        assert!(err.detail.contains("integer"), "{err}");
        // A $defs cycle fails loudly, never loops or passes silently.
        let cyclic = r##"{"type":"object",
            "properties":{"n":{"$ref":"#/$defs/a"}},
            "$defs":{"a":{"$ref":"#/$defs/b"},"b":{"$ref":"#/$defs/a"}}}"##;
        let err = check_shape(cyclic, &json!({"n": 1})).unwrap_err();
        assert!(err.detail.contains("cycle"), "{err}");
    }

    #[test]
    fn dangling_refs_and_bad_schema_are_loud() {
        assert!(check_shape("not json", &json!({})).is_err());
        let bad = r##"{"type":"object","properties":{"n":{"$ref":"#/$defs/gone"}}}"##;
        assert!(check_shape(bad, &json!({"n": 1}))
            .unwrap_err()
            .detail
            .contains("does not resolve"));
    }
}
