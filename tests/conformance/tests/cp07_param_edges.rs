// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: plan §2.4 O-QA row (adversarial per new kind) + review defect-8
//! assignment (integration log 2026-08-13); contracts/commands/*.yaml
//! examples.invalid for the 15 CP-0.7 kinds; the defect-8 STANDS ruling
//! (stage-3 mistyped/extra → out_of_range over the contract fixture
//! spelling — integration log 2026-08-13, OpNode review RULINGS).
//!
//! Runs the corpus-rev-13 `cp07ParamEdges` case lists on the ASSEMBLED
//! in-process engine under WD-1 raw-byte discipline: every sequence
//! element reaches `Engine::apply_bytes` as the verbatim byte slice of
//! the committed kind file (span-extracted, fidelity-checked).
//!
//! DRIFT ALARM: cases carrying `pendingArm` pin TODAY'S observed
//! behavior. The rev-13 wiring-seam arms TRIPPED by design at the
//! CP-0.7 finalize (assembled_engine lifted to the shipped five-crate
//! composition) and were resolved at corpus rev 14 with premise-real
//! re-encodings (live-op primes). Remaining arms await a flagged O-CORE
//! fix/ruling (QA-F2/F3/F4/F5/F12) or an unshipped wave (color-lookup,
//! linked sources, nested composite). When such a landing changes an
//! answer, this suite FAILS the workspace with the case's pendingArm
//! printed: finalize by re-encoding the case through the generator
//! (companion edit + O-QA ratification, the standing procedure). Stale
//! corpus = loud red, never fake green.

use appname_conformance::harness::{apply_raw, Outcome};
use appname_conformance::json::{parse, Value};
use appname_conformance::jsonspan;
use appname_conformance::{assembly_point, paths};

const PE_KINDS: [&str; 15] = [
    "adjustment.add",
    "filter.apply",
    "filter.smart_add",
    "history.goto",
    "history.snapshot",
    "layer.rasterize",
    "op.delete",
    "op.reorder",
    "op.set_mask",
    "op.set_params",
    "op.toggle",
    "smartobject.convert",
    "smartobject.edit_contents",
    "smartobject.rasterize",
    "smartobject.replace",
];

/// The generator's embedded prime-kit literals (rule 7: extracted from
/// the shipped CLI 2026-08-13; re-verified here on every run).
const PE_DOC_ENVELOPE_ID: &str = "c07e0000-0000-4000-8000-000000000d0c";
const PE_BG_LAYER: &str = "b5366c8c-2bf6-50c2-92c6-0ea0c9509522";

fn load_kind(kind: &str) -> (String, Value, jsonspan::Node) {
    let path = paths::repo_root()
        .join("fixtures")
        .join("adversarial")
        .join("generated")
        .join("kinds")
        .join(format!("{kind}.json"));
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let doc = parse(&text).unwrap();
    let span = jsonspan::parse(&text);
    (text, doc, span)
}

/// Span-extract `cp07ParamEdges[i].envelopeSequence[j]` with the WD-1
/// fidelity gate (the raw slice reparses to exactly the corpus value).
fn pe_raw<'a>(
    text: &'a str,
    span: &jsonspan::Node,
    doc: &Value,
    case_idx: usize,
    seq_idx: usize,
) -> &'a str {
    let node = span
        .get("cp07ParamEdges")
        .and_then(|c| c.idx(case_idx))
        .and_then(|c| c.get("envelopeSequence"))
        .and_then(|s| s.idx(seq_idx))
        .unwrap_or_else(|| panic!("no span for cp07ParamEdges[{case_idx}].seq[{seq_idx}]"));
    let expect = doc
        .get("cp07ParamEdges")
        .and_then(Value::as_arr)
        .and_then(|a| a.get(case_idx))
        .and_then(|c| c.get("envelopeSequence"))
        .and_then(Value::as_arr)
        .and_then(|a| a.get(seq_idx))
        .unwrap_or_else(|| panic!("no value at cp07ParamEdges[{case_idx}].seq[{seq_idx}]"));
    let raw = node.raw(text);
    let reparsed = parse(raw).unwrap_or_else(|e| panic!("raw slice does not parse: {e}"));
    assert!(
        &reparsed == expect,
        "WD-1 span fidelity broke at cp07ParamEdges[{case_idx}].seq[{seq_idx}]"
    );
    raw
}

