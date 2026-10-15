#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# replay_determinism.sh — determinism-replay runner, LIVE at wave 4 (plan
# §2.1 "Test harness v1": determinism replay, byte-identity via CLI on the
# saved package).
#
# Spec: determinism spine D-48; ES §7.8/§10.6 (journal replay), §10.4
# (undo/redo); proto evidence REPORT-A F10; O-CORE slice-3 selftest (script
# → save → reopen → replay → identical state_hash, plus an independent
# second run with byte-identical package AND export).
#
# Checks:
#   selftest        the CLI's own 5-check determinism proof (script → save
#                   → reopen → replay → state_hash identity; independent
#                   second run byte-identity) must PASS — this is the
#                   state_hash leg of the runner
#   save-load-save  per run: `script` with an empty ops file rewrites the
#                   package; bytes before == after (§7.6 determinism)
#   walk-reject     per run: undo/redo past the boundary reject (exit 1)
#                   and leave the package bytes untouched (doc.new cannot
#                   be undone across the package boundary — CLI contract)
#   render-identity across TWO INDEPENDENT runs: exported PNG byte-equal
#   structure       across runs: journal length/cursor + geometry equal
#
# NOTE on package bytes across runs: `new` mints a fresh envelope uuid per
# invocation (CLI surface), so whole-package byte equality across runs is
# NOT expected through `new`; the fixed-id byte-identity proof is
# selftest's independent-run check, which this runner requires green.
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

WORK="$(mktemp -d "${TMPDIR:-/tmp}/appname-qa-replay-XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

sha() { shasum -a 256 "$1" | cut -d' ' -f1; }

fail=0
note() { echo "$1"; }

# ---- selftest: the state_hash replay proof ---------------------------------
if "$CLI" selftest > "$WORK/selftest.txt" 2>&1; then
    note "selftest PASS ($(tail -1 "$WORK/selftest.txt"))"
else
    note "selftest FAIL"
    sed 's/^/    /' "$WORK/selftest.txt"
    fail=1
fi

declare -a EXPORT_SHA STRUCTURE

for run in 1 2; do
    dir="$WORK/run$run"
    mkdir -p "$dir"
    pkg="$dir/doc.appname"

    "$CLI" new --width 96 --height 64 --background white \
        --out "$pkg" --json > "$dir/new.json"

    # save-load-save byte identity via an empty transactional script
    printf '# empty ops — save/load/save byte probe\n' > "$dir/empty.jsonl"
    before="$(sha "$pkg")"
    "$CLI" script --doc "$pkg" --file "$dir/empty.jsonl" --json \
        > "$dir/script.json"
    after="$(sha "$pkg")"
    if [ "$before" = "$after" ]; then
        note "save-load-save run$run PASS ($after)"
    else
        note "save-load-save run$run FAIL ($before != $after)"
        fail=1
    fi

    # boundary walk: undo/redo must reject and leave bytes untouched
    walk_ok=1
    set +e
    "$CLI" undo --doc "$pkg" --steps 999 --json > "$dir/undo.json"
    [ $? -eq 1 ] || walk_ok=0
    "$CLI" redo --doc "$pkg" --steps 999 --json > "$dir/redo.json"
    [ $? -eq 1 ] || walk_ok=0
    set -e
    if [ "$walk_ok" -eq 1 ] && [ "$(sha "$pkg")" = "$after" ]; then
        note "walk-reject run$run PASS (exit 1 both ways, bytes untouched)"
    else
        note "walk-reject run$run FAIL"
        fail=1
    fi

    "$CLI" export --doc "$pkg" --format png --out "$dir/out.png" --json \
        > "$dir/export.json"
    EXPORT_SHA[$run]="$(sha "$dir/out.png")"

    STRUCTURE[$run]="$("$CLI" info --doc "$pkg" --json | python3 -c '
import json, sys
i = json.load(sys.stdin)["info"]
print(i["journal"]["length"], i["journal"]["cursor"], i["width"], i["height"], len(i["layers"]))')"
done

