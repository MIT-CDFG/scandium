#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# O-UI QUIET-TICK INSTRUMENT (C-341).
#
# WHY THIS EXISTS. q-scenario measured which of their banked lessons are
# enforced EXECUTABLY versus which exist only as prose, and found a 0-of-12
# row that explained a real defect; O-INT ran it on themselves and found the
# guard that saved their session lives in 15 pieces of prose and 0 lines of
# code. Run on THIS desk (C-341): of the seven rules in the quiet-tick
# apparatus, ZERO execute in an artifact this desk owns. The apparatus lived
# entirely in a cron prompt -- which dies at restart, the trap this desk
# itself minted as G-112 and then walked into.
#
# So this file is the apparatus, executable. It is deliberately small and
# it deliberately REFUSES more than it reports:
#
#   rc 0  OK          measured, and the controls could have disagreed
#   rc 2  UNMEASURED  a control did not fire on its known-positive case, or
#                     the arguments do not name a window -- NOT a pass
#   rc 3  VOID        the window contains no commits, so every path count is
#                     a property of the window and not of the tree (G-114)
#
# VOID IS NOT GREEN. The whole point: a 0/0/0/0 over an empty window looks
# exactly like a clean re-pin and carries no information, and this desk has
# published that vacuum as a result. It cannot be published from here.
#
# WHAT THIS DOES NOT MEASURE, said in the output rather than in a comment
# (G-119 -- a guard's output line is part of the guard):
#   * it does not check quiescence: a ref can move while this runs;
#   * it does not run or read any test, so it says nothing about PASS;
#   * it does not look at any tree but this repo's -- no bogos path, ever.
#
# Shell notes: bash, not zsh, and every other-tree read is ${ref}:path --
# the bare $ref:path form is trap A6 and printed four plausible zeros at
# C-339.

set -u

# C-349, from O-WATCH 3: AN INCLUSION LIST FAILS SILENT; AN EXCLUSION LIST FAILS
# LOUD. REPIN_PATHS below is an inclusion list of four hand-picked paths, and on
# 2026-08-26 it printed "goldens=0 artifacts=0 fixtures=0 shell=0" over a window
# that changed contracts/MANIFEST.sha256, two contracts/commands/*.yaml and
# crates/appname-core/src/engine/registry_gen.rs -- generated ENGINE code. The
# control fired; the AIM was wrong (G-115, in this file, one tick after it was
# written). It is kept as a NARROW PIN and clearly labelled as one; the primary
# reading is now the exclusion below, which defaults every path IN and names the
# ones deliberately left out. A directory nobody anticipated shows up demanding a
# look instead of vanishing from the count.
REPIN_PATHS="goldens artifacts fixtures shell"
NON_PRODUCT_PREFIXES="reports/ scripts/ docs/ adr/ plans/ research/ LICENSES/ .claude/"
SINCE=""
UNTIL="main"
CTL_WINDOW=""          # a window KNOWN to touch shell/ -- the positive control
CTL_PATH="shell"

usage() {
    echo "usage: $0 --since <ref> [--until <ref>] [--control <ref>..<ref>]" >&2
    echo "  --since    the tick anchor: what main was at the LAST LOGGED tick" >&2
    echo "  --control  a window known to touch ${CTL_PATH}/ (default: auto)" >&2
    exit 2
}

while [ $# -gt 0 ]; do
    case "$1" in
        --since)   SINCE="${2:-}"; shift 2 ;;
        --until)   UNTIL="${2:-}"; shift 2 ;;
        --control) CTL_WINDOW="${2:-}"; shift 2 ;;
        -h|--help) usage ;;
        *) echo "o-ui-tick: unknown argument '$1'" >&2; usage ;;
    esac
done
[ -n "$SINCE" ] || { echo "o-ui-tick: UNMEASURED-NO-ANCHOR(--since is required; a tick with no anchor is a last-hop check)" >&2; exit 2; }

git rev-parse --git-dir >/dev/null 2>&1 || { echo "o-ui-tick: UNMEASURED-NOT-A-REPO($PWD)" >&2; exit 2; }
for r in "$SINCE" "$UNTIL"; do
    git rev-parse --verify --quiet "${r}^{commit}" >/dev/null || {
        echo "o-ui-tick: UNMEASURED-BAD-REF($r)" >&2; exit 2; }
done

NOW=$(date "+%Y-%m-%dT%H:%M:%S%z")
LANE=$(git rev-parse --abbrev-ref HEAD)
LANE_SHA=$(git rev-parse --short HEAD)
S=$(git rev-parse --short "$SINCE")
U=$(git rev-parse --short "$UNTIL")

