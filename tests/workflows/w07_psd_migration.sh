#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# w07_psd_migration.sh — workflow gate W07 (CP-0.7 subset per plan §2.4:
# "psd-migration: the merged PSD import corpus subset + compat reports").
# Runs tests/perf/src/bin/w07_psd_migration.rs in verify mode: six
# byte-level corpus-subset PSDs (O-IO's proven emitter, reused verbatim)
# decode with zero crashes, land journaled, carry schema-tagged §42.4
# compat reports (unsupported features listed, layers imported as
# fallbacks — never dropped), and byte-match the engine-produced
# composite + report goldens under goldens/psd_w07/.
#
# The FULL W07 (every psd-corpus file, §34.1 tolerance diffing, the
# 500 MB perf contract) is a 1.0-gate row (spec artifact gates: ["1.0"]).
#
# Exit: 0 pass · 1 fail · 2 environment not ready.

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

BIN="$ROOT/target/release/w07_psd_migration"
if [ ! -x "$BIN" ]; then
    echo "SKIP: scenario binary not built (cargo build -p appname-perf --bins --release)"
    exit 2
fi

if "$BIN"; then
    echo "W07 psd-migration: PASS"
else
    echo "W07 psd-migration: FAIL"
    exit 1
fi
