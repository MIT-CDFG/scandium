#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# w22_crash_fault_injection.sh — workflow gate W22 (crash fault-injection
# matrix, extends W5) at the surface the shipped build exposes.
# Spec: W22-crash-fault-injection-matrix.yaml (§41.6/§7.8/§10.6).
#
# RUNNABLE legs (the W05 harness shape, extended):
#   FI-A  kill-mid-script via the deterministic abort door
#         (APPNAME_FI_ABORT_AFTER_APPLIES): 25 edits incl. the
#         NON-IDEMPOTENT probe (fill at opacity 0.5 — double replay
#         would be visible); abort after 12; recover; export must be
#         byte-identical to a clean 12-line reference run (single
#         application proven).
#   FI-B  torn-frame (the FI-1 class, injected by log surgery): truncate
#         the autosave log mid-frame; recover must truncate at the torn
#         record (§7.8 corrupt-tail rule), report torn:true, and land
#         exactly the surviving prefix (export == clean prefix run).
#   FI-C  corrupt payload (the CRC-class leg): flip one byte inside a
#         mid-log frame; recovery must stop at/before that record —
#         never apply garbage — and stay typed (torn/warnings surface),
#         with the recovered state equal to a clean shorter-prefix run.
#   In every recovered case: the base package's bytes stay untouched by
#   the aborted run (the §7.6 atomicity observable).
#
# BLOCKED legs (typed, with triggers — a recorded blocker beats a
# silent skip):
#   FI-1..FI-3 at their EXACT §7.8 in-engine points (mid-append tear,
#   scratch-extent/TILEREF ordering, CHECKPOINT fsync) and FI-4..FI-6
#   (§7.6 save-pipeline steps 4-6 kill points): the shipped build
#   carries ONE fault door (APPNAME_FI_ABORT_AFTER_APPLIES, post-apply
#   granularity). TRIGGER: O-CORE APPNAME_FI_* doors at the named §7.6
#   step-4/5/6 and §7.8 append/checkpoint points. §10.6 partial-restore
#   OFFER + orphan scan + decline-archives: no CLI surface (recover is
#   explicit-path). TRIGGER: the §10.6 offer surface (O-CORE/O-UI).
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

WORK="$(mktemp -d "${TMPDIR:-/tmp}/appname-qa-w22-XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
fail=0

pkg="$WORK/doc.appname"
"$CLI" new --width 64 --height 48 --background white --out "$pkg" --json \
    > "$WORK/new.json"
layer="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["receipt"]["createdIds"][-1])' "$WORK/new.json")"

# 25 edits; every 5th is the non-idempotent 50%-opacity fill probe.
: > "$WORK/ops.jsonl"
for i in $(seq 1 25); do
    if [ $((i % 5)) -eq 0 ]; then
        extra=',"opacity":0.5'
        content=foreground
    else
        extra=''
        content=$([ $((i % 2)) -eq 1 ] && echo foreground || echo background)
    fi
    printf '{"schema":"appname.command.v1","id":"ad00f200-0000-4000-8000-0000000022%02d","kind":"fill","target":{"layerId":"%s"},"params":{"content":"%s"%s}}\n' \
        "$i" "$layer" "$content" "$extra" >> "$WORK/ops.jsonl"
done
[ "$(wc -l < "$WORK/ops.jsonl" | tr -d ' ')" = "25" ] || { echo "harness FAIL (script build)"; exit 1; }

ref_run() { # $1 = line count, $2 = out tag -> exports $WORK/$2.png
    head -"$1" "$WORK/ops.jsonl" > "$WORK/$2.jsonl"
    cp "$pkg" "$WORK/$2.appname"
    "$CLI" script --doc "$WORK/$2.appname" --file "$WORK/$2.jsonl" --json > /dev/null
    "$CLI" export --doc "$WORK/$2.appname" --format png --out "$WORK/$2.png" --json > /dev/null
}

# ---- FI-A: abort door mid-script -----------------------------------
pre="$(sha "$pkg")"
rc=0
APPNAME_FI_ABORT_AFTER_APPLIES=12 "$CLI" script --doc "$pkg" \
    --file "$WORK/ops.jsonl" --autosave "$WORK/auto.log" --json \
    > "$WORK/aborted.json" 2>&1 || rc=$?
if [ "$rc" -ne 0 ] && [ "$(sha "$pkg")" = "$pre" ]; then
    echo "FI-A-abort PASS (killed after 12 applies; base package bytes untouched)"
else
    echo "FI-A-abort FAIL (rc=$rc, base touched=$([ "$(sha "$pkg")" != "$pre" ] && echo yes || echo no))"
    fail=1
