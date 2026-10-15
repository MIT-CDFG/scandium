#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
"""generate_corpus.py — deterministic adversarial-corpus generator (O-QA).

Spec: plan AGENT-ASSIGNMENTS-full-build.md section 2.1 row "Test harness v1";
ES section 10.2 (appname.command.v1 envelope), section 40.1 (error taxonomy
baseline), Appendix G (MessageId registry).

Regenerates fixtures/adversarial/** from three mechanical sources:

  1. the round-1 mockup's adversarial suite (READ-ONLY reference):
       appname-proto/scripts/adversarial.jsonl   (29 envelope cases)
       appname-proto/scripts/adversarial.py      (12 CLI driver cases,
                                                  transcribed as the D_TABLE
                                                  below; observed codes from
                                                  REPORT-A section 4.2)
  2. this repo's frozen contracts mirror:
       contracts/commands/_index.yaml            (the 102 command kinds)
       contracts/commands/<kind>.yaml            (validMinimal envelopes;
                                                  doc.new paramsSchema +
                                                  examples.invalid)
       contracts/schemas/command-envelope.v1.json
       contracts/message_ids.yaml                (v5.5 error-id registry)
  3. the plan's CP-0.1 registry rule: all 102 kinds registered, unimplemented
     kinds return error.command.not_implemented.

DETERMINISM: seedless and stable. All iteration is over sorted keys, all JSON
is dumped with sort_keys=True/indent=2/ensure_ascii=True, envelope ids are
assigned by fixed positional schemes (documented in the manifest). Re-running
against unchanged inputs is byte-identical (input sha256s are recorded in the
manifest, so input drift is visible, never silent).

Expected-outcome provenance tiers (per corpus case, field "codeTier"):
  contract — code comes from a contracts/ file (examples.invalid expectError,
             a registered v5.5 MessageId, envelope-schema required fields)
  proto    — code is round-1 proto observed behavior (adversarial.jsonl
             EXPECT lines / REPORT-A section 4.2); flagged when the id is
             absent from the v5.5 registry
  plan     — error.command.not_implemented, mandated by the plan's CP-0.1
             registry rule but not present in contracts/message_ids.yaml
             (open question OQ-1 in reports/O-QA-log.md)

Usage:  python3 fixtures/adversarial/generate_corpus.py
          [--proto /path/to/appname-proto]   (default: sibling checkout)
"""

import argparse
import hashlib
import json
import os
import re
import sys

import yaml  # PyYAML, stdlib-adjacent; used at authoring time only

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
# The proto reference tree: the environment variable first, then the sibling
# position a checkout and a worktree both put this tree in — never an absolute
# path (operator WORD 24, RULING A; this file ships in the public tarball).
#   checkout:  <parent>/appname            -> sibling is <parent>/<proto>
#   worktree:  <parent>/appname-wt/<name>  -> REPO is <parent>/appname-wt, so the
#                                             sibling is one level further up
DEFAULT_PROTO = os.environ.get("APPNAME_PROTO_ROOT", "").strip()
if not DEFAULT_PROTO:
    _proto_name = os.environ.get("APPNAME_PROTO_DIRNAME", "").strip()
    if _proto_name:
        for _base in (os.path.dirname(REPO), os.path.dirname(os.path.dirname(REPO))):
            _cand = os.path.join(_base, _proto_name)
            if os.path.isdir(_cand):
                DEFAULT_PROTO = _cand
                break

CASE_RE = re.compile(r"^# CASE (\S+) EXPECT exit=(\S+) code=(\S+) :: (.*)$")

PLAN_IDS = ["error.command.not_implemented"]

# ---- CP-0.3 expectation overlay (SCAFFOLD; finalize after O-CORE's stroke
# wiring merges) -----------------------------------------------------------
# Mechanism: cp01Expect always stands. A kind file WITHOUT a `cp03` key
# carries cp01Expect forward unchanged at CP-0.3. A `cp03` overlay appears
# ONLY where behavior legitimately changes with the checkpoint, split by
# ENGINE ASSEMBLY per the integration-log CP-0.3 wave-2 arbitration #1:
# core's Engine gains a kind-implementation registration seam at
# CONSTRUCTION; appname-tools exports the stroke KindImpls;
# appname-cli/appname-cabi assemble the full engine. Bare-core engines keep
# answering not_implemented for stroke kinds AT EVERY CHECKPOINT; assembled
# engines implement them once the wiring lands.
CP03_ARBITRATION = (
    "integration-log CP-0.3 wave-2 arbitration #1: kind-implementation "
    "registration seam at engine construction; appname-tools exports the "
    "stroke KindImpls; appname-cli/appname-cabi assemble the full engine; "
    "bare-core engines keep not_implemented"
)
CP03_PENDING_KINDS = []  # finalized 2026-08-12 (stroke wiring 7adb470)

# CP-0.5 waves 2/3 arming (O-INT slice, 2026-08-12): every milestone-0.5
# kind not yet implemented is PENDING — the drift alarm watches each on
# the assembled surface and fails the workspace the moment one flips
# while its overlay is absent. Derived MECHANICALLY from the index at
# generation time (milestone == "0.5" minus the implemented set); spill/
# autosave add no kinds, masks + color + geometry do.
CP05_IMPLEMENTED = ["brush.stroke", "doc.new", "eraser.stroke"]


def cp05_pending_kinds(kinds):
    return sorted(
        k for k, row in kinds.items()
        if str(row["milestone"]) == "0.5"
        and k not in CP05_IMPLEMENTED
        and k not in finalized_kinds()
    )


def finalized_kinds():
    """The ONE finalized list (rev-7 drift fix, O-QA ratification entry):
    CP03_FINALIZED_KINDS alone was never the whole story once wave-1
    landed — the O-TOOLS companion log flagged the naming drift. Every
    consumer (manifest, pending derivation) now goes through here."""
    return (CP03_FINALIZED_KINDS + CP05_WAVE1_FINALIZED_KINDS
            + CP05_WAVE2_FINALIZED_KINDS
            + CP05_WAVE2_CORE_MASK_KINDS + CP05_WAVE2_CORE_DOC_KINDS
            + CP05_WAVE3_FINALIZED_KINDS
            + CP05_WAVE4_FINALIZED_KINDS + CP07_WAVE1_FINALIZED_KINDS
            + CP07_WAVE2_FINALIZED_KINDS + CP10_DOOR_SLICE_KINDS
            + CP10_TOOLS_ROUND_KINDS + CP10_DOOR_SLICE_2_KINDS
            + CP10_DOOR_SLICE_3_KINDS + CP10_WARPS_ROW_KINDS
            + CP10_RETOUCH_ROW_KINDS + CP10_STROKE_PATH_KINDS)
CP03_FINALIZED_KINDS = ["brush.stroke", "eraser.stroke"]

# ---- CP-0.5 wave-2 re-entry (O-TOOLS transform.free/perspective/warp) ----
# MECHANICAL companion edits by O-TOOLS (0dceb11) following the
# drift-alarm procedure, RATIFIED by O-QA and folded into the generator
# at rev 7 so the corpus stays generator-produced (their hand-applied
# files are reproduced byte-faithfully by this block).
CP05_WAVE2_FINALIZED_KINDS = [
    "transform.free", "transform.perspective", "transform.warp"]
CP05_WAVE2_ARBITRATION = (
    CP03_ARBITRATION
    + "; applied at CP-0.5 wave 2: appname-tools exports the "
    "transform.free/perspective/warp KindImpls (kind_impls::assembly)")
CP05_WAVE2_PROVENANCE = (
    "stage map (O-TOOLS lane-log CP-0.5 wave-2 entry, section 40.6 over "
    "the pinned order): the contract validMinimal passes stage-5 schema "
    "and fails at stage-6 target resolution - the example target.layerId "
    "does not resolve on the case's fresh handle -> bad_target. Full "
    "stage-6 order behind it: raster-layer target (transient multi-select "
    "groups later) -> section 11.1 locks (POSITION/ALL block; PIXELS does "
    "NOT, section 40.6) -> semantic geometry gates (invertible matrix / "
    "non-degenerate TL-TR-BR-BL quad / mesh point-count = (3c+1)(3r+1) "
    "-> out_of_range; preset kinds without section 18.4 math -> "
    "not_implemented) -> resample vocabulary (auto + the section 18.2 "
    "wire names -> out_of_range otherwise) -> pure engines (xform/warp/"
    "resample modules) -> whole-plane bake through the recorder tile "
    "doors; identity transforms answer the section 40.1 empty-delta push")
CP05_WAVE2_NOTE = (
    "bareCore keeps not_implemented at every checkpoint (arbitration #1 "
    "invariant); the accept path is exercised by the in-crate transform "
    "test battery (kind_impls transform_tests) with real painted "
    "targets, not by this reject-matrix case")

# ---- CP-0.5 wave-2 re-entry, O-CORE half (mask family + doc color) -------
# MECHANICAL companion edit by O-CORE following the drift-alarm procedure
# (rev 8) -- flagged for O-QA ratification (reports/O-CORE-log.md CP-0.5
# wave-2 entry). Overlay context rule: G-NI cases run on the FRESH
# (uninitialized) handle, so every assembled expectation states the
# fresh-handle truth; for the doc.* color kinds that is bad_target (the
# stage-4 implemented-check now passes and the document-slot check
# fires). Richer-context behavior (the CLI sweep's initialized scratch
# doc: doc.assign_profile accepts; doc.depth_convert answers
# feature-gated not_implemented) lives in the sweep's context table,
# where that context is created.
CP05_WAVE2_CORE_MASK_KINDS = [
    "mask.add", "mask.apply", "mask.delete", "mask.edit",
    "mask.set_props", "mask.to_selection",
]
CP05_WAVE2_CORE_DOC_KINDS = [
    "doc.assign_profile", "doc.convert_profile", "doc.depth_convert",
]

# ---- CP-0.5 wave-4 re-entry (O-CORE kind tail) ---------------------------
# MECHANICAL companion edit by O-CORE (rev 10, drift-alarm procedure) --
# flagged for O-QA ratification. Fresh-handle rule as with the doc.* color
# kinds: every wave-4 kind passes stage 4 (implemented) on assembled
# engines and fails the document-slot check on the corpus's FRESH handle
# -> bad_target; richer initialized-context behavior (several validMinimal
# envelopes are VALID commands on the CLI sweep's scratch doc) lives in
# the sweep's context table. doc.duplicate stays cp01 carry-forward: the
# dagger surface is the engine door + CLI verb; the apply path answers
# not_implemented by design on bare AND assembled engines.
CP05_WAVE4_FINALIZED_KINDS = [
    "grid.set",
    "guide.add", "guide.clear", "guide.delete", "guide.move",
    "image.flatten",
    "layer.add", "layer.align", "layer.delete", "layer.distribute",
    "layer.duplicate", "layer.group", "layer.merge_down",
    "layer.merge_visible", "layer.rename", "layer.reorder",
    "layer.set_props", "layer.transform", "layer.ungroup",
]
CP05_WAVE4_STAGE_MAP = (
    "O-CORE lane-log CP-0.5 wave-4 kind-tail entry (sections 40.7/24.2/"
    "12.4 over the pinned order): stage 5 = contract-generated schema; "
    "stage 6 = target/parent/member resolution (unresolved -> bad_target; "
    "non-group parents -> target_kind_mismatch; cycles -> bad_target) -> "
    "section 11.1 locks (transitive ALL; POSITION blocks moves/aligns; "
    "PIXELS blocks merge receivers; set_props' locks-only escape hatch "
    "under ALL) -> scope gates (parametric layer kinds, general "
    "transform matrices/meshes, non-normal merge blends, feathered "
    "bakes, group merge participants -> not_implemented naming their "
    "rows) -> structural/value/tile deltas through the recorder doors "
    "(RemoveSubtree/MoveNode/SetLayerProps/SetGuides; image.flatten is "
    "class C on the snapshot inverse)"
)
CP05_WAVE2_CORE_MASK_STAGE_MAP = (
    "O-CORE lane-log CP-0.5 wave-2 mask entry (section 40.3 over the "
    "pinned order): stage 5 = contract-generated schema; stage 6 = target "
    "resolution (mask.add: layerId owner + slot/duplicate gates "
    "[target_in_use / duplicate_id]; others: layerId + maskId membership) "
    "-> section 11.1 locks (ALL blocks all six; PIXELS blocks mask.edit + "
    "mask.apply; TRANSPARENCY blocks mask.apply; mask.to_selection is "
    "never lock-gated per section 40.4) -> scope gates (vector masks / "
    "from:path / gaussian_blur / feathered bake -> not_implemented, the "
    "O-TOOLS-and-path-arena seams) -> coverage edits through the "
    "recorder doors (section 13.2 exact combine; section 11.2 exact bake "
    "formula)")
CP05_WAVE2_CORE_DOC_STAGE_MAP = (
    "O-CORE lane-log CP-0.5 wave-2 color entry (section 40.2 over the "
    "pinned order): the contract validMinimal passes stage 4 "
    "(implemented) on assembled engines and fails the document-slot "
    "check on the case's FRESH handle (uninitialized -> bad_target); "
    "initialized-context behavior is the CLI sweep's context table "
    "(assign accepts as an untagged retag, section 5.3; depth_convert "
    "to:u16 is feature-gated not_implemented in the default build)")
CP05_WAVE2_CORE_ARBITRATION = (
    CP03_ARBITRATION
    + "; applied at CP-0.5 wave 2: appname-core exports the mask.* and "
    "doc.* color KindImpls (engine::kinds::register); appname-cli "
    "assembles the full engine")
CP05_WAVE2_CORE_NOTE = (
    "bareCore keeps not_implemented at every checkpoint (arbitration #1 "
    "invariant); the accept paths are exercised by the in-crate mask/"
    "color test batteries with real targets, selections, coverage, and "
    "profiles, not by this reject-matrix case")
CP03_STAGE_MAP = (
    "O-CORE lane-log wave-2 stroke entry (section 40.5 over the pinned "
    "order): stage 5 = contract-generated schema; stage 6 = target "
    "resolution (layerId; else typed) -> section 11.1 locks (PIXELS/ALL "
    "block both; TRANSPARENCY constrains paint, BLOCKS eraser) -> "
    "eraseToHistory gate -> samples resolve+decode (parse failure -> "
    "bad_target) -> preset resolve (built-in miss -> bad_target) -> "
    "color decode -> render -> section 40.1 empty-outcome push"
)


# ---- CP-0.5 wave-3 re-entry (O-TOOLS doc geometry + selection kinds) -----
# MECHANICAL companion edit by O-TOOLS following the drift-alarm procedure
# (rev 9) -- flagged for O-QA ratification (reports/O-TOOLS-log.md CP-0.5
# wave-3 entry). Overlay context rule as at rev 8: G-NI cases run on the
# FRESH (uninitialized) handle, so every assembled expectation states the
# fresh-handle truth (the document-slot check fires -> bad_target);
# initialized-context behavior (accepts, out_of_range crop-rect gates,
# selection_required live-state gates) lives in the CLI sweep's context
# table, where that context is created.
CP05_WAVE3_FINALIZED_KINDS = [
    "doc.canvas_resize", "doc.crop", "doc.resize", "doc.rotate",
    "doc.trim", "selection.all", "selection.invert", "selection.modify",
    "selection.none", "selection.set", "selection.transform"]
CP05_WAVE3_STAGE_MAP = (
    "O-TOOLS lane-log CP-0.5 wave-3 entry: the contract validMinimal "
    "passes stage 4 (implemented) on assembled engines and fails the "
    "document-slot check on the case's FRESH handle (uninitialized -> "
    "bad_target). Initialized-context stage-6 orders: doc geometry "
    "(section 40.2) = param/cross-field gates (derived dims 1..=1_000_000"
    " / canvas >= 1x1 / crop-rect intersect-or-allowGrowth / >= 1 trim "
    "side -> out_of_range) -> section 40.2 all-surface walk (layers, "
    "mask coverages, selection) through ONE global mapping via the "
    "recorder tile doors + SetGeometry; typed-pending arms answer "
    "not_implemented (crop hide [no section 19.1 crop_rect core field], "
    "straighten, multi-layer color-basis trim [no composite]); "
    "selections (section 40.4) = live-state gates (modify/transform "
    "require an active selection -> selection_required; wand seed "
    "in-canvas -> out_of_range) -> section 13 engines (exact scanline / "
    "shared wand predicate / EDT refinements / section 18.2 mask "
    "transform) -> selection-surface installs + the section 13.1 "
    "exact-bounds door; none-on-inactive and identity edits answer the "
    "section 40.1 empty push")
CP05_WAVE3_ARBITRATION = (
    CP03_ARBITRATION
    + "; applied at CP-0.5 wave 3: appname-tools exports the doc-geometry"
    " and selection KindImpls (kind_impls::assembly); the lifted FillKind"
    " rides with_tools_fill pending the brokered core/cli/cabi swap")
CP05_WAVE3_NOTE = (
    "bareCore keeps not_implemented at every checkpoint (arbitration #1 "
    "invariant); the accept paths are exercised by the in-crate "
    "doc-geometry/selection engine test batteries with real documents, "
    "content, and live selections, not by this reject-matrix case")

