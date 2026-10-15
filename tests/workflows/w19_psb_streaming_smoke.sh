#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# w19_psb_streaming_smoke.sh — W19 psb-streaming-open (SMOKE per plan §2.5).
# Runs tests/perf/src/bin/w19_psb_smoke.rs (see its module doc for the leg map
# and the spec anchors); this wrapper records the residue that stays
# outside the binary's reach:
#   tier residue: 150 MP nightly variant + 600 MP weekly/release + RSS/first-view/pan budgets belong to the perf tier (§41.8) — not claimed by this smoke
#
# Exit: 0 pass · 1 fail · 2 environment not ready.

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="$ROOT/target/release/w19_psb_smoke"
[ -x "$BIN" ] || { echo "SKIP: scenario binary not built (cargo build -p appname-perf --bins --release)"; exit 2; }

if "$BIN"; then
    echo "RESIDUE recorded: tier residue: 150 MP nightly variant + 600 MP weekly/release + RSS/first-view/pan budgets belong to the perf tier (§41.8) — not claimed by this smoke"
    echo "W19 psb-streaming-open (SMOKE per plan §2.5): PASS"
else
    echo "W19 psb-streaming-open (SMOKE per plan §2.5): FAIL"
    exit 1
fi