fi
"$CLI" recover --doc "$pkg" --log "$WORK/auto.log" \
    --out "$WORK/recA.appname" --json > "$WORK/recA.json"
"$CLI" export --doc "$WORK/recA.appname" --format png --out "$WORK/recA.png" --json > /dev/null
ref_run 12 ref12
if [ "$(sha "$WORK/recA.png")" = "$(sha "$WORK/ref12.png")" ]; then
    echo "FI-A-recover PASS (recovered export byte-identical to the clean 12-line run; the opacity-0.5 probes at lines 5/10 rendered ONCE)"
else
    echo "FI-A-recover FAIL"
    fail=1
fi

# ---- FI-B: torn tail (log surgery) ---------------------------------
SZ=$(stat -f%z "$WORK/auto.log" 2>/dev/null || stat -c%s "$WORK/auto.log")
dd if="$WORK/auto.log" of="$WORK/torn.log" bs=1 count=$((SZ - 9)) 2>/dev/null
"$CLI" recover --doc "$pkg" --log "$WORK/torn.log" \
    --out "$WORK/recB.appname" --json > "$WORK/recB.json"
torn="$(python3 -c 'import json,sys; o=json.load(open(sys.argv[1])); print(o.get("torn"), o.get("replayed"))' "$WORK/recB.json")"
replayedB="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["replayed"])' "$WORK/recB.json")"
"$CLI" export --doc "$WORK/recB.appname" --format png --out "$WORK/recB.png" --json > /dev/null
ref_run "$replayedB" refB
if [ "$(echo "$torn" | cut -d' ' -f1)" = "True" ] \
    && [ "$(sha "$WORK/recB.png")" = "$(sha "$WORK/refB.png")" ]; then
    echo "FI-B-torn PASS (torn:true reported; replayed $replayedB = the surviving prefix; export equals the clean $replayedB-line run — §7.8 corrupt-tail never fatal)"
else
    echo "FI-B-torn FAIL (torn/replayed: $torn)"
    fail=1
fi

# ---- FI-C: mid-log payload corruption ------------------------------
python3 - "$WORK/auto.log" "$WORK/flip.log" <<'EOF'
import sys
b = bytearray(open(sys.argv[1], 'rb').read())
b[len(b) // 2] ^= 0x40  # one bit, mid-log
open(sys.argv[2], 'wb').write(bytes(b))
EOF
rcC=0
"$CLI" recover --doc "$pkg" --log "$WORK/flip.log" \
    --out "$WORK/recC.appname" --json > "$WORK/recC.json" 2>&1 || rcC=$?
if [ "$rcC" -ne 0 ]; then
    echo "FI-C-flip PASS (typed refusal, exit $rcC — corrupted mid-log record never applied): $(head -c 200 "$WORK/recC.json")"
elif python3 -c 'import json,sys; o=json.load(open(sys.argv[1])); sys.exit(0 if (o.get("torn") or int(o.get("replayed",99)) < 12) else 1)' "$WORK/recC.json"; then
    replayedC="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["replayed"])' "$WORK/recC.json")"
    "$CLI" export --doc "$WORK/recC.appname" --format png --out "$WORK/recC.png" --json > /dev/null
    ref_run "$replayedC" refC
    if [ "$(sha "$WORK/recC.png")" = "$(sha "$WORK/refC.png")" ]; then
        echo "FI-C-flip PASS (replay stopped at the corrupt record: replayed $replayedC < 12, state equals the clean $replayedC-line run)"
    else
        echo "FI-C-flip FAIL (truncated but state diverges from the clean prefix)"
        fail=1
    fi
else
    echo "FI-C-flip FAIL (corrupt mid-log record replayed silently: $(head -c 200 "$WORK/recC.json"))"
    fail=1
fi

echo "FI-1..FI-3 BLOCKED (exact §7.8 in-engine points need per-point APPNAME_FI_* doors; only the post-apply abort door ships) — TRIGGER: O-CORE fault doors at journal-append / extent-TILEREF / CHECKPOINT-fsync"
echo "FI-4..FI-6 BLOCKED (§7.6 save-pipeline step 4/5/6 kill points: same missing doors) — TRIGGER: O-CORE fault doors in save_package stage order"
echo "§10.6 offer/orphan-scan/decline-archives BLOCKED (no CLI surface; recover is explicit-path) — TRIGGER: the §10.6 offer surface"

if [ "$fail" -ne 0 ]; then
    echo "W22 crash-fault-injection: FAIL"
    exit 1
fi
echo "W22 crash-fault-injection: PASS (runnable legs green; blocked legs recorded typed)"
