#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# quick_select_smoke.sh — quick-select workflow smoke over the SHIPPED
# CLI (O-QA CP-1.0 ratification batch, W-additions item: "what the new
# surface makes runnable"). ES §13.3 quick selection drives a §13.2-gated
# fill end-to-end: seed-select the red surface, fill blue THROUGH the
# quick selection, byte-exact two-run determinism, subtract-over-wider-
# prior leg, undo byte-identity at the export door.
#
# Surface honesty (recorded): the GrabCut refine tier and the §13.4
# Select&Mask pipeline have NO wire kinds yet (escalations riding with
# O-INT — refine-tier addressing + the stage-9 layer-creation door), so
# this smoke covers the wire-live quick arm; the engine-backed refine/
# Select&Mask surface is smoked in
# tests/conformance/tests/select_workflows_smoke.rs.
#
# Spec: ES §13.2/§13.3; contracts selection.set.yaml (source:"quick",
# seed, op); plan §2.5 O-QA row (workflow gates); D-48.
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
if [ -z "$CLI" ] && command -v appname >/dev/null 2>&1; then
    CLI="$(command -v appname)"
fi
if [ -z "$CLI" ]; then
    echo "SKIP: appname-cli not built (cargo build -p appname-cli)"
    exit 2
fi

WORK="$(mktemp -d "${TMPDIR:-/tmp}/appname-qa-quick-XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
exp() {
    "$CLI" export --doc "$1" --format png --out "$2" --json > /dev/null
    sha "$2"
}

fail=0

# Fresh doc; the background layer id feeds the fill targets.
"$CLI" new --width 96 --height 64 --background white --out "$WORK/base.appname" \
    --json > "$WORK/new.json"
layer="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["receipt"]["createdIds"][-1])' "$WORK/new.json")"

# Script A: two-tone setup (left half red) + quick seed-select + blue
# fill THROUGH the selection. One script run = one process, so the
# selection surface stays live between commands (the package does not
# persist it — recorded CLI truth).
cat > "$WORK/a.jsonl" <<EOF
{"schema":"appname.command.v1","id":"a10c0001-0000-4000-8000-000000000001","kind":"selection.set","params":{"source":"rect","rect":{"x":0,"y":0,"w":48,"h":64},"antialias":false}}
{"schema":"appname.command.v1","id":"a10c0002-0000-4000-8000-000000000002","kind":"fill","target":{"layerId":"$layer"},"params":{"content":"color","color":{"model":"rgb","comps":[1.0,0.0,0.0,1.0]}}}
{"schema":"appname.command.v1","id":"a10c0003-0000-4000-8000-000000000003","kind":"selection.none","params":{}}
{"schema":"appname.command.v1","id":"a10c0004-0000-4000-8000-000000000004","kind":"selection.set","params":{"source":"quick","seed":[10,32]}}
{"schema":"appname.command.v1","id":"a10c0005-0000-4000-8000-000000000005","kind":"fill","target":{"layerId":"$layer"},"params":{"content":"color","color":{"model":"rgb","comps":[0.0,0.0,1.0,1.0]}}}
EOF
# Script B: the two-tone setup ONLY (the pre-quick truth).
head -3 "$WORK/a.jsonl" > "$WORK/b.jsonl"

cp "$WORK/base.appname" "$WORK/a1.appname"
cp "$WORK/base.appname" "$WORK/a2.appname"
cp "$WORK/base.appname" "$WORK/b.appname"

"$CLI" script --doc "$WORK/a1.appname" --file "$WORK/a.jsonl" --json > "$WORK/a1.json"
"$CLI" script --doc "$WORK/a2.appname" --file "$WORK/a.jsonl" --json > "$WORK/a2.json"
"$CLI" script --doc "$WORK/b.appname"  --file "$WORK/b.jsonl" --json > "$WORK/b.json"

EA1="$(exp "$WORK/a1.appname" "$WORK/a1.png")"
EA2="$(exp "$WORK/a2.appname" "$WORK/a2.png")"
EB="$(exp "$WORK/b.appname" "$WORK/b.png")"

