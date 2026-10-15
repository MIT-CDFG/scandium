// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Engine conformance suites — LIVE (wave-4 activation of the slice-1
//! staged assertions; formerly engine_pending.rs, all `#[ignore]`s dropped).
//!
//! Spec: plan §2.1 rows "Command engine" / "Test harness v1"; ES §10.2
//! (single apply entry), §40.1 (taxonomy); pinned validation order
//! (reports/O-CORE-log.md, binding OQ-3 arbitration); WD-1 (raw corpus
//! bytes, fidelity-checked spans); wave-2 binding note (ported envelope
//! cases hold under PRIMED-SEQUENTIAL semantics only).
//!
//! Aggregate contract, reproduced independently by the slice-3 review:
//! 165 PASS / 0 FAIL / 1 SKIP over the 166 engine-runnable corpus cases
//! (the skip is the 10^6 x 10^6 px validMaximal schema-only fixture).

use appname_conformance::harness::{apply_raw, case_raw, check_case, check_case_asm, is_runnable};
use appname_conformance::json::{parse, Value};
use appname_conformance::jsonspan;
use appname_conformance::{corpus, paths};
use appname_core::Engine;

/// Load one corpus file three ways: source text (raw bytes), value tree
/// (expectations), span tree (WD-1 slices).
fn load3(rel: &[&str]) -> (String, Value, jsonspan::Node) {
    let mut p = paths::adversarial_dir();
    for seg in rel {
        p = p.join(seg);
    }
    let text = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()));
    let doc = parse(&text).unwrap_or_else(|e| panic!("parse {}: {e}", p.display()));
    let span = jsonspan::parse(&text);
    (text, doc, span)
}

/// The raw prime envelope (contract validMinimal doc.new) — taken as a
/// verbatim byte slice OF THE CORPUS FILE per WD-1.
fn prime_raw(text: &str, doc: &Value, span: &jsonspan::Node) -> String {
    let cases = doc.get("cases").and_then(Value::as_arr).unwrap();
    let idx = cases
        .iter()
        .position(|c| c.get("caseId").and_then(Value::as_str) == Some("G-DN-accept-valid-minimal"))
        .expect("doc.new matrix carries the validMinimal accept");
    case_raw(text, span, doc, idx, "envelope", None).to_string()
}

fn doc_new_file() -> (String, Value, jsonspan::Node) {
    load3(&["generated", "kinds", "doc.new.json"])
}

/// Run one apply-driver case on a FRESH engine of the caller's assembly
/// (sequence-aware: every element but the last must apply cleanly; the
/// last is asserted).
fn run_fresh(
    mut engine: Engine,
    text: &str,
    doc: &Value,
    span: &jsonspan::Node,
    idx: usize,
    case: &Value,
) -> Result<(), String> {
    run_fresh_asm(&mut engine, text, doc, span, idx, case, false, None)
}

#[allow(clippy::too_many_arguments)]
fn run_fresh_asm(
    engine: &mut Engine,
    text: &str,
    doc: &Value,
    span: &jsonspan::Node,
    idx: usize,
    case: &Value,
    assembled: bool,
    overlay: Option<&Value>,
) -> Result<(), String> {
    let cid = case.get("caseId").and_then(Value::as_str).unwrap();
    let outcome = if case.get("envelopeSequence").is_some_and(|s| !s.is_null()) {
        let n = case
            .get("envelopeSequence")
            .and_then(Value::as_arr)
            .unwrap()
            .len();
        let mut last = None;
        for j in 0..n {
            let raw = case_raw(text, span, doc, idx, "envelopeSequence", Some(j));
            let out = apply_raw(engine, raw);
            if j + 1 < n && !out.ok {
                return Err(format!(
                    "{cid}: sequence step {j} failed early: {:?} {}",
                    out.code, out.message
                ));
            }
            last = Some(out);
        }
        last.unwrap()
    } else {
        let raw = case_raw(text, span, doc, idx, "envelope", None);
        apply_raw(engine, raw)
    };
    check_case_asm(case, &outcome, assembled, overlay)
}

/// The 29 ported proto envelope cases under the binding PRIMED-SEQUENTIAL
/// semantics: ONE engine, ONE doc.new applied first, then file order
/// (wave-2 integration note — pinned here; fresh-per-case would void the
/// A09/A20 premises).
#[test]
fn cp01_ported_envelope_cases_primed_sequential() {
    let (dtext, ddoc, dspan) = doc_new_file();
    let prime = prime_raw(&dtext, &ddoc, &dspan);
    let (text, doc, span) = load3(&["ported", "envelope.json"]);

    let mut engine = Engine::new();
    let primed = apply_raw(&mut engine, &prime);
    assert!(primed.ok, "prime doc.new failed: {:?}", primed.code);

    let cases = corpus::cases(&doc);
    let mut failures = Vec::new();
    for (i, case) in cases.iter().enumerate() {
        assert!(is_runnable(case), "ported envelope cases are all runnable");
        let raw = case_raw(&text, &span, &doc, i, "envelope", None);
        let out = apply_raw(&mut engine, raw);
        if let Err(e) = check_case(case, &out) {
            failures.push(e);
        }
    }
    assert_eq!(cases.len(), 29);
    assert!(
        failures.is_empty(),
        "ported envelope failures:\n{}",
        failures.join("\n")
    );
}

