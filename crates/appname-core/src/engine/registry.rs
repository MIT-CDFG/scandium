// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §10.2 (versioned command JSON schemas), §10.3 (the command
//! catalog — 102 kinds); plan §2.1 row "Command engine".
//!
//! The registry is **mechanically generated** from the frozen contracts
//! mirror (`contracts/commands/`): [`super::registry_gen`] is emitted by
//! `crates/appname-core/tools/gen_command_registry.py` (no hand-typed kind
//! list — rule-8 spirit). Standing checkpoint gates in this module's tests:
//!
//! * three-way equality: registry == `_index.yaml` == 102;
//! * source drift: every contract file re-fingerprinted at test time against
//!   the generation-time fingerprints (a mirror update without regeneration
//!   fails the test naming the regeneration command).
//!
//! Param validation is a purpose-built interpreter over each kind's embedded
//! `paramsSchema` (JSON Schema subset actually used by the contracts:
//! object / required / properties / integer / number / string / boolean /
//! enum / minimum / maximum / format:uuid / additionalProperties:false /
//! top-level `oneOf` of required-alternatives incl. the `not:{required}`
//! mutual-exclusion form — fix-round M4).
//! The `jsonschema` crate the ES sketches (§1.3) has no deps/ record at
//! CP-0.1 — recorded as an open question; the interpreter is the
//! deps-neutral stand-in and covers every shape `doc.new` needs. Codes
//! follow the REPORT-A-proven taxonomy (binding arbitration OQ-2): wrong
//! type / enum miss / bounds / unknown key → `error.schema.out_of_range`;
//! absent required → `error.schema.missing_field`.

use std::collections::HashMap;
use std::sync::LazyLock;

use serde_json::{Map, Value};

use super::error::EngineError;
#[cfg(test)]
use super::registry_gen::SOURCE_FINGERPRINTS;
use super::registry_gen::{COMMANDS, COMMAND_COUNT};

/// One generated catalog entry (fields from the `_index.yaml` row + the
/// per-kind package's `paramsSchema`/`defaults` as canonical JSON).
pub struct CommandSpec {
    pub kind: &'static str,
    pub family: &'static str,
    pub file: &'static str,
    /// §10.3 class letter: "R" | "M" | "C".
    pub class: &'static str,
    /// †-kind executing outside the undo stack (§10.3/§10.4).
    pub dagger: bool,
    pub inverse: Option<&'static str>,
    pub milestone: &'static str,
    pub params_schema: &'static str,
    pub defaults: &'static str,
    /// The contract's `examples.validMinimal` envelope as canonical JSON —
    /// the shape corpus not_implemented cases are generated from (stroke
    /// kinds legitimately carry a `samples` assetId; review blocker 1).
    pub valid_minimal: &'static str,
}

/// Catalog lookup (index order preserved from `_index.yaml`).
pub fn lookup(kind: &str) -> Option<&'static CommandSpec> {
    COMMANDS.iter().find(|c| c.kind == kind)
}

/// Per-kind `paramsSchema`, parsed ONCE (O-WATCH #1 MINOR-4): the embedded
/// canonical-JSON strings deserialize a single time at first use; every
/// apply of every kind shares the same `'static` parsed value thereafter.
static PARSED_SCHEMAS: LazyLock<HashMap<&'static str, Value>> = LazyLock::new(|| {
    COMMANDS
        .iter()
        .map(|c| {
            let v: Value = serde_json::from_str(c.params_schema)
                .expect("generated params_schema is valid JSON (generator invariant)");
            (c.kind, v)
        })
        .collect()
});

/// The shared parsed schema for a registered kind.
pub fn parsed_schema(kind: &str) -> Option<&'static Value> {
    PARSED_SCHEMAS.get(kind)
}

pub fn kinds() -> impl Iterator<Item = &'static str> {
    COMMANDS.iter().map(|c| c.kind)
}

pub const fn count() -> usize {
    COMMAND_COUNT
}

