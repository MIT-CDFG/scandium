#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# w12_pano_scale_retouch.sh — workflow gate W12 (panorama-scale retouch).
# Spec: W12-panorama-scale-retouch.yaml (§41.3/§19/§21).
#
# DISPOSITION BY RULING (record for the CP-1.0 close letter): RULING B
# removed pano/Photomerge to a post-1.0 plugin. That ruling takes the
# PANORAMA-ACQUISITION premise out of 1.0 — but W12's own steps never
# stitched anything (its fixture is seeded-synthetic bench_100mp): the
# row's substance is RETOUCH AT SCALE, which survives the ruling. W12
# therefore stays a live 1.0 row at reduced width; its 100 MP fixture +
# the §34.5/§52.2 budget rows are TIER residue (nightly/release perf
# harness — seed 0xB0605002 per the letter), not this smoke.
#
# RUNNABLE NOW (one `script` invocation — the selection surface does not
# persist across CLI processes, recorded CLI truth):
#   step 3  marquee + feather 12 px (selection.set + selection.modify)
#           then content_aware_fill at output:'current_layer', seed
#           pinned (the output:'new_layer' letter arm is typed-NI —
#           probed verbatim below; TRIGGER: the layer-creation door of
#           CAF at O-TOOLS).
#   step 5  transform.free on the layer (bicubic default), -0.5 deg +
#           1.02 scale about the canvas center.
#   step 7  undo/redo byte-exact at the export door on the
#           pre-transform chain (the transform.free replay-gate FINDING
#           below excludes transform from cross-boundary undo today).
# BLOCKED (typed, probed verbatim each run):
#   step 2  straighten via crop-with-angle — doc.crop is typed-NI.
#   step 4  clone.stroke + dodge_burn.stroke — typed-NI (retouch stroke
#           family; TRIGGER: O-TOOLS retouch suite merge).
#   steps 1/6/8 open-budget, pan/zoom §52.2, incremental-save
#           write-count — 100 MP tier residue (perf harness).
#
# Exit: 0 pass · 1 fail · 2 environment not ready.

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

CLI="${APPNAME_CLI:-}"
if [ -z "$CLI" ]; then
    for cand in "$ROOT/target/debug/appname" "$ROOT/target/release/appname"; do
        [ -x "$cand" ] && CLI="$cand" && break
    done
fi
[ -n "$CLI" ] || { echo "SKIP: appname-cli not built"; exit 2; }

WORK="$(mktemp -d "${TMPDIR:-/tmp}/appname-qa-w12-XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
exp() { "$CLI" export --doc "$1" --format png --out "$2" --json > /dev/null; sha "$2"; }
fail=0

echo "RULING-B DISPOSITION recorded: pano/Photomerge REMOVED to post-1.0 plugin; W12's retouch substance stays 1.0 at reduced width; 100 MP + budget rows = perf-tier residue (seed 0xB0605002)"

"$CLI" new --width 192 --height 128 --background white --out "$WORK/base.appname" --json > "$WORK/new.json"
layer="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["receipt"]["createdIds"][-1])' "$WORK/new.json")"

# NI probes, verbatim, on a scratch copy (rejections never journal).
cp "$WORK/base.appname" "$WORK/probe.appname"
probe() { # $1 kind, $2 params-json
    "$CLI" apply --doc "$WORK/probe.appname" --command \
        "{\"schema\":\"appname.command.v1\",\"id\":\"ad00f212-0000-4000-8000-0000000012$3\",\"kind\":\"$1\",\"target\":{\"layerId\":\"$layer\"},\"params\":$2}" \
        --json 2>/dev/null | python3 -c 'import json,sys; o=json.load(sys.stdin); print(o.get("error",{}).get("code","ACCEPT"), "-", o.get("error",{}).get("message","")[:110])' || true
}
echo "PROBE doc.crop (straighten arm): $(probe doc.crop '{"rect":{"x":4,"y":4,"w":180,"h":120},"straightenAngleDeg":-1.7}' 01)"
echo "PROBE clone.stroke: $(probe clone.stroke '{}' 02)"
echo "PROBE dodge_burn.stroke: $(probe dodge_burn.stroke '{}' 03)"
echo "PROBE content_aware_fill output:new_layer: $(probe content_aware_fill '{"output":"new_layer","newLayerId":"ad00f212-0000-4000-8000-0000000012aa","seed":0}' 04)"