# ---- CP-0.5 wave-1 re-entry (O-CORE fill + edit.cut implementation) ------
# MECHANICAL companion edit by O-CORE following the drift-alarm procedure
# verbatim ("fill `assembled` in the generator, move the kind to
# finalizedKinds, regenerate, and extend the aggregate expectations") —
# flagged for O-QA ratification (reports/O-CORE-log.md CP-0.5 wave-1
# entry). Same arbitration-#1 split: the impls live IN appname-core
# (engine::kinds::register) but bare-core engines (no assembly) keep
# answering not_implemented; only assembled engines implement them.
CP05_WAVE1_FINALIZED_KINDS = ["edit.cut", "edit.paste", "edit.paste_into", "fill"]
CP05_WAVE1_STAGE_MAP = (
    "O-CORE lane-log CP-0.5 wave-1 fill/clipboard entry (section 40.5 "
    "over the pinned order): stage 5 = contract-generated schema; stage 6 "
    "= target resolution (layerId only; mask/channel surfaces answer "
    "typed until their rows) -> section 11.1 locks (PIXELS/ALL block "
    "both; TRANSPARENCY constrains fill [preserveTransparency applies "
    "the same clamp], BLOCKS edit.cut) -> fill content/blend gates "
    "(color/foreground/background + normal blend implemented; gradient/"
    "pattern/content_aware/non-normal blends -> not_implemented; the "
    "flood-seed variant is LIVE since the brokered fill lift -- "
    "ownership moved to appname-tools' FillKind wrapping core's "
    "register(), section 22.6 shared wand predicate) -> "
    "composite/clear through the recorder doors -> "
    "section 40.1 empty-outcome push; paste family: stage-4-analog asset "
    "resolution (params.assetId must name an INGESTED raster asset -> "
    "bad_target) precedes creation-uuid collision gates (duplicate_id) "
    "and paste_into's live-state selection gate (selection_required)"
)
CP05_WAVE1_TARGET_NOTE = {
    "edit.cut": (
        "the contract validMinimal (no params) fails at stage-6 target "
        "resolution — the example target.layerId does not resolve on the "
        "case's fresh handle (a RESOLVING non-raster target would answer "
        "error.command.target_kind_mismatch per the section 40.5 "
        "addition)"),
    "fill": (
        "the contract validMinimal (content: foreground) passes stage-5 "
        "schema and fails at stage-6 target resolution — the example "
        "target.layerId does not resolve on the case's fresh handle "
        "(fill's contract stays inside the section 40.1 baseline: "
        "bad_target; it carries no target_kind_mismatch row)"),
    "edit.paste": (
        "the contract validMinimal passes stage-5 schema and fails at the "
        "stage-4-analog asset resolution — the example params.assetId "
        "does not name an ingested raster asset on the case's fresh "
        "handle (section 40.5: assetId MUST resolve -> bad_target)"),
    "edit.paste_into": (
        "the contract validMinimal passes stage-5 schema and fails at the "
        "stage-4-analog asset resolution — the example params.assetId "
        "does not name an ingested raster asset on the case's fresh "
        "handle; the asset gate PRECEDES the live-state selection gate "
        "(pinned-order discipline), so bad_target wins over "
        "selection_required here"),
}


# ---- CP-0.7 wave-1 re-entry (O-CORE OpNode family + daggers) -------------
# MECHANICAL companion edit by O-CORE (rev 11, drift-alarm procedure;
# rev 12 = the OpNode fix-round stage-map refresh: unknown_op_kind,
# PIXELS gates, dagger position coordinates -- same case set/counts) --
# flagged for O-QA ratification. Fresh-handle rule as before for the
# eleven assembled kinds (bad_target: doc-slot check). The TWO history
# daggers are ENGINE-BUILT-INS (doc.new precedent -- they manipulate the
# history machinery no KindImpl can reach), so their bareCore expectation
# moves off not_implemented for the first time: bare AND assembled
# engines answer the uninitialized doc-slot check on the fresh handle.
CP07_WAVE1_FINALIZED_KINDS = [
    "adjustment.add", "op.delete", "op.reorder", "op.set_mask",
    "op.set_params", "op.toggle",
    "smartobject.convert", "smartobject.edit_contents",
    "smartobject.rasterize", "smartobject.replace",
    "layer.rasterize",
    "history.goto", "history.snapshot",
]
CP07_DAGGER_KINDS = ["history.goto", "history.snapshot"]

# ---- CP-0.7 wave-2 re-entry (O-TOOLS wiring slice, rev-14 finalize) ------
# The five wiring-slice kinds went live in the SHIPPED assembly at the
# ratified ask-1 wiring (reports/O-TOOLS-log.md 2026-08-14 + O-INT
# addendum); the rev-13 drift alarms tripped by design at the O-QA
# finalize (assembled_engine lifted to the five-crate composition) and
# this revision resolves them.  Fresh-handle rule as every prior wave:
# validMinimal passes stage 4 (implemented) and fails the document-slot
# check on the case's FRESH handle (uninitialized -> bad_target;
# observed: "document not initialized; the first command must be
# doc.new").  bareCore keeps not_implemented (arbitration #1 invariant:
# the filters/adjust registrations ride the assembly wrap, not core).
# ---- CP-1.0 door slice re-entry (O-CORE vector family, rev-15 finalize) --
# The seven vector.* kinds + selection.from_path went live in appname-core
# (engine::kinds::register, the CP-1.0 door slice) with NO VectorGeometry
# service registered on the shipped assembly (appname-vector's register()
# lands with the TEXTVEC wiring slice).  Fresh-handle rule as every prior
# wave: validMinimal passes stage 4 (implemented) and fails the
# document-slot check (uninitialized -> bad_target).  bareCore ALSO
# answers bad_target: these KindImpls live in core's own register(), which
# the bare-corpus assembled_engine... no -- bareCore engines in this
# corpus are EngineAssembly::new() WITHOUT kinds::register, so bareCore
# keeps not_implemented (arbitration #1 invariant).
CP10_DOOR_SLICE_KINDS = [
    "vector.path_new", "vector.path_edit", "vector.path_delete",
    "vector.path_bool", "vector.shape_add", "vector.shape_set_style",
    "vector.edit", "selection.from_path",
]

# ---- CP-1.0 door slice 2 re-entry (O-CORE text family, TEXTVEC Doors ----
# 6-10). MECHANICAL companion edit by O-CORE following the drift-alarm
# procedure verbatim (the 0dceb11 precedent) -- FLAGGED for O-QA
# ratification (reports/O-CORE-log.md door-slice-2 entry). The five
# text.* kinds went live in appname-core (engine::kinds::register) with
# NO TextLayout service registered on the shipped assembly (appname-text's
# register lands at its wiring slice, one PRODUCTION_CHAIN row).
# Fresh-handle rule as every prior wave: validMinimal passes stage 4 and
# fails the document-slot check (uninitialized -> bad_target). bareCore
# keeps not_implemented (bare corpus engines carry no registrations).
CP10_DOOR_SLICE_2_KINDS = [
    "text.add", "text.edit", "text.set_style", "text.set_paragraph",
    "text.to_shape",
]

# ---- CP-1.0 door slice 3 re-entry (O-CORE: the ten-doors slice) ----------
# MECHANICAL companion edit by O-CORE following the drift-alarm procedure
# (rev 17) -- flagged for O-QA ratification (reports/O-CORE-log.md door
# slice 3 entry). Fresh-handle rule as every prior wave: all thirteen
# kinds pass stage 4 on assembled engines (core's own kinds::register
# carries them) and fail the document-slot check on the corpus's FRESH
# handle -> bad_target; richer initialized-context behavior (accepts,
# the section 9.6 creation-id cascade, dangling targets) lives in the
# CLI sweep's context table, where that context is created. bareCore
# keeps not_implemented (arbitration #1 invariant).
CP10_DOOR_SLICE_3_KINDS = [
    "channel.add", "channel.apply_image", "channel.calculations",
    "channel.delete", "channel.duplicate", "channel.edit",
    "doc.mode_convert", "meta.set",
    "selection.load", "selection.save",
    "slice.add", "slice.delete", "slice.edit",
]
CP10_DOOR_SLICE_3_STAGE_MAP = (
    "O-CORE lane-log CP-1.0 door-slice-3 entry (sections 14/29.2/40.4/"
    "40.8/6.2 over the pinned order): channel targets resolve "
    "target.channelId (unresolved/component -> bad_target, section 14.1 "
    "derived-view divergence recorded); section 14.3 spot params answer "
    "typed error.unsupported.spot_channels; selection.save gates on the "
    "active selection (selection_required) then the section 9.6 global "
    "creation-id discipline (duplicate_id); apply_image/calculations "
    "dispatch on render's injected kernel routing (separable/triple/"
    "dissolve-reserved/legacy add-subtract) AFTER target+source "
    "resolution (mode-gated component selectors -> "
    "target_kind_mismatch; foreign docId -> bad_target, single-document "
    "posture); slice.* gate live-state index + canvas-intersecting "
    "bounds (out_of_range); meta.set validates per registered key "
    "(out_of_range/missing_field); doc.mode_convert orders the section "
    "40.2 profile gates (profile_space_mismatch) BEFORE the cmyk_a/"
    "parametric typed refusals, then executes through ONE cached CMM "
    "transform with the class-C snapshot inverse")
CP10_DOOR_SLICE_2_STAGE_MAP = (
    "O-CORE lane-log CP-1.0 door-slice-2 entry (sections 40.7/23.1-23.5 "
    "over the pinned order): every kind except text.add resolves "
    "target.layerId (unresolved -> bad_target; non-Text -> "
    "target_kind_mismatch) BEFORE the service gate; content-mutating "
    "kinds gate PIXELS/ALL through the one-place effective-lock set; "
    "range params check start<=end arm-side and end<=len service-side "
    "(both out_of_range, the family rule); text.add checks on_path "
    "pathId resolution (bad_target) and parent group-ness "
    "(target_kind_mismatch) with ALL on the parent chain blocking; "
    "text-requiring execution answers the typed NI mirror "
    "(error.command.not_implemented naming the TextLayout seam) until "
    "appname-text registers; to_shape is the Door-9 bake -- its OUTPUT "
    "geometry value-captures through the vector capture table (D-48), "
    "and it ALSO requires the VectorGeometry service (the "
    "service-to-service seam)")

# ---- CP-1.0 consolidated O-TOOLS round (content_aware_fill consumer) -----
# MECHANICAL companion edit by O-TOOLS following the drift-alarm
# procedure verbatim (the 0dceb11 precedent above) -- FLAGGED for O-QA
# ratification (reports/O-TOOLS-log.md CP-1.0 consolidated-round entry).
# ContentAwareFillKind rides kind_impls::assembly (arbitration #1: bare
# core never sees it).  Fresh-handle rule as every prior wave.
CP10_TOOLS_ROUND_KINDS = ["content_aware_fill"]

# ---- CP-1.0 warps row (O-TOOLS transform.puppet) -------------------------
# MECHANICAL companion edit by O-TOOLS following the drift-alarm
# procedure verbatim (the 0dceb11 precedent) -- FLAGGED for O-QA
# ratification (reports/O-TOOLS-log.md warps-row entry).
# TransformPuppetKind rides kind_impls::assembly (arbitration #1: bare
# core never sees it).  Fresh-handle rule as every prior wave.
CP10_WARPS_ROW_KINDS = ["transform.puppet"]
CP10_WARPS_ROW_STAGE_MAP = (
    "O-TOOLS lane-log warps-row entry (sections 45.5/18.4/40.6 over the "
    "pinned order): target resolves raster-first (unresolved -> "
    "bad_target; parametric split later wave); section-11.1 "
    "transform-family locks (POSITION/ALL block, PIXELS does not); "
    "resample name validated; pins re-validated typed (1..=512, finite "
    "pos/rotationDeg, integer depth); the section-40.6 live-state "
    "pin-in-content-bounds gate (empty layer = the zero-area-alpha-hull "
    "state -> out_of_range naming tool.puppet.empty_layer); then the "
    "wire-form arm split: no rotationDeg anywhere = the section-45.5 "
    "identity fixed point (targets == rests on the wire, one pos per "
    "pin) -> bit-exact no-op, section-40.1 empty push; any rotationDeg "
    "= the wire-expressible deformation -> the elected ARAP backend "
    "(libigl behind the ruled cpp-shim shape, feature arap-libigl) or "
    "the typed backend not_implemented on default builds "
    "(validation-complete posture); the wire-cannot-carry-drag-targets "
    "contract gap is RECORDED + ESCALATED, never invented around")
# ---- CP-1.0 retouch row (O-TOOLS: the five stroke kinds) -----------------
# MECHANICAL companion edit by O-TOOLS following the drift-alarm
# procedure verbatim (the 0dceb11 precedent) -- FLAGGED for O-QA
# ratification (reports/O-TOOLS-log.md retouch-row entry).  The five
# KindImpls ride kind_impls::assembly (arbitration #1: bare core never
# sees them).  Fresh-handle rule as every prior wave.  After this row
# the remaining NI set is exactly stroke_path (O-TEXTVEC follow-on) +
# doc.duplicate (dagger, permanently NI by design) -- the d07
# transactional-probe design EXHAUSTS at stroke_path's flip (decision
# to O-INT; O-CORE lane-log process note).
# ---- CP-1.0 stroke_path slice re-entry (O-TEXTVEC) ----
# The LAST registry kind (rev 19): the arm lands beside FillKind
# (section-10.3 fill/stroke family), geometry through the Door-1 stroke
# arms (section-20.2 position rule in the service). MECHANICAL companion
# by O-TEXTVEC, flagged for O-QA ratification (reports/O-TEXTVEC-log.md).
# Remaining pending set after this slice: doc.duplicate ONLY (dagger,
# permanently NI by design) -- TERMINAL.
CP10_STROKE_PATH_KINDS = ["stroke_path"]

CP10_RETOUCH_ROW_KINDS = [
    "clone.stroke", "heal.stroke", "dodge_burn.stroke",
    "smudge.stroke", "blur_sharpen.stroke",
]
CP10_RETOUCH_ROW_STAGE_MAP = (
    "O-TOOLS lane-log retouch-row entry (sections 40.5/21.1/21.2/21.5/"
    "45.1/45.2 over the pinned order): family target resolution first "
    "(layer-only, the standing brush posture; unresolved -> bad_target) "
    "-> section-11.1 locks (PIXELS/ALL block all five; TRANSPARENCY "
    "constrains clone/smudge, no-op for the alpha-preserving kinds) -> "
    "samples asset + preset resolve (bad_target) -> the per-kind pinned "
    "cross-field micro-orders: clone section-45.2 tri-state "
    "(composite+sampleAllLayers:false -> out_of_range) then the typed "
    "composite-sampling doors (not_implemented, sampling_source "
    "posture); heal spot-forbids-source/offset out_of_range, "
    "sampled|pattern -> error.schema.exclusive_params, "
    "healing-requires -> missing_field, pattern -> typed "
    "error.retouch.pattern_missing (no pattern arena in v1); "
    "blur_sharpen protectDetail-with-blur on RAW params -> "
    "out_of_range; smudge fingerPainting -> typed not_implemented "
    "(contract gap: no color field to capture the record-time "
    "effective color); spot proximity failure -> typed "
    "error.retouch.no_source_found; heal commit = appname-algo "
    "poisson_heal multigrid (section 45.1), spot content_aware = the "
    "section-45.3 exemplar engine with H = Omega and the "
    "blake3(mergeKey) seed")
CP10_TOOLS_ROUND_STAGE_MAP = (
    "O-TOOLS lane-log CP-1.0 consolidated-round entry (sections 40.5/45.3 "
    "over the pinned order): cross-fields first (customRegion without "
    "sampleArea custom -> out_of_range; output != current_layer without "
    "newLayerId -> missing_field), then the typed door refusals (output "
    "new_layer/duplicate_layer -> not_implemented naming the missing "
    "public raster-node construction door; sampleArea custom -> "
    "not_implemented naming the channel-arena row; target.maskId source "
    "-> not_implemented naming the unpinned output semantics), then "
    "stage-4 target resolution (unresolved -> bad_target; non-raster -> "
    "target_kind_mismatch), the live-state selection gate "
    "(selection_required: the selection IS the hole H), the section-11.1 "
    "effective-locks PIXELS gate for current_layer, and the section-45.3 "
    "deterministic exemplar engine (failure rule 2 all-hole -> "
    "invalid_arg pending its dedicated wire id; failure rule 1 starved "
    "-> SUCCEEDS via diffusion fallback)")
CP10_DOOR_SLICE_STAGE_MAP = (
    "O-CORE lane-log CP-1.0 door-slice entry (sections 40.7/40.4/20.1-20.4 "
    "over the pinned order): path kinds address params.pathId (unresolved "
    "-> bad_target); path_bool is the section 40.7 XOR semantic check "
    "(params.pathIds XOR shape-layer target -> missing_field) with "
    "shape-mode locks PIXELS/ALL via the one-place effective set; shape "
    "kinds resolve target.layerId and answer target_kind_mismatch on "
    "non-shape targets; geometry-requiring execution answers the typed "
    "NI mirror (error.command.not_implemented naming the VectorGeometry "
    "seam) until appname-vector registers; shape_add value-captures its "
    "generated sidecar (D-48; journal captures beside the journal)")

CP07_WAVE2_FINALIZED_KINDS = [
    "filter.apply", "filter.smart_add",
    "image.auto_color", "image.auto_contrast", "image.auto_tone",
]
CP07_WAVE2_STAGE_MAP = (
    "O-TOOLS lane-log 2026-08-14 wiring-slice + tail-flip entries "
    "(sections 40.6/12.1/16.1/17.2 over the pinned order): filter kinds "
    "route params.filterId through the live section 16.1 registry "
    "(unregistered -> error.schema.unknown_op_kind), validate nested "
    "params against the REGISTRY-OWNED per-op schema (authoritative per "
    "the filter.apply notes precedent), then resolve target "
    "(non-raster for filter.apply / non-smart for filter.smart_add -> "
    "target_kind_mismatch; unresolved -> bad_target); image.auto_* "
    "kinds take an optional targetLayer and bake section 17.2 analyses "
    "through the Levels machinery (initialized-context accepts are the "
    "CLI sweep's table: identity-adjacent on the all-white scratch doc)")
CP07_WAVE1_STAGE_MAP = (
    "O-CORE lane-log CP-0.7 wave-1 + fix-round entry (sections 40.6/11.5/"
    "12.4/40.8 over the pinned order): op kinds address params.opId "
    "(unresolved -> bad_target; owner-ancestry ALL blocks; the "
    "content-rewriting smartobject/rasterize kinds block on PIXELS too, "
    "section 40.7 -- fix-round M6); adjustment.add re-validates op params "
    "against the assembly's section 12.1 registry (unregistered opKind -> "
    "error.schema.unknown_op_kind, the section 40.6 Part-40 addition -- "
    "fix-round M9 ruling); smartobject "
    "kinds gate on embedded raster sources (linked/nested/multi-layer -> "
    "not_implemented naming their rows); rasterize is class C on the "
    "snapshot inverse (enabled filter chains and non-identity placement "
    "-> not_implemented until the kernel/resample rows); the daggers "
    "execute OUTSIDE the undo stack -- snapshot journals as a MARKER with "
    "a no-op history placeholder keeping journal-history correspondence "
    "exact (fix-round B1), goto{position} addresses UNDO-STACK "
    "coordinates (markers hold no slot; out-of-range and in-range-but-"
    "unreachable -> error.schema.out_of_range, fix-round M3/B1) and "
    "re-runs undo/redo appending nothing, goto{snapshot} "
    "journals as a class-C restore replaying against marker-recreated "
    "snapshots")


