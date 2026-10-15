#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# w09_accessibility.sh — workflow gate W09 (accessibility), TRANSLATED +
# STAGED. Spec: W09-accessibility.yaml (§34.6 W9/§31/Part 49).
#
# W9 is THE one §34.6 workflow that is not headless (§41.1: runner
# ui-harness — keyboard scripts, screen-reader walker, live regions,
# contrast scan). STAGED STATUS (typed): a11y completion is IN FLIGHT
# at O-I18N-A11Y/O-UI this wave; main's shell/ carries no a11y walker
# or keyboard-script runner door yet (probed below). TRIGGER: the a11y
# completion merge landing the §31.5 walkers + a headless-drivable
# probe entry (shell --a11y-walk / --screenshot-tour probe mode) and
# tests/a11y/keyboard-scripts.yaml mirrored into the build.
#
# ENCODED STEPS (bind on merge; W9 letter):
#   1  keyboard-only W1 steps 1-4 (task parity, Part 49 §49.1)
#   2  Layers-panel focus order == visual order
#   3  all panels expose name/role/value (§31.1)
#   4  selection change announces via live region (§31.3)
#   5  WCAG-AA contrast scan of the theme (§31.4)
#
# Exit: 0 pass · 1 fail · 2 staged/environment not ready.

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

PROBE=""
for cand in "$ROOT/shell" "$ROOT/crates/appname-shell"; do
    [ -d "$cand" ] && PROBE="$cand" && break
done
# A11Y-5 landed hook A (the APPNAME_A11Y_LOG JSONL announcement sink) as
# the drivable observation door; the walker probe looks for it directly.
WALKER=no
if [ -n "$PROBE" ] && grep -rql "APPNAME_A11Y_LOG\|NSAccessibility\|accessibilityRole" "$PROBE" 2>/dev/null; then
    WALKER=yes
fi
KEYSCRIPTS=no
[ -f "$ROOT/tests/a11y/keyboard-scripts.yaml" ] && KEYSCRIPTS=yes

if [ "$WALKER" = "no" ] || [ "$KEYSCRIPTS" = "no" ]; then
    echo "W09 accessibility: STAGED (probed: shell a11y sink door=$WALKER, keyboard-scripts mirrored=$KEYSCRIPTS — a11y completion in flight at O-I18N-A11Y/O-UI)"
    echo "TRIGGER: the a11y completion merge (§31.5 walkers + probe entry + keyboard-scripts.yaml); steps 1-5 encoded above, harness grows their drivers on landing (O-QA re-encode, same-day)"
    exit 2
fi

# Doors present on the candidate. Drive them:
#  - the KS harness (tests/a11y/run_ks.py) live-exercises hook A on the
#    candidate binary + validates the frozen sink/D-43-label contract +
#    emits the KS-1..8 ledger (steps 1-4: keyboard task-parity announces);
#  - the shell's own a11y ctest subset covers the WCAG-AA theme scan
#    (step 5, tst_theme_contrast), the tree roles/names lint (step 3,
#    tst_a11y_tree_lint), the announcer live-region policy (step 4,
#    tst_announcer) and the focus-probe scenes (tst_focus_probes).
# STRUCTURAL LIMIT recorded honestly: the frozen shell exposes no external
# KS-script-runner door, so KS key sequences are not driven end-to-end —
# run_ks.py records that as its ledger's typed disposition (not a fake pass).
SHELL_BIN=""
for cand in "$ROOT/shell/build/appname-shell" "$ROOT/shell/build/Release/appname-shell"; do
    [ -x "$cand" ] && SHELL_BIN="$cand" && break
done
if [ -z "$SHELL_BIN" ]; then
    echo "W09 accessibility: STAGED (keyboard-scripts + sink door present, but no built shell binary — build it: cmake --build shell/build, then re-run)"
    exit 2
fi

rc=0
echo "--- W09 step 1-4: KS harness (live hook A + sink/label contract + KS-1..8 ledger)"
python3 "$ROOT/tests/a11y/run_ks.py" || rc=1

if [ -d "$ROOT/shell/build" ]; then
    echo "--- W09 step 3/4/5: shell a11y ctest subset (tree lint, announcer, focus probes, WCAG-AA theme scan)"
    ( cd "$ROOT/shell/build" && ctest --output-on-failure \
        -R "tst_a11y_tree_lint|tst_announcer|tst_focus_probes|tst_theme_contrast|tst_selection_scan" ) || rc=1
fi

if [ "$rc" -eq 0 ]; then
    echo "W09 accessibility: PASS (hook A live on the candidate + sink/D-43-label contract + a11y ctest subset green; KS keyboard-drive gap typed-skipped per the run_ks ledger — reported to O-INT)"
    exit 0
fi
echo "W09 accessibility: FAIL (a driven leg reported a failure — see output above)"
exit 1
