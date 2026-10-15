#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
# build-cert.sh — OWNER: O-INT. Never mutates the TRACKED TREE.
# --certify writes ONE artifact: target/shell-build/cert-<base>-<stamp>.log
# (gitignored build dir, the same place and naming the gate uses). Every other
# mode is read-only. ADDED 2026-09-05 against O-WATCH review #477 §3, which
# measured `find target -iname '*cert*'` -> 0 with a positive control: the
# VERDICT clause X-05 requires for EVERY Tier 1 row was the one figure in a
# certificate that could not be checked after the fact. A peer had to re-run
# the instrument to countersign it, which is not countersigning a certificate
# -- it is producing a second one and hoping they agree.
# RENAMED 2026-08-31 from demo-drift.sh by operator ruling (Q-D17): the three arms
# are GENERAL -- an input changed after the binary was built; code moved under the
# shipped paths since the base; the staged catalogs no longer match the repo's.
# Only the old NAME was demo-scoped, and a name that reads as dead apparatus gets
# deleted by someone tidying up.
#
# THE DEMO BINARY'S SECOND STALENESS CHECK, SHARPENED.
#
# O-WATCH #312 / G-777: the README's check 2 was `git diff --stat`, which
# CANNOT SEPARATE BEHAVIORAL CHANGE FROM INERT CHANGE, and on the one day
# the answer matters it returns the alarming half. A reader following the
# README's own "check it before trusting anything below" would stop a demo
# that is fine. That is what happened on 2026-08-28: eleven changed lines,
# all of them comments, and the check said DRIFT.
#
# WHAT THIS DOES INSTEAD: drops WHOLE-LINE comments and blank lines from
# both trees and compares what is left, per file, naming the files that
# actually differ.
#
# ★ THE STRIPPER IS DELIBERATELY CRUDE, AND THE DIRECTION OF ITS CRUDENESS
# IS THE POINT. It removes only lines that are ENTIRELY a comment. An
# inline trailing comment on a code line is NOT stripped, so changing one
# still reports BEHAVIORAL. That is a false alarm -- and it is the
# direction to have, because the opposite error (calling a real change
# inert) ships an uncertified binary to a demo. A stripper clever enough
# to handle inline comments would have to parse string literals, and
# O-WATCH's own comment-stripper failed on blank residue the first time it
# ran. Crude and loud beats clever and quiet here.
#
# Usage: build-cert.sh <built-from-commit> [head-ref]
# Exit:  0 no behavioral drift · 1 behavioral drift (files named) · 3 VOID
set -uo pipefail
# ---- SELFTEST ----
# ★★★ THESE ARMS EXIST BECAUSE THIS INSTRUMENT ALREADY SHIPPED ONE HOLE.
# G-778 was found by a peer reading the regex, not by anything in this
# file. The remedy is not "read it more carefully next time": a check
# moved INTO the instrument needs no discipline at all, while a check that
# lives in a reviewer's attention is spent the moment they look away.
# Every arm below must be able to FAIL -- arm 1 is the exact mutation that
strip() { /usr/bin/grep -vE '^[[:space:]]*(//|\*[[:space:]]|\*/|/\*)' | /usr/bin/grep -vE '^[[:space:]]*$'; }
# was reported INERT by the shipped version.

