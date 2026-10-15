#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# caf_current_layer_e2e.sh — content_aware_fill output:'current_layer'
# end-to-end over the SHIPPED CLI (O-QA CP-1.0 ratification batch,
# W-additions item): blemish setup → rect hole (the live selection IS
# the hole H, §45.3) → CAF → export; TWO-RUN DETERMINISM byte-exact
# across independent processes (the §45.3 engine is a pure function of
# (inputs, seed) over the ratified seeded stream — the layout O-QA
# ratified 2026-08-16, guarded live by caf_stream_conformance); undo/
# redo byte-identity at the export door.
#
# Note the script boundary: the package does NOT persist the selection
# surface (recorded CLI truth), so the hole-selection and the CAF ride
# ONE `script` invocation.
#
# Spec: ES §45.3; contracts content_aware_fill.yaml; D-48; plan §2.5
# O-QA row (workflow gates).
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

WORK="$(mktemp -d "${TMPDIR:-/tmp}/appname-qa-caf-XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
exp() {
    "$CLI" export --doc "$1" --format png --out "$2" --json > /dev/null
    sha "$2"
}

fail=0

"$CLI" new --width 96 --height 64 --background white --out "$WORK/base.appname" \
    --json > "$WORK/new.json"
layer="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["receipt"]["createdIds"][-1])' "$WORK/new.json")"

# Full flow: red field, blue 12×12 blemish (via rect selection + fill,
# which leaves the blemish rect selected = the hole H), CAF
# current_layer with a pinned seed.
cat > "$WORK/full.jsonl" <<EOF
{"schema":"appname.command.v1","id":"caf00001-0000-4000-8000-000000000001","kind":"fill","target":{"layerId":"$layer"},"params":{"content":"color","color":{"model":"rgb","comps":[1.0,0.0,0.0,1.0]}}}
{"schema":"appname.command.v1","id":"caf00002-0000-4000-8000-000000000002","kind":"selection.set","params":{"source":"rect","rect":{"x":40,"y":24,"w":12,"h":12},"antialias":false}}
{"schema":"appname.command.v1","id":"caf00003-0000-4000-8000-000000000003","kind":"fill","target":{"layerId":"$layer"},"params":{"content":"color","color":{"model":"rgb","comps":[0.0,0.0,1.0,1.0]}}}
{"schema":"appname.command.v1","id":"caf00004-0000-4000-8000-000000000004","kind":"content_aware_fill","target":{"layerId":"$layer"},"params":{"output":"current_layer","seed":0}}
EOF
# Blemish-only flow: the pre-CAF truth.
head -3 "$WORK/full.jsonl" > "$WORK/blemish.jsonl"

cp "$WORK/base.appname" "$WORK/f1.appname"
cp "$WORK/base.appname" "$WORK/f2.appname"
cp "$WORK/base.appname" "$WORK/pre.appname"

"$CLI" script --doc "$WORK/f1.appname" --file "$WORK/full.jsonl" --json > "$WORK/f1.json"
"$CLI" script --doc "$WORK/f2.appname" --file "$WORK/full.jsonl" --json > "$WORK/f2.json"
"$CLI" script --doc "$WORK/pre.appname" --file "$WORK/blemish.jsonl" --json > "$WORK/pre.json"

EF1="$(exp "$WORK/f1.appname" "$WORK/f1.png")"
EF2="$(exp "$WORK/f2.appname" "$WORK/f2.png")"
EPRE="$(exp "$WORK/pre.appname" "$WORK/pre.png")"

# 1. CAF changed pixels (the blemish was synthesized over).
if [ "$EF1" != "$EPRE" ]; then
    echo "caf-heals PASS (export changed by the fill)"
else
    echo "caf-heals FAIL (CAF was a no-op)"
    fail=1
fi

# 2. Two-run determinism byte-exact across processes (§45.3 + the
#    ratified seeded-stream layout; seed pinned in the envelope).
if [ "$EF1" = "$EF2" ]; then
    echo "two-run-determinism PASS ($EF1)"
else
    echo "two-run-determinism FAIL ($EF1 != $EF2)"
    fail=1
fi

# 3. Pixel-level "healed from the red surround" witnesses live at the
#    conformance tier (goldens/caf + the in-crate caf_kind_tests), where
#    pixels are readable directly; this workflow smoke asserts the
#    export-level truths (changed, deterministic, undo-exact) honestly
#    rather than shelling out a PNG decoder here.

# 4. Undo byte-identity at the export door: undoing the CAF restores
#    the blemish truth; redo re-lands it.
"$CLI" undo --doc "$WORK/f1.appname" --json > "$WORK/undo.json"
EU="$(exp "$WORK/f1.appname" "$WORK/u.png")"
if [ "$EU" = "$EPRE" ]; then
    echo "undo-restore PASS (export == pre-CAF blemish)"
else
    echo "undo-restore FAIL"
    fail=1
fi
"$CLI" redo --doc "$WORK/f1.appname" --json > "$WORK/redo.json"
ER="$(exp "$WORK/f1.appname" "$WORK/r.png")"
if [ "$ER" = "$EF1" ]; then
    echo "redo-restore PASS (export == healed state)"
else
    echo "redo-restore FAIL"
    fail=1
fi

# 5. Journal truth: 5 rows journaled (doc.new + three setup + the CAF),
#    and the saved package replays to the same export on reopen (exp()
#    above already reopened the package in a fresh process each time —
#    the byte-identity legs ARE the replay evidence).
jlen="$("$CLI" info --doc "$WORK/f1.appname" --json | python3 -c \
    'import json,sys; print(json.load(sys.stdin)["info"]["journal"]["length"])')"
if [ "$jlen" = "5" ]; then
    echo "journal PASS (5 rows: doc.new + 3 setup + CAF)"
else
    echo "journal FAIL (length $jlen != 5)"
    fail=1
fi

if [ "$fail" -ne 0 ]; then
    echo "CAF current_layer e2e: FAIL"
    exit 1
fi
echo "CAF current_layer e2e: PASS"
