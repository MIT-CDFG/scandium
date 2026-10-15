#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
# Acceptance-battery guards: denominator, tour fidelity, and the ctest extractor.
#
# WHY THIS FILE EXISTS AT ALL, and it is the point rather than a footnote:
# these guards were written and used for a full night of acceptance batteries
# WITHOUT EVER BEING TRACKED. They lived in a session-scoped temp directory
# outside the repository. `tour_ok`'s fidelity arm is the only instrument that
# has ever checked artifacts/tour-chrome against a fresh tour in this build —
# it caught 23 of 24 stale goldens on a lane where two rounds of green suites
# had missed them — and it was one session-close from ceasing to exist.
#
# THE RULE THAT PUT IT HERE: a phase-bound or session-bound instrument must be
# written with a home, or it degrades into a disposition — no green to go
# stale, no line to retire, no audit that can find it. THE BETTER THE
# UNDURABLE INSTRUMENT, THE MORE ITS EVAPORATION COSTS: a bad one leaves
# nothing behind either, but nobody was relying on it.
#
# TWO ARMS, AND THE SECOND IS THE ONE NOTHING ELSE HAS:
#   line ~"NO-AB-DIFFER"       DETERMINISM  - run A vs run B
#   line ~"NO-ARTIFACT-DIFFER" FIDELITY     - run A vs the COMMITTED goldens
# A double-run byte-identical check proves determinism, NOT fidelity. Two runs
# of one binary agreeing say nothing about whether either matches what is
# committed — the round-trip fallacy, satisfied by any consistent pair of
# errors. scripts/gate.sh has nine steps and none of them is the tour.
#
# Every function here REFUSES rather than returning a reassuring value, and
# every refusal is exercised by guards_selftest below. Run that before you
# trust any grant: a guard you have not watched refuse is not in service.
# The naive form `[ "$RANSUM" = "$DECLARED" ]` reports YES when BOTH are 0 —
# i.e. it passes having measured nothing. That is the {} == {} vacuity.
partition_ok() {   # $1=RANSUM  $2=declared  $3=floor(known minimum suite count)
  for v in "$1" "$2"; do
    case "$v" in ""|*[!0-9]*) echo "UNMEASURED-NONNUMERIC(ransum='$1' declared='$2')"; return;; esac
  done
  [ "$2" -ge "$3" ] || { echo "UNMEASURED-DECLARED-BELOW-FLOOR($2<$3)"; return; }
  [ "$1" = "$2" ] && echo "YES($1/$2)" || echo "NO-SUITES-SILENTLY-DROPPED($1/$2)"
}