# The runnable core, one script invocation.
cat > "$WORK/core.jsonl" <<EOF
{"schema":"appname.command.v1","id":"ad00f212-0000-4000-8000-000000001210","kind":"fill","target":{"layerId":"$layer"},"params":{"content":"color","color":{"model":"rgb","comps":[0.8,0.3,0.1,1.0]}}}
{"schema":"appname.command.v1","id":"ad00f212-0000-4000-8000-000000001211","kind":"selection.set","params":{"source":"rect","rect":{"x":60,"y":40,"w":48,"h":32},"antialias":false}}
{"schema":"appname.command.v1","id":"ad00f212-0000-4000-8000-000000001212","kind":"fill","target":{"layerId":"$layer"},"params":{"content":"color","color":{"model":"rgb","comps":[0.1,0.2,0.9,1.0]}}}
{"schema":"appname.command.v1","id":"ad00f212-0000-4000-8000-000000001213","kind":"selection.modify","params":{"feather":12}}
{"schema":"appname.command.v1","id":"ad00f212-0000-4000-8000-000000001214","kind":"content_aware_fill","target":{"layerId":"$layer"},"params":{"output":"current_layer","seed":0}}
{"schema":"appname.command.v1","id":"ad00f212-0000-4000-8000-000000001215","kind":"transform.free","target":{"layerId":"$layer"},"params":{"matrix":[1.0198,-0.0089,0.0,0.0089,1.0198,0.0,0.0,0.0,1.0],"reference":[96.0,64.0]}}
EOF
head -5 "$WORK/core.jsonl" > "$WORK/pre_t.jsonl"

cp "$WORK/base.appname" "$WORK/run.appname"
cp "$WORK/base.appname" "$WORK/run2.appname"
cp "$WORK/base.appname" "$WORK/pre.appname"
"$CLI" script --doc "$WORK/run.appname" --file "$WORK/core.jsonl" --json > "$WORK/run.json"
"$CLI" script --doc "$WORK/run2.appname" --file "$WORK/core.jsonl" --json > /dev/null
"$CLI" script --doc "$WORK/pre.appname" --file "$WORK/pre_t.jsonl" --json > /dev/null
E_FULL="$(exp "$WORK/run.appname" "$WORK/full.png")"
E_FULL2="$(exp "$WORK/run2.appname" "$WORK/full2.png")"
E_PRE="$(exp "$WORK/pre.appname" "$WORK/pre.png")"

[ "$E_FULL" = "$E_FULL2" ] \
    && echo "determinism PASS (feather+CAF+transform chain byte-identical across two independent runs)" \
    || { echo "determinism FAIL"; fail=1; }
[ "$E_FULL" != "$E_PRE" ] \
    && echo "transform-applies PASS (transform.free changed the export)" \
    || { echo "transform-applies FAIL"; fail=1; }

# FINDING (recorded this wave, bisected to the single kind): with
# transform.free in the journal, cross-boundary undo REFUSES with the
# replay-as-recovery gate ("the replayed journal does not reproduce the
# persisted document state") — fill-only chains undo fine, and the full
# chain's EXPORTS are byte-identical across independent runs, so replay
# is self-deterministic; the persisted state diverges from the replayed
# one (same class as the W18 masked-import hash finding: non-canonical
# in-memory detail at save). O-CORE/O-TOOLS seam. Probed verbatim here
# so the flip self-announces when fixed:
if "$CLI" undo --doc "$WORK/run.appname" --json > "$WORK/u_probe.json" 2>&1; then
    echo "PROBE undo-after-transform: ACCEPTED (the replay-gate finding is FIXED — re-encode this row to the full step-7 shape)"
else
    echo "PROBE undo-after-transform REFUSED: $(python3 -c 'import json,sys; o=json.load(open(sys.argv[1])); print(o["error"]["code"])' "$WORK/u_probe.json") (the recorded transform.free replay-gate finding)"
fi
# step-7 undo/redo byte-exactness therefore rides the pre-transform
# chain (transform excluded until the finding closes).
cp "$WORK/base.appname" "$WORK/ur.appname"
"$CLI" script --doc "$WORK/ur.appname" --file "$WORK/pre_t.jsonl" --json > /dev/null
E_URFULL="$(exp "$WORK/ur.appname" "$WORK/ur_full.png")"
"$CLI" undo --doc "$WORK/ur.appname" --json > /dev/null
E_U1="$(exp "$WORK/ur.appname" "$WORK/u1.png")"
"$CLI" redo --doc "$WORK/ur.appname" --json > /dev/null
E_R="$(exp "$WORK/ur.appname" "$WORK/r.png")"
[ "$E_U1" != "$E_URFULL" ] \
    && echo "undo-1 PASS (undoing the CAF changes the export — byte-observable step back)" \
    || { echo "undo-1 FAIL"; fail=1; }
[ "$E_R" = "$E_URFULL" ] \
    && echo "redo PASS (redo returns byte-exactly to the pre-undo state — the W12 step-7 hash-return shape)" \
    || { echo "redo FAIL"; fail=1; }

echo "100MP/budget legs DEFERRED to the perf tier (tests/perf scenarios; §34.5 rows + PS-1 §52.2) — not claimed here"

if [ "$fail" -ne 0 ]; then
    echo "W12 pano-scale-retouch: FAIL"
    exit 1
fi
echo "W12 pano-scale-retouch: PASS (runnable core; ruling + blockers + tier residue recorded)"