# C-344: THE ANCHOR IS TYPED BY HAND EVERY TICK, AND UNTIL NOW NOTHING
# CHECKED IT. `git rev-list A..B` is happy to answer for an A that is not
# on B's history -- it just means "reachable from B and not from A", which
# for an unrelated A is a count of B's whole history, not of a window. That
# number looks exactly like a window count. So: the anchor must be an
# ANCESTOR of the endpoint, or there is no window and this refuses. The
# anchor's own date is printed beside it, because a wrong-but-valid anchor
# is invisible as a sha and obvious as a timestamp.
if ! git merge-base --is-ancestor "$S" "$U" 2>/dev/null; then
    echo "o-ui-tick: UNMEASURED-ANCHOR-NOT-ANCESTOR(${S} is not on ${U}'s history — 'rev-list ${S}..${U}' would count ${U}'s history, not a window)" >&2
    exit 2
fi
S_DATE=$(git log -1 --format=%ci "$S")
U_DATE=$(git log -1 --format=%ci "$U")

echo "o-ui-tick: moment $NOW · repo $(pwd) · lane ${LANE}@${LANE_SHA}"
echo "o-ui-tick: window ${S}..${U} · anchor ${S_DATE} · endpoint ${U_DATE}"

# ---- rule 2: COUNT THE COMMITS FIRST. An empty window VOIDS. ------------
N=$(git rev-list --count "${S}..${U}")
echo "o-ui-tick: window-commits ${N}"
if [ "$N" -eq 0 ]; then
    echo "o-ui-tick: VOID(window ${S}..${U} holds 0 commits — every path count below would be a property of the window, not of the tree)" >&2
    exit 3
fi

# ---- rule 4: the control must be able to disagree ----------------------
# Default control: the newest window on THIS lane that touched CTL_PATH.
if [ -z "$CTL_WINDOW" ]; then
    ctl_commit=$(git log -1 --format=%h -- "$CTL_PATH")
    if [ -n "$ctl_commit" ]; then
        CTL_WINDOW="${ctl_commit}~1..${ctl_commit}"
    fi
fi
CTL_N=0
if [ -n "$CTL_WINDOW" ]; then
    CTL_N=$(git rev-list --count ${CTL_WINDOW} -- "$CTL_PATH" 2>/dev/null || echo 0)
fi
if [ "$CTL_N" -lt 1 ]; then
    echo "o-ui-tick: UNMEASURED-CONTROL-CANNOT-DISAGREE(window='${CTL_WINDOW}' path='${CTL_PATH}' returned ${CTL_N}; a re-pin whose instrument has not been seen to fire is decoration — G-115)" >&2
    exit 2
fi
echo "o-ui-tick: control-can-disagree YES(${CTL_PATH}=${CTL_N} over ${CTL_WINDOW})"

# ---- rule 6: the diffstat beside the count -----------------------------
STAT=$(git diff --stat "${S}..${U}" | tail -1 | sed 's/^ *//')
echo "o-ui-tick: window-diffstat ${STAT:-none}"

# ---- rule 2/3: the re-pin, with the count welded to the line -----------
line=""
for p in $REPIN_PATHS; do
    c=$(git rev-list --count "${S}..${U}" -- "$p")
    line="${line}${p}=${c} "
done
echo "o-ui-tick: narrow-pin-4-paths ${line}(INCLUSION LIST — blind to every path not named here)"

# The primary reading: everything that changed EXCEPT the deliberately excluded
# prefixes. Fails loud -- an unanticipated directory appears in this list.
changed=$(git diff --name-only "${S}..${U}")
kept=""
nkept=0
for f in $changed; do
    skip=0
    for pre in $NON_PRODUCT_PREFIXES; do
        case "$f" in "$pre"*) skip=1; break ;; esac
    done
    [ "$skip" -eq 1 ] && continue
    nkept=$((nkept + 1))
    [ "$nkept" -le 8 ] && kept="${kept}${f} "
done
if [ "$nkept" -eq 0 ]; then
    echo "o-ui-tick: changed-outside-excluded 0 files OVER ${N} commits (excluded prefixes: ${NON_PRODUCT_PREFIXES})"
else
    more=""
    [ "$nkept" -gt 8 ] && more="… and $((nkept - 8)) more"
    echo "o-ui-tick: changed-outside-excluded ${nkept} files OVER ${N} commits -> ${kept}${more}"
fi

# ---- rule 6 again, for the lane itself ---------------------------------
AB=$(git rev-list --left-right --count "${UNTIL}...HEAD" | tr '\t' '/')
LSTAT=$(git diff --stat "${UNTIL}...HEAD" | tail -1 | sed 's/^ *//')
echo "o-ui-tick: lane-vs-${UNTIL} behind/ahead ${AB} · diffstat ${LSTAT:-none}"

# ---- rule 7: the queue is tested against the graph, not asserted -------
echo "o-ui-tick: queue-is-NOT-checked-here(read reports/O-UI-log.md and ancestor-test each item; a list in a prompt is a snapshot of when it was armed)"

# ---- G-119: name what was NOT measured, in the output ------------------
echo "o-ui-tick: NOT-MEASURED quiescence(a ref may move during this run) · test-results(no suite is run or read) · other-trees(no path outside $(basename "$(pwd)") is touched)"
echo "o-ui-tick: OK"
exit 0