# Tour guard. `diff -r` returns 0 for two EMPTY dirs, so AB=0 is vacuous when the
# tour produced nothing. Floors each side independently before comparing.
# ★★★★★ NAME THE MEMBERS, BECAUSE A COUNT IS NOT A SET AND THIS INSTRUMENT
# PROVED IT THE HARD WAY. On 2026-09-01 the fidelity arm returned
# NO-ARTIFACT-DIFFER(A=32 ART=32) inside a gate run; the SAME TREE passed on
# the next gate and on four idle re-runs, and the differing file could not be
# named because this function reported only counts and the run's temp dirs are
# discarded. An investigation that cannot start is worse than a red: it is a red
# that trains its reader to shrug (G-514).
#
# ★★ CAPPED ON PURPOSE. A wholesale mismatch would otherwise print the whole
# population into a gate line; six names identify a defect, thirty-two hide it.
# The cap ANNOUNCES itself (+N more) so a truncated list is never read as a
# complete one -- the inclusion-list failure this ledger keeps naming.
_tour_members() {   # $1=dirX $2=dirY -> "a.png b.png (+3 more)" or ""
  # ★ THE SAME EXCLUSION AS THE VERDICT ARMS, ADDED C-795. Without it this
  # helper reported a member the CHECK deliberately ignores: tour_ok excluded
  # *.md from the counts and both diff arms at C-790 and I did not carry the
  # exclusion here, so the failure line read "(+30 more)" over 36 members while
  # only 35 frames actually differed -- PROVENANCE.md making up the difference.
  # Found by cross-checking this message against an independent per-pixel
  # comparison, not by reading the code. A REPORT THAT NAMES WHAT THE CHECK
  # IGNORES SENDS ITS READER LOOKING FOR A FRAME THAT DOES NOT EXIST.
  _tm_all=$(diff -rq --exclude='*.md' "$1" "$2" 2>/dev/null | awk '
      /^Files .* differ$/ { n = split($2, p, "/"); print p[n]; next }
      /^Only in /         { print $NF; next }
    ' | LC_ALL=C sort -u)
  [ -n "$_tm_all" ] || { printf ''; return; }
  _tm_n=$(printf '%s\n' "$_tm_all" | wc -l | tr -d ' ')
  _tm_head=$(printf '%s\n' "$_tm_all" | head -6 | tr '\n' ' ' | sed 's/ $//')
  if [ "$_tm_n" -gt 6 ]; then printf '%s (+%s more)' "$_tm_head" "$((_tm_n - 6))"
  else printf '%s' "$_tm_head"; fi
}

# ★★★★★ MARKDOWN IS DOCUMENTATION, NOT A GOLDEN -- ADDED 2026-09-11 (O-UI,
# C-790) AGAINST A LIVE BREAKAGE, NOT A HYPOTHETICAL. On 09-11 10:00 another
# desk landed `artifacts/tour-chrome/PROVENANCE.md` INTO the goldens directory
# on the operator's word (H-06, a provenance line per image). This function
# counted and diffed EVERY entry in that directory, and the screenshot tour
# writes images only -- so the artifact arm began reporting
#   NO-ARTIFACT-DIFFER(A=40 ART=41 -- PROVENANCE.md)
# on a tree where every golden matched. Measured with the real directory and
# isolated with a PNG-only copy of it that returned YES in the same breath, so
# the cause is named rather than inferred.
#
# ★★★★ WHY EXCLUDE `*.md` AND NOT RESTRICT TO `*.png`: restricting to a format
# would silently drop a future golden that is not a PNG -- the failure mode
# where the guard gets quieter as the population grows. Excluding documentation
# states the actual contract ("a document beside the goldens is not one of
# them") and keeps every image format in scope.
#
# ★★★ THE EXCLUSION IS REPORTED, because an exclusion that excludes nothing is
# information too (the _excl_unmatched principle below): DOC=n appears in every
# verdict, so a reader can tell "no documents present" from "documents ignored"
# without reading this comment.
tour_ok() {   # $1=runA $2=runB $3=artifacts $4=floor
  for d in "$1" "$2" "$3"; do
    [ -d "$d" ] || { echo "UNMEASURED-MISSING-DIR($d)"; return; }
  done
  # `grep -c` exits 1 on zero matches and would abort a `set -e` caller, so the
  # count goes through `grep -v | wc -l` -- the recorded zsh/grep trap.
  a=$(ls -1 "$1" 2>/dev/null | grep -v '\.md$' | wc -l | tr -d ' ')
  b=$(ls -1 "$2" 2>/dev/null | grep -v '\.md$' | wc -l | tr -d ' ')
  r=$(ls -1 "$3" 2>/dev/null | grep -v '\.md$' | wc -l | tr -d ' ')
  doc=$(ls -1 "$3" 2>/dev/null | grep -c '\.md$' || true)
  for n in "$a" "$b" "$r"; do
    [ "$n" -ge "$4" ] 2>/dev/null || { echo "UNMEASURED-BELOW-FLOOR(A=$a B=$b ART=$r floor=$4 DOC=$doc)"; return; }
  done
  diff -r --exclude='*.md' "$1" "$2" >/dev/null 2>&1 || { echo "NO-AB-DIFFER(A=$a B=$b -- $(_tour_members "$1" "$2"))"; return; }
  diff -r --exclude='*.md' "$1" "$3" >/dev/null 2>&1 || { echo "NO-ARTIFACT-DIFFER(A=$a ART=$r -- $(_tour_members "$1" "$3"))"; return; }
  echo "YES(A=$a B=$b ART=$r DOC=$doc)"
}

# ---------------------------------------------------------------------------
# ★ THE EXTRACTOR, shipped WITH the guard — because leaving it to callers was
# my own violation of "control the stage that fails silently". partition_ok
# hardens the COMPARISON and assumes a number arrives; the observed failure
# mode is that NO number arrives. TWO independent batteries wrote their own
# extractor and BOTH were wrong in the same way: `[0-9]+ tests passed` cannot
# match "100% tests passed out of 12", because the digits are followed by `%`.
# One caught it via its control; the other only because partition_ok refused.
#
# Emits the count, or the EMPTY STRING for unmeasurable input — never 0.
# partition_ok already maps empty to UNMEASURED-NONNUMERIC.
shell_fresh_ok() {   # $1=binary $2=repo-root $3=build-dir ; stdin: dependency-graph input paths
  # FIDELITY OF THE RUNNER, not merely its existence. A MISSING binary was
  # already refused upstream; a STALE one was used silently and its output
  # attributed to the tree under test -- so the caller could not tell "this
  # tree renders differently" from "this binary is old", and it reported the
  # first. Measured cost: a 31-hour-old appname-shell turned an innocent
  # golden re-pin into a NO-ARTIFACT-DIFFER that read as a regression from a
  # 49-file landing. The goldens were correct.
  #
  # The rule is bound A's arm 2: a binary must not be older than any SOURCE in
  # its own dependency closure. Inputs arrive on STDIN so this stays a pure
  # function the selftest can exercise -- the ctest_ran shape. Generated
  # inputs under the build dir are excluded BY DESIGN (a binary may
  # legitimately postdate its own generated config); O-INT measured that
  # exclusion on a real graph: 225 inputs, 88 under the repo, 30 excluded,
  # ZERO of the 30 tracked by git. Toolchain paths outside the repo are not
  # ours to judge.
  # DEFAULT-EMPTY EXPANSIONS, and they are load-bearing. gate.sh runs under
  # `set -euo pipefail` (gate.sh:64). With a short argument list, a bare "$3"
  # is UNBOUND, so under -u the expansion ERRORS and this function DIES BEFORE
  # its own argument checks can refuse -- output empty, verdict silent. The
  # validation added to be un-bypassable was bypassed by the CALLER'S SHELL
  # OPTIONS, and the selftest could not see it because the selftest does not
  # run under -u. Caught only by running gate.sh end to end.
  local bin="${1-}" root="${2-}" build="${3-}" bm newest=0 newf="" n=0 nx=0 in m
  [ -n "$bin" ] && [ -e "$bin" ] || { echo "UNMEASURED-NO-BINARY($bin)"; return; }
  [ -n "$root" ] || { echo "UNMEASURED-NO-ROOT"; return; }
  # THE GUARD DEFENDS AGAINST ITS OWN CALLER (O-INT, 2026-08-26). They called
  # this with TWO arguments instead of three -- omitting the root, leaving
  # $build empty -- and got a confident YES(nsrc=29 excl=0). A SILENT WRONG
  # ANSWER, produced while testing the fix for exactly that class, one message
  # after we agreed the caller was the important question.
  # The call-site assertion (UNMEASURED-EXCLUSION-VACUOUS in gate.sh) cannot
  # help here: a direct call bypasses the call site. So the argument contract
  # is checked HERE, where no caller can route around it.
  [ -n "$build" ] || { echo "UNMEASURED-NO-BUILD-DIR"; return; }
  case "$build" in
    "$root"/*|"$root") ;;
    *) echo "UNMEASURED-BUILD-OUTSIDE-ROOT(build=$build not under root=$root)"; return ;;
  esac
  bm=$(stat -f %m "$bin" 2>/dev/null) || { echo "UNMEASURED-NO-STAT($bin)"; return; }
  while IFS= read -r in; do
    case "$in" in "$root"/*) ;; *) continue ;; esac
    case "$in" in "$build"/*) nx=$((nx + 1)); continue ;; esac
    [ -f "$in" ] || continue
    n=$((n + 1))
    m=$(stat -f %m "$in" 2>/dev/null || echo 0)
    [ "$m" -gt "$newest" ] && { newest=$m; newf=$in; }
  done
  # POSITIVE CONTROL, inline: a real closure has many sources. One or none
  # means the graph was not read, and a freshness verdict from an unread graph
  # is the vacuous green this guard exists to prevent.
  [ "$n" -ge 2 ] || { echo "UNMEASURED-NO-CLOSURE(nsrc=$n)"; return; }
  [ "$newest" -le "$bm" ] || { echo "STALE-RUNNER(bin=${bin##*/} src=${newf##*/})"; return; }
  # NON-VACUITY ON THE EXCLUSION (O-WATCH's requirement 4). An exclusion that
  # matches NOTHING fails silently and looks exactly like an exclusion that
  # found nothing to exclude -- the same family as a zero-population check
  # reading GREEN. That is precisely what hid the relative-vs-absolute bug:
  # the filter matched zero paths and reported a clean YES. Reporting the
  # excluded COUNT makes the filter's own work visible, so a caller passing a
  # path shape the filter cannot match shows up as excl=0 against a build dir
  # that demonstrably contains inputs.
  # THE TOKEN IS DIAGNOSTIC, NOT A CONTRACT (O-WATCH, 2026-08-26). Measured on
  # both refs: NOTHING PARSES IT -- 0 occurrences of `runner YES` anywhere on
  # main or lane/ui. Gating is RETURN-CODE based, which is the robust choice.
  # The residual their measurement exposes is NAMING: a string that reads like
  # a grant is a string a future reader may start parsing, and the format then
  # becomes load-bearing without anyone deciding it should. If you find
  # yourself matching on this text, add a field instead of relying on the
  # shape -- or make the contract explicit rather than inheriting it by habit.
  echo "YES(nsrc=$n excl=$nx)"
}

ctest_ran() {                       # stdin: raw ctest output
  awk '
    /No tests were found/ { found_none = 1 }
    # ANCHORED to the summary sentence ctest itself prints. The unanchored
    # /out of [0-9]+/ fired on EVERY line and END kept the LAST, so any text
    # after the summary containing "out of N" overwrote the real count -- and
    # "The following tests FAILED:" is printed AFTER the summary. Found by
    # O-WATCH 3, reproduced here: a summary saying 38 with a failure list saying
    # "12 out of 9 assertions" yielded 9. WORSE, corruption to the SAME number as
    # declared forged a green -- "1 tests failed out of 39" plus "12 out of 39
    # assertions" gave 39, and partition_ok(39,39) read YES on output that says a
    # test FAILED.
    # FIRST match, not last. Anchoring the PHRASE narrowed the trigger and left
    # the RULE intact -- `{n=...}` fires per line and END prints the LAST, so any
    # post-summary line carrying the phrase still overwrites. O-WATCH 3 called the
    # residual. The stream that proves it is a SILENT SHRINK, not a failure:
    #   "100% tests passed, 0 tests failed out of 7"    <- 7 suites actually ran
    #   "some suite printed: 0 tests failed out of 39"  <- post-summary, phrase
    # last-match -> 39 -> partition_ok YES(39/39), and ctest rc is 0 because
    # nothing FAILED, so the ctrc branch in gate.sh never fires: a gate PASS on a run
    # that executed 7 of 39 suites. First-match is correct by the output
    # ORDER as ctest emits it -- the summary prints once, before the failure
    # list -- so no text after it can reach the value, whatever it says.
    # THE PHRASE ABOVE IS NOT THE ONLY SUMMARY CTEST PRINTS. When NOTHING
    # fails, ctest OMITS the failure clause entirely:
    #      98% tests passed, 1 tests failed out of 43   <- the form above
    #     100% tests passed out of 43                   <- zero failures
    # This matched only the first, so it returned EMPTY on a clean run,
    # gate.sh read UNMEASURED-NONNUMERIC, and shell-behavior FAILED.
    # THE FIRST FULLY GREEN SHELL SUITE IS WHAT BROKE ITS OWN COUNTER
    # (2026-08-30, ctest rc=0, 100% of 43). Every fix logged above hardened
    # this parser against CORRUPTED FAILURE output. Nobody had ever fed it a
    # PASS. A guard exercised only by the failing case is half-tested, and
    # the untested half is the one that reports success.
    # Both phrases are anchored and both are FIRST-match, so the
    # silent-shrink defence is unchanged; the failed-form still WINS when a
    # stream carries both, because that is the form that proves a real
    # summary line rather than incidental prose.
    # (No apostrophes in this block: the program is single-quoted in sh, and
    # one apostrophe here closes it. Learned the direct way, this tick.)
    match($0, /tests failed out of [0-9]+/) { if (n == "") n = substr($0, RSTART + 20, RLENGTH - 20) }
    match($0, /tests passed out of [0-9]+/) { if (p == "") p = substr($0, RSTART + 20, RLENGTH - 20) }
    END { if (found_none) exit; if (n != "") print n; else if (p != "") print p }
  '
}

# ★ AND ITS SELF-CONTROL, so the extractor cannot be trusted un-exercised.
# Prints PASS/FAIL per case; returns non-zero if any case fails. Run it BEFORE
# using ctest_ran, the way you would exhibit a refusal before believing a grant.
# ★ The inputs are deliberately drawn from REAL ctest output, not from strings
# written alongside the extractor — a control sharing provenance with the
# instrument shares its blind spots.
ctest_ran_selftest() {
  local rc=0 got
  # $1=expected  $2=label  $3..=sample LINES -- NOT stdin.
  #
  # C-3xx: THE ARMS WERE NOT WIRED TO THE VERDICT. Every arm was `printf ... | _c`,
  # and the right-hand side of a pipeline runs in a SUBSHELL, so `rc=1` here was
  # set in a child and discarded. ctest_ran_selftest returned 0 WITH FAIL LINES ON
  # SCREEN -- proven by planting last-match: 1 FAIL printed, return code 0. That
  # made guards_selftest line ~575 (`ctest_ran_selftest || rc=1`) dead wiring, and
  # through it gate.sh:320 (`if ! _gs=$(guards_selftest ...)`) -- the step whose
  # whole job is to verify the guards before the gate trusts them. Eight arms
  # could not reach the verdict they existed to produce.
  # Taking the sample as ARGUMENTS keeps _c in the current shell, so rc survives.
  _c() {
    local want="$1" label="$2"; shift 2
    got=$(printf '%s\n' "$@" | ctest_ran)
    set -- "$want" "$label"
    if [ "$got" = "$1" ]; then printf '  PASS %-28s -> %s\n' "$2" "[$got]"
    else printf '  FAIL %-28s -> got [%s] want [%s]\n' "$2" "$got" "$1"; rc=1; fi
  }
  _c 12 "all-passed" "100% tests passed, 0 tests failed out of 12"       
  _c 2 "some-failed" "50% tests passed, 1 tests failed out of 2"         
  _c "" "vacuous-green" "No tests were found!!!"                            
  _c "" "empty-input" ""                                                  
  _c "" "no-count-line" "Total Test time (real) = 374.83 sec"               
  # THE ARM SET WAS INCOMPLETE IN ONE DIRECTION, which is the finding rather than
  # "untested": all five arms above feed streams whose LAST "out of N" IS the
  # summary. None fed text AFTER it -- the only shape that exposes last-match. A
  # function watched five times, never once in the position where it breaks.
  # General shape, per O-WATCH: AN INPUT WHERE THE THING BEING EXTRACTED IS NOT
  # THE LAST OCCURRENCE. Both arms below have it.
  _c 38 "post-summary-noise" "100% tests passed, 0 tests failed out of 38" "The following tests FAILED:" "  1 - x (Failed) 12 out of 9 assertions"
  _c 39 "not-last-occurrence" "50% tests passed, 1 tests failed out of 39" "The following tests FAILED:" "  1 - x (Failed) 12 out of 39 assertions"
  # THE ARM THAT DISTINGUISHES THE TWO RULES -- the one both arm sets lacked.
  # Every arm above returns the SAME value under first-match and last-match, so
  # the set could not see which rule shipped. Its post-summary text says
  # "12 out of N assertions", which the anchored pattern never matches under
  # either rule. This arm's post-summary line CARRIES THE ANCHORED PHRASE:
  # last-match -> 39 (forged), first-match -> 7 (true). It is the only arm that
  # reds if first-match is ever reverted.
  _c 7 "post-summary-carries-phrase" "100% tests passed, 0 tests failed out of 7" "some suite printed: 0 tests failed out of 39"
  unset -f _c
  return $rc
}

# ---------------------------------------------------------------------------
# ★ SELF-TEST FOR BOTH GUARDS, so the file proves itself rather than asking to
# be believed. Exercises every refusal arm AND a grant — a guard that only ever
# refuses is as useless as one that only ever grants, and neither is visible
# from a single result. Returns non-zero if any case misbehaves.
# ---------------------------------------------------------------------------
# ★ declared_set_ok — the DECLARED TEST SET of a build dir, or a refusal token.
# Designed by O-UI 5; controls re-verified by O-INT before landing.
#
# WHY THIS EXISTS: a dropped add_test registration is invisible from every
# direction except this one. The build succeeds, the suite runs, and
# `ctest -R <dropped-name>` EXITS 0 printing "No tests were found!!!" — the
# alarm is in the log text and the gate reads the exit code.
#
# ★★ AND THE GUARD CLAUSE IS LOAD-BEARING, NOT DEFENSIVE. Measured: pointing
# the extractor at a MISSING build dir yields an EMPTY SET, which `comm`
# then reports as "no differences" — indistinguishable from a clean
# comparison. That is the same trap one level up. An empty set must be
# UNMEASURED, never clean.
declared_set_ok() {   # $1=build-dir ; prints names one per line, or a token
  [ -d "$1" ] || { echo "UNMEASURED-NO-BUILD-DIR($1)"; return; }
  n=$(ctest --test-dir "$1" -N 2>/dev/null | sed -n 's/^  Test *#[0-9]*: *//p' | sort -u | wc -l | tr -d ' ')
  [ "$n" -gt 0 ] 2>/dev/null || { echo "UNMEASURED-EMPTY-SET($1)"; return; }
  ctest --test-dir "$1" -N 2>/dev/null | sed -n 's/^  Test *#[0-9]*: *//p' | sort -u
}

# ─────────────────────────────────────────────────────────────────────────────
# LEG A — REGISTRATION COMPLETENESS (operator ruling 2026-08-26).
#
# WHY THIS LANDS BEFORE EXECUTION. The gate NAMES its suites with `ctest -N`. A
# test binary that is BUILT but never `add_test`-registered is absent from that
# naming set, so wiring execution WITHOUT this check runs every named suite,
# passes, and never touches the unregistered one. Measured on main 2026-08-26:
# 11 built targets, 10 registered, delta `tst_g5_frameprobe` — the very test
# that measures the repaint change the Qt ruling exists to run.
#
# THE DIRECTION OF HARM INVERTS ONCE EXECUTION IS WIRED. `NOT-RUN` is honest —
# a reader sees a hole. A green over an incomplete naming set HIDES the absence,
# because a green reads as coverage. Executing an incomplete naming set is MORE
# dangerous than declaring one.
#
# The neighbouring trap is already documented in this file: a dropped `add_test`
# is invisible from every direction except this one, and `ctest -R <dropped>`
# EXITS 0 printing "No tests were found!!!". But that guard watches for set
# SHRINKAGE — and A SUITE THAT WAS NEVER REGISTERED NEVER SHRANK. Shrinkage
# detection is structurally unable to find this class.
#
# Every delta is either a documented exclusion NAMED IN THE OUTPUT, or a RED.
# G-124: an exclusion must name its EXPIRY CONDITION, not merely its reason, or
# it outlives what it protected against — as :1195's "reds by design until the
# repaint scoping is corrected" did, the scoping having merged at 907ee4c5.
#
# $1 = newline list of BUILT names   $2 = newline list of REGISTERED names
# $3 = space-separated documented exclusions
# Kept free of the filesystem so both arms are demonstrable in guards_selftest.
# _excl_unmatched -- an exclusion that excludes NOTHING is information, whichever
# way it happened: a stale exclusion whose suite is gone, a typo, or an argument
# in the wrong slot. O-UI 6 called `orphan_binaries_ok <bdir> <srcdir>` and the
# second path landed in $excl, where it matched nothing and WAS SILENTLY IGNORED
# -- the call returned a clean YES. They had credited the overlap precondition
# with catching that shape; it caught the OLD two-set signature for a different
# reason (two paths are two one-element sets sharing no names) and the current
# entry does not catch it at all.
#
# Reported, never refused: an exclusion naming a suite that is legitimately not
# built this configuration is valid, so this cannot be a hard error. It is a
# NOTICE appended to the verdict -- the caller decides.
_excl_unmatched() {                 # $1=excl string  $2=population (newline set)
    local excl="$1" pop="$2" e miss=""
    [ -n "$excl" ] || return 0
    for e in $excl; do
        printf '%s\n' "$pop" | grep -Fxq "$e" || miss="$miss$e
"
    done
    [ -n "$miss" ] || return 0
    printf '%s' "$miss" | grep . | tr '\n' ',' | sed 's/,$//'
}

registration_delta_ok() {
  # $4 = OPTIONAL test-source dir. Without it this stays filesystem-free.
  #
  # C-3xx, O-UI 6: "BINARY PRESENT, NOT REGISTERED" IS AMBIGUOUS AND THE TWO
  # READINGS HAVE OPPOSITE REMEDIES.
  #
  #     source EXISTS  -> added-but-unregistered -> REGISTER IT
  #     source ABSENT  -> retired-but-stale      -> DELETE THE BINARY
  #
  # The old wording, `UNREGISTERED-SUITE(x)`, names only the first. O-UI hit the
  # second on a live gate: `UNREGISTERED-SUITE(tst_interim_repaint)` on a binary
  # from Aug 17 whose source was DELETED at 6897f042 on main ("retire the §9.5
  # interim repaint at its designed self-disarm moment"). THE REMEDY THE MESSAGE
  # IMPLIES WOULD HAVE RESURRECTED A TEST RETIRED TEN DAYS EARLIER. The guard was
  # right that something was wrong and pointed at the exact opposite of the fix.
  #
  # G-119: the message IS the guard. A verdict that names a remedy it cannot
  # justify is worse than one that names none -- it sends a careful person the
  # wrong way, confidently. With $4 the two are separated; without it, the text
  # states BOTH readings and the disambiguator instead of picking one.
  local built="$1" reg="$2" excl="${3:-}" srcdir="${4:-}" n_b n_r named="" unreg="" stale="" phantom="" b r out="" _xu=""
  n_b=$(printf '%s\n' "$built" | grep -c . || true)
  n_r=$(printf '%s\n' "$reg"   | grep -c . || true)
  # POPULATION GUARDS. Zero on either side makes the comparison vacuous, and a
  # vacuous comparison must never read as agreement.
  [ "$n_b" -gt 0 ] || { echo "UNMEASURED-NO-BUILT-BINARIES"; return; }
  [ "$n_r" -gt 0 ] || { echo "UNMEASURED-NO-REGISTRATIONS"; return; }

  # OVERLAP ARM -- G-126, a gap in this SHIPPED guard, demonstrated before it was
  # fixed. FOR A SET COMPARISON THE CONTROL IS OVERLAP, NOT NON-EMPTINESS. Both
  # arms above passed while the two sets came from DIFFERENT UNIVERSES and
  # intersected at ZERO: `ninja -t targets` (not `targets all`) yielded 128 names
  # against 40 registered, no name in common, and the guard confidently reported
  # all 40 registered suites as phantoms and all 128 as unregistered. A wrong
  # finding delivered with both population guards green.
  #
  # Zero overlap is AMBIGUOUS between an extraction defect and total registration
  # collapse. It is reported UNMEASURED rather than as a delta because the two are
  # indistinguishable from here and UNMEASURED still STOPS the gate -- it does not
  # pass. The difference that matters is between "I cannot tell you what is wrong"
  # and a confident list of 40 problems that do not exist.
  local n_ov
  n_ov=$(comm -12 <(printf '%s\n' "$built" | grep . | sort -u) \
                  <(printf '%s\n' "$reg"   | grep . | sort -u) | grep -c . || true)
  [ "$n_ov" -gt 0 ] || { echo "UNMEASURED-ZERO-OVERLAP(built=$n_b registered=$n_r share 0 names -- different universes, or total registration collapse; indistinguishable from here)"; return; }

  # Lists accumulate NEWLINE-separated and are joined explicitly. An earlier
  # draft built space-separated lists and joined with `echo $list | tr ' ' ','`,
  # which relies on the CALLER'S SHELL word-splitting: bash splits and printed
  # "b,c"; zsh does not and printed ",b,c". Same code, two shells, two answers.
  # A guard whose OUTPUT depends on who sourced it is not a guard.
  _rj() { printf '%s' "$1" | grep . | tr '\n' ',' | sed 's/,$//'; }

  # DIRECTION 1 — BUILT but NOT REGISTERED: a silent drop. The suite exists and
  # the gate will never name it, so execution cannot reach it.
  for b in $(printf '%s\n' "$built" | grep .); do
    printf '%s\n' "$reg" | grep -Fxq "$b" && continue
      case " $excl " in
        *" $b "*) named="$named$b
" ; continue ;;
      esac
      # THE DISAMBIGUATOR, and it is one cheap test neither guard consulted.
      if [ -n "$srcdir" ] && [ ! -f "$srcdir/$b.cpp" ]; then
        stale="$stale$b
"
      else
        unreg="$unreg$b
"
      fi
  done

  # DIRECTION 2 — REGISTERED but NOT BUILT: a PHANTOM. Found on main 2026-08-26:
  # tst_auto_adjust_menu is listed by `ctest -N` as a test and has no binary;
  # ctest prints "Could not find executable" and STILL LISTS IT. So a phantom
  # INFLATES the declared count that partition_ok compares against its floor —
  # `suites-declared YES(38/38)` counted 37 real suites and one that cannot run.
  # ★ Once execution is wired a phantom is the MORE dangerous direction: it pads
  # the number that makes coverage look complete.
  for r in $(printf '%s\n' "$reg" | grep .); do
    printf '%s\n' "$built" | grep -Fxq "$r" || phantom="$phantom$r
"
  done

  # WITHOUT $4 the guard cannot tell the two readings apart, so it names BOTH and
  # the disambiguator instead of implying a remedy. WITH $4 they are separated and
  # each verdict states its OWN remedy.
  if [ -n "$unreg" ]; then
    if [ -n "$srcdir" ]; then
      out="UNREGISTERED-SUITE($(_rj "$unreg") -- source present, REGISTER IT)"
    else
      out="UNREGISTERED-SUITE($(_rj "$unreg") -- built and not registered; check whether a source exists before acting: source present means REGISTER IT, source absent means a RETIRED suite left its binary behind and the fix is to DELETE IT)"
    fi
  fi
  # STALE BINARY OF A RETIRED SUITE. The opposite remedy, and the one the old
  # single label sent people away from.
  [ -n "$stale" ]   && out="${out:+$out+}STALE-BINARY-RETIRED-SUITE($(_rj "$stale") -- NO SOURCE in $srcdir; the suite was retired and its binary remains. DELETE THE BINARY. DO NOT REGISTER IT)"
  # ORDER IS LOAD-BEARING: the EXCL notice must precede the PHANTOM component,
  # because registration_complete_ok strips the phantom half and everything after
  # it. Appended last, this notice was SILENTLY DISCARDED on the gate path --
  # measured, minutes after I added it.
  _xu=$(_excl_unmatched "$excl" "$built")
  [ -n "$_xu" ] && out="${out:+$out+}EXCL-MATCHED-NOTHING($_xu -- named as excluded but not in the built set: stale exclusion, typo, or an argument in the wrong slot)"
  [ -n "$phantom" ] && out="${out:+$out+}PHANTOM-SUITE($(_rj "$phantom"))"
  if [ -n "$out" ]; then echo "$out"
  elif [ -n "$named" ]; then echo "YES(built=$n_b registered=$n_r excluded=$(_rj "$named"))"
  else echo "YES(built=$n_b registered=$n_r)"; fi
}

