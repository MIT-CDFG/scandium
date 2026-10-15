// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Corpus self-checks — RUN GREEN at CP-0.1 wave 1 (no engine needed).
//!
//! Spec: plan AGENT-ASSIGNMENTS-full-build.md §2.1 "Test harness v1";
//! ES §10.2 (envelope), §40.1 (error taxonomy), Appendix G (MessageId
//! registry); fixtures/README.md case-record contract.
//!
//! These tests validate the COMMITTED corpus bytes against the frozen
//! contracts mirror: files parse, the expected-outcome taxonomy is
//! well-formed and tiered, per-kind coverage equals the 102-kind index, and
//! case counts match the declared inventory (29 + 12 ported = the proto's
//! 41-case suite). They never invoke the engine and never re-run the
//! generator (the proto checkout is not assumed present in CI).

use appname_conformance::{contracts, corpus, expect, json::Value};
use std::collections::{BTreeMap, BTreeSet};

fn all_cases() -> Vec<(String, Value)> {
    // (origin label, case) across every corpus file, deterministic order.
    let mut out = Vec::new();
    for c in corpus::cases(&corpus::ported_envelope()) {
        out.push(("ported/envelope.json".to_string(), c.clone()));
    }
    for c in corpus::cases(&corpus::ported_cli()) {
        out.push(("ported/cli.json".to_string(), c.clone()));
    }
    for c in corpus::cases(&corpus::envelope_rejects()) {
        out.push(("generated/envelope_rejects.json".to_string(), c.clone()));
    }
    for kind in corpus::kind_file_names() {
        for c in corpus::cases(&corpus::kind_file(&kind)) {
            out.push((format!("generated/kinds/{kind}.json"), c.clone()));
        }
    }
    out
}

fn case_id(c: &Value) -> &str {
    c.get("caseId").and_then(Value::as_str).expect("caseId")
}

#[test]
fn every_corpus_file_parses() {
    let files = corpus::corpus_files();
    // manifest + 2 ported + envelope_rejects + one file per kind
    let expected = 4 + corpus::kind_file_names().len();
    assert_eq!(
        files.len(),
        expected,
        "corpus file inventory drifted: {files:#?}"
    );
    for f in files {
        corpus::load(&f); // panics with the path on any parse failure
    }
}

#[test]
fn per_kind_coverage_equals_contract_index() {
    let index = contracts::index_kinds();
    assert_eq!(index.len(), 102, "contract index no longer lists 102 kinds");
    let index_set: BTreeSet<_> = index.iter().cloned().collect();
    assert_eq!(index_set.len(), 102, "duplicate kind rows in _index.yaml");

    let files: BTreeSet<_> = corpus::kind_file_names().into_iter().collect();
    assert_eq!(
        files, index_set,
        "generated/kinds/*.json must cover exactly the contract index"
    );

    let manifest = corpus::manifest();
    let manifest_kinds: BTreeSet<_> = manifest
        .get("kinds")
        .and_then(Value::as_arr)
        .expect("manifest.kinds")
        .iter()
        .map(|v| v.as_str().expect("kind string").to_string())
        .collect();
    assert_eq!(manifest_kinds, index_set, "manifest.kinds drifted");

    // each kind file's cases target that kind (apply-driver cases)
    for kind in &files {
        let doc = corpus::kind_file(kind);
        assert_eq!(doc.get("kind").and_then(Value::as_str), Some(kind.as_str()));
        for c in corpus::cases(&doc) {
            if let Some(env) = c.get("envelope") {
                if !env.is_null() {
                    assert_eq!(
                        env.get("kind").and_then(Value::as_str),
                        Some(kind.as_str()),
                        "{}: envelope kind mismatch",
                        case_id(c)
                    );
                }
            }
            if let Some(seq) = c.get("envelopeSequence").and_then(Value::as_arr) {
                for env in seq {
                    assert_eq!(
                        env.get("kind").and_then(Value::as_str),
                        Some(kind.as_str()),
                        "{}: sequence envelope kind mismatch",
                        case_id(c)
                    );
                }
            }
        }
    }
}

