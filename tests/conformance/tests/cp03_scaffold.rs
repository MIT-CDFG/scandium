// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! CP-0.3 expectation overlay — FINALIZED (slice B) — plus the standing
//! drift alarm for any FUTURE pending overlay.
//!
//! Spec: integration-log wave-2 arbitration #1 (assembly seam); O-CORE
//! lane-log wave-2 stroke entry (§40.5 stage map); corpus manifest
//! `cp03Overlay`; plan §0.6 rule 6.
//!
//! History (evidence, rule 8): the slice-A alarm FIRED on the stroke
//! wiring merge (7adb470) exactly as designed — `brush.stroke` answered
//! `error.command.bad_target` on the assembled surface while the overlay
//! was pending — and was resolved by corpus revision 4 (the finalize).
//! The alarm below is generic: it re-arms automatically for whatever
//! `pendingKinds` names next.

use appname_conformance::harness::{apply_raw, case_raw};
use appname_conformance::json::{parse, Value};
use appname_conformance::jsonspan;
use appname_conformance::{assembly_point, corpus, paths};
use appname_core::Engine;

fn overlay_list(key: &str) -> Vec<String> {
    corpus::manifest()
        .get("cp03Overlay")
        .and_then(|o| o.get(key))
        .and_then(Value::as_arr)
        .unwrap_or_else(|| panic!("manifest.cp03Overlay.{key}"))
        .iter()
        .map(|v| v.as_str().expect("kind").to_string())
        .collect()
}

fn kind_case_raw(kind: &str) -> (String, Value, jsonspan::Node) {
    let p = paths::adversarial_dir()
        .join("generated")
        .join("kinds")
        .join(format!("{kind}.json"));
    let text = std::fs::read_to_string(&p).unwrap();
    let doc = parse(&text).unwrap();
    let span = jsonspan::parse(&text);
    (text, doc, span)
}

