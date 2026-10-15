// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Reviewer hardening (slice B, adopted): the 101-kind sweep through the
//! SHIPPED assembled surface — the `appname` CLI binary — not just the
//! in-process assembly. For every kind file except doc.new, the corpus
//! NI envelope (contract validMinimal, raw bytes) goes through
//! `appname apply` on one scratch doc: still-unimplemented kinds must
//! answer `error.command.not_implemented`; implemented kinds (2 stroke at
//! CP-0.3, + fill/edit.cut at CP-0.5 wave 1) must answer their per-kind
//! assembled overlay (`error.command.bad_target`, stage-6 target
//! resolution).
//!
//! Spec: plan registry rule (all 102 registered); integration-log wave-2
//! arbitration #1; corpus manifest `cp03Overlay`; pinned validation
//! order. One doc is sound: every apply here REJECTS, rejections never
//! journal, and the a0-scheme envelope ids are unique per kind anyway.

use appname_conformance::harness::case_raw;
use appname_conformance::json::{parse, Value};
use appname_conformance::jsonspan;
use appname_conformance::{corpus, paths};
use std::process::Command;

fn cli_bin() -> std::path::PathBuf {
    // Binary-freshness guard (O-QA ratification batch, backlog closure):
    // rebuild-or-override via the shared harness door — a stale prebuilt
    // binary can answer a superseded overlay table (observed 2026-08-16).
    appname_conformance::shipped_cli::fresh_cli_bin()
}

