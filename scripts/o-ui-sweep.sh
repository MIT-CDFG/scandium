#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# O-UI SWEEP — the payload's four checks as an ARTIFACT, not retyped prose.
#
# WHY THIS FILE EXISTS. The sweep lived only as prose in reports/O-UI-WAKE-PAYLOAD.md
# and was retyped at every tick. Four measured failures, one mechanism:
#   C-633  typed '^=======$' — in BRE a mid-alternation $ is a LITERAL, so the marker
#          count read 2 of 3 for ~40 ticks. The payload's own form has no $.
#   C-634  diagnosed from scratch a rule the payload already carried verbatim.
#   C-641  dropped the positive control on the exact tick the window came back 0.
#   C-649  omitted check 3 for 13 consecutive ticks while reporting "sweep clean".
# G-121: a lesson has three homes — prose, prompt, artifact — and only the third
# survives you. This is the third home.
#
# NOT a replacement for reading the payload: the payload carries the REASONS, and a
# reason is what lets you tell a broken check from a clean one. This carries the FORMS.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 2
MYFILES='shell/qml/Main.qml shell/qml/LayersPanel.qml shell/qml/ToolPalette.qml'
MINE='Main\.qml|LayersPanel\.qml|ToolPalette\.qml|tst_sample_blob_adversarial|tst_layers_blend_row|O-UI'
# THIRD ARM (C-661): a commit can discuss my surface while NEITHER touching my files
# NOR naming them in its message -- 36214f32 was about AdjustmentParamDialog, said so
# only inside a reports/ diff, and both existing arms correctly read 0. Derived from
# the QML tree, never typed. Main/Theme EXCLUDED: as bare words they matched 15 of 40
# commits of ordinary prose on their own and drowned the arm at 50%.
TOPIC=$(find shell/qml -name '*.qml' | sed 's#.*/##; s#\.qml$##' \
        | grep -vxE 'Main|Theme' | paste -sd'|' -)

# 0 — THE POPULATION IS A GRAPH QUESTION, SO ASK THE GRAPH (C-718).
# This was `--since=<my last ledger commit>` and it was STRUCTURALLY BLIND:
# the bound is my own last ledger WRITE, which happens at the END of a tick,
# so anything a peer committed DURING that tick has an older committer date
# and is invisible to every future --since. Not late — NEVER SEEN. Measured
# at C-718: position said 61 behind while the window said 0, and all 61 were
# dated before my bound.
#
# C-585 said DERIVE the bound rather than type it, and I derived a TIMESTAMP.
# The deeper error was using a clock for a reachability question at all.
# `HEAD..main` is exact, self-maintaining, and cannot skew.
echo "bound: HEAD..main (graph, not clock — C-718)"

# 1 — did anything land on my surface?
# The control's JOB CHANGED with the population. Under --since a bare 0 was
# vacuity-capable (a typo'd path or a rounded date returns 0), so the control
# proved the query COULD fire. Under HEAD..main a 0 means "I am up to date",
# which is a TRUE zero. What still needs proving is that the refs RESOLVE, so
# the control counts main's commits outright: if that is 0 the ref is wrong.
W=$(git rev-list --count HEAD..main)
CTRL=$(git rev-list --count main)
SURF=0; OWNED=0; CITE=0; TOPIC_N=0
for c in $(git rev-list HEAD..main); do
  [ "$(git show --name-only --format='' "$c" | grep -cE '^(shell|crates)/')" -gt 0 ] && SURF=$((SURF+1))
  [ "$(git log -1 --format=%B "$c" | grep -cE "$MINE")" -gt 0 ] && CITE=$((CITE+1))
  # scoped to the WINDOW, so the cost is one `git show` per landed commit
  [ "$(git show "$c" | grep -cE "$TOPIC")" -gt 0 ] && TOPIC_N=$((TOPIC_N+1))
  for f in $MYFILES; do git show --name-only --format='' "$c" | grep -qxF "$f" && OWNED=$((OWNED+1)); done
done
echo "[1] window=$W  control(-6h)=$CTRL  surface=$SURF  owned=$OWNED  cite-arm=$CITE  topic-arm=$TOPIC_N"
[ "$TOPIC_N" -gt 0 ] && echo "    topic-arm fired: a landed commit DISCUSSES my surface -- read it, the other arms cannot see this"
[ "$W" -eq 0 ] && [ "$CTRL" -eq 0 ] && echo "    !! CONTROL DID NOT FIRE — a bare 0 here is vacuity-capable, do not report it as quiet"
# C-651: the control uses a WIDER bound, so CTRL < W is arithmetically impossible.
# If it ever prints, the control query is broken and BOTH numbers are suspect —
# which is worse than a silent control, because the window still looks answered.
[ "$CTRL" -lt "$W" ] && echo "    !! CONTROL IS BROKEN — control($CTRL) < window($W) with a wider bound is impossible; distrust both"

# 2 — instruments and suites. Marker grep uses the payload's form: NO '$' (C-633).
M=$(grep -c '^<<<<<<<\|^=======\|^>>>>>>>' reports/QUESTIONS-FOR-BILL.md)
F=$(python3 scripts/o-ui-brief-traps.py --selftest 2>&1 | grep -cE '^[[:space:]]*FAIL')
PP=$(bash -c '. scripts/instruments/oint-probe.sh 2>/dev/null; p_paste reports/QUESTIONS-FOR-BILL.md reports/INTEGRATION-log.md 2>&1' | tail -1)
echo "[2] position=$(git rev-list --left-right --count main...HEAD | tr '\t' '/')  markers=$M  selftest_FAIL=$F  p_paste=$PP"
printf '    2a VERDICT: '; nice -n 19 ./target/shell-build/tst_sample_blob_adversarial 2>&1 | grep -oE '[0-9]+ passed, [0-9]+ failed' || echo "NOT RUN"
echo "    2b is DETACHED by design — launch it separately; carry the last verdict WITH ITS TIMESTAMP (C-649: 13 ticks carried)"

# 3 — is my shipped QML in the binary? FRESHNESS GATE FIRST (C-560), then the literal.
ART=target/shell-build/appname-shell
if [ ! -e "$ART" ]; then echo "[3] NO ARTIFACT — nothing to gate"; else
  AM=$(date -r "$ART" '+%s'); QS=$(git log -1 --format=%ct -- shell/qml/)
  if [ "$AM" -lt "$QS" ]; then
    echo "[3] NOT-YET-CUT — artifact $(date -r "$ART" '+%FT%T') predates $(git log -1 --format='%h %cI' -- shell/qml/ | cut -c1-27)"
    echo "    Report NOT-YET-CUT, NEVER 'present'. No probe result would mean anything."
  else
    echo "[3] artifact is fresh — probe admissible, but it MUST be a literal the change INTRODUCED;"
    echo "    a reference-only or comment-only change is UNMEASURABLE-BY-THIS-METHOD. Name what CAN see it."
  fi
fi