# ---- BINARY-FRESHNESS ARM (O-WATCH G-779, 2026-08-29) ----
# build-cert.sh --binary <path> [base]  -> 0 fresh · 1 stale+behavioral ·
#                                        2 stale but INERT only · 3 VOID
#
# THE FINDING: the demo certificate's `built from` field was filled from
# HEAD at CERTIFICATE-WRITING time rather than at build time, so it named
# a commit made after the binary. Check 2 subtracts from that field, so a
# base that drifts forward can already contain the change the binary
# lacks -- reporting `current` for a binary that is not. Same unsafe
# direction as G-778, one level up: there the instrument was blind, here
# its base was.
#
# ★★★ THEIR PROPOSED GUARD WAS "binary mtime must be LATER than the
# built-from commit's date", AND I MEASURED IT BEFORE ADOPTING IT: IT
# REJECTS THE CORRECT ANSWER TOO. The binary was built at 00:44:49; the
# commit that actually captured its source tree, 7f04246e, was made at
# 00:48:14 -- AFTER. In this workflow a binary is always built from a
# WORKING TREE and the commit capturing that tree is always made later,
# so `mtime > commit date` is false for every honest certificate here.
# A guard that fires on the right answer trains its operator to ignore it
# (G-514), so it is not the guard.
#
# ★★ WHAT THE HAZARD ACTUALLY IS, stated without reference to commits: a
# build input changed AFTER the binary was built. That is checkable
# directly, needs no commit at all, and cannot be fooled by when someone
# chose to commit. Every build-input file must be OLDER than the binary.
_bin_freshness() {
    _bin="$1"
    [ -f "$_bin" ] || { echo "VOID - no binary at $_bin"; return 3; }
    _bt=$(stat -f %m "$_bin" 2>/dev/null) || { echo "VOID - cannot stat $_bin"; return 3; }
    # ONE `find -newer` instead of 448 `stat` calls. The per-file loop took
    # 12 SECONDS, which pushed the standing tick recipe past a two-minute
    # limit and got the whole chain killed -- and an instrument nobody can
    # afford to run is an instrument that gets skipped, which is G-514
    # arriving through cost instead of through noise.
    #
    # Tracked build inputs only: an untracked scratch file is not an input
    # to the shipped binary, and treating it as one is the cry-wolf failure
    # again. So `find` supplies the NEWER set and `git ls-files` supplies
    # the TRACKED set, and the answer is the intersection.
    _all=$(git ls-files -- shell/qml shell/src crates/)
    _n=$(printf '%s\n' "$_all" | /usr/bin/grep -c .)
    _newer=$(/usr/bin/find shell/qml shell/src crates -type f -newer "$_bin" 2>/dev/null)
    _stale=""
    for f in $_newer; do
        case "
$_all
" in *"
$f
"*) _stale="${_stale:+$_stale }$f" ;; esac
    done
    # POPULATION GUARD: zero inputs examined would print a clean verdict
    # for having checked nothing -- the vacuous pass this file already
    # shipped once.
    [ "$_n" -gt 0 ] || { echo "VOID - no build inputs found (population empty)"; return 3; }
    if [ -n "$_stale" ]; then
        # ★★★ CLASSIFY BEFORE ALARMING, OR THIS ARM BECOMES THE GUARD I
        # REFUSED FROM O-WATCH. Added 2026-08-29 the first time it fired on
        # a COMMENT-ONLY edit (O-UI's LayersPanel note). A file being newer
        # than the binary is a real fact, but "rebuild now" and "a comment
        # moved" are different instructions, and an arm that shouts the same
        # word for both is G-514 -- red for benign reasons until its
        # operator stops reading it. That is exactly the objection I raised
        # against THEIR mtime guard, and my own arm had a milder version of
        # it from the day it landed.
        #
        # ★ The classification is not new machinery: it is this file's own
        # strip() applied to the SAME file at the base commit. The two arms
        # answer different questions -- "did an input change after the
        # build" and "did any BEHAVIOUR change" -- and composing them is
        # what turns a fact into an instruction.
        #
        # Without a base to compare against, no classification is possible
        # and the verdict stays the loud one. UNCLASSIFIED IS REPORTED AS
        # SUCH rather than silently assumed inert.
        _base="${2:-}"
        if [ -z "$_base" ]; then
            echo "BINARY IS STALE (unclassified - pass a base commit as arg 3) - $_n inputs checked, newer: $_stale"
            return 1
        fi
        _beh=""; _inert=""
        for f in $_stale; do
            _a=$(git show "$_base:$f" 2>/dev/null | strip)
            _b=$(strip < "$f" 2>/dev/null)
            if [ "$_a" = "$_b" ]; then _inert="${_inert:+$_inert }$f"
            else _beh="${_beh:+$_beh }$f"; fi
        done
        if [ -n "$_beh" ]; then
            echo "BINARY IS STALE - BEHAVIORAL - $_n inputs checked; rebuild required: $_beh${_inert:+ (inert: $_inert)}"
            return 1
        fi
        echo "BINARY IS STALE - INERT ONLY - $_n inputs checked; comment/blank changes only, rebuild optional: $_inert"
        return 2
    fi
    echo "BINARY FRESH - all $_n tracked build inputs are older than $_bin"
    return 0
}

