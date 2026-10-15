// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Aud-parity **P-1** conformance probe — "does every contract's own
//! `validMinimal` example actually pass its own contract?"
//!
//! The finding that motivates this: a shipped contract example that the
//! engine itself refuses. `clone.stroke`'s `validMinimal` uses
//! `source:"composite"` with the defaulted `sampleAllLayers:false`, which
//! its own contract note 1 (§45.2 sample-scope tri-state) declares the
//! INVALID combination — the engine refuses it with
//! `error.schema.out_of_range`. This probe is the tool that would have
//! caught that, run across the whole contract set.
//!
//! ★ THAT MOTIVATING INSTANCE IS NOW CLOSED — `clone.stroke`'s `validMinimal`
//! gained `sampleAllLayers: true` at bogos `92d4781`, and its pin was removed
//! here when the probe's own stale-pin guard red on it. The history is kept
//! because it is WHY this probe exists; do not read it as a live defect. The
//! only pin remaining is `adjustment.add`, and that one is EXPECTED-FAIL BY
//! RULING rather than unfixed debt — see its `fix` pointer.
//!
//! Two legs, because a shipped example can be refused at two different
//! stages:
//!
//! ## Leg 1 — SCHEMA conformance (document-independent). Every kind's
//! `validMinimal` params are run through the SAME stage-3 validator the
//! apply door uses (`registry::validate_params`), with no document at all.
//! A failure means the example is malformed against its own registered
//! `paramsSchema`. This leg is currently GREEN with zero failures — no
//! example violates its own schema shape — but it is a real gate: a future
//! example-vs-schema drift reds here.
//!
//! ## Leg 2 — SERVED conformance (the example is accepted by the engine,
//! not typed-refused at EXECUTION). Each `validMinimal` is applied to a
//! resolved assembled-engine document (target/source layers, a real
//! `samples` asset, a built-in `brushPreset`) and any `error.schema.*` /
//! `error.command.unknown_kind` / `error.api.*` refusal is flagged: those
//! codes mean the engine rejected the example's own shape, not the probe
//! document. Some refusals are enforced at execution rather than in
//! `paramsSchema` (a params key the schema forbids via
//! `additionalProperties:false`; the `clone.stroke` tri-state) — Leg 1
//! cannot see them, Leg 2 does. (That tri-state is cited as the MECHANISM,
//! which still holds; its instance is fixed, per the note above.)
//!
//! ### Live-state exclusions (documented, not gated). A bare probe
//! document legitimately cannot host every example: a crop rect must
//! intersect the canvas, a guide/slice index needs a guide/slice to exist,
//! a puppet pin must land inside real content, `layer.duplicate`'s idMap
//! must cover the actual target subtree. Those examples are contract-VALID
//! and would run against the document each assumes; their refusal here is a
//! probe-document artifact, NOT a conformance defect. The kinds in
//! [`LIVE_STATE_EXCLUSIONS`] are therefore skipped by Leg 2 (Leg 1 still
//! covers their schema shape). Each carries the reason it is document-
//! dependent.
//!
//! ## EXPECTED-FAIL, not a hard gate. The genuine defects are all fixable
//! only by editing the frozen bogos contracts (spec **v5.6**) — out of
//! scope appname-side. Each is PINNED in [`EXECUTION_EXPECTED_FAILS`] with
//! its fix pointer: the probe is GREEN now while documenting the debt, and
//! reds the moment a fix lands (a pin stops reproducing) — the reminder to
//! unpin.
//!
//! Coverage note: this exercises `validMinimal` (the registry embeds it as
//! canonical JSON). `validMaximal` lives only in the YAML contracts;
//! extending to it needs a contract-YAML reader (no YAML crate is in the
//! deps allowlist) or a `valid_maximal` registry field — recorded as a
//! fold-in, not built here.

use appname_core::engine::registry;
use serde_json::Value;

use appname_conformance::assembly_point;
use appname_conformance::stroke_goldens;

/// A shipped `validMinimal` the engine refuses as malformed against its own
/// contract — genuine P-1 debt, pinned with its fix pointer.
struct ExpectedFail {
    kind: &'static str,
    /// The wire code the engine currently answers.
    code: &'static str,
    /// A substring the refusal message must contain (keeps the pin honest
    /// about WHICH defect reproduces).
    msg_needle: &'static str,
    /// Where the fix lives (self-documenting the pin).
    fix: &'static str,
}