/// All 101 registered-but-unimplemented kinds answer
/// error.command.not_implemented (plan registry rule; stage 4 of the
/// pinned order). Fresh engine per case; envelopes are the contracts' own
/// validMinimal shapes, fed as raw corpus bytes.
#[test]
fn cp01_not_implemented_all_kinds() {
    let mut seen = 0;
    let mut failures = Vec::new();
    for kind in corpus::kind_file_names() {
        if kind == "doc.new" {
            continue;
        }
        let (text, doc, span) = load3(&["generated", "kinds", &format!("{kind}.json")]);
        for (i, case) in corpus::cases(&doc).iter().enumerate() {
            assert!(is_runnable(case), "{kind}: NI case must be runnable");
            // CP-0.7 dagger built-ins (doc.new precedent): the overlay's
            // bareCore is their bare-engine truth — the first departure
            // from the cp01 carry-forward (O-CORE rev-11 companion,
            // flagged for O-QA ratification).
            if matches!(kind.as_str(), "history.snapshot" | "history.goto") {
                let raw = case_raw(&text, &span, &doc, i, "envelope", None);
                let mut engine = Assembly::BareCore.engine();
                let out = apply_raw(&mut engine, raw);
                let want = doc
                    .get("cp03")
                    .and_then(|o| o.get("bareCore"))
                    .and_then(|b| b.get("code"))
                    .and_then(Value::as_str)
                    .expect("dagger overlay");
                if out.code.as_deref() != Some(want) {
                    failures.push(format!(
                        "{kind}: wanted {want}, got {:?} ({})",
                        out.code, out.message
                    ));
                }
                seen += 1;
                continue;
            }
            if let Err(e) = run_fresh(Assembly::BareCore.engine(), &text, &doc, &span, i, case) {
                failures.push(e);
            }
            seen += 1;
        }
    }
    assert_eq!(seen, 101, "one live case per unimplemented kind");
    assert!(
        failures.is_empty(),
        "not-implemented failures:\n{}",
        failures.join("\n")
    );
}

/// The doc.new param-validation matrix: 25 runnable cases pass, exactly 1
/// deliberate skip (the validMaximal schema-only fixture — a 10^12-px
/// allocation; OQ-6 tracks a validate-only entry).
#[test]
fn cp01_doc_new_reject_matrix() {
    let (text, doc, span) = doc_new_file();
    let mut ran = 0;
    let mut skipped = Vec::new();
    let mut failures = Vec::new();
    for (i, case) in corpus::cases(&doc).iter().enumerate() {
        if !is_runnable(case) {
            skipped.push(
                case.get("caseId")
                    .and_then(Value::as_str)
                    .unwrap()
                    .to_string(),
            );
            continue;
        }
        if let Err(e) = run_fresh(Assembly::BareCore.engine(), &text, &doc, &span, i, case) {
            failures.push(e);
        }
        ran += 1;
    }
    assert_eq!(ran, 25, "doc.new runnable count");
    assert_eq!(
        skipped,
        ["G-DN-accept-valid-maximal-schema-only"],
        "exactly the declared skip, counted never silent"
    );
    assert!(
        failures.is_empty(),
        "doc.new matrix failures:\n{}",
        failures.join("\n")
    );
}

/// Envelope-stage rejects (stages 0-3 of the pinned order + the stage-2
/// duplicate-id sequence). Fresh engine per case.
#[test]
fn cp01_envelope_stage_rejects() {
    let (text, doc, span) = load3(&["generated", "envelope_rejects.json"]);
    let mut failures = Vec::new();
    let cases = corpus::cases(&doc);
    for (i, case) in cases.iter().enumerate() {
        assert!(is_runnable(case));
        if let Err(e) = run_fresh(Assembly::BareCore.engine(), &text, &doc, &span, i, case) {
            failures.push(e);
        }
    }
    assert_eq!(cases.len(), 10);
    assert!(
        failures.is_empty(),
        "envelope-stage failures:\n{}",
        failures.join("\n")
    );
}