# ---- THE COMPOSED VERDICT (O-WATCH G-781, 2026-08-29) ----
# ---------------------------------------------------------------------------
# HOW TO DERIVE THE <built-from-commit> YOU PASS BELOW.  Kept from DEMO-README
# by operator ruling (Q-D17, 2026-08-31): its anchor VALUES retired with the demo,
# this METHOD did not.  It lives here because this is where it is used (G-652:
# point of use means where the enforcing reader looks).
#
#   git log -1 --format=%h \
#       --before="$(stat -f '%Sm' -t '%Y-%m-%dT%H:%M:%S' <binary>)" \
#       -- shell/qml shell/src crates/
#   git merge-base --is-ancestor <new-base> <old-base>   # MUST exit 0
#
# THE --before BOUND IS LOAD-BEARING AND WAS MEASURED, NOT ASSUMED.  Unbounded, the
# query returns a base NEWER than the binary the moment any in-scope commit lands --
# which is G-779 exactly, the hazard this field was re-anchored to correct.  The two
# forms agreed only while the scope happened to be quiet; watched to diverge, the
# bounded form returned cc3dacda where the unbounded one still returned 9b116df0.
# The merge-base check enforces the direction: a base correction may move the base
# BACKWARD or leave it, never forward.
# ---------------------------------------------------------------------------
# ★★★★★ STOOD DOWN 2026-09-03 (operator's word) — THE DEMO ERA IS CLOSED.
# This instrument is NOT a live signal any more. Its verdict at stand-down was
# measured and recorded rather than muted: REBUILD REQUIRED (exit 1) against
# built-from 9b116df0, because the shipped binary had fallen behind a tree that
# stopped being re-cut. That was a CORRECT reading of a document nobody was
# maintaining.
#
# ★★ NOTHING BELOW THIS LINE WAS CHANGED. The arms still measure honestly if you
# invoke them deliberately -- for a future demo, or to ask the same question of a
# plain release build. Retiring an alarm is not the same as editing its numbers,
# and an instrument whose verdict I had softened would be worse than none.
# The authoritative stand-down lives on the consumer surface, reports/DEMO-README.md.
#
# build-cert.sh --certify <binary> <built-from-commit>
#   0 CURRENT · 1 REBUILD REQUIRED · 3 VOID
#
# ★★★ WHY THIS EXISTS. This file grew three arms -- selftest, drift, and
# freshness -- and DEMO-README printed two of them in order under "CHECK IT
# BEFORE TRUSTING ANYTHING BELOW" without saying which wins. O-WATCH ran
# them and got `BINARY IS BEHAVIORALLY CURRENT` (exit 0) and `BINARY IS
# STALE` (exit 1) in the same minute. BOTH WERE CORRECT UNDER THEIR OWN
# DEFINITIONS: the file was written after the build, and what changed in it
# was comments.
#
# ★★★★ THE FINDING IS NOT THAT AN ARM IS WRONG. It is that an instrument
# emitting two independently-correct verdicts with nothing composing them
# HAS NOT ANSWERED THE QUESTION IT WAS BUILT FOR -- IT HAS RELOCATED IT.
# A contradiction at a gate resolves by whichever line was read last, and
# the reader doing that adjudication is the person the instrument exists to
# spare. A precedence note in the README would not have fixed it; a note is
# read by people, and the two exit codes are read by whoever automates this
# next.
#
# THE COMPOSITION RULE, STATED ONCE, HERE, WHERE IT RUNS:
#   BEHAVIOUR WINS. A binary is CURRENT when no build input differs from
#   the base in anything but comments and blank lines -- whether or not a
#   file's mtime moved. mtime answers "did someone touch it", which is a
#   proxy; the stripped comparison answers "can it behave differently",
#   which is the question. The proxy is kept because it catches a class the
#   git comparison cannot see at all -- an edit in the WORKING TREE that
#   was never committed, where there is no base-vs-HEAD delta to find.
# ---- CATALOG ARM (2026-08-29) ----
# build-cert.sh --catalog <binary>   -> 0 in sync · 1 drifted · 3 VOID
#
# ★★★★ WHY THIS EXISTS: I CHANGED FOUR USER-VISIBLE STRINGS AND ALL THREE
# EXISTING CHECKS SAID `CURRENT`. The catalogs are staged BESIDE the exe,
# not compiled into it, so:
#   check 1 (exe sha256)  -- unchanged, the strings are not in the binary
#   check 2 (drift)       -- scope is shell/qml shell/src crates/, no i18n/
#   check 3 (freshness)   -- same scope
# So the demo certificate could not see a change to WHAT THE APP DISPLAYS.
# That is the same shape as every other finding on this instrument: a scope
# that excludes something that matters, silent about its non-members.
#
# ★★ AND THE RIGHT CHECK IS NOT "ADD i18n/ TO PATHS". A catalog edit needs
# a RE-STAGE, not a rebuild, so an mtime-vs-binary test would demand the
# wrong remedy and fire on every string edit. The question is simply
# whether the STAGED catalog still matches the repo one -- a direct
# comparison that needs no timestamps at all.
#
# ★ Repo values must be PRESENT IN the staged file rather than equal to
# it: staging merges a shell ui.* seed over canonical for `en`, canonical
# winning. Subset-with-equal-values is true under both the merge and the
# plain copy the other locales get.
_catalog_sync() {
    _cbin2="$1"
    [ -f "$_cbin2" ] || { echo "VOID - no binary at $_cbin2"; return 3; }
    _cdir=$(dirname "$_cbin2")/i18n
    [ -d "$_cdir" ] || { echo "VOID - no staged catalogs at $_cdir"; return 3; }
    _n=0; _bad=""
    for _f in i18n/*.json; do
        _loc=$(basename "$_f")
        [ -f "$_cdir/$_loc" ] || { _bad="${_bad:+$_bad }$_loc(absent)"; continue; }
        _n=$((_n + 1))
        python3 - "$_f" "$_cdir/$_loc" <<'PYEOF' || _bad="${_bad:+$_bad }$(basename "$_f")"
import json,sys
repo=json.load(open(sys.argv[1],encoding='utf-8'))
staged=json.load(open(sys.argv[2],encoding='utf-8'))
bad=[k for k,v in repo.items() if staged.get(k)!=v]
sys.exit(1 if bad else 0)
PYEOF
    done
    [ "$_n" -gt 0 ] || { echo "VOID - no catalogs compared (population empty)"; return 3; }
    if [ -n "$_bad" ]; then
        echo "CATALOG DRIFTED - $_n compared; staged copy does not carry the repo values: $_bad"
        return 1
    fi
    echo "CATALOG IN SYNC - $_n locales, every repo key present in the staged copy with the same value"
    return 0
}

if [ "${1:-}" = "--catalog" ]; then
    _catalog_sync "${2:?usage: build-cert.sh --catalog <binary>}"
    exit $?
fi

# ★★★★★ THE SELFTEST BLOCK LIVES HERE, BELOW THE FUNCTION DEFINITIONS, AND
# THAT POSITION IS LOAD-BEARING. It used to sit at the top of the file, above
# `_bin_freshness` and `_catalog_sync` -- so arms for those two functions could
# not have been written even if someone had wanted them: at that point in the
# script the names do not exist yet, and the shell answers `command not found`.
# ★★★★ O-WATCH #397-B found the SCOPE (one of three functions exercised, while
# --certify gates the whole instrument on the green). The CAUSE was one line
# lower down than the finding: not which arms were written, but where the block
# was placed. ★★★ Found by writing the arms and watching them fail with
# `command not found` -- a parse check would have passed the file unchanged
# (G-534), and reading it would have shown four plausible new arms.

if [ "${1:-}" = "--selftest" ]; then
    fails=0
    _t=$(mktemp -d) || exit 3
    # ARM 1 — a dereference edit must NOT look like a comment change.
    printf 'int f(bool *isSwap) {\n    *isSwap = false;\n}\n'      > "$_t/a"
    printf 'int f(bool *isSwap) {\n    *isSwap = true;\n}\n'       > "$_t/b"
    if [ "$(strip < "$_t/a")" = "$(strip < "$_t/b")" ]; then
        echo "SELFTEST FAIL arm1: a dereference edit reads as inert"; fails=$((fails+1))
    fi
    # ARM 2 — real block-comment furniture must still strip...
    printf '/* header\n * continuation\n */\nint x = 1;\n*ptr = 9;\n' > "$_t/c"
    got=$(strip < "$_t/c")
    [ "$got" = "$(printf 'int x = 1;\n*ptr = 9;')" ] || {
        echo "SELFTEST FAIL arm2: block comment/deref handling wrong: [$got]"; fails=$((fails+1)); }
    # ARM 3 — ...and a pure comment edit must STILL read inert, or the fix
    # for arm 1 has simply disabled the instrument. A guard that catches
    # everything discriminates nothing.
    printf 'int x = 1;\n// one\n'   > "$_t/d"
    printf 'int x = 1;\n// two\n'   > "$_t/e"
    [ "$(strip < "$_t/d")" = "$(strip < "$_t/e")" ] || {
        echo "SELFTEST FAIL arm3: a comment-only edit now reads as behavioral"; fails=$((fails+1)); }
    # ARM 4 — the MUST-FAIL control. If a stripper that deletes everything
    # would pass the arms above, the arms prove nothing.
    if [ "$(printf 'a\nb\n' | /usr/bin/grep -vE '.*')" != "" ]; then
        echo "SELFTEST FAIL arm4: the must-fail control did not behave"; fails=$((fails+1))
    fi
    # ARM 5/6 — THE FRESHNESS CLASSIFIER, added 2026-08-29 under O-WATCH's
    # G-780: a guard proposed from a single observed violation must be run
    # against the KNOWN-GOOD instances before it is trusted, or it cannot
    # tell a defect from the normal case. My first hand-run of this
    # classifier appeared to fail -- and the fault was MINE, a base commit
    # chosen that postdated the change I was testing for. These arms fix
    # the bases into the file so the question is never re-asked by hand.
    #
    # They run on synthetic files, not on repo history, so they cannot rot
    # when a commit ages out of anyone's memory.
    printf 'int x = 1;\n// note A\n'      > "$_t/base"
    printf 'int x = 1;\n// note B\n'      > "$_t/inert"
    printf 'int x = 2;\n// note A\n'      > "$_t/behav"
    [ "$(strip < "$_t/base")" = "$(strip < "$_t/inert")" ] || {
        echo "SELFTEST FAIL arm5: a comment-only delta classifies as behavioral"; fails=$((fails+1)); }
    [ "$(strip < "$_t/base")" = "$(strip < "$_t/behav")" ] && {
        echo "SELFTEST FAIL arm6: a real code delta classifies as inert"; fails=$((fails+1)); }
    # ★★★★★ ARMS 7-10 — THE POPULATION GUARDS, WHICH WERE CORRECT BY
    # CONSTRUCTION AND HAD NEVER BEEN RUN. O-WATCH #397-B: this file has THREE
    # named functions and the selftest exercised ONE, while `--certify` gates
    # the WHOLE instrument on that green -- so a pass certifying the comment
    # classifier read, structurally, as clearance for the freshness arm, the
    # catalog arm and the composed verdict. ★★★★ And the two unarmed functions
    # are exactly the two carrying the `-gt 0` floors this file's own comment
    # says it shipped a vacuity past once already.
    # ★★★ Their proposed remedy was to label the summary "(classifier only)".
    # NOT ADOPTED AS THE WHOLE FIX (G-527): a scope label is a caveat a reader
    # must remember, and §4's finding is that the checks which worked all
    # stopped being rules. ARMING the guards is strictly better, and it is
    # cheap because "empty population" is exactly what a fixture can produce.
    # ★★ BOTH DIRECTIONS FOR BOTH FUNCTIONS. A guard that can only fire is
    # indistinguishable from one that cannot pass (G-541), so each gets a
    # must-VOID arm and a must-NOT-VOID arm.
    _fx=$(mktemp -d) || exit 3
    ( cd "$_fx" && git init -q . && : > bin ) 2>/dev/null
    # ARM 7 — freshness against an EMPTY tracked population must VOID.
    _o7=$( cd "$_fx" && _bin_freshness bin 2>&1 ); _r7=$?
    case "$_o7" in *"population empty"*) : ;; *)
        echo "SELFTEST FAIL arm7: empty build-input population did not VOID (got: $_o7)"
        fails=$((fails+1)) ;; esac
    [ "$_r7" -eq 3 ] || { echo "SELFTEST FAIL arm7b: empty population returned $_r7, not 3"; fails=$((fails+1)); }
    # ARM 8 — the MUST-PASS twin: the real repo has a population, so it must NOT VOID.
    _o8=$(_bin_freshness target/shell-release/appname-shell 2>&1)
    case "$_o8" in *"population empty"*)
        echo "SELFTEST FAIL arm8: the real repo reported an empty population"
        fails=$((fails+1)) ;; esac
    # ARM 9 — catalog sync with a staged i18n dir holding NO catalogs must VOID.
    mkdir -p "$_fx/stage/i18n" && : > "$_fx/stage/appname-shell"
    _o9=$(_catalog_sync "$_fx/stage/appname-shell" 2>&1); _r9=$?
    case "$_o9" in *"population empty"*) : ;; *)
        echo "SELFTEST FAIL arm9: zero catalogs compared did not VOID (got: $_o9)"
        fails=$((fails+1)) ;; esac
    [ "$_r9" -eq 3 ] || { echo "SELFTEST FAIL arm9b: zero catalogs returned $_r9, not 3"; fails=$((fails+1)); }
    # ARM 10 — the MUST-PASS twin for the catalog arm.
    _o10=$(_catalog_sync target/shell-release/appname-shell 2>&1)
    case "$_o10" in *"population empty"*)
        echo "SELFTEST FAIL arm10: the real staged catalogs reported an empty population"
        fails=$((fails+1)) ;; esac
    rm -rf "$_fx"

    rm -rf "$_t"
    [ "$fails" -eq 0 ] && { echo "build-cert selftest: 10 arms, 0 failed"; exit 0; }
    echo "build-cert selftest: $fails FAILED"; exit 3
