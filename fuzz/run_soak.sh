#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# run_soak.sh — the NIGHTLY FUZZ ENTRY POINT (ci_local tier; no CI
# system exists — O-BUILD wires `bash fuzz/run_soak.sh` into the
# scripts/ci_local.sh nightly section, handoff in reports/O-QA-log.md).
#
# Builds the cargo-fuzz-layout targets and soaks each over its committed
# seed corpus. On a NIGHTLY toolchain with cargo-fuzz installed it uses
# the full engine (ASan + coverage); on this stable-only host it runs
# the libFuzzer binaries directly — same targets, same corpus, same
# never-panic bar, WITHOUT sanitizers (recorded honestly per rule 8).
#
# Usage: bash fuzz/run_soak.sh [max_total_time_seconds_per_target]
# Exit: 0 = every target completed its soak with zero crashes/OOMs.

set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
SECS="${1:-300}"
# CP-1.0 additions (O-WATCH review #9 planned row): vector sidecar,
# pack manifest, action.v1 — the input surfaces this checkpoint added.
# fuzz_raw_probe: the O-IO raw_fuzz_floor lift (RAW-binding slice
# handoff — §34.2 raw_probe at the LibRaw wrapper boundary, both module
# states per input).
TARGETS="fuzz_psd_import fuzz_codec_import fuzz_vector_sidecar fuzz_pack_manifest fuzz_action_v1 fuzz_raw_probe"

cd "$HERE"
if command -v cargo-fuzz >/dev/null 2>&1 && rustup toolchain list 2>/dev/null | grep -q nightly; then
    ENGINE="cargo-fuzz (nightly, sanitized)"
    for t in $TARGETS; do
        cargo +nightly fuzz run "$t" "corpus/$t" -- -max_total_time="$SECS" -print_final_stats=1
    done
else
    ENGINE="direct libFuzzer loop (stable host, unsanitized)"
    cargo build --release
    for t in $TARGETS; do
        mkdir -p "findings/$t"
        # Stable builds carry NO coverage feedback (sancov needs the
        # nightly cargo-fuzz engine), so nothing ever merges into an
        # empty primary corpus and mutation degrades to blind bytes.
        # Run on a scratch COPY of the seed corpus instead: every
        # mutation then starts from a real structured input. Recorded
        # honestly: unsanitized AND unguided — depth rides the seeds +
        # the in-workspace structure-aware fuzz floor until nightly
        # lands.
        work="$(mktemp -d "${TMPDIR:-/tmp}/appname-fuzz-$t-XXXXXX")"
        cp "corpus/$t"/* "$work"/
        echo "==== soak $t (${SECS}s, engine: $ENGINE)"
        "./target/release/$t" "$work" \
            -max_total_time="$SECS" -print_final_stats=1 -artifact_prefix="findings/$t/" \
            2>&1 | tail -12
        rm -rf "$work"
        crashes="$(ls "findings/$t" | wc -l | tr -d ' ')"
        if [ "$crashes" != "0" ]; then
            echo "SOAK FAIL: $t produced $crashes finding(s) under findings/$t"
            exit 1
        fi
        echo "==== $t: zero findings"
    done
fi
echo "fuzz soak: PASS (targets: $TARGETS; ${SECS}s each; engine: $ENGINE)"