/// Engine assemblies the aggregate runs under (CP-0.3 cp03-overlay
/// mechanism, integration-log wave-2 arbitration #1). `BareCore` =
/// `Engine::new()` — per the arbitration it answers `not_implemented`
/// for stroke kinds at EVERY checkpoint, so the 165/0/1 pin holds for
/// bare-core beyond CP-0.1. An `Assembled` variant (CLI/cabi-style
/// construction wiring appname-tools' stroke KindImpls) joins at the
/// cp03 finalize wave, with its own expected tallies driven by the
/// corpus `cp03.assembled` overlays; until then the drift alarm in
/// cp03_scaffold.rs guards the seam.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Assembly {
    BareCore,
    /// The CLI/cabi construction (arbitration #1), built in-process from
    /// the shipped assembly point (cp03 finalize; OQ-9 gap note in the
    /// lane log). Expectations overlay per the corpus cp03 rule: 2
    /// kind-file overlays (stroke NI -> bad_target) + 4 ported overlays
    /// (proto-shaped stroke params -> stage-5 missing_field); the other
    /// 160 expectations carry cp01 forward. Counts stay 165/0/1.
    Assembled,
}

impl Assembly {
    fn engine(self) -> Engine {
        match self {
            Assembly::BareCore => Engine::new(),
            Assembly::Assembled => appname_conformance::assembly_point::assembled_engine(),
        }
    }
    fn assembled(self) -> bool {
        self == Assembly::Assembled
    }
    fn expected(self) -> (u32, u32) {
        match self {
            Assembly::BareCore | Assembly::Assembled => (165, 1),
        }
    }
}

/// The kind-file cp03 overlay for assembled runs (`cp03.assembled`),
/// when the file declares one.
fn file_overlay(doc: &Value) -> Option<&Value> {
    doc.get("cp03")
        .and_then(|o| o.get("assembled"))
        .filter(|v| !v.is_null())
}

/// Aggregate + machine contract, parameterized by engine assembly: the
/// full engine-runnable corpus scores 165 PASS / 0 FAIL / 1 SKIP on the
/// bare-core assembly (the slice-3 review's independent number), and
/// every rejection along the way is structured (error.* code plus a
/// non-empty message — enforced inside check_case/apply paths).
#[test]
fn cp01_corpus_aggregate_165_0_1() {
    for assembly in [Assembly::BareCore, Assembly::Assembled] {
        aggregate_for(assembly);
    }
}

fn aggregate_for(assembly: Assembly) {
    let mut pass = 0u32;
    let mut fail = Vec::new();
    let mut skip = 0u32;

    // ported envelope, primed-sequential
    {
        let (dtext, ddoc, dspan) = doc_new_file();
        let prime = prime_raw(&dtext, &ddoc, &dspan);
        let (text, doc, span) = load3(&["ported", "envelope.json"]);
        let mut engine = Engine::new();
        assert!(apply_raw(&mut engine, &prime).ok);
        for (i, case) in corpus::cases(&doc).iter().enumerate() {
            let raw = case_raw(&text, &span, &doc, i, "envelope", None);
            let out = apply_raw(&mut engine, raw);
            match check_case(case, &out) {
                Ok(()) => pass += 1,
                Err(e) => fail.push(e),
            }
        }
    }
    // generated suites, fresh engine per case
    let mut files: Vec<Vec<String>> =
        vec![vec!["generated".into(), "envelope_rejects.json".into()]];
    for kind in corpus::kind_file_names() {
        files.push(vec![
            "generated".into(),
            "kinds".into(),
            format!("{kind}.json"),
        ]);
    }
    for rel in files {
        let rel_refs: Vec<&str> = rel.iter().map(String::as_str).collect();
        let (text, doc, span) = load3(&rel_refs);
        for (i, case) in corpus::cases(&doc).iter().enumerate() {
            if !is_runnable(case) {
                skip += 1;
                continue;
            }
            let mut engine = assembly.engine();
            // CP-0.7 dagger built-ins: on the BARE leg their expectation
            // is the overlay's bareCore (doc.new precedent — the first
            // bare departure from the cp01 carry-forward; O-CORE rev-11
            // companion, flagged for O-QA). The `true` below selects the
            // overlay expectation channel for exactly that case.
            let bare_dagger = !assembly.assembled()
                && doc
                    .get("kind")
                    .and_then(Value::as_str)
                    .map(|k| k.starts_with("history."))
                    .unwrap_or(false);
            let (use_overlay, sel_assembled) = if bare_dagger {
                (doc.get("cp03").and_then(|o| o.get("bareCore")), true)
            } else {
                (file_overlay(&doc), assembly.assembled())
            };
            match run_fresh_asm(
                &mut engine,
                &text,
                &doc,
                &span,
                i,
                case,
                sel_assembled,
                use_overlay,
            ) {
                Ok(()) => pass += 1,
                Err(e) => fail.push(e),
            }
        }
    }

    assert!(
        fail.is_empty(),
        "corpus aggregate FAIL={}:\n{}",
        fail.len(),
        fail.join("\n")
    );
    assert_eq!(
        (pass, skip),
        assembly.expected(),
        "aggregate drifted from the reviewed 165 PASS / 0 FAIL / 1 SKIP \
         ({assembly:?})"
    );
}