#[test]
fn cli_ni_sweep_101_kinds_on_the_shipped_binary() {
    let cli = cli_bin();
    let work = std::env::temp_dir().join(format!("appname-qa-cli-ni-sweep-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).unwrap();
    let doc_path = work.join("doc.appname");
    let new_out = Command::new(&cli)
        .args(["new", "--width", "96", "--height", "64", "--out"])
        .arg(&doc_path)
        .arg("--json")
        .output()
        .expect("spawn appname");
    assert!(new_out.status.success(), "scratch doc");

    let mut ni = 0;
    let mut overlaid = 0;
    for kind in corpus::kind_file_names() {
        if kind == "doc.new" {
            continue;
        }
        let p = paths::adversarial_dir()
            .join("generated")
            .join("kinds")
            .join(format!("{kind}.json"));
        let text = std::fs::read_to_string(&p).unwrap();
        let doc = parse(&text).unwrap();
        let span = jsonspan::parse(&text);
        let raw = case_raw(&text, &span, &doc, 0, "envelope", None);

        // expectation: kind-file cp03.assembled overlay if present, else
        // cp01 not_implemented carries forward (manifest rule). Overlays
        // state the corpus's FRESH-handle truth; this sweep runs against
        // an INITIALIZED scratch doc, so the doc.* color kinds carry
        // their initialized-context expectation here, where that context
        // is created (O-CORE CP-0.5 wave 2, rev 8 — flagged for O-QA
        // ratification).
        let assembled = doc.get("cp03").and_then(|o| o.get("assembled"));
        let (outcome, want) = match kind.as_str() {
            // {profile: null} is a valid untagged retag (§5.3).
            "doc.assign_profile" => ("accept", ""),
            // to:u16 is feature-gated in the default build (§6.3).
            "doc.depth_convert" => ("reject", "error.command.not_implemented"),
            // O-CORE v1.0.1 §19.1: the non-destructive crop
            // (deletePixels:false — the validMinimal default) is now SERVED.
            // By the alphabetical walk doc.canvas_resize has already enlarged
            // the scratch doc to 2048×1024, so validMinimal's rect
            // {100,80,1720,920} intersects → the crop ACCEPTS, setting
            // crop_rect (image_size derives to the crop size). The following
            // doc.resize/rotate reflow the canvas and CLEAR that crop
            // (set_full_canvas), so their accepts are unaffected.
            "doc.crop" => ("accept", ""),
            // Geometry accepts mutate the scratch doc (canvas 2048×1024 →
            // resize 1200×600 → rotate 600×1200); trim's transparent
            // basis finds nothing on the opaque doc (§40.1 no-op accept).
            "doc.canvas_resize" | "doc.resize" | "doc.rotate" | "doc.trim" => ("accept", ""),
            // selection.all/invert/none/set succeed on any doc state
            // (invert-on-inactive = select-all; none-on-inactive no-ops).
            // modify and transform hit the stage-4 live-state gate: the
            // `.appname` package does NOT persist the selection surface
            // (O-CORE handoff, O-TOOLS wave-3 lane log), so each CLI
            // invocation reloads an inactive selection.
            "selection.all" | "selection.invert" | "selection.none" | "selection.set" => {
                ("accept", "")
            }
            "selection.modify" | "selection.transform" => {
                ("reject", "error.command.selection_required")
            }
            // O-CORE CP-0.5 wave 4 (rev 10 — flagged for O-QA
            // ratification): initialized-context answers on the evolving
            // scratch doc. grid.set + guide.add are plain accepts;
            // guide.clear clears the just-added guide (accept);
            // guide.delete/move then hit the live-state index gate on the
            // emptied vectors; image.flatten no-ops on the already-flat
            // scratch doc; layer.add creates its feedfeed layer;
            // layer.merge_visible collapses it into the background.
            "grid.set" | "guide.add" | "guide.clear" | "image.flatten" | "layer.merge_visible" => {
                ("accept", "")
            }
            // Door-slice-3 cascade (same companion): the corpus reuses
            // ONE creation id (feedfeed-…0001) across creation kinds;
            // channel.add (alphabetically first accepting creation kind
            // since slice 3) mints it into the channels arena, so
            // layer.add now answers the §9.6 GLOBAL creation-id
            // discipline (QA-F11 door — cross-arena) on the evolving
            // doc. layer.merge_visible keeps its accept (no-op on the
            // background-only stack, the image.flatten reading).
            // Fold-in note for O-QA: per-kind distinct creation ids in
            // the corpus would decouple these rows.
            "layer.add" => ("reject", "error.schema.duplicate_id"),
            "guide.delete" | "guide.move" => ("reject", "error.schema.out_of_range"),
            // O-CORE CP-0.7 fix round (rev 12 — flagged for O-QA
            // ratification): history.snapshot is a †-marker accept on any
            // initialized doc; history.goto{position:0} on the reopened
            // scratch doc is IN-RANGE-BUT-UNREACHABLE (the CLI reloads
            // with an empty in-process history, §10.4) and answers
            // error.schema.out_of_range per the contract targetResolution
            // (fix-round M3 — the old row pinned the leaked
            // error.history.nothing_to_undo).
            "history.snapshot" => ("accept", ""),
            "history.goto" => ("reject", "error.schema.out_of_range"),
            // 2026-08-17 strictness re-encode (O-TOOLS MECHANICAL
            // companion, observe-first on the shipped binary, flagged
            // for O-QA ratification): the ruled schema-letter validation
            // supersedes the rev-13 row's recorded-as-observed tolerance
            // ("the example's `channels` block is ignored-unknown" — the
            // drift itself, documented in passing, never a sanctioned
            // rationale). validMinimal's dead-shaped nested params
            // (channels wrapper; the sigmaPx divergence class, fold-in
            // registered) now refuse typed at stage 3, BEFORE the
            // dangling-target anchor.
            "adjustment.add" => ("reject", "error.schema.out_of_range"),
            // O-TOOLS CP-0.7 wave 2 (same flagged companion): the five
            // wiring-slice kinds went live in the shipped assembly.
            // image.auto_*: §40.2 accepts on the initialized scratch doc
            // (all-white content → identity-adjacent §17.2 analyses;
            // §40.1 empty-or-epsilon delta accept).
            "image.auto_tone" | "image.auto_contrast" | "image.auto_color" => ("accept", ""),
            // 2026-08-17 strictness re-encode (same companion as the
            // adjustment.add row): the frozen validMinimals' dead-named
            // params (sigmaPx / radiusPx — the registered fold-in
            // divergence) now refuse typed at stage 3, before the
            // dangling-target bad_target the rev-13 rows pinned.
            "filter.apply" | "filter.smart_add" => ("reject", "error.schema.out_of_range"),
            // O-QA CP-1.0 re-pin round (this lane's own encode against
            // the LIVE service — the TEXTVEC Door-1 wiring merged at
            // main e8f5f9b registers appname_vector's VectorGeometry on
            // BOTH production assemblies; the flip below is the filed
            // designed tripwire, reconciled flip-by-flip in the lane
            // log). The six RESOLVING kinds keep bad_target: stage-4
            // resolution still precedes geometry, and validMinimal's
            // pathId / target ids dangle on the scratch doc — the
            // ANSWER is unchanged, but live §20 geometry (not the typed
            // NI mirror) now sits behind it. path_bool's validMinimal
            // is doc-path mode (pathIds), live per the filing; its
            // APPEND (shape-layer) mode stays typed-NI under the
            // fold-in hold — not observable through this arm.
            "vector.path_edit"
            | "vector.path_delete"
            | "vector.path_bool"
            | "selection.from_path" => ("reject", "error.command.bad_target"),
            "vector.shape_set_style" | "vector.edit" => ("reject", "error.command.bad_target"),
            // …and the two CREATION kinds (nothing to resolve first)
            // FLIP typed-NI → live accept on the initialized context:
            // path_new normalizes + records its path through the real
            // service; shape_add (validMinimal primitive: ellipse — not
            // the `custom` asset-door arm, which stays typed-NI per the
            // filing) constructs and value-captures its sidecar (D-48).
            // Their kind-file overlays keep the FRESH-handle bad_target
            // answer (document-slot check precedes everything; the
            // armed corpus did NOT move — verified this round).
            "vector.path_new" | "vector.shape_add" => ("accept", ""),
            // O-TOOLS CP-1.0 consolidated round (MECHANICAL companion,
            // flagged for O-QA ratification — same procedure): the
            // content_aware_fill arm is LIVE for output:'current_layer';
            // validMinimal's defaulted output new_layer answers the typed
            // layer-creation-door NI on the initialized context (the
            // kind-file overlay carries the FRESH-handle bad_target
            // answer; this table is the initialized-context truth, same
            // split as path_new/shape_add above).
            "content_aware_fill" => ("reject", "error.command.not_implemented"),
            // O-TEXTVEC CP-1.0 text-kinds wiring slice (MECHANICAL
            // companion, flagged for O-QA ratification — same
            // procedure): appname-text's TextLayout service is now the
            // PRODUCTION_CHAIN row, so the typed TextLayout-NI mirror
            // is gone from the shipped assembly. text.add (creation
            // kind, nothing to resolve) flips typed-NI → live accept:
            // validMinimal (point at [100,100], no text) creates an
            // EMPTY text layer through the live Door 6 (the
            // create-then-type pin; empty bounds, zero tiles). The four
            // RESOLVING kinds keep stage-4 bad_target on their dangling
            // validMinimal targets — the ANSWER is unchanged, but live
            // §23 layout (not the NI mirror) now sits behind it (the
            // pinned vector-arm order, resolution precedes the service
            // gate). Kind-file overlays keep the FRESH-handle
            // bad_target answer (document-slot check precedes
            // everything). Prior row (O-CORE door slice 2): text.add
            // reject error.command.not_implemented.
            "text.add" => ("accept", ""),
            // O-TEXTVEC stroke_path slice (MECHANICAL companion, flagged
            // for O-QA ratification — the same procedure): the LAST
            // registry kind flips typed-NI → live. validMinimal's
            // source.pathId dangles on the scratch doc, so the arm's
            // stage-4 source resolution answers bad_target. (Order
            // note, review NOTE-7: schema-CHECKABLE violations answer
            // pre-arm via the registry's stage-5 validation; within
            // the arm, target resolution + locks precede the param
            // gates, and source resolution follows them — the fill-arm
            // precedent. validMinimal is schema-clean, so the dangling
            // source is what answers here.)
            "stroke_path" => ("reject", "error.command.bad_target"),
            "text.edit" | "text.set_style" | "text.set_paragraph" | "text.to_shape" => {
                ("reject", "error.command.bad_target")
            }
            // O-CORE CP-1.0 door slice 3 (MECHANICAL companion, flagged
            // for O-QA ratification — same procedure as every row block
            // above): the thirteen slice-3 kinds went live. Initialized-
            // context answers on the evolving alphabetical scratch doc,
            // each verified through the shipped CLI this round:
            // channel.add's validMinimal CREATES its feedfeed channel
            // (accept — the walk's doc now carries it);
            // channel.calculations resolves its sources but its
            // creation id collides with the channel.add mint → §9.6
            // duplicate_id; apply_image/delete/duplicate/edit dangle
            // their example targets → stage-4 bad_target;
            // doc.mode_convert{to:gray_a} EXECUTES (accept — the scratch
            // doc is GrayA from here on; §6.2 warnings ride the
            // receipt); meta.set/slice.add/slice.delete are plain
            // doc-furniture accepts; slice.edit's example index/bounds
            // miss the live list → §40.8 out_of_range; selection.load
            // dangles its source channel → bad_target; selection.save
            // hits the stage-4 selection gate on the CLI's reloaded
            // (inactive) selection → selection_required.
            "channel.add" | "doc.mode_convert" | "meta.set" | "slice.add" | "slice.delete" => {
                ("accept", "")
            }
            "channel.calculations" => ("reject", "error.schema.duplicate_id"),
            "channel.apply_image"
            | "channel.delete"
            | "channel.duplicate"
            | "channel.edit"
            | "selection.load" => ("reject", "error.command.bad_target"),
            "selection.save" => ("reject", "error.command.selection_required"),
            "slice.edit" => ("reject", "error.schema.out_of_range"),
            _ => (
                "reject",
                assembled
                    .and_then(|a| a.get("code"))
                    .and_then(Value::as_str)
                    .unwrap_or("error.command.not_implemented"),
            ),
        };

        let out = Command::new(&cli)
            .args(["apply", "--doc"])
            .arg(&doc_path)
            .args(["--command", raw, "--json"])
            .output()
            .expect("spawn appname");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let obj = parse(stdout.trim())
            .unwrap_or_else(|e| panic!("{kind}: not one JSON object ({e}): {stdout}"));
        if outcome == "accept" {
            assert_eq!(out.status.code(), Some(0), "{kind}: accept class");
            assert_eq!(
                obj.get("ok").and_then(Value::as_bool),
                Some(true),
                "{kind}: ok:true"
            );
            overlaid += 1;
        } else {
            assert_eq!(
                out.status.code(),
                Some(1),
                "{kind}: reject class through the CLI"
            );
            assert_eq!(
                obj.get("ok").and_then(Value::as_bool),
                Some(false),
                "{kind}: ok:false"
            );
            let code = obj
                .get("error")
                .and_then(|e| e.get("code"))
                .and_then(Value::as_str)
                .unwrap_or("");
            assert_eq!(
                code, want,
                "{kind}: shipped-binary sweep diverged from the corpus \
                 expectation"
            );
            let has_overlay = assembled.is_some();
            if !has_overlay && want == "error.command.not_implemented" {
                ni += 1;
            } else {
                overlaid += 1;
            }
        }
    }
    // Count updated by O-CORE at CP-0.5 wave 1 (fill + clipboard-family
    // overlays) and by O-TOOLS at CP-0.5 wave 2 (transform.free/
    // perspective/warp overlays, implementing-wave re-entry per the
    // manifest rule) — MECHANICAL companion edits, flagged for O-QA
    // ratification.
    assert_eq!(
        (ni, overlaid),
        (1, 100),
        "1 unimplemented + 100 overlaid (O-TEXTVEC stroke_path slice, \
         MECHANICAL companion flagged for O-QA ratification: the LAST \
         registry kind leaves the NI class — its validMinimal's dangling \
         source.pathId answers stage-4 bad_target inside the overlaid \
         class. The remaining NI set is EXACTLY doc.duplicate, the \
         dagger kind, permanently NI by design: this tally is TERMINAL — \
         no future flip moves it. The d07 transactional probe re-pins to \
         doc.duplicate in the same companion [the recorded \
         probe-exhaustion decision proposal to O-INT]. Prior history: \
         (2,99) at the O-TOOLS retouch row, whose note follows.) \
         (2,99 note: O-TOOLS CP-1.0 retouch row, \
         MECHANICAL companion flagged for O-QA ratification: the five \
         retouch strokes — clone/heal/dodge_burn/smudge/blur_sharpen — \
         leave the NI class for their finalized-cp10-retouch-row \
         overlays; each validMinimal's dangling target layerId answers \
         stage-6 bad_target on the initialized context, OBSERVED on the \
         shipped CLI per kind before encoding [observe-first rule]; \
         stage-6 target resolution precedes every retouch arm. The \
         REMAINING NI set is exactly stroke_path [O-TEXTVEC follow-on] \
         + doc.duplicate [dagger, permanently NI by design] — the d07 \
         transactional-probe design EXHAUSTS at stroke_path's flip \
         [decision to O-INT, O-CORE lane-log process note]. Prior \
         history: (7,94) at the O-TEXTVEC text-kinds slice, whose note \
         follows.) \
         (7,94 note: O-TEXTVEC CP-1.0 text-kinds \
         wiring slice, MECHANICAL companion flagged for O-QA \
         ratification: text.add flips typed-NI → live accept IN PLACE \
         inside the overlaid class — its kind-file overlay has counted \
         it in `overlaid` since door slice 2, so the walk is the \
         IDENTITY this round, exactly the path_new/shape_add precedent; \
         the four resolving text kinds keep bad_target with the live \
         service now behind them; re-derived on the rebased union per \
         the observe-first re-encoding rule — the tally is main's. \
         Prior history: (7,94) at the O-TOOLS warps row, whose note \
         follows.) \
         (7,94 note: O-TOOLS CP-1.0 warps row, \
         RE-DERIVED on the rebased union per the observe-first \
         re-encoding rule [O-INT merge-bounce instruction, main d5e2b29]: \
         transform.puppet leaves door-slice-3's NI set for its \
         finalized-cp10-warps-row overlay — dangling validMinimal target \
         keeps bad_target inside the overlaid class; stage-6 target \
         resolution precedes the identity/backend arm split; the merged \
         tally is neither parent's numbers. Remaining NI set: \
         doc.duplicate [dagger, typed by design], the five retouch \
         strokes, stroke_path. Prior history: (8,93) at the O-CORE \
         CP-1.0 door slice 3, whose note follows.) \
         (8,93 note: O-CORE door slice 3 — the thirteen slice-3 kinds — \
         selection.save/load, the six channel.* kinds, \
         slice.add/edit/delete, meta.set, doc.mode_convert — left the NI \
         class for initialized-context overlays, + the layer.add \
         §9.6-cascade row. Prior history: (21,80) at the O-CORE CP-1.0 \
         door slice 2 [five text.* overlays: four resolving bad_target, \
         text.add the TextLayout-NI mirror]; (26,75) at the O-QA CP-1.0 \
         re-pin round [vector.path_new/shape_add flipped to live accept \
         in place against the LIVE VectorGeometry service, main \
         e8f5f9b]; (26,75) at the O-TOOLS CP-1.0 consolidated round; \
         (27,74) at the O-CORE CP-1.0 door slice; (35,66) at the O-TOOLS \
         CP-0.7 wave-2 wiring slice)"
    );
    let _ = std::fs::remove_dir_all(work);
}
