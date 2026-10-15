#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# THE BATTERY, WITH CONTENTION RECORDED FOR ITS WHOLE DURATION.
#
# WHY (C-713, paid for in a real lost verdict). A 43-minute battery took a
# QFATAL "Test function timed out" -> SIGABRT in tst_palette_pins while a
# peer was linking test binaries inside the window (their mtimes land at
# 04:33; my run was 03:51-04:34). The whole battery ran 2.4x its control
# arm's time. Nothing asserted anything false: a function did not return
# within QtTest's 300s per-function watchdog.
#
# THE GATE COULD NOT HAVE CAUGHT IT. `o-ui-machine-free.sh` checks the
# LAUNCH INSTANT -- load was 1.21 against a ceiling of 20 -- and the
# exposure is the whole DURATION of the run. C-704 recorded the window
# between check and use; the real window is check-to-COMPLETION.
#
# WHAT THIS ADDS, AND ITS LIMIT, STATED SO IT CANNOT BE OVERSOLD:
# it does NOT prevent contention. It records load across the run and
# refuses to call a watchdog-under-contention a RED, because that verdict
# describes the machine and not the code. Rule 2 says VOID IS NOT GREEN;
# this is its mirror -- VOID IS NOT RED EITHER, and a desk that reports a
# false red burns the same credibility as one that reports a false green.
#
# THE DISCRIMINATION IS DELIBERATELY NARROW, so it can never launder a
# real failure:
#   * ONLY a QtTest per-function watchdog timeout can be voided.
#   * An ASSERTION failure is a RED at any load, always. Load never
#     excuses a wrong answer, only a slow one.
#   * A timeout with load that stayed BELOW the ceiling stays RED -- that
#     is a genuine hang and the most important thing this can find.
#
# rc 0 = GREEN   rc 1 = RED   rc 3 = VOID (re-run required, not a verdict)
set -uo pipefail
cd "$(dirname "$0")/.." || exit 2
BUILD=${1:-target/shell-build}
CEIL=${LOAD_CEIL:-$(( $(sysctl -n hw.ncpu 2>/dev/null || echo 8) * 2 ))}
# ★ THE LOG AND THE SAMPLES ARE THE VERDICT'S EVIDENCE AND ARE KEPT (C-720).
# The first version mktemp'd both and rm -f'd them at every exit, so a VOID
# destroyed the only record that could show whether voiding was right — and
# it did exactly that once. Stable paths, overwritten each run, auditable
# after the fact.
EVID="$BUILD/Testing/o-ui-battery"
mkdir -p "$EVID"
LOG="$EVID/ctest.log"; SAMPLES="$EVID/load-samples.txt"

./scripts/o-ui-machine-free.sh || { echo "NOT LAUNCHED — gate refused"; exit 2; }

# Sample load every 30s for the whole run. The samples ARE the evidence;
# a verdict without them cannot be classified and must not be guessed.
( while :; do uptime | sed 's/.*averages: //' | awk '{print $1}' | tr -d ,; sleep 30; done > "$SAMPLES" ) &
SAMPLER=$!
trap 'kill "$SAMPLER" 2>/dev/null' EXIT   # kill your own only: this is mine

echo "BATTERY START $(date '+%H:%M:%S') sha=$(git rev-parse --short HEAD) ceiling=$CEIL"
ctest --test-dir "$BUILD" --timeout 900 > "$LOG" 2>&1
RC=$?
kill "$SAMPLER" 2>/dev/null

MAXLOAD=$(sort -g "$SAMPLES" 2>/dev/null | tail -1)
NSAMP=$(wc -l < "$SAMPLES" | tr -d ' ')
echo "CTEST_RC=$RC  load samples=$NSAMP  max=${MAXLOAD:-unknown}  ceiling=$CEIL"
grep -E 'tests passed|tests failed' "$LOG"

if [ "$RC" -eq 0 ]; then
  echo "VERDICT: GREEN"; echo "  evidence kept: $LOG , $SAMPLES"; exit 0
fi

# Grep the FAILURE SHAPE, never tail (C-527: tail on a check that reports
# failures first is a green-making machine).
#
# ★ THE QUANTIFIER IS THE WHOLE SAFETY PROPERTY (C-720). This read
# `TIMEDOUT > 0`, i.e. ANY timeout. That is wrong and it is the exact
# failure this script was written to prevent: with THREE failures of which
# TWO were timeouts, it voided the run and the third — whose reason was an
# assertion for all anyone could tell — vanished with it. One assertion
# beside one timeout, under load, and bad news is dismissed.
#
# The rule is ALL: every failing test must be a watchdog timeout before
# contention can explain the run. If even one failure is not a timeout,
# the run is RED and the load is irrelevant to it.
#
# WHY THE OLD TRUTH TABLE MISSED IT: all five cases had ONE failure. A
# quantifier bug needs TWO to become visible, so the table could not see
# it however many single-failure rows it had.
# `grep -c` EXITS 1 when the count is zero, so `... || echo 0` printed the
# grep's own "0" AND the fallback "0" — a two-line value that made every
# `[ "$TIMEDOUT" -eq ... ]` below die with "integer expression expected".
# The verdict was still RED, but by ACCIDENT: the comparisons errored out
# and fell through to the assertion branch rather than evaluating. A gate
# that reaches the right answer through a broken comparison is not a gate.
TIMEDOUT=$(grep -c 'Test function timed out' "$BUILD/Testing/Temporary/LastTest.log" 2>/dev/null)
[ -z "$TIMEDOUT" ] && TIMEDOUT=0
NFAILED=$(grep -oE '[0-9]+ tests failed' "$LOG" | grep -oE '^[0-9]+' | head -1)
[ -z "$NFAILED" ] && NFAILED=$(wc -l < "$BUILD/Testing/Temporary/LastTestsFailed.log" 2>/dev/null | tr -d ' ')
[ -z "$NFAILED" ] && NFAILED=0
OVER=0
[ -n "$MAXLOAD" ] && awk -v l="$MAXLOAD" -v c="$CEIL" 'BEGIN{exit !(l+0 > c+0)}' && OVER=1
echo "  failures=$NFAILED  of which watchdog timeouts=$TIMEDOUT  load_over_ceiling=$OVER"

if [ "$TIMEDOUT" -gt 0 ] && [ "$TIMEDOUT" -eq "$NFAILED" ] && [ "$OVER" -eq 1 ]; then
  echo "VERDICT: VOID — watchdog timeout with load ${MAXLOAD} over ceiling ${CEIL}."
  echo "  This describes the MACHINE, not the code. Re-run required; NOT a red."
  echo "  (An assertion failure would have been RED regardless of load.)"
  echo "  evidence kept: $LOG , $SAMPLES"; exit 3
fi
if [ "$TIMEDOUT" -gt 0 ]; then
  echo "VERDICT: RED — watchdog timeout with load max ${MAXLOAD} UNDER ceiling ${CEIL}."
  echo "  Contention does not explain this. A genuine hang is the finding."
else
  echo "VERDICT: RED — assertion failure. Load is never an excuse for a wrong answer."
fi
echo "  evidence kept: $LOG , $SAMPLES"; exit 1
