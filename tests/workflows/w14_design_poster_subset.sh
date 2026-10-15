#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# w14_design_poster_subset.sh — W14 design-poster GROUNDWORK SUBSET over
# the SHIPPED CLI (authored at the O-QA re-pin round 2026-08-16;
# leg D re-encoded + every trigger re-probed at the O-QA accumulated
# ratification round 2026-08-17). W14 proper is a CP-1.0 gate needing
# vector + text surfaces together (spec §41.4,
# tests/workflows/W14-design-poster.yaml). The TEXTVEC Door-1 merge
# (main e8f5f9b) put the vector DOC-MODEL spine live on the shipped
# assembly — shape layers, primitives, styling, grouping, duplication —
# and this script proves exactly that spine, then records every
# remaining gap with its trigger, each probed VERBATIM:
#
#   GAP 1 (step 1): linear-gradient fill layer (§22.4) — probed:
#     `error.schema.unknown_op_kind` ("gradient_fill" not in the §12.1
#     op registry; the compute half is the O-TOOLS seam).
#   GAP 2 (step 2): vector.path_bool shape-layer (APPEND) mode — probed:
#     typed NI ("the contract's XOR addressing leaves the appended
#     operand unspecified"; re-probed at the ratification round with the
#     evolved suffix "— escalated to O-INT; §20.3 bool_ops appends land
#     on its ruling"); TRIGGER: the path_bool APPEND spec-gap
#     fold-in hold (O-CORE door relay).
#   GAP 3 (step 3): the text kinds are now DOORED but SERVICELESS
#     (O-CORE door slice 2, Doors 6-10: core's kinds::register carries
#     the text family; NO TextLayout service is on the shipped assembly)
#     — re-probed at the accumulated ratification round: text.add
#     answers the typed TextLayout-NI mirror ("no TextLayout service is
#     registered on this assembly (appname-text registers it at
#     construction; bare-core mirror)"); the resolving kinds answer
#     stage-4 bad_target on dangling targets. TRIGGER (narrowed): the
#     appname-text wiring slice — one PRODUCTION_CHAIN row registering
#     TextLayout (the TEXTVEC filing at C-5 close).
#   GAP 4 (step 5): layer.rasterize of a shape layer — probed:
#     `error.command.target_kind_mismatch` ("layer kind \"shape\" has no
#     rasterize arm"); TRIGGER: the shape rasterize arm (O-CORE), which
#     also unblocks the duplicate→rasterize→blur chain. Re-probed
#     verbatim at the ratification round: STANDS.
#   GAP 5 (steps 6-7): SHAPE-LAYER COMPOSITE — probed: export refuses a
#     VISIBLE shape layer with `error.command.not_implemented` ("layer
#     kind \"shape\" has no CP-0.5 composite path"); invisible shape
#     layers composite fine (boundary probed both ways; leg C PINS the
#     refusal so the flip self-announces when the composite path lands —
#     this script then re-encodes). TRIGGER: the shape
#     coverage→composite binding (O-RENDER/O-CORE seam). Re-probed at
#     the ratification round: STANDS (NOT flipped by the subtree-
#     composite door — that door serves ABI read_layer, not export);
#     message gained the "render composite refused: layer <id>:" prefix
#     (leg C's code + substring pin covers both spellings).
#   GAP 6 (steps 6/8): golden w14_poster.png + layered PDF export — no
#     shape composite yet (GAP 5), no PDF on the CLI (formats: png,
#     jpeg), no §11.3 layer-effects doc-model surface; TRIGGER: full
#     W14 at its tier + the O-IO export subsystem.
#
# Canvas rides at 877×1240 (A3 proportions, subset scale); the full
# 3508×4961 + budget legs belong to full W14 at its tier.
#
# What IS proven: shape_add (rounded_rect + ellipse, pinned ids, solid
# fill + 4 px centered stroke), shape_set_style on a RESOLVING shape
# target (live-service styling — beyond the NI-sweep's dangling-id arm),
# layer.group isolated + multiply@85%, layer.duplicate of a shape layer
# (idMap arm over vector content), save/reopen structure, the GAP-5
# refusal pin, the DEFECT-1 FIX pin (leg D, RE-ENCODED at the O-QA
# accumulated ratification round: the filed rounded_rect sidecar
# non-fixpoint was rooted in serde_json's non-roundtripping float parse
# and fixed by the float_roundtrip election — TEXTVEC fix, main
# 0996cdf; the leg's old refusal pin self-announced exactly as designed
# and now pins the SUCCESS: cross-package undo/redo cursor walk on the
# rounded_rect-bearing package, so a fixpoint REGRESSION re-announces
# here), the same walk on the ellipse doc (the pair isolates any future
# primitive-dependent drift), and — on the raster ground with shapes
# hidden, now the FULL poster spine since every primitive is
# fixpoint-clean — a two-tone fill, filter.apply gaussian, export,
# two-run byte determinism (independent processes end to end),
# undo/redo byte-identity at the export door.
#
# Spec: §41.4 (W14); ES §20.2 shape styling, §40.3 idMap, §16.3
# gaussian; D-48. Exit: 0 pass · 1 fail · 2 environment not ready.

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

