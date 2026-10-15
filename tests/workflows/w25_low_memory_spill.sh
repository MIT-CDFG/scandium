#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# w25_low_memory_spill.sh — workflow gate W25 (CP-0.7 subset per plan
# §2.4: "the CP-0.5 spill machinery under the OpNode graph").
#
# Leg 1 (engine-level, release scenario binary under /usr/bin/time -l):
#   tests/perf/src/bin/w25_low_memory_spill.rs — identical deterministic
#   stream (pastes/fills + smartobject.convert + history.snapshot) on an
#   unbounded engine and an 8 MiB-budget engine: spills > 0, revives > 0,
#   resident ≤ budget, state_hash LOSSLESS both at the final state and
#   after the full undo/redo revive walk. Peak RSS asserted against the
#   w25 tier bound in tests/perf/scenarios.yaml.
#
# Leg 2 (shipped-binary): the SAME edit script through `appname script
# --history-budget 8` vs unbudgeted — exported composite byte-identical
# (spilling is lossless at the CLI door too).
#
# Recorded pending (full W25 is a 1.0 gate): env_lowram8 OS-level cap,
# stall histogram p95, scratch-extent truncation probe, disk-full
# injection (Appendix H retryable) — see the lane-log wave-2 entry.
#
# Exit: 0 pass · 1 fail · 2 environment not ready.

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

BIN="$ROOT/target/release/w25_low_memory_spill"
if [ ! -x "$BIN" ]; then
    echo "SKIP: scenario binary not built (cargo build -p appname-perf --bins --release)"
    exit 2
fi
CLI="${APPNAME_CLI:-}"
if [ -z "$CLI" ]; then
    for cand in "$ROOT/target/debug/appname" "$ROOT/target/release/appname"; do
        [ -x "$cand" ] && CLI="$cand" && break
    done
fi
[ -n "$CLI" ] || { echo "SKIP: appname-cli not built"; exit 2; }

TAG="${APPNAME_PERF_TIER:-}"
[ -n "$TAG" ] || TAG="$(cat "$ROOT/.claude/perf/baselines/HOST_TAG" 2>/dev/null || true)"
[ -n "$TAG" ] || TAG="$(hostname -s | tr '[:upper:]' '[:lower:]')"
YAML="$ROOT/tests/perf/scenarios.yaml"
bound="$(awk -F': ' -v k="w25_peak_rss_mib_$TAG" '$1==k{print $2}' "$YAML")"
[ -n "$bound" ] || bound="$(awk -F': ' '$1=="w25_peak_rss_mib_default"{print $2}' "$YAML")"
[ -n "$bound" ] || { echo "W25: no tier bound in scenarios.yaml"; exit 2; }

WORK="$(mktemp -d "${TMPDIR:-/tmp}/appname-qa-w25-XXXXXX")"
OUT="$WORK/scenario.out"
trap 'rm -rf "$WORK"' EXIT
sha() { shasum -a 256 "$1" | cut -d' ' -f1; }

fail=0
if /usr/bin/time -l "$BIN" > "$OUT" 2>&1; then
    grep -E "stats:|walk:|PASS" "$OUT" | sed 's/^/  /'
else
    echo "W25 scenario binary FAILED:"
    tail -5 "$OUT" | sed 's/^/  /'
    fail=1
fi
rss_bytes="$(awk '/maximum resident set size/{print $1}' "$OUT")"
if [ -n "$rss_bytes" ]; then
    rss_mib=$((rss_bytes / 1048576))
    if [ "$rss_mib" -le "$bound" ]; then
        echo "peak-rss PASS (${rss_mib} MiB <= tier bound ${bound} MiB, tier $TAG)"
    else
        echo "peak-rss FAIL (${rss_mib} MiB > tier bound ${bound} MiB)"
        fail=1
    fi
else
    echo "peak-rss FAIL (no RSS reading)"
    fail=1
fi

# ---- Leg 2: shipped-binary --history-budget losslessness -----------------
SCRIPT="$WORK/edit.jsonl"
LID="25000000-0000-4000-8000-00000000b001"
{
    printf '%s\n' "{\"schema\":\"appname.command.v1\",\"id\":\"25000000-0000-4000-8000-00000000a001\",\"kind\":\"layer.add\",\"params\":{\"kind\":\"raster\",\"newLayerId\":\"$LID\",\"name\":\"plane\"}}"
    for i in 1 2 3 4 5 6; do
        printf '%s\n' "{\"schema\":\"appname.command.v1\",\"id\":\"25000000-0000-4000-8000-00000000a10$i\",\"kind\":\"fill\",\"target\":{\"layerId\":\"$LID\"},\"params\":{\"content\":\"$([ $((i % 2)) = 1 ] && echo foreground || echo background)\"}}"
    done
    printf '%s\n' "{\"schema\":\"appname.command.v1\",\"id\":\"25000000-0000-4000-8000-00000000a201\",\"kind\":\"history.snapshot\",\"params\":{\"name\":\"cli leg\"}}"
} > "$SCRIPT"

for mode in unbounded budgeted; do
    pkg="$WORK/$mode.appname"
    "$CLI" new --width 1024 --height 1024 --background white --out "$pkg" --json >/dev/null
    if [ "$mode" = "budgeted" ]; then
        "$CLI" script --doc "$pkg" --file "$SCRIPT" --history-budget 8 --json >/dev/null
    else
        "$CLI" script --doc "$pkg" --file "$SCRIPT" --json >/dev/null
    fi
    "$CLI" export --doc "$pkg" --format png --out "$WORK/$mode.png" --json >/dev/null
done
EU="$(sha "$WORK/unbounded.png")"
EB="$(sha "$WORK/budgeted.png")"
if [ "$EU" = "$EB" ]; then
    echo "cli-budget-lossless PASS (exports byte-identical, $EU)"
else
    echo "cli-budget-lossless FAIL ($EU != $EB)"
    fail=1
fi

if [ "$fail" -ne 0 ]; then
    echo "W25 low-memory-spill: FAIL"
    exit 1
fi
echo "W25 low-memory-spill: PASS"