/// FNV-1a 64 over raw bytes — drift-test fingerprint only, implemented
/// identically in `tools/gen_command_registry.py`. Never an identity
/// (BLAKE3 is the one content identity, ES §0.3).
#[cfg(test)]
pub(crate) fn fnv1a64(data: &[u8]) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01B3);
    }
    h
}

/// Validate `params` against a kind's generated `paramsSchema` and return
/// the effective map with schema defaults filled in (ES §10.2 stage-3).
///
/// Pinned micro-order inside this stage (OQ-3 record):
/// 1. `params` must be an object when the schema declares properties
///    (absence = empty object);
/// 2. required fields, in the schema's `required` order → `missing_field`;
/// 3. each declared property, in sorted key order (canonical JSON):
///    type / enum / bounds / format checks → `out_of_range`;
///    absent optional fields take their schema `default`;
/// 4. unknown keys last (`additionalProperties: false`) → `out_of_range`.
///
/// Non-finite policy note (§40.1): JSON itself cannot deliver NaN/Inf
/// literals (RFC 8259) and `serde_json` rejects out-of-range magnitudes at
/// parse, which surfaces as `error.api.invalid_arg` at stage 0 — so the
/// "clamp non-finite numeric params" rule is unreachable on this wire path;
/// recorded in the lane log for the write-side (journal) work.
pub fn validate_params(
    spec: &CommandSpec,
    params: Option<&Map<String, Value>>,
) -> Result<Map<String, Value>, EngineError> {
    let schema: &'static Value =
        parsed_schema(spec.kind).expect("every registry spec has a parsed schema");
    let empty = Map::new();
    let given = params.unwrap_or(&empty);

    let props = match schema.get("properties").and_then(Value::as_object) {
        Some(p) => p,
        None => {
            // Schema with no declared properties: nothing to validate at
            // this stage (target/state stages own the rest).
            return Ok(given.clone());
        }
    };

    // 2. Required fields, schema order.
    if let Some(req) = schema.get("required").and_then(Value::as_array) {
        for r in req {
            let name = r.as_str().expect("required entries are strings");
            if !given.contains_key(name) {
                return Err(EngineError::missing(name));
            }
        }
    }

    // 2b. Top-level `oneOf` of required-alternative branches (fix-round
    // M4): the contracts' exclusive-params shape — each branch is
    // `{required: [..]}`, optionally with `not: {required: [..]}` (the
    // op.set_mask/transform.warp mutual-exclusion form). Exactly one
    // branch must be satisfied; violations (none satisfied OR several) are
    // stage-3 schema violations answering `error.schema.missing_field`
    // (pinned by the history.goto/op.set_mask contract invalid cases for
    // both the neither- and the both-present payloads). Branches carrying
    // no `required` list (structured unions) stay a stage-6 concern.
    if let Some(one) = schema.get("oneOf").and_then(Value::as_array) {
        let mut alternatives: Vec<String> = Vec::new();
        let mut satisfied = 0usize;
        for branch in one {
            let req: Vec<&str> = branch
                .get("required")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            if req.is_empty() {
                continue;
            }
            alternatives.push(req.join("+"));
            let mut ok = req.iter().all(|r| given.contains_key(*r));
            if let Some(banned) = branch
                .get("not")
                .and_then(|n| n.get("required"))
                .and_then(Value::as_array)
            {
                let banned: Vec<&str> = banned.iter().filter_map(Value::as_str).collect();
                if !banned.is_empty() && banned.iter().all(|r| given.contains_key(*r)) {
                    ok = false;
                }
            }
            if ok {
                satisfied += 1;
            }
        }
        if !alternatives.is_empty() && satisfied != 1 {
            let field = alternatives.join("|");
            return Err(EngineError::with_field(
                super::error::ErrorCode::MissingField,
                field.clone(),
                format!(
                    "exactly one of the parameter alternatives ({field}) must be present \
                     (stage-3 oneOf, {} given satisfy {satisfied})",
                    given.len()
                ),
            ));
        }
    }

    // 3. Declared properties.
    let mut effective = Map::new();
    for (name, prop) in props {
        match given.get(name) {
            Some(v) => {
                check_value(name, v, prop)?;
                effective.insert(name.clone(), v.clone());
            }
            None => {
                if let Some(d) = prop.get("default") {
                    effective.insert(name.clone(), d.clone());
                }
            }
        }
    }

    // 4. Unknown keys (additionalProperties: false in every contract).
    let additional_banned = schema
        .get("additionalProperties")
        .map(|v| v == &Value::Bool(false))
        .unwrap_or(false);
    if additional_banned {
        for key in given.keys() {
            if !props.contains_key(key) {
                return Err(EngineError::range(
                    key.as_str(),
                    format!("unknown parameter `{key}` (additionalProperties: false)"),
                ));
            }
        }
    }

    Ok(effective)
}

