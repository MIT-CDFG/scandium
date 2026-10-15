#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# w03_large_image.sh — workflow gate W03, LIVE since CP-0.5 wave 3 (the
# spill mechanism landed; scaffold activated per the staged plan).
#
# Spec: plan workflow matrix W03; ES §10.4/§3.4. Runs the engine-level
# scenario binary (tests/perf/src/bin/w03_spill_scenario.rs: 12288x8192
# transparent canvas, pasted 2048² plane, 10 fills under a 64 MiB history
# budget — in-process invariants: spills occurred, resident history
# bounded, undo/redo byte-identical across spilled steps) under
# `/usr/bin/time -l` and asserts the PEAK-RSS tier bound from
# tests/perf/scenarios.yaml. FLAGGED (lane log): the shipped CLI exposes
# no history-budget flag yet — the shipped-binary variant of this gate
# activates when O-CORE lands that surface; the engine under test is the
# shipped code either way.
#
# Exit: 0 pass · 1 fail · 2 environment not ready.

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

# RELEASE build: the scenario is pixel-heavy (2048^2 fills x10); the
# debug profile distorts both wall time and the RSS reading (core's
# reference smoke also ran --release).
BIN="$ROOT/target/release/w03_spill_scenario"
if [ ! -x "$BIN" ]; then
    echo "SKIP: scenario binary not built (cargo build -p appname-perf --bins --release)"
    exit 2
fi

# stable tier tag: env override -> pinned baseline tag -> hostname
# (hostnames proved network-volatile on macOS; the pin travels with the
# baselines it describes)
TAG="${APPNAME_PERF_TIER:-}"
[ -n "$TAG" ] || TAG="$(cat "$ROOT/.claude/perf/baselines/HOST_TAG" 2>/dev/null || true)"
[ -n "$TAG" ] || TAG="$(hostname -s | tr '[:upper:]' '[:lower:]')"
YAML="$ROOT/tests/perf/scenarios.yaml"
bound="$(awk -F': ' -v k="w03_peak_rss_mib_$TAG" '$1==k{print $2}' "$YAML")"
[ -n "$bound" ] || bound="$(awk -F': ' '$1=="w03_peak_rss_mib_default"{print $2}' "$YAML")"
[ -n "$bound" ] || { echo "W03: no tier bound in scenarios.yaml"; exit 2; }

OUT="$(mktemp "${TMPDIR:-/tmp}/appname-qa-w03-XXXXXX")"
trap 'rm -f "$OUT"' EXIT

fail=0
if /usr/bin/time -l "$BIN" > "$OUT" 2>&1; then
    grep -E "stats:|PASS" "$OUT" | sed 's/^/  /'
else
    echo "W03 scenario binary FAILED:"
    tail -5 "$OUT" | sed 's/^/  /'
    fail=1
fi

# macOS `time -l` reports "maximum resident set size" in BYTES
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
    echo "peak-rss FAIL (no RSS reading from /usr/bin/time -l)"
    fail=1
fi

if [ "$fail" -ne 0 ]; then
    echo "W03 large-image: FAIL"
    exit 1
fi
echo "W03 large-image: PASS"
