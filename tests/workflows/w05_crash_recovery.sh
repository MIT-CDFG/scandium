#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# w05_crash_recovery.sh — workflow gate W05, LIVE since CP-0.5 wave 3
# (autosave + recovery landed; scaffold activated per the staged plan).
#
# Spec: plan workflow matrix W05; ES §7.8 (autosave journal framing,
# torn-tail never fatal), §10.6 (recovery); CLI `script --autosave` +
# `recover` + the deterministic abort door
# APPNAME_FI_ABORT_AFTER_APPLIES (fault injection: process aborts after
# N applies, no save, no destructors).
#
# Flow: 6-line fill script; abort after 3. Assert: the base package's
# bytes are untouched by the aborted run; `recover` replays exactly the
# 3 autosaved commands; the recovered package matches a CLEAN 3-line
# reference run byte-for-byte on export and structurally on the journal.
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

WORK="$(mktemp -d "${TMPDIR:-/tmp}/appname-qa-w05-XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
jlen() {
    "$CLI" info --doc "$1" --json | python3 -c \
        'import json,sys; print(json.load(sys.stdin)["info"]["journal"]["length"])'
}

fail=0
pkg="$WORK/doc.appname"
"$CLI" new --width 64 --height 48 --background white --out "$pkg" --json \
    > "$WORK/new.json"
layer="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["receipt"]["createdIds"][-1])' "$WORK/new.json")"

# 6 alternating fills (distinct fixed ids; each journals)
: > "$WORK/six.jsonl"
for i in 1 2 3 4 5 6; do
    content=$([ $((i % 2)) -eq 1 ] && echo foreground || echo background)
    printf '{"schema":"appname.command.v1","id":"ad00f05%d-0000-4000-8000-00000000f05%d","kind":"fill","target":{"layerId":"%s"},"params":{"content":"%s"}}\n' \
        "$i" "$i" "$layer" "$content" >> "$WORK/six.jsonl"
done
head -3 "$WORK/six.jsonl" > "$WORK/three.jsonl"

# clean 3-line reference run
cp "$pkg" "$WORK/ref.appname"
"$CLI" script --doc "$WORK/ref.appname" --file "$WORK/three.jsonl" --json \
    > "$WORK/ref_script.json"
"$CLI" export --doc "$WORK/ref.appname" --format png --out "$WORK/ref.png" \
    --json > /dev/null
REF_J="$(jlen "$WORK/ref.appname")"

# aborted run: autosave + fault door after 3 of 6 applies
pre="$(sha "$pkg")"
rc=0
APPNAME_FI_ABORT_AFTER_APPLIES=3 "$CLI" script --doc "$pkg" \
    --file "$WORK/six.jsonl" --autosave "$WORK/auto.log" --json \
    > "$WORK/aborted.json" 2>&1 || rc=$?
if [ "$rc" -ne 0 ]; then
    echo "abort-door PASS (script aborted, exit $rc)"
else
    echo "abort-door FAIL (script completed despite the fault door)"
    fail=1
fi
if [ "$(sha "$pkg")" = "$pre" ]; then
    echo "base-untouched PASS (aborted run persisted nothing to the package)"
else
    echo "base-untouched FAIL"
    fail=1
fi

# recovery: replay the autosaved 3 onto the base
"$CLI" recover --doc "$pkg" --log "$WORK/auto.log" \
    --out "$WORK/recovered.appname" --json > "$WORK/recover.json"
REC_J="$(jlen "$WORK/recovered.appname")"
if [ "$REC_J" = "$REF_J" ]; then
    echo "recover-journal PASS (length $REC_J == clean reference)"
else
    echo "recover-journal FAIL ($REC_J != $REF_J)"
    fail=1
fi
"$CLI" export --doc "$WORK/recovered.appname" --format png \
    --out "$WORK/rec.png" --json > /dev/null
if [ "$(sha "$WORK/rec.png")" = "$(sha "$WORK/ref.png")" ]; then
    echo "recover-state PASS (export byte-identical to the clean 3-line run)"
else
    echo "recover-state FAIL"
    fail=1
fi

if [ "$fail" -ne 0 ]; then
    echo "W05 crash-recovery: FAIL"
    exit 1
fi
echo "W05 crash-recovery: PASS"