# PHANTOM DIRECTION, from ctest's OWN view rather than a second method.
# $1 = newline list of "testname<TAB>commandpath" ("" command = ctest could not
# resolve one). A phantom is a registered test whose command is absent or does
# not exist on disk. ctest itself reports the absence -- json-v1 simply omits
# the "command" key -- so this reads ctest's verdict instead of inventing one.
phantom_tests_ok() {
  local rows="$1" n=0 ph="" name cmd line
  n=$(printf '%s\n' "$rows" | grep -c . || true)
  [ "$n" -gt 0 ] || { echo "UNMEASURED-NO-REGISTRATIONS"; return; }
  # TWO CAUSES, TWO LABELS. An earlier version collapsed both into PHANTOM-SUITE
  # and O-WATCH caught it with my own selftest: two arms, two causes, one
  # indistinguishable string. That is the UNMEASURED-versus-FAIL collapse filed
  # against main's run_step, one level down and inside a guard -- the care taken
  # to detect both never reached the verdict. The remedies differ:
  # ★ CORRECTED 2026-08-26 AFTER MEASURING THE MECHANISM RATHER THAN ASSUMING IT.
  # I first documented key-absence as "CMake could not resolve it -> a
  # registration problem". THAT IS BACKWARDS. CTestTestfile.cmake carries a
  # complete, correct command path for tst_auto_adjust_menu, identical in form to
  # every healthy entry; json-v1 omits the "command" key because CTEST SEARCHED
  # FOR THE EXECUTABLE AND COULD NOT FIND IT -- which is exactly the
  # "Could not find executable" line ctest -N prints for it. Registration is
  # fine; the BINARY was never built.
  # So BOTH labels report a missing binary and differ only in WHEN it went
  # missing. They are kept distinct because the remedies still differ slightly,
  # but the diagnosis text now says what is actually true:
  #   NOT-BUILT       ctest could not find the executable when it built its view
  #                   -> the target was never built in this build dir. BUILD
  #                      problem, not a CMake problem. (tst_auto_adjust_menu)
  #   MISSING-BINARY  ctest DID resolve a path and the file is gone NOW
  #                   -> deleted after configure. Narrow, but real, and
  #                      key-presence alone cannot see it.
  # ★ The label I shipped would have sent a responder to edit CMakeLists.txt when
  # the fix is to build the target. A wrong diagnosis is worse than none: none
  # makes you look, a wrong one makes you look somewhere else.
  # The two also answer different questions in different TENSES: key-presence
  # reports what ctest could resolve at configure, -e reports what is true now.
  # A registration whose binary was deleted after configure is real and
  # key-presence alone cannot see it. Same verdict class RED, different
  # diagnostic, so a responder does not re-derive what the guard already knew.
  local unres="" miss=""
  while IFS=$'\t' read -r name cmd; do
    [ -n "$name" ] || continue
    if [ -z "$cmd" ];      then unres="$unres$name
"
    elif [ ! -e "$cmd" ];  then miss="$miss$name
"
    fi
  done <<EOF
$rows
EOF
  local o=""
  [ -n "$unres" ] && o="PHANTOM-NOT-BUILT($(printf '%s' "$unres" | grep . | tr '\n' ',' | sed 's/,$//'))"
  [ -n "$miss" ]  && o="${o:+$o+}PHANTOM-MISSING-BINARY($(printf '%s' "$miss" | grep . | tr '\n' ',' | sed 's/,$//'))"
  if [ -n "$o" ]; then echo "$o"; else echo "YES(registered=$n)"; fi
}