#[test]
fn no_codeanyof_hedges_remain() {
    // Wave-4 discipline: the validation order is PINNED (O-CORE-log,
    // binding OQ-3 arbitration; manifest hedgeCollapse table), so every
    // cp01 expectation names exactly one code (or none for
    // any-structured). A reappearing hedge means someone re-opened a
    // pinned question — route through O-QA review instead.
    let manifest = corpus::manifest();
    let collapsed = manifest
        .get("hedgeCollapse")
        .and_then(|h| h.get("collapsed"))
        .and_then(Value::as_arr)
        .expect("manifest.hedgeCollapse.collapsed");
    assert_eq!(collapsed.len(), 14, "the wave-4 collapse covered 14 cases");
    for (origin, c) in all_cases() {
        if let Some(cp01) = c.get("cp01Expect").filter(|v| !v.is_null()) {
            assert!(
                cp01.get("codeAnyOf").is_none_or(Value::is_null),
                "{origin}/{}: codeAnyOf hedge after the wave-4 collapse",
                case_id(&c)
            );
        }
    }
}

#[test]
fn declared_counts_match_reality() {
    let manifest = corpus::manifest();
    let counts = manifest.get("counts").expect("manifest.counts");
    let get = |k: &str| {
        counts
            .get(k)
            .and_then(Value::as_i64)
            .unwrap_or_else(|| panic!("manifest.counts.{k}"))
    };

    let ported_env = corpus::cases(&corpus::ported_envelope()).len() as i64;
    let ported_cli = corpus::cases(&corpus::ported_cli()).len() as i64;
    let env_rej = corpus::cases(&corpus::envelope_rejects()).len() as i64;
    let kinds = corpus::kind_file_names();
    let mut ni = 0i64;
    let mut doc_new = 0i64;
    for kind in &kinds {
        let n = corpus::cases(&corpus::kind_file(kind)).len() as i64;
        if kind == "doc.new" {
            doc_new = n;
        } else {
            ni += n;
        }
    }

    assert_eq!(get("portedEnvelope"), ported_env);
    assert_eq!(get("portedCli"), ported_cli);
    assert_eq!(get("portedTotal"), ported_env + ported_cli);
    assert_eq!(get("generatedNotImplemented"), ni);
    assert_eq!(get("generatedDocNew"), doc_new);
    assert_eq!(get("generatedEnvelopeRejects"), env_rej);
    assert_eq!(get("kindFiles"), kinds.len() as i64);
    assert_eq!(
        get("total"),
        ported_env + ported_cli + ni + doc_new + env_rej
    );
}

#[test]
fn ported_inventory_is_the_41_case_suite() {
    let env = corpus::ported_envelope();
    let cli = corpus::ported_cli();
    let env_ids: Vec<_> = corpus::cases(&env)
        .iter()
        .map(|c| case_id(c).to_string())
        .collect();
    let cli_ids: Vec<_> = corpus::cases(&cli)
        .iter()
        .map(|c| case_id(c).to_string())
        .collect();

    let want_env: Vec<String> = (1..=29).map(|n| format!("P-A{n:02}")).collect();
    let want_cli: Vec<String> = (1..=12).map(|n| format!("P-D{n:02}")).collect();
    assert_eq!(env_ids, want_env, "ported envelope inventory != A01..A29");
    assert_eq!(cli_ids, want_cli, "ported CLI inventory != D01..D12");
    assert_eq!(
        env_ids.len() + cli_ids.len(),
        41,
        "the proven 41-case suite"
    );

    // every ported case preserves proto evidence
    for c in corpus::cases(&env).iter().chain(corpus::cases(&cli)) {
        let pe = c.get("protoExpect").expect("protoExpect present");
        assert!(!pe.is_null(), "{}: protoExpect missing", case_id(c));
        assert!(
            pe.get("declared").and_then(Value::as_str).is_some(),
            "{}: protoExpect.declared missing",
            case_id(c)
        );
    }
}