# ---- 2026-08-17 strictness ruling re-encode (O-TOOLS sweep slice) --------
# MECHANICAL companion edit by O-TOOLS, observe-first re-encoding
# (drift-alarm procedure; FLAGGED for O-QA ratification with the
# strictness-sweep lane-log entry): the ruled schema-letter validation
# (unknown params refuse typed per additionalProperties:false) changes
# what the shipped binary PRINTS for the three kinds whose FROZEN
# contract validMinimal examples carry dead-shaped nested params --
# filter.apply {sigmaPx} (divergence already registered for fold-in),
# filter.smart_add {radiusPx}, adjustment.add {channels:{master:
# {points}}} (same dead-example class; engine schemas follow their
# normative rows). Stage-3 nested param validation precedes stage-6
# target resolution (section 40.1 pinned order), so these envelopes now
# answer error.schema.out_of_range on assembled engines instead of the
# dangling-target bad_target. bareCore keeps not_implemented untouched.
STRICT_DEAD_EXAMPLE_KINDS = ["adjustment.add", "filter.apply",
                             "filter.smart_add"]


def cp03_overlay(kind):
    o = _cp03_overlay_base(kind)
    if o is not None and kind in STRICT_DEAD_EXAMPLE_KINDS:
        # The assembled CODE stays the FRESH-handle truth (bad_target:
        # the document-slot check precedes stage-3 nested validation on
        # an uninitialized handle -- verified in-process); only the
        # INITIALIZED-context answer moved, and that lives in the CLI
        # sweep's context table where the context is created.
        o = dict(o)
        o["assembledProvenance"] = (
            "2026-08-17 strictness re-encode (observe-first, shipped-binary "
            "sweep): the contract validMinimal's nested params use a "
            "dead-shaped example (sigmaPx / radiusPx / channels-wrapper "
            "class; frozen mirror, divergence registered for fold-in); on "
            "an INITIALIZED doc the ruled schema-letter validation refuses "
            "the unknown keys typed at stage 3 before target resolution "
            "(CLI-sweep context row); on the FRESH corpus handle the "
            "document-slot bad_target still answers first; "
            + o["assembledProvenance"])
    return o


def _cp03_overlay_base(kind):
    if kind in CP10_DOOR_SLICE_3_KINDS:
        return {
            "status": "finalized-cp10-door-slice-3",
            "bareCore": reject("error.command.not_implemented"),
            "assembled": reject("error.command.bad_target"),
            "assembledProvenance": (
                "stage map: the contract validMinimal passes stage 4 on "
                "assembled engines (core's own kinds::register carries "
                "the thirteen door-slice-3 kinds since CP-1.0 door slice "
                "3) and fails the document-slot check on the case's "
                "FRESH handle (uninitialized -> bad_target); "
                "initialized-context behavior is the CLI sweep's context "
                "table (accepts for channel.add/doc.mode_convert/"
                "meta.set/slice.add/slice.delete; the section 9.6 "
                "creation-id cascade; dangling targets -> bad_target); "
                + CP10_DOOR_SLICE_3_STAGE_MAP),
            "arbitration": CP03_ARBITRATION
            + "; applied at CP-1.0 door slice 3: appname-core's "
              "engine::kinds::register grew the channels/slices/meta/"
              "mode-convert/apply-image families; the section 14.4 "
              "kernel routing rides appname-assembly's "
              "with_render_blend_kernels PRODUCTION_CHAIN row",
            "note": "bareCore keeps not_implemented at every checkpoint "
                    "(bare corpus engines carry no kind registrations at "
                    "all); accept paths, section 34 ptr_eq COW, the "
                    "section 14.4 bit-identity legs, and the adversarial "
                    "gates are exercised by the in-crate kinds_channels/"
                    "kinds_slices_meta/kinds_mode/kinds_apply_image "
                    "batteries, not by this reject-matrix case",
        }
    if kind in CP10_DOOR_SLICE_2_KINDS:
        return {
            "status": "finalized-cp10-door-slice-2",
            "bareCore": reject("error.command.not_implemented"),
            "assembled": reject("error.command.bad_target"),
            "assembledProvenance": (
                "stage map: the contract validMinimal passes stage 4 on "
                "assembled engines (core's own kinds::register carries the "
                "text family since CP-1.0 door slice 2) and fails the "
                "document-slot check on the case's FRESH handle "
                "(uninitialized -> bad_target); initialized-context "
                "behavior is the CLI sweep's context table (dangling "
                "validMinimal targets -> bad_target for the four "
                "resolving kinds; text.add -> the typed TextLayout-NI "
                "mirror); " + CP10_DOOR_SLICE_2_STAGE_MAP),
            "arbitration": CP03_ARBITRATION
            + "; applied at CP-1.0 door slice 2: appname-core's "
              "engine::kinds::register grew the text family; the "
              "TextLayout service registration is appname-text's wiring "
              "slice (one appname-assembly PRODUCTION_CHAIN row)",
            "note": "bareCore keeps not_implemented at every checkpoint "
                    "(bare corpus engines carry no kind registrations at "
                    "all); text-requiring accept paths are exercised by "
                    "the in-crate kinds_text battery with the probe "
                    "service, not by this reject-matrix case",
        }
    if kind in CP10_STROKE_PATH_KINDS:
        return {
            "status": "finalized-cp10-stroke-path-slice",
            "bareCore": reject("error.command.not_implemented"),
            "assembled": reject("error.command.bad_target"),
            "assembledProvenance": (
                "stage map: on a FRESH handle the document-slot check "
                "precedes everything (no doc.new has run), answering "
                "bad_target before stage-4 source resolution; on an "
                "initialized context validMinimal's dangling "
                "source.pathId answers stage-4 bad_target from the arm "
                "-- both observed on the shipped surface before "
                "encoding (observe-first rule)"),
            "arbitration": CP03_ARBITRATION
            + "; applied at the O-TEXTVEC stroke_path slice: "
              "appname-tools' kind_impls::assembly grew StrokePathKind "
              "beside FillKind (the section-10.3 fill/stroke family), "
              "with geometry crossing the Door-1 seam "
              "(stroke_path_coverage/stroke_boundary_coverage; "
              "section-20.2 position rule in the service)",
            "note": "bareCore keeps not_implemented at every checkpoint "
                    "(arbitration #1 invariant); the tool:'brush' arm "
                    "answers typed not_implemented after its brushPreset "
                    "cross-field (the rasterize-wire ruling's interim "
                    "wire; the brush-engine trace lands with its own "
                    "slice); the LAST registry kind -- 102/102 minus the "
                    "permanent dagger",
        }
    if kind in CP10_RETOUCH_ROW_KINDS:
        return {
            "status": "finalized-cp10-retouch-row",
            "bareCore": reject("error.command.not_implemented"),
            "assembled": reject("error.command.bad_target"),
            "assembledProvenance": (
                "stage map: the contract validMinimal passes stage 4 on "
                "assembled engines (appname-tools' kind_impls::assembly "
                "carries the five retouch KindImpls since the CP-1.0 "
                "retouch row) and fails the document-slot check on the "
                "case's FRESH handle (uninitialized -> bad_target); "
                "initialized-context behavior: each validMinimal's "
                "dangling target layerId keeps bad_target (stage-6 "
                "target resolution precedes every arm; OBSERVED on the "
                "shipped CLI this round for all five); "
                + CP10_RETOUCH_ROW_STAGE_MAP),
            "arbitration": CP03_ARBITRATION
            + "; applied at the CP-1.0 retouch row: appname-tools' "
              "kind_impls::assembly grew the five stroke KindImpls over "
              "the crate's retouch pure engines (heal consumes the "
              "merged appname-algo section-45.1 poisson core, item 5; "
              "spot content-aware rides the section-45.3 exemplar "
              "engine, CAF-consumer precedent)",
            "note": "bareCore keeps not_implemented at every checkpoint "
                    "(arbitration #1 invariant); accept paths, the "
                    "contract examples.invalid matrix (incl. "
                    "exclusive_params + pattern_missing), lock/selection "
                    "gates, and replay determinism are exercised by the "
                    "in-crate kind_impls_retouch battery and the retouch "
                    "engine tests, not by this reject-matrix case",
        }
    if kind in CP10_WARPS_ROW_KINDS:
        return {
            "status": "finalized-cp10-warps-row",
            "bareCore": reject("error.command.not_implemented"),
            "assembled": reject("error.command.bad_target"),
            "assembledProvenance": (
                "stage map: the contract validMinimal passes stage 4 on "
                "assembled engines (appname-tools' kind_impls::assembly "
                "carries TransformPuppetKind since the CP-1.0 warps row) "
                "and fails the document-slot check on the case's FRESH "
                "handle (uninitialized -> bad_target); initialized-context "
                "behavior: the validMinimal's dangling layerId keeps "
                "bad_target (stage-6 target resolution precedes every "
                "arm); " + CP10_WARPS_ROW_STAGE_MAP),
            "arbitration": CP03_ARBITRATION
            + "; applied at the CP-1.0 warps row: appname-tools' "
              "kind_impls::assembly grew TransformPuppetKind over the "
              "in-crate puppet engines (mesh/ARAP driver/rasterizer; "
              "MLS rigid = appname-algo item 9)",
            "note": "bareCore keeps not_implemented at every checkpoint "
                    "(arbitration #1 invariant); the identity-arm accept "
                    "path, the reject matrix, the lock split, and the "
                    "empty-layer gate are exercised by the in-crate "
                    "transform_tests puppet battery with real documents, "
                    "not by this reject-matrix case",
        }
    if kind in CP10_TOOLS_ROUND_KINDS:
        return {
            "status": "finalized-cp10-tools-round",
            "bareCore": reject("error.command.not_implemented"),
            "assembled": reject("error.command.bad_target"),
            "assembledProvenance": (
                "stage map: the contract validMinimal passes stage 4 on "
                "assembled engines (appname-tools' kind_impls::assembly "
                "carries ContentAwareFillKind since the CP-1.0 "
                "consolidated O-TOOLS round) and fails the document-slot "
                "check on the case's FRESH handle (uninitialized -> "
                "bad_target); initialized-context behavior: the "
                "validMinimal's defaulted output new_layer answers the "
                "typed layer-creation-door not_implemented "
                "(validation-complete: the newLayerId cross-field is "
                "enforced first); " + CP10_TOOLS_ROUND_STAGE_MAP),
            "arbitration": CP03_ARBITRATION
            + "; applied at the CP-1.0 consolidated O-TOOLS round: "
              "appname-tools' kind_impls::assembly grew "
              "ContentAwareFillKind over the merged appname-algo "
              "section-45.3 exemplar engine (Row A)",
            "note": "bareCore keeps not_implemented at every checkpoint "
                    "(arbitration #1 invariant); the live current_layer "
                    "accept path, the failure-rule pair, and the lock/"
                    "selection gates are exercised by the in-crate "
                    "caf_kind_tests battery with real documents and "
                    "selections, not by this reject-matrix case",
        }
    if kind in CP10_DOOR_SLICE_KINDS:
        return {
            "status": "finalized-cp10-door-slice",
            "bareCore": reject("error.command.not_implemented"),
            "assembled": reject("error.command.bad_target"),
            "assembledProvenance": (
                "stage map: the contract validMinimal passes stage 4 on "
                "assembled engines (core's own kinds::register carries the "
                "vector family since the CP-1.0 door slice) and fails the "
                "document-slot check on the case's FRESH handle "
                "(uninitialized -> bad_target); initialized-context "
                "behavior is the CLI sweep's context table (dangling "
                "validMinimal ids -> bad_target for the six resolving "
                "kinds; path_new/shape_add -> the typed geometry-NI "
                "mirror); " + CP10_DOOR_SLICE_STAGE_MAP),
            "arbitration": CP03_ARBITRATION
            + "; applied at the CP-1.0 door slice (rev-15 finalize): "
              "appname-core's engine::kinds::register grew the vector "
              "family; the VectorGeometry service registration is the "
              "TEXTVEC wiring slice's",
            "note": "bareCore keeps not_implemented at every checkpoint "
                    "(bare corpus engines carry no kind registrations at "
                    "all); geometry-requiring accept paths are exercised "
                    "by the in-crate kinds_vector battery with the probe "
                    "service, not by this reject-matrix case",
        }
    if kind in CP07_WAVE2_FINALIZED_KINDS:
        return {
            "status": "finalized-cp07-wave2",
            "bareCore": reject("error.command.not_implemented"),
            "assembled": reject("error.command.bad_target"),
            "assembledProvenance": (
                "stage map: the contract validMinimal passes stage 4 on "
                "the SHIPPED five-crate assembly (adjust/filters register "
                "wraps, ratified ask-1 wiring) and fails the "
                "document-slot check on the case's FRESH handle "
                "(uninitialized -> bad_target); initialized-context "
                "behavior is the CLI sweep's context table (image.auto_* "
                "accept identity-adjacent; filter.* fall through to "
                "stage-6 unresolved-target bad_target); "
                + CP07_WAVE2_STAGE_MAP),
            "arbitration": CP03_ARBITRATION
            + "; applied at CP-0.7 wave 2 (rev-14 finalize): "
              "appname-filters/appname-adjust export register() wraps "
              "consumed by the cli/cabi assembly points AND the O-QA "
              "assembled_engine since the finalize round",
            "note": "bareCore keeps not_implemented at every checkpoint "
                    "(arbitration #1 invariant: these KindImpls ride the "
                    "filters/adjust register wraps, which bare core never "
                    "sees); the accept/bake paths are exercised by the "
                    "in-crate auto_kinds/filter_kinds batteries and the "
                    "cp07ParamEdges prime kit, not by this reject-matrix "
                    "case",
        }
    if kind in CP07_WAVE1_FINALIZED_KINDS:
        bare = (
            reject("error.command.bad_target")
            if kind in CP07_DAGGER_KINDS
            else reject("error.command.not_implemented")
        )
        return {
            "status": "finalized-cp07-wave1",
            "bareCore": bare,
            "assembled": reject("error.command.bad_target"),
            "assembledProvenance": (
                "stage map: the contract validMinimal passes stage 4 on "
                "assembled engines (daggers: on EVERY engine -- built-ins) "
                "and fails the document-slot check on the case's FRESH "
                "handle (uninitialized -> bad_target); initialized-context "
                "behavior is the CLI sweep's context table; "
                + CP07_WAVE1_STAGE_MAP),
            "arbitration": CP03_ARBITRATION
            + "; applied at CP-0.7 wave 1: appname-core exports the "
              "OpNode-family KindImpls (engine::kinds::register); the "
              "history daggers are engine-built-ins per the doc.new "
              "precedent",
            "note": "the DAGGER rows are the first bareCore departure from "
                    "not_implemented: built-ins answer the doc gate on "
                    "bare engines too (doc.new precedent); the accept "
                    "paths are exercised by the in-crate kinds_ops "
                    "batteries, not by this reject-matrix case",
        }

    if kind in CP05_WAVE4_FINALIZED_KINDS:
        return {
            "status": "finalized-cp05-wave4",
            "bareCore": reject("error.command.not_implemented"),
            "assembled": reject("error.command.bad_target"),
            "assembledProvenance": (
                "stage map: the contract validMinimal passes stage 4 "
                "(implemented) on assembled engines and fails the "
                "document-slot check on the case's FRESH handle "
                "(uninitialized -> bad_target); initialized-context "
                "behavior is the CLI sweep's context table; "
                + CP05_WAVE4_STAGE_MAP),
            "arbitration": CP03_ARBITRATION
            + "; applied at CP-0.5 wave 4: appname-core exports the "
              "layer/guide/grid/merge KindImpls (engine::kinds::register)",
            "note": "bareCore keeps not_implemented at every checkpoint "
                    "(arbitration #1 invariant); the accept paths are "
                    "exercised by the in-crate wave-4 batteries "
                    "(kinds_layer/_geom/_merge/_guides) with real trees, "
                    "content, and furniture, not by this reject-matrix "
                    "case",
        }
    if kind in CP05_WAVE3_FINALIZED_KINDS:
        return {
            "status": "finalized-cp05-wave3",
            "bareCore": reject("error.command.not_implemented"),
            "assembled": reject("error.command.bad_target"),
            "assembledProvenance": "stage map: " + CP05_WAVE3_STAGE_MAP,
            "arbitration": CP05_WAVE3_ARBITRATION,
            "note": CP05_WAVE3_NOTE,
        }
    if kind in CP05_WAVE2_CORE_MASK_KINDS or kind in CP05_WAVE2_CORE_DOC_KINDS:
        if kind in CP05_WAVE2_CORE_MASK_KINDS:
            provenance = (
                "stage map: the contract validMinimal passes stage-5 "
                "schema and fails at stage-6 target resolution -- the "
                "example target ids do not resolve on the case's fresh "
                "handle; " + CP05_WAVE2_CORE_MASK_STAGE_MAP)
        else:
            provenance = "stage map: " + CP05_WAVE2_CORE_DOC_STAGE_MAP
        return {
            "status": "finalized-cp05-wave2",
            "bareCore": reject("error.command.not_implemented"),
            "assembled": reject("error.command.bad_target"),
            "assembledProvenance": provenance,
            "arbitration": CP05_WAVE2_CORE_ARBITRATION,
            "note": CP05_WAVE2_CORE_NOTE,
        }
    if kind in CP05_WAVE2_FINALIZED_KINDS:
        return {
            "status": "finalized-cp05-wave2",
            "bareCore": reject("error.command.not_implemented"),
            "assembled": reject("error.command.bad_target"),
            "assembledProvenance": CP05_WAVE2_PROVENANCE,
            "arbitration": CP05_WAVE2_ARBITRATION,
            "note": CP05_WAVE2_NOTE,
        }
    if kind in CP05_WAVE1_FINALIZED_KINDS:
        return {
            "status": "finalized-cp05-wave1",
            "bareCore": reject("error.command.not_implemented"),
            "assembled": reject("error.command.bad_target"),
            "assembledProvenance": (
                "stage map: " + CP05_WAVE1_TARGET_NOTE[kind] + "; "
                + CP05_WAVE1_STAGE_MAP),
            "arbitration": CP03_ARBITRATION
            + "; applied at CP-0.5 wave 1: appname-core exports the "
              "fill/edit.cut KindImpls (engine::kinds::register); "
              "appname-cli assembles the full engine",
            "note": "bareCore keeps not_implemented at every checkpoint "
                    "(arbitration #1 invariant); the accept path is "
                    "exercised by the in-crate fill/edit.cut test battery "
                    "with real targets and selections, not by this "
                    "reject-matrix case",
        }
    if kind not in CP03_FINALIZED_KINDS:
        return None
    return {
        "status": "finalized-cp03",
        "bareCore": reject("error.command.not_implemented"),
        "assembled": reject("error.command.bad_target"),
        "assembledProvenance": (
            "stage map: the contract validMinimal passes stage-5 schema "
            "(required present) and fails at stage-6 target resolution — "
            "the example target.layerId does not resolve on the case's "
            "fresh handle; " + CP03_STAGE_MAP),
        "arbitration": CP03_ARBITRATION,
        "note": "bareCore keeps not_implemented at every checkpoint "
                "(arbitration #1 invariant); the accept path is exercised "
                "by the stroke goldens and workflow runners with real "
                "targets and samples assets, not by this reject-matrix "
                "case",
    }


