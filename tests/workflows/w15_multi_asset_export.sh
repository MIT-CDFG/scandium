#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# w15_multi_asset_export.sh — W15 multi-asset-export (slice leg).
# Runs tests/perf/src/bin/w15_slice_export.rs (see its module doc for the leg map
# and the spec anchors); this wrapper records the residue that stays
# outside the binary's reach:
#   artboard leg: gated Post-1.0 (§19.4) — DECLARED SKIP per §41.1 until artboard.add ships; CLI --export-slices wiring is the recorded seam residue (io door proven)
#
# Exit: 0 pass · 1 fail · 2 environment not ready.

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="$ROOT/target/release/w15_slice_export"
[ -x "$BIN" ] || { echo "SKIP: scenario binary not built (cargo build -p appname-perf --bins --release)"; exit 2; }

if "$BIN"; then
    echo "RESIDUE recorded: artboard leg: gated Post-1.0 (§19.4) — DECLARED SKIP per §41.1 until artboard.add ships; CLI --export-slices wiring is the recorded seam residue (io door proven)"
    echo "W15 multi-asset-export (slice leg): PASS"
else
    echo "W15 multi-asset-export (slice leg): FAIL"
    exit 1
fi
