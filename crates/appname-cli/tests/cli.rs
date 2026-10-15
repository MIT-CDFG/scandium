// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! CLI integration tests: the real `appname` binary, spawned per case, with
//! the corpus D-series expectations (fixtures/adversarial/ported/cli.json)
//! as the reference bar for exit classes and rejection shapes.
//! Exit discipline (proto-pinned): 0 ok, 1 command rejected, 2 IO error;
//! clap usage errors exit 2 (clap default).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_appname")
}

fn work_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("appname-cli-test-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(args: &[&str]) -> Output {
    Command::new(bin())
        .args(args)
        .output()
        .expect("binary spawns")
}

fn run_env(args: &[&str], envs: &[(&str, &str)]) -> Output {
    let mut c = Command::new(bin());
    c.args(args);
    for (k, v) in envs {
        c.env(k, v);
    }
    c.output().expect("binary spawns")
}

fn exit_code(o: &Output) -> i32 {
    o.status.code().expect("no signal")
}

fn stdout_json(o: &Output) -> serde_json::Value {
    let text = String::from_utf8_lossy(&o.stdout);
    serde_json::from_str(text.trim()).unwrap_or_else(|e| panic!("stdout not JSON: {e}\n{text}"))
}

fn new_doc(dir: &Path, name: &str) -> PathBuf {
    let doc = dir.join(name);
    let o = run(&[
        "new",
        "--width",
        "64",
        "--height",
        "48",
        "--out",
        doc.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(
        exit_code(&o),
        0,
        "stderr: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    doc
}

/// QA-F8 regression (the W16 swap repro, verbatim flow): `appname export`
/// must SERVE smart-object content — byte-stable across
/// `smartobject.convert`, and CHANGED when the embedded content changes
/// (the silent-omit bug exported byte-identical PNGs across a content
/// swap because only `pixels()`-bearing layers were counted).
#[test]
fn export_serves_smart_content_and_changes_across_a_content_swap() {
    let dir = work_dir("qa-f8-smart-export");
    let doc = dir.join("doc.appname");
    let o = run(&[
        "new",
        "--width",
        "64",
        "--height",
        "64",
        "--background",
        "white",
        "--out",
        doc.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(exit_code(&o), 0);
    let d = doc.to_str().unwrap();
    let lid = "c0000000-0000-4000-8000-00000000bbb1";
    let sol = "c0000000-0000-4000-8000-00000000bbb2";
    let apply = |cmd: String| {
        let o = run(&["apply", "--doc", d, "--json", "--command", &cmd]);
        assert_eq!(
            exit_code(&o),
            0,
            "apply failed: {}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        );
        stdout_json(&o)
    };
    apply(format!(
        r#"{{"schema":"appname.command.v1","id":"c0000000-0000-4000-8000-00000000aaa1","kind":"layer.add","params":{{"kind":"raster","newLayerId":"{lid}","name":"art"}}}}"#
    ));
    apply(format!(
        r#"{{"schema":"appname.command.v1","id":"c0000000-0000-4000-8000-00000000aaa2","kind":"fill","target":{{"layerId":"{lid}"}},"params":{{"content":"foreground"}}}}"#
    ));
    let export = |name: &str| -> Vec<u8> {
        let out = dir.join(name);
        let o = run(&[
            "export",
            "--doc",
            d,
            "--format",
            "png",
            "--out",
            out.to_str().unwrap(),
            "--json",
        ]);
        assert_eq!(
            exit_code(&o),
            0,
            "export failed: {}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        );
        std::fs::read(&out).unwrap()
    };
    let pre = export("pre.png");
    apply(format!(
        r#"{{"schema":"appname.command.v1","id":"c0000000-0000-4000-8000-00000000aaa3","kind":"smartobject.convert","params":{{"layerIds":["{lid}"],"newLayerId":"{sol}"}}}}"#
    ));
    let conv = export("conv.png");
    assert_eq!(
        pre, conv,
        "convert preserves the look — smart content SERVED at the export door"
    );
    // Swap the embedded content (QA's exact repro: blue-64 fixture).
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/w16/blue-64.png");
    let o = run(&[
        "asset",
        "add-image",
        "--doc",
        d,
        "--file",
        fixture.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(exit_code(&o), 0);
    let blue = stdout_json(&o)["assetId"].as_str().unwrap().to_string();
    let convert_asset = "ef89c10d-7f6b-5459-aeaa-ed4bc08828f4"; // content-derived (W16)
    apply(format!(
        r#"{{"schema":"appname.command.v1","id":"c0000000-0000-4000-8000-00000000aaa5","kind":"smartobject.edit_contents","target":{{"layerId":"{sol}"}},"params":{{"assetBefore":"{convert_asset}","assetAfter":"{blue}"}}}}"#
    ));
    let post = export("post.png");
    assert_ne!(
        post, conv,
        "REGRESSION QA-F8: export must change when smart content changes \
         (the silent-omit bug exported byte-identical PNGs across the swap)"
    );
    // Determinism: the served export is stable across re-export.
    assert_eq!(post, export("post2.png"));
}

/// Reviewer MAJOR (QA-F8 adjacent): the §40.2 geometry walks enumerate
/// `raster_surfaces()`, which has no smart-object arm — `doc.resize`,
/// `doc.rotate`, and anchored `doc.canvas_resize` would silently EXCLUDE
/// the smart proxy from the transform. The three shapes must REFUSE TYPED
/// (`error.command.not_implemented`, naming the §18.2 placement row) on a
/// smart-bearing document and leave it untouched; a smart-free document
/// keeps current behavior byte-for-byte (all three succeed).
#[test]
fn geometry_kinds_refuse_typed_on_smart_documents_and_pass_without() {
    let three_shapes = |n: u32| {
        [
            format!(
                r#"{{"schema":"appname.command.v1","id":"c0000000-0000-4000-8000-0000{n}000ee01","kind":"doc.resize","params":{{"width":16}}}}"#
            ),
            format!(
                r#"{{"schema":"appname.command.v1","id":"c0000000-0000-4000-8000-0000{n}000ee02","kind":"doc.canvas_resize","params":{{"width":32,"height":32,"anchor":"center"}}}}"#
            ),
            format!(
                r#"{{"schema":"appname.command.v1","id":"c0000000-0000-4000-8000-0000{n}000ee03","kind":"doc.rotate","params":{{"angleDeg":90}}}}"#
            ),
        ]
    };

    // Smart-bearing document: 8x8 white background converted in place.
    let dir = work_dir("qa-f8-geometry-guard");
    let doc = dir.join("smart.appname");
    let o = run(&[
        "new",
        "--width",
        "8",
        "--height",
        "8",
        "--background",
        "white",
        "--out",
        doc.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(exit_code(&o), 0);
    let d = doc.to_str().unwrap();
    let o = run(&["info", "--doc", d, "--json"]);
    let bg = stdout_json(&o)["info"]["layers"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let o = run(&[
        "apply",
        "--doc",
        d,
        "--json",
        "--command",
        &format!(
            r#"{{"schema":"appname.command.v1","id":"c0000000-0000-4000-8000-00000000ccc1","kind":"smartobject.convert","params":{{"layerIds":["{bg}"],"newLayerId":"c0000000-0000-4000-8000-00000000ccc2"}}}}"#
        ),
    ]);
    assert_eq!(exit_code(&o), 0, "{}", String::from_utf8_lossy(&o.stdout));
    let bytes_before = std::fs::read(&doc).unwrap();

    for cmd in three_shapes(1) {
        let o = run(&["apply", "--doc", d, "--json", "--command", &cmd]);
        assert_eq!(exit_code(&o), 1, "must refuse: {cmd}");
        let v = stdout_json(&o);
        assert_eq!(
            v["error"]["code"], "error.command.not_implemented",
            "typed refusal expected, got: {v}"
        );
        let detail = v["error"]["message"].as_str().unwrap_or_default();
        assert!(
            detail.contains("18.2"),
            "refusal names the §18.2 placement row: {detail}"
        );
    }
    assert_eq!(
        std::fs::read(&doc).unwrap(),
        bytes_before,
        "refusals journal nothing and mutate nothing (package bytes stable)"
    );
    // Origin-anchor canvas growth moves no document-space coordinate and
    // stays SERVED on the smart document.
    let o = run(&[
        "apply",
        "--doc",
        d,
        "--json",
        "--command",
        r#"{"schema":"appname.command.v1","id":"c0000000-0000-4000-8000-00000000cce4","kind":"doc.canvas_resize","params":{"width":12,"height":12,"anchor":"top_left"}}"#,
    ]);
    assert_eq!(
        exit_code(&o),
        0,
        "top_left anchor passes: {}",
        String::from_utf8_lossy(&o.stdout)
    );

    // Smart-free document: the same three shapes keep current behavior.
    let plain = dir.join("plain.appname");
    let o = run(&[
        "new",
        "--width",
        "8",
        "--height",
        "8",
        "--background",
        "white",
        "--out",
        plain.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(exit_code(&o), 0);
    let p = plain.to_str().unwrap();
    for cmd in three_shapes(2) {
        let o = run(&["apply", "--doc", p, "--json", "--command", &cmd]);
        assert_eq!(
            exit_code(&o),
            0,
            "smart-free doc keeps current behavior: {cmd}\n{}",
            String::from_utf8_lossy(&o.stdout)
        );
    }
}

#[test]
fn new_then_info_round_trip_with_json_shapes() {
    let dir = work_dir("new-info");
    let doc = new_doc(&dir, "a.appname");
    let o = run(&["info", "--doc", doc.to_str().unwrap(), "--json"]);
    assert_eq!(exit_code(&o), 0);
    let v = stdout_json(&o);
    assert_eq!(v["ok"], true);
    assert_eq!(v["info"]["width"], 64);
    assert_eq!(v["info"]["height"], 48);
    assert_eq!(v["info"]["layers"].as_array().unwrap().len(), 1);
    assert_eq!(v["info"]["journal"]["length"], 1);
    assert_eq!(v["info"]["journal"]["cursor"], 1);
    assert_eq!(
        v["info"]["journal"]["canUndo"], false,
        "doc.new not undoable across the boundary"
    );
    assert_eq!(v["info"]["warnings"], 0);
}

#[test]
fn p_d01_non_json_command_rejects_exit_1_with_shape() {
    let dir = work_dir("d01");
    let doc = new_doc(&dir, "a.appname");
    let o = run(&[
        "apply",
        "--doc",
        doc.to_str().unwrap(),
        "--command",
        "this is not json",
        "--json",
    ]);
    assert_eq!(exit_code(&o), 1);
    let v = stdout_json(&o);
    assert_eq!(v["ok"], false);
    assert!(v["error"]["code"].as_str().unwrap().starts_with("error."));
    assert!(v["error"]["message"].is_string());
}

#[test]
fn p_d02_oversize_envelope_via_file_rejects_exit_1() {
    let dir = work_dir("d02");
    let doc = new_doc(&dir, "a.appname");
    let big = dir.join("big.json");
    let pad = "x".repeat(1024 * 1024);
    std::fs::write(
        &big,
        format!("{{\"schema\":\"appname.command.v1\",\"id\":\"d0000000-0000-4000-8000-000000000001\",\"kind\":\"doc.new\",\"params\":{{\"pad\":\"{pad}\"}}}}"),
    )
    .unwrap();
    let o = run(&[
        "apply",
        "--doc",
        doc.to_str().unwrap(),
        "--file",
        big.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(exit_code(&o), 1);
    let v = stdout_json(&o);
    assert_eq!(v["ok"], false);
}

#[test]
fn p_d07_script_is_transactional_failing_line_persists_nothing() {
    let dir = work_dir("d07");
    let doc = new_doc(&dir, "a.appname");
    let before = std::fs::read(&doc).unwrap();
    let script = dir.join("ops.jsonl");
    // RULING (O-INT, stroke_path flip): this probe is now DEDICATED
    // NI-PATH COVERAGE — doc.duplicate is the dagger kind, permanently
    // NI BY DESIGN, so a script hitting the stage-4 not_implemented
    // path is pinned here forever (no future flip re-pins this
    // anchor). The TRANSACTIONAL invariant itself is DECOUPLED from
    // the NI inventory: see the schema-failure mid-script probe below
    // (p_d07b), which asserts the stronger rollback property. Both
    // flagged for O-QA ratification.
    std::fs::write(
        &script,
        "{\"schema\":\"appname.command.v1\",\"id\":\"d0000000-0000-4000-8000-000000000002\",\"kind\":\"doc.duplicate\"}\n",
    )
    .unwrap();
    let o = run(&[
        "script",
        "--doc",
        doc.to_str().unwrap(),
        "--file",
        script.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(exit_code(&o), 1);
    let v = stdout_json(&o);
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["code"], "error.command.not_implemented");
    assert_eq!(v["line"], 1);
    let after = std::fs::read(&doc).unwrap();
    assert_eq!(before, after, "package bytes untouched on script failure");
}

#[test]
fn p_d07b_script_mid_failure_rolls_back_everything() {
    // RULING (O-INT, stroke_path flip): the TRANSACTIONAL invariant
    // pinned to a SCHEMA failure — permanently stable under any future
    // kind implementation — and strengthened to the MID-SCRIPT form:
    // line 1 SUCCEEDS (a real layer lands), line 2 fails stage-5
    // (widthPx out of range), and NOTHING persists, INCLUDING line 1's
    // effect. Flagged for O-QA ratification.
    let dir = work_dir("d07b");
    let doc = new_doc(&dir, "a.appname");
    let before = std::fs::read(&doc).unwrap();
    let script = dir.join("ops.jsonl");
    std::fs::write(
        &script,
        concat!(
            "{\"schema\":\"appname.command.v1\",\"id\":\"d0000000-0000-4000-8000-00000000b001\",\"kind\":\"layer.add\",\"params\":{\"kind\":\"raster\",\"newLayerId\":\"d0000000-0000-4000-8000-00000000b0aa\",\"name\":\"doomed\"}}\n",
            "{\"schema\":\"appname.command.v1\",\"id\":\"d0000000-0000-4000-8000-00000000b002\",\"kind\":\"doc.resize\",\"params\":{\"width\":-5}}\n",
        ),
    )
    .unwrap();
    let o = run(&[
        "script",
        "--doc",
        doc.to_str().unwrap(),
        "--file",
        script.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(exit_code(&o), 1);
    let v = stdout_json(&o);
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["code"], "error.schema.out_of_range");
    assert_eq!(v["line"], 2, "the FAILING line is line 2");
    let after = std::fs::read(&doc).unwrap();
    assert_eq!(
        before, after,
        "MID-SCRIPT rollback: line 1's landed layer must NOT persist"
    );
}

#[test]
fn p_d08_info_on_missing_package_is_io_class_exit_2() {
    let dir = work_dir("d08");
    let missing = dir.join("nope.appname");
    let o = run(&["info", "--doc", missing.to_str().unwrap(), "--json"]);
    assert_eq!(exit_code(&o), 2);
    let v = stdout_json(&o);
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["code"], "error.io.read_failed");
}

#[test]
fn p_d09_import_non_image_rejects_with_structured_error() {
    let dir = work_dir("d09");
    let doc = new_doc(&dir, "a.appname");
    let not_image = dir.join("notes.txt");
    std::fs::write(&not_image, "plain text").unwrap();
    let o = run(&[
        "import",
        "--doc",
        doc.to_str().unwrap(),
        "--image",
        not_image.to_str().unwrap(),
        "--json",
    ]);
    let code = exit_code(&o);
    assert!(
        code == 1 || code == 2,
        "corpus allows either rejection class, got {code}"
    );
    let v = stdout_json(&o);
    assert_eq!(v["ok"], false);
    // Journaled import (CP-0.5 wave 1): the file READS fine, then the
    // decode-validated raster-ingestion door refuses → command-rejection
    // class (the corpus deliberately leaves the class unpinned: "exit=1|2
    // code=*"; the proto's io class came from its decode-at-read shape).
    assert_eq!(v["error"]["code"], "error.command.bad_target");
    assert_eq!(code, 1, "command-rejection class at the ingestion door");
}

#[test]
fn p_d11_png_export_with_quality_rejects_exit_1_before_any_engine_call() {
    let dir = work_dir("d11");
    // The doc path need not even exist: CLI-level validation fires first.
    let doc = dir.join("absent.appname");
    let out = dir.join("out.png");
    let o = run(&[
        "export",
        "--doc",
        doc.to_str().unwrap(),
        "--format",
        "png",
        "--quality",
        "90",
        "--out",
        out.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(exit_code(&o), 1);
    let v = stdout_json(&o);
    assert_eq!(v["ok"], false);
    assert!(v["error"]["message"]
        .as_str()
        .unwrap()
        .contains("--quality"));
}

#[test]
fn p_d12_lang_es_rejection_is_one_localized_stderr_line() {
    let dir = work_dir("d12");
    let doc = new_doc(&dir, "a.appname");
    let i18n_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("i18n");
    let envelope = "{\"schema\":\"appname.command.v1\",\"id\":\"d0000000-0000-4000-8000-000000000003\",\"kind\":\"zzz.not_a_kind\"}";
    let o = run_env(
        &[
            "--lang",
            "es",
            "apply",
            "--doc",
            doc.to_str().unwrap(),
            "--command",
            envelope,
        ],
        &[("APPNAME_I18N_DIR", i18n_dir.to_str().unwrap())],
    );
    assert_eq!(exit_code(&o), 1);
    assert!(
        o.stdout.is_empty(),
        "human mode prints rejections to stderr only"
    );
    let stderr = String::from_utf8_lossy(&o.stderr);
    let lines: Vec<&str> = stderr.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 1, "exactly one stderr line: {stderr:?}");
    // The wire code is never translated; the surrounding text is the
    // catalog's (the seed es template happens to read "Error {code}: …" —
    // same word in both languages — so the language-switch proof below uses
    // catalogs whose templates visibly differ).
    assert!(lines[0].contains("error.command.unknown_kind"));

    // --lang switching provably drives the catalog: ja and pseudo differ
    // from the es/en rendering of the same rejection.
    for (lang, marker) in [("ja", "エラー"), ("pseudo", "[!!!")] {
        let o2 = run_env(
            &[
                "--lang",
                lang,
                "apply",
                "--doc",
                doc.to_str().unwrap(),
                "--command",
                envelope,
            ],
            &[("APPNAME_I18N_DIR", i18n_dir.to_str().unwrap())],
        );
        assert_eq!(exit_code(&o2), 1);
        let line = String::from_utf8_lossy(&o2.stderr).trim().to_string();
        assert!(line.contains(marker), "{lang}: {line}");
        assert_ne!(line, lines[0], "{lang} renders differently from es");
    }

    // No catalogs anywhere on disk: the EMBEDDED en catalog answers (review
    // minor 3 — human output never key-echoes for en), rendered through the
    // en template rather than echoing the id.
    let o3 = run_env(
        &[
            "--lang",
            "es",
            "apply",
            "--doc",
            doc.to_str().unwrap(),
            "--command",
            envelope,
        ],
        &[(
            "APPNAME_I18N_DIR",
            dir.join("no-catalogs-here").to_str().unwrap(),
        )],
    );
    let fallback = String::from_utf8_lossy(&o3.stderr);
    assert!(
        fallback.starts_with("Error error.command.unknown_kind"),
        "embedded en fallback: {fallback}"
    );
    assert!(
        !fallback.contains("cli.error"),
        "no key-echo with the embedded catalog: {fallback}"
    );
}

#[test]
fn i18n_resolves_from_the_executable_adjacent_directory() {
    // Review minor 3: without APPNAME_I18N_DIR (and regardless of CWD — the
    // proto's CWD-relative default is the recorded divergence), catalogs
    // resolve next to the binary.
    let dir = work_dir("exe-adjacent");
    let doc = new_doc(&dir, "a.appname");
    let exe_i18n = Path::new(bin()).parent().unwrap().join("i18n");
    std::fs::create_dir_all(&exe_i18n).unwrap();
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("i18n/ja.json");
    std::fs::copy(&src, exe_i18n.join("ja.json")).unwrap();

    let envelope = "{\"schema\":\"appname.command.v1\",\"id\":\"d0000000-0000-4000-8000-000000000004\",\"kind\":\"zzz.not_a_kind\"}";
    let mut c = Command::new(bin());
    c.args([
        "--lang",
        "ja",
        "apply",
        "--doc",
        doc.to_str().unwrap(),
        "--command",
        envelope,
    ]);
    c.env_remove("APPNAME_I18N_DIR");
    c.current_dir(&dir); // CWD holds no i18n/ — exe-adjacent must win
    let o = c.output().unwrap();
    assert_eq!(exit_code(&o), 1);
    let line = String::from_utf8_lossy(&o.stderr);
    assert!(
        line.contains("エラー"),
        "exe-adjacent ja catalog resolved: {line}"
    );
    let _ = std::fs::remove_file(exe_i18n.join("ja.json"));
}

#[test]
fn undo_on_doc_new_only_package_rejects_and_redo_has_nothing() {
    let dir = work_dir("undo");
    let doc = new_doc(&dir, "a.appname");
    let o = run(&["undo", "--doc", doc.to_str().unwrap(), "--json"]);
    assert_eq!(exit_code(&o), 1);
    let v = stdout_json(&o);
    assert_eq!(v["error"]["code"], "error.history.nothing_to_undo");
    assert!(v["error"]["message"].as_str().unwrap().contains("doc.new"));

    let o = run(&["redo", "--doc", doc.to_str().unwrap(), "--json"]);
    assert_eq!(exit_code(&o), 1);
    let v = stdout_json(&o);
    assert_eq!(v["error"]["code"], "error.history.nothing_to_redo");
}

/// q-scenario/Q-9 regression: `undo --steps N` must move the UNDO STACK,
/// not the journal index. A `history.snapshot` is a dagger MARKER -- it
/// occupies a journal slot and no stack slot (sec 40.1) -- so with one
/// newest, the old `target = cursor - steps` arithmetic named a position
/// the engine deliberately cannot occupy. The two convergence loops undid
/// a real step and redid it, landing back where they started, while
/// `performed` (computed BEFORE the move) still reported the full count:
/// ok:true with an explicit FALSE COUNT and nothing changed.
///
/// Pinned here as the DISHONEST-SUCCESS shape, because every cheap signal
/// said pass: exit 0, ok:true, performed:1.
fn marker_doc(dir: &Path, name: &str) -> PathBuf {
    let doc = new_doc(dir, name);
    let lid = "feedfeed-0000-4000-8000-000000000001";
    let ops = dir.join(format!("{name}.ops.jsonl"));
    let lines = format!(
        concat!(
            r#"{{"schema":"appname.command.v1","id":"00000000-0009-4000-8000-000000000001","kind":"layer.add","params":{{"kind":"raster","newLayerId":"{lid}"}}}}"#,
            "\n",
            r#"{{"schema":"appname.command.v1","id":"00000000-0009-4000-8000-000000000002","kind":"fill","target":{{"layerId":"{lid}"}},"params":{{"content":"foreground"}}}}"#,
            "\n",
            r#"{{"schema":"appname.command.v1","id":"00000000-0009-4000-8000-000000000003","kind":"history.snapshot","params":{{}}}}"#,
            "\n",
        ),
        lid = lid
    );
    std::fs::write(&ops, lines).unwrap();
    let o = run(&[
        "script",
        "--doc",
        doc.to_str().unwrap(),
        "--file",
        ops.to_str().unwrap(),
    ]);
    assert_eq!(
        exit_code(&o),
        0,
        "fixture script failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    doc
}

#[test]
fn q9_undo_one_step_over_a_tail_marker_actually_moves_and_counts_truthfully() {
    let dir = work_dir("q9undo");
    let doc = marker_doc(&dir, "a.appname");
    let d = doc.to_str().unwrap();

    // Precondition: the marker is newest and the cursor has absorbed it.
    let v = stdout_json(&run(&["history", "--doc", d, "--json"]));
    assert_eq!(v["cursor"], 4, "fixture precondition");
    assert_eq!(v["entries"][3]["kind"], "history.snapshot");
    assert!(
        v["entries"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["active"] == true),
        "fixture precondition: all active"
    );

    // ONE step must undo the newest REAL step (the fill), skipping the
    // marker -- landing at cursor 2, NOT at the unoccupiable 3.
    let o = run(&["undo", "--doc", d, "--steps", "1", "--json"]);
    assert_eq!(exit_code(&o), 0);
    let v = stdout_json(&o);
    assert_eq!(v["ok"], true);
    assert_eq!(v["performed"], 1, "performed must count what MOVED");
    assert_eq!(v["cursor"], 2, "the cursor must actually move (Q-9)");
    assert_eq!(v["entries"][2]["active"], false, "the fill must be undone");

    // The undo genuinely happened, so redo must now be available. Pre-fix
    // this returned error.history.nothing_to_redo -- the tell that the
    // reported success was false.
    let o = run(&["redo", "--doc", d, "--json"]);
    assert_eq!(
        exit_code(&o),
        0,
        "redo must be available after a real undo: {}",
        String::from_utf8_lossy(&o.stderr)
    );
}

#[test]
fn q9_undo_two_steps_over_a_tail_marker_does_two_real_steps() {
    let dir = work_dir("q9undo2");
    let doc = marker_doc(&dir, "b.appname");
    let d = doc.to_str().unwrap();

    // Pre-fix this reported performed:2 while undoing ONE real step (the
    // marker was counted as the second), leaving layer.add still active.
    let o = run(&["undo", "--doc", d, "--steps", "2", "--json"]);
    assert_eq!(exit_code(&o), 0);
    let v = stdout_json(&o);
    assert_eq!(v["performed"], 2);
    assert_eq!(v["cursor"], 1, "two REAL steps: fill and layer.add");
    assert_eq!(v["entries"][1]["active"], false, "layer.add must be undone");
    assert_eq!(v["entries"][2]["active"], false, "fill must be undone");
}

#[test]
fn q9_positive_control_no_marker_undo_is_unchanged() {
    // The same document WITHOUT the snapshot: this path was always correct
    // and must stay correct, so the fix is shown to be marker-specific and
    // not a blanket change to undo.
    let dir = work_dir("q9ctl");
    let doc = new_doc(&dir, "c.appname");
    let d = doc.to_str().unwrap();
    let lid = "feedfeed-0000-4000-8000-000000000002";
    let ops = dir.join("c.ops.jsonl");
    let lines = format!(
        concat!(
            r#"{{"schema":"appname.command.v1","id":"00000000-0009-4000-8000-000000000011","kind":"layer.add","params":{{"kind":"raster","newLayerId":"{lid}"}}}}"#,
            "\n",
            r#"{{"schema":"appname.command.v1","id":"00000000-0009-4000-8000-000000000012","kind":"fill","target":{{"layerId":"{lid}"}},"params":{{"content":"foreground"}}}}"#,
            "\n",
        ),
        lid = lid
    );
    std::fs::write(&ops, lines).unwrap();
    assert_eq!(
        exit_code(&run(&[
            "script",
            "--doc",
            d,
            "--file",
            ops.to_str().unwrap()
        ])),
        0
    );
    let o = run(&["undo", "--doc", d, "--steps", "1", "--json"]);
    assert_eq!(exit_code(&o), 0);
    let v = stdout_json(&o);
    assert_eq!(v["performed"], 1);
    assert_eq!(v["cursor"], 2);
    assert_eq!(v["entries"][2]["active"], false);
}

#[test]
fn export_png_twice_is_byte_identical_and_reexport_after_reopen_too() {
    let dir = work_dir("export");
    let doc = new_doc(&dir, "a.appname");
    let out1 = dir.join("one.png");
    let out2 = dir.join("two.png");
    assert_eq!(
        exit_code(&run(&[
            "export",
            "--doc",
            doc.to_str().unwrap(),
            "--format",
            "png",
            "--out",
            out1.to_str().unwrap()
        ])),
        0
    );
    assert_eq!(
        exit_code(&run(&[
            "export",
            "--doc",
            doc.to_str().unwrap(),
            "--format",
            "png",
            "--out",
            out2.to_str().unwrap()
        ])),
        0
    );
    let b1 = std::fs::read(&out1).unwrap();
    let b2 = std::fs::read(&out2).unwrap();
    assert_eq!(
        b1, b2,
        "re-export across independent invocations is byte-identical"
    );
    assert_eq!(&b1[1..4], b"PNG");
}

#[test]
fn export_serves_all_six_formats_through_the_io_pipeline() {
    // O-QA W21/W10/W13/W15 finding 4: the CLI export verb now consumes
    // the merged six-format §29.1 io pipeline (was png-only + jpeg-NI).
    let dir = work_dir("exportfmts");
    let doc = new_doc(&dir, "a.appname");
    for (fmt, ext) in [
        ("png", "png"),
        ("jpeg", "jpg"),
        ("webp", "webp"),
        ("tiff", "tiff"),
        ("gif", "gif"),
        ("bmp", "bmp"),
    ] {
        let out = dir.join(format!("x.{ext}"));
        let o = run(&[
            "export",
            "--doc",
            doc.to_str().unwrap(),
            "--format",
            fmt,
            "--out",
            out.to_str().unwrap(),
            "--json",
        ]);
        assert_eq!(exit_code(&o), 0, "{fmt} exports");
        let v = stdout_json(&o);
        assert_eq!(v["ok"], true, "{fmt} ok");
        assert!(v["bytes"].as_u64().unwrap() > 0, "{fmt} wrote bytes");
        assert!(out.exists(), "{fmt} file written");
        // Deterministic: a second run is byte-identical.
        let out2 = dir.join(format!("y.{ext}"));
        run(&[
            "export",
            "--doc",
            doc.to_str().unwrap(),
            "--format",
            fmt,
            "--out",
            out2.to_str().unwrap(),
            "--json",
        ]);
        assert_eq!(
            std::fs::read(&out).unwrap(),
            std::fs::read(&out2).unwrap(),
            "{fmt} byte-identical across runs"
        );
    }
    // --quality gates: jpeg/webp accept it, others reject typed.
    let out = dir.join("q.jpg");
    let o = run(&[
        "export",
        "--doc",
        doc.to_str().unwrap(),
        "--format",
        "jpeg",
        "--quality",
        "60",
        "--out",
        out.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(exit_code(&o), 0, "jpeg accepts --quality");
    let out = dir.join("q.png");
    let o = run(&[
        "export",
        "--doc",
        doc.to_str().unwrap(),
        "--format",
        "png",
        "--quality",
        "60",
        "--out",
        out.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(exit_code(&o), 1, "png rejects --quality");
    assert_eq!(stdout_json(&o)["error"]["code"], "error.api.invalid_arg");
}

#[test]
fn import_png_adds_a_layer_and_round_trips() {
    let dir = work_dir("import");
    let doc = new_doc(&dir, "a.appname");
    // A 2x2 png via the export of a tiny doc? Simpler: hand-encode one.
    let png_path = dir.join("in.png");
    {
        let mut bytes = Vec::new();
        {
            let mut enc = png::Encoder::new(std::io::Cursor::new(&mut bytes), 2, 2);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().unwrap();
            w.write_image_data(&[255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 0, 0, 0, 0])
                .unwrap();
        }
        std::fs::write(&png_path, bytes).unwrap();
    }
    let o = run(&[
        "import",
        "--doc",
        doc.to_str().unwrap(),
        "--image",
        png_path.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(
        exit_code(&o),
        0,
        "stderr: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    let v = stdout_json(&o);
    assert_eq!(v["ok"], true);
    // Journaled import (CP-0.5 wave 1): edit.paste fixes the layer name.
    assert_eq!(v["layerName"], "Pasted");
    assert!(
        v["assetId"].is_string(),
        "the ingested raster asset id is reported"
    );

    let info = stdout_json(&run(&["info", "--doc", doc.to_str().unwrap(), "--json"]));
    assert_eq!(info["info"]["layers"].as_array().unwrap().len(), 2);
    // The import is a journal entry now, so replay-as-recovery WORKS across
    // the package boundary: undo removes the pasted layer (the inverse of
    // the retired IO-call door's documented refusal).
    let o = run(&["undo", "--doc", doc.to_str().unwrap(), "--json"]);
    assert_eq!(
        exit_code(&o),
        0,
        "stderr: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    let info = stdout_json(&run(&["info", "--doc", doc.to_str().unwrap(), "--json"]));
    assert_eq!(
        info["info"]["layers"].as_array().unwrap().len(),
        1,
        "undo removed the paste"
    );
    let o = run(&["redo", "--doc", doc.to_str().unwrap(), "--json"]);
    assert_eq!(
        exit_code(&o),
        0,
        "stderr: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    let info = stdout_json(&run(&["info", "--doc", doc.to_str().unwrap(), "--json"]));
    assert_eq!(
        info["info"]["layers"].as_array().unwrap().len(),
        2,
        "redo re-pastes"
    );

    // Multi-layer export goes through render's compositor (wave-2
    // arbitration 3) and stays deterministic across invocations.
    let e1 = dir.join("multi1.png");
    let e2 = dir.join("multi2.png");
    assert_eq!(
        exit_code(&run(&[
            "export",
            "--doc",
            doc.to_str().unwrap(),
            "--format",
            "png",
            "--out",
            e1.to_str().unwrap()
        ])),
        0
    );
    assert_eq!(
        exit_code(&run(&[
            "export",
            "--doc",
            doc.to_str().unwrap(),
            "--format",
            "png",
            "--out",
            e2.to_str().unwrap()
        ])),
        0
    );
    let b1 = std::fs::read(&e1).unwrap();
    assert_eq!(
        b1,
        std::fs::read(&e2).unwrap(),
        "render-path export deterministic"
    );
    assert_eq!(&b1[1..4], b"PNG");
}

#[test]
fn script_then_history_shows_rows() {
    let dir = work_dir("history");
    let doc = new_doc(&dir, "a.appname");
    let o = run(&["history", "--doc", doc.to_str().unwrap(), "--json"]);
    assert_eq!(exit_code(&o), 0);
    let v = stdout_json(&o);
    assert_eq!(v["length"], 1);
    assert_eq!(v["entries"][0]["kind"], "doc.new");
    assert_eq!(v["entries"][0]["active"], true);
}

#[test]
fn selftest_passes_end_to_end() {
    let o = run(&["selftest", "--json"]);
    let v = stdout_json(&o);
    assert_eq!(v["ok"], true, "selftest rows: {}", v["checks"]);
    assert_eq!(exit_code(&o), 0);
    let checks = v["checks"].as_array().unwrap();
    assert_eq!(checks.len(), 5);
    assert!(checks.iter().all(|c| c["status"] == "PASS"));
}

#[test]
fn clap_usage_errors_exit_2() {
    let o = run(&[
        "export",
        "--doc",
        "x.appname",
        "--format",
        "bmp",
        "--out",
        "y.bmp",
    ]);
    assert_eq!(
        exit_code(&o),
        2,
        "clap usage errors exit 2 (proto contract)"
    );
}

// ---------------- CP-0.3 wave 2: stroke pipeline end-to-end ----------------

/// The built-in hard-round preset id (documented derivation: uuid v5 over
/// the engine namespace — recomputed here because integration tests cannot
/// import bin-crate internals).
fn hard_round_16_id() -> uuid::Uuid {
    uuid::Uuid::new_v5(
        &appname_core::engine::doc_new::APPNAME_NS,
        b"preset/builtin/hard-round-16",
    )
}

fn samples_blob() -> Vec<u8> {
    use appname_tools::brush::{encode_samples, InputSample};
    let pts = [(10.0, 12.0, 0.0), (40.0, 30.0, 0.04), (58.0, 44.0, 0.08)];
    let samples: Vec<InputSample> = pts
        .iter()
        .map(|(x, y, t)| InputSample {
            x: *x,
            y: *y,
            pressure: 1.0,
            tilt_x: 0.0,
            tilt_y: 0.0,
            rotation: 0.0,
            velocity: 0.0,
            t: *t,
        })
        .collect();
    encode_samples(&samples).unwrap()
}

#[test]
fn asset_add_then_stroke_script_end_to_end() {
    let dir = work_dir("stroke-e2e");
    let doc = new_doc(&dir, "a.appname");

    // Pre-stroke export as the change baseline.
    let out0 = dir.join("before.png");
    assert_eq!(
        exit_code(&run(&[
            "export",
            "--doc",
            doc.to_str().unwrap(),
            "--format",
            "png",
            "--out",
            out0.to_str().unwrap()
        ])),
        0
    );

    // Register the samples blob; the printed assetId equals the
    // content-derived id (script authors can precompute it).
    let blob = samples_blob();
    let blob_path = dir.join("stroke.samples");
    std::fs::write(&blob_path, &blob).unwrap();
    let o = run(&[
        "asset",
        "add",
        "--doc",
        doc.to_str().unwrap(),
        "--file",
        blob_path.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(
        exit_code(&o),
        0,
        "stderr: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    let v = stdout_json(&o);
    let asset_id = v["assetId"].as_str().unwrap().to_string();
    assert_eq!(asset_id, appname_core::asset_id_for(&blob).to_string());

    // Garbage blob rejects typed before touching the package.
    let junk = dir.join("junk.bin");
    std::fs::write(&junk, b"not samples").unwrap();
    let o = run(&[
        "asset",
        "add",
        "--doc",
        doc.to_str().unwrap(),
        "--file",
        junk.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(exit_code(&o), 1);
    assert_eq!(stdout_json(&o)["error"]["code"], "error.command.bad_target");

    // The stroke script (raw envelope bytes, WD-1): target = the background
    // layer id from info.
    let info = stdout_json(&run(&["info", "--doc", doc.to_str().unwrap(), "--json"]));
    let layer_id = info["info"]["layers"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let script = dir.join("ops.jsonl");
    std::fs::write(
        &script,
        format!(
            "{{\"schema\":\"appname.command.v1\",\"id\":\"e0000000-0000-4000-8000-0000000e2e01\",\"kind\":\"brush.stroke\",\"target\":{{\"layerId\":\"{layer_id}\"}},\"params\":{{\"brushPreset\":\"{}\",\"color\":{{\"model\":\"rgb\",\"comps\":[1,0,0,0]}},\"seed\":7}},\"samples\":\"{asset_id}\"}}\n",
            hard_round_16_id()
        ),
    )
    .unwrap();
    let o = run(&[
        "script",
        "--doc",
        doc.to_str().unwrap(),
        "--file",
        script.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(
        exit_code(&o),
        0,
        "stdout: {}",
        String::from_utf8_lossy(&o.stdout)
    );
    let v = stdout_json(&o);
    assert_eq!(v["applied"], 1);
    assert!(
        !v["receipts"][0]["dirty"].as_array().unwrap().is_empty(),
        "receipt carries dirty rects"
    );

    // The stroke changed the export; two exports of the stroked doc are
    // byte-identical (determinism across invocations).
    let out1 = dir.join("after.png");
    let out2 = dir.join("after2.png");
    assert_eq!(
        exit_code(&run(&[
            "export",
            "--doc",
            doc.to_str().unwrap(),
            "--format",
            "png",
            "--out",
            out1.to_str().unwrap()
        ])),
        0
    );
    assert_eq!(
        exit_code(&run(&[
            "export",
            "--doc",
            doc.to_str().unwrap(),
            "--format",
            "png",
            "--out",
            out2.to_str().unwrap()
        ])),
        0
    );
    let b0 = std::fs::read(&out0).unwrap();
    let b1 = std::fs::read(&out1).unwrap();
    let b2 = std::fs::read(&out2).unwrap();
    assert_ne!(b0, b1, "stroke visible in the export");
    assert_eq!(b1, b2);

    // Undo across the package boundary (replay-as-recovery over the
    // assembled engine + persisted asset), then redo.
    let o = run(&["undo", "--doc", doc.to_str().unwrap(), "--json"]);
    assert_eq!(
        exit_code(&o),
        0,
        "stdout: {}",
        String::from_utf8_lossy(&o.stdout)
    );
    let out_undone = dir.join("undone.png");
    run(&[
        "export",
        "--doc",
        doc.to_str().unwrap(),
        "--format",
        "png",
        "--out",
        out_undone.to_str().unwrap(),
    ]);
    assert_eq!(
        std::fs::read(&out_undone).unwrap(),
        b0,
        "undo restores the pre-stroke export"
    );
    let o = run(&["redo", "--doc", doc.to_str().unwrap(), "--json"]);
    assert_eq!(exit_code(&o), 0);
    let out_redone = dir.join("redone.png");
    run(&[
        "export",
        "--doc",
        doc.to_str().unwrap(),
        "--format",
        "png",
        "--out",
        out_redone.to_str().unwrap(),
    ]);
    assert_eq!(
        std::fs::read(&out_redone).unwrap(),
        b1,
        "redo restores the stroked export"
    );

    // History shows both entries.
    let h = stdout_json(&run(&["history", "--doc", doc.to_str().unwrap(), "--json"]));
    assert_eq!(h["length"], 2);
    assert_eq!(h["entries"][1]["kind"], "brush.stroke");
}

/// W5-style crash recovery: hard-kill (abort, no destructors) mid-script
/// with `--autosave`, then `recover` restores exactly the last journaled
/// command's state (the formal W5 gate is O-QA's; this is the mechanism
/// proof through the shipped binary).
#[test]
fn w5_hard_kill_mid_script_recovers_to_last_command() {
    let dir = work_dir("w5");
    let doc = new_doc(&dir, "a.appname");
    let base_bytes = std::fs::read(&doc).unwrap();
    let layer = stdout_json(&run(&["info", "--doc", doc.to_str().unwrap(), "--json"]))["info"]
        ["layers"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    // Six fills; the process aborts after the third.
    let script: String = (0..6)
        .map(|i| {
            format!(
                r#"{{"schema":"appname.command.v1","id":"c5000000-0000-4000-8000-0000000000{i:02x}","kind":"fill","target":{{"layerId":"{layer}"}},"params":{{"content":"color","color":{{"model":"rgb","comps":[0.{i}5,0.2,0.7,0]}}}}}}"#
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let script_path = dir.join("ops.jsonl");
    std::fs::write(&script_path, &script).unwrap();
    let log_path = dir.join("autosave.log");

    let out = std::process::Command::new(bin())
        .args([
            "script",
            "--doc",
            doc.to_str().unwrap(),
            "--file",
            script_path.to_str().unwrap(),
            "--autosave",
            log_path.to_str().unwrap(),
            "--json",
        ])
        .env("APPNAME_FI_ABORT_AFTER_APPLIES", "3")
        .output()
        .unwrap();
    assert!(!out.status.success(), "the process died hard (abort)");
    // The package on disk is untouched (transactional script never saved).
    assert_eq!(
        std::fs::read(&doc).unwrap(),
        base_bytes,
        "base package byte-identical"
    );
    assert!(log_path.exists(), "journal survived the kill");

    // Recover onto the base.
    let recovered_path = dir.join("recovered.appname");
    let o = run(&[
        "recover",
        "--doc",
        doc.to_str().unwrap(),
        "--log",
        log_path.to_str().unwrap(),
        "--out",
        recovered_path.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(
        exit_code(&o),
        0,
        "stderr: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    let v = stdout_json(&o);
    assert_eq!(v["replayed"], 3, "exactly the journaled commands replayed");
    assert_eq!(v["torn"], false);

    // Reference: the first three script lines applied cleanly.
    let ref_doc = dir.join("ref.appname");
    std::fs::write(&ref_doc, &base_bytes).unwrap();
    let ref_script = dir.join("ref.jsonl");
    std::fs::write(
        &ref_script,
        script.lines().take(3).collect::<Vec<_>>().join("\n"),
    )
    .unwrap();
    assert_eq!(
        exit_code(&run(&[
            "script",
            "--doc",
            ref_doc.to_str().unwrap(),
            "--file",
            ref_script.to_str().unwrap()
        ])),
        0
    );
    // State-hash equality through the public loader (the W5 criterion).
    let assembly = appname_core::EngineAssembly::new(); // kinds irrelevant for loading state
    let rec = appname_core::load_package_with(&assembly, &recovered_path).unwrap();
    let reference = appname_core::load_package_with(&assembly, &ref_doc).unwrap();
    assert_eq!(
        rec.engine.state_hash(),
        reference.engine.state_hash(),
        "recovered state == clean reference run of the journaled prefix"
    );
}

/// Wave-4 finding (a): a journaled selection-mutating command must not
/// break cross-invocation undo — the replay gate compares PERSISTENT
/// state (the live selection is §14.2 session state the package cannot
/// carry).
#[test]
fn selection_mutating_journal_still_undoes_across_the_package_boundary() {
    let dir = work_dir("selgate");
    let doc = new_doc(&dir, "a.appname");
    let layer = stdout_json(&run(&["info", "--doc", doc.to_str().unwrap(), "--json"]))["info"]
        ["layers"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    // mask.add (reveal_all) then mask.to_selection: the second command
    // leaves a LIVE selection in the journal's replayed state.
    let m = "c5e10000-0000-4000-8000-0000000000aa";
    let ops = format!(
        "{{\"schema\":\"appname.command.v1\",\"id\":\"c5e10000-0000-4000-8000-000000000001\",\"kind\":\"mask.add\",\"target\":{{\"layerId\":\"{layer}\"}},\"params\":{{\"kind\":\"raster\",\"newMaskId\":\"{m}\"}}}}\n{{\"schema\":\"appname.command.v1\",\"id\":\"c5e10000-0000-4000-8000-000000000002\",\"kind\":\"mask.to_selection\",\"target\":{{\"layerId\":\"{layer}\",\"maskId\":\"{m}\"}},\"params\":{{\"op\":\"replace\"}}}}"
    );
    let script = dir.join("sel.jsonl");
    std::fs::write(&script, ops).unwrap();
    assert_eq!(
        exit_code(&run(&[
            "script",
            "--doc",
            doc.to_str().unwrap(),
            "--file",
            script.to_str().unwrap()
        ])),
        0
    );
    // Cross-invocation undo previously failed the byte-fidelity gate here
    // (replayed state carries the selection; the loaded package cannot).
    let o = run(&["undo", "--doc", doc.to_str().unwrap(), "--json"]);
    assert_eq!(
        exit_code(&o),
        0,
        "stderr: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    let o = run(&["redo", "--doc", doc.to_str().unwrap(), "--json"]);
    assert_eq!(
        exit_code(&o),
        0,
        "stderr: {}",
        String::from_utf8_lossy(&o.stderr)
    );
}

/// §12.1 PARAMETRIC RE-RENDER end-to-end with PRODUCTION kernels (the
/// reopened-CP-0.7 slice): an enabled gaussian smart filter shows
/// blurred pixels at the export door; an adjustment layer (invert)
/// affects everything below it; `op.toggle` off is byte-identical to
/// never-applied (§12.4); and every export is a fresh load+re-render, so
/// consecutive byte-equal exports pin two-run determinism across
/// process boundaries.
#[test]
fn export_rerenders_parametric_content_with_production_kernels() {
    let dir = work_dir("param-rerender");
    let doc = dir.join("doc.appname");
    let o = run(&[
        "new",
        "--width",
        "64",
        "--height",
        "64",
        "--background",
        "white",
        "--out",
        doc.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(exit_code(&o), 0);
    let d = doc.to_str().unwrap();
    let lid = "12100000-0000-4000-8000-00000000bbb1";
    let sol = "12100000-0000-4000-8000-00000000bbb2";
    let fop = "12100000-0000-4000-8000-00000000ccc1";
    let adl = "12100000-0000-4000-8000-00000000ddd1";
    let ado = "12100000-0000-4000-8000-00000000ddd2";
    let apply = |cmd: String| {
        let o = run(&["apply", "--doc", d, "--json", "--command", &cmd]);
        assert_eq!(
            exit_code(&o),
            0,
            "apply failed: {}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        );
        stdout_json(&o)
    };
    let export = |name: &str| -> Vec<u8> {
        let out = dir.join(name);
        let o = run(&[
            "export",
            "--doc",
            d,
            "--format",
            "png",
            "--out",
            out.to_str().unwrap(),
            "--json",
        ]);
        assert_eq!(
            exit_code(&o),
            0,
            "export failed: {}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        );
        std::fs::read(&out).unwrap()
    };
    // Black art layer over the white background, converted to a smart
    // object (opaque content whose blurred edges let the backdrop show).
    apply(format!(
        r#"{{"schema":"appname.command.v1","id":"12100000-0000-4000-8000-00000000aaa1","kind":"layer.add","params":{{"kind":"raster","newLayerId":"{lid}","name":"art"}}}}"#
    ));
    apply(format!(
        r#"{{"schema":"appname.command.v1","id":"12100000-0000-4000-8000-00000000aaa2","kind":"fill","target":{{"layerId":"{lid}"}},"params":{{"content":"foreground"}}}}"#
    ));
    apply(format!(
        r#"{{"schema":"appname.command.v1","id":"12100000-0000-4000-8000-00000000aaa3","kind":"smartobject.convert","params":{{"layerIds":["{lid}"],"newLayerId":"{sol}"}}}}"#
    ));
    let e0 = export("e0.png");

    // Enabled gaussian smart filter: BLURRED pixels at the export door.
    let receipt = apply(format!(
        r#"{{"schema":"appname.command.v1","id":"12100000-0000-4000-8000-00000000aaa4","kind":"filter.smart_add","target":{{"layerId":"{sol}"}},"params":{{"filterId":"filter.gaussian_blur","params":{{"radius":6.0}},"newOpId":"{fop}"}}}}"#
    ));
    assert!(
        receipt["receipt"]["dirty"]
            .as_array()
            .map(|a| !a.is_empty())
            .unwrap_or(
                receipt["dirty"]
                    .as_array()
                    .map(|a| !a.is_empty())
                    .unwrap_or(false)
            ),
        "chain-edit D emission reaches the CLI receipt: {receipt}"
    );
    let e1 = export("e1.png");
    assert_ne!(e1, e0, "the enabled gaussian chain changes the export");
    assert_eq!(
        e1,
        export("e1b.png"),
        "two-run byte-identity (fresh load each run)"
    );

    // §12.4 at the door: toggle off == never applied, byte-true; back on
    // re-renders the SAME bytes.
    apply(format!(
        r#"{{"schema":"appname.command.v1","id":"12100000-0000-4000-8000-00000000aaa5","kind":"op.toggle","params":{{"opId":"{fop}","enabled":false}}}}"#
    ));
    assert_eq!(
        export("e0b.png"),
        e0,
        "disabled chain == never-applied (§12.4)"
    );
    apply(format!(
        r#"{{"schema":"appname.command.v1","id":"12100000-0000-4000-8000-00000000aaa6","kind":"op.toggle","params":{{"opId":"{fop}","enabled":true}}}}"#
    ));
    assert_eq!(
        export("e1c.png"),
        e1,
        "re-enabled chain re-renders identically"
    );

    // Adjustment layer (invert) above: affects everything below it.
    apply(format!(
        r#"{{"schema":"appname.command.v1","id":"12100000-0000-4000-8000-00000000aaa7","kind":"adjustment.add","target":{{"layerId":"{sol}"}},"params":{{"opKind":"adjustment.invert","newLayerId":"{adl}","newOpId":"{ado}","params":{{}}}}}}"#
    ));
    let e2 = export("e2.png");
    assert_ne!(e2, e1, "the enabled adjustment changes the composite");
    assert_eq!(e2, export("e2b.png"), "adjusted export is deterministic");
    // Hide the adjustment layer: identity restored byte-true.
    apply(format!(
        r#"{{"schema":"appname.command.v1","id":"12100000-0000-4000-8000-00000000aaa8","kind":"layer.set_props","target":{{"layerId":"{adl}"}},"params":{{"visible":false}}}}"#
    ));
    assert_eq!(export("e1d.png"), e1, "hidden adjustment exports identity");
}

/// CLI-EXPORT ROUTING CLOSURE (the CP-0.7 recorded residual): a
/// §4.5-faded smart doc — layer-level opacity on a smart layer — RENDERS
/// at the compositor but used to REFUSE at core's narrower export arm.
/// Export now routes through `composite_document_with` (the compositor
/// truth), so the strict-subset asymmetry collapses to EQUALITY at this
/// door: the export must succeed AND be byte-identical to the render
/// truth computed over the same package with the same assembly.
#[test]
fn export_routes_faded_smart_doc_through_the_compositor_truth() {
    let dir = work_dir("export-routing-closure");
    let doc = dir.join("doc.appname");
    let o = run(&[
        "new",
        "--width",
        "64",
        "--height",
        "64",
        "--background",
        "white",
        "--out",
        doc.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(exit_code(&o), 0);
    let d = doc.to_str().unwrap();
    let lid = "fade0000-0000-4000-8000-00000000bbb1";
    let sol = "fade0000-0000-4000-8000-00000000bbb2";
    let apply = |cmd: String| {
        let o = run(&["apply", "--doc", d, "--json", "--command", &cmd]);
        assert_eq!(
            exit_code(&o),
            0,
            "apply failed: {}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        );
    };
    apply(format!(
        r#"{{"schema":"appname.command.v1","id":"fade0000-0000-4000-8000-00000000aaa1","kind":"layer.add","params":{{"kind":"raster","newLayerId":"{lid}","name":"art"}}}}"#
    ));
    apply(format!(
        r#"{{"schema":"appname.command.v1","id":"fade0000-0000-4000-8000-00000000aaa2","kind":"fill","target":{{"layerId":"{lid}"}},"params":{{"content":"foreground"}}}}"#
    ));
    apply(format!(
        r#"{{"schema":"appname.command.v1","id":"fade0000-0000-4000-8000-00000000aaa3","kind":"smartobject.convert","params":{{"layerIds":["{lid}"],"newLayerId":"{sol}"}}}}"#
    ));
    // The fade: §4.5 layer-level opacity on the smart layer — the exact
    // shape core's reference arm refuses ("non-default compositing
    // props") and render serves.
    apply(format!(
        r#"{{"schema":"appname.command.v1","id":"fade0000-0000-4000-8000-00000000aaa4","kind":"layer.set_props","target":{{"layerId":"{sol}"}},"params":{{"opacity":0.5}}}}"#
    ));

    let out = dir.join("faded.png");
    let o = run(&[
        "export",
        "--doc",
        d,
        "--format",
        "png",
        "--out",
        out.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(
        exit_code(&o),
        0,
        "ROUTING CLOSURE: the faded smart doc must EXPORT (was: typed refusal at \
         core's narrower arm). stderr: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    let exported = std::fs::read(&out).unwrap();

    // The render truth, computed over the same package with the same
    // assembly the CLI uses — THE production constructor (MAJOR-1 ruled
    // swap: this mirror was a STALE pre-vector copy of the chain, and it
    // builds the reference engine the shipped binary is compared
    // against; it can no longer drift).
    let asm = appname_assembly::production_assembly();
    let bytes = std::fs::read(&doc).unwrap();
    let loaded = appname_core::load_package_bytes_with(&asm, &bytes).unwrap();
    let doc_ref = loaded.engine.document().unwrap();
    let canvas = doc_ref.geometry().canvas_rect();
    let composite =
        appname_render::composite_document_with(doc_ref, &asm, canvas).expect("renders");
    let plane = composite.region_rgba_f32(canvas);
    let truth =
        appname_core::encode_composite_png(canvas.width() as u32, canvas.height() as u32, &plane)
            .unwrap();
    assert_eq!(
        exported, truth,
        "the faded-smart doc EXPORTS == RENDERS byte-for-byte (the asymmetry \
         collapsed to equality at the CLI door)"
    );
    // And the export stays deterministic across a fresh process.
    let out2 = dir.join("faded2.png");
    let o = run(&[
        "export",
        "--doc",
        d,
        "--format",
        "png",
        "--out",
        out2.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(exit_code(&o), 0);
    assert_eq!(exported, std::fs::read(&out2).unwrap());
}

// --- Aud-parity P-6: the one-shot apply-door selection guard --------------
// A select→fill sequence run as two separate one-shot `apply` invocations
// silently fills the WHOLE layer (selection is session state and does not
// persist across invocations). The guard warns LOUDLY on stderr before
// applying, without refusing (whole-layer fill is legitimate) and without
// touching the clean stdout `--json` contract. The `script` door persists
// session state within its run and is intentionally unaffected.

fn bg_layer_id(doc: &Path) -> String {
    let o = run(&["info", "--doc", doc.to_str().unwrap(), "--json"]);
    assert_eq!(exit_code(&o), 0, "info failed");
    let info = stdout_json(&o);
    info["info"]["layers"][0]["id"]
        .as_str()
        .expect("layer id")
        .to_string()
}

#[test]
fn one_shot_fill_without_selection_warns_loudly_on_stderr() {
    let dir = work_dir("p6-fill-warns");
    let doc = new_doc(&dir, "doc.appname");
    let lid = bg_layer_id(&doc);

    let cmd = format!(
        r#"{{"schema":"appname.command.v1","id":"c6000000-0000-4000-8000-000000000001","kind":"fill","target":{{"layerId":"{lid}"}},"params":{{"content":"foreground"}}}}"#
    );
    let o = run(&[
        "apply",
        "--doc",
        doc.to_str().unwrap(),
        "--json",
        "--command",
        &cmd,
    ]);

    // The op is legitimate: it succeeds, ok:true, exit 0 — NOT refused.
    assert_eq!(
        exit_code(&o),
        0,
        "stderr: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert_eq!(stdout_json(&o)["ok"], serde_json::json!(true));

    // But a loud warning lands on stderr, and stdout stays clean JSON.
    let stderr = String::from_utf8_lossy(&o.stderr);
    assert!(
        stderr.contains("no active selection") && stderr.contains("one-shot"),
        "expected the P-6 selection warning on stderr, got: {stderr:?}"
    );
    let stdout = String::from_utf8_lossy(&o.stdout);
    assert!(
        !stdout.contains("no active selection"),
        "the warning must not pollute the stdout --json contract: {stdout:?}"
    );
}

#[test]
fn one_shot_non_selection_kind_does_not_warn() {
    let dir = work_dir("p6-no-warn");
    let doc = new_doc(&dir, "doc.appname");

    // layer.add does not scope to the active selection → no warning.
    let cmd = r#"{"schema":"appname.command.v1","id":"c6000000-0000-4000-8000-000000000002","kind":"layer.add","params":{"kind":"raster","newLayerId":"c6000000-0000-4000-8000-0000000000aa","name":"art"}}"#;
    let o = run(&[
        "apply",
        "--doc",
        doc.to_str().unwrap(),
        "--json",
        "--command",
        cmd,
    ]);
    assert_eq!(
        exit_code(&o),
        0,
        "stderr: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    let stderr = String::from_utf8_lossy(&o.stderr);
    assert!(
        !stderr.contains("no active selection"),
        "layer.add must not warn: {stderr:?}"
    );
}

#[test]
fn script_door_is_unaffected_by_the_selection_guard() {
    // A select→fill SCRIPT keeps the selection live across the two envelopes
    // in ONE process; the guard is scoped to the one-shot apply door, so no
    // warning is emitted here.
    let dir = work_dir("p6-script-quiet");
    let doc = new_doc(&dir, "doc.appname");
    let lid = bg_layer_id(&doc);

    let script = dir.join("select_then_fill.jsonl");
    std::fs::write(
        &script,
        format!(
            "{}\n{}\n",
            r#"{"schema":"appname.command.v1","id":"c6000000-0000-4000-8000-000000000010","kind":"selection.set","params":{"source":"rect","rect":{"x":2,"y":2,"w":8,"h":6}}}"#,
            format!(
                r#"{{"schema":"appname.command.v1","id":"c6000000-0000-4000-8000-000000000011","kind":"fill","target":{{"layerId":"{lid}"}},"params":{{"content":"foreground"}}}}"#
            )
        ),
    )
    .unwrap();

    let o = run(&[
        "script",
        "--doc",
        doc.to_str().unwrap(),
        "--json",
        "--file",
        script.to_str().unwrap(),
    ]);
    assert_eq!(
        exit_code(&o),
        0,
        "stderr: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    let stderr = String::from_utf8_lossy(&o.stderr);
    assert!(
        !stderr.contains("no active selection"),
        "the script door must not warn (selection is live + door is unguarded): {stderr:?}"
    );
}