# 1. The quick-gated fill changed pixels (the selection flooded the red
#    surface and the fill landed inside it).
if [ "$EA1" != "$EB" ]; then
    echo "quick-gated-fill PASS (export changed by the through-selection fill)"
else
    echo "quick-gated-fill FAIL (quick selection selected nothing?)"
    fail=1
fi

# 2. Two-run determinism, byte-exact (independent processes, same flow).
if [ "$EA1" = "$EA2" ]; then
    echo "two-run-determinism PASS ($EA1)"
else
    echo "two-run-determinism FAIL ($EA1 != $EA2)"
    fail=1
fi

# 3. Undo byte-identity at the export door: undoing the gated fill
#    restores the two-tone truth (the selection surface itself never
#    prints to exports).
"$CLI" undo --doc "$WORK/a1.appname" --json > "$WORK/undo.json"
EU="$(exp "$WORK/a1.appname" "$WORK/u.png")"
if [ "$EU" = "$EB" ]; then
    echo "undo-restore PASS (export == pre-quick two-tone)"
else
    echo "undo-restore FAIL"
    fail=1
fi
"$CLI" redo --doc "$WORK/a1.appname" --json > "$WORK/redo.json"
ER="$(exp "$WORK/a1.appname" "$WORK/r.png")"
if [ "$ER" = "$EA1" ]; then
    echo "redo-restore PASS (export == quick-filled state)"
else
    echo "redo-restore FAIL"
    fail=1
fi

# 4. Subtract-over-wider-prior leg (the O-TOOLS-flagged shape at the
#    workflow tier): select-all, quick-SUBTRACT the red surface, fill
#    green — green must land on the white half only, deterministically.
cat > "$WORK/c.jsonl" <<EOF
{"schema":"appname.command.v1","id":"a10c0011-0000-4000-8000-000000000011","kind":"selection.set","params":{"source":"rect","rect":{"x":0,"y":0,"w":48,"h":64},"antialias":false}}
{"schema":"appname.command.v1","id":"a10c0012-0000-4000-8000-000000000012","kind":"fill","target":{"layerId":"$layer"},"params":{"content":"color","color":{"model":"rgb","comps":[1.0,0.0,0.0,1.0]}}}
{"schema":"appname.command.v1","id":"a10c0013-0000-4000-8000-000000000013","kind":"selection.all","params":{}}
{"schema":"appname.command.v1","id":"a10c0014-0000-4000-8000-000000000014","kind":"selection.set","params":{"source":"quick","seed":[10,32],"op":"subtract"}}
{"schema":"appname.command.v1","id":"a10c0015-0000-4000-8000-000000000015","kind":"fill","target":{"layerId":"$layer"},"params":{"content":"color","color":{"model":"rgb","comps":[0.0,1.0,0.0,1.0]}}}
EOF
cp "$WORK/base.appname" "$WORK/c1.appname"
cp "$WORK/base.appname" "$WORK/c2.appname"
"$CLI" script --doc "$WORK/c1.appname" --file "$WORK/c.jsonl" --json > "$WORK/c1.json"
"$CLI" script --doc "$WORK/c2.appname" --file "$WORK/c.jsonl" --json > "$WORK/c2.json"
EC1="$(exp "$WORK/c1.appname" "$WORK/c1.png")"
EC2="$(exp "$WORK/c2.appname" "$WORK/c2.png")"
if [ "$EC1" = "$EC2" ] && [ "$EC1" != "$EB" ] && [ "$EC1" != "$EA1" ]; then
    echo "subtract-leg PASS (deterministic, distinct from base and add-leg)"
else
    echo "subtract-leg FAIL (EC1=$EC1 EC2=$EC2)"
    fail=1
fi

if [ "$fail" -ne 0 ]; then
    echo "quick-select smoke: FAIL"
    exit 1
fi
echo "quick-select smoke: PASS"