fn check_value(name: &str, v: &Value, prop: &Value) -> Result<(), EngineError> {
    use super::envelope::type_name;

    // Enum membership (with or without a type keyword).
    if let Some(allowed) = prop.get("enum").and_then(Value::as_array) {
        if !allowed.contains(v) {
            return Err(EngineError::range(
                name,
                format!("`{name}` must be one of {allowed:?}, got {v}"),
            ));
        }
        return Ok(());
    }

    match prop.get("type").and_then(Value::as_str) {
        Some("integer") => {
            let n = match v.as_f64() {
                Some(n) if v.is_number() => n,
                _ => {
                    return Err(EngineError::range(
                        name,
                        format!("`{name}` must be an integer, got {}", type_name(v)),
                    ))
                }
            };
            if n.fract() != 0.0 {
                return Err(EngineError::range(
                    name,
                    format!("`{name}` must be an integer, got {n}"),
                ));
            }
            check_bounds(name, n, prop)
        }
        Some("number") => {
            let n = match v.as_f64() {
                Some(n) if v.is_number() => n,
                _ => {
                    return Err(EngineError::range(
                        name,
                        format!("`{name}` must be a number, got {}", type_name(v)),
                    ))
                }
            };
            check_bounds(name, n, prop)
        }
        Some("string") => {
            let s = match v.as_str() {
                Some(s) => s,
                None => {
                    return Err(EngineError::range(
                        name,
                        format!("`{name}` must be a string, got {}", type_name(v)),
                    ))
                }
            };
            if prop.get("format").and_then(Value::as_str) == Some("uuid") {
                let id = uuid::Uuid::parse_str(s).map_err(|_| {
                    EngineError::range(name, format!("`{name}` is not a valid UUID: \"{s}\""))
                })?;
                if id.is_nil() {
                    return Err(EngineError::range(
                        name,
                        format!("`{name}` must not be the nil UUID"),
                    ));
                }
            }
            // §40.1 stage-3 length rows (QA-F2 closure): minLength/maxLength
            // are documented-range violations → error.schema.out_of_range.
            // Counted in Unicode scalar values (chars), matching the
            // contracts' "1..=255 chars" wording.
            if let Some(lo) = prop.get("minLength").and_then(Value::as_u64) {
                if (s.chars().count() as u64) < lo {
                    return Err(EngineError::range(
                        name,
                        format!("`{name}` must be at least {lo} character(s) long"),
                    ));
                }
            }
            if let Some(hi) = prop.get("maxLength").and_then(Value::as_u64) {
                if (s.chars().count() as u64) > hi {
                    return Err(EngineError::range(
                        name,
                        format!("`{name}` must be at most {hi} character(s) long"),
                    ));
                }
            }
            Ok(())
        }
        Some("boolean") => match v {
            Value::Bool(_) => Ok(()),
            other => Err(EngineError::range(
                name,
                format!("`{name}` must be a boolean, got {}", type_name(other)),
            )),
        },
        Some("object") | Some("array") => {
            // Structured params (assembled kinds): the interpreter checks
            // presence/JSON-type only; the kind's stage 6 owns the
            // semantic contract (§40.5 — e.g. §22.1 color validation).
            let ok = matches!(
                (prop.get("type").and_then(Value::as_str), v),
                (Some("object"), Value::Object(_)) | (Some("array"), Value::Array(_))
            );
            if !ok {
                return Err(EngineError::range(
                    name,
                    format!("`{name}` must be a {}, got {}", prop["type"], type_name(v)),
                ));
            }
            // §40.1 stage-3 length row for arrays (QA-F4 closure):
            // minItems/maxItems are documented-range violations →
            // error.schema.out_of_range — and they fire at stage 5,
            // BEFORE any kind arm's live-state gates (the QA-F4 order).
            if let Value::Array(items) = v {
                if let Some(lo) = prop.get("minItems").and_then(Value::as_u64) {
                    if (items.len() as u64) < lo {
                        return Err(EngineError::range(
                            name,
                            format!(
                                "`{name}` must have at least {lo} item(s), got {}",
                                items.len()
                            ),
                        ));
                    }
                }
                if let Some(hi) = prop.get("maxItems").and_then(Value::as_u64) {
                    if (items.len() as u64) > hi {
                        return Err(EngineError::range(
                            name,
                            format!(
                                "`{name}` must have at most {hi} item(s), got {}",
                                items.len()
                            ),
                        ));
                    }
                }
            }
            // Property-level `oneOf` of required-alternatives on an OBJECT
            // param (QA-F5 closure): the smartobject.replace `source`
            // shape — `oneOf: [{required:[assetId]}, {required:[path]}]`.
            // Exactly one branch must be satisfied; none or both →
            // stage-3 error.schema.missing_field (contract "source-oneof-
            // both-set" row), fired BEFORE any arm-side live-state gate.
            if let (Value::Object(obj), Some(one)) =
                (v, prop.get("oneOf").and_then(Value::as_array))
            {
                let mut alternatives: Vec<String> = Vec::new();
                let mut satisfied = 0usize;
                for branch in one {
                    let req: Vec<&str> = branch
                        .get("required")
                        .and_then(Value::as_array)
                        .map(|a| a.iter().filter_map(Value::as_str).collect())
                        .unwrap_or_default();
                    if req.is_empty() {
                        continue;
                    }
                    alternatives.push(req.join("+"));
                    if req.iter().all(|r| obj.contains_key(*r)) {
                        satisfied += 1;
                    }
                }
                if !alternatives.is_empty() && satisfied != 1 {
                    let field = format!("{name}.{}", alternatives.join(&format!("|{name}.")));
                    return Err(EngineError::with_field(
                        super::error::ErrorCode::MissingField,
                        field.clone(),
                        format!(
                            "exactly one of the `{name}` alternatives ({field}) must be \
                             present (stage-3 oneOf, {satisfied} satisfied)"
                        ),
                    ));
                }
            }
            Ok(())
        }
        None if prop.get("$ref").is_some()
            || prop.get("oneOf").is_some()
            || prop.get("anyOf").is_some() =>
        {
            // $ref'd / union-typed structured params: stage 5 defers to the
            // kind's stage-6 semantic validation (documented subset; e.g.
            // fill's conditional `seed`).
            Ok(())
        }
        None if prop.get("type").map(Value::is_array).unwrap_or(false) => {
            // JSON Schema union `type: [..]` (doc.assign_profile's
            // `string|null` profile): accept when the value matches ANY
            // listed primitive; deeper constraints stay stage-6.
            let types = prop["type"].as_array().expect("checked");
            let ok = types.iter().filter_map(Value::as_str).any(|t| match t {
                "null" => v.is_null(),
                "string" => v.is_string(),
                "number" => v.is_number(),
                "integer" => v.as_i64().is_some() || v.as_u64().is_some(),
                "boolean" => v.is_boolean(),
                "object" => v.is_object(),
                "array" => v.is_array(),
                _ => false,
            });
            if ok {
                Ok(())
            } else {
                Err(EngineError::range(
                    name,
                    format!(
                        "`{name}` must match one of {}, got {}",
                        prop["type"],
                        type_name(v)
                    ),
                ))
            }
        }
        None if prop.get("type").is_none() => {
            // UNCONSTRAINED property (no type/enum/$ref/oneOf/anyOf —
            // `meta.set`'s `value`, whose per-key shapes live in the
            // contract's allOf/if-then): any JSON validates at stage 3;
            // the kind's stage-6 arm owns the per-key semantics (§40.8
            // "validates"). Door-slice-3 subset growth, recorded.
            Ok(())
        }
        other => Err(EngineError::range(
            name,
            format!("`{name}`: schema shape {other:?} is beyond the CP-0.3 validator subset"),
        )),
    }
}

