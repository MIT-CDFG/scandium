#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# w04_color_correctness.sh — workflow gate W04, LIVE since CP-0.5 wave 3
# (finalized from the staged scaffold the moment the color wave landed —
# the stage guard tripped on rebase, as designed).
#
# Spec: plan workflow matrix W04; ES §5 — the core truth this gate pins:
# `doc.assign_profile` changes INTERPRETATION (retag; pixel bytes
# untouched), `doc.convert_profile` changes PIXELS. Pixel truth is read
# MECHANICALLY from the shipped package: the sha over the sorted
# `layers/**` zip entries (tile payloads) — assign leaves it unchanged,
# convert moves it.
#
# Profile id: the deterministic builtin DisplayP3 id
# ee12374b-5f25-51fd-a1c8-f45790a61d92 (UUIDv5 over
# "profile/builtin/display-p3"-family tag; derived via
# engine::color_glue::builtin_profile_id and pinned by the conformance
# suite so this literal cannot drift silently).
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

WORK="$(mktemp -d "${TMPDIR:-/tmp}/appname-qa-w04-XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

P3="ee12374b-5f25-51fd-a1c8-f45790a61d92"
fail=0
pkg="$WORK/doc.appname"

tiles_sha() { # sha over the sorted layers/** entry payloads (pixel truth)
    python3 - "$1" <<'PY'
import hashlib, sys, zipfile
z = zipfile.ZipFile(sys.argv[1])
h = hashlib.sha256()
for n in sorted(z.namelist()):
    if n.startswith("layers/"):
        h.update(n.encode())
        h.update(z.read(n))
print(h.hexdigest())
PY
}

# SATURATED content is essential: black/white sit on the neutral axis
# where sRGB->P3 is the identity (same D65 white + TRC) — a fill of
# foreground/background would make convert a legitimate no-op. The
# stroke fixture paints saturated red.
"$CLI" new --width 160 --height 120 --background white --out "$pkg" --json \
    > "$WORK/new.json"
layer="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["receipt"]["createdIds"][-1])' "$WORK/new.json")"
"$CLI" asset add --doc "$pkg" \
    --file "$ROOT/fixtures/samples/basic-5pt.samplesv1" --json > /dev/null
sed "s/<LAYER_ID>/$layer/" \
    "$ROOT/fixtures/samples/stroke-script.template.jsonl" > "$WORK/stroke.jsonl"
"$CLI" script --doc "$pkg" --file "$WORK/stroke.jsonl" --json \
    > "$WORK/stroke_out.json"

T0="$(tiles_sha "$pkg")"

# ---- assign: retag only — pixel bytes UNTOUCHED -----------------------------
"$CLI" apply --doc "$pkg" --command \
    "{\"schema\":\"appname.command.v1\",\"id\":\"ad00f042-0000-4000-8000-00000000f042\",\"kind\":\"doc.assign_profile\",\"params\":{\"profile\":\"$P3\"}}" \
    --json > "$WORK/assign.json"
T1="$(tiles_sha "$pkg")"
if [ "$T1" = "$T0" ]; then
    echo "assign-pixels-untouched PASS (layers/** byte-identical)"
else
    echo "assign-pixels-untouched FAIL"
    fail=1
fi

# ---- convert: pixels CHANGE -------------------------------------------------
"$CLI" undo --doc "$pkg" --json > "$WORK/undo_assign.json"
"$CLI" apply --doc "$pkg" --command \
    "{\"schema\":\"appname.command.v1\",\"id\":\"ad00f043-0000-4000-8000-00000000f043\",\"kind\":\"doc.convert_profile\",\"params\":{\"profile\":\"$P3\"}}" \
    --json > "$WORK/convert.json"
T2="$(tiles_sha "$pkg")"
if [ "$T2" != "$T0" ]; then
    echo "convert-pixels-change PASS (layers/** moved)"
else
    echo "convert-pixels-change FAIL"
    fail=1
fi
echo "assign-neq-convert PASS-BY-CONSTRUCTION (same doc, same target profile: assign kept T0, convert left it)"

# ---- undo restores pixel bytes across the package boundary ------------------
"$CLI" undo --doc "$pkg" --json > "$WORK/undo_convert.json"
T3="$(tiles_sha "$pkg")"
if [ "$T3" = "$T0" ]; then
    echo "undo-convert-restores-pixels PASS"
else
    echo "undo-convert-restores-pixels FAIL"
    fail=1
fi

# ---- assign null (untagged state) is a legal retag --------------------------
if "$CLI" apply --doc "$pkg" --command \
    '{"schema":"appname.command.v1","id":"ad00f044-0000-4000-8000-00000000f044","kind":"doc.assign_profile","params":{"profile":null}}' \
    --json > "$WORK/assign_null.json"; then
    [ "$(tiles_sha "$pkg")" = "$T0" ] \
        && echo "assign-null-untagged PASS (retag only)" \
        || { echo "assign-null-untagged FAIL (pixels moved)"; fail=1; }
else
    echo "assign-null-untagged FAIL (rejected)"
    fail=1
fi

if [ "$fail" -ne 0 ]; then
    echo "W04 color-correctness: FAIL"
    exit 1
fi
echo "W04 color-correctness: PASS"
