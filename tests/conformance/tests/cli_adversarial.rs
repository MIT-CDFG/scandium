// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Ported CLI driver cases (P-D01..P-D12) against the REAL `appname`
//! binary — wave-4 activation. The 8 `cp01Runnable` cases execute; the 4
//! journal-walk cases (P-D03..P-D06) stay corpus-recorded with reasons
//! (their premise needs kinds beyond doc.new; degenerate analogues run in
//! property_live.rs) — counted, never silent.
//!
//! Spec: plan §2.1 CLI verb surface (exit 0/1/2, --json one object);
//! proto machine contract REPORT-A §4.2 (structured rejections, envelope
//! id quoted, package bytes untouched, no panic); corpus
//! fixtures/adversarial/ported/cli.json is the single source for argv
//! (interpreted, placeholder-substituted) and expectations.

use appname_conformance::json::{parse, Value};
use appname_conformance::jsonspan;
use appname_conformance::{corpus, paths};
use std::path::{Path, PathBuf};
use std::process::Command;

const PANIC_MARKERS: [&str; 4] = [
    "panicked at",
    "RUST_BACKTRACE",
    "stack backtrace",
    "thread '",
];

fn cli_bin() -> PathBuf {
    // Binary-freshness guard (O-QA ratification batch, backlog closure):
    // rebuild-or-override via the shared harness door.
    appname_conformance::shipped_cli::fresh_cli_bin()
}

struct Run {
    exit: i32,
    stdout: String,
    stderr: String,
    obj: Option<Value>,
}

fn run(args: &[String], envs: &[(String, String)]) -> Run {
    let mut cmd = Command::new(cli_bin());
    cmd.args(args);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn appname");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    for m in PANIC_MARKERS {
        assert!(
            !stdout.contains(m) && !stderr.contains(m),
            "panic marker {m:?} in output:\n{stdout}\n{stderr}"
        );
    }
    Run {
        exit: out.status.code().expect("exit code"),
        obj: parse(stdout.trim()).ok(),
        stdout,
        stderr,
    }
}