# Filesystem wrapper: discovers BUILT binaries in $1, reads REGISTERED names on
# stdin (from `ctest -N`), delegates the comparison to registration_delta_ok.
registration_complete_ok() {   # $1=build dir  $2=documented exclusions  $3=test-source dir (optional)
  # $3 IS LOAD-BEARING AND WAS INVISIBLE TWICE OVER: absent from this header and
  # absent from the local declaration, while :567 forwarded it positionally as
  # registration_delta_ok's $4. O-WATCH filed the header; the missing `local` is
  # the half neither of us named. A header is the line a reader consults for the
  # signature -- understating the arity by one is a description that acquires a
  # maintenance debt the moment the code moves, and nobody is assigned it.
  local bdir="$1" excl="${2:-}" srcdir="${3:-}" built reg gen
  [ -d "$bdir" ] || { echo "UNMEASURED-NO-BUILD-DIR($bdir)"; return; }
  # STATED ASSUMPTION WITH ITS EXPIRY CONDITION (G-124 applied to an assumption
  # rather than an exclusion; from O-WATCH's rule-12 review of b830c256).
  # THE TWO SIDES OF THIS COMPARISON USE DIFFERENT METHODS: registered names come
  # from ctest's own view (ctest -N), built binaries from this find. A comparison
  # whose sides use different methods measures the METHOD DIFFERENCE as well as
  # the thing -- which is what produced "0 built" and ten inverted accusations
  # when a draft looked for add_executable in a tree that writes qt_add_executable.
  # A MULTI-CONFIG generator (Xcode, Visual Studio) emits into Release/ Debug/
  # RelWithDebInfo/ MinSizeRel/. ctest searches those; this find does not:
  #   built-in-subdir AND registered   -> read as PHANTOM. Fails CLOSED (a false
  #                                       RED sends someone looking). Safe.
  #   built-in-subdir AND UNREGISTERED -> absent from the built side, so no
  #                                       UNREGISTERED-SUITE fires. FAILS OPEN,
  #                                       in this guard's PRIMARY direction.
  # ★ NOT A GUESS ABOUT CMAKE -- A DOCUMENTED DIVERGENCE FROM WHAT CTEST DOES.
  # ctest names its own search paths when it cannot find an executable. Captured
  # verbatim from this build dir (paths abbreviated):
  #     Could not find executable <build>/tst_auto_adjust_menu
  #     Looked in the following places:
  #       <build>/tst_auto_adjust_menu
  #       <build>/Release/…   <build>/Debug/…   <build>/MinSizeRel/…
  #       <build>/RelWithDebInfo/…   <build>/Deployment/…   <build>/Development/…
  # SIX per-config subdirectories, 24 such candidate paths in one message.
  # THIS FIND SEARCHES ONE. The divergence is measured from ctest's own mouth
  # rather than argued from reasoning about generators -- and on this tree those
  # six directories do not exist (0 of 6 present), which is why the gap is latent
  # rather than absent.
  # EXPIRY CONDITION: sound only while the generator is single-config. Asserted
  # rather than assumed -- an assumption that is currently true and unstated is
  # the thing that rots.
  # ENUMERATE WHAT IS KNOWN SAFE AND REFUSE EVERYTHING ELSE.
  # The first draft of this assertion listed the multi-config generators it knew
  # (Xcode, Visual Studio) and defaulted the rest to safe -- an INCLUSION LIST OF
  # DANGEROUS, which is the same defect this assertion exists to close, written
  # into the fix for it. Measured against cmake 4.4.2's own generator list:
  #   Ninja Multi-Config          = "Generates build-<Config>.ninja files"  -> was ACCEPTED
  #   Green Hills MULTI                                                     -> was ACCEPTED
  #   Kate - Ninja Multi-Config                                             -> was ACCEPTED
  # Ninja Multi-Config is the NEIGHBOURING CHOICE to this repo's own Ninja -- the
  # single most probable generator change here is exactly the one that slipped
  # through, landing the guard back in the fail-open state while the stated
  # assumption still sat in the comment reading as if it were enforced.
  # An unrecognised, new, or mistyped generator now refuses LOUDLY. A single-config
  # generator missing from this list is a FALSE REFUSAL -- fails closed, someone
  # adds it. That is the correct direction to be wrong in.
  gen=$(sed -n 's/^CMAKE_GENERATOR:INTERNAL=//p' "$bdir/CMakeCache.txt" 2>/dev/null | head -1)
  case "$gen" in
    Ninja|"Unix Makefiles"|"NMake Makefiles"|"NMake Makefiles JOM"|\
    "MinGW Makefiles"|"MSYS Makefiles"|"Borland Makefiles"|"Watcom WMake") ;;
    *) echo "UNMEASURED-UNRECOGNISED-GENERATOR(${gen:-unreadable})"; return ;;
  esac

  # ── BOTH SIDES FROM ONE AUTHORITY (O-WATCH's rule, and the defect that forced
  # this rewrite). The first version compared ctest TEST NAMES against BINARY
  # NAMES. add_test decouples them: `add_test(NAME x COMMAND y)` is legal and
  # f3c94410 uses exactly that form --
  #   add_test(NAME tst_transform_repaint_footprint COMMAND tst_g5_frameprobe …)
  # Under name-equality that lands TWO FALSE FINDINGS on a change that fixes the
  # hole: tst_g5_frameprobe still "unregistered" (no test bears its name) and a
  # new phantom tst_transform_repaint_footprint (no binary bears that name).
  # Worse, it fails OPEN in general: a test invoking a binary under any other
  # name would leave that binary looking uncovered forever.
  # ctest --show-only=json-v1 carries each test's ACTUAL command, so coverage is
  # now "which BINARIES do registered tests invoke", and the phantom direction is
  # read out of ctest's own resolution rather than a second method.
  local _rows
  _rows=$(ctest --test-dir "$bdir" --show-only=json-v1 2>/dev/null | python3 -c '
import json,sys
try: d=json.load(sys.stdin)
except Exception: sys.exit(3)
ts=d.get("tests")
if not ts: sys.exit(4)
for t in ts:
    c=t.get("command") or []
    print("%s\t%s" % (t.get("name",""), c[0] if c else ""))
' 2>/dev/null) || { echo "UNMEASURED-CTEST-JSON-UNREADABLE($bdir)"; return; }
  [ -n "$_rows" ] || { echo "UNMEASURED-NO-REGISTRATIONS"; return; }

  # covered = basenames of commands that EXIST on disk
  local covered
  covered=$(printf '%s\n' "$_rows" | awk -F'\t' 'NF>1 && $2!=""{n=split($2,a,"/"); print a[n]}' | sort -u)
  built=$(find "$bdir" -maxdepth 1 -type f -perm -u+x -name 'tst_*' 2>/dev/null | sed 's|.*/||' | sort)

  local _d _p out=""
  # $3 = OPTIONAL test-source dir, passed through as registration_delta_ok's $4 so
  # an unregistered binary gets the RIGHT remedy instead of the both-readings text.
  #
  # ★ RESIDUAL IDENTITY, STATED AS AN ASSUMPTION WITH ITS MEASUREMENT (O-WATCH's
  # rule-12 ruling, and they were explicit that it must NOT be written as though
  # it were structural):
  #
  #     BINARY BASENAME -> SAME-NAMED SOURCE under $3
  #     MEASURED 40 binaries / 40 with a same-named source / 0 without,
  #     ON THIS TREE, 2026-08-27. Re-check it; it is not a guarantee.
  #
  # A binary built from a differently-named source breaks this the same way
  # `add_test(NAME x COMMAND y)` broke test-name == binary, which is the defect
  # already closed at :520. Both sides reaching THIS call are command-derived, so
  # the test-name identity is gone; this is the one behind it.
  _d=$(registration_delta_ok "$built" "$covered" "$excl" "$srcdir")
  _p=$(phantom_tests_ok "$_rows")
  case "$_d" in UNMEASURED*) echo "$_d"; return ;; esac
  case "$_p" in UNMEASURED*) echo "$_p"; return ;; esac
  # STRIP THE PHANTOM COMPONENT BY NAME, NOT BY POSITION. This was `${_d%%+*}`,
  # which drops EVERYTHING after the first `+` -- so any component added to
  # registration_delta_ok after the phantom half vanished here without a trace.
  # Measured on EXCL-MATCHED-NOTHING minutes after it was added: present in the
  # full verdict, gone after truncation. A positional strip silently eats future
  # findings; a named one drops exactly what it means to.
  case "$_d" in *UNREGISTERED-SUITE*|*STALE-BINARY-RETIRED-SUITE*|*EXCL-MATCHED-NOTHING*)
      out="${_d%%+PHANTOM-SUITE*}" ;;
  esac
  # registration_delta_ok's OWN phantom half is discarded here (%%+*): it is
  # DEAD BY DESIGN on this path, because `covered` basenames are derived only
  # from command paths that EXIST, so a covered name cannot be unbuilt.
  # phantom_tests_ok supersedes it with ctest's own resolution. Left unreachable
  # deliberately -- do not "fix" it by feeding unresolved names into covered.
  case "$_p" in PHANTOM-*) out="${out:+$out+}$_p" ;; esac
  if [ -n "$out" ]; then echo "$out"; else echo "$_d"; fi
}