#[test]
fn cp03_overlay_finalized_shape() {
    // CP-0.5 arming after the wave-4 kind-tail re-entry (O-CORE, rev
    // 10 — MECHANICAL companion edit per the drift-alarm procedure,
    // flagged for O-QA ratification): ONE kind remains
    // PENDING (derived mechanically by the generator from the index;
    // geometry/layer/selection/guide rows next); the drift alarm watches
    // each on the assembled surface until its implementing wave
    // re-enters it.
    let pending = overlay_list("pendingKinds");
    assert_eq!(
        pending.len(),
        1,
        "the armed CP-0.5 pending set (changes only via the drift-alarm \
         re-entry procedure + O-QA review): {pending:?}"
    );
    assert!(
        pending
            .iter()
            .all(|k| !overlay_list("finalizedKinds").contains(k)),
        "a kind cannot be pending and finalized at once"
    );
    // The fill + clipboard family re-entered with its implementing wave
    // (O-CORE CP-0.5 wave 1, drift-alarm procedure); the transform family
    // re-entered at CP-0.5 wave 2 (O-TOOLS) — MECHANICAL companion edits,
    // flagged for O-QA ratification.
    let finalized = overlay_list("finalizedKinds");
    assert_eq!(
        finalized,
        [
            "brush.stroke",
            "eraser.stroke",
            "edit.cut",
            "edit.paste",
            "edit.paste_into",
            "fill",
            "transform.free",
            "transform.perspective",
            "transform.warp",
            "mask.add",
            "mask.apply",
            "mask.delete",
            "mask.edit",
            "mask.set_props",
            "mask.to_selection",
            "doc.assign_profile",
            "doc.convert_profile",
            "doc.depth_convert",
            "doc.canvas_resize",
            "doc.crop",
            "doc.resize",
            "doc.rotate",
            "doc.trim",
            "selection.all",
            "selection.invert",
            "selection.modify",
            "selection.none",
            "selection.set",
            "selection.transform",
            "grid.set",
            "guide.add",
            "guide.clear",
            "guide.delete",
            "guide.move",
            "image.flatten",
            "layer.add",
            "layer.align",
            "layer.delete",
            "layer.distribute",
            "layer.duplicate",
            "layer.group",
            "layer.merge_down",
            "layer.merge_visible",
            "layer.rename",
            "layer.reorder",
            "layer.set_props",
            "layer.transform",
            "layer.ungroup",
            "adjustment.add",
            "op.delete",
            "op.reorder",
            "op.set_mask",
            "op.set_params",
            "op.toggle",
            "smartobject.convert",
            "smartobject.edit_contents",
            "smartobject.rasterize",
            "smartobject.replace",
            "layer.rasterize",
            "history.goto",
            "history.snapshot",
            // CP-0.7 wave 2 (rev-14 finalize, O-QA's OWN edit): the five
            // wiring-slice kinds re-entered against the shipped
            // five-crate composition (assembled_engine == cli/cabi
            // assembly since the ratified ask-1 wiring).
            "filter.apply",
            "filter.smart_add",
            "image.auto_color",
            "image.auto_contrast",
            "image.auto_tone",
            // CP-1.0 door slice (rev-15 finalize, O-CORE — MECHANICAL
            // companion per the drift-alarm procedure, flagged for O-QA
            // ratification): the eight vector-family kinds re-entered
            // against core's own kinds::register (the VectorGeometry
            // service registration stays the TEXTVEC wiring slice's).
            "vector.path_new",
            "vector.path_edit",
            "vector.path_delete",
            "vector.path_bool",
            "vector.shape_add",
            "vector.shape_set_style",
            "vector.edit",
            "selection.from_path",
            // O-TOOLS CP-1.0 consolidated round (MECHANICAL companion,
            // drift-alarm procedure, flagged for O-QA ratification):
            // content_aware_fill re-entered with its implementing round.
            "content_aware_fill",
            // O-CORE CP-1.0 door slice 2 (MECHANICAL companion,
            // drift-alarm procedure, flagged for O-QA ratification):
            // the text family re-entered with its implementing slice.
            "text.add",
            "text.edit",
            "text.set_style",
            "text.set_paragraph",
            "text.to_shape",
            // O-CORE CP-1.0 door slice 3 (MECHANICAL companion,
            // drift-alarm procedure, flagged for O-QA ratification):
            // the ten-doors slice's thirteen kinds re-entered with
            // their implementing wave (rev 17).
            "channel.add",
            "channel.apply_image",
            "channel.calculations",
            "channel.delete",
            "channel.duplicate",
            "channel.edit",
            "doc.mode_convert",
            "meta.set",
            "selection.load",
            "selection.save",
            "slice.add",
            "slice.delete",
            "slice.edit",
            // O-TOOLS CP-1.0 warps row (MECHANICAL companion,
            // drift-alarm procedure, flagged for O-QA ratification;
            // RE-EXTENDED on the rebased union — generator order:
            // door-slice-3 block then the warps-row block):
            // transform.puppet re-entered with its implementing row.
            "transform.puppet",
            // O-TOOLS CP-1.0 retouch row (MECHANICAL companion,
            // drift-alarm procedure, flagged for O-QA ratification):
            // the five stroke kinds re-entered with their implementing
            // row (rev 18). Remaining NI after this row: stroke_path +
            // doc.duplicate (dagger) — probe design exhausts at
            // stroke_path's flip (O-INT decision pending).
            "clone.stroke",
            "heal.stroke",
            "dodge_burn.stroke",
            "smudge.stroke",
            "blur_sharpen.stroke",
            // O-TEXTVEC stroke slice (rev 19): the LAST registry kind.
            "stroke_path",
        ]
    );
    for kind in corpus::kind_file_names() {
        let doc = corpus::kind_file(&kind);
        let overlay = doc.get("cp03");
        if finalized.contains(&kind) {
            let o = overlay.unwrap_or_else(|| panic!("{kind}: cp03 overlay missing"));
            // Explicit wave-3 list first: the doc./selection. prefixes
            // now span waves (O-TOOLS rev-9 companion — flagged for O-QA
            // ratification with the rev-9 regen).
            const WAVE3: [&str; 11] = [
                "doc.canvas_resize",
                "doc.crop",
                "doc.resize",
                "doc.rotate",
                "doc.trim",
                "selection.all",
                "selection.invert",
                "selection.modify",
                "selection.none",
                "selection.set",
                "selection.transform",
            ];
            // CP-0.7 wave 2 (rev-14 finalize): the five wiring-slice
            // kinds — named explicitly because the image./filter.
            // prefixes span waves.
            const WAVE2_CP07: [&str; 5] = [
                "filter.apply",
                "filter.smart_add",
                "image.auto_color",
                "image.auto_contrast",
                "image.auto_tone",
            ];
            // CP-1.0 door slice: explicit list first (selection.from_path
            // would otherwise fall through the wave cascades).
            const DOOR_SLICE_CP10: [&str; 8] = [
                "vector.path_new",
                "vector.path_edit",
                "vector.path_delete",
                "vector.path_bool",
                "vector.shape_add",
                "vector.shape_set_style",
                "vector.edit",
                "selection.from_path",
            ];
            // O-CORE CP-1.0 door slice 3 (MECHANICAL companion,
            // flagged for O-QA ratification): explicit list FIRST —
            // the doc./selection./channel./slice./meta prefixes span
            // waves.
            const DOOR_SLICE_3_CP10: [&str; 13] = [
                "channel.add",
                "channel.apply_image",
                "channel.calculations",
                "channel.delete",
                "channel.duplicate",
                "channel.edit",
                "doc.mode_convert",
                "meta.set",
                "selection.load",
                "selection.save",
                "slice.add",
                "slice.delete",
                "slice.edit",
            ];
            // O-TOOLS CP-1.0 retouch row (MECHANICAL companion,
            // flagged for O-QA ratification): explicit list FIRST —
            // clone./heal. would otherwise fall through the cascades.
            const RETOUCH_ROW_CP10: [&str; 5] = [
                "clone.stroke",
                "heal.stroke",
                "dodge_burn.stroke",
                "smudge.stroke",
                "blur_sharpen.stroke",
            ];
            let want_status = if kind == "stroke_path" {
                // O-TEXTVEC stroke slice (MECHANICAL companion, flagged
                // for O-QA ratification) — the LAST registry kind.
                "finalized-cp10-stroke-path-slice"
            } else if RETOUCH_ROW_CP10.contains(&kind.as_str()) {
                "finalized-cp10-retouch-row"
            } else if DOOR_SLICE_3_CP10.contains(&kind.as_str()) {
                "finalized-cp10-door-slice-3"
            } else if kind == "transform.puppet" {
                // O-TOOLS CP-1.0 warps row (MECHANICAL companion,
                // flagged for O-QA ratification) — named explicitly
                // because the transform. prefix spans waves.
                "finalized-cp10-warps-row"
            } else if kind == "content_aware_fill" {
                // O-TOOLS CP-1.0 consolidated round (MECHANICAL
                // companion, flagged for O-QA ratification).
                "finalized-cp10-tools-round"
            } else if kind.starts_with("text.") {
                // O-CORE CP-1.0 door slice 2 (MECHANICAL companion,
                // flagged for O-QA ratification).
                "finalized-cp10-door-slice-2"
            } else if DOOR_SLICE_CP10.contains(&kind.as_str()) {
                "finalized-cp10-door-slice"
            } else if WAVE2_CP07.contains(&kind.as_str()) {
                "finalized-cp07-wave2"
            } else if kind.starts_with("adjustment.")
                || kind.starts_with("op.")
                || kind.starts_with("smartobject.")
                || kind.starts_with("history.")
                || kind == "layer.rasterize"
            {
                "finalized-cp07-wave1"
            } else if kind.starts_with("grid.")
                || kind.starts_with("guide.")
                || kind.starts_with("image.")
                || kind.starts_with("layer.")
            {
                "finalized-cp05-wave4"
            } else if kind.starts_with("brush.") || kind.starts_with("eraser.") {
                "finalized-cp03"
            } else if WAVE3.contains(&kind.as_str()) {
                "finalized-cp05-wave3"
            } else if kind.starts_with("transform.")
                || kind.starts_with("mask.")
                || kind.starts_with("doc.")
            {
                "finalized-cp05-wave2"
            } else {
                "finalized-cp05-wave1"
            };
            assert_eq!(o.get("status").and_then(Value::as_str), Some(want_status));
            // bareCore: not_implemented for ASSEMBLED kinds (arbitration
            // #1); the CP-0.7 history daggers are engine-BUILT-INS and
            // answer the doc gate on bare engines too (doc.new
            // precedent) — the overlay is the source of truth.
            let want_bare = if kind.starts_with("history.") {
                "error.command.bad_target"
            } else {
                "error.command.not_implemented"
            };
            assert_eq!(
                o.get("bareCore")
                    .and_then(|b| b.get("code"))
                    .and_then(Value::as_str),
                Some(want_bare),
                "{kind}: bare-core expectation"
            );
            assert_eq!(
                o.get("assembled")
                    .and_then(|a| a.get("code"))
                    .and_then(Value::as_str),
                Some("error.command.bad_target"),
                "{kind}: assembled stage-6 expectation (stage map; the                  2026-08-17 strictness re-encode moved only the                  INITIALIZED-context answers — CLI-sweep table rows —                  while the fresh-handle document-slot truth here is                  unchanged)"
            );
        } else {
            assert!(overlay.is_none(), "{kind}: undeclared cp03 overlay");
        }
    }
    // the 4 ported assembled overlays are exactly the declared set
    let declared = overlay_list("portedAssembledOverlayCases");
    assert_eq!(declared, ["P-A11", "P-A14", "P-A19", "P-A23"]);
    let env = corpus::ported_envelope();
    let actual: Vec<String> = corpus::cases(&env)
        .iter()
        .filter(|c| c.get("cp03Assembled").is_some_and(|v| !v.is_null()))
        .map(|c| c.get("caseId").and_then(Value::as_str).unwrap().to_string())
        .collect();
    assert_eq!(actual, declared, "ported overlay set drifted");
}

