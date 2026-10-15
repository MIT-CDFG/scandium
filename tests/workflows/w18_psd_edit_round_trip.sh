#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# w18_psd_edit_round_trip.sh — W18 psd-edit-round-trip.
# Runs tests/perf/src/bin/w18_psd_edit_roundtrip.rs (see its module doc for the leg map
# and the spec anchors); this wrapper records the residue that stays
# outside the binary's reach:
#   full_featured_rgb16 residue: 16-bit ENGINE landing gated on the u16 feature election (import-side 16-bit decode probed green in the bin); mask/channel edit arms typed-blocked in-bin
#
# Exit: 0 pass · 1 fail · 2 environment not ready.

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="$ROOT/target/release/w18_psd_edit_roundtrip"
[ -x "$BIN" ] || { echo "SKIP: scenario binary not built (cargo build -p appname-perf --bins --release)"; exit 2; }

if "$BIN"; then
    echo "RESIDUE recorded: full_featured_rgb16 residue: 16-bit ENGINE landing gated on the u16 feature election (import-side 16-bit decode probed green in the bin); mask/channel edit arms typed-blocked in-bin"
    echo "W18 psd-edit-round-trip: PASS"
else
    echo "W18 psd-edit-round-trip: FAIL"
    exit 1
fi
