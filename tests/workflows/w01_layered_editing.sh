#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# w01_layered_editing.sh — workflow gate W01 (runnable subset, LIVE since
# CP-0.5 wave 1): layered editing end-to-end via the CLI — layers (via the
# journaled edit.paste import), fill, undo/redo across the package
# boundary, byte-exact export deltas at every step.
#
# Spec: plan workflow matrix W01; contracts fill.yaml / edit.paste.yaml;
# D-48 (one apply entry; journaled import since CP-0.5 wave 1).
#
# Flow (byte asserts per step):
#   E0 = export of the fresh white doc
#   fill background (content: foreground)          -> E1 != E0
#   import E0.png as a covering top layer          -> E2 == E0 (opaque
#                                                     white covers all)
#   undo (import) -> export == E1 ; undo (fill) -> export == E0
#   redo x2       -> export == E2 ; journal 3/3, layers 2
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

WORK="$(mktemp -d "${TMPDIR:-/tmp}/appname-qa-w01-XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
exp() { "$CLI" export --doc "$pkg" --format png --out "$1" --json > /dev/null; sha "$1"; }
structure() {
    "$CLI" info --doc "$pkg" --json | python3 -c \
        'import json,sys; i=json.load(sys.stdin)["info"]; print(i["journal"]["length"], i["journal"]["cursor"], len(i["layers"]))'
}

fail=0
pkg="$WORK/doc.appname"
"$CLI" new --width 96 --height 64 --background white --out "$pkg" --json \
    > "$WORK/new.json"
layer="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["receipt"]["createdIds"][-1])' "$WORK/new.json")"

E0="$(exp "$WORK/e0.png")"

"$CLI" apply --doc "$pkg" --command \
    "{\"schema\":\"appname.command.v1\",\"id\":\"ad00f001-0000-4000-8000-00000000f001\",\"kind\":\"fill\",\"target\":{\"layerId\":\"$layer\"},\"params\":{\"content\":\"foreground\"}}" \
    --json > "$WORK/fill.json"
E1="$(exp "$WORK/e1.png")"
if [ "$E1" != "$E0" ]; then
    echo "fill-changes-export PASS"
else
    echo "fill-changes-export FAIL"
    fail=1
fi

"$CLI" import --doc "$pkg" --image "$WORK/e0.png" --json \
    > "$WORK/import.json"
E2="$(exp "$WORK/e2.png")"
if [ "$E2" = "$E0" ]; then
    echo "covering-layer-restores-look PASS (E2 == E0 byte-exact)"
else
    echo "covering-layer-restores-look FAIL"
    fail=1
fi

"$CLI" undo --doc "$pkg" --json > "$WORK/undo1.json"
[ "$(exp "$WORK/u1.png")" = "$E1" ] \
    && echo "undo-import PASS (export == E1)" \
    || { echo "undo-import FAIL"; fail=1; }
"$CLI" undo --doc "$pkg" --json > "$WORK/undo2.json"
[ "$(exp "$WORK/u2.png")" = "$E0" ] \
    && echo "undo-fill PASS (export == E0)" \
    || { echo "undo-fill FAIL"; fail=1; }
"$CLI" redo --doc "$pkg" --steps 2 --json > "$WORK/redo.json"
[ "$(exp "$WORK/r2.png")" = "$E2" ] \
    && echo "redo-both PASS (export == E2)" \
    || { echo "redo-both FAIL"; fail=1; }

s="$(structure)"
if [ "$s" = "3 3 2" ]; then
    echo "structure PASS (journal 3/3, layers 2)"
else
    echo "structure FAIL ($s)"
    fail=1
fi

if [ "$fail" -ne 0 ]; then
    echo "W01 layered-editing: FAIL"
    exit 1
fi
echo "W01 layered-editing: PASS"