# suites_fresh_ok -- C-3xx. THE GAP RUN #1 EXPOSED.
#
# shell_fresh_ok checks the RUNNER against its sources and prints "runner YES".
# The gate then executes FORTY OTHER BINARIES it never looks at. On run #1, EIGHT
# of those forty were stale -- one missing the very test case the registration
# line named -- and the gate printed `suites-ran YES(39/39)` while executing
# suites built from day-old code. THE COUNT IS HONEST ABOUT HOW MANY RAN AND
# SAYS NOTHING ABOUT WHAT THEY WERE BUILT FROM.
#
# Filesystem-dependent by nature: mtime IS the measurement. Takes its roots as
# arguments so both arms are demonstrable on a temp fixture.
suites_fresh_ok() {                 # $1=build dir  $2=test source dir
    local bdir="$1" sdir="$2" compared=0 stale="" b s
    [ -d "$bdir" ] || { echo "UNMEASURED-NO-BUILD-DIR($bdir)"; return 2; }
    [ -d "$sdir" ] || { echo "UNMEASURED-NO-SOURCE-DIR($sdir)"; return 2; }
    for b in "$bdir"/tst_*; do
        [ -f "$b" ] && [ -x "$b" ] || continue
        s="$sdir/$(basename "$b").cpp"
        [ -f "$s" ] || continue
        compared=$((compared+1))
        [ "$s" -nt "$b" ] && stale="$stale $(basename "$b")"
    done
    # DEAD-PROBE ARM. A zero denominator is not freshness. "0 stale" printed by a
    # loop that compared nothing is indistinguishable from success -- the failure
    # mode that produced six dead extractions in this repo before it was named.
    [ "$compared" -gt 0 ] || { echo "UNMEASURED-NO-PAIRS-COMPARED"; return 2; }
    if [ -n "$stale" ]; then
        echo "STALE-SUITES($compared compared:$stale)"; return 1
    fi
    echo "YES($compared compared, 0 stale)"; return 0
}