CLI="${APPNAME_CLI:-}"
if [ -z "$CLI" ]; then
    for cand in "$ROOT/target/debug/appname" "$ROOT/target/release/appname"; do
        [ -x "$cand" ] && CLI="$cand" && break
    done
fi
if [ -z "$CLI" ] && command -v appname >/dev/null 2>&1; then
    CLI="$(command -v appname)"
fi
if [ -z "$CLI" ]; then
    echo "SKIP: appname-cli not built (cargo build -p appname-cli)"
    exit 2
fi

WORK="$(mktemp -d "${TMPDIR:-/tmp}/appname-qa-w14-XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
jget() { python3 -c 'import json,sys
o = json.load(sys.stdin)
for k in sys.argv[1:]:
    o = o[int(k)] if isinstance(o, list) else o[k]
print(o)' "$@"; }

fail=0

RRECT="b0000000-0000-4000-8000-000000000001"
ELLIP="b0000000-0000-4000-8000-000000000002"
GROUP="b0000000-0000-4000-8000-000000000003"
DUPSHAPE="b0000000-0000-4000-8000-000000000004"

# The poster vector spine as ONE transactional script (pinned ids keep
# every target addressable and the walk deterministic).
cat > "$WORK/poster.jsonl" <<EOF
{"schema":"appname.command.v1","id":"a0000000-0000-4000-8000-0000000e1401","kind":"vector.shape_add","params":{"primitive":{"kind":"rounded_rect","geometry":{"x":100,"y":200,"w":400,"h":300},"cornerRadius":24},"fill":{"type":"solid","color":{"model":"rgba","comps":[0.2,0.4,0.9,1]}},"stroke":{"widthPx":4,"color":{"model":"rgba","comps":[0,0,0,1]},"align":"center"},"newLayerId":"$RRECT"}}
{"schema":"appname.command.v1","id":"a0000000-0000-4000-8000-0000000e1402","kind":"vector.shape_add","params":{"primitive":{"kind":"ellipse","geometry":{"x":300,"y":350,"w":300,"h":300}},"fill":{"type":"solid","color":{"model":"rgba","comps":[0.95,0.75,0.1,1]}},"stroke":{"widthPx":0},"newLayerId":"$ELLIP"}}
{"schema":"appname.command.v1","id":"a0000000-0000-4000-8000-0000000e1403","kind":"vector.shape_set_style","target":{"layerId":"$ELLIP"},"params":{"stroke":{"widthPx":4,"color":{"model":"rgba","comps":[0,0,0,1]},"align":"center"}}}
{"schema":"appname.command.v1","id":"a0000000-0000-4000-8000-0000000e1404","kind":"layer.group","params":{"layerIds":["$RRECT","$ELLIP"],"name":"Shapes","isolated":true,"newGroupId":"$GROUP"}}
{"schema":"appname.command.v1","id":"a0000000-0000-4000-8000-0000000e1405","kind":"layer.set_props","target":{"layerId":"$GROUP"},"params":{"blend":"multiply","opacity":0.85}}
{"schema":"appname.command.v1","id":"a0000000-0000-4000-8000-0000000e1406","kind":"layer.duplicate","target":{"layerId":"$RRECT"},"params":{"idMap":{"$RRECT":"$DUPSHAPE"}}}
EOF

# Leg A: the doc-model spine applies end-to-end (one transaction).
"$CLI" new --width 877 --height 1240 --background white \
    --out "$WORK/poster.appname" --json > "$WORK/new.json"
"$CLI" script --doc "$WORK/poster.appname" --file "$WORK/poster.jsonl" \
    --json > "$WORK/script.json"
APPLIED="$(jget applied < "$WORK/script.json")"
if [ "$APPLIED" = "6" ]; then
    echo "PASS vector-spine-applies (6 commands, one transaction)"
else
    echo "FAIL vector-spine-applies: applied=$APPLIED"
    fail=1
fi

# Leg B: reopened structure (fresh process reads the saved package).
STRUCT="$("$CLI" info --doc "$WORK/poster.appname" --json \
    | python3 -c 'import json,sys
info = json.load(sys.stdin)["info"]
print(",".join(l["kind"] + ":" + l["name"] for l in info["layers"]))')"
# Observed truth (deterministic walk: the duplicate inserts above its
# source INSIDE the group; info flattens the tree): ground, the group,
# its three members (source, the "Shape copy" duplicate, the ellipse).
WANT_STRUCT="raster:Background,group:Shapes,shape:Shape,shape:Shape copy,shape:Shape"
if [ "$STRUCT" = "$WANT_STRUCT" ]; then
    echo "PASS reopened-structure ($STRUCT)"
else
    echo "FAIL reopened-structure: got $STRUCT want $WANT_STRUCT"
    fail=1
fi

# Leg C: the GAP-5 refusal PIN (visible shape → composite refuses with
# exactly the probed id; a flip means the trigger fired — re-encode).
set +e
REFUSAL="$("$CLI" export --doc "$WORK/poster.appname" --format png \
    --out "$WORK/refused.png" --json 2>/dev/null)"
set -e
CODE="$(printf '%s' "$REFUSAL" | jget error code)"
# RE-ENCODED for the CP-1.0 candidate (O-QA, 2026-08-19): the shape-
# composite refusal MESSAGE evolved since the CP-0.5 pin — shape FILL
# compositing landed, so the refusal is now scoped to shape STROKE
# compositing ("(§20.3) has no reference path yet … fill-only v1").
# The refusal is STILL typed (error.command.not_implemented) and never
# omitted — the D-48 typed-refusal discipline holds; only the wording
# progressed. Per this leg's own "a flip means the trigger fired —
# re-encode" instruction, the pin now accepts either the historical
# CP-0.5 wording OR the shipped §20.3 fill-only-v1 shape-composite
# refusal (both are the same GAP-5 typed refusal at their checkpoint).
if [ "$CODE" = "error.command.not_implemented" ] \
    && printf '%s' "$REFUSAL" \
        | grep -qE 'no CP-0.5 composite path|shape STROKE compositing \(§20.3\)|fill-only v1'; then
    echo "PASS composite-refusal-pinned ($CODE — GAP 5 trigger armed; §20.3 fill-only-v1 re-encode)"
else
    echo "FAIL composite-refusal-pinned: got $REFUSAL"
    fail=1
fi

# Leg D: the DEFECT-1 FIX PIN (RE-ENCODED, O-QA accumulated
# ratification round 2026-08-17). History: this leg pinned the filed
# refusal (rounded_rect sidecar non-fixpoint under the registered
# validate_sidecar broke replay-as-recovery undo); the root cause was
# serde_json's non-correctly-rounded float parse (1-ulp drift), fixed
# at the source by the float_roundtrip feature election (TEXTVEC
# DEFECT-1 fix, main 0996cdf; deps/serde.yaml amendment records the
# election). The old pin self-announced the flip exactly as designed
# ("A flip here means the fix landed: re-encode legs D/D2") and the
# re-encode pins the SUCCESS: cross-package undo/redo cursor walk on
# the rounded_rect-BEARING poster package (6 spine commands + doc.new =
# cursor 7 -> 6 -> 7). load(package) == replay(journal) is implied by
# the undo door opening at all (replay-as-recovery is its gate). A
# fixpoint REGRESSION on any primitive re-announces here as a FAIL.
"$CLI" undo --doc "$WORK/poster.appname" --json > "$WORK/undo_d.json"
PCUR_UNDO="$(jget cursor < "$WORK/undo_d.json")"
"$CLI" redo --doc "$WORK/poster.appname" --json > "$WORK/redo_d.json"
PCUR_REDO="$(jget cursor < "$WORK/redo_d.json")"
if [ "$PCUR_UNDO" = "6" ] && [ "$PCUR_REDO" = "7" ]; then
    echo "PASS undo-redo-cursor-walk-rounded-rect (DEFECT-1 fixed: 7 -> 6 -> 7 on the poster package)"
else
    echo "FAIL undo-redo-cursor-walk-rounded-rect: undo=$PCUR_UNDO redo=$PCUR_REDO (a DEFECT-1-class fixpoint regression re-announces here)"
    fail=1
fi

# Leg D2: the same walk on the ellipse-only doc — the pair
# (rounded_rect above, ellipse here) isolates any future
# primitive-dependent fixpoint drift the way the DEFECT-1
# characterization did.
"$CLI" new --width 877 --height 1240 --background white \
    --out "$WORK/ellip.appname" --json > /dev/null
"$CLI" apply --doc "$WORK/ellip.appname" --command "{\"schema\":\"appname.command.v1\",\"id\":\"a0000000-0000-4000-8000-0000000e1410\",\"kind\":\"vector.shape_add\",\"params\":{\"primitive\":{\"kind\":\"ellipse\",\"geometry\":{\"x\":300,\"y\":350,\"w\":300,\"h\":300}},\"fill\":{\"type\":\"solid\",\"color\":{\"model\":\"rgba\",\"comps\":[0.95,0.75,0.1,1]}},\"stroke\":{\"widthPx\":0},\"newLayerId\":\"$ELLIP\"}}" \
    --json > /dev/null
"$CLI" undo --doc "$WORK/ellip.appname" --json > /dev/null
CUR_AFTER_UNDO="$("$CLI" info --doc "$WORK/ellip.appname" --json | jget info journal cursor)"
"$CLI" redo --doc "$WORK/ellip.appname" --json > /dev/null
CUR_AFTER_REDO="$("$CLI" info --doc "$WORK/ellip.appname" --json | jget info journal cursor)"
if [ "$CUR_AFTER_UNDO" = "1" ] && [ "$CUR_AFTER_REDO" = "2" ]; then
    echo "PASS undo-redo-cursor-walk-ellipse (2 -> 1 -> 2)"
else
    echo "FAIL undo-redo-cursor-walk-ellipse: undo=$CUR_AFTER_UNDO redo=$CUR_AFTER_REDO"
    fail=1
fi

# Leg E: with the shape content HIDDEN (the probed boundary: invisible
# shape layers composite), the raster ground runs the export legs. The
# spine here is the FULL poster spine (re-encoded with the DEFECT-1
# fix: rounded_rect is fixpoint-clean, so nothing forces the old
# ellipse-only variant), so the cross-package undo leg below exercises
# the export door over a rounded_rect-bearing package too.
# RE-ENCODED for the CP-1.0 candidate (O-QA, 2026-08-19): the raster
# leg's gaussian_blur param key was the stale "radiusPx"; the registered
# filter schema's key is "radius" (crates/appname-filters op_bridge;
# schema additionalProperties:false), and the 2026-08-17 strictness
# ruling now HARD-REJECTS unknown keys (error.schema.out_of_range). The
# candidate is correct (radius:12 applies; radiusPx is a genuine reject);
# this is a stale-key harness fix, not a product finding.
run_raster_legs() {
    local doc="$1"
    "$CLI" new --width 877 --height 1240 --background white \
        --out "$doc" --json > "$WORK/new2.json"
    local bg
    bg="$(jget receipt createdIds -1 < "$WORK/new2.json")"
    "$CLI" script --doc "$doc" --file "$WORK/poster.jsonl" --json > /dev/null
    sed "s/BGID/$bg/g" > "$WORK/raster.jsonl" <<'EOF'
{"schema":"appname.command.v1","id":"a0000000-0000-4000-8000-0000000e1408","kind":"layer.set_props","target":{"layerId":"b0000000-0000-4000-8000-000000000003"},"params":{"visible":false}}
{"schema":"appname.command.v1","id":"a0000000-0000-4000-8000-0000000e1409","kind":"layer.set_props","target":{"layerId":"b0000000-0000-4000-8000-000000000004"},"params":{"visible":false}}
{"schema":"appname.command.v1","id":"a0000000-0000-4000-8000-0000000e140a","kind":"selection.set","params":{"source":"rect","rect":{"x":0,"y":0,"w":438,"h":1240},"antialias":false}}
{"schema":"appname.command.v1","id":"a0000000-0000-4000-8000-0000000e140b","kind":"fill","target":{"layerId":"BGID"},"params":{"content":"color","color":{"model":"rgb","comps":[0.85,0.15,0.15,1.0]}}}
{"schema":"appname.command.v1","id":"a0000000-0000-4000-8000-0000000e140c","kind":"selection.none","params":{}}
{"schema":"appname.command.v1","id":"a0000000-0000-4000-8000-0000000e140d","kind":"filter.apply","target":{"layerId":"BGID"},"params":{"filterId":"filter.gaussian_blur","params":{"radius":12}}}
EOF
    "$CLI" script --doc "$doc" --file "$WORK/raster.jsonl" --json > /dev/null
}

run_raster_legs "$WORK/r1.appname"
"$CLI" export --doc "$WORK/r1.appname" --format png --out "$WORK/r1.png" \
    --json > /dev/null
H1="$(sha "$WORK/r1.png")"
run_raster_legs "$WORK/r2.appname"
"$CLI" export --doc "$WORK/r2.appname" --format png --out "$WORK/r2.png" \
    --json > /dev/null
H2="$(sha "$WORK/r2.png")"
if [ "$H1" = "$H2" ]; then
    echo "PASS two-run-determinism ($H1)"
else
    echo "FAIL two-run-determinism: $H1 vs $H2"
    fail=1
fi

# undo the gaussian → export differs; redo → byte-exact again.
"$CLI" undo --doc "$WORK/r1.appname" --json > /dev/null
"$CLI" export --doc "$WORK/r1.appname" --format png --out "$WORK/undone.png" \
    --json > /dev/null
"$CLI" redo --doc "$WORK/r1.appname" --json > /dev/null
"$CLI" export --doc "$WORK/r1.appname" --format png --out "$WORK/redone.png" \
    --json > /dev/null
if [ "$(sha "$WORK/undone.png")" != "$H1" ] \
    && [ "$(sha "$WORK/redone.png")" = "$H1" ]; then
    echo "PASS undo-redo-restore (undo moves the export, redo restores byte-exact)"
else
    echo "FAIL undo-redo-restore"
    fail=1
fi

# Recorded gaps (not failures — the honest W14 posture; triggers in the
# header block).
echo "GAP gradient-fill-layer (§22.4): unknown_op_kind — O-TOOLS §12.1 seam"
echo "GAP path_bool-shape-mode: typed NI — APPEND fold-in hold"
echo "GAP text-kinds (step 3): doored serviceless (typed TextLayout-NI) — appname-text wiring slice"
echo "GAP shape-rasterize (step 5 chain): no shape arm — O-CORE"
echo "GAP shape-composite (steps 6-7): pinned refusal in leg C — O-RENDER/O-CORE seam"
echo "GAP golden/PDF/effects (steps 6-8): full W14 at its tier — O-IO export subsystem"
echo "DEFECT-1 (FIXED, main 0996cdf): float_roundtrip election — leg D pins the success; a regression re-announces"

if [ "$fail" -eq 0 ]; then
    echo "W14 design-poster SUBSET: PASS (vector doc-model spine live; gaps recorded with triggers)"
else
    echo "W14 design-poster SUBSET: FAIL"
fi
exit "$fail"