# ---- CP-0.7 param-edge reject expansion (O-QA wave 2, corpus rev 13) -----
# Review defect-8 assignment (integration log 2026-08-13, "OpNode review:
# FAIL" entry): corpus rev 11 had zero param-edge rejects for the new
# kinds.  This section encodes the contracts' examples.invalid for the 15
# CP-0.7 kinds as PE-cases under a NEW per-kind-file key `cp07ParamEdges`
# (the historical `cases` arrays and their 165/0/1 aggregate pins are
# untouched -- additive by design so the parallel O-TOOLS count-pin round
# collides on nothing here).
#
# RULING CODIFIED (cited per case that uses it): integration log
# 2026-08-13 -- "Defect 8 divergence (stage-3 mistyped->out_of_range vs
# contract missing_field): STANDS -- this is the OQ-3 micro-order election
# already recorded as a divergence in the CP-0.5 core log; joins the
# fold-in register; corpus may codify it."  Mistyped-type and
# extra/unknown params therefore pin error.schema.out_of_range, NOT the
# contract fixture spelling.  oneOf violations are NOT covered by the
# ruling: the engine spells them error.schema.missing_field exactly as
# the contracts do (observed: "exactly one of the parameter alternatives
# ... must be present (stage-3 oneOf)").
#
# SEAM STATE (rev-14 finalize, 2026-08-14): the section 12.1 op registry
# is LIVE (8 adjustments + 7 filters; O-TOOLS wiring + ratified ask-1 +
# tail flip), and assembled_engine carries the shipped five-crate
# composition.  The rev-13 alarms for that seam TRIPPED by design and
# are resolved below with premise-real re-encodings (live-op primes).
# Arms still carrying `pendingArm` await a flagged O-CORE fix/ruling or
# an unshipped wave; the runner failing on one after that landing IS the
# drift alarm; finalize via the standing companion-edit + O-QA
# ratification procedure.
#
# All expectations below were OBSERVED on the shipped CLI (main 14e8d66,
# 2026-08-13) before encoding -- nothing hand-invented (rule 7).  The
# prime kit is deterministic: fixed envelope ids, content-derived created
# ids (the runner asserts the embedded literals against prime receipts).

PE_RULING = (
    "integration-log ruling 2026-08-13 (OpNode review, defect 8): "
    "stage-3 mistyped/extra params -> error.schema.out_of_range STANDS "
    "over the contract fixture spelling (missing_field); OQ-3 "
    "micro-order election, fold-in register")
PE_SEAM = (
    "seam state (rev-14 finalize, 2026-08-14): the section 12.1 op "
    "registry is LIVE in the shipped five-crate composition (8 "
    "adjustments + 7 filters; O-TOOLS wiring + ratified ask-1 + tail "
    "flip); assembled_engine == the cli/cabi assembly. History: rev 13 "
    "encoded the pre-wiring unpopulated registry (defect-11 footnote, "
    "core merge 2026-08-13) and its pendingArm alarms tripped by design "
    "at this finalize")
PE_FIN = (
    "pendingArm ENTERED (rev-14 finalize, 2026-08-14): re-observed on "
    "the shipped five-crate composition (assembled_engine lifted to the "
    "cli/cabi wrap; every expectation below OBSERVED before encoding, "
    "rule 7)")
# 2026-08-17 strictness re-encode boilerplate (cited per re-encoded case).
PE_STRICT = ("2026-08-17 strictness re-encode (observe-first): the ruled "
             "schema-letter validation refuses the contract payload's "
             "dead-shaped nested params (sigmaPx/radiusPx/channels class) "
             "typed at stage 3, so the nested params are remapped onto the "
             "registry-owned spelling to keep this case's intended-stage "
             "premise REAL (the rev-14 premise-real procedure)")
PE_BG_LAYER = "b5366c8c-2bf6-50c2-92c6-0ea0c9509522"  # created by
# PE_PRIME_DOC (content-derived from the FIXED prime envelope; extracted
# from Engine::apply_bytes on the assembled engine 2026-08-13 and
# re-asserted by cp07_prime_kit_ids_are_real on every run)
PE_LID = "c07e0000-0000-4000-8000-00000000bbb1"  # primed raster layer
PE_SOL = "c07e0000-0000-4000-8000-00000000bbb2"  # primed smart layer
PE_ASSET = "c07e0000-0000-4000-8000-00000000ddd1"  # primed smart source
# rev-14 finalize additions (live-op premises; every id fixed and every
# prime observed accepting on the shipped five-crate composition
# 2026-08-14 before encoding -- rule 7):
PE_ADJL = "c07e0000-0000-4000-8000-00000000ccc1"  # primed adjustment layer
PE_OP = "c07e0000-0000-4000-8000-00000000ccc2"  # primed curves op (on ADJL)
PE_CHOP = "c07e0000-0000-4000-8000-00000000ccc3"  # primed chain op (on SOL)

PE_PRIME_DOC = {
    "schema": "appname.command.v1",
    "id": "c07e0000-0000-4000-8000-000000000d0c",
    "kind": "doc.new",
    "params": {"width": 64, "height": 64, "background": "white"},
}
PE_PRIME_LAYER = {
    "schema": "appname.command.v1",
    "id": "c07e0000-0000-4000-8000-00000000aaa1",
    "kind": "layer.add",
    "params": {"kind": "raster", "newLayerId": PE_LID, "name": "PE"},
}
PE_PRIME_FILL = {
    "schema": "appname.command.v1",
    "id": "c07e0000-0000-4000-8000-00000000aaa2",
    "kind": "fill",
    "target": {"layerId": PE_LID},
    "params": {"content": "foreground"},
}
PE_PRIME_SO = {
    "schema": "appname.command.v1",
    "id": "c07e0000-0000-4000-8000-00000000aaa3",
    "kind": "smartobject.convert",
    # QA-F7 ruling (accept-if-equal): the arbitrary newAssetId is DROPPED
    # -- absent means the content-derived id binds (SS7.4); the old literal
    # was never honored (the QA-F7 observation).
    "params": {"layerIds": [PE_LID], "newLayerId": PE_SOL},
}
PE_PRIME_LOCK_ALL_SO = {
    "schema": "appname.command.v1",
    "id": "c07e0000-0000-4000-8000-00000000aaa4",
    "kind": "layer.set_props",
    "target": {"layerId": PE_SOL},
    "params": {"locks": {"all": True}},
}
PE_PRIME_LOCK_PIX_L = {
    "schema": "appname.command.v1",
    "id": "c07e0000-0000-4000-8000-00000000aaa5",
    "kind": "layer.set_props",
    "target": {"layerId": PE_LID},
    "params": {"locks": {"pixels": True}},
}
PE_PRIME_ADJ = {
    # a LIVE curves op (rev-14 finalize): params in the REGISTRY-OWNED
    # schema spelling -- top-level point arrays (authoritative per the
    # filter.apply notes precedent; the contract example's `channels`
    # nesting is ignored-unknown live, O-TOOLS bridge pin 8 fold-in note)
    "schema": "appname.command.v1",
    "id": "c07e0000-0000-4000-8000-00000000aaa6",
    "kind": "adjustment.add",
    "params": {"opKind": "adjustment.curves", "newLayerId": PE_ADJL,
               "newOpId": PE_OP,
               "params": {"master": [[0.0, 0.0], [1.0, 1.0]]}},
}
PE_PRIME_LOCK_ALL_ADJ = {
    "schema": "appname.command.v1",
    "id": "c07e0000-0000-4000-8000-00000000aaa7",
    "kind": "layer.set_props",
    "target": {"layerId": PE_ADJL},
    "params": {"locks": {"all": True}},
}
PE_PRIME_CHAIN = {
    # a LIVE smart-filter chain op through the chain-insert door
    # (rev-14 finalize; the tail flip made this primable)
    "schema": "appname.command.v1",
    "id": "c07e0000-0000-4000-8000-00000000aaa8",
    "kind": "filter.smart_add",
    "target": {"layerId": PE_SOL},
    "params": {"filterId": "filter.gaussian_blur",
               "params": {"radius": 2.0}, "newOpId": PE_CHOP},
}
PE_PRIMES = {
    "doc": [PE_PRIME_DOC],
    "layer": [PE_PRIME_DOC, PE_PRIME_LAYER],
    "so": [PE_PRIME_DOC, PE_PRIME_LAYER, PE_PRIME_FILL, PE_PRIME_SO],
    "so-locked": [PE_PRIME_DOC, PE_PRIME_LAYER, PE_PRIME_FILL,
                  PE_PRIME_SO, PE_PRIME_LOCK_ALL_SO],
    "layer-locked": [PE_PRIME_DOC, PE_PRIME_LAYER, PE_PRIME_LOCK_PIX_L],
    "op": [PE_PRIME_DOC, PE_PRIME_ADJ],
    "op-locked": [PE_PRIME_DOC, PE_PRIME_ADJ, PE_PRIME_LOCK_ALL_ADJ],
    "so-chain": [PE_PRIME_DOC, PE_PRIME_LAYER, PE_PRIME_FILL,
                 PE_PRIME_SO, PE_PRIME_CHAIN],
}


def pe_pending(code, enters_with):
    return {"code": code, "entersWith": enters_with}


def pe_case(kind, idx, prime, expect, tier, why, pending=None, remap=None,
            notes=None):
    """One param-edge spec row; payload loads from the contract at emit."""
    return {"kind": kind, "idx": idx, "prime": prime, "expect": expect,
            "tier": tier, "why": why, "pending": pending,
            "remap": remap or [], "notes": notes}