# orphan_binaries_ok -- C-3xx, rule 12 (O-WATCH 3), class named by O-UI 6.
#
# A DELIBERATE TWO-AUTHORITY COMPARISON. DO NOT "FIX" THIS INTO ONE AUTHORITY.
# O-UI removed exactly this shape from their C-354 draft as a mixed-authority
# defect and were wrong about why it was there. An ORPHAN is DEFINED as a binary
# with no target behind it: no target => no EXECUTABLE_LINKER rule => it can
# never appear in a rule-derived population. A single authority built from the
# build definition CANNOT see one. The difference between the authorities IS the
# quantity being measured.
#
# O-WATCH's rule, restated by them to cover this case: when two sides are meant
# to describe the SAME set, hold the method constant -- a difference you cannot
# attribute is worthless. When the difference between two authorities IS the
# quantity, two authorities are mandatory.
#
# ★ AND A DIFFERENCE-HUNT CARRIES AN OBLIGATION AN AGREEMENT-CHECK DOES NOT.
# In an agreement-check contamination surfaces as a red you go investigate. HERE
# CONTAMINATION SURFACES AS THE FINDING -- there is no "something is wrong"
# signal to fall back on, because the disagreement IS the output. So every
# non-phenomenal source of difference must be excluded by PRECONDITION before
# the arm means anything. The preconditions below are load-bearing, not hygiene.
#
# Measured, so the danger is concrete: the live build dir holds 40 `tst_*`
# DIRECTORIES, and `[ -x dir ]` is TRUE. A filesystem side without `-type f`
# reports EXACTLY 40 ORPHANS -- the same number as the true target count. That
# is the plausible wrong answer that gets believed, not the absurd one that gets
# caught. Callers MUST pass a `-type f`-filtered set; G-144 says the overlap arm
# catches DISJOINT universes and not CONTAMINATED ones, so it will not save you.
_orphan_delta_ok() {                # $1=filesystem set  $2=rule-derived set  $3=excl
                                    # PRIVATE. The public entry is orphan_binaries_ok,
                                    # which OWNS its derivation. Callers must not reach
                                    # this directly -- passing pre-computed sets is the
                                    # defect O-WATCH found: contamination is a SUPERSET
                                    # and a superset OVERLAPS PERFECTLY, so it clears
                                    # every precondition below and emits a confident,
                                    # specific, entirely false orphan.
    local fs="$1" rule="$2" excl="${3:-}" n_f n_r n_ov orph named="" o
    n_f=$(printf '%s\n' "$fs"   | grep -c . || true)
    n_r=$(printf '%s\n' "$rule" | grep -c . || true)
    # PRECONDITION 1 -- rule side empty. Under six of the seven generators the
    # allowlist permits, `ninja -t targets all` does not exist, so this side comes
    # back empty and EVERY filesystem entry reads as an orphan.
    [ "$n_r" -gt 0 ] || { echo "UNMEASURED-NO-RULE-DERIVED-TARGETS(every filesystem entry would read as an orphan)"; return 2; }
    # PRECONDITION 2 -- filesystem side empty: nothing to hunt, and 0 orphans from
    # an empty scan is indistinguishable from 0 orphans from a clean tree.
    [ "$n_f" -gt 0 ] || { echo "UNMEASURED-NO-FILESYSTEM-BINARIES"; return 2; }
    # PRECONDITION 3 -- G-126. Disjoint sets make every entry a false orphan.
    n_ov=$(comm -12 <(printf '%s\n' "$fs" | grep . | sort -u) \
                    <(printf '%s\n' "$rule" | grep . | sort -u) | grep -c . || true)
    [ "$n_ov" -gt 0 ] || { echo "UNMEASURED-ZERO-OVERLAP(fs=$n_f rule=$n_r share 0 names -- different universes; every entry would read as an orphan)"; return 2; }

    orph=$(comm -23 <(printf '%s\n' "$fs" | grep . | sort -u) \
                    <(printf '%s\n' "$rule" | grep . | sort -u))
    # `for o in $orph` SPLITS DIFFERENTLY BY SHELL and this is not theoretical:
    # zsh does NOT split an unquoted PARAMETER (one iteration, the whole list) while
    # bash does (one per line) -- and zsh DOES split an unquoted command
    # substitution, which is the opposite of the case that bit `_rj`. Under zsh the
    # `case " $excl "` test then never matched and A DOCUMENTED EXCLUSION WAS
    # ANNOUNCED AS AN ORPHAN. Measured: bash -> ORPHAN-BINARY(c), zsh ->
    # ORPHAN-BINARY(c\nz) with z excluded. Read the lines explicitly.
    #
    # And COLLECT ALL of them, matching registration_delta_ok. Returning on the
    # first orphan makes a responder remove one and re-run into the next.
    # Accumulate NEWLINE-separated and join EXPLICITLY. The first draft of this fix
    # built a space-separated list and joined it with `echo $found | tr ' ' ','` --
    # which relies on the CALLER'S SHELL word-splitting and produced ",c" under zsh
    # (leading comma from the leading space, since zsh does not split an unquoted
    # parameter). That is the identical defect this file already documents in
    # `_rj`, REINTRODUCED INSIDE THE FIX FOR A SPLITTING BUG. A guard whose OUTPUT
    # depends on who sourced it is not a guard.
    local found=""
    while IFS= read -r o; do
        [ -n "$o" ] || continue
        case " $excl " in *" $o "*) named="$named $o"; continue ;; esac
        found="$found$o
"
    done <<EOF
$orph
EOF
    if [ -n "$found" ]; then
        echo "ORPHAN-BINARY($(printf '%s' "$found" | grep . | tr '\n' ',' | sed 's/,$//') -- on disk, no build rule produces them; a retired target left its binary behind)"
        return 1
    fi
    local _xu; _xu=$(_excl_unmatched "$excl" "$fs")
    if [ -n "$_xu" ]; then
        echo "EXCL-MATCHED-NOTHING($_xu -- named as excluded but not in the filesystem set: stale exclusion, typo, or an argument in the wrong slot)"
        return 1
    fi
    if [ -n "$named" ]; then
        echo "YES(fs=$n_f rule=$n_r overlap=$n_ov, 0 orphans, excluded:$named)"
    else
        echo "YES(fs=$n_f rule=$n_r overlap=$n_ov, 0 orphans)"
    fi
    return 0
}

# PUBLIC ENTRY. Takes the BUILD DIR and derives BOTH sides itself, so contamination
# is impossible by construction rather than by the caller's discipline -- the same
# structural point we agreed for the pin panic: a guard and the thing it guards must
# not sit in different functions. `registration_complete_ok` already had this shape;
# this one trusted a caller and O-WATCH demonstrated the cost on the live tree.
#
# `-type f` IS THE LOAD-BEARING PART. The build dir holds 40 `tst_*` DIRECTORIES and
# `[ -x dir ]` is TRUE, so an `-x`-only side yields 80 names, clears all three
# preconditions (a superset overlaps perfectly), and reports 40 false orphans --
# the same count as the true target set.
orphan_binaries_ok() {              # $1=build dir  $2=excl
    local bdir="$1" excl="${2:-}" fs rule
    [ -d "$bdir" ] || { echo "UNMEASURED-NO-BUILD-DIR($bdir)"; return 2; }
    # Precondition 0 -- the rule side is a NINJA interface. Six of the seven
    # generators the allowlist permits have no `ninja -t targets`, and an empty
    # rule side makes every filesystem entry read as an orphan.
    [ -f "$bdir/build.ninja" ] || { echo "UNMEASURED-NOT-A-NINJA-BUILD-DIR($bdir -- rule-derived side is a ninja interface)"; return 2; }
    command -v ninja >/dev/null 2>&1 || { echo "UNMEASURED-NO-NINJA-ON-PATH"; return 2; }
    fs=$(find "$bdir" -maxdepth 1 -type f -perm -u+x -name 'tst_*' -exec basename {} \; 2>/dev/null | sort -u)
    rule=$(ninja -C "$bdir" -t targets all 2>/dev/null | grep 'EXECUTABLE_LINKER' \
             | sed 's/:.*//' | xargs -n1 basename 2>/dev/null | grep '^tst_' | sort -u)
    _orphan_delta_ok "$fs" "$rule" "$excl"
}