/// Kinds whose `validMinimal` refusal in a bare probe document is a
/// LIVE-STATE artifact (the example needs document state this probe does
/// not build), NOT a contract defect. Skipped by Leg 2 with the reason.
const LIVE_STATE_EXCLUSIONS: &[(&str, &str)] = &[
    (
        "doc.crop",
        "crop rect must intersect the probe canvas (§40.2 live geometry)",
    ),
    (
        "layer.duplicate",
        "idMap must cover the probe target's actual subtree identity (CMD-001)",
    ),
    (
        "transform.puppet",
        "pins must land inside real layer content bounds (§40.6)",
    ),
    (
        "guide.move",
        "guide index into an empty guide list (live-state)",
    ),
    (
        "guide.delete",
        "guide index into an empty guide list (live-state)",
    ),
    (
        "slice.edit",
        "slice index into an empty slice list (§40.8 live-state)",
    ),
    (
        "slice.delete",
        "slice index into an empty slice list (§40.8 live-state)",
    ),
];

/// KNOWN execution-stage failures: `validMinimal` examples the engine
/// serves and then refuses on their own shape. All are bogos contract
/// edits → spec v5.6.
const EXECUTION_EXPECTED_FAILS: &[ExpectedFail] = &[ExpectedFail {
    kind: "adjustment.add",
    code: "error.schema.out_of_range",
    msg_needle: "channels",
    fix: "DO NOT 'reconcile' by editing the example — that direction was tried at bogos \
              29beabc and REVERTED at 926389e as a spec-integrity defect. The DIRECTION IS \
              RULED (operator 2026-08-26, ruling B): the contract is right and THE ENGINE IS \
              NON-CONFORMANT. adjustment.add.yaml validMinimal carries the §9.6 structured \
              shape (channels:/{points:…}) at four normative sites; ops.rs:314 registers a \
              flat additionalProperties:false schema over master/r/g/b and refuses it. This \
              pin is EXPECTED-FAIL BY RULING and stays until the engine conforms — see the \
              engine row filed at appname f7fb0f7d. Per-key isolation measured at the door: \
              channels refused, {points:…} refused, interpolation refused on its own.",
}];

fn is_typed_refusal(code: &str) -> bool {
    code.starts_with("error.schema.")
        || code == "error.command.unknown_kind"
        || code.starts_with("error.api.")
}

// ---------------------------------------------------------------------------
// Leg 1 — schema conformance over every validMinimal (document-independent).
// ---------------------------------------------------------------------------

#[test]
fn every_valid_minimal_validates_against_its_own_schema() {
    let mut flagged: Vec<(String, String, String)> = Vec::new();
    for kind in registry::kinds() {
        let spec = registry::lookup(kind).expect("registered");
        let vm: Value = serde_json::from_str(spec.valid_minimal).expect("validMinimal is JSON");
        let params = vm.get("params").and_then(Value::as_object);
        if let Err(e) = registry::validate_params(spec, params) {
            flagged.push((kind.to_string(), e.code.as_str().to_string(), e.message));
        }
    }
    eprintln!(
        "P-1 Leg 1 (schema): {} validMinimal example(s) fail their own schema",
        flagged.len()
    );
    for (kind, code, msg) in &flagged {
        eprintln!("    {kind}  validMinimal  {code}  ({msg})");
    }
    assert!(
        flagged.is_empty(),
        "P-1 Leg 1: validMinimal example(s) violate their own registered schema: {flagged:?}. \
         Each is a genuine example-vs-schema contract inconsistency — pin or fix."
    );
}

// ---------------------------------------------------------------------------
// Leg 2 — served conformance: apply every validMinimal to a resolved doc
// and flag typed refusals (excluding documented live-state kinds).
// ---------------------------------------------------------------------------