# remap entries: (path-tuple-into-payload, value) -- premise-faithful id
# rewrites onto the deterministic prime kit, recorded per case.
# (a function, not a constant: reject()/accept() are defined further down)
def pe_specs():
    return [
        # -- adjustment.add ----------------------------------------------------
        pe_case("adjustment.add", 0, "doc",
                reject("error.schema.unknown_op_kind"), "contract",
                "contract expectError verbatim; stable across the O-TOOLS "
                "landing (an unregistered opKind stays unregistered)"),
        pe_case("adjustment.add", 1, "doc",
                reject("error.schema.out_of_range"), "contract",
                PE_FIN + "; adjustment.curves is registered and the nested "
                "params are remapped onto the REGISTRY-OWNED schema "
                "spelling (top-level point arrays, authoritative per the "
                "filter.apply notes precedent) so the single-point premise "
                "is REAL; observed: '2-16 points (section 17.2)'",
                remap=[(("params", "params"), {"master": [[0.0, 0.0]]})],
                notes="the contract example's `channels` nesting is "
                      "ignored-unknown under the registry-owned schema and "
                      "ACCEPTS as an identity curve (observed at the "
                      "finalize; O-TOOLS bridge pin 8, fold-in note) -- "
                      "the premise remap keeps this case a live "
                      "out-of-range probe"),
        pe_case("adjustment.add", 2, "doc",
                reject("error.schema.unknown_op_kind"), "plan",
                PE_SEAM + "; adjustment.color_lookup is NOT in the slice-1 "
                "section 17.2 registered set (8 adjustments) -- "
                "unknown_op_kind re-observed at the rev-14 finalize, so "
                "this arm stays correctly ARMED",
                pending=pe_pending("error.asset.lut_invalid",
                                   "adjustment.color_lookup registration "
                                   "(color-lookup wave)")),
        pe_case("adjustment.add", 3, "doc",
                reject("error.schema.duplicate_id"), "contract",
                PE_FIN + "; adjustment.curves is registered; " + PE_STRICT +
                ", and the section 9.6 creation-uuid collision fires on "
                "the remapped newLayerId; observed: 'newLayerId ... "
                "already exists (section 9.6)'",
                remap=[(("params", "newLayerId"), PE_BG_LAYER),
                       (("params", "params"),
                        {"master": [[0.0, 0.0], [1.0, 1.0]]})],
                notes="newLayerId remapped onto the prime doc's background "
                      "layer so the collision premise is REAL (the "
                      "contract's arena id resolves nowhere on the primed "
                      "doc); the rev-13/14 'ignored-unknown identity' "
                      "reading of the channels nesting is SUPERSEDED by "
                      "the strictness ruling"),
        # -- op.set_params -----------------------------------------------------
        pe_case("op.set_params", 0, "doc",
                reject("error.command.bad_target"), "contract",
                "contract expectError verbatim; params.opId does not resolve"),
        pe_case("op.set_params", 1, "doc",
                reject("error.schema.missing_field"), "contract",
                "contract expectError verbatim; required `params` missing "
                "(stage 5 precedes op resolution)"),
        pe_case("op.set_params", 2, "op",
                reject("error.schema.out_of_range"), "contract",
                PE_FIN + "; a LIVE curves op is primed (adjustment.add "
                "through the registry), opId remapped onto it, and the "
                "nested params remapped onto the registry-owned spelling "
                "with x=2.0 outside [0,1] so the out-of-range premise is "
                "REAL; observed: 'coordinates in [0,1]'",
                remap=[(("params", "opId"), PE_OP),
                       (("params", "params"),
                        {"master": [[0.0, 0.0], [2.0, 1.0]]})],
                notes="history: the rev-14 observation (the example's "
                      "`channels` nesting ACCEPTED as ignored-unknown "
                      "identity on the live op) is SUPERSEDED by the "
                      "2026-08-17 strictness ruling -- unknown nested "
                      "keys now refuse typed; this case's own remapped "
                      "payload is unaffected"),
        # -- op.set_mask -------------------------------------------------------
        pe_case("op.set_mask", 0, "doc",
                reject("error.schema.missing_field"), "contract",
                "contract expectError verbatim; oneOf(maskId|from) both-set "
                "-- the engine spells stage-3 oneOf violations missing_field "
                "exactly as the contract does (oneOf is NOT under the "
                "defect-8 ruling)"),
        pe_case("op.set_mask", 1, "op",
                reject("error.command.selection_required"), "contract",
                PE_FIN + "; a LIVE curves op is primed and opId remapped "
                "onto it; from:'selection' with no active selection makes "
                "the premise REAL; observed: \"op.set_mask "
                "from:'selection' requires an active selection (section "
                "40.6)\"",
                remap=[(("params", "opId"), PE_OP)]),
        pe_case("op.set_mask", 2, "doc",
                reject("error.command.bad_target"), "contract",
                "contract expectError verbatim"),
        # -- op.reorder --------------------------------------------------------
        pe_case("op.reorder", 0, "so-chain",
                reject("error.schema.out_of_range"), "contract",
                PE_FIN + "; QA-F12 CLOSED by the O-CORE door slice "
                "(rev 15): index beyond 0..len(chain)-1 answers the "
                "contract's out_of_range live-state bound (the former "
                "CLAMP borrowed the section 40.6 filter.smart_add INSERT "
                "precedent, which reorder has no domain for); observed: "
                "'`index` must lie within 0..0 (chain length 1), got "
                "9999 (section 40.6 live-state bound)'",
                remap=[(("params", "opId"), PE_CHOP)],
                notes="opId remapped onto the primed chain op so the "
                      "beyond-chain-end premise is REAL"),
        pe_case("op.reorder", 1, "doc",
                reject("error.schema.missing_field"), "contract",
                "contract expectError verbatim; required `index` missing"),
        pe_case("op.reorder", 2, "doc",
                reject("error.command.bad_target"), "contract",
                "contract expectError verbatim"),
        # -- op.toggle ---------------------------------------------------------
        pe_case("op.toggle", 0, "doc",
                reject("error.schema.out_of_range"), "plan",
                "RULED divergence codified: " + PE_RULING + "; observed: "
                "'`enabled` must be a boolean, got string'"),
        pe_case("op.toggle", 1, "doc",
                reject("error.command.bad_target"), "contract",
                "contract expectError verbatim"),
        pe_case("op.toggle", 2, "op-locked",
                reject("error.command.locked"), "contract",
                PE_FIN + "; a LIVE curves op is primed under an ALL-locked "
                "owner and opId remapped onto it, so the lock premise is "
                "REAL; observed: 'ALL lock along the ancestry blocks "
                "(section 11.1 transitive rule)'",
                remap=[(("params", "opId"), PE_OP)]),
        # -- op.delete ---------------------------------------------------------
        pe_case("op.delete", 0, "doc",
                reject("error.schema.missing_field"), "contract",
                "contract expectError verbatim; required `opId` missing"),
        pe_case("op.delete", 1, "doc",
                reject("error.command.bad_target"), "contract",
                "contract expectError verbatim"),
        pe_case("op.delete", 2, "op-locked",
                reject("error.command.locked"), "contract",
                PE_FIN + "; a LIVE curves op is primed under an ALL-locked "
                "owner and opId remapped onto it, so the lock premise is "
                "REAL; observed: 'ALL lock along the ancestry blocks "
                "(section 11.1 transitive rule)'",
                remap=[(("params", "opId"), PE_OP)]),
        # -- smartobject.convert ----------------------------------------------
        pe_case("smartobject.convert", 0, "doc",
                reject("error.schema.out_of_range"), "contract",
                "QA-F4 CLOSED by the O-CORE door slice (rev 15): the "
                "stage-5 validator grew minItems/maxItems, so "
                "layerIds:[] answers out_of_range BEFORE the arm's v1 "
                "single-source not_implemented gate (the contract's "
                "stage order); observed: '`layerIds` must have at least "
                "1 item(s), got 0'"),
        pe_case("smartobject.convert", 1, "doc",
                reject("error.command.not_implemented"), "plan",
                "the v1 single-source gate answers multi-source lists before "
                "parent resolution; the contract's shared-parent premise "
                "(bad_target) re-enters with the multi-layer convert row",
                pending=pe_pending("error.command.bad_target",
                                   "multi-layer convert (compositor flatten "
                                   "row)")),
        pe_case("smartobject.convert", 2, "doc",
                reject("error.command.bad_target"), "plan",
                "cycle premise unreachable at v1 (nested convert gates "
                "target_kind_mismatch; linked sources gated); the contract's "
                "example layer resolves nowhere -> bad_target observed",
                pending=pe_pending("error.smartobject.cycle",
                                   "nested-composite render row")),
        pe_case("smartobject.convert", 3, "doc",
                reject("error.command.bad_target"), "plan",
                "depth premise unreachable at v1 (nesting gated); the "
                "contract's example layer resolves nowhere -> bad_target "
                "observed",
                pending=pe_pending("error.smartobject.too_deep",
                                   "nested-composite render row")),
        # -- smartobject.edit_contents ----------------------------------------
        pe_case("smartobject.edit_contents", 0, "so",
                reject("error.schema.missing_field"), "contract",
                "contract expectError verbatim; required `assetAfter` "
                "missing (stage 5 precedes target resolution)"),
        pe_case("smartobject.edit_contents", 1, "so",
                reject("error.command.bad_target"), "contract",
                "contract expectError verbatim; observed detail: "
                "'assetBefore ... does not match the target's current "
                "source' -- the stale-session premise is REAL on the primed "
                "smart object",
                remap=[(("target", "layerId"), PE_SOL)],
                notes="target remapped onto the primed smart layer so the "
                      "assetBefore mismatch (not target resolution) is what "
                      "fires"),
        pe_case("smartobject.edit_contents", 2, "so",
                reject("error.command.bad_target"), "contract",
                "contract expectError verbatim; target.layerId does not "
                "resolve"),
        # -- smartobject.replace ----------------------------------------------
        pe_case("smartobject.replace", 0, "so",
                reject("error.schema.missing_field"), "contract",
                "QA-F5 CLOSED by the O-CORE door slice (rev 15): "
                "property-level oneOf-of-required now runs at stage 5, "
                "so source both-set answers missing_field BEFORE the "
                "linked-source v1 gate (which still owns the "
                "single-member path form, re-observed unchanged)",
                remap=[(("target", "layerId"), PE_SOL)]),
        pe_case("smartobject.replace", 1, "so",
                reject("error.command.not_implemented"), "plan",
                "the linked-source v1 gate fires on path/relinkOnly before "
                "the embedded-target check",
                pending=pe_pending("error.command.target_kind_mismatch",
                                   "linked-source arm (section 11.5 "
                                   "drift-detection row)"),
                remap=[(("target", "layerId"), PE_SOL)]),
        pe_case("smartobject.replace", 2, "so",
                reject("error.command.not_implemented"), "plan",
                "the linked-source v1 gate fires on `path` before file "
                "existence",
                pending=pe_pending("error.io.file_not_found",
                                   "linked-source arm (section 11.5 "
                                   "drift-detection row)"),
                remap=[(("target", "layerId"), PE_SOL)]),
        pe_case("smartobject.replace", 3, "so",
                reject("error.command.bad_target"), "plan",
                "cycle premise unreachable at v1 (nesting gated); the "
                "contract's source asset resolves to no ingested raster "
                "asset -> bad_target observed",
                pending=pe_pending("error.smartobject.cycle",
                                   "nested-composite render row"),
                remap=[(("target", "layerId"), PE_SOL)]),
        # -- smartobject.rasterize --------------------------------------------
        pe_case("smartobject.rasterize", 0, "so",
                reject("error.schema.out_of_range"), "plan",
                "RULED divergence codified: " + PE_RULING + "; observed: "
                "'unknown parameter `what` (additionalProperties: false)'",
                remap=[(("target", "layerId"), PE_SOL)]),
        pe_case("smartobject.rasterize", 1, "layer",
                reject("error.command.target_kind_mismatch"), "contract",
                "contract expectError verbatim; observed: 'the target is "
                "not a smart object'",
                remap=[(("target", "layerId"), PE_LID)],
                notes="prime stops before the convert so the target stays a "
                      "PLAIN raster layer -- the mismatch premise is real"),
        pe_case("smartobject.rasterize", 2, "so-locked",
                reject("error.command.locked"), "contract",
                "contract expectError verbatim; observed: 'ALL lock along "
                "the ancestry blocks (section 11.1 transitive rule)' -- the "
                "lock premise is real (prime sets locks.all on the smart "
                "layer)",
                remap=[(("target", "layerId"), PE_SOL)]),
        # -- layer.rasterize ---------------------------------------------------
        pe_case("layer.rasterize", 0, "layer",
                reject("error.schema.out_of_range"), "contract",
                "contract expectError verbatim; `what` outside the enum",
                remap=[(("target", "layerId"), PE_LID)]),
        pe_case("layer.rasterize", 1, "layer",
                reject("error.command.target_kind_mismatch"), "contract",
                "QA-F3 CLOSED by the O-CORE door slice (rev 15): "
                "what:'effects' on an already-raster layer with no "
                "effects answers the contract's target_kind_mismatch "
                "row (effects are not in the engine, so a raster target "
                "can never carry them); what:'content' keeps the "
                "documented no-op push",
                remap=[(("target", "layerId"), PE_LID)]),
        pe_case("layer.rasterize", 2, "layer-locked",
                reject("error.command.locked"), "contract",
                "contract expectError verbatim; the PIXELS-lock premise is "
                "real (prime sets locks.pixels)",
                remap=[(("target", "layerId"), PE_LID)]),
        # -- history.snapshot --------------------------------------------------
        pe_case("history.snapshot", 0, "doc",
                reject("error.schema.out_of_range"), "contract",
                "QA-F2 CLOSED by the O-CORE door slice (rev 15): the "
                "stage-5 validator grew the contract length rows "
                "(minLength/maxLength); name:'' answers out_of_range; "
                "observed: '`name` must be at least 1 character(s) "
                "long'"),
        pe_case("history.snapshot", 1, "doc",
                reject("error.schema.out_of_range"), "contract",
                "contract expectError verbatim; scope outside the enum"),
        pe_case("history.snapshot", 2, "doc",
                reject("error.schema.out_of_range"), "plan",
                "RULED divergence codified: " + PE_RULING + "; observed: "
                "'unknown parameter `persist` (additionalProperties: "
                "false)'"),
        # -- history.goto ------------------------------------------------------
        pe_case("history.goto", 0, "doc",
                reject("error.schema.missing_field"), "contract",
                "contract expectError verbatim; oneOf(position|snapshot) "
                "zero-given -- engine spells stage-3 oneOf missing_field as "
                "the contract does"),
        pe_case("history.goto", 1, "doc",
                reject("error.schema.missing_field"), "contract",
                "contract expectError verbatim; oneOf both-given"),
        pe_case("history.goto", 2, "doc",
                reject("error.schema.out_of_range"), "contract",
                "contract expectError verbatim; position < 0"),
        pe_case("history.goto", 3, "doc",
                reject("error.command.bad_target"), "contract",
                "contract expectError verbatim; observed: 'no snapshot "
                "named ... (section 40.8)'"),
        # -- filter.apply (live since the wiring slice; rev-14 finalize) -------
        pe_case("filter.apply", 0, "doc",
                reject("error.schema.unknown_op_kind"), "contract",
                PE_FIN + "; the section 16.1 filter registry is live; "
                "observed: 'filterId \"filter.does_not_exist\" is not "
                "registered in the section 16.1 filter registry'"),
        pe_case("filter.apply", 1, "so",
                reject("error.command.target_kind_mismatch"), "contract",
                PE_FIN + "; target remapped onto the primed SMART OBJECT "
                "layer so the non-raster premise is REAL (the contract's "
                "'parametric-target' id resolves nowhere; the section "
                "12.4 gate answers every non-raster kind); " + PE_STRICT +
                "; observed: 'the target MUST be a Raster layer -- "
                "rasterize first (section 12.4 ...)'",
                remap=[(("target", "layerId"), PE_SOL),
                       (("params", "params"), {"radius": 4.0})]),
        pe_case("filter.apply", 2, "doc",
                reject("error.schema.out_of_range"), "contract",
                PE_FIN + "; nested params remapped onto the registry-owned "
                "spelling (radius:-3) so the out-of-range premise is REAL; "
                "stage-3 nested validation fires BEFORE stage-6 target "
                "resolution (the unresolved example target is never "
                "reached); observed: '-3 outside 0.1..=1000 (section "
                "16.3)'",
                remap=[(("params", "params"), {"radius": -3.0})],
                notes="history: the rev-14 verbatim-payload observation "
                      "(sigmaPx ignored-unknown; the filter EXECUTED with "
                      "the default radius on a real raster target) is "
                      "SUPERSEDED by the 2026-08-17 strictness ruling -- "
                      "the verbatim example now refuses typed at stage 3; "
                      "the sigmaPx divergence stays fold-in registered"),
        # -- filter.smart_add (live end-to-end since the tail flip) ------------
        pe_case("filter.smart_add", 0, "layer",
                reject("error.command.target_kind_mismatch"), "contract",
                PE_FIN + "; target remapped onto the primed PLAIN raster "
                "layer so the not-a-smart-object premise is REAL; "
                + PE_STRICT + "; observed: 'the target MUST be a "
                "SmartObject -- run smartobject.convert first (section "
                "40.6)'",
                remap=[(("target", "layerId"), PE_LID),
                       (("params", "params"), {"radius": 4.0})]),
        pe_case("filter.smart_add", 1, "doc",
                reject("error.schema.unknown_op_kind"), "contract",
                PE_FIN + "; the section 16.1 filter registry is live; "
                "observed: 'filterId \"filter.does_not_exist\" is not "
                "registered in the section 16.1 filter registry'"),
        pe_case("filter.smart_add", 2, "so-chain",
                reject("error.schema.duplicate_id"), "contract",
                PE_FIN + "; a LIVE chain op is primed (filter.smart_add "
                "through the chain-insert door) and newOpId remapped onto "
                "its id so the section 9.6 collision premise is REAL; "
                "target remapped onto the primed smart layer; observed: "
                "'newOpId ... already exists (section 9.6)'",
                remap=[(("target", "layerId"), PE_SOL),
                       (("params", "newOpId"), PE_CHOP),
                       (("params", "params"), {"radius": 4.0})],
                notes="cross-arena probe recorded as finding QA-F11: a "
                      "newOpId equal to an existing LAYER id ACCEPTS "
                      "(uniqueness is enforced per-arena only; observed "
                      "on both filter.smart_add and adjustment.add) -- "
                      "needs an O-CORE section 9.6 ruling"),
    ]


def pe_set_path(obj, path, value):
    cur = obj
    for key in path[:-1]:
        cur = cur[key]
    cur[path[-1]] = value


def cp07_param_edges(repo, kinds):
    """Emit the PE-case lists, keyed by kind (loaded from the contracts;
    payloads verbatim except the recorded premise remaps)."""
    out = {}
    contract_cache = {}
    for spec in pe_specs():
        kind = spec["kind"]
        if kind not in contract_cache:
            contract_cache[kind] = yaml.safe_load(
                open(os.path.join(repo, "contracts", "commands",
                                  kinds[kind]["file"]), encoding="utf-8"))
        inv = contract_cache[kind]["examples"]["invalid"][spec["idx"]]
        payload = json.loads(json.dumps(inv["payload"]))
        for path, value in spec["remap"]:
            pe_set_path(payload, list(path), value)
        provenance = (
            "contracts/commands/%s examples.invalid[%d] ('%s', "
            "expectError %s)%s; %s" % (
                kinds[kind]["file"], spec["idx"], inv["case"],
                inv["expectError"],
                "" if not spec["remap"] else
                " with premise remaps onto the deterministic prime kit "
                "(recorded in `remaps`)",
                spec["why"]))
        case = {
            "caseId": "PE-%s-%d" % (kind, spec["idx"]),
            "title": "%s: %s (param edge)" % (kind, inv["case"]),
            "driver": "apply",
            "envelopeSequence": PE_PRIMES[spec["prime"]] + [payload],
            "cp07Expect": spec["expect"],
            "codeTier": spec["tier"],
            "provenance": provenance,
            "pendingArm": spec["pending"],
            "remaps": [{"path": "/".join(p), "value": v}
                       for p, v in spec["remap"]],
            "notes": spec["notes"],
        }
        out.setdefault(kind, []).append(case)
    return out


# Ported A-case assembled overlays: the proto-shaped stroke params are not
# the v5.5 contract surface, so on ASSEMBLED engines stage 5 fires before
# any stroke semantics: brush.stroke requires [brushPreset, color] in
# schema order and the proto params carry neither brushPreset nor a v5.5
# color object -> error.schema.missing_field (field=brushPreset first).
CP03_ASSEMBLED_PORTED = {
    "A11": "brush.stroke", "A14": "brush.stroke",
    "A19": "brush.stroke", "A23": "brush.stroke",
}
CP03_ASSEMBLED_PORTED_NOTE = (
    "assembled stage 5: v5.5 brush.stroke paramsSchema requires "
    "[brushPreset, color] (schema order) and the proto-shaped params lack "
    "brushPreset -> missing_field; the proto param surface diverges from "
    "the v5.5 contract, so the proto expectation does not resume on "
    "assembled engines (A23's accept included); " + CP03_STAGE_MAP
)
PLAN_PROVENANCE = (
    "plan AGENT-ASSIGNMENTS-full-build.md CP-0.1 registry rule: all 102 kinds "
    "registered from CP-0.1; unimplemented kinds return "
    "error.command.not_implemented"
)
ORDERING_NOTE = (
    "validation-order at CP-0.1 (param/target checks vs the not_implemented "
    "dispatch check) is not yet pinned by O-CORE; see OQ-3 in "
    "reports/O-QA-log.md"
)
# OQ-3 is CLOSED: the validation order is pinned (reports/O-CORE-log.md,
# "Validation order (pinned; binding arbitration OQ-3)"): stage 0 API
# boundary -> 1 envelope schema -> 2 duplicate id (before kind) -> 3 kind
# known -> 4 implemented -> 5 params -> 6 target/live-state -> 7 execute.
# Stage 4 therefore precedes stages 5 and 6: every former ni-or-X hedge
# collapses to error.command.not_implemented while the kind is
# unimplemented; the X arm re-enters with the kind's implementing wave.
STAGE4_PIN = (
    "hedge collapsed (wave 4): pinned validation order, reports/"
    "O-CORE-log.md (binding OQ-3 arbitration) — stage 4 implemented-check "
    "precedes stage 5 params and stage 6 target/live-state; reproduced on "
    "the merged engine 2026-08-11 (corpus run 165 PASS / 0 FAIL / 1 SKIP)"
)

# Observed codes for proto cases declared `code=*`, transcribed from
# appname-proto/REPORT-A.md section 4.2 (mechanical transcription).
PROTO_OBSERVED = {
    "A07": "error.schema.out_of_range",
    "A11": "error.schema.out_of_range",
    "A14": "error.schema.out_of_range",
    "A16": "error.schema.out_of_range",
    "A17": "error.schema.out_of_range",
    "A20": "error.command.bad_target",
    "D01": "error.api.invalid_arg",
    "D02": "error.api.invalid_arg",
    "D09": "error.io.read_failed",
}


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        h.update(f.read())
    return h.hexdigest()


def dump(path, obj):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        json.dump(obj, f, indent=2, sort_keys=True, ensure_ascii=True)
        f.write("\n")


def reject(code, exit_=1, any_of=None):
    return {
        "outcome": "reject",
        "exit": exit_,
        "exitAnyOf": None,
        "code": code,
        "codeAnyOf": any_of,
    }


def accept(exit_=0):
    return {
        "outcome": "accept",
        "exit": exit_,
        "exitAnyOf": None,
        "code": None,
        "codeAnyOf": None,
    }


# --------------------------------------------------------------------------
# ported/envelope.json — the proto's 29 JSONL envelope cases, verbatim
# envelopes, proto expectations preserved, CP-0.1 expectations mapped.
# --------------------------------------------------------------------------