if [ "${EXPORT_SHA[1]}" = "${EXPORT_SHA[2]}" ]; then
    note "render-identity PASS (${EXPORT_SHA[1]})"
else
    note "render-identity FAIL (${EXPORT_SHA[1]} != ${EXPORT_SHA[2]})"
    fail=1
fi

if [ "${STRUCTURE[1]}" = "${STRUCTURE[2]}" ]; then
    note "structure PASS (${STRUCTURE[1]})"
else
    note "structure FAIL (${STRUCTURE[1]} != ${STRUCTURE[2]})"
    fail=1
fi

# ---- stroke+asset leg (LIVE since CP-0.3 slice B; arbitrations #1/#4) ------
# Fixed inputs: fixtures/samples/basic-5pt.samplesv1 (asset id is
# content-derived, baked into stroke-script.template.jsonl) + the built-in
# hard-round-16 preset. Cross-run whole-PACKAGE identity stays a non-goal
# (`new` mints a fresh doc.new envelope uuid per run); the byte-identity
# legs are: cross-INVOCATION save-load-save on the stroked package, and
# cross-run EXPORT identity — plus undo/redo across the package boundary
# (replay-as-recovery now replays the stroke + asset reference).
declare -a STROKE_EXPORT_SHA
for run in 1 2; do
    dir="$WORK/run$run"
    pkg="$dir/doc.appname"
    base_png="$dir/out.png"           # pre-stroke export from the leg above

    aid="$("$CLI" asset add --doc "$pkg" \
        --file "$ROOT/fixtures/samples/basic-5pt.samplesv1" --json \
        | python3 -c 'import json,sys; print(json.load(sys.stdin)["assetId"])')"
    grep -q "$aid" "$ROOT/fixtures/samples/stroke-script.template.jsonl" || {
        echo "stroke-asset run$run FAIL (content-derived assetId $aid not in template)"
        fail=1
        continue
    }
    layer="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["receipt"]["createdIds"][-1])' "$dir/new.json")"
    sed "s/<LAYER_ID>/$layer/" \
        "$ROOT/fixtures/samples/stroke-script.template.jsonl" \
        > "$dir/stroke.jsonl"
    "$CLI" script --doc "$pkg" --file "$dir/stroke.jsonl" --json \
        > "$dir/stroke_script.json"

    "$CLI" export --doc "$pkg" --format png --out "$dir/stroked.png" --json \
        > "$dir/stroked_export.json"
    if [ "$(sha "$dir/stroked.png")" = "$(sha "$base_png")" ]; then
        echo "stroke-export-change run$run FAIL (stroke did not change the export)"
        fail=1
    else
        echo "stroke-export-change run$run PASS"
    fi

    # cross-invocation byte identity with stroke + asset aboard
    before="$(sha "$pkg")"
    "$CLI" script --doc "$pkg" --file "$dir/empty.jsonl" --json \
        > "$dir/script_stroked.json"
    if [ "$(sha "$pkg")" = "$before" ]; then
        echo "stroke-save-load-save run$run PASS ($before)"
    else
        echo "stroke-save-load-save run$run FAIL"
        fail=1
    fi

    # undo/redo across the package boundary (replay-as-recovery replays
    # the stroke and resolves the samples asset)
    "$CLI" undo --doc "$pkg" --json > "$dir/undo_stroke.json"
    "$CLI" export --doc "$pkg" --format png --out "$dir/undone.png" --json \
        > "$dir/undone_export.json"
    "$CLI" redo --doc "$pkg" --json > "$dir/redo_stroke.json"
    "$CLI" export --doc "$pkg" --format png --out "$dir/redone.png" --json \
        > "$dir/redone_export.json"
    if [ "$(sha "$dir/undone.png")" = "$(sha "$base_png")" ] \
        && [ "$(sha "$dir/redone.png")" = "$(sha "$dir/stroked.png")" ]; then
        echo "stroke-undo-redo run$run PASS (undo==pre-stroke, redo==stroked, byte-exact)"
    else
        echo "stroke-undo-redo run$run FAIL"
        fail=1
    fi
    STROKE_EXPORT_SHA[$run]="$(sha "$dir/stroked.png")"