guards_selftest() {
  local rc=0 t got
  # DENOMINATOR (O-INT's hardening, 2026-08-26). Without a count of checks that
  # actually RAN, this function cannot tell "all passed" from "none ran" -- and
  # that is not hypothetical: O-INT appended six cases that SILENTLY DID NOT
  # RUN because `unset -f _g` sat before the end of the function. Six
  # command-not-found lines on stderr and rc STAYED 0. A green selftest
  # reporting success while a third of it never executed.
  _gn=0
  _g() { _gn=$((_gn + 1)); got="$2"; if [ "$got" = "$1" ]; then printf '  PASS %-26s -> %s\n' "$3" "$got"
         else printf '  FAIL %-26s -> got [%s] want [%s]\n' "$3" "$got" "$1"; rc=1; fi; }

  _g "UNMEASURED-NONNUMERIC(ransum='' declared='36')" "$(partition_ok "" 36 30)"        "empty-ransum"
  _g "UNMEASURED-DECLARED-BELOW-FLOOR(0<30)"          "$(partition_ok 0 0 30)"          "vacuous {}=={}"
  _g "NO-SUITES-SILENTLY-DROPPED(35/36)"              "$(partition_ok 35 36 30)"        "dropped-suite"
  _g "YES(36/36)"                                     "$(partition_ok 36 36 30)"        "grant"

  t=$(mktemp -d) || return 1
  mkdir -p "$t/a" "$t/b" "$t/g"
  i=1; while [ $i -le 24 ]; do printf 'frame%s' $i > "$t/a/$i.png"
    cp "$t/a/$i.png" "$t/b/$i.png"; cp "$t/a/$i.png" "$t/g/$i.png"; i=$((i+1)); done
  _g "UNMEASURED-MISSING-DIR($t/nope)"  "$(tour_ok "$t/a" "$t/b" "$t/nope" 20)" "missing-dir"
  _g "YES(A=24 B=24 ART=24 DOC=0)"      "$(tour_ok "$t/a" "$t/b" "$t/g" 20)"    "grant"
  # ★★★★★ THE ARM FOR THE CASE THE DOC EXCLUSION EXISTS FOR, ADDED 2026-09-12
  # (O-INT). C-790 taught tour_ok that documentation is not a golden and added
  # DOC=n to every verdict -- and NOTHING IN THIS SELFTEST EXERCISED EITHER.
  # The only arm that touched the new format was the `grant` line above, which
  # still WANTED the old string, so the file shipped with its own control red:
  # guards_selftest rc=1 on main AND on lane/ui, and gate.sh:484 turns that into
  # shell-behavior UNMEASURED (return 2). Two desks ran the five BEHAVIOURAL
  # controls on the real goldens directory and neither ran the file's OWN
  # selftest -- I landed it, so the miss is mine.
  #
  # ★★ This arm is the one that would have caught the original breakage: a
  # document sitting IN the goldens directory must not be counted (ART stays
  # 24, not 25) and must not be diffed (the verdict stays YES), and DOC=1 must
  # SAY that one was ignored. The doc is left in place for the two arms below,
  # so the exclusion is proven in the failure paths too -- their expected
  # strings are unchanged, which is the point.
  printf 'provenance, not a golden' > "$t/g/PROVENANCE.md"
  _g "YES(A=24 B=24 ART=24 DOC=1)"      "$(tour_ok "$t/a" "$t/b" "$t/g" 20)"    "a document in the goldens dir is IGNORED and SAID"
  printf 'DIFFERENT' > "$t/b/7.png"
  _g "NO-AB-DIFFER(A=24 B=24 -- 7.png)"          "$(tour_ok "$t/a" "$t/b" "$t/g" 20)"    "determinism arm NAMES its member"
  cp "$t/a/7.png" "$t/b/7.png"; printf 'DIFFERENT' > "$t/g/7.png"
  _g "NO-ARTIFACT-DIFFER(A=24 ART=24 -- 7.png)"  "$(tour_ok "$t/a" "$t/b" "$t/g" 20)"    "FIDELITY arm NAMES its member"
  rm -rf "$t"

  # ---- shell_fresh_ok: the RUNNER's freshness (bound A arm 2, stdin-shaped) ----
  t=$(mktemp -d) || return 1
  mkdir -p "$t/repo/src" "$t/repo/build"
  printf 'a' > "$t/repo/src/one.cpp"; printf 'b' > "$t/repo/src/two.cpp"
  touch -t 202601010000 "$t/repo/src/one.cpp" "$t/repo/src/two.cpp"
  printf 'bin' > "$t/repo/build/app"; touch -t 202606010000 "$t/repo/build/app"
  _g "YES(nsrc=2 excl=0)" \
     "$(printf '%s\n%s\n' "$t/repo/src/one.cpp" "$t/repo/src/two.cpp" | shell_fresh_ok "$t/repo/build/app" "$t/repo" "$t/repo/build")" \
     "runner grant"
  touch -t 202612010000 "$t/repo/src/two.cpp"
  # ---- LEG A: registration completeness. BOTH DIRECTIONS DEMONSTRATED.
  # A guard that always refuses is a broken probe with good manners; a guard
  # only ever watched to pass is not in service. Five arms: two population
  # guards, one grant, one RED on a silent drop, one grant with the exclusion
  # NAMED in the output.
  _g "UNMEASURED-NO-BUILT-BINARIES" "$(registration_delta_ok "" "a" "")" "reg zero-built"
  _g "UNMEASURED-NO-REGISTRATIONS"  "$(registration_delta_ok "a" "" "")" "reg zero-registered"
  _g "YES(built=2 registered=2)"    "$(registration_delta_ok "$(printf 'a\nb')" "$(printf 'a\nb')" "")" "reg complete"
  _g "UNREGISTERED-SUITE(c -- built and not registered; check whether a source exists before acting: source present means REGISTER IT, source absent means a RETIRED suite left its binary behind and the fix is to DELETE IT)"        "$(registration_delta_ok "$(printf 'a\nb\nc')" "$(printf 'a\nb')" "")" "reg SILENT-DROP RED"
  _g "YES(built=3 registered=2 excluded=c)" "$(registration_delta_ok "$(printf 'a\nb\nc')" "$(printf 'a\nb')" "c")" "reg exclusion NAMED"
  # O-UI's live red: the ambiguity, both directions, with and without $4.
  # DISTINCT VARIABLE. An earlier block in this file was broken exactly this way:
  # reusing `t` and rm -rf'ing it left five LATER arms pointing at a deleted
  # fixture. The denominator caught it then and caught it again now.
  treg=$(mktemp -d) || return 1
  mkdir -p "$treg/tests"; : > "$treg/tests/tst_kept.cpp"
  _g "UNREGISTERED-SUITE(tst_kept -- source present, REGISTER IT)" \
     "$(registration_delta_ok "$(printf 'a\ntst_kept')" "$(printf 'a')" "" "$treg/tests")" "reg SOURCE PRESENT -> register"
  _g "STALE-BINARY-RETIRED-SUITE(tst_retired -- NO SOURCE in $treg/tests; the suite was retired and its binary remains. DELETE THE BINARY. DO NOT REGISTER IT)" \
     "$(registration_delta_ok "$(printf 'a\ntst_retired')" "$(printf 'a')" "" "$treg/tests")" "reg SOURCE ABSENT -> delete"
  _g "UNREGISTERED-SUITE(tst_kept -- source present, REGISTER IT)+STALE-BINARY-RETIRED-SUITE(tst_retired -- NO SOURCE in $treg/tests; the suite was retired and its binary remains. DELETE THE BINARY. DO NOT REGISTER IT)" \
     "$(registration_delta_ok "$(printf 'a\ntst_kept\ntst_retired')" "$(printf 'a')" "" "$treg/tests")" "reg BOTH readings at once"
  rm -rf "$treg"

  _g "EXCL-MATCHED-NOTHING(ghost -- named as excluded but not in the built set: stale exclusion, typo, or an argument in the wrong slot)" \
     "$(registration_delta_ok "$(printf 'a\nb')" "$(printf 'a\nb')" "ghost")" "reg EXCL matched nothing"
  _g "YES(built=2 registered=1 excluded=b)" \
     "$(registration_delta_ok "$(printf 'a\nb')" "$(printf 'a')" "b")" "reg EXCL that DOES match"

  _g "PHANTOM-SUITE(z)"             "$(registration_delta_ok "$(printf 'a\nb')" "$(printf 'a\nb\nz')" "")" "reg PHANTOM RED"
  _g "UNREGISTERED-SUITE(c -- built and not registered; check whether a source exists before acting: source present means REGISTER IT, source absent means a RETIRED suite left its binary behind and the fix is to DELETE IT)+PHANTOM-SUITE(z)" "$(registration_delta_ok "$(printf 'a\nc')" "$(printf 'a\nz')" "")" "reg BOTH directions"
  _g "UNMEASURED-ZERO-OVERLAP(built=2 registered=2 share 0 names -- different universes, or total registration collapse; indistinguishable from here)" \
     "$(registration_delta_ok "$(printf 'a\nb')" "$(printf 'y\nz')" "")" "reg G-126 ZERO-OVERLAP"
  _g "UNREGISTERED-SUITE(b -- built and not registered; check whether a source exists before acting: source present means REGISTER IT, source absent means a RETIRED suite left its binary behind and the fix is to DELETE IT)+PHANTOM-SUITE(z)" \
     "$(registration_delta_ok "$(printf 'a\nb')" "$(printf 'a\nz')" "")" "reg partial-overlap still measures"
  _g "UNREGISTERED-SUITE(b,c -- built and not registered; check whether a source exists before acting: source present means REGISTER IT, source absent means a RETIRED suite left its binary behind and the fix is to DELETE IT)"      "$(registration_delta_ok "$(printf 'a\nb\nc')" "$(printf 'a')" "")" "reg join no leading comma"
  _rgt=$(mktemp -d); mkdir -p "$_rgt/bd"; printf 'CMAKE_GENERATOR:INTERNAL=Xcode\n' > "$_rgt/bd/CMakeCache.txt"
  _g "UNMEASURED-UNRECOGNISED-GENERATOR(Xcode)" "$(echo a | registration_complete_ok "$_rgt/bd" "")" "reg multiconfig refused"
  printf 'CMAKE_GENERATOR:INTERNAL=Ninja Multi-Config\n' > "$_rgt/bd/CMakeCache.txt"
  _g "UNMEASURED-UNRECOGNISED-GENERATOR(Ninja Multi-Config)" "$(echo a | registration_complete_ok "$_rgt/bd" "")" "reg ninja-multiconfig refused"
  printf 'CMAKE_GENERATOR:INTERNAL=Ninja\n' > "$_rgt/bd/CMakeCache.txt"
  # Plain Ninja must PASS the generator gate. It then refuses at the NEXT gate
  # (no real ctest project in a fixture dir), and that is the correct verdict --
  # what this arm proves is that a recognised generator is NOT refused as a
  # generator. A check that refuses everything is not a check.
  _g "UNMEASURED-CTEST-JSON-UNREADABLE($_rgt/bd)" "$(echo a | registration_complete_ok "$_rgt/bd" "")" "reg plain-Ninja accepted"
  rm -f "$_rgt/bd/CMakeCache.txt"
  _g "UNMEASURED-UNRECOGNISED-GENERATOR(unreadable)" "$(echo a | registration_complete_ok "$_rgt/bd" "")" "reg no-cache refused"
  rm -rf "$_rgt"; unset _rgt
  # ---- phantom_tests_ok: ctest's own resolution, not a second method ----
  _rgt=$(mktemp -d); printf 'bin' > "$_rgt/real"
  _g "YES(registered=2)" "$(phantom_tests_ok "$(printf 'a\t%s\nb\t%s' "$_rgt/real" "$_rgt/real")")" "phantom none"
  _g "PHANTOM-NOT-BUILT(b)" "$(phantom_tests_ok "$(printf 'a\t%s\nb\t' "$_rgt/real")")" "phantom NOT-BUILT"
  _g "PHANTOM-MISSING-BINARY(b)" "$(phantom_tests_ok "$(printf 'a\t%s\nb\t%s' "$_rgt/real" "$_rgt/absent")")" "phantom MISSING-BINARY"
  _g "PHANTOM-NOT-BUILT(b)+PHANTOM-MISSING-BINARY(c)" "$(phantom_tests_ok "$(printf 'a\t%s\nb\t\nc\t%s' "$_rgt/real" "$_rgt/absent")")" "phantom BOTH causes"
  _g "UNMEASURED-NO-REGISTRATIONS" "$(phantom_tests_ok "")" "phantom zero-registered"
  rm -rf "$_rgt"; unset _rgt
  # ---- THE REGRESSION THAT FORCED THE JSON REWRITE: add_test decouples the
  # test NAME from the binary it invokes. f3c94410 registers
  #   add_test(NAME tst_transform_repaint_footprint COMMAND tst_g5_frameprobe ...)
  # Under the old name-equality model that produced TWO false findings on a
  # change that fixes the hole. Comparing BINARIES, it is covered. ----
  _g "YES(built=1 registered=1)" "$(registration_delta_ok "tst_g5_frameprobe" "tst_g5_frameprobe" "")" "decoupled name covered"

  _g "STALE-RUNNER(bin=app src=two.cpp)" \
     "$(printf '%s\n%s\n' "$t/repo/src/one.cpp" "$t/repo/src/two.cpp" | shell_fresh_ok "$t/repo/build/app" "$t/repo" "$t/repo/build")" \
     "stale runner"
  touch -t 202601010000 "$t/repo/src/two.cpp"
  printf 'g' > "$t/repo/build/gen.h"; touch -t 202612010000 "$t/repo/build/gen.h"
  _g "YES(nsrc=2 excl=1)" \
     "$(printf '%s\n%s\n%s\n' "$t/repo/src/one.cpp" "$t/repo/src/two.cpp" "$t/repo/build/gen.h" | shell_fresh_ok "$t/repo/build/app" "$t/repo" "$t/repo/build")" \
     "build-dir excluded NON-VACUOUSLY"
  _g "UNMEASURED-NO-CLOSURE(nsrc=0)" \
     "$(printf '' | shell_fresh_ok "$t/repo/build/app" "$t/repo" "$t/repo/build")" \
     "unread graph"
  _g "UNMEASURED-NO-BINARY($t/repo/build/absent)" \
     "$(printf '%s\n' "$t/repo/src/one.cpp" | shell_fresh_ok "$t/repo/build/absent" "$t/repo" "$t/repo/build")" \
     "missing runner"
  # O-INT's exact mistake: 2 args to a 3-arg function. Must REFUSE, not answer.
  _g "UNMEASURED-NO-BUILD-DIR" \
     "$(printf '%s\n' "$t/repo/src/one.cpp" | shell_fresh_ok "$t/repo/build/app" "$t/repo")" \
     "two-arg call refused"
  _g "UNMEASURED-BUILD-OUTSIDE-ROOT(build=/elsewhere not under root=$t/repo)" \
     "$(printf '%s\n' "$t/repo/src/one.cpp" | shell_fresh_ok "$t/repo/build/app" "$t/repo" "/elsewhere")" \
     "build dir outside root"
  rm -rf "$t"

  d=$(mktemp -d) || return 1
  mkdir -p "$d/empty"
  _g "UNMEASURED-NO-BUILD-DIR($d/nope)"  "$(declared_set_ok "$d/nope")"  "declared_set missing-dir"
  _g "UNMEASURED-EMPTY-SET($d/empty)"    "$(declared_set_ok "$d/empty")" "declared_set empty-set"
  rm -rf "$d"

  ctest_ran_selftest || rc=1
  # DENOMINATOR AGAINST THE FILE, not against a hand-maintained number.
  # A FIXED floor does not close the defect it exists for: append three cases
  # after `unset -f _g`, they silently do not run, the counter stays at the
  # old value and STILL MEETS the old floor. So the expected count is derived
  # from the SOURCE -- every line whose first token is `_g` -- and compared to
  # what actually executed. Appending anywhere raises the expectation
  # automatically; appending in the dead zone raises it WITHOUT raising the
  # count, which is exactly the failure.
  # (O-INT's first floor was ASSUMED at 21 and fired falsely on its first run,
  # because ctest_ran_selftest uses its own helper and never touches this
  # counter. Deriving it removes the assumption rather than correcting it.)

  # suites_fresh_ok -- the run-#1 freshness gap. FIXTURE NEEDS A GUARANTEED SECOND
  # OF SEPARATION: the first draft of this demonstration touched the source in the
  # SAME SECOND as the binary and the positive arm printed YES(3 compared, 0 stale)
  # -- the function was correct and the fixture could not express the ordering it
  # was testing. G-132: an mtime check demonstrated on a same-second fixture is
  # demonstrated against a coin flip.
  t=$(mktemp -d) || return 1
  mkdir -p "$t/b" "$t/s" "$t/empty"
  for _n in a b c; do printf 'x' > "$t/s/tst_$_n.cpp"; done
  sleep 1
  for _n in a b c; do : > "$t/b/tst_$_n"; chmod +x "$t/b/tst_$_n"; done
  _g "YES(3 compared, 0 stale)"        "$(suites_fresh_ok "$t/b" "$t/s")"     "fresh-suites grant"
  _g "UNMEASURED-NO-BUILD-DIR($t/nope)" "$(suites_fresh_ok "$t/nope" "$t/s")" "fresh-suites missing-dir"
  _g "UNMEASURED-NO-PAIRS-COMPARED"    "$(suites_fresh_ok "$t/empty" "$t/s")" "fresh-suites DEAD-PROBE"
  sleep 1; touch "$t/s/tst_b.cpp"
  _g "STALE-SUITES(3 compared: tst_b)" "$(suites_fresh_ok "$t/b" "$t/s")"     "fresh-suites STALE RED"
  rm -rf "$t"

  # orphan_binaries_ok -- the two-authority difference-hunt. Every precondition
  # gets an arm, because in a difference-hunt a failed precondition does not
  # produce a red, it produces A FINDING THAT LOOKS REAL.
  _g "YES(fs=3 rule=3 overlap=3, 0 orphans)" \
     "$(_orphan_delta_ok "$(printf 'a\nb\nc')" "$(printf 'a\nb\nc')" "")" "orphan clean"
  _g "ORPHAN-BINARY(z -- on disk, no build rule produces them; a retired target left its binary behind)" \
     "$(_orphan_delta_ok "$(printf 'a\nb\nz')" "$(printf 'a\nb')" "")" "orphan DETECTED"
  _g "YES(fs=3 rule=2 overlap=2, 0 orphans, excluded: z)" \
     "$(_orphan_delta_ok "$(printf 'a\nb\nz')" "$(printf 'a\nb')" "z")" "orphan exclusion NAMED"
  _g "UNMEASURED-NO-RULE-DERIVED-TARGETS(every filesystem entry would read as an orphan)" \
     "$(_orphan_delta_ok "$(printf 'a\nb')" "" "")" "orphan PRECOND rule-empty"
  _g "UNMEASURED-NO-FILESYSTEM-BINARIES" \
     "$(_orphan_delta_ok "" "$(printf 'a\nb')" "")" "orphan PRECOND fs-empty"
  _g "EXCL-MATCHED-NOTHING(nosuch -- named as excluded but not in the filesystem set: stale exclusion, typo, or an argument in the wrong slot)" \
     "$(_orphan_delta_ok "$(printf 'a\nb')" "$(printf 'a\nb')" "nosuch")" "orphan EXCL matched nothing"
  _g "UNMEASURED-ZERO-OVERLAP(fs=2 rule=2 share 0 names -- different universes; every entry would read as an orphan)" \
     "$(_orphan_delta_ok "$(printf 'a\nb')" "$(printf 'y\nz')" "")" "orphan PRECOND zero-overlap"
  _g "ORPHAN-BINARY(c,z -- on disk, no build rule produces them; a retired target left its binary behind)" \
     "$(_orphan_delta_ok "$(printf 'a\nc\nz')" "$(printf 'a')" "")" "orphan COLLECTS ALL"
  _g "ORPHAN-BINARY(c -- on disk, no build rule produces them; a retired target left its binary behind)" \
     "$(_orphan_delta_ok "$(printf 'a\nc\nz')" "$(printf 'a')" "z")" "orphan excl not announced"
  t=$(mktemp -d) || return 1
  _g "UNMEASURED-NO-BUILD-DIR($t/nope)"  "$(orphan_binaries_ok "$t/nope" "")" "orphan-pub missing-dir"
  _g "UNMEASURED-NOT-A-NINJA-BUILD-DIR($t -- rule-derived side is a ninja interface)" \
     "$(orphan_binaries_ok "$t" "")" "orphan-pub not-ninja"
  rm -rf "$t"

  local _gsites
  _gsites=$(grep -c '^[[:space:]]*_g ' "${BASH_SOURCE[0]}" 2>/dev/null || echo 0)
  if [ "$_gsites" -lt 2 ]; then
    printf '  FAIL %-26s -> could not count _g sites in %s (got %s)\n' "selftest-denominator" "${BASH_SOURCE[0]##*/}" "$_gsites"
    rc=1
  elif [ "$_gn" -ne "$_gsites" ]; then
    printf '  FAIL %-26s -> %s of %s checks ran; cases after `unset -f _g` do not execute\n' "selftest-denominator" "$_gn" "$_gsites"
    rc=1
  else
    printf '  PASS %-26s -> %s of %s checks ran\n' "selftest-denominator" "$_gn" "$_gsites"
  fi
  unset -f _g
  return $rc
}