# Per-case CP-0.1 mapping. Rationale tokens:
#   env-stage   envelope parse/validate is ported exactly from proto (plan
#               section 2.1 O-CORE row), so envelope-stage outcomes carry over
#   ni          kind is registered but unimplemented at CP-0.1 -> plan rule
#   ni-or-X     both a validation violation and the unimplemented dispatch
#               check apply; order unspecified -> codeAnyOf (OQ-3)
#   docnew      doc.new is implemented at CP-0.1; contract governs
A_CP01 = {
    "A01": (reject("error.command.not_implemented"), "plan",
            "ni: layer.add is registered but unimplemented at CP-0.1; the "
            "proto accept expectation resumes when layer.add lands"),
    "A02": (reject("error.command.not_implemented"), "plan",
            STAGE4_PIN + "; the out_of_range arm re-enters when "
            "filter.apply lands"),
    "A03": (reject("error.command.not_implemented"), "plan",
            STAGE4_PIN + "; the out_of_range arm re-enters when "
            "filter.apply lands"),
    "A04": (reject("error.command.not_implemented"), "plan",
            STAGE4_PIN + "; the out_of_range arm re-enters when "
            "filter.apply lands"),
    "A05": (reject("error.command.not_implemented"), "plan",
            STAGE4_PIN + "; the out_of_range arm re-enters when "
            "layer.set_props lands"),
    "A06": (reject("error.command.unknown_kind"), "proto",
            "registry miss: layer.bogus_kind is not one of the 102 kinds; "
            "proto code error.command.unknown_kind is a registered v5.5 id "
            "(the error.schema.unknown_command_kind twin is OQ-2)"),
    "A07": (reject("error.schema.out_of_range"), "proto",
            "env-stage: malformed envelope uuid, proto observed code"),
    "A08": (reject("error.schema.wrong_schema"), "proto",
            "env-stage: proto code; error.schema.wrong_schema is NOT in the "
            "v5.5 MessageId registry (OQ-2)"),
    "A09": (reject("error.command.not_implemented"), "plan",
            "ni: the duplicate-id premise is void at CP-0.1 because A01 "
            "(layer.add) is rejected and never journaled; selection.all then "
            "hits the unimplemented dispatch check; proto duplicate_id "
            "expectation resumes when layer.add lands"),
    "A10": (reject("error.command.not_implemented"), "plan",
            STAGE4_PIN + "; the out_of_range arm re-enters when doc.crop "
            "lands"),
    "A11": (reject("error.command.not_implemented"), "plan",
            STAGE4_PIN + "; the out_of_range arm re-enters when "
            "brush.stroke lands"),
    "A12": (reject("error.command.not_implemented"), "plan",
            STAGE4_PIN + "; the unknown_op_kind arm re-enters when "
            "filter.apply lands"),
    "A13": (reject("error.command.not_implemented"), "plan",
            STAGE4_PIN + "; the out_of_range arm re-enters when "
            "selection.set lands"),
    "A14": (reject("error.command.not_implemented"), "plan",
            STAGE4_PIN + "; per the slice-2 blocker-1 fix, stage 1 "
            "validates samples as string-or-null and CARRIES it — dispatch "
            "answers; implemented stroke consumers may refuse unsupported "
            "samples later (that arm re-enters with brush.stroke)"),
    "A15": (reject("error.command.not_implemented"), "plan",
            STAGE4_PIN + "; the bad_target arm re-enters when filter.apply "
            "lands (stage 6 target resolution)"),
    "A16": (reject("error.schema.out_of_range"), "proto",
            "env-stage: unknown top-level envelope key, proto observed code"),
    "A17": (reject("error.command.not_implemented"), "plan",
            STAGE4_PIN + "; the exclusivity arm (proto observed "
            "out_of_range; v5.5 registers error.schema.exclusive_params, "
            "OQ-2) re-enters when filter.apply lands"),
    "A18": (reject("error.command.not_implemented"), "plan",
            STAGE4_PIN + "; the missing_field arm re-enters when "
            "filter.apply lands"),
    "A19": (reject("error.command.not_implemented"), "plan",
            STAGE4_PIN + "; the out_of_range arm re-enters when "
            "brush.stroke lands"),
    "A20": (reject("error.command.bad_target"), "contract",
            "docnew: doc.new IS implemented at CP-0.1; "
            "contracts/commands/doc.new.yaml examples.invalid "
            "already-initialized pins error.command.bad_target"),
}
for _n in range(21, 30):
    A_CP01["A%02d" % _n] = (
        reject("error.command.not_implemented"), "plan",
        "ni: kind is registered but unimplemented at CP-0.1; the proto "
        "accept expectation resumes when the kind lands")


def load_ported_envelope(proto_dir, kinds):
    src = os.path.join(proto_dir, "scripts", "adversarial.jsonl")
    cases = []
    pending = None
    with open(src, encoding="utf-8") as f:
        for raw in f:
            raw = raw.strip()
            m = CASE_RE.match(raw)
            if m:
                pending = (m.group(1), m.group(2), m.group(3), m.group(4))
                continue
            if not raw or raw.startswith("#") or pending is None:
                continue
            cid, exp_exit, exp_code, desc = pending
            pending = None
            env = json.loads(raw)
            proto_expect = {
                "declared": "exit=%s code=%s" % (exp_exit, exp_code),
                "exit": int(exp_exit),
                "exitAnyOf": None,
                "outcome": "accept" if exp_code == "-" else "reject",
                "code": None if exp_code in ("-", "*") else exp_code,
                "observedCode": PROTO_OBSERVED.get(cid),
            }
            cp01, tier, note = A_CP01[cid]
            kind_known = env.get("kind") in kinds
            cp03_assembled = None
            if cid in CP03_ASSEMBLED_PORTED:
                cp03_assembled = reject("error.schema.missing_field")
                note = note + ". cp03Assembled: " + CP03_ASSEMBLED_PORTED_NOTE
            cases.append({
                "cp03Assembled": cp03_assembled,
                "caseId": "P-" + cid,
                "title": desc,
                "driver": "apply",
                "envelope": env,
                "protoExpect": proto_expect,
                "cp01Expect": cp01,
                "codeTier": tier,
                "cp01Runnable": True,
                "kindInV55Registry": kind_known,
                "provenance": "appname-proto/scripts/adversarial.jsonl CASE "
                              + cid + " (verbatim envelope; observed codes "
                              "from REPORT-A section 4.2)",
                "notes": note,
            })
    if len(cases) != 29:
        raise SystemExit("expected 29 JSONL cases, found %d" % len(cases))
    return {
        "suite": "ported-envelope",
        "source": "appname-proto/scripts/adversarial.jsonl (29 envelope "
                  "cases of the proven 41-case suite; REPORT-A section 4.2: "
                  "41/41 PASS, zero panics, two independent binaries)",
        "note": "envelopes verbatim (JSON key order normalized by "
                "sort_keys); the proto JSONL remains the byte-level "
                "authority. P-A01/P-A09 share an envelope id BY DESIGN "
                "(duplicate-id case).",
        "cases": cases,
    }


# --------------------------------------------------------------------------
# ported/cli.json — the proto driver's 12 CLI-level cases (D01..D12),
# transcribed mechanically from appname-proto/scripts/adversarial.py and
# REPORT-A section 4.2. argv uses the CLI verb surface the plan fixes:
# new/apply/script/undo/redo/history/export/import/info/selftest, exit 0/1/2,
# --json. Placeholders: <DOC> scratch package, <WORK> temp dir,
# <LAYER_ID_FROM_NEW> parsed from `new --json`, <I18N_DIR> catalog dir.
# --------------------------------------------------------------------------

D_TABLE = [
    {
        "caseId": "P-D01",
        "title": "apply a non-JSON command string",
        "cli": {"argv": ["apply", "--doc", "<DOC>", "--command",
                         "this is not json", "--json"]},
        "asserts": ["exit", "json-shape", "no-panic", "structured",
                    "pkg-untouched"],
        "protoExpect": {"declared": "exit=1 code=*", "exit": 1,
                        "exitAnyOf": None, "outcome": "reject", "code": None,
                        "observedCode": PROTO_OBSERVED["D01"]},
        "cp01Expect": reject("error.api.invalid_arg"),
        "codeTier": "proto",
        "cp01Runnable": True,
        "notes": "code pinned (wave 4): stage 0 API boundary of the pinned "
                 "validation order (JSON must parse) emits the "
                 "error.api.invalid_arg wire code (engine ErrorCode); the "
                 "id remains outside the v5.5 registry (OQ-2)",
    },
    {
        "caseId": "P-D02",
        "title": "envelope document over the 1 MiB cap (via --file)",
        "cli": {"argv": ["apply", "--doc", "<DOC>", "--file",
                         "<WORK>/big.json", "--json"],
                "files": {"big.json": {
                    "synthesize": "selection.all envelope, id "
                    "ad00d002-0000-4000-8000-00000000d002, params.pad = "
                    "'x' * 1228800 (1200 KiB, over the 1 MiB cap)"}}},
        "asserts": ["exit", "json-shape", "no-panic", "structured",
                    "pkg-untouched"],
        "protoExpect": {"declared": "exit=1 code=*", "exit": 1,
                        "exitAnyOf": None, "outcome": "reject", "code": None,
                        "observedCode": PROTO_OBSERVED["D02"]},
        "cp01Expect": reject("error.api.invalid_arg"),
        "codeTier": "proto",
        "cp01Runnable": True,
        "notes": "code pinned (wave 4): the 1 MiB cap is stage 0 of the "
                 "pinned validation order (section 9.6) -> "
                 "error.api.invalid_arg; registry gap as D01 (OQ-2)",
    },
    {
        "caseId": "P-D03",
        "title": "undo --steps 999 clamps to journal start",
        "cli": {"argv": ["undo", "--doc", "<DOC>", "--steps", "999",
                         "--json"]},
        "asserts": ["exit", "json-shape", "no-panic",
                    "clamped: performed == cursor-1 and cursor == 1"],
        "protoExpect": {"declared": "exit=0 code=-", "exit": 0,
                        "exitAnyOf": None, "outcome": "accept", "code": None,
                        "observedCode": None},
        "cp01Expect": None,
        "codeTier": "proto",
        "cp01Runnable": False,
        "notes": "setup premise (journal populated by the A-series) is void "
                 "at CP-0.1 where only doc.new lands; clamp semantics "
                 "re-enter at CP-0.3 with stroke kinds",
    },
    {
        "caseId": "P-D04",
        "title": "undo past start",
        "cli": {"argv": ["undo", "--doc", "<DOC>", "--json"]},
        "asserts": ["exit", "json-shape", "no-panic", "structured", "code"],
        "protoExpect": {"declared": "exit=1 code=error.history."
                        "nothing_to_undo", "exit": 1, "exitAnyOf": None,
                        "outcome": "reject",
                        "code": "error.history.nothing_to_undo",
                        "observedCode": None},
        "cp01Expect": None,
        "codeTier": "proto",
        "cp01Runnable": False,
        "notes": "error.history.* ids are not in the v5.5 registry (OQ-2) "
                 "though the engine emits them (ErrorCode wire strings); "
                 "the proto premise (clamp-walk to start, then undo) needs "
                 "kinds beyond doc.new — the degenerate fresh-doc analogue "
                 "(cursor<=1 -> nothing_to_undo) is asserted at engine "
                 "level by prop_journal_cursor_invariants",
    },
    {
        "caseId": "P-D05",
        "title": "redo --steps 999 clamps to journal end",
        "cli": {"argv": ["redo", "--doc", "<DOC>", "--steps", "999",
                         "--json"]},
        "asserts": ["exit", "json-shape", "no-panic",
                    "clamped: cursor == journal length"],
        "protoExpect": {"declared": "exit=0 code=-", "exit": 0,
                        "exitAnyOf": None, "outcome": "accept", "code": None,
                        "observedCode": None},
        "cp01Expect": None,
        "codeTier": "proto",
        "cp01Runnable": False,
        "notes": "same premise gap as P-D03",
    },
    {
        "caseId": "P-D06",
        "title": "redo past end",
        "cli": {"argv": ["redo", "--doc", "<DOC>", "--json"]},
        "asserts": ["exit", "json-shape", "no-panic", "structured", "code"],
        "protoExpect": {"declared": "exit=1 code=error.history."
                        "nothing_to_redo", "exit": 1, "exitAnyOf": None,
                        "outcome": "reject",
                        "code": "error.history.nothing_to_redo",
                        "observedCode": None},
        "cp01Expect": None,
        "codeTier": "proto",
        "cp01Runnable": False,
        "notes": "same registry gap and degenerate-analogue coverage as "
                 "P-D04 (prop_journal_cursor_invariants)",
    },
    {
        "caseId": "P-D07",
        "title": "script is transactional: failing line persists nothing",
        "cli": {"argv": ["script", "--doc", "<DOC>", "--file",
                         "<WORK>/bad.jsonl", "--json"],
                "files": {"bad.jsonl": {
                    "synthesize": "the SAME selection.all envelope twice, id "
                    "ad00d701-0000-4000-8000-00000000d701 on both lines"}}},
        "asserts": ["exit", "json-shape", "no-panic", "structured", "code",
                    "line-quoted", "pkg-untouched"],
        "protoExpect": {"declared": "exit=1 code=error.schema.duplicate_id "
                        "line=2", "exit": 1, "exitAnyOf": None,
                        "outcome": "reject",
                        "code": "error.schema.duplicate_id",
                        "observedCode": None},
        "cp01Expect": reject("error.command.not_implemented"),
        "codeTier": "plan",
        "cp01Runnable": True,
        "notes": "at CP-0.1 line 1 (selection.all) already fails with "
                 "not_implemented, so expect line=1 and that code; the "
                 "transactionality assertion (package bytes untouched on "
                 "failure) holds at every checkpoint; proto duplicate_id@"
                 "line=2 resumes when selection.all lands",
    },
    {
        "caseId": "P-D08",
        "title": "info on a missing package (IO error class)",
        "cli": {"argv": ["info", "--doc", "<WORK>/missing.appname",
                         "--json"], "needsDoc": False},
        "asserts": ["exit", "json-shape", "no-panic", "structured", "code"],
        "protoExpect": {"declared": "exit=2 code=error.io.read_failed",
                        "exit": 2, "exitAnyOf": None, "outcome": "reject",
                        "code": "error.io.read_failed",
                        "observedCode": None},
        "cp01Expect": reject("error.io.read_failed", exit_=2),
        "codeTier": "contract",
        "cp01Runnable": True,
        "notes": "error.io.read_failed is a registered v5.5 id; exit 2 is "
                 "the fixed IO/environment class",
    },
    {
        "caseId": "P-D09",
        "title": "import a non-image file",
        "cli": {"argv": ["import", "--doc", "<DOC>", "--image",
                         "<WORK>/not_an_image.txt", "--json"],
                "files": {"not_an_image.txt": {
                    "synthesize": "plain text, no image magic bytes"}}},
        "asserts": ["exit", "json-shape", "no-panic", "structured",
                    "pkg-untouched"],
        "protoExpect": {"declared": "exit=1|2 code=*", "exit": None,
                        "exitAnyOf": [1, 2], "outcome": "reject",
                        "code": None,
                        "observedCode": PROTO_OBSERVED["D09"]},
        "cp01Expect": {"outcome": "reject", "exit": None,
                       "exitAnyOf": [1, 2], "code": None, "codeAnyOf": None},
        "codeTier": "proto",
        "cp01Runnable": True,
        "notes": "structured rejection either as command rejection (1) or "
                 "IO class (2); proto observed exit 2 error.io.read_failed. "
                 "The import verb is LIVE at CP-0.1 (an IO call outside the "
                 "journal); the decode-failure class the runner observes is "
                 "recorded in the lane log, not over-pinned here",
    },
    {
        "caseId": "P-D10",
        "title": "layer.delete on the only layer of a fresh doc",
        "cli": {"steps": [
            {"argv": ["new", "--width", "16", "--height", "16", "--out",
                      "<WORK>/single.appname", "--json"],
             "capture": {"LAYER_ID_FROM_NEW": "receipt.createdIds[-1]"}},
            {"argv": ["apply", "--doc", "<WORK>/single.appname",
                      "--command", "<ENVELOPE>", "--json"],
             "envelope": {"schema": "appname.command.v1",
                          "id": "ad00d100-0000-4000-8000-00000000d100",
                          "kind": "layer.delete",
                          "params": {"layerId": "<LAYER_ID_FROM_NEW>"}}}]},
        "asserts": ["exit", "json-shape", "no-panic", "structured", "code",
                    "id-quoted"],
        "protoExpect": {"declared": "exit=1 code=error.command.bad_target",
                        "exit": 1, "exitAnyOf": None, "outcome": "reject",
                        "code": "error.command.bad_target",
                        "observedCode": None},
        "cp01Expect": reject("error.schema.out_of_range"),
        "codeTier": "plan",
        "cp01Runnable": True,
        "notes": STAGE4_PIN + "; RE-ENTERED at O-CORE CP-0.5 wave 4 (rev "
                 "10): layer.delete is live, and the PROTO-SHAPED "
                 "envelope (params.layerId) diverges from the v5.5 "
                 "contract (target.layerId; layer.delete takes NO params, "
                 "additionalProperties:false) -> the stage-5 extra-param "
                 "reject fires before any live-state check (the stroke "
                 "A-case precedent). The contract-shaped last-layer "
                 "delete ACCEPTS (empty tree legal, section 40.7). "
                 "CLI note: `new --json` reports the background layer as "
                 "receipt.createdIds[-1] (ApplyReceipt contract: [root "
                 "group, background layer])",
    },
    {
        "caseId": "P-D11",
        "title": "png export with --quality (CLI arg validation)",
        "cli": {"argv": ["export", "--doc", "<DOC>", "--format", "png",
                         "--quality", "50", "--out", "<WORK>/x.png",
                         "--json"]},
        "asserts": ["exit", "json-shape", "no-panic", "structured"],
        "protoExpect": {"declared": "exit=1 code=error.api.invalid_arg",
                        "exit": 1, "exitAnyOf": None, "outcome": "reject",
                        "code": "error.api.invalid_arg",
                        "observedCode": None},
        "cp01Expect": reject("error.api.invalid_arg"),
        "codeTier": "proto",
        "cp01Runnable": True,
        "notes": "code pinned (wave 4): CLI v1 rejects --quality with "
                 "--format png as error.api.invalid_arg before any engine "
                 "call (crates/appname-cli, observed 2026-08-11); registry "
                 "gap as D01 (OQ-2)",
    },
    {
        "caseId": "P-D12",
        "title": "human-mode --lang es rejection: one localized stderr line",
        "cli": {"argv": ["apply", "--doc", "<DOC>", "--command",
                         "<ENVELOPE>", "--lang", "es"],
                "envelope": {"schema": "appname.command.v1",
                             "id": "ad00d120-0000-4000-8000-00000000d120",
                             "kind": "layer.bogus_kind", "params": {}},
                "env": {"APPNAME_I18N_DIR": "<I18N_DIR>"}},
        "asserts": ["exit", "no-panic",
                    "single localized stderr line naming the code",
                    "id-quoted-in-line"],
        "protoExpect": {"declared": "exit=1 code=error.command.unknown_kind "
                        "(human mode)", "exit": 1, "exitAnyOf": None,
                        "outcome": "reject",
                        "code": "error.command.unknown_kind",
                        "observedCode": None},
        "cp01Expect": reject("error.command.unknown_kind"),
        "codeTier": "proto",
        "cp01Runnable": True,
        "notes": "registry miss works from CP-0.1; i18n loader is in the "
                 "CP-0.1 CLI row (proto catalogs as seed)",
    },
]


