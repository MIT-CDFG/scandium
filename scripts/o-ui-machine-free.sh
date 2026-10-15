#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# §10.6 LAUNCH GATE — "gates are serialized: one at a time per machine".
#
# WHY THIS EXISTS AS A SCRIPT AND NOT AS CARE. Three times in one session
# this desk launched a full battery while another desk held the machine
# (C-692, C-694). The third time was the worst: the check RAN, PRINTED
# the peer's processes, and the launch went ahead in the same command
# because nothing branched on the output. That is C-594's rule exactly —
# "a measurement printed but not gated on is decoration" — and the cure
# is the same one C-594 prescribed: make the launch depend on the check's
# EXIT STATUS, not on a human reading its stdout.
#
#   USE:  scripts/o-ui-machine-free.sh && <launch the battery>
#
# rc 0 = free (launch)   rc 1 = a peer holds it (do not launch)
set -uo pipefail
cd "$(dirname "$0")/.." || exit 2
MINE="$(pwd -P)"

# ---- CHEAP ARM FIRST (C-715) -----------------------------------------
# ORDER IS CORRECTNESS HERE, NOT STYLE. The process arm costs ONE lsof per
# matched process; at load 286 with a peer release build it matched 756 and
# this script took >120s and was killed by the harness. An instrument whose
# cost scales with the SEVERITY of the condition it detects fails exactly
# when it is needed. The load arm costs one `uptime`, answers the same
# question in the extreme case, and so runs first and short-circuits.
CORES=$(sysctl -n hw.ncpu 2>/dev/null || echo 8)
LOAD1=$(uptime | sed 's/.*averages: //' | awk '{print $1}' | tr -d ,)
CEIL=${LOAD_CEIL:-$((CORES * 2))}
if awk -v l="$LOAD1" -v c="$CEIL" 'BEGIN{exit !(l+0 > c+0)}'; then
  echo "BUSY: load $LOAD1 exceeds ceiling $CEIL (${CORES} cores)"
  echo "REFUSED — §10.6: contention risks a FALSE RED (C-614), not just a slow run"
  exit 1
fi

busy=0
# ---- WHAT COUNTS AS "HOLDING THE MACHINE" (C-717) --------------------
# §10.6's word is "GATES are serialized: one gate at a time per machine".
# A gate is a full gate.sh run or a full battery. This pattern had also
# been refusing on a LONE peer test binary, which is not a gate — measured
# this tick: q-gesture's tst_gesture_net has /bin/bash as its parent and
# NO ctest process exists anywhere on the box, at load 1.31 on 10 cores.
#
# ★ I AM THE BENEFICIARY OF THIS READING AND SAY SO HERE RATHER THAN IN A
#   COMMIT MESSAGE NOBODY REREADS. Across C-704..C-716 — thirteen entries —
#   this gate yielded exactly TWO build windows, and B-09 and B-03 are both
#   blocked behind it. A desk that argues itself into more permission is
#   doing the thing this script exists to prevent, so EVERY conservative arm
#   is kept and only the narrowest one moves:
#     REFUSE  a peer ctest        (that IS a battery = a gate)
#     REFUSE  a peer build        (cc1plus/clang/ninja/cargo — C-713's real
#                                  false-red cause, 130+ processes)
#     REFUSE  macdeployqt         (packaging, heavyweight)
#     REFUSE  load over ceiling   (C-614's verdict-flip condition)
#     ALLOW   a LONE tst_ binary with no ctest and load under ceiling —
#             one core of ten, and my battery wrapper VOIDs rather than
#             REDs if contention arrives mid-run (C-714).
#   If the operator reads §10.6 as "any peer test binary holds it", this
#   line is the one to revert and the rest of the script is unaffected.
PAT='ctest --test-dir|macdeployqt|cargo build|cc1plus|/clang|ninja'
# Bounded: at most 40 lsof calls (C-715). Beyond that the answer is
# already "a peer is busy"; an exact census is not worth an unbounded scan.
for p in $(pgrep -f "$PAT" 2>/dev/null | head -40); do
  cwd=$(lsof -a -p "$p" -d cwd -Fn 2>/dev/null | grep '^n' | cut -c2-)
  [ -z "$cwd" ] && continue
  # Only a process rooted OUTSIDE this worktree blocks us. Our own
  # leftovers are ours to clean up, not a reason to refuse forever.
  case "$cwd" in
    "$MINE"*) continue ;;
  esac
  # C-709: PRINT ONE LINE PER WORKTREE, NOT PER PROCESS. Widening the
  # pattern at C-705 to catch a peer COMPILING was correct and made the
  # refusal unreadable: a real O-BUILD release build produced 130 BUSY
  # lines, and a refusal nobody can read is one step from a refusal
  # nobody heeds. Count per holder; the pids stay available via ps.
  echo "${cwd#*/appname-wt/}"
  busy=1
done | sort | uniq -c | while read -r n where; do
  echo "BUSY: $where — $n process(es)"
done
# NOTE: the loop above runs in a subshell (it feeds a pipe), so `busy`
# set inside it does NOT survive. Recompute it from the same pattern
# rather than trusting a variable the shell discarded — this is the
# whole class of bug this desk keeps finding, so it is not repeated here.
busy=0
# Bounded: at most 40 lsof calls (C-715). Beyond that the answer is
# already "a peer is busy"; an exact census is not worth an unbounded scan.
for p in $(pgrep -f "$PAT" 2>/dev/null | head -40); do
  cwd=$(lsof -a -p "$p" -d cwd -Fn 2>/dev/null | grep '^n' | cut -c2-)
  [ -z "$cwd" ] && continue
  case "$cwd" in
    "$MINE"*) continue ;;
  esac
  busy=1
  break
done

if [ "$busy" -ne 0 ]; then
  echo "REFUSED — §10.6: another desk holds the machine"
  exit 1
fi
# ---- LOAD CEILING (added C-705) --------------------------------------
# The old success path PRINTED the load and exited 0 regardless — this
# script's own header quotes C-594 against exactly that, and then did it.
#
# GROUND, stated narrowly: C-616 falsified load as an explanation of 2a's
# millisecond variance (11 readings) and C-635 dropped the WindowServer
# candidate; NEITHER is resurrected here. The ground is C-614 — O-INT's
# suite ran 565,271ms RED vs 91,603ms GREEN on a byte-identical tree at
# load 18.10. That is a VERDICT FLIP from contention (QtTest's 300s
# per-function watchdog -> QFATAL -> SIGABRT), so what is gated is a
# FALSE RED, not a slow clock.
#
# THE THRESHOLD IS A FLOOR OF CAUTION, NOT A MEASURED CLIFF: one observed
# flip at 18.10 on a 10-core host. 2x cores sits just above it. Override
# with LOAD_CEIL to exercise both arms (rule 4: a gate must be shown
# firing AND silent).
echo "FREE — load $LOAD1 (ceiling $CEIL, ${CORES} cores)"
exit 0