/// Apply one `validMinimal` to a resolved probe document, with the
/// example's referenced entities rewritten onto ones the document carries
/// so execution reaches the kind's live validation stages. Returns the wire
/// code + message on refusal, `None` on ok.
fn apply_resolved(example_json: &str) -> Option<(String, String)> {
    let mut env: Value = serde_json::from_str(example_json).expect("example is JSON");
    let mut engine = assembly_point::assembled_engine();

    let target = engine
        .apply_json(
            r#"{"schema":"appname.command.v1","id":"ffffffff-0000-4000-8000-000000000001",
                "kind":"doc.new","params":{"width":64,"height":64,"background":"white"}}"#,
        )
        .expect("probe doc.new applies")
        .created_ids
        .last()
        .copied()
        .expect("doc.new creates the background layer")
        .to_string();
    let source = "ffffffff-0000-4000-8000-0000000000a2";
    engine
        .apply_json(&format!(
            r#"{{"schema":"appname.command.v1","id":"ffffffff-0000-4000-8000-000000000002",
                "kind":"layer.add","params":{{"kind":"raster","newLayerId":"{source}","name":"src"}}}}"#
        ))
        .expect("probe source layer.add applies");
    let samples_id = engine
        .add_samples_asset(stroke_goldens::fixed_samples_basic())
        .to_string();
    let preset = assembly_point::preset_hard_round_16_id().to_string();

    if let Some(t) = env.get_mut("target").and_then(Value::as_object_mut) {
        if t.get("layerId").map(Value::is_string).unwrap_or(false) {
            t.insert("layerId".into(), Value::String(target));
        }
    }
    if env.get("samples").map(Value::is_string).unwrap_or(false) {
        env["samples"] = Value::String(samples_id);
    }
    if let Some(p) = env.get_mut("params").and_then(Value::as_object_mut) {
        if p.get("brushPreset").map(Value::is_string).unwrap_or(false) {
            p.insert("brushPreset".into(), Value::String(preset));
        }
        if let Some(s) = p.get_mut("source").and_then(Value::as_object_mut) {
            if s.get("layerId").map(Value::is_string).unwrap_or(false) {
                s.insert("layerId".into(), Value::String(source.to_string()));
            }
        }
    }

    match engine.apply_json(&env.to_string()) {
        Ok(_) => None,
        Err(e) => Some((e.code.as_str().to_string(), e.message)),
    }
}

#[test]
fn every_valid_minimal_is_served_not_typed_refused() {
    let excluded = |kind: &str| LIVE_STATE_EXCLUSIONS.iter().any(|(k, _)| *k == kind);

    // (kind, code, message) for each typed refusal, excluding live-state kinds.
    let mut flagged: Vec<(String, String, String)> = Vec::new();
    for kind in registry::kinds() {
        if excluded(kind) {
            continue;
        }
        let spec = registry::lookup(kind).expect("registered");
        if let Some((code, msg)) = apply_resolved(spec.valid_minimal) {
            if is_typed_refusal(&code) {
                flagged.push((kind.to_string(), code, msg));
            }
        }
    }

    eprintln!(
        "P-1 Leg 2 (served): {} validMinimal example(s) typed-refused at execution:",
        flagged.len()
    );
    for (kind, code, msg) in &flagged {
        eprintln!("    {kind}  validMinimal  {code}  ({msg})");
    }

    // NEW typed refusal (outside the pinned set) = undocumented debt.
    let pinned = |kind: &str| EXECUTION_EXPECTED_FAILS.iter().any(|e| e.kind == kind);
    let new_failures: Vec<&(String, String, String)> =
        flagged.iter().filter(|(k, _, _)| !pinned(k)).collect();
    assert!(
        new_failures.is_empty(),
        "P-1 Leg 2 found NEW typed-refused validMinimal example(s) not pinned: {new_failures:?}. \
         Pin genuine contract defects (with a fix pointer), add a live-state exclusion (with a \
         reason) if it is a probe-doc artifact, or fix the probe."
    );

    // Each pinned failure must STILL reproduce with the expected code+needle.
    for ef in EXECUTION_EXPECTED_FAILS {
        let hit = flagged.iter().find(|(k, _, _)| k == ef.kind);
        match hit {
            None => panic!(
                "P-1 EXPECTED-FAIL pin for {} no longer reproduces (expected {} / \"{}\"). \
                 The fix likely landed — remove the stale pin. Fix pointer: {}",
                ef.kind, ef.code, ef.msg_needle, ef.fix
            ),
            Some((_, code, msg)) => {
                assert_eq!(
                    code, ef.code,
                    "P-1 pin {} changed shape: expected {}, got {} ({msg}). Re-triage.",
                    ef.kind, ef.code, code
                );
                assert!(
                    msg.contains(ef.msg_needle),
                    "P-1 pin {} message lost its needle \"{}\": {msg}",
                    ef.kind,
                    ef.msg_needle
                );
            }
        }
    }
}