/// Arbitration-#1 invariant, engine level: BARE-CORE engines answer
/// `not_implemented` for the finalized stroke kinds at every checkpoint —
/// unchanged by the wiring merge (raw corpus bytes, WD-1).
#[test]
fn bare_core_stroke_kinds_stay_not_implemented() {
    for kind in overlay_list("finalizedKinds") {
        let (text, doc, span) = kind_case_raw(&kind);
        // The overlay's bareCore code is the source of truth (CP-0.7
        // daggers are engine-built-ins answering the doc gate on bare
        // engines — doc.new precedent; everything else keeps the
        // arbitration-#1 not_implemented invariant).
        let want = doc
            .get("cp03")
            .and_then(|o| o.get("bareCore"))
            .and_then(|b| b.get("code"))
            .and_then(Value::as_str)
            .unwrap_or("error.command.not_implemented")
            .to_string();
        let raw = case_raw(&text, &span, &doc, 0, "envelope", None);
        let mut engine = Engine::new();
        let out = apply_raw(&mut engine, raw);
        assert_eq!(
            out.code.as_deref(),
            Some(want.as_str()),
            "{kind}: bare-core expectation broke; got ok={} code={:?}",
            out.ok,
            out.code
        );
    }
}

/// Finalized-consistency: the IN-PROCESS assembled engine answers each
/// finalized kind's `cp03.assembled` expectation for the corpus NI
/// envelope (stage-6 bad_target on the unresolvable example target), and
/// the in-process construction agrees with the shipped CLI binary.
#[test]
fn assembled_engine_matches_finalized_overlays() {
    for kind in overlay_list("finalizedKinds") {
        let (text, doc, span) = kind_case_raw(&kind);
        let want = doc
            .get("cp03")
            .and_then(|o| o.get("assembled"))
            .and_then(|a| a.get("code"))
            .and_then(Value::as_str)
            .expect("finalized overlay code");
        let raw = case_raw(&text, &span, &doc, 0, "envelope", None);
        let mut engine = assembly_point::assembled_engine();
        let out = apply_raw(&mut engine, raw);
        assert_eq!(
            out.code.as_deref(),
            Some(want),
            "{kind}: in-process assembled engine diverges from the \
             finalized overlay; got ok={} code={:?} ({})",
            out.ok,
            out.code,
            out.message
        );
    }
}