fn check_bounds(name: &str, n: f64, prop: &Value) -> Result<(), EngineError> {
    if let Some(lo) = prop.get("minimum").and_then(Value::as_f64) {
        if n < lo {
            return Err(EngineError::range(
                name,
                format!("`{name}` must be >= {lo}, got {n}"),
            ));
        }
    }
    if let Some(hi) = prop.get("maximum").and_then(Value::as_f64) {
        if n > hi {
            return Err(EngineError::range(
                name,
                format!("`{name}` must be <= {hi}, got {n}"),
            ));
        }
    }
    Ok(())
}

/// Extract the `kind:` field of every command row of `_index.yaml` text.
/// Purpose-built line extractor for the machine-generated index artifact
/// (single-line flow-map rows) — used by the three-way equality test; not a
/// general YAML parser.
#[cfg(test)]
pub(crate) fn index_kinds(index_text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in index_text.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("- { kind: ") {
            if let Some(end) = rest.find(',') {
                out.push(rest[..end].trim().to_string());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    fn commands_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../contracts/commands")
    }

    /// Standing checkpoint gate (plan §2.1): the generated registry, the
    /// `_index.yaml` artifact, and the constant 102 agree exactly.
    #[test]
    fn three_way_equality_registry_equals_index_yaml_equals_102() {
        let index_text = std::fs::read_to_string(commands_dir().join("_index.yaml"))
            .expect("contracts/commands/_index.yaml readable");
        let index: BTreeSet<String> = index_kinds(&index_text).into_iter().collect();
        let registry: BTreeSet<String> = kinds().map(str::to_string).collect();

        assert_eq!(index.len(), 102, "_index.yaml kind rows");
        assert_eq!(registry.len(), 102, "registry kinds");
        assert_eq!(COMMAND_COUNT, 102, "generated COMMAND_COUNT");
        assert_eq!(COMMANDS.len(), 102, "generated table length");
        assert_eq!(index, registry, "registry kinds == _index.yaml kinds");
    }

    /// Drift gate: contracts changed without regenerating the table.
    #[test]
    fn generated_registry_is_current_with_contracts_sources() {
        for (file, fnv, len) in SOURCE_FINGERPRINTS.iter() {
            let bytes = std::fs::read(commands_dir().join(file))
                .unwrap_or_else(|e| panic!("cannot read contracts/commands/{file}: {e}"));
            assert_eq!(
                (fnv1a64(&bytes), bytes.len()),
                (*fnv, *len),
                "contracts/commands/{file} changed since generation — re-run \
                 `python3 crates/appname-core/tools/gen_command_registry.py`"
            );
        }
        // And nothing appeared in the directory that generation never saw.
        let mut on_disk: Vec<String> = std::fs::read_dir(commands_dir())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".yaml"))
            .collect();
        on_disk.sort();
        let mut fingered: Vec<String> = SOURCE_FINGERPRINTS
            .iter()
            .map(|(f, _, _)| f.to_string())
            .collect();
        fingered.sort();
        assert_eq!(
            on_disk, fingered,
            "contracts/commands file set drifted — regenerate"
        );
    }

    #[test]
    fn parsed_schemas_are_shared_not_reparsed_across_applies() {
        // MINOR-4 no-reparse proof: the parsed schema is one 'static
        // allocation — two lookups (as two applies perform) return the SAME
        // address, which is only possible if nothing re-parses.
        let a = parsed_schema("doc.new").unwrap() as *const Value;
        let b = parsed_schema("doc.new").unwrap() as *const Value;
        assert_eq!(a, b, "same 'static parsed value on every access");
        // And through the public apply path (two applies of one kind):
        let mut e1 = crate::engine::Engine::new();
        e1.apply_json(r#"{"schema":"appname.command.v1","id":"a0000000-0000-4000-8000-00000000a001","kind":"doc.new","params":{"width":8,"height":8}}"#).unwrap();
        let mut e2 = crate::engine::Engine::new();
        e2.apply_json(r#"{"schema":"appname.command.v1","id":"a0000000-0000-4000-8000-00000000a002","kind":"doc.new","params":{"width":8,"height":8}}"#).unwrap();
        let c = parsed_schema("doc.new").unwrap() as *const Value;
        assert_eq!(a, c, "applies consume the shared parse, never a fresh one");
    }

    #[test]
    fn every_kind_carries_parseable_schema_and_defaults() {
        for spec in COMMANDS.iter() {
            let s: serde_json::Value = serde_json::from_str(spec.params_schema)
                .unwrap_or_else(|e| panic!("{}: params_schema not JSON: {e}", spec.kind));
            assert!(
                s.is_object(),
                "{}: params_schema must be an object",
                spec.kind
            );
            let d: serde_json::Value = serde_json::from_str(spec.defaults)
                .unwrap_or_else(|e| panic!("{}: defaults not JSON: {e}", spec.kind));
            assert!(d.is_object(), "{}: defaults must be an object", spec.kind);
            assert!(
                matches!(spec.class, "R" | "M" | "C"),
                "{}: class {}",
                spec.kind,
                spec.class
            );
        }
    }

    #[test]
    fn every_valid_minimal_is_a_v1_envelope_of_its_kind() {
        let mut with_samples = Vec::new();
        for spec in COMMANDS.iter() {
            let vm: serde_json::Value = serde_json::from_str(spec.valid_minimal)
                .unwrap_or_else(|e| panic!("{}: validMinimal not JSON: {e}", spec.kind));
            assert_eq!(vm["schema"], "appname.command.v1", "{}", spec.kind);
            assert_eq!(vm["kind"], spec.kind);
            if vm.get("samples").map(|s| s.is_string()).unwrap_or(false) {
                with_samples.push(spec.kind);
            }
        }
        // The seven stroke kinds carry a samples assetId in their contract
        // envelopes (review blocker 1 fixture-fidelity check).
        assert_eq!(
            with_samples,
            [
                "brush.stroke",
                "eraser.stroke",
                "clone.stroke",
                "heal.stroke",
                "dodge_burn.stroke",
                "smudge.stroke",
                "blur_sharpen.stroke"
            ]
        );
    }

    #[test]
    fn doc_new_row_matches_its_contract_header() {
        let spec = lookup("doc.new").expect("doc.new registered");
        assert_eq!(spec.family, "doc");
        assert_eq!(spec.class, "R");
        assert!(!spec.dagger);
        assert_eq!(spec.inverse, Some("delta"));
        assert_eq!(spec.milestone, "0.1");
        // The schema carries the §40.2 required pair (array order is
        // contract order — canonical JSON sorts object keys, never arrays).
        let s: serde_json::Value = serde_json::from_str(spec.params_schema).unwrap();
        assert_eq!(s["required"], serde_json::json!(["width", "height"]));
    }

    #[test]
    fn dagger_kinds_are_exactly_the_three_of_10_3() {
        let daggers: BTreeSet<&str> = COMMANDS
            .iter()
            .filter(|c| c.dagger)
            .map(|c| c.kind)
            .collect();
        let expect: BTreeSet<&str> = ["doc.duplicate", "history.snapshot", "history.goto"]
            .into_iter()
            .collect();
        assert_eq!(daggers, expect);
    }

    #[test]
    fn unknown_kind_lookup_is_none() {
        assert!(lookup("doc.transmogrify").is_none());
        assert!(lookup("").is_none());
    }
}