fn check(case: &Value, got: &Outcome) -> Result<(), String> {
    let cid = case.get("caseId").and_then(Value::as_str).unwrap();
    let expect = case.get("cp07Expect").unwrap();
    let want_reject = expect.get("outcome").and_then(Value::as_str) == Some("reject");
    let pending = case
        .get("pendingArm")
        .filter(|v| !v.is_null())
        .map(|p| {
            format!(
                "\n  DRIFT ALARM: this case carries pendingArm {{code: {:?}, entersWith: \
                 {:?}}} — if the answer changed because that wave landed, FINALIZE via the \
                 generator (companion edit + O-QA ratification), never by hand-editing the \
                 kind file.",
                p.get("code").and_then(Value::as_str).unwrap_or("?"),
                p.get("entersWith").and_then(Value::as_str).unwrap_or("?")
            )
        })
        .unwrap_or_default();
    if want_reject {
        let want_code = expect.get("code").and_then(Value::as_str).unwrap();
        if got.ok {
            return Err(format!(
                "{cid}: expected reject {want_code}, engine ACCEPTED{pending}"
            ));
        }
        let got_code = got.code.as_deref().unwrap_or("");
        if got_code != want_code {
            return Err(format!(
                "{cid}: expected {want_code}, got {got_code} ({}){pending}",
                got.message
            ));
        }
    } else if !got.ok {
        return Err(format!(
            "{cid}: expected accept, engine rejected {} ({}){pending}",
            got.code.as_deref().unwrap_or(""),
            got.message
        ));
    }
    Ok(())
}

/// All 49 param-edge cases, fresh assembled engine per case, sequences
/// primed-through (every element but the last must apply cleanly).
#[test]
fn cp07_param_edges_all_cases() {
    let mut total = 0usize;
    let mut failures = Vec::new();
    for kind in PE_KINDS {
        let (text, doc, span) = load_kind(kind);
        let cases = doc
            .get("cp07ParamEdges")
            .and_then(Value::as_arr)
            .unwrap_or_else(|| panic!("{kind}: no cp07ParamEdges"));
        for (i, case) in cases.iter().enumerate() {
            total += 1;
            let cid = case.get("caseId").and_then(Value::as_str).unwrap();
            let n = case
                .get("envelopeSequence")
                .and_then(Value::as_arr)
                .unwrap()
                .len();
            let mut engine = assembly_point::assembled_engine();
            let mut last = None;
            let mut early = None;
            for j in 0..n {
                let raw = pe_raw(&text, &span, &doc, i, j);
                let out = apply_raw(&mut engine, raw);
                if j + 1 < n && !out.ok {
                    early = Some(format!(
                        "{cid}: prime step {j} failed: {} ({})",
                        out.code.as_deref().unwrap_or(""),
                        out.message
                    ));
                    break;
                }
                last = Some(out);
            }
            if let Some(e) = early {
                failures.push(e);
                continue;
            }
            if let Err(e) = check(case, &last.unwrap()) {
                failures.push(e);
            }
        }
    }
    assert_eq!(total, 49, "PE case inventory moved — regenerate + re-pin");
    assert!(
        failures.is_empty(),
        "cp07 param edges: {} of {total} failed\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Manifest bookkeeping: counts match the emitted files; tiers legal;
/// every referenced code (current + pendingArm) is a registered v5.5 id
/// except the plan-tier not_implemented (OQ-1, standing).
#[test]
fn cp07_param_edges_manifest_pins() {
    let mpath = paths::repo_root()
        .join("fixtures")
        .join("adversarial")
        .join("manifest.json");
    let manifest = parse(&std::fs::read_to_string(mpath).unwrap()).unwrap();
    let pe = manifest.get("cp07ParamEdges").expect("manifest section");
    let counts = pe.get("counts").and_then(Value::as_obj).unwrap();
    let mut total = 0i64;
    for kind in PE_KINDS {
        let (_, doc, _) = load_kind(kind);
        let n = doc
            .get("cp07ParamEdges")
            .and_then(Value::as_arr)
            .map(|a| a.len() as i64)
            .unwrap_or(0);
        assert_eq!(
            counts.get(kind).and_then(Value::as_i64),
            Some(n),
            "{kind}: manifest count vs file"
        );
        total += n;
        let cases = doc.get("cp07ParamEdges").and_then(Value::as_arr).unwrap();
        for case in cases {
            let tier = case.get("codeTier").and_then(Value::as_str);
            assert!(
                matches!(tier, Some("contract" | "plan")),
                "bad tier {tier:?}"
            );
            assert!(
                case.get("provenance")
                    .and_then(Value::as_str)
                    .is_some_and(|p| !p.is_empty()),
                "empty provenance"
            );
        }
    }
    assert_eq!(pe.get("total").and_then(Value::as_i64), Some(total));
    assert_eq!(total, 49);
}

/// Rule-7 premise check: the prime doc.new really creates the background
/// layer the generator embeds (the adjustment.add collision premise and
/// every remap literal hang off this).
#[test]
fn cp07_prime_kit_ids_are_real() {
    let mut engine = assembly_point::assembled_engine();
    let prime = format!(
        "{{\"schema\":\"appname.command.v1\",\"id\":\"{PE_DOC_ENVELOPE_ID}\",\
         \"kind\":\"doc.new\",\"params\":{{\"width\":64,\"height\":64,\
         \"background\":\"white\"}}}}"
    );
    let receipt = engine
        .apply_bytes(prime.as_bytes())
        .expect("prime doc.new applies");
    let last = receipt
        .created_ids
        .last()
        .expect("doc.new reports created ids");
    assert_eq!(
        last.to_string(),
        PE_BG_LAYER,
        "prime background layer id drifted — regenerate the corpus prime kit"
    );
}
