#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# package_roundtrip.sh — package round-trip runner, LIVE at wave 4 (plan
# §2.1 "Test harness v1").
#
# Spec: ES §7.6 (deterministic package writer; save→load→save byte-stable),
# §7.8 (journal in package); determinism spine D-48; proto evidence
# REPORT-A §4.2 (rejections leave package bytes untouched).
#
# Checks:
#   save-load-save  `script` with an empty ops file = load + save; package
#                   bytes before == after (the round-trip core)
#   reopen          a byte-copied package reopens; journal/geometry equal
#   render-equal    exports from original and copy byte-identical
#   reject-intact   a rejected envelope (unknown kind, pinned
#                   error.command.unknown_kind) leaves bytes untouched
#   import-stage    import an exported PNG (LIVE at CP-0.1: an IO call
#                   outside the journal): journal length unchanged, layer
#                   count 2, and the imported package still round-trips
#                   byte-stable through save-load-save
#
# Exit: 0 all checks pass · 1 a check failed · 2 environment not ready.

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

WORK="$(mktemp -d "${TMPDIR:-/tmp}/appname-qa-roundtrip-XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
structure() {
    "$CLI" info --doc "$1" --json | python3 -c '
import json, sys
i = json.load(sys.stdin)["info"]
print(i["journal"]["length"], i["journal"]["cursor"], i["width"], i["height"], len(i["layers"]))'
}

fail=0
pkg="$WORK/doc.appname"
copy="$WORK/copy.appname"
empty="$WORK/empty.jsonl"
printf '# empty ops — save/load/save byte probe\n' > "$empty"

"$CLI" new --width 96 --height 64 --background white --out "$pkg" --json \
    > "$WORK/new.json"

# ---- save-load-save ---------------------------------------------------------
before="$(sha "$pkg")"
"$CLI" script --doc "$pkg" --file "$empty" --json > "$WORK/script1.json"
after="$(sha "$pkg")"
if [ "$before" = "$after" ]; then
    echo "save-load-save PASS ($after)"
else
    echo "save-load-save FAIL ($before != $after)"
    fail=1
fi

# ---- reopen -----------------------------------------------------------------
cp "$pkg" "$copy"
orig_s="$(structure "$pkg")"
if copy_s="$(structure "$copy")" && [ "$orig_s" = "$copy_s" ]; then
    echo "reopen PASS ($copy_s)"
else
    echo "reopen FAIL (orig '$orig_s' vs copy '${copy_s:-<info failed>}')"
    fail=1
fi

# ---- render-equal -----------------------------------------------------------
"$CLI" export --doc "$pkg" --format png --out "$WORK/a.png" --json \
    > "$WORK/exp_a.json"
"$CLI" export --doc "$copy" --format png --out "$WORK/b.png" --json \
    > "$WORK/exp_b.json"
if [ "$(sha "$WORK/a.png")" = "$(sha "$WORK/b.png")" ]; then
    echo "render-equal PASS ($(sha "$WORK/a.png"))"
else
    echo "render-equal FAIL"
    fail=1
fi

# ---- reject-intact ----------------------------------------------------------
before="$(sha "$pkg")"
set +e
"$CLI" apply --doc "$pkg" --command \
    '{"schema":"appname.command.v1","id":"e0e0e0e0-0000-4000-8000-0000000000ff","kind":"doc.does_not_exist","params":{}}' \
    --json > "$WORK/reject.json"
rc=$?
set -e
code="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["error"]["code"])' "$WORK/reject.json")"
if [ "$rc" -eq 1 ] && [ "$code" = "error.command.unknown_kind" ] \
    && [ "$(sha "$pkg")" = "$before" ]; then
    echo "reject-intact PASS (exit 1, $code, bytes unchanged)"
else
    echo "reject-intact FAIL (exit $rc, code $code)"
    fail=1
fi