def load_ported_cli():
    cases = []
    for d in D_TABLE:
        c = dict(d)
        c["driver"] = "cli"
        c["provenance"] = ("appname-proto/scripts/adversarial.py case "
                           + d["caseId"][2:] + ", transcribed; observed "
                           "codes/exits from REPORT-A section 4.2")
        cases.append(c)
    if len(cases) != 12:
        raise SystemExit("expected 12 CLI cases, found %d" % len(cases))
    return {
        "suite": "ported-cli",
        "source": "appname-proto/scripts/adversarial.py driver-level cases "
                  "D01..D12 of the proven 41-case suite (REPORT-A section "
                  "4.2)",
        "note": "argv is written against the plan-fixed CLI verb surface "
                "(new/apply/script/undo/redo/history/export/import/info/"
                "selftest, exit 0/1/2, --json). Executable by the wave-2 "
                "CLI runner; cp01Runnable=false cases carry the reason in "
                "notes.",
        "cases": cases,
    }


# --------------------------------------------------------------------------
# generated/kinds/<kind>.json — the per-kind reject matrix scaffold.
# --------------------------------------------------------------------------

def load_kinds(repo):
    idx = yaml.safe_load(
        open(os.path.join(repo, "contracts", "commands", "_index.yaml"),
             encoding="utf-8"))
    rows = idx["commands"]
    if len(rows) != 102:
        raise SystemExit("expected 102 kinds in _index.yaml, found %d"
                         % len(rows))
    return {r["kind"]: r for r in rows}


def not_implemented_case(repo, kind, row, ordinal):
    contract = yaml.safe_load(
        open(os.path.join(repo, "contracts", "commands", row["file"]),
             encoding="utf-8"))
    env = dict(contract["examples"]["validMinimal"])
    env["id"] = "a0000000-0000-4000-8000-%012d" % ordinal
    return {
        "caseId": "G-NI-" + kind,
        "title": "schema-valid %s envelope at CP-0.1 (kind registered, "
                 "unimplemented)" % kind,
        "driver": "apply",
        "envelope": env,
        "protoExpect": None,
        "cp01Expect": reject("error.command.not_implemented"),
        "codeTier": "plan",
        "cp01Runnable": True,
        "provenance": PLAN_PROVENANCE + "; envelope = contracts/commands/"
                      + row["file"] + " examples.validMinimal with the id "
                      "rewritten to the a0-prefix positional scheme",
        "notes": "any uuid references inside params/target are the "
                 "contract's own example values and need not resolve; the "
                 "plan's registry rule makes not_implemented the expected "
                 "code regardless (" + ORDERING_NOTE + ")",
    }


# ---- doc.new param-validation matrix -------------------------------------

def doc_new_cases(repo, row):
    contract = yaml.safe_load(
        open(os.path.join(repo, "contracts", "commands", row["file"]),
             encoding="utf-8"))
    schema = contract["paramsSchema"]
    props = schema["properties"]
    required = list(schema["required"])
    vm = contract["examples"]["validMinimal"]
    vmx = contract["examples"]["validMaximal"]
    base = dict(vm["params"])  # {width, height}

    derivation = []  # (constraint description, caseId)
    gen = []  # (slug, title, params, cp01Expect, tier, provenance, notes)

    src = "generated from contracts/commands/doc.new.yaml paramsSchema"

    # 1. missing required properties -> error.schema.missing_field
    for p in sorted(required):
        params = {k: v for k, v in base.items() if k != p}
        gen.append((
            "missing-" + p,
            "required param '%s' absent" % p,
            params,
            reject("error.schema.missing_field"),
            "contract",
            src + " (required: %s); code per examples.invalid "
            "missing-required precedent" % p,
            None))
        derivation.append(("required:" + p, "missing-" + p))

    # 2. numeric bounds -> error.schema.out_of_range (below = minimum-1,
    #    above = maximum+1; mechanical rule)
    for p in sorted(props):
        spec = props[p]
        if spec.get("type") in ("integer", "number") and "minimum" in spec:
            below = spec["minimum"] - 1
            above = spec["maximum"] + 1
            for tag, val in (("below-min", below), ("above-max", above)):
                params = dict(base)
                params[p] = val
                gen.append((
                    p + "-" + tag,
                    "%s %s (%s bound %s)" % (
                        p, val, tag,
                        spec["minimum" if tag == "below-min" else "maximum"]),
                    params,
                    reject("error.schema.out_of_range"),
                    "contract",
                    src + " (%s %s..%s); code per examples.invalid "
                    "out-of-range precedent" % (p, spec["minimum"],
                                                spec["maximum"]),
                    None))
                derivation.append(("bounds:%s:%s" % (p, tag), p + "-" + tag))

    # 3. enum violations -> error.schema.out_of_range (proto A13 precedent:
    #    enum miss reported as out_of_range)
    for p in sorted(props):
        spec = props[p]
        if "enum" in spec:
            params = dict(base)
            params[p] = "not_in_enum"
            gen.append((
                p + "-bad-enum",
                "%s outside its enum %s" % (p, spec["enum"]),
                params,
                reject("error.schema.out_of_range"),
                "proto",
                src + " (enum %s); code per proto A13 precedent (enum miss "
                "-> out_of_range)" % p,
                None))
            derivation.append(("enum:" + p, p + "-bad-enum"))

    # 4. wrong JSON type per property -> error.schema.out_of_range (proto
    #    A07/A11/A16 precedent: shape violations reported as out_of_range;
    #    v5.5 registers no distinct bad-type id, OQ-2). Mechanical rule:
    #    numeric prop -> str(minimum); enum/string prop -> 12345.
    for p in sorted(props):
        spec = props[p]
        if spec.get("type") in ("integer", "number"):
            wrong = str(spec["minimum"])
        else:
            wrong = 12345
        params = dict(base)
        params[p] = wrong
        gen.append((
            p + "-bad-type",
            "%s with a JSON value of the wrong type (%r)" % (p, wrong),
            params,
            reject("error.schema.out_of_range"),
            "proto",
            src + " (type of %s); code per proto shape-violation precedent "
            "(A07/A11/A16), no distinct bad-type id in v5.5 (OQ-2)" % p,
            None))
        derivation.append(("type:" + p, p + "-bad-type"))

    # 5. unknown param field (additionalProperties: false) -> proto A16
    params = dict(base)
    params["unexpected_param"] = True
    gen.append((
        "unknown-param-field",
        "unknown key under params (additionalProperties: false)",
        params,
        reject("error.schema.out_of_range"),
        "proto",
        src + " (additionalProperties: false); code per proto A16 precedent",
        None))
    derivation.append(("additionalProperties", "unknown-param-field"))

    # ---- assemble generated reject envelopes (b0-prefix positional ids) --
    cases = []
    for ordinal, (slug, title, params, cp01, tier, prov, note) in \
            enumerate(sorted(gen, key=lambda g: g[0]), start=1):
        cases.append({
            "caseId": "G-DN-" + slug,
            "title": "doc.new: " + title,
            "driver": "apply",
            "envelope": {
                "schema": "appname.command.v1",
                "id": "b0000000-0000-4000-8000-%012d" % ordinal,
                "kind": "doc.new",
                "params": params,
            },
            "protoExpect": None,
            "cp01Expect": cp01,
            "codeTier": tier,
            "cp01Runnable": True,
            "provenance": prov,
            "notes": note or "reject on a FRESH document handle (doc.new is "
                             "implemented at CP-0.1)",
        })

    # 6. the contract's own examples.invalid, verbatim. The
    #    already-initialized case (expectError bad_target) is emitted as an
    #    envelopeSequence so it is self-contained and engine-runnable: a
    #    dedicated prime doc.new initializes the fresh handle, then the
    #    contract payload (verbatim) must reject at stage 6a.
    prime_env = {
        "schema": "appname.command.v1",
        "id": "b0000000-0000-4000-8000-777777777777",
        "kind": "doc.new",
        "params": {"width": 64, "height": 64},
    }
    for i, inv in enumerate(contract["examples"]["invalid"]):
        notes = "contract examples.invalid[%d] verbatim" % i
        envelope = inv["payload"]
        sequence = None
        if inv["expectError"] == "error.color.profile_space_mismatch":
            notes += ("; live-state stage 6b of the pinned order (section "
                      "5.5 profile-space check inside CP-0.1 doc.new)")
        if inv["expectError"] == "error.command.bad_target":
            sequence = [prime_env, inv["payload"]]
            envelope = None
            notes += ("; emitted as a sequence: the b0...777777777777 "
                      "prime doc.new initializes the handle, then the "
                      "verbatim payload rejects at stage 6a (fresh-handle "
                      "check of the pinned order)")
        case = {
            "caseId": "G-DN-contract-invalid-%d" % i,
            "title": "doc.new: " + inv["case"],
            "driver": "apply",
            "envelope": envelope,
            "protoExpect": None,
            "cp01Expect": reject(inv["expectError"]),
            "codeTier": "contract",
            "cp01Runnable": True,
            "provenance": "contracts/commands/doc.new.yaml "
                          "examples.invalid[%d] (payload and expectError "
                          "verbatim)" % i,
            "notes": notes,
        }
        if sequence is not None:
            case["envelopeSequence"] = sequence
        cases.append(case)
        derivation.append(("examples.invalid[%d]" % i,
                           "contract-invalid-%d" % i))

    # 7. accepts: contract examples + minimum boundary
    cases.append({
        "caseId": "G-DN-accept-valid-minimal",
        "title": "doc.new: contract validMinimal accepted",
        "driver": "apply",
        "envelope": vm,
        "protoExpect": None,
        "cp01Expect": accept(),
        "codeTier": "contract",
        "cp01Runnable": True,
        "provenance": "contracts/commands/doc.new.yaml examples."
                      "validMinimal verbatim",
        "notes": None,
    })
    cases.append({
        "caseId": "G-DN-accept-valid-maximal-schema-only",
        "title": "doc.new: contract validMaximal is schema-valid "
                 "(NOT executed: a 10^6 x 10^6 px allocation)",
        "driver": "apply",
        "envelope": vmx,
        "protoExpect": None,
        "cp01Expect": accept(),
        "codeTier": "contract",
        "cp01Runnable": False,
        "provenance": "contracts/commands/doc.new.yaml examples."
                      "validMaximal verbatim",
        "notes": "schema-validity fixture only; execution would allocate a "
                 "10^12-pixel document and the profile param additionally "
                 "needs section 5.5 resolution — the runner asserts "
                 "validation acceptance if O-CORE exposes a validate-only "
                 "entry, else skips",
    })
    bmin = {"schema": "appname.command.v1",
            "id": "b0000000-0000-4000-8000-999999999999",
            "kind": "doc.new",
            "params": {"width": 1, "height": 1}}
    cases.append({
        "caseId": "G-DN-accept-min-boundary",
        "title": "doc.new: 1x1 minimum-boundary document accepted",
        "driver": "apply",
        "envelope": bmin,
        "protoExpect": None,
        "cp01Expect": accept(),
        "codeTier": "contract",
        "cp01Runnable": True,
        "provenance": "generated from contracts/commands/doc.new.yaml "
                      "paramsSchema minima (width 1, height 1); boundary-"
                      "accept discipline per proto A21 (1x1 resize accepted)",
        "notes": None,
    })
    derivation.append(("boundary-accept:min", "accept-min-boundary"))

    return cases, [{"constraint": c, "caseId": "G-DN-" + s}
                   for c, s in derivation]


# --------------------------------------------------------------------------
# generated/envelope_rejects.json — envelope-stage cases (doc.new-flavored
# payloads exercising contracts/schemas/command-envelope.v1.json itself).
# --------------------------------------------------------------------------

def envelope_reject_cases():
    ok_params = {"width": 64, "height": 64}

    def env(**over):
        e = {"schema": "appname.command.v1", "id": None, "kind": "doc.new",
             "params": dict(ok_params)}
        e.update(over)
        return {k: v for k, v in e.items() if v is not None or k == "id"}

    defs = [
        ("duplicate-id",
         "same envelope id submitted twice (second apply must reject)",
         {"sequence": True},
         reject("error.schema.duplicate_id"), "contract",
         "contracts/message_ids.yaml error.schema.duplicate_id; proto A09 "
         "precedent (journal-duplicate check)",
         "runner applies the sequence in order on a fresh handle: first "
         "doc.new succeeds, byte-identical resubmission must reject"),
        ("id-not-uuid", "id is not a uuid",
         {"id": "not-a-uuid"},
         reject("error.schema.out_of_range"), "proto",
         "envelope schema format uuid; code per proto A07 observed",
         None),
        ("kind-not-string", "kind is a JSON number",
         {"kind": 5},
         reject("error.schema.out_of_range"), "proto",
         "envelope schema kind: string; code per proto shape-violation "
         "precedent (OQ-2)",
         None),
        ("kind-unknown", "kind names no registered command",
         {"kind": "doc.does_not_exist"},
         reject("error.command.unknown_kind"), "proto",
         "registry miss; proto A06 code, a registered v5.5 id "
         "(error.schema.unknown_command_kind twin is OQ-2)",
         None),
        ("missing-id-key", "required envelope key id absent",
         {"id": "__DROP__"},
         reject("error.schema.missing_field"), "contract",
         "contracts/schemas/command-envelope.v1.json required: id; "
         "error.schema.missing_field is a registered v5.5 id",
         None),
        ("missing-kind-key", "required envelope key kind absent",
         {"kind": None},
         reject("error.schema.missing_field"), "contract",
         "contracts/schemas/command-envelope.v1.json required: kind",
         None),
        ("missing-schema-key", "required envelope key schema absent",
         {"schema": None},
         reject("error.schema.missing_field"), "contract",
         "contracts/schemas/command-envelope.v1.json required: schema",
         None),
        ("params-not-object", "params is a JSON number",
         {"params": 3},
         reject("error.schema.out_of_range"), "proto",
         "envelope schema params: object; code per proto shape-violation "
         "precedent (OQ-2)",
         None),
        ("schema-wrong-const", "schema string is appname.command.v2",
         {"schema": "appname.command.v2"},
         reject("error.schema.wrong_schema"), "proto",
         "envelope schema const; code per proto A08 — "
         "error.schema.wrong_schema is NOT a registered v5.5 id (OQ-2)",
         None),
        ("unknown-top-level-key", "unknown envelope key beside the schema",
         {"unexpected_envelope_key": True},
         reject("error.schema.out_of_range"), "proto",
         "proto A16 observed (unknown top-level key rejected)",
         None),
    ]

    cases = []
    for ordinal, (slug, title, over, cp01, tier, prov, note) in \
            enumerate(sorted(defs, key=lambda d: d[0]), start=1):
        eid = "e0000000-0000-4000-8000-%012d" % ordinal
        if over.get("sequence"):
            e = env(id=eid)
            case_env = None
            seq = [e, json.loads(json.dumps(e))]
        else:
            e = env(id=eid)
            for k, v in over.items():
                if v is None:
                    e.pop(k, None)
                elif v == "__DROP__":
                    e.pop(k, None)
                else:
                    e[k] = v
            case_env = e
            seq = None
        cases.append({
            "caseId": "G-ENV-" + slug,
            "title": "envelope: " + title,
            "driver": "apply",
            "envelope": case_env,
            "envelopeSequence": seq,
            "protoExpect": None,
            "cp01Expect": cp01,
            "codeTier": tier,
            "cp01Runnable": True,
            "provenance": prov,
            "notes": note,
        })
    return {
        "suite": "generated-envelope-rejects",
        "source": "contracts/schemas/command-envelope.v1.json + proto "
                  "envelope-stage precedents (A06/A07/A08/A16); doc.new "
                  "payloads so the cases are executable from CP-0.1",
        "note": "envelope-stage validation is ported exactly from the proto "
                "(plan section 2.1 O-CORE row), so these are expected to "
                "hold at every checkpoint. G-ENV-duplicate-id repeats its "
                "envelope BY DESIGN.",
        "cases": cases,
    }


# --------------------------------------------------------------------------