#[test]
fn taxonomy_is_well_formed_and_tiered() {
    let manifest = corpus::manifest();
    let tax = manifest.get("taxonomy").expect("manifest.taxonomy");
    let list = |k: &str| -> BTreeSet<String> {
        tax.get(k)
            .and_then(Value::as_arr)
            .unwrap_or_else(|| panic!("taxonomy.{k}"))
            .iter()
            .map(|v| v.as_str().expect("id string").to_string())
            .collect()
    };
    let v55 = list("v55ErrorIds");
    let plan = list("planIds");
    let proto_only = list("protoOnlyIds");

    // manifest's v5.5 registry snapshot == the frozen contracts mirror
    assert_eq!(
        v55,
        contracts::v55_error_ids(),
        "manifest v55ErrorIds drifted from contracts/message_ids.yaml"
    );

    // the plan-tier id is real and is NOT yet a registered v5.5 id (OQ-1
    // tripwire: when the registry gains it, regenerate the corpus and
    // retire the plan tier)
    assert_eq!(
        plan.iter().collect::<Vec<_>>(),
        ["error.command.not_implemented"]
    );
    assert!(
        !v55.contains("error.command.not_implemented"),
        "OQ-1 resolved upstream: error.command.not_implemented is now \
         registered — regenerate the corpus and retire the plan tier"
    );

    // every referenced code is accounted for by exactly the declared tiers
    let mut referenced = BTreeSet::new();
    for (_, c) in all_cases() {
        expect::collect_codes(&c, &mut referenced);
    }
    let known: BTreeSet<_> = v55.union(&plan).cloned().collect::<BTreeSet<_>>();
    let known: BTreeSet<_> = known.union(&proto_only).cloned().collect();
    let unknown: Vec<_> = referenced.difference(&known).collect();
    assert!(unknown.is_empty(), "codes outside all tiers: {unknown:?}");
    let computed_proto_only: BTreeSet<_> = referenced
        .iter()
        .filter(|c| !v55.contains(*c) && !plan.contains(*c))
        .cloned()
        .collect();
    assert_eq!(
        computed_proto_only, proto_only,
        "manifest.protoOnlyIds must equal the codes actually referenced \
         beyond v5.5+plan (OQ-2 inventory)"
    );

    // tier field sanity on every case
    for (origin, c) in all_cases() {
        let tier = c.get("codeTier").and_then(Value::as_str);
        assert!(
            matches!(tier, Some("contract" | "proto" | "plan")),
            "{origin}/{}: bad codeTier {tier:?}",
            case_id(&c)
        );
    }
}