/// THE STANDING DRIFT ALARM, re-armed for the future: for every kind in
/// `pendingKinds` (empty after the finalize), the ASSEMBLED surface (the
/// shipped CLI binary) must still answer `not_implemented`; anything else
/// while the overlay says pending fails the workspace with finalize
/// instructions. This is the mechanism that fired on 7adb470 and got
/// this slice sequenced correctly.
#[test]
fn cp03_drift_alarm_pending_kinds_on_assembled_surface() {
    let pending = overlay_list("pendingKinds");
    if pending.is_empty() {
        return; // armed but quiet: nothing pending after the finalize
    }
    // Binary-freshness guard (O-QA ratification batch, backlog closure):
    // rebuild-or-override via the shared harness door.
    let cli = appname_conformance::shipped_cli::fresh_cli_bin();
    let work = std::env::temp_dir().join(format!("appname-qa-cp03-alarm-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).unwrap();
    let doc_path = work.join("doc.appname");
    let new_out = std::process::Command::new(&cli)
        .args(["new", "--width", "96", "--height", "64", "--out"])
        .arg(&doc_path)
        .arg("--json")
        .output()
        .expect("spawn appname");
    assert!(new_out.status.success(), "scratch doc for the alarm");
    for kind in pending {
        let (text, doc, span) = kind_case_raw(&kind);
        let raw = case_raw(&text, &span, &doc, 0, "envelope", None);
        let out = std::process::Command::new(&cli)
            .args(["apply", "--doc"])
            .arg(&doc_path)
            .args(["--command", raw, "--json"])
            .output()
            .expect("spawn appname");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let obj = parse(stdout.trim())
            .unwrap_or_else(|e| panic!("{kind}: CLI stdout not one JSON object ({e}): {stdout}"));
        let code = obj
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(Value::as_str)
            .unwrap_or("<ok>");
        assert_eq!(
            code, "error.command.not_implemented",
            "\n\nCP DRIFT ALARM: `{kind}` answered `{code}` on the \
             ASSEMBLED surface while the corpus overlay marks it \
             PENDING.\nFinalize via O-QA review: fill `assembled` in the \
             generator, move the kind to finalizedKinds, regenerate, and \
             extend the aggregate expectations.\n"
        );
    }
    let _ = std::fs::remove_dir_all(work);
}