def collect_codes(obj, out):
    if isinstance(obj, dict):
        for k, v in sorted(obj.items()):
            if k in ("code", "observedCode") and isinstance(v, str):
                out.add(v)
            elif k == "codeAnyOf" and isinstance(v, list):
                out.update(x for x in v if isinstance(x, str))
            else:
                collect_codes(v, out)
    elif isinstance(obj, list):
        for v in obj:
            collect_codes(v, out)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--proto", default=DEFAULT_PROTO)
    a = ap.parse_args()
    if not a.proto:
        raise SystemExit(
            "no proto reference configured: pass --proto, or set APPNAME_PROTO_ROOT "
            "(or APPNAME_PROTO_DIRNAME for the sibling of this tree). An empty value "
            "would make the check below read a RELATIVE path and could pass in the "
            "wrong tree (operator WORD 24, RULING A).")
    if not os.path.isdir(os.path.join(a.proto, "scripts")):
        raise SystemExit("proto reference not found at %s (need "
                         "scripts/adversarial.jsonl)" % a.proto)

    kinds = load_kinds(REPO)
    sorted_kinds = sorted(kinds)

    ported_env = load_ported_envelope(a.proto, kinds)
    ported_cli = load_ported_cli()
    env_rejects = envelope_reject_cases()

    param_edges = cp07_param_edges(REPO, kinds)

    kind_files = {}
    ni_count = 0
    for ordinal, kind in enumerate(sorted_kinds, start=1):
        row = kinds[kind]
        if kind == "doc.new":
            cases, derivation = doc_new_cases(REPO, row)
        else:
            cases = [not_implemented_case(REPO, kind, row, ordinal)]
            ni_count += 1
            derivation = None
        doc = {
            "kind": kind,
            "family": row["family"],
            "milestone": str(row["milestone"]),
            "contractFile": row["file"],
            "cases": cases,
        }
        overlay = cp03_overlay(kind)
        if overlay is not None:
            doc["cp03"] = overlay
        if derivation is not None:
            doc["docNewDerivation"] = derivation
        if kind in param_edges:
            doc["cp07ParamEdges"] = param_edges[kind]
        kind_files[kind] = doc

    # ---- write everything -------------------------------------------------
    out = HERE
    dump(os.path.join(out, "ported", "envelope.json"), ported_env)
    dump(os.path.join(out, "ported", "cli.json"), ported_cli)
    dump(os.path.join(out, "generated", "envelope_rejects.json"),
         env_rejects)
    for kind in sorted_kinds:
        dump(os.path.join(out, "generated", "kinds", kind + ".json"),
             kind_files[kind])

    # ---- manifest ---------------------------------------------------------
    msg = yaml.safe_load(open(os.path.join(REPO, "contracts",
                                           "message_ids.yaml"),
                              encoding="utf-8"))
    v55_error_ids = sorted(e["id"] for e in msg["messageIds"]
                           if e["id"].startswith("error."))
    referenced = set()
    collect_codes(ported_env, referenced)
    collect_codes(ported_cli, referenced)
    collect_codes(env_rejects, referenced)
    for doc in kind_files.values():
        collect_codes(doc, referenced)
    proto_only = sorted(referenced - set(v55_error_ids) - set(PLAN_IDS))

    docnew_cases = len(kind_files["doc.new"]["cases"])
    manifest = {
        "schema": "appname.qa.adversarialCorpus.v1",
        "generator": "fixtures/adversarial/generate_corpus.py "
                     "(seedless/stable; sorted iteration; byte-identical "
                     "re-runs against unchanged inputs)",
        "inputs": {
            "contracts/commands/_index.yaml": sha256_file(
                os.path.join(REPO, "contracts", "commands", "_index.yaml")),
            "contracts/commands/doc.new.yaml": sha256_file(
                os.path.join(REPO, "contracts", "commands", "doc.new.yaml")),
            "contracts/message_ids.yaml": sha256_file(
                os.path.join(REPO, "contracts", "message_ids.yaml")),
            "contracts/schemas/command-envelope.v1.json": sha256_file(
                os.path.join(REPO, "contracts", "schemas",
                             "command-envelope.v1.json")),
            "appname-proto/scripts/adversarial.jsonl": sha256_file(
                os.path.join(a.proto, "scripts", "adversarial.jsonl")),
        },
        "idSchemes": {
            "a0-prefix": "not-implemented envelopes: 1-based position of "
                         "the kind in the SORTED 102-kind list",
            "b0-prefix": "generated doc.new reject/accept envelopes: "
                         "1-based position in the sorted slug list "
                         "(999999999999 = min-boundary accept; "
                         "777777777777 = the already-initialized prime)",
            "e0-prefix": "generated envelope-stage cases: 1-based position "
                         "in the sorted slug list",
            "ported": "proto envelope ids verbatim",
        },
        "counts": {
            "portedEnvelope": len(ported_env["cases"]),
            "portedCli": len(ported_cli["cases"]),
            "portedTotal": len(ported_env["cases"])
                           + len(ported_cli["cases"]),
            "generatedNotImplemented": ni_count,
            "generatedDocNew": docnew_cases,
            "generatedEnvelopeRejects": len(env_rejects["cases"]),
            "kindFiles": len(kind_files),
            "total": len(ported_env["cases"]) + len(ported_cli["cases"])
                     + ni_count + docnew_cases + len(env_rejects["cases"]),
        },
        "kinds": sorted_kinds,
        "taxonomy": {
            "v55ErrorIds": v55_error_ids,
            "planIds": PLAN_IDS,
            "protoOnlyIds": proto_only,
            "note": "every code referenced by a case is in v55ErrorIds, "
                    "planIds, or protoOnlyIds; protoOnlyIds and planIds are "
                    "taxonomy divergences tracked as OQ-1/OQ-2 in "
                    "reports/O-QA-log.md",
        },
        "corpusRevision": 19,
        "cp07ParamEdges": {
            "rule": "ADDITIVE layer (O-QA wave 2, review defect-8 "
                    "assignment): per-kind `cp07ParamEdges` case lists "
                    "encode the contracts' examples.invalid for the 15 "
                    "CP-0.7 kinds; the historical `cases` arrays and "
                    "their aggregate pins are untouched. Run by "
                    "tests/conformance/tests/cp07_param_edges.rs on the "
                    "ASSEMBLED in-process engine, WD-1 raw-byte "
                    "discipline, prime-kit sequences.",
            "counts": {kind: len(v) for kind, v in
                       sorted(param_edges.items())},
            "total": sum(len(v) for v in param_edges.values()),
            "ruling": PE_RULING,
            "seamState": PE_SEAM,
            "primeKit": {
                "docEnvelopeId": PE_PRIME_DOC["id"],
                "backgroundLayerId": PE_BG_LAYER,
                "layerId": PE_LID,
                "smartLayerId": PE_SOL,
                "smartAssetId": "content-derived (QA-F7 ruling: the "
                                "asset id is the SS7.4 content address; "
                                "no fixed literal)",
                "adjustmentLayerId": PE_ADJL,
                "curvesOpId": PE_OP,
                "chainOpId": PE_CHOP,
                "note": "created ids are content-derived and "
                        "envelope-fixed; the runner asserts the embedded "
                        "literals against prime receipts (rule 7: nothing "
                        "hand-invented -- literals were extracted from "
                        "the shipped CLI 2026-08-13 and are re-verified "
                        "on every run; the rev-14 live-op prime ids were "
                        "observed accepting on the shipped five-crate "
                        "composition 2026-08-14 before encoding)",
            },
            "pendingArms": "rev-14 finalize: the eight rev-13 wiring-seam "
                           "alarms TRIPPED by design when assembled_engine "
                           "lifted to the shipped five-crate composition "
                           "and are resolved by this revision "
                           "(premise-real re-encodings, live-op primes); "
                           "arms still armed pin TODAY'S observed "
                           "behavior and carry pendingArm{code, "
                           "entersWith} for a flagged O-CORE fix/ruling "
                           "(QA-F2/F3/F4/F5/F12) or an unshipped wave "
                           "(color-lookup, linked sources, nested "
                           "composite); the runner failing after such a "
                           "landing IS the drift alarm -- finalize via "
                           "companion edit + O-QA ratification",
            "findings": {
                "QA-F2": "history.snapshot accepts name:'' despite "
                         "contract minLength:1",
                "QA-F3": "layer.rasterize what:'effects' on a plain "
                         "raster layer accepts (no-op) vs contract "
                         "target_kind_mismatch",
                "QA-F4": "smartobject.convert layerIds:[] answers the "
                         "v1 single-source not_implemented gate before "
                         "schema minItems out_of_range",
                "QA-F5": "smartobject.replace linked-source gate fires "
                         "before the source oneOf both-set check",
                "QA-F6": "smartobject.convert REQUIRES "
                         "newLayerId/newAssetId at the headless door "
                         "(missing_field) where the contract marks them "
                         "optional ('wrapper fills when omitted') -- "
                         "requiredness divergence, needs an O-CORE "
                         "recording or fix",
                "QA-F11": "section 9.6 creation-uuid uniqueness is "
                          "enforced per-arena only: a newOpId equal to "
                          "an existing LAYER id ACCEPTS (observed on "
                          "filter.smart_add and adjustment.add at the "
                          "rev-14 finalize) -- needs an O-CORE ruling",
                "QA-F12": "op.reorder index beyond the chain end CLAMPS "
                          "and ACCEPTS on a live chain op (section 40.6 "
                          "filter.smart_add clamp precedent) where the "
                          "contract spells error.schema.out_of_range -- "
                          "divergence, needs an O-CORE fix or ruling",
            },
        },
        "cp03Overlay": {
            "rule": "cp01Expect always stands; a kind file without a cp03 "
                    "key (and a case without cp03Assembled) carries it "
                    "forward unchanged on EVERY assembly. Overlays appear "
                    "only where behavior legitimately changes, split by "
                    "engine assembly (bareCore vs assembled, arbitration "
                    "#1).",
            "cp10DoorSlice3": "2026-08-17: rev 17 -- the thirteen "
                              "door-slice-3 kinds (selection.save/load, "
                              "the six channel.* kinds, slice.add/edit/"
                              "delete, meta.set, doc.mode_convert) "
                              "re-enter with their implementing wave per "
                              "the drift-alarm procedure; MECHANICAL "
                              "companion edit by O-CORE, flagged for "
                              "O-QA ratification "
                              "(reports/O-CORE-log.md)",
            "cp10RetouchRow": "2026-08-18: rev 18 -- the five retouch "
                              "stroke kinds (clone/heal/dodge_burn/"
                              "smudge/blur_sharpen) re-enter with their "
                              "implementing row per the drift-alarm "
                              "procedure; MECHANICAL companion edit by "
                              "O-TOOLS, flagged for O-QA ratification "
                              "(reports/O-TOOLS-log.md); remaining NI "
                              "set after this row: stroke_path + "
                              "doc.duplicate (dagger)",
            "cp10StrokePath": "2026-08-18: rev 19 -- stroke_path, the "
                              "LAST registry kind, re-enters with its "
                              "implementing slice per the drift-alarm "
                              "procedure (O-TEXTVEC stroke slice: the arm "
                              "beside FillKind, geometry through the "
                              "Door-1 stroke arms); MECHANICAL companion "
                              "edit by O-TEXTVEC, flagged for O-QA "
                              "ratification (reports/O-TEXTVEC-log.md); "
                              "remaining NI set after this slice: "
                              "doc.duplicate ONLY (dagger, permanently NI "
                              "by design) -- the pending set is TERMINAL",
            "pendingKinds": cp05_pending_kinds(kinds),
            "finalizedKinds": finalized_kinds(),
            "portedAssembledOverlayCases": sorted(
                "P-" + c for c in CP03_ASSEMBLED_PORTED),
            "arbitration": CP03_ARBITRATION,
            "stageMap": CP03_STAGE_MAP,
            "finalized": "2026-08-12, after stroke wiring merge 7adb470 "
                         "(drift alarm fired as designed and was resolved "
                         "by this revision); the alarm now guards any "
                         "FUTURE pendingKinds entry the same way",
            "cp05Wave1": "2026-08-12: fill + edit.cut re-enter with their "
                         "implementing wave (O-CORE CP-0.5 wave 1) per "
                         "the drift-alarm procedure; MECHANICAL companion "
                         "edit by O-CORE, RATIFIED by O-QA "
                         "(reports/O-QA-log.md ratification entry)",
            "cp05Wave2": "2026-08-12: transform.free/perspective/warp "
                         "re-entered by O-TOOLS' MECHANICAL companion "
                         "edits (0dceb11) per the procedure; RATIFIED by "
                         "O-QA and folded into the generator at rev 7 "
                         "(with the CP03_FINALIZED_KINDS naming-drift "
                         "fix: finalized_kinds() is now the one source)",
            "cp07Wave1": "2026-08-12: the eleven OpNode-family kinds + the "
                         "two history daggers re-enter with their "
                         "implementing wave (O-CORE CP-0.7 wave 1) per "
                         "the drift-alarm procedure at rev 11; the dagger "
                         "rows are the first bareCore departure "
                         "(engine-built-ins, doc.new precedent); "
                         "MECHANICAL companion edit by O-CORE, flagged "
                         "for O-QA ratification",
            "cp07Wave2": "2026-08-14: rev 14 (the O-QA FINALIZE round) -- "
                         "the five wiring-slice kinds (filter.apply, "
                         "filter.smart_add, image.auto_tone/contrast/"
                         "color) re-enter with their implementing wave "
                         "per the drift-alarm procedure; encoded by O-QA "
                         "ITSELF against the shipped five-crate "
                         "composition (assembled_engine == cli/cabi "
                         "assembly), resolving the tripped rev-13 "
                         "pendingArm alarms in the same revision "
                         "(premise-real re-encodings with live-op "
                         "primes; findings QA-F11/QA-F12 filed)",
            "cp07FixRound": "2026-08-13: rev 12 -- the OpNode review "
                            "fix-round stage-map refresh (unknown_op_kind "
                            "for unregistered opKind, section 40.7 PIXELS "
                            "gates, dagger undo-stack position "
                            "coordinates with out_of_range refusals); "
                            "case set and counts unchanged; MECHANICAL "
                            "companion edit by O-CORE, flagged for O-QA "
                            "ratification",
            "cp05Wave4": "2026-08-12: the nineteen wave-4 tail kinds "
                         "(layer family, geometry, merge/flatten, guides/"
                         "grid) re-enter with their implementing wave "
                         "(O-CORE CP-0.5 wave 4) per the drift-alarm "
                         "procedure at rev 10; the brokered fill lift's "
                         "stale flood-seed provenance sentence refreshed "
                         "in the same regen; MECHANICAL companion edit "
                         "by O-CORE, flagged for O-QA ratification",
            "cp05Wave2Core": "2026-08-12: the six mask.* kinds and the "
                             "three doc.* color kinds re-enter with "
                             "their implementing wave (O-CORE CP-0.5 "
                             "wave 2) per the drift-alarm procedure at "
                             "rev 8; MECHANICAL companion edit by "
                             "O-CORE, flagged for O-QA ratification "
                             "(reports/O-CORE-log.md)",
            "cp05Arming": "revision 8: the remaining milestone-0.5 "
                          "kinds stay PENDING (derived mechanically from "
                          "the index); the drift alarm watches each on "
                          "the assembled surface for the wave-2/3 "
                          "landings (geometry next; spill/autosave add "
                          "no kinds)",
        },
        "hedgeCollapse": {
            "why": "wave-4 collapse per the pinned validation order "
                   "(reports/O-CORE-log.md, binding OQ-3 arbitration): "
                   "stage 4 implemented-check precedes stage 5 params and "
                   "stage 6 target/live-state. Former arms transcribed "
                   "from the wave-1 corpus (git history); each re-enters "
                   "with its kind's implementing wave.",
            "collapsed": [
                {"caseId": "P-A02", "formerArms": ["error.command.not_implemented", "error.schema.out_of_range"], "pinned": "error.command.not_implemented", "winningStage": "4 < 5"},
                {"caseId": "P-A03", "formerArms": ["error.command.not_implemented", "error.schema.out_of_range"], "pinned": "error.command.not_implemented", "winningStage": "4 < 5"},
                {"caseId": "P-A04", "formerArms": ["error.command.not_implemented", "error.schema.out_of_range"], "pinned": "error.command.not_implemented", "winningStage": "4 < 5"},
                {"caseId": "P-A05", "formerArms": ["error.command.not_implemented", "error.schema.out_of_range"], "pinned": "error.command.not_implemented", "winningStage": "4 < 5"},
                {"caseId": "P-A10", "formerArms": ["error.command.not_implemented", "error.schema.out_of_range"], "pinned": "error.command.not_implemented", "winningStage": "4 < 5"},
                {"caseId": "P-A11", "formerArms": ["error.command.not_implemented", "error.schema.out_of_range"], "pinned": "error.command.not_implemented", "winningStage": "4 < 5"},
                {"caseId": "P-A12", "formerArms": ["error.command.not_implemented", "error.schema.unknown_op_kind"], "pinned": "error.command.not_implemented", "winningStage": "4 < 5"},
                {"caseId": "P-A13", "formerArms": ["error.command.not_implemented", "error.schema.out_of_range"], "pinned": "error.command.not_implemented", "winningStage": "4 < 5"},
                {"caseId": "P-A14", "formerArms": ["error.command.not_implemented", "error.schema.out_of_range"], "pinned": "error.command.not_implemented", "winningStage": "4 (samples carried at stage 1, slice-2 blocker-1)"},
                {"caseId": "P-A15", "formerArms": ["error.command.bad_target", "error.command.not_implemented"], "pinned": "error.command.not_implemented", "winningStage": "4 < 6"},
                {"caseId": "P-A17", "formerArms": ["error.command.not_implemented", "error.schema.exclusive_params", "error.schema.out_of_range"], "pinned": "error.command.not_implemented", "winningStage": "4 < 5"},
                {"caseId": "P-A18", "formerArms": ["error.command.not_implemented", "error.schema.missing_field"], "pinned": "error.command.not_implemented", "winningStage": "4 < 5"},
                {"caseId": "P-A19", "formerArms": ["error.command.not_implemented", "error.schema.out_of_range"], "pinned": "error.command.not_implemented", "winningStage": "4 < 5"},
                {"caseId": "P-D10", "formerArms": ["error.command.bad_target", "error.command.not_implemented"], "pinned": "error.command.not_implemented", "winningStage": "4 < 6"},
            ],
        },
        "intentionalDuplicateEnvelopeIds": [
            ["P-A01", "P-A09"],
            ["G-ENV-duplicate-id (sequence, applied twice by design)"],
            ["G-DN-contract-invalid-3 (prime + payload sequence; distinct "
             "ids, listed for the reader)"],
        ],
        "docNewDerivation": kind_files["doc.new"]["docNewDerivation"],
        "openQuestions": [
            "OQ-1: error.command.not_implemented is mandated by the plan "
            "CP-0.1 registry rule but absent from contracts/"
            "message_ids.yaml",
            "OQ-2: proto codes absent from the v5.5 registry (error.schema."
            "wrong_schema, error.api.*, error.history.*) and unresolved "
            "twins (error.command.unknown_kind vs error.schema."
            "unknown_command_kind; out_of_range vs exclusive_params for "
            "flat+nested params)",
            "OQ-3: CLOSED at wave 4 — validation order pinned "
            "(reports/O-CORE-log.md); all 14 codeAnyOf hedges collapsed to "
            "stage-4 error.command.not_implemented (see hedgeCollapse)",
        ],
    }
    dump(os.path.join(out, "manifest.json"), manifest)

    n_files = 3 + len(kind_files) + 1
    print("corpus written: %d files, %d cases (%d ported + %d generated)"
          % (n_files, manifest["counts"]["total"],
             manifest["counts"]["portedTotal"],
             manifest["counts"]["total"]
             - manifest["counts"]["portedTotal"]))
    return 0


if __name__ == "__main__":
    sys.exit(main())