done
if [ "${STROKE_EXPORT_SHA[1]:-a}" = "${STROKE_EXPORT_SHA[2]:-b}" ]; then
    echo "stroke-render-identity PASS (${STROKE_EXPORT_SHA[1]})"
else
    echo "stroke-render-identity FAIL (${STROKE_EXPORT_SHA[1]:-?} != ${STROKE_EXPORT_SHA[2]:-?})"
    fail=1
fi

# ---- shared-base whole-PACKAGE byte-identity leg (aud-replay F3) -----------
# The legs above mint `new` PER RUN, so the doc.new envelope uuid differs
# across runs and whole-package bytes are (correctly) a documented non-goal
# there — only export + save-load-save + structure are compared. F3's
# insight: mint the base ONCE and `cp` it into both run dirs. Now the
# envelope uuid AND the created layer id are shared, so an identical
# deterministic op sequence (same fixed samples asset + stroke template)
# must produce BYTE-IDENTICAL PACKAGES across two independent CLI processes.
# This GATES the strongest determinism property the engine has — cross-run
# whole-package byte identity — which held but was untested (F3). The audit
# proved it across its 288-seed sweep; here it is one `cp` + the assertion
# on the runner's fixed deterministic input. Packages carry fixed-epoch
# timestamps and never mint envelope timestamps (io/package.rs), so no
# wall-clock leak can defeat it.
sb_base="$WORK/sharedbase.appname"
"$CLI" new --width 96 --height 64 --background white --out "$sb_base" --json \
    > "$WORK/sharedbase.new.json"
sb_layer="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["receipt"]["createdIds"][-1])' "$WORK/sharedbase.new.json")"
sed "s/<LAYER_ID>/$sb_layer/" \
    "$ROOT/fixtures/samples/stroke-script.template.jsonl" > "$WORK/sharedbase.stroke.jsonl"

declare -a SB_PKG_SHA SB_EXPORT_SHA
for run in 1 2; do
    sbdir="$WORK/sb-run$run"
    mkdir -p "$sbdir"
    sbpkg="$sbdir/shared.appname"
    cp "$sb_base" "$sbpkg"            # THE single mint, shared across runs
    "$CLI" asset add --doc "$sbpkg" \
        --file "$ROOT/fixtures/samples/basic-5pt.samplesv1" --json \
        > "$sbdir/asset.json"
    "$CLI" script --doc "$sbpkg" --file "$WORK/sharedbase.stroke.jsonl" --json \
        > "$sbdir/script.json"
    "$CLI" export --doc "$sbpkg" --format png --out "$sbdir/out.png" --json \
        > "$sbdir/export.json"
    SB_PKG_SHA[$run]="$(sha "$sbpkg")"
    SB_EXPORT_SHA[$run]="$(sha "$sbdir/out.png")"
done
if [ "${SB_PKG_SHA[1]}" = "${SB_PKG_SHA[2]}" ]; then
    note "shared-base package-identity PASS (${SB_PKG_SHA[1]})"
else
    note "shared-base package-identity FAIL (${SB_PKG_SHA[1]} != ${SB_PKG_SHA[2]})"
    fail=1
fi
if [ "${SB_EXPORT_SHA[1]}" = "${SB_EXPORT_SHA[2]}" ]; then
    note "shared-base export-identity PASS (${SB_EXPORT_SHA[1]})"
else
    note "shared-base export-identity FAIL (${SB_EXPORT_SHA[1]} != ${SB_EXPORT_SHA[2]})"
    fail=1
fi

if [ "$fail" -ne 0 ]; then
    echo "replay_determinism: FAIL"
    exit 1
fi
echo "replay_determinism: PASS"