# ── EXECUTED-NOT-SOURCED GUARD (C-3xx, O-WATCH 3).
#
# This file is a pure function library and had NO argument dispatch, so
#
#     bash scripts/battery_guards.sh guards_selftest
#
# defined every function and exited 0 HAVING RUN NOTHING. rc=0, zero output,
# zero arms. The exit code was genuinely this command's own -- it was not a pipe's
# and not a stale $? -- and it was hollow. NOTHING EXITS 0. Reading the right exit
# code does not establish that the thing you meant to run ran; the discriminator
# is EVIDENCE OF WORK -- output lines, an arm count, a tally.
#
# Dispatch is allowlisted rather than open: a library that will run any function
# named on its command line is a worse trap than the one being closed.
if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    case "${1:-}" in
        guards_selftest)     guards_selftest; exit $? ;;
        ctest_ran_selftest)  ctest_ran_selftest; exit $? ;;
        "")  echo "battery_guards.sh is a FUNCTION LIBRARY. Source it:" >&2
             echo "    . scripts/battery_guards.sh && guards_selftest" >&2
             echo "Or run an allowlisted entry point: guards_selftest | ctest_ran_selftest" >&2
             exit 2 ;;
        *)   echo "battery_guards.sh: '$1' is not an allowlisted entry point." >&2
             echo "Allowed: guards_selftest | ctest_ran_selftest. Sourcing gives you all functions." >&2
             exit 2 ;;
    esac
fi