#[test]
fn expectation_records_are_well_formed() {
    for (origin, c) in all_cases() {
        let id = case_id(&c).to_string();
        let driver = c.get("driver").and_then(Value::as_str);
        assert!(
            matches!(driver, Some("apply" | "cli")),
            "{origin}/{id}: driver {driver:?}"
        );
        if driver == Some("apply") {
            let has_env = c.get("envelope").map(|e| !e.is_null()).unwrap_or(false);
            let has_seq = c
                .get("envelopeSequence")
                .map(|e| !e.is_null())
                .unwrap_or(false);
            assert!(
                has_env ^ has_seq,
                "{origin}/{id}: apply case needs exactly one of envelope / \
                 envelopeSequence"
            );
        } else {
            assert!(
                c.get("cli").is_some(),
                "{origin}/{id}: cli case needs a cli block"
            );
        }

        // rule-7 enforcement: provenance on every case
        let prov = c.get("provenance").and_then(Value::as_str);
        assert!(
            prov.is_some_and(|p| !p.is_empty()),
            "{origin}/{id}: provenance missing"
        );

        let runnable = c
            .get("cp01Runnable")
            .and_then(Value::as_bool)
            .unwrap_or_else(|| panic!("{origin}/{id}: cp01Runnable"));
        let cp01 = c.get("cp01Expect").expect("cp01Expect key");
        if cp01.is_null() {
            assert!(
                !runnable,
                "{origin}/{id}: null cp01Expect only on non-runnable cases"
            );
            let notes = c.get("notes").and_then(Value::as_str);
            assert!(
                notes.is_some_and(|n| !n.is_empty()),
                "{origin}/{id}: non-runnable case must explain itself"
            );
            continue;
        }
        let outcome = cp01.get("outcome").and_then(Value::as_str);
        match outcome {
            Some("accept") => {
                assert_eq!(
                    cp01.get("exit").and_then(Value::as_i64),
                    Some(0),
                    "{origin}/{id}: accept exits 0"
                );
                assert!(
                    cp01.get("code").is_none_or(Value::is_null),
                    "{origin}/{id}: accept carries no code"
                );
            }
            Some("reject") => {
                let exit = cp01.get("exit").and_then(Value::as_i64);
                let exit_any: Option<Vec<i64>> = cp01
                    .get("exitAnyOf")
                    .and_then(Value::as_arr)
                    .map(|a| a.iter().filter_map(Value::as_i64).collect());
                match (exit, &exit_any) {
                    (Some(e), None) => assert!(e == 1 || e == 2, "{origin}/{id}: reject exit {e}"),
                    (None, Some(list)) => assert!(
                        !list.is_empty() && list.iter().all(|e| *e == 1 || *e == 2),
                        "{origin}/{id}: bad exitAnyOf {list:?}"
                    ),
                    other => panic!(
                        "{origin}/{id}: reject needs exit xor exitAnyOf, \
                         got {other:?}"
                    ),
                }
                if let Some(any) = cp01.get("codeAnyOf").filter(|v| !v.is_null()) {
                    let n = any.as_arr().map(|a| a.len()).unwrap_or(0);
                    assert!(n >= 2, "{origin}/{id}: codeAnyOf needs 2+ alternatives");
                    assert!(
                        cp01.get("code").is_none_or(Value::is_null),
                        "{origin}/{id}: code and codeAnyOf are exclusive"
                    );
                }
            }
            other => panic!("{origin}/{id}: bad outcome {other:?}"),
        }
    }
}

#[test]
fn not_implemented_matrix_covers_all_kinds_but_doc_new() {
    let mut seen = 0;
    for kind in corpus::kind_file_names() {
        if kind == "doc.new" {
            continue;
        }
        let doc = corpus::kind_file(&kind);
        let cases = corpus::cases(&doc);
        assert_eq!(
            cases.len(),
            1,
            "{kind}: CP-0.1 scaffold carries exactly the not-implemented \
             case (param matrices arrive with the kind's checkpoint)"
        );
        let c = &cases[0];
        assert_eq!(case_id(c), format!("G-NI-{kind}"));
        let cp01 = c.get("cp01Expect").expect("cp01Expect");
        assert_eq!(
            cp01.get("code").and_then(Value::as_str),
            Some("error.command.not_implemented"),
            "{kind}: CP-0.1 expectation is the plan's registry rule"
        );
        assert_eq!(cp01.get("exit").and_then(Value::as_i64), Some(1));
        assert_eq!(c.get("codeTier").and_then(Value::as_str), Some("plan"));
        let env = c.get("envelope").expect("envelope");
        assert_eq!(
            env.get("schema").and_then(Value::as_str),
            Some("appname.command.v1")
        );
        let id = env.get("id").and_then(Value::as_str).expect("id");
        assert!(
            expect::is_uuid_shaped(id) && id.starts_with("a0000000-"),
            "{kind}: a0-scheme envelope id, got {id}"
        );
        seen += 1;
    }
    assert_eq!(seen, 101, "101 not-implemented kinds beside doc.new");
}

