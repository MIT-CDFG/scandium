#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# w11_print_points.sh — W11 print-oriented-photo (engine-side points).
# Runs tests/perf/src/bin/w11_print_points.rs (see its module doc for the leg map
# and the spec anchors); this wrapper records the residue that stays
# outside the binary's reach:
#   UI residue: the print DIALOG + soft-proof toggle + w11_proof.png golden are O-UI residue (not merged) — TRIGGER: the O-UI print/proof chrome row
#
# Exit: 0 pass · 1 fail · 2 environment not ready.

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="$ROOT/target/release/w11_print_points"
[ -x "$BIN" ] || { echo "SKIP: scenario binary not built (cargo build -p appname-perf --bins --release)"; exit 2; }

if "$BIN"; then
    echo "RESIDUE recorded: UI residue: the print DIALOG + soft-proof toggle + w11_proof.png golden are O-UI residue (not merged) — TRIGGER: the O-UI print/proof chrome row"
    echo "W11 print-oriented-photo (engine-side points): PASS"
else
    echo "W11 print-oriented-photo (engine-side points): FAIL"
    exit 1
fi