# ---- import-stage (LIVE; JOURNALED since CP-0.5 wave 1) ---------------------
# `import` now applies edit.paste through the one entry (import_layer
# retired — the last D-48 exception gone): the journal GROWS by one, the
# import is undoable, and replayability survives.
pre_s="$(structure "$pkg")"
if "$CLI" import --doc "$pkg" --image "$WORK/a.png" --json \
    > "$WORK/import.json"; then
    post_s="$(structure "$pkg")"
    pre_j="$(echo "$pre_s" | cut -d' ' -f1)"
    post_j="$(echo "$post_s" | cut -d' ' -f1-2)"
    layers="$(echo "$post_s" | awk '{print $5}')"
    imp_ok=1
    [ "$post_j" = "$((pre_j + 1)) $((pre_j + 1))" ] || imp_ok=0  # journaled
    [ "$layers" = "2" ] || imp_ok=0
    # imported package still round-trips byte-stable
    before="$(sha "$pkg")"
    "$CLI" script --doc "$pkg" --file "$empty" --json > "$WORK/script2.json"
    [ "$(sha "$pkg")" = "$before" ] || imp_ok=0
    # the journaled import is UNDOABLE across the package boundary (the
    # capability the retirement bought); redo restores it
    "$CLI" undo --doc "$pkg" --json > "$WORK/undo_import.json" || imp_ok=0
    [ "$(structure "$pkg" | awk '{print $5}')" = "1" ] || imp_ok=0
    "$CLI" redo --doc "$pkg" --json > "$WORK/redo_import.json" || imp_ok=0
    [ "$(structure "$pkg" | awk '{print $5}')" = "2" ] || imp_ok=0
    if [ "$imp_ok" -eq 1 ]; then
        echo "import-stage PASS (journaled: $post_j, layers $layers, undo/redo across boundary, save-load-save stable)"
    else
        echo "import-stage FAIL (journal '$pre_s'->'$post_s', layers $layers)"
        fail=1
    fi
else
    echo "import-stage FAIL (import exited $?)"
    fail=1
fi

# ---- samples-asset round-trip leg (LIVE since CP-0.3 slice B; arb. #4) ------
has_asset() { # has_asset <pkg> <assetId> -> exit 0 iff assets/<id> packaged
    python3 - "$1" "$2" <<'PY'
import sys, zipfile
names = zipfile.ZipFile(sys.argv[1]).namelist()
sys.exit(0 if f"assets/{sys.argv[2]}" in names else 1)
PY
}

# A FRESH package for this leg: the import-stage leg above deliberately
# broke journal replayability on $pkg (imports live outside the journal),
# so undo there refuses by contract — the GC probes need a clean journal.
apkg="$WORK/asset.appname"
"$CLI" new --width 96 --height 64 --background white --out "$apkg" --json \
    > "$WORK/asset_new.json"
aid="$("$CLI" asset add --doc "$apkg" \
    --file "$ROOT/fixtures/samples/basic-5pt.samplesv1" --json \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["assetId"])')"
layer="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["receipt"]["createdIds"][-1])' "$WORK/asset_new.json")"
sed "s/<LAYER_ID>/$layer/" \
    "$ROOT/fixtures/samples/stroke-script.template.jsonl" > "$WORK/stroke.jsonl"
"$CLI" script --doc "$apkg" --file "$WORK/stroke.jsonl" --json \
    > "$WORK/stroke_script.json"

# save-load-save byte identity with the stroke + asset aboard
before="$(sha "$apkg")"
"$CLI" script --doc "$apkg" --file "$empty" --json > "$WORK/script3.json"
if [ "$(sha "$apkg")" = "$before" ] && has_asset "$apkg" "$aid"; then
    echo "asset-save-load-save PASS (asset $aid aboard, bytes stable)"
else
    echo "asset-save-load-save FAIL"
    fail=1
fi

# byte-copy reopen + render-equal on the stroked composite
cp "$apkg" "$WORK/stroked-copy.appname"
"$CLI" export --doc "$apkg" --format png --out "$WORK/s1.png" --json \
    > "$WORK/exp_s1.json"
"$CLI" export --doc "$WORK/stroked-copy.appname" --format png \
    --out "$WORK/s2.png" --json > "$WORK/exp_s2.json"
if [ "$(sha "$WORK/s1.png")" = "$(sha "$WORK/s2.png")" ]; then
    echo "asset-render-equal PASS ($(sha "$WORK/s1.png"))"
else
    echo "asset-render-equal FAIL"
    fail=1
fi

# GC probe: undone-tail RETENTION (the core-review-verified case), then
# collection once the tail is truncated
"$CLI" undo --doc "$apkg" --json > "$WORK/undo_gc.json"
if has_asset "$apkg" "$aid"; then
    echo "asset-gc-retention PASS (stroke undone but journal-retained -> asset persists)"
else
    echo "asset-gc-retention FAIL (asset collected while its undone entry is still journaled)"
    fail=1
fi
# import truncates the undone tail (outside the journal, proto rule) ->
# the stroke leaves the journal -> the asset is unreferenced -> collected
"$CLI" import --doc "$apkg" --image "$WORK/s1.png" --json \
    > "$WORK/import_gc.json"
if has_asset "$apkg" "$aid"; then
    echo "asset-gc-collect FAIL (unreferenced asset survived the truncating save)"
    fail=1
else
    echo "asset-gc-collect PASS (tail truncated -> asset collected)"
fi

if [ "$fail" -ne 0 ]; then
    echo "package_roundtrip: FAIL"
    exit 1
fi
echo "package_roundtrip: PASS"