#[test]
fn doc_new_matrix_has_the_derived_shape() {
    let doc = corpus::kind_file("doc.new");
    let cases = corpus::cases(&doc);
    assert!(
        cases.len() >= 20,
        "doc.new matrix unexpectedly small: {}",
        cases.len()
    );
    let ids: BTreeSet<_> = cases.iter().map(|c| case_id(c).to_string()).collect();

    // required / bounds / unknown-field cases derived from paramsSchema
    for must in [
        "G-DN-missing-width",
        "G-DN-missing-height",
        "G-DN-width-below-min",
        "G-DN-width-above-max",
        "G-DN-height-below-min",
        "G-DN-height-above-max",
        "G-DN-resolutionPpi-below-min",
        "G-DN-resolutionPpi-above-max",
        "G-DN-unknown-param-field",
        "G-DN-accept-valid-minimal",
        "G-DN-accept-min-boundary",
    ] {
        assert!(ids.contains(must), "doc.new matrix lost {must}");
    }
    let enum_cases = ids.iter().filter(|i| i.ends_with("-bad-enum")).count();
    let type_cases = ids.iter().filter(|i| i.ends_with("-bad-type")).count();
    let contract_cases = ids
        .iter()
        .filter(|i| i.starts_with("G-DN-contract-invalid-"))
        .count();
    assert!(enum_cases >= 3, "enum coverage shrank: {enum_cases}");
    assert!(type_cases >= 7, "bad-type coverage shrank: {type_cases}");
    assert_eq!(
        contract_cases, 4,
        "the four contract examples.invalid cases port verbatim"
    );

    // the derivation table resolves
    let derivation = doc
        .get("docNewDerivation")
        .and_then(Value::as_arr)
        .expect("docNewDerivation");
    assert!(!derivation.is_empty());
    for d in derivation {
        let cid = d.get("caseId").and_then(Value::as_str).expect("caseId");
        assert!(ids.contains(cid), "derivation names missing case {cid}");
    }

    // at least one accept and every reject pins a code (doc.new is
    // implemented at CP-0.1 — no anyOf hedging inside its param matrix)
    let mut accepts = 0;
    for c in cases {
        let cp01 = c.get("cp01Expect").expect("cp01Expect");
        match cp01.get("outcome").and_then(Value::as_str) {
            Some("accept") => accepts += 1,
            Some("reject") => {
                assert!(
                    cp01.get("code")
                        .and_then(Value::as_str)
                        .is_some_and(|s| s.starts_with("error.")),
                    "{}: doc.new reject without pinned code",
                    case_id(c)
                );
            }
            other => panic!("{}: outcome {other:?}", case_id(c)),
        }
    }
    assert!(accepts >= 2, "boundary accepts belong in the matrix");
}

#[test]
fn envelope_ids_unique_except_declared_duplicates() {
    // Intentional duplicates, declared in the manifest: P-A09 reuses
    // P-A01's id (duplicate-id case), G-ENV-duplicate-id repeats its own
    // envelope inside a sequence.
    let mut by_id: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (origin, c) in all_cases() {
        let cid = case_id(&c).to_string();
        let mut push = |env: &Value| {
            if let Some(id) = env.get("id").and_then(Value::as_str) {
                // uniqueness is a JOURNAL invariant: only well-formed ids
                // can journal, so malformed-id cases (P-A07,
                // G-ENV-id-not-uuid) are outside it by construction
                if expect::is_uuid_shaped(id) {
                    by_id
                        .entry(id.to_string())
                        .or_default()
                        .push(format!("{origin}/{cid}"));
                }
            }
        };
        if let Some(env) = c.get("envelope").filter(|e| !e.is_null()) {
            push(env);
        }
        if let Some(seq) = c.get("envelopeSequence").and_then(Value::as_arr) {
            for env in seq {
                push(env);
            }
        }
    }
    for (id, holders) in &by_id {
        if holders.len() == 1 {
            continue;
        }
        let ok_pair = holders.len() == 2
            && holders.iter().any(|h| h.ends_with("/P-A01"))
            && holders.iter().any(|h| h.ends_with("/P-A09"));
        let ok_seq =
            holders.len() == 2 && holders.iter().all(|h| h.ends_with("/G-ENV-duplicate-id"));
        assert!(
            ok_pair || ok_seq,
            "undeclared duplicate envelope id {id}: {holders:?}"
        );
    }
}

#[test]
fn case_ids_globally_unique() {
    let mut seen = BTreeSet::new();
    for (origin, c) in all_cases() {
        let id = case_id(&c).to_string();
        assert!(seen.insert(id.clone()), "duplicate caseId {id} ({origin})");
    }
    assert_eq!(seen.len(), 178, "declared inventory: 178 cases");
}
