#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# WHICH ROOT DOES A RECORDED ARTIFACT LIVE AT? Resolve a build product BY CONTENT.
#
# ⛔ WHY THIS EXISTS. Three times in two days a desk reported a recorded artifact
# missing or changed, and was measuring a DIFFERENT worktree's copy of the same
# filename: O-WATCH #662 ("the artifact is gone" -- it was in the sibling worktree,
# entry 858), #678 §4 (":906 is orphaned" -- intact at build/, entry 874), and the
# near-misses in between. `appname-wt/` holds 36 git worktrees; one filename can
# name 59 different files.
#
# ★★★★★ THE RULE, O-WATCH #679 §2, adopted here: when a record carries a PATH and a
# HASH, THE HASH IS THE ADDRESS. Resolve by CONTENT, then check the path agrees.
# `[ -e path ]` answers "does something with this name exist", which across 36
# worktrees is very nearly no information at all.
#
#   usage: oint-whichroot.sh <sha256-prefix> <filename> [size-bytes]
#          -- the size is the CHEAP HALF of a recorded identity and prunes the hash
#             work; records print both, so pass it. Without it every copy is hashed:
#             59 copies x 9.5 MB took over a minute on this host, which is a tool
#             nobody will run. Cost measured, not guessed (entry 875).
#          oint-whichroot.sh --census <filename>            # every copy, with its hash
#
# Exit: 0 found (or census printed) · 1 NOT FOUND anywhere · 2 UNMEASURED · 3 VOID

set -u
FAMILY=$(cd "$(dirname "$0")/../../.." && pwd)   # .../appname-wt

usage() { sed -n '5,22p' "$0"; }
case "${1:-}" in -h|--help|"") usage; [ -z "${1:-}" ] && exit 3 || exit 0 ;; esac

command -v shasum >/dev/null 2>&1 || { echo "whichroot: UNMEASURED-NO-SHASUM" >&2; exit 2; }

if [ "$1" = "--census" ]; then
    name="${2:?usage}"
    n=0
    while IFS= read -r f; do
        printf '%s  %s  %s\n' "$(shasum -a 256 "$f" | cut -c1-16)" "$(/usr/bin/stat -f '%10z' "$f")" "${f#"$FAMILY"/}"
        n=$((n+1))
    done < <(find "$FAMILY" -name "$name" -type f 2>/dev/null | sort)
    echo "whichroot: census $n copy(ies) of $name under $FAMILY"
    [ "$n" -gt 0 ] || exit 1
    exit 0
fi

want="${1:?usage}"; name="${2:?usage}"; wantsz="${3:-}"
[ "${#want}" -ge 8 ] || { echo "whichroot: VOID - give at least 8 hex chars; a short prefix collides" >&2; exit 3; }
hits=0; seen=0; hashed=0
while IFS= read -r f; do
    seen=$((seen+1))
    # ★ SIZE FIRST. A size mismatch is a definitive non-match and costs a stat;
    # it cannot produce a false NEGATIVE, only skip work. A size MATCH proves
    # nothing (:907 is the same size as its predecessor and different bytes),
    # so the hash still decides.
    if [ -n "$wantsz" ] && [ "$(/usr/bin/stat -f '%z' "$f")" != "$wantsz" ]; then continue; fi
    hashed=$((hashed+1))
    h=$(shasum -a 256 "$f" | cut -d' ' -f1)
    case "$h" in "$want"*)
        printf 'whichroot: MATCH %s  %s B  %s\n' "$(printf '%s' "$h" | cut -c1-16)" "$(/usr/bin/stat -f '%z' "$f")" "${f#"$FAMILY"/}"
        hits=$((hits+1)) ;;
    esac
done < <(find "$FAMILY" -name "$name" -type f 2>/dev/null | sort)
# ★ THE POPULATION IS PART OF THE ANSWER. "0 matches" means nothing without it:
# a typo in the filename also returns 0, and looks identical to a real absence.
echo "whichroot: $hits match(es) among $seen copy(ies) of $name ($hashed hashed)"
[ "$seen" -gt 0 ] || { echo "whichroot: NOT-A-POPULATION(no file named $name anywhere under $FAMILY -- check the name before believing the zero)" >&2; exit 1; }
[ "$hits" -gt 0 ] || exit 1
exit 0