/// Fresh scratch dir + the proto driver's 96x64 scratch doc.
fn scratch(tag: &str) -> (PathBuf, PathBuf) {
    let work = std::env::temp_dir().join(format!("appname-qa-cli-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).expect("mk workdir");
    let doc = work.join("doc.appname");
    let r = run(
        &[
            "new".into(),
            "--width".into(),
            "96".into(),
            "--height".into(),
            "64".into(),
            "--out".into(),
            doc.display().to_string(),
            "--json".into(),
        ],
        &[],
    );
    assert_eq!(r.exit, 0, "scratch doc: {}", r.stdout);
    (work, doc)
}

/// Corpus access: the case document, its raw text and span tree.
fn cli_corpus() -> (String, Value, jsonspan::Node) {
    let p = paths::adversarial_dir().join("ported").join("cli.json");
    let text = std::fs::read_to_string(&p).expect("read cli.json");
    let doc = parse(&text).expect("parse cli.json");
    let span = jsonspan::parse(&text);
    (text, doc, span)
}

fn case<'a>(doc: &'a Value, id: &str) -> (usize, &'a Value) {
    let cases = corpus::cases(doc);
    let i = cases
        .iter()
        .position(|c| c.get("caseId").and_then(Value::as_str) == Some(id))
        .unwrap_or_else(|| panic!("{id} missing from cli.json"));
    (i, &cases[i])
}

/// Interpret the corpus argv with placeholder substitution — the corpus
/// stays the single source of the command line.
fn argv_of(case: &Value, subs: &[(&str, String)]) -> Vec<String> {
    let argv = case
        .get("cli")
        .and_then(|c| c.get("argv"))
        .and_then(Value::as_arr)
        .expect("cli.argv");
    argv.iter()
        .map(|a| {
            let mut s = a.as_str().expect("argv strings").to_string();
            for (k, v) in subs {
                s = s.replace(k, v);
            }
            s
        })
        .collect()
}

fn expect_of(case: &Value) -> (&Value, Option<&str>) {
    let cp01 = case.get("cp01Expect").expect("cp01Expect");
    (cp01, cp01.get("code").and_then(Value::as_str))
}

fn assert_exit(case_id: &str, cp01: &Value, got: i32) {
    if let Some(want) = cp01.get("exit").and_then(Value::as_i64) {
        assert_eq!(got as i64, want, "{case_id}: exit");
    } else if let Some(any) = cp01.get("exitAnyOf").and_then(Value::as_arr) {
        assert!(
            any.iter()
                .filter_map(Value::as_i64)
                .any(|e| e == got as i64),
            "{case_id}: exit {got} not in {any:?}"
        );
    } else {
        panic!("{case_id}: no exit expectation");
    }
}

fn assert_rejection(case_id: &str, r: &Run, code_want: Option<&str>) {
    let obj = r
        .obj
        .as_ref()
        .unwrap_or_else(|| panic!("{case_id}: stdout is not one JSON object: {}", r.stdout));
    assert_eq!(
        obj.get("ok").and_then(Value::as_bool),
        Some(false),
        "{case_id}: ok:false"
    );
    let err = obj.get("error").expect("error object");
    let code = err.get("code").and_then(Value::as_str).unwrap_or("");
    assert!(
        code.starts_with("error."),
        "{case_id}: structured code, got {code:?}"
    );
    assert!(
        err.get("message")
            .and_then(Value::as_str)
            .is_some_and(|m| !m.is_empty()),
        "{case_id}: non-empty message"
    );
    if let Some(want) = code_want {
        assert_eq!(code, want, "{case_id}: pinned code");
    }
}

fn bytes_of(p: &Path) -> Vec<u8> {
    std::fs::read(p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

#[test]
fn p_d01_apply_non_json() {
    let (_, doc, _) = cli_corpus();
    let (_, c) = case(&doc, "P-D01");
    let (work, pkg) = scratch("d01");
    let before = bytes_of(&pkg);
    let argv = argv_of(c, &[("<DOC>", pkg.display().to_string())]);
    let r = run(&argv, &[]);
    let (cp01, code) = expect_of(c);
    assert_exit("P-D01", cp01, r.exit);
    assert_rejection("P-D01", &r, code);
    assert_eq!(bytes_of(&pkg), before, "P-D01: package bytes untouched");
    let _ = std::fs::remove_dir_all(work);
}

#[test]
fn p_d02_envelope_over_1mib() {
    let (_, doc, _) = cli_corpus();
    let (_, c) = case(&doc, "P-D02");
    let (work, pkg) = scratch("d02");
    // corpus files.synthesize: selection.all envelope, params.pad 1200 KiB
    let big = work.join("big.json");
    let pad = "x".repeat(1200 * 1024);
    std::fs::write(
        &big,
        format!(
            r#"{{"schema":"appname.command.v1","id":"ad00d002-0000-4000-8000-00000000d002","kind":"selection.all","params":{{"pad":"{pad}"}}}}"#
        ),
    )
    .unwrap();
    let before = bytes_of(&pkg);
    let argv = argv_of(
        c,
        &[
            ("<DOC>", pkg.display().to_string()),
            ("<WORK>", work.display().to_string()),
        ],
    );
    let r = run(&argv, &[]);
    let (cp01, code) = expect_of(c);
    assert_exit("P-D02", cp01, r.exit);
    assert_rejection("P-D02", &r, code);
    assert_eq!(bytes_of(&pkg), before, "P-D02: package bytes untouched");
    let _ = std::fs::remove_dir_all(work);
}

#[test]
fn p_d07_script_transactional() {
    let (_, doc, _) = cli_corpus();
    let (_, c) = case(&doc, "P-D07");
    let (work, pkg) = scratch("d07");
    // RULING (O-INT, stroke_path flip; mirrored in the cli.rs copy):
    // this probe is now DEDICATED NI-PATH COVERAGE — doc.duplicate is
    // the dagger kind, permanently NI BY DESIGN, so a script hitting
    // the stage-4 not_implemented path is pinned forever (the anchor
    // never exhausts). The TRANSACTIONAL invariant is DECOUPLED: see
    // the schema-failure mid-script probe below. Both flagged for
    // O-QA ratification.
    let line = r#"{"schema":"appname.command.v1","id":"ad00d701-0000-4000-8000-00000000d701","kind":"doc.duplicate","params":{}}"#;
    let bad = work.join("bad.jsonl");
    std::fs::write(&bad, format!("{line}\n{line}\n")).unwrap();
    let before = bytes_of(&pkg);
    let argv = argv_of(
        c,
        &[
            ("<DOC>", pkg.display().to_string()),
            ("<WORK>", work.display().to_string()),
        ],
    );
    let r = run(&argv, &[]);
    let (cp01, code) = expect_of(c);
    assert_exit("P-D07", cp01, r.exit);
    assert_rejection("P-D07", &r, code);
    let obj = r.obj.as_ref().unwrap();
    // CP-0.1 premise shift (corpus notes): line 1 already fails with
    // not_implemented; transactionality must hold regardless.
    assert_eq!(
        obj.get("line").and_then(Value::as_i64),
        Some(1),
        "P-D07: failing line quoted"
    );
    assert_eq!(
        obj.get("id").and_then(Value::as_str),
        Some("ad00d701-0000-4000-8000-00000000d701"),
        "P-D07: envelope id quoted"
    );
    assert_eq!(
        bytes_of(&pkg),
        before,
        "P-D07: nothing persisted on script failure"
    );
    let _ = std::fs::remove_dir_all(work);
}

#[test]
fn p_d07b_script_mid_failure_rolls_back_everything() {
    // RULING (O-INT, stroke_path flip): the transactional invariant
    // pinned to a SCHEMA failure (stage-5 out-of-range — permanently
    // stable under any future implementation) in the STRONGER
    // mid-script form: line 1 SUCCEEDS, line 2 fails, NOTHING persists
    // including line 1's landed layer. Flagged for O-QA ratification.
    let (_, doc, _) = cli_corpus();
    let (_, c) = case(&doc, "P-D07");
    let (work, pkg) = scratch("d07b");
    let ok_line = r#"{"schema":"appname.command.v1","id":"ad00d701-0000-4000-8000-00000000d7b1","kind":"layer.add","params":{"kind":"raster","newLayerId":"ad00d701-0000-4000-8000-00000000d7aa","name":"doomed"}}"#;
    let bad_line = r#"{"schema":"appname.command.v1","id":"ad00d701-0000-4000-8000-00000000d7b2","kind":"doc.resize","params":{"width":-5}}"#;
    std::fs::write(work.join("bad.jsonl"), format!("{ok_line}\n{bad_line}\n")).unwrap();
    let before = bytes_of(&pkg);
    let argv = argv_of(
        c,
        &[
            ("<DOC>", pkg.display().to_string()),
            ("<WORK>", work.display().to_string()),
        ],
    );
    let r = run(&argv, &[]);
    assert_eq!(r.exit, 1, "P-D07b: reject exit");
    let obj = r.obj.as_ref().unwrap();
    assert_eq!(
        obj.get("error")
            .and_then(|e| e.get("code"))
            .and_then(Value::as_str),
        Some("error.schema.out_of_range"),
        "P-D07b: the schema failure is the anchor"
    );
    assert_eq!(
        obj.get("line").and_then(Value::as_i64),
        Some(2),
        "P-D07b: line 2 is the failing line (line 1 SUCCEEDED first)"
    );
    assert_eq!(
        bytes_of(&pkg),
        before,
        "P-D07b: mid-script rollback — line 1's layer must NOT persist"
    );
    let _ = std::fs::remove_dir_all(work);
}

#[test]
fn p_d08_info_missing_package() {
    let (_, doc, _) = cli_corpus();
    let (_, c) = case(&doc, "P-D08");
    let work = std::env::temp_dir().join(format!("appname-qa-cli-d08-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).unwrap();
    let argv = argv_of(c, &[("<WORK>", work.display().to_string())]);
    let r = run(&argv, &[]);
    let (cp01, code) = expect_of(c);
    assert_exit("P-D08", cp01, r.exit);
    assert_rejection("P-D08", &r, code);
    let _ = std::fs::remove_dir_all(work);
}

#[test]
fn p_d09_import_non_image() {
    let (_, doc, _) = cli_corpus();
    let (_, c) = case(&doc, "P-D09");
    let (work, pkg) = scratch("d09");
    std::fs::write(
        work.join("not_an_image.txt"),
        "plain text, no image magic bytes\n",
    )
    .unwrap();
    let before = bytes_of(&pkg);
    let argv = argv_of(
        c,
        &[
            ("<DOC>", pkg.display().to_string()),
            ("<WORK>", work.display().to_string()),
        ],
    );
    let r = run(&argv, &[]);
    let (cp01, code) = expect_of(c);
    assert_exit("P-D09", cp01, r.exit);
    assert_rejection("P-D09", &r, code);
    // record the observed class for the lane log (corpus keeps it unpinned)
    let obj = r.obj.as_ref().unwrap();
    eprintln!(
        "P-D09 observed: exit {} code {}",
        r.exit,
        obj.get("error")
            .and_then(|e| e.get("code"))
            .and_then(Value::as_str)
            .unwrap_or("?")
    );
    assert_eq!(bytes_of(&pkg), before, "P-D09: package bytes untouched");
    let _ = std::fs::remove_dir_all(work);
}

#[test]
fn p_d10_layer_delete_last_layer() {
    let (text, doc, span) = cli_corpus();
    let (ci, c) = case(&doc, "P-D10");
    let work = std::env::temp_dir().join(format!("appname-qa-cli-d10-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).unwrap();

    let steps = c
        .get("cli")
        .and_then(|x| x.get("steps"))
        .and_then(Value::as_arr)
        .expect("steps");

    // step 0: new 16x16, capture receipt.createdIds[-1] per the corpus spec
    let argv0: Vec<String> = steps[0]
        .get("argv")
        .and_then(Value::as_arr)
        .unwrap()
        .iter()
        .map(|a| {
            a.as_str()
                .unwrap()
                .replace("<WORK>", &work.display().to_string())
        })
        .collect();
    let r0 = run(&argv0, &[]);
    assert_eq!(r0.exit, 0, "P-D10 setup: {}", r0.stdout);
    let capture_spec = steps[0]
        .get("capture")
        .and_then(|x| x.get("LAYER_ID_FROM_NEW"))
        .and_then(Value::as_str)
        .unwrap();
    assert_eq!(
        capture_spec, "receipt.createdIds[-1]",
        "corpus capture spec"
    );
    let created = r0
        .obj
        .as_ref()
        .unwrap()
        .get("receipt")
        .and_then(|x| x.get("createdIds"))
        .and_then(Value::as_arr)
        .expect("createdIds");
    let layer_id = created.last().unwrap().as_str().unwrap().to_string();

    // step 1: apply the corpus envelope (raw slice, placeholder filled)
    let env_node = span
        .get("cases")
        .and_then(|x| x.idx(ci))
        .and_then(|x| x.get("cli"))
        .and_then(|x| x.get("steps"))
        .and_then(|x| x.idx(1))
        .and_then(|x| x.get("envelope"))
        .expect("envelope span");
    let env_raw = env_node
        .raw(&text)
        .replace("<LAYER_ID_FROM_NEW>", &layer_id);
    let argv1: Vec<String> = steps[1]
        .get("argv")
        .and_then(Value::as_arr)
        .unwrap()
        .iter()
        .map(|a| {
            a.as_str()
                .unwrap()
                .replace("<WORK>", &work.display().to_string())
                .replace("<ENVELOPE>", &env_raw)
        })
        .collect();
    let r1 = run(&argv1, &[]);
    let (cp01, code) = expect_of(c);
    assert_exit("P-D10", cp01, r1.exit);
    assert_rejection("P-D10", &r1, code);
    assert_eq!(
        r1.obj.as_ref().unwrap().get("id").and_then(Value::as_str),
        Some("ad00d100-0000-4000-8000-00000000d100"),
        "P-D10: envelope id quoted"
    );
    let _ = std::fs::remove_dir_all(work);
}

#[test]
fn p_d11_png_export_with_quality() {
    let (_, doc, _) = cli_corpus();
    let (_, c) = case(&doc, "P-D11");
    let (work, pkg) = scratch("d11");
    let argv = argv_of(
        c,
        &[
            ("<DOC>", pkg.display().to_string()),
            ("<WORK>", work.display().to_string()),
        ],
    );
    let r = run(&argv, &[]);
    let (cp01, code) = expect_of(c);
    assert_exit("P-D11", cp01, r.exit);
    assert_rejection("P-D11", &r, code);
    let _ = std::fs::remove_dir_all(work);
}

#[test]
fn p_d12_human_mode_localized_rejection() {
    let (text, doc, span) = cli_corpus();
    let (ci, c) = case(&doc, "P-D12");
    let (work, pkg) = scratch("d12");
    let env_node = span
        .get("cases")
        .and_then(|x| x.idx(ci))
        .and_then(|x| x.get("cli"))
        .and_then(|x| x.get("envelope"))
        .expect("envelope span");
    let env_raw = env_node.raw(&text).to_string();
    let i18n_dir = paths::repo_root()
        .join("crates")
        .join("appname-cli")
        .join("i18n");
    assert!(i18n_dir.is_dir(), "CLI i18n catalogs missing");
    let argv = argv_of(
        c,
        &[
            ("<DOC>", pkg.display().to_string()),
            ("<ENVELOPE>", env_raw),
        ],
    );
    let r = run(
        &argv,
        &[(
            "APPNAME_I18N_DIR".to_string(),
            i18n_dir.display().to_string(),
        )],
    );
    let (cp01, code) = expect_of(c);
    assert_exit("P-D12", cp01, r.exit);
    // human mode: stdout stays EMPTY (proto driver assertion,
    // appname-proto/scripts/adversarial.py D12) — the rejection goes to
    // stderr only
    assert!(
        r.stdout.is_empty(),
        "P-D12: human-mode stdout must be empty, got: {}",
        r.stdout
    );
    // and exactly ONE stderr line carrying the wire code and the
    // envelope id (wire constants are never translated)
    let lines: Vec<&str> = r.stderr.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 1, "P-D12: one localized line, got {lines:?}");
    let line = lines[0];
    assert!(
        line.contains(code.unwrap()),
        "P-D12: wire code in line: {line}"
    );
    assert!(
        line.contains("ad00d120-0000-4000-8000-00000000d120"),
        "P-D12: envelope id quoted in line: {line}"
    );
    let _ = std::fs::remove_dir_all(work);
}

/// Honest accounting for the 4 non-runnable CLI cases: premise + reason
/// stay declared in the corpus; their degenerate analogues run in
/// property_live.rs (cursor invariants).
#[test]
fn non_runnable_cli_cases_are_exactly_the_declared_four() {
    let (_, doc, _) = cli_corpus();
    let skipped: Vec<String> = corpus::cases(&doc)
        .iter()
        .filter(|c| {
            !c.get("cp01Runnable")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .map(|c| {
            let id = c.get("caseId").and_then(Value::as_str).unwrap();
            assert!(
                c.get("notes")
                    .and_then(Value::as_str)
                    .is_some_and(|n| !n.is_empty()),
                "{id}: non-runnable case must carry its reason"
            );
            id.to_string()
        })
        .collect();
    assert_eq!(skipped, ["P-D03", "P-D04", "P-D05", "P-D06"]);
}