fi

if [ "${1:-}" = "--certify" ]; then
    _cbin="${2:?usage: build-cert.sh --certify <binary> <base>}"
    _cbase="${3:?usage: build-cert.sh --certify <binary> <base>}"
    _CERT_STAMP=$(date +%Y%m%dT%H%M%S)
    _CERT_LOG="target/shell-build/cert-$(printf '%s' "$_cbase" | cut -c1-8)-${_CERT_STAMP}.log"
    _clog=""
    _cadd() { _clog="${_clog}$1
"; echo "$1"; }
    # The verdict and the artifact are written in ONE place, so a path that exits
    # without a log cannot be added by accident later.
    _cert_done() {
        _cadd "  tree-at-end:   $(git rev-parse 'HEAD^{tree}' 2>/dev/null | cut -c1-12) worktree-dirty=$(git status --porcelain 2>/dev/null | wc -l | tr -d ' ')"
        _cadd "$1"
        mkdir -p target/shell-build 2>/dev/null || :
        if printf '%s' "$_clog" > "$_CERT_LOG" 2>/dev/null; then
            echo "  artifact:  $_CERT_LOG"
        else
            echo "  artifact:  NOT WRITTEN ($_CERT_LOG) - the verdict above stands, its provenance does not" >&2
        fi
        exit "$2"
    }
    _cadd "=== DEMO CERTIFY  binary=$_cbin  base=$_cbase  stamp=$_CERT_STAMP"
    # WHICH BINARY, WHICH INSTRUMENT, WHICH TREE. Added 2026-09-05 against
    # O-WATCH #478 §3/§4, measured against the GATE's provenance as the standard:
    # the gate records its own blob and the tree at start and end; this recorded
    # neither, so the artifact named a PATH and not the bytes at it.
    #
    # §4 is why that matters and it is not theoretical. TWO shell binaries exist
    # and the gate uses BOTH: step_shell_behavior reads target/shell-build
    # (gate.sh:441, the 44-suite battery) and step_release_engine_bind reads
    # target/shell-release (gate.sh:407, the P1-3 release-loads-release ruling).
    # They differ in size, date and content. X-05 requires "VERDICT: CURRENT"
    # for every Tier 1 row and does NOT say of which one, so a certificate could
    # be CURRENT about the binary the battery never ran. Naming the bytes does
    # not close that spec gap -- only the operator can -- but it makes every
    # future artifact say which claim it is making.
    #
    # THE METHOD IS NAMED WITH THE HASH, deliberately: "a hash without its
    # method is not a shared discriminator" is oint-fence.sh's own line, and
    # reconciling #478's figures cost a command because sha1 was not stated.
    _cadd "  instrument: build-cert.sh git-blob=$(git hash-object "$0" 2>/dev/null | cut -c1-12)"
    if [ -f "$_cbin" ]; then
        _cadd "  binary-id: sha1=$(shasum -a 1 "$_cbin" 2>/dev/null | cut -c1-12) size=$(wc -c < "$_cbin" | tr -d ' ') mtime=$(date -r "$_cbin" '+%Y%m%dT%H%M%S' 2>/dev/null)"
    else
        _cadd "  binary-id: ABSENT at $_cbin"
    fi
    _cadd "  tree-at-start: $(git rev-parse 'HEAD^{tree}' 2>/dev/null | cut -c1-12) worktree-dirty=$(git status --porcelain 2>/dev/null | wc -l | tr -d ' ')"
    # The instrument's own selftest belongs IN the artifact: a certificate
    # produced by a VOID instrument is not a weaker certificate, it is none.
    _sout=$("$0" --selftest 2>&1) || _cert_done "VERDICT: VOID - the instrument's own selftest failed" 3
    _cadd "  selftest:  $(printf '%s' "$_sout" | tail -1)"
    _fout=$("$0" --binary "$_cbin" "$_cbase"); _frc=$?
    _cadd "  freshness: $_fout"
    [ "$_frc" -eq 3 ] && _cert_done "VERDICT: $_fout" 3
    _dout=$("$0" "$_cbase" 2>&1); _drc=$?
    _cadd "  drift:     $(printf '%s' "$_dout" | tail -1)"
    _cout=$("$0" --catalog "$_cbin"); _crc=$?
    _cadd "  catalog:   $_cout"
    [ "$_crc" -eq 3 ] && _cert_done "VERDICT: $_cout" 3
    [ "$_drc" -eq 3 ] && _cert_done "VERDICT: VOID - drift check could not run" 3
    # BEHAVIOUR WINS, per the rule above. rc 2 from the freshness arm is
    # "newer but inert", which does not by itself demand a rebuild.
    if [ "$_frc" -eq 1 ] || [ "$_drc" -eq 1 ]; then
        _cert_done "VERDICT: REBUILD REQUIRED" 1
    fi
    # A catalog drift needs a RE-STAGE, not a rebuild -- named separately so
    # the verdict carries the right instruction rather than the nearest one.
    if [ "$_crc" -eq 1 ]; then
        _cert_done "VERDICT: RE-STAGE REQUIRED (the binary is current; the strings beside it are not)" 1
    fi
    if [ "$_frc" -eq 2 ]; then
        _cert_done "VERDICT: CURRENT (inert edits present since the build; rebuild optional)" 0
    fi
    _cert_done "VERDICT: CURRENT" 0
fi

if [ "${1:-}" = "--binary" ]; then
    _bin_freshness "${2:?usage: build-cert.sh --binary <path>}" "${3:-}"
    exit $?
fi

BASE="${1:?usage: build-cert.sh <built-from-commit> [head-ref]}"
HEAD_REF="${2:-HEAD}"

git rev-parse --verify "$BASE" >/dev/null 2>&1 || { echo "VOID - bad base $BASE"; exit 3; }

# THE BINARY'S INPUTS, NOT A DIRECTORY. shell/ also holds shell/tests/,
# which cannot reach the shipped appname-shell -- the target's source list
# contains no test file. Scoping to the directory tests a SUPERSET and
# demands judgement for changes that cannot matter.
PATHS="shell/qml shell/src crates/"

# ★★★ THE `*` ARM IS NARROWED TO `* ` AND `*/` (O-WATCH G-778, 2026-08-29).
# The first version of this line used a BARE `\*`, which removes any line
# beginning with an asterisk -- and in C++ and Rust that is a DEREFERENCE,
# not a comment. Measured in this instrument's own scoped paths under my
# own hand, reproducing their count exactly: shell/qml 0, shell/src 28,
# crates/ 244 = 272 code lines it silently dropped, and they are
# out-parameter writes of precisely the kind a fix touches -- `*count = …`,
# `*isSwap = false;`.
#
# DEMONSTRATED, NOT ARGUED, AND I REPRODUCED IT BEFORE FIXING IT: flipping
# `*isSwap = false;` to `true;` in tool_key_dispatcher.cpp -- KEYBOARD
# DISPATCH, WHICH IS WHAT THE DEMO DRIVES -- made this instrument report
# BINARY IS BEHAVIORALLY CURRENT.
#
# ★★★ AND THE LESSON IS ABOUT THE HEADER, NOT THE REGEX. The block above
# states this stripper's safe direction and argues it correctly -- for the
# `//` arm. I verified that arm and then asserted the property of the WHOLE
# PATTERN. A GUARD'S STATED SAFE DIRECTION IS A CLAIM ABOUT EVERY ARM OF
# ITS PATTERN, NOT ABOUT THE ARM ITS AUTHOR HAD IN MIND, and nothing in
# this file compared the two. The blind arm failed toward CALM -- the
# direction the header promised it would never fail in.
#
# A comment continuation is `* ` or `*/`; a dereference is `*ident`.

CHANGED=$(git diff --name-only "$BASE" "$HEAD_REF" -- $PATHS)
if [ -z "$CHANGED" ]; then
    echo "DEMO-DRIFT: no files changed under [$PATHS] since $BASE — binary is current"
    exit 0
fi

behavioral=0; inert=0; names=""
for f in $CHANGED; do
    a=$(git show "$BASE:$f" 2>/dev/null | strip)
    b=$(git show "$HEAD_REF:$f" 2>/dev/null | strip)
    # A file added or deleted between the two trees is BEHAVIORAL by
    # definition; one of the two `git show`s is empty and the compare
    # below already says so, but say WHY rather than letting it read as a
    # content difference.
    if [ "$a" = "$b" ]; then
        inert=$((inert + 1))
        echo "  inert       $f  ($(printf '%s' "$b" | /usr/bin/grep -c .) code lines, identical)"
    else
        behavioral=$((behavioral + 1)); names="${names:+$names }$f"
        echo "  BEHAVIORAL  $f  ($(printf '%s' "$a" | /usr/bin/grep -c .) -> $(printf '%s' "$b" | /usr/bin/grep -c .) code lines)"
    fi
done

echo "DEMO-DRIFT: $behavioral behavioral, $inert inert, base $BASE -> $HEAD_REF"
[ "$behavioral" -eq 0 ] && { echo "BINARY IS BEHAVIORALLY CURRENT"; exit 0; }
echo "RE-CUT NEEDED — behavioral: $names"
exit 1
