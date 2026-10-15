# shellcheck shell=sh
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
# oint-probe.sh — OWNER: O-INT (integration desk)
# oint-probe.sh — primitives for the REPORTING path, not the certifying path.
#
# WHY THIS EXISTS (G-387, 2026-08-27): every audit of the committed instruments —
# gate.sh, battery_guards.sh, oint-fence.sh — came back clean today. POSIX common
# subset only, epoch comparison rather than clock-face, numstat for enforcement
# rather than grep, a scope check that refuses its own author's edit. EVERY DEFECT
# FOUND ALL DAY WAS IN AN AD-HOC PROBE WRITTEN IN THE MOMENT, and the ad-hoc probe
# is what produces the figures that go to the operator. The toolchain is audited;
# the reporting path has no selftest, no control by default, and no reviewer.
#
# These are not new rules. Each one is a defect that was MEASURED today, turned into
# the cheapest available call so the correct form stops costing more than the wrong
# one. Source it; do not execute it.
#
# CONSTRAINT (G-445): every predicate here stays inside the POSIX common subset —
# no \b, \d, \s, -P, no lookaround. The two greps on this host (ugrep 7.8.4
# interactively, BSD grep 2.6.0 in a script) and the two gits (2.15.0, Apple 2.39.5)
# disagree outside that subset, and 2.39.5's ERE makes a \b pattern match NOTHING,
# which reads as a clean negative rather than an error.

# p_count FILE PATTERN [-F]
#   Count matching lines. NEVER use `grep -c ... || echo 0`: grep -c PRINTS 0 AND
#   EXITS 1 on no match, so the `||` appends a SECOND zero and every downstream
#   comparison sees "0\n0". That defect was catalogued at 13:0x and walked into four
#   more times the same day, twice inside probes auditing for defects.
p_count() {
    _pc_f=$1; _pc_p=$2; _pc_mode=${3:--E}
    [ -f "$_pc_f" ] || { echo "UNMEASURED-NO-FILE($_pc_f)"; return 2; }
    _pc_n=$(/usr/bin/grep -c "$_pc_mode" -- "$_pc_p" "$_pc_f" 2>/dev/null)
    case "$_pc_n" in ''|*[!0-9]*) _pc_n=0 ;; esac
    printf '%s\n' "$_pc_n"
}

# p_rc CMD...
#   Run CMD and print ITS exit status. Never `cmd | tail -n` then `$?` — that reads
#   the LAST pipeline element's status, and zsh has no PIPESTATUS to recover it.
#   Measured today: a fence printing "FENCE FAILED - DO NOT COMMIT" reported rc=0
#   because the status came from `tail`. Unpiped re-run: rc=1.
p_rc() {
    "$@" >/dev/null 2>&1
    printf '%s\n' "$?"
}

# p_out CMD...
#   Run CMD keeping stderr VISIBLE in the captured output, and prefix the rc.
#   Never `2>/dev/null` around something whose failure you have not already
#   characterised: measured today, `git grep -P ... 2>/dev/null` returned 0 and the
#   zero was read as a count. Stderr held "fatal: cannot use Perl-compatible regexes
#   when not compiled with USE_LIBPCRE". A SWALLOWED FATAL READ AS A MEASUREMENT.
p_out() {
    _po=$("$@" 2>&1); _po_rc=$?
    printf 'rc=%s\n%s\n' "$_po_rc" "$_po"
}

# p_epoch PATH [c|m]
#   Timestamp as raw epoch seconds. NEVER compare formatted stamps: a timestamp
#   compared without its date is a CLOCK-FACE comparison and inverts on any pair
#   whose LATER member carries the EARLIER clock face, at ANY separation. Measured
#   today on a pair sixteen days apart; it cost two files out of a 405 partition and
#   the only symptom was that the partition did not sum.
p_epoch() {
    _pe_k=${2:-m}
    stat -f "%$_pe_k" "$1" 2>/dev/null || printf '0\n'
}

# p_verdict LABEL ACTUAL EXPECTED
#   Print a verdict COMPUTED from the two values. A conclusion composed inside a
#   probe is not a reading of the result — it is a prediction that prints itself
#   regardless (G-447, O-WATCH). Measured today: a computed line printed
#   "DOES NOT match" over a dead probe where a composed one would have printed
#   "REPRODUCES" over the same corpse.
p_verdict() {
    if [ "$2" = "$3" ]; then printf '  %-46s %s == %s  MATCH\n' "$1" "$2" "$3"
    else printf '  %-46s %s vs %s  ★ DIFFERS\n' "$1" "$2" "$3"; fi
}

# p_partition LABEL A B C TOTAL
#   Assert a partition sums to its population. A partition that fails to sum
#   IDENTIFIES ITS OWN SHORT SIDE without relitigating the predicate — that is how
#   the clock-face defect above was found, by a peer, from the sum alone.
p_partition() {
    _pp_s=$(( $2 + $3 + $4 ))
    if [ "$_pp_s" -eq "$5" ]; then printf '  %-34s %s+%s+%s = %s == %s  CLOSES\n' "$1" "$2" "$3" "$4" "$_pp_s" "$5"
    else printf '  %-34s %s+%s+%s = %s vs %s  ★ SHORT BY %s\n' "$1" "$2" "$3" "$4" "$_pp_s" "$5" "$(( $5 - _pp_s ))"; fi
}

# p_msg_unsafe TEXTFILE
#   Print the count of backtick-delimited spans in TEXTFILE whose first word resolves
#   to an executable on this host. NONZERO MEANS: do not put this text in a
#   double-quoted shell string.
#
#   WHY (G-451, O-WATCH, 2026-08-27): in a fleet where desks quote each other
#   verbatim, ONE DESK'S PROSE IS CODE IN THE OTHER DESK'S SHELL. Backticks inside a
#   double-quoted string are command substitution. Measured in the peer ledger the
#   day this was written: 5462 backticked spans, 66 distinct first words resolving to
#   executables, and 27 complete mutating commands WITH ARGUMENTS -- including
#   an absolute recursive chmod against the frozen spec repository, and a
#   git checkout against the record pin. The author cannot see the execution site;
#   the executor cannot see what they are quoting until it has run.
#
#   THIS FUNCTION DOES NOT MAKE QUOTING SAFE. The rule is absolute and prior to it:
#   A COMMIT MESSAGE OR SHELL STRING CONTAINING PEER TEXT USES -F - WITH A
#   SINGLE-QUOTED HEREDOC, NEVER -m "...". Forty-plus fences used that form; the one
#   that did not is how this was found. This scanner exists to make the payload
#   VISIBLE, not to license the unsafe form.
p_msg_unsafe() {
    [ -f "$1" ] || { echo "UNMEASURED-NO-FILE($1)"; return 2; }
    awk '
      { line = $0
        while (match(line, /`[^`]+`/)) {
            span = substr(line, RSTART+1, RLENGTH-2)
            line = substr(line, RSTART+RLENGTH)
            n = split(span, w, /[ \t]+/)
            if (n >= 1 && w[1] != "") print w[1]
        } }
    ' "$1" 2>/dev/null | sort -u | while IFS= read -r _w; do
        command -v -- "$_w" >/dev/null 2>&1 && printf '%s\n' "$_w"
    done | wc -l | tr -d ' '
}

# p_rowzero [DIR]
#   Row-zero tag state, or an UNMEASURED verdict naming why. Prints one line.
#
#   WHY (G-454, O-WATCH, 2026-08-27): they ran a tag check against the wrong
#   repository, git exited 129 with an empty stdout, `wc -l` turned that into 0, and
#   the line read as a clean pass. MEASURED ON MY OWN FORMULATION THE SAME TICK: run
#   from a directory with NO git repository at all, my standing row-zero lines print
#     tags at HEAD [] · total 0 · v5.6 0 · porcelain 0
#   which is indistinguishable from "nothing tagged, tree clean". THE CHECK THE
#   STANDING INSTRUCTION CALLS THE ONLY STOP PASSES WHERE IT CANNOT MEASURE.
#
#   INAPPLICABLE AND PASSING ARE DIFFERENT STATES. Every arm's exit status is read
#   before its output is counted, because the discriminator is the status and a pipe
#   discards it.
p_rowzero() {
    _rz_d=${1:-.}
    ( cd "$_rz_d" 2>/dev/null || { echo "UNMEASURED-NO-DIR($_rz_d)"; exit 2; }
      git rev-parse --git-dir >/dev/null 2>&1 || { echo "UNMEASURED-NOT-A-REPO($_rz_d)"; exit 2; }
      _t=$(git tag)             || { echo "UNMEASURED-TAG-LIST-FAILED(rc=$?)"; exit 2; }
      _h=$(git tag --points-at HEAD) || { echo "UNMEASURED-POINTS-AT-FAILED(rc=$?)"; exit 2; }
      # 2026-08-28: `-uno`. Bare --porcelain COUNTS UNTRACKED FILES, so this field --
      # renamed on 08-27 SPECIFICALLY TO STATE THE POPULATION IT COUNTS -- was still
      # counting a different one: an untracked scratch file read out as
      # tracked_modified=1. THE LABEL MOVED AND THE EXPRESSION DID NOT -- G-458's own
      # disease surviving inside the fix named after it. It went unseen because the
      # selftest had arms for TRACKED-MODIFIED (fires) and IGNORED (must not move) and
      # NO ARM FOR UNTRACKED: two of three categories tested, the untested one broken.
      # tagset= : THE WATCH. 2026-08-28, from O-WATCH #243 and verified here.
      # at_head= ABOVE IS BLIND WHERE EVERY TAG IN THIS REPOSITORY ACTUALLY LANDS:
      # `git tag --points-at HEAD` CANNOT SEE A TAG ON AN ANCESTOR, and 6 of appname's
      # 7 tags ARE ancestors with NONE at the tip. A CEREMONY TAG ON THE CERTIFIED
      # COMMIT -- an ancestor, not the tip -- WOULD RETURN 0 FROM THE FIELD BUILT TO
      # CATCH EXACTLY THAT EVENT. A DETECTOR BUILT AROUND THE EXCEPTION AND CALLED THE
      # WATCH. at_head= STAYS AS THE LOUD ARM; IT IS NOT THE WATCH.
      #
      # AND tags= IS A COUNT, NOT A SET: `git tag | wc -l` reads 7 across a
      # delete-and-create at a different commit exactly as across nothing happening.
      # This field is a CONTENT PIN over name+objectname for every ref, so creation,
      # deletion, RETARGET and rename all move it.
      #
      # LC_ALL=C IS LOAD-BEARING AND ITS AGREEMENT TODAY IS AN ACCIDENT OF THE DATA.
      # The three locales agree on the CURRENT tag names, so measuring them proves
      # nothing; on a constructed input (a-b / ab / Ab) C and en_US.UTF-8 ORDER
      # DIFFERENTLY ON THIS MACHINE. Unpinned, two desks could compute DIFFERENT PINS
      # FROM AN IDENTICAL TAG SET and one would report a false emergency -- the pin's
      # own failure mode is the alarm it exists to raise.
      #
      # THE EMPTY SET IS ITS OWN CASE: $(...) strips trailing newlines, so a naive
      # `printf '%s\n' "$_ts"` feeds ONE NEWLINE where the true pipeline feeds NOTHING,
      # and a tagless repo would pin to the hash of "\n" rather than of "". Guarded.
      _ts=$(git for-each-ref --format='%(refname:short) %(objectname)' refs/tags) \
        || { echo "UNMEASURED-FOR-EACH-REF-FAILED(rc=$?)"; exit 2; }
      if [ -z "$_ts" ]; then
          _pin=$(printf '' | /usr/bin/shasum -a 256 | /usr/bin/cut -c1-12)
      else
          _pin=$(printf '%s\n' "$_ts" | LC_ALL=C /usr/bin/sort | /usr/bin/shasum -a 256 | /usr/bin/cut -c1-12)
      fi
      _p=$(git status --porcelain -uno) || { echo "UNMEASURED-STATUS-FAILED(rc=$?)"; exit 2; }
      # nearmiss= : G-457 (O-WATCH, 2026-08-27). A CONTROL EMBEDDED IN PRODUCTION
      # DATA COVERS EXACTLY THE POPULATION THAT CARRIES IT, AND ITS COVERAGE IS
      # INVISIBLE FROM THE READING IT PROTECTS. bogos carries v5.5-freeze, so its
      # v56=0 is backed by a live neighbour that must be and is excluded. appname
      # carries NO v5.5 tag, so its v56=0 is backed only by the fixture. The two
      # zeros were TYPOGRAPHICALLY IDENTICAL AND EVIDENTIALLY DIFFERENT; this field
      # is what makes the difference legible at the point of reading.
      # ignored= : G-397 applied to my own standing line. "ignored 2" in the FROZEN
      # repository reads to a cold reader as two unexplained files; the fact that
      # neither can be inside the 309 was in my head, not on the line. NAMES, not a
      # count, because the count is what made it opaque.
      #
      # I FIRST TRIED TO ADD ign_tracked= AND TESTED WHETHER IT COULD EVER FIRE. It
      # cannot: git lists a path as '!!' only when it is UNTRACKED-and-ignored -- a
      # tracked file matching .gitignore is not listed at all (fixture-verified). So
      # ign_tracked would be 0 BY CONSTRUCTION, the vacuous control I have spent the
      # day objecting to. The structural fact is stated here ONCE instead of faked as
      # a measurement every tick: '!!' IMPLIES UNTRACKED IMPLIES OUTSIDE THE 309,
      # BECAUSE THE 309 ARE A SUBSET OF THE TRACKED SET.
      #
      # 2026-08-27, from O-WATCH #193: `--porcelain --ignored` runs at the DEFAULT
      # -unormal, which COLLAPSES A WHOLLY-IGNORED DIRECTORY INTO ONE ENTRY WITH A
      # TRAILING SLASH. In bogos this arm prints `tools/__pycache__/` where -uall
      # shows TWELVE files. THIS ARM IS NOT WRONG -- it NAMES paths and a directory
      # is a path -- but THE TRAILING SLASH IS THE ONLY SIGNAL THAT ONE ENTRY
      # STANDS FOR N FILES, and a slash is quiet.
      #
      # I IMPLEMENTED THE EXPANSION -- (N) appended to collapsed entries only -- and
      # BACKED IT OUT ON COST. 38/38 selftests passed and the output read
      # `tools/__pycache__/(12)`, the number that mattered. But -uall enumerates
      # EVERY ignored file and appname's target/ holds 432,771: p_rowzero went 1s ->
      # 9s, twice per tick, for a count informative in bogos and noise for a build
      # directory. REJECTED ON THE MEASUREMENT.
      #
      # SO THE SLASH STAYS AND THIS COMMENT CARRIES WHAT IT MEANS: a trailing slash
      # here is A DIRECTORY OF UNKNOWN SIZE, NOT ONE FILE. When the count is wanted:
      #     git status --porcelain --ignored -uall | grep '^!! <dir>/' | wc -l
      _i=$(git status --porcelain --ignored 2>/dev/null | /usr/bin/grep '^!! ' | sed 's/^!! //' | tr '\n' ',' | sed 's/,$//')
      # tracked_modified= : was 'porcelain=' until 2026-08-27 16:50. G-458 (O-WATCH):
      # A CHECK WHOSE ARM CAN FIRE IS NOT THEREBY A CHECK OF WHAT ITS LABEL CLAIMS,
      # AND A MISLABELLED LIVE CHECK IS WORSE THAN A TAUTOLOGY. I established as
      # G-431 that porcelain 0 means NO TRACKED FILE IS MODIFIED and NOT that nothing
      # was written -- the newest write in bogos was an ignored .pyc that porcelain
      # cannot see -- and then kept 'porcelain=' as the label, which is the git
      # SUBCOMMAND NAME, not the property. A reader takes it as "clean". THE FIELD I
      # NAMED THE DEFECT AFTER STILL CARRIED THE DEFECTIVE NAME. The new name states
      # the population it counts, and ignored=[] beside it names what it cannot see.
      printf 'tags=%s at_head=%s tagset=%s v56=%s nearmiss=%s tracked_modified=%s ignored=[%s] ALL-ARMS-rc0\n' \
        "$(printf '%s' "$_t" | /usr/bin/grep -c .)" \
        "$(printf '%s' "$_h" | /usr/bin/grep -c .)" \
        "$_pin" \
        "$(printf '%s' "$_t" | /usr/bin/grep -c '^v5\.6')" \
        "$(printf '%s' "$_t" | /usr/bin/grep -c '^v5\.5')" \
        "$(printf '%s' "$_p" | /usr/bin/grep -c .)" "$_i" )
}

# p_ground — ONE LINE OF GROUND, SO AN ENTRY WITHOUT IT TAKES EFFORT.
#
# WHY (2026-08-28, from O-WATCH #251 run on my own page): of my last TEN ledger
# entries, exactly ONE carried a full ground block; SEVEN did not mention bogos at
# all; SIX carried no tag pin. THE MEASUREMENT NEVER LAPSED -- I re-derived every
# tick and the pins were exact every time -- WHAT THINNED WAS THE RECORD. A seat
# can keep doing its job perfectly while its log stops being evidence that it did,
# and that is worse than a missed measurement: a missed measurement is discoverable
# later, an unrecorded one is not.
#
# AND IT ARRIVED THROUGH RESPONSIVENESS, NOT NEGLECT. Entries 268-275 were all
# inter-tick, written because a peer message arrived. Every one was worth writing.
# The aggregate drifted anyway -- which is why the remedy cannot be "remember to
# include it". G-652 applied correctly: a check moved into the instrument needs no
# discipline at all.
#
# Repository facts only -- no selftests, no side effects, cheap enough to run in
# every entry. guards= and probe_selftest stay separate BECAUSE they execute.
# Paths are PARAMETERS with the real desks as defaults, SO THE UNMEASURED BRANCH
# CAN BE FIRED BY A FIXTURE. Hardcoded paths would have made that branch
# unreachable from any test -- the vacuous control this file exists to refuse.
# CANONICAL TAG PINS — §5's WATCH, MOVED OUT OF PROSE AND INTO THE INSTRUMENT.
#
# ADDED 2026-09-04 (entry 568). §4 already listed `tagset=` among "the things
# that worked all stopped being rules — a check moved into the instrument needs
# no discipline at all". THAT WAS HALF TRUE: the MEASUREMENT lived in the probe
# while the CANONICAL VALUE it must be compared against was PROSE in an
# appendix, so the comparison was still a human reading two numbers in two
# places. The appname pin moved 7->8 tags and I caught it only because the
# digits snagged my eye in a line I read every tick — attention, which is the
# thing §4 says not to rely on.
#
# UPDATING ONE OF THESE IS A DELIBERATE ACT WITH A DIFF, which is the point:
# prose goes stale silently; a constant goes stale in a commit someone has to
# write. `pins=DRIFT(...)` is the emergency §5 names, and it now prints itself
# every tick without being remembered.
_CANON_PIN_APPNAME=79c245f650ca   # 8 tags; was f0646e2b3426/7 until q-tutorial/pre-rebase-t35 (entry 568)
_CANON_PIN_BOGOS=e0e9c844ab80     # 2 tags; unchanged since the freeze

# ---------------------------------------------------------------------------
# WHERE THE TREES ARE (operator WORD 24, RULING A, 2026-09-26). This file used
# to carry the build machine's own absolute paths as defaults, and the public
# source tarball ships it. Rule: an environment variable first, then the
# positions a checkout and a worktree put THIS tree in, never an absolute path.
#
#   _p_tree  — this repository, derived from the script's own location
#              (<tree>/scripts/instruments/oint-probe.sh), so a checkout and a
#              worktree both resolve without being named.
#   _p_spec  — the frozen specification repository. It is NOT derivable from
#              here and is NOT named here: set APPNAME_SPEC_ROOT to point at it.
#              Unset is not an error and not a pass — the arms that need it
#              report UNMEASURED, which is this desk's own vocabulary for a
#              control that could not run.
_p_self_dir() { CDPATH= cd -- "$(dirname -- "$0")" && pwd -P; }
_p_tree() {
    if [ -n "${APPNAME_TREE:-}" ]; then printf '%s' "$APPNAME_TREE"; return 0; fi
    _pt_d=$(_p_self_dir) || return 1
    # <tree>/scripts/instruments -> <tree>
    printf '%s' "$(dirname -- "$(dirname -- "$_pt_d")")"
}
_p_spec() { printf '%s' "${APPNAME_SPEC_ROOT:-}"; }

p_ground() {
    _g_ap=${1:-$(_p_tree)}
    _g_bp=${2:-$(_p_spec)}
    _g_a=$(p_rowzero "$_g_ap")
    _g_b=$(p_rowzero "$_g_bp")
    _g_f=$( cd "$_g_bp" 2>/dev/null \
            && git rev-list -n1 v5.6-freeze 2>/dev/null | cut -c1-8 )
    _g_k=$( cd "$_g_bp" 2>/dev/null \
            && git cat-file -t v5.6-freeze 2>/dev/null )
    [ -n "$_g_f" ] || _g_f=UNMEASURED
    [ -n "$_g_k" ] || _g_k=UNMEASURED
    # fence= : A DERIVED VERDICT, NOT A PRINTED LABEL. 2026-08-28. My tick recipe ran
    # `git status --porcelain -uno` and then ECHOED "(empty above = no review pending)"
    # UNCONDITIONALLY -- so when O-WATCH left an undeclared review in the shared
    # worktree, the status printed one path and the reassuring label printed anyway,
    # directly beneath it. A LABEL THAT PRINTS REGARDLESS OF THE MEASUREMENT IS THE
    # SAME DEFECT AS A CONTROL THAT CANNOT FIRE, wearing prose instead of code. The
    # verdict now comes from the count, so the two cannot disagree.
    # ★ `-uno` REMOVED 2026-09-10 (entry 803, O-WATCH #605 §3). It is
    # --untracked-files=no, so a file that has never been `git add`ed was not
    # merely unnamed -- IT WAS NOT COUNTED, and the verdict read EMPTY while a
    # whole authored document waited. Their demonstration named the fixture file
    # `PROVENANCE.md` on purpose: Gate 5's next required rights artefact is a NEW
    # FILE, which is exactly the shape the -uno form could not see. The morning's
    # miss was a tracked file going stale; its next form is a new file never
    # appearing at all. ★ COUNT AND NAMES STAY ON ONE COMMAND (#605 §2 verified
    # that property of the previous fix and it must survive): both read the same
    # `git status --porcelain`, so PENDING(3:a,b) with a third unnamed is
    # impossible. Untracked entries are marked `?` so "authored, never added"
    # is distinguishable from "tracked and modified" at a glance.
    _g_n=$( cd "$_g_ap" 2>/dev/null && git status --porcelain 2>/dev/null | /usr/bin/grep -c . )
    [ -n "$_g_n" ] || _g_n=UNMEASURED
    # ★ NAME THE POPULATION, NOT JUST ITS SIZE. 2026-09-10, entry 801. `PENDING(2)`
    # printed on every tick for nine hours while ONE of the two was WATCH-log (dirty
    # ~20 min, fenced every tick) and the other was RELEASE-RIGHTS-1.0.md, dirty since
    # 02:15 and carrying the operator's OWN amended §2.9 -- a release-blocking rights
    # law that was NOT ON MAIN while I cited it. I supplied the population from memory
    # every tick and supplied it wrong. A FIELD WHOSE NORMAL VALUE IS NON-ZERO CARRIES
    # NO INFORMATION IN ITS MAGNITUDE: 2 looked exactly like the 2 of every other tick.
    # Paths are joined with `,` and no spaces so the field-scoped extractor
    # (`sed 's/.*fence=//;s/ .*//'`) still reads the whole field.
    _g_fn=$( cd "$_g_ap" 2>/dev/null && git status --porcelain 2>/dev/null \
             | /usr/bin/awk '{ st=substr($0,1,2); path=substr($0,4);
                               if (st == "??") printf "?%s\n", path;
                               else printf "%s\n", path }' \
             | /usr/bin/head -4 | /usr/bin/tr '\n' ',' \
             | /usr/bin/sed 's/,$//' )
    if [ "$_g_n" != "UNMEASURED" ] && [ "$_g_n" -gt 4 ] 2>/dev/null; then
        _g_fn="$_g_fn,+$((_g_n - 4))"
    fi
    if [ "$_g_n" = "0" ]; then _g_fl=EMPTY
    elif [ -z "$_g_fn" ]; then _g_fl="PENDING($_g_n)"
    else _g_fl="PENDING($_g_n:$_g_fn)"; fi
    # paste= joins the ground line for the same reason fence= did: it is a
    # DERIVED VERDICT that prints every tick without anyone remembering it (§4).
    _g_p=$(p_paste)
    # pins= : THE CANONICAL COMPARISON, DERIVED NOT REMEMBERED. Placed BEFORE
    # paste= because paste='s DRIFT form carries spaces and must stay last.
    _g_pa=$(printf '%s' "$_g_a" | sed 's/.*tagset=//;s/ .*//')
    _g_pb=$(printf '%s' "$_g_b" | sed 's/.*tagset=//;s/ .*//')
    _g_pd=""
    if [ "$_g_pa" != "$_CANON_PIN_APPNAME" ]; then
        _g_pd="appname:$_g_pa!=$_CANON_PIN_APPNAME"
    fi
    if [ "$_g_pb" != "$_CANON_PIN_BOGOS" ]; then
        _g_pd="${_g_pd:+$_g_pd,}bogos:$_g_pb!=$_CANON_PIN_BOGOS"
    fi
    if [ -n "$_g_pd" ]; then _g_pn="DRIFT($_g_pd)"; else _g_pn=OK; fi
    printf 'GROUND appname[%s] bogos[%s] v5.6-freeze=%s/%s pins=%s fence=%s paste=%s\n' \
      "$(printf '%s' "$_g_a" | sed 's/ ALL-ARMS-rc0//')" \
      "$(printf '%s' "$_g_b" | sed 's/ ALL-ARMS-rc0//')" \
      "$_g_k" "$_g_f" "$_g_pn" "$_g_fl" "$_g_p"
}

# p_paste — DOES THE UNBLOCK PASTE STILL COVER THE OPEN QUESTION SET?
#
# Added 2026-08-29 after the paste drifted TWICE in one day, in two different
# ways: first in its NUMBERS (five superseded figures for Q-D9 while the
# question body had been re-measured twice -- G-542), then in its SET (it said
# FIVE and listed five while NINE were open; Q-D12..Q-D15 were filed after it
# was written and never reached it). Hand-fixing it twice is what §4 calls a
# rule; this is the check moved into the instrument so it needs no discipline.
#
# THE PREDICATE, and both halves were WRONG on their first draft -- recorded
# because the corrections are the reason this arm can be trusted:
#   * ANSWERED is the question's OWN `**ANSWER:` line with the `______` slot
#     filled. A first draft used "the block contains a checkmark" and marked
#     Q-D6 ANSWERED -- the mark belonged to the NEXT section's heading, which
#     carries no Q-D id and so was not a split point. The block boundary now
#     ends at the next heading of ANY level.
#   * LISTED is a Q-D id at the START of a line inside the paste's fenced
#     block. A first draft counted every id in the block and picked up `Q-D4`
#     from a cross-reference inside Q-D15's prose. A mention is not a row.
# Both are §11's disease: a corpus that discusses its own subject sets a
# false-positive floor, and a predicate is an instrument with the same duties.
#
# Paths are PARAMETERS for the same reason as p_ground's: the UNMEASURED
# branch must be reachable from a fixture.
# The WELL-FORMED shapes of p_paste's verdict, as a predicate with ONE home.
#
# THE VERDICT NAMES ITS POPULATION (`(Q-D)`), added 2026-09-05 against entry
# 636. That entry: A-03 sat blocked on an operator word for a week with NO
# unblock paste, while this probe read `8/8` green the whole time -- because
# its population is Q-D QUESTION IDS scraped from QUESTIONS-FOR-BILL.md, and a
# TASK-level block cannot appear in it and therefore cannot DRIFT. The verdict
# never lied; I read a question-coverage number as a paste-coverage number.
# This is the SAME remedy #420-A applied one level down (VACUOUS names its
# denominator's provenance): a verdict that states its population cannot be
# mistaken for a broader one.
#
# ⛔ NARROWER THAN 636 ASKED FOR, DELIBERATELY. 636 said the population SHOULD
# cover task blocks. It is not cleanly derivable -- there is no machine-readable
# list of 'tasks blocked on a word'; landables are a judgement this desk reports,
# not a set the tree carries. Inventing a fragile second population would be a
# guard that goes wrong quietly, which is the failure this file exists against.
# What IS fixed is the misreading that cost the week.
# Echoes "ok", or the digit-normalised shape that failed.
#
# DRIFT WAS MISSING HERE AND THAT WAS THE WHOLE BUG (Finding C, operator's word
# 2026-09-03). p_paste emits THREE honest verdicts -- VACUOUS, N/N and DRIFT --
# and the live arm accepted only the first two, so an accurate DRIFT report
# reddened the selftest. A benign red is the guard that trains its reader to
# stop reading it (G-514), and this one had already been carried across a
# stand-down as a known-not-fixed.
#
# DRIFT was never untested: `paste-missing` and `paste-extra` below pin both of
# its forms exactly. The accept-list was simply narrower than the function it
# was checking -- a guard and its subject disagreeing with nothing comparing
# them (G-519).
#
# ONE HOME IS LOAD-BEARING: if the selftest restated this pattern it would pass
# by construction against its own copy, which is the two-copy failure (§0a) in
# the shape §11 names -- a control that tests its own restatement proves
# nothing. The arms below feed shapes THROUGH this function.
p_paste_shape() {
    # An AMBIG suffix rides ON a verdict; it never replaces one. Strip it, but
    # only when it NAMES ids -- a bare "!AMBIG()" is noise and must not pass.
    _ps_v="${1:-}"
    case "$_ps_v" in
        *'!AMBIG('*')')
            _ps_a="${_ps_v#*'!AMBIG('}"; _ps_a="${_ps_a%)}"
            case "$_ps_a" in
                Q-D[0-9]*) _ps_v="${_ps_v%%'!AMBIG('*}" ;;
                *) printf '%s' "${1:-}" | sed 's/[0-9][0-9]*/N/g'; return 0 ;;
            esac
            ;;
    esac
    case "$(printf '%s' "$_ps_v" | sed 's/[0-9][0-9]*/N/g')" in
        "VACUOUS(Q-D open=N)")        echo ok ;;
        "N/N(Q-D)")                   echo ok ;;
        "DRIFT(open=N listed=N "*")") echo ok ;;
        *) printf '%s' "${1:-}" | sed 's/[0-9][0-9]*/N/g' ;;
    esac
}

p_paste() {
    _pp_q=${1:-$(_p_tree)/reports/QUESTIONS-FOR-BILL.md}
    _pp_l=${2:-$(_p_tree)/reports/INTEGRATION-log.md}
    if [ ! -r "$_pp_q" ] || [ ! -r "$_pp_l" ]; then
        printf 'UNMEASURED'
        return 0
    fi
    QF="$_pp_q" LF="$_pp_l" python3 - <<'PY' 2>/dev/null || printf 'UNMEASURED'
import io, os, re
key = lambda s: int(s.split('-D')[1])
q = io.open(os.environ['QF'], encoding='utf-8').read()
heads = [(m.start(), m.group(1))
         for m in re.finditer(r'^#{2,3} .*?(Q-D\d+)', q, re.M)]
# O-WATCH #447 counted 31 headings against 35 ids and asked where the gap was.
# The gap is benign (5 ids are bold sub-items, 1 id is matched twice), but the
# DOUBLE MATCH is not: `answered` is a membership test, so ANY slice bearing an
# **ANSWER: silences the id -- including a slice that is merely PROSE ABOUT it.
# A prose heading aimed at an OPEN question therefore drops a live human
# decision out of the paste while the verdict stays green. Measured by mutation
# 2026-09-04: injecting one such heading took Q-D23 from the open set and moved
# the verdict 8/8 -> 7/7, shape still "ok". Ambiguity in a control is a finding,
# not something to resolve silently, so it is NAMED and rides on the verdict.
_hids = [h[1] for h in heads]
ambig = sorted({x for x in _hids if _hids.count(x) > 1}, key=key)
filed, answered = [], []
for i, (pos, qid) in enumerate(heads):
    end = heads[i + 1][0] if i + 1 < len(heads) else len(q)
    nxt = re.search(r'^#{1,3} ', q[pos + 3:end], re.M)   # ANY heading closes the block
    if nxt:
        end = pos + 3 + nxt.start()
    body = q[pos:end]
    filed.append(qid)
    m = re.search(r'\*\*ANSWER:.*', body)
    if m and '______' not in m.group(0):
        answered.append(qid)
openq = sorted({x for x in filed if x not in answered}, key=key)

L = io.open(os.environ['LF'], encoding='utf-8').read().split('\n')
try:
    s = next(i for i, l in enumerate(L)
             if l.startswith('# ') and 'UNBLOCK PASTE' in l
             and re.search(r'O-INT #\d+', l))
    f1 = next(i for i in range(s, len(L)) if L[i].strip() == '```')
    f2 = next(i for i in range(f1 + 1, len(L)) if L[i].strip() == '```')
except StopIteration:
    print('UNMEASURED', end='')
    raise SystemExit(0)
listed = sorted({m.group(1) for m in
                 (re.match(r'^(Q-D\d+)\s', l) for l in L[f1:f2]) if m}, key=key)

missing = [x for x in openq if x not in listed]
extra   = [x for x in listed if x not in openq]
if not missing and not extra:
    # O-WATCH #420-A: "0/0" and "16/16" are the SAME SHAPE, so a reader cannot
    # tell a check that covered sixteen from one whose denominator went to zero.
    # The check is NOT wrong -- with no open questions "is every open question
    # listed?" is trivially yes -- but its VERDICT could not carry that. Remedy
    # is the fleet's existing move: oint-fence.sh names its own independence
    # ("both counted independently") for exactly this reason. A verdict that
    # states its denominator's provenance cannot be mistaken for a measurement.
    if not openq:
        out = 'VACUOUS(Q-D open=0)'
    else:
        out = '%d/%d(Q-D)' % (len(listed), len(openq))
else:
    bits = ['open=%d' % len(openq), 'listed=%d' % len(listed)]
    if missing:
        bits.append('missing=' + ','.join(missing))
    if extra:
        bits.append('extra=' + ','.join(extra))
    out = 'DRIFT(' + ' '.join(bits) + ')'
if ambig:
    out += '!AMBIG(' + ','.join(ambig) + ')'
print(out, end='')
PY
}

# probe_selftest — every arm must FIRE and every arm must be able to FAIL.
# A selftest whose negative arm cannot fail is the vacuous green these exist against.
probe_selftest() {
    _t=$(mktemp -d) || return 9
    _f=0; _n=0; _u=0   # _u: arms that could not run (WORD 24 RULING A)
    _t0=$(date +%s)
    _ck() { _n=$((_n+1)); if [ "$2" = "$3" ]; then :; else echo "  FAIL $1: got [$2] want [$3]"; _f=$((_f+1)); fi; }

    printf 'alpha\nbeta\nalpha\n' > "$_t/a.txt"

    # p_count: positive, ZERO (the || echo 0 trap), and missing-file
    _ck "count-hit"     "$(p_count "$_t/a.txt" 'alpha')"        "2"
    _ck "count-ZERO"    "$(p_count "$_t/a.txt" 'gamma')"        "0"
    _ck "count-missing" "$(p_count "$_t/nope.txt" 'x' 2>/dev/null)" "UNMEASURED-NO-FILE($_t/nope.txt)"

    # p_rc: true and false must differ, and a pipeline must not mask
    _ck "rc-true"  "$(p_rc true)"  "0"
    _ck "rc-false" "$(p_rc false)" "1"
    _ck "rc-127"   "$(p_rc /nonexistent/cmd)" "127"

    # p_paste — ONE ARM THAT MUST PASS, TWO THAT MUST FIRE, ONE UNMEASURED.
    # The fixtures are built from the REAL files so the predicate is exercised
    # against the corpus it will actually read (§11: a pattern never shown to
    # match anything in the target file is not an instrument for that file).
    _pq=$(_p_tree)/reports/QUESTIONS-FOR-BILL.md
    _pl=$(_p_tree)/reports/INTEGRATION-log.md
    if [ -r "$_pq" ] && [ -r "$_pl" ]; then
        # must PASS: the live pair agrees. The verdict is now one of TWO shapes,
        # and which one is itself the finding (#420-A):
        #   VACUOUS(open=0)  the open set is empty -- nothing was covered
        #   N/N              N open questions, all of them listed
        # ★★★ THIS ARM USED TO PIN "N/N" WITH DIGITS NORMALISED, WHICH MEANT IT
        # PASSED IDENTICALLY ON 0/0 AND ON 16/16 -- the selftest INHERITED the
        # exact blindness #420-A found in the verdict. A must-pass arm that
        # cannot tell an empty population from a full one is not measuring the
        # property it names. It now accepts either shape and the SHAPE IS NAMED.
        _pv=$(p_paste "$_pq" "$_pl")
        case "$(p_paste_shape "$_pv")" in
            ok) _ck "paste-live-shape" "ok" "ok" ;;
            *)  _ck "paste-live-shape" "$(p_paste_shape "$_pv")" "a well-formed p_paste shape" ;;
        esac
        # THE SHAPE PREDICATE'S OWN ARMS (Finding C). Three must ACCEPT and two
        # must REJECT -- because an accept-list that accepts everything is not a
        # check, and UNMEASURED must stay a finding rather than become a pass.
        # Fed THROUGH p_paste_shape, never against a restatement of its pattern.
        _ck "shape-accepts-vacuous" "$(p_paste_shape 'VACUOUS(Q-D open=0)')" "ok"
        _ck "shape-accepts-n-over-n" "$(p_paste_shape '14/14(Q-D)')" "ok"
        _ck "shape-accepts-drift" \
            "$(p_paste_shape 'DRIFT(open=14 listed=13 missing=Q-D35)')" "ok"
        _ck "shape-accepts-drift-extra" \
            "$(p_paste_shape 'DRIFT(open=2 listed=2 missing=Q-D502 extra=Q-D99)')" "ok"
        # must REJECT: an unreadable input is a finding, not a shape.
        _ck "shape-rejects-unmeasured" "$(p_paste_shape 'UNMEASURED')" "UNMEASURED"
        # must REJECT: a malformed verdict must not be swallowed.
        _ck "shape-rejects-garbage" "$(p_paste_shape 'WOBBLE(7)')" "WOBBLE(N)"
        # must ACCEPT the AMBIG suffix -- the LIVE corpus carries one today, so
        # a shape predicate that rejected it would turn a true warning into a
        # daily red and the fleet would learn to ignore it.
        _ck "shape-accepts-ambig" "$(p_paste_shape '8/8(Q-D)!AMBIG(Q-D6)')" "ok"
        # must REJECT a suffix that names NOTHING: "!AMBIG()" would otherwise be
        # a way to append the word without the evidence.
        _ck "shape-rejects-ambig-empty" "$(p_paste_shape '8/8(Q-D)!AMBIG()')" \
            "N/N(Q-D)!AMBIG()"
        _ck "shape-rejects-ambig-junk" "$(p_paste_shape '8/8(Q-D)!AMBIG(bogus)')" \
            "N/N(Q-D)!AMBIG(bogus)"
        # THE NON-VACUOUS PATH IS ALREADY PINNED, and by an arm that cannot be
        # satisfied vacuously: `paste-synth-live` below builds a fixture with a
        # NON-EMPTY open set and requires exactly "2/2". If the vacuity test ever
        # widened to swallow a real population, that arm goes red -- so no extra
        # arm is added here. (One was drafted and deleted: it compared a missing
        # file against its own fallback string, which is a control that cannot
        # fire, and `bash -n` accepted it -- 10b-E, in the commit fixing 10b-E's
        # own family.)
        # must FIRE: drop a listed row and the missing id must be NAMED.
        #
        # ★★★ THE VICTIM IS DERIVED, NOT TYPED -- AND THIS IS THE SECOND TIME THIS
        # ARM DIED OF A BAKED-IN LITERAL. The first version asserted
        # "DRIFT(open=9 listed=8 missing=Q-D15)" verbatim and went RED when a
        # tenth question was filed; I masked the COUNTS and left the NAME, and on
        # 2026-08-29 the operator ANSWERED Q-D15 -- so `grep -v '^Q-D15 '` removed
        # a row that was no longer there, the verdict stayed n/n, and the arm
        # reported FAIL for a reason with nothing to do with the property under
        # test. A control anchored to a live-corpus MEMBER destroys itself when
        # the corpus MOVES, exactly as §11's name-based control destroys itself
        # when documented -- and answering questions is the desk doing its job,
        # so this guard reddened as a REWARD for progress (G-514's cry-wolf shape).
        # Both the count and the identity are now derived from the live pair.
        # ★★★★★ AND IT DIED A THIRD TIME, ON 2026-08-30, FROM THE SAME CAUSE THE
        # TWO PARAGRAPHS ABOVE DIAGNOSE. The open set reached ZERO -- every
        # question answered -- so the paste was regenerated with NO fenced rows,
        # and both must-FIRE arms went quiet: there was nothing left to drop or
        # rename. ★★★★ The desk emptying its own queue is the desk SUCCEEDING,
        # and this guard reddened for it. That is G-514's cry-wolf shape arriving
        # a third time, and the third time is the one that kills the strategy:
        # DERIVING the victim from the live corpus is not enough, because a
        # corpus can go EMPTY as legitimately as it can move.
        # ★★★ SO THE FIXTURE IS NOW SYNTHETIC (G-508): a two-question file and a
        # matching paste, built here, mutated here, and unreachable from anything
        # the desks do. Answering every question can no longer silence this arm.
        # ★★ The LIVE pair keeps its own must-PASS arm above -- that is what
        # checks the real corpus -- so nothing is lost by making the FIRE arms
        # synthetic. What changes is that the fire arms now test the PROPERTY
        # (does p_paste name a missing id, does it name an extra one) instead of
        # testing the property AND the current contents of two report files.
        printf '## Q-D501 synthetic alpha\n  **ANSWER:** ______\n\n## Q-D502 synthetic beta\n  **ANSWER:** ______\n' \
            > "$_t/q-synth.md"
        printf '# UNBLOCK PASTE — O-INT #8 synthetic\n\n```\nQ-D501 alpha\nQ-D502 beta\n```\n' \
            > "$_t/p-synth.md"
        # must PASS on the synthetic pair too: a fixture that cannot agree cannot
        # then be shown to disagree.
        _ck "paste-synth-live" "$(p_paste "$_t/q-synth.md" "$_t/p-synth.md")" "2/2(Q-D)"
        # ★★★★★ THE ARM THAT DISCRIMINATES #420-A's FIX, AND IT IS SYNTHETIC FOR
        # THE SAME REASON THE TWO ABOVE ARE: a ZERO-OPEN fixture -- every ANSWER
        # slot filled, an empty fenced block -- must read VACUOUS and NOT "0/0".
        # Against the pre-fix verdict this arm goes RED with the defect's own
        # signature (got [0/0] want [VACUOUS(open=0)]), which is the only way to
        # know it can fire. The `paste-live-shape` arm above CANNOT do this job:
        # it accepts either shape by design, so "0/0" normalises to "N/N" and
        # passes there whether the fix is present or not.
        printf '## Q-D601 synthetic closed\n  **ANSWER:** RULED\n' > "$_t/q-zero.md"
        printf '# UNBLOCK PASTE — O-INT #8 synthetic zero\n\n```\n```\n' > "$_t/p-zero.md"
        _ck "paste-zero-open-is-VACUOUS" \
            "$(p_paste "$_t/q-zero.md" "$_t/p-zero.md")" "VACUOUS(Q-D open=0)"
        _pvic=Q-D502
        /usr/bin/grep -v "^$_pvic " "$_t/p-synth.md" > "$_t/log-drop.md"
        _ck "paste-missing" \
            "$(p_paste "$_t/q-synth.md" "$_t/log-drop.md")" \
            "DRIFT(open=2 listed=1 missing=$_pvic)"
        sed "s/^$_pvic /Q-D99 /" "$_t/p-synth.md" > "$_t/log-extra.md"
        _ck "paste-extra" "$(p_paste "$_t/q-synth.md" "$_t/log-extra.md")" \
            "DRIFT(open=2 listed=2 missing=$_pvic extra=Q-D99)"
        # ★★★★★ O-WATCH #447's COUNT, TURNED INTO AN ARM. They measured 31
        # headings against 35 ids. Most of that gap is benign, but it exposed a
        # real one: `answered` is a MEMBERSHIP test, so a second heading that
        # merely mentions an id and carries an **ANSWER: silences that id. Aimed
        # at an OPEN question, it drops a live human decision from the paste and
        # the verdict stays green. This arm injects exactly that heading and
        # requires BOTH halves of the signature: the id leaves the open set
        # (open=1, and it shows up as `extra` because the paste still lists it)
        # AND the suffix NAMES it. Against the pre-fix code the suffix is absent,
        # so the arm goes red with the defect's own signature.
        cp "$_t/q-synth.md" "$_t/q-dup.md"
        printf '\n### A NOTE MENTIONING Q-D501 IN PASSING\n\n  **ANSWER:** belongs to another question.\n' \
            >> "$_t/q-dup.md"
        _ck "paste-dup-heading-is-AMBIG" \
            "$(p_paste "$_t/q-dup.md" "$_t/p-synth.md")" \
            "DRIFT(open=1 listed=2 extra=Q-D501)!AMBIG(Q-D501)"
        # ★★ THE NEGATIVE CONTROL IS ALREADY ABOVE AND IS NOT A NEW ARM:
        # `paste-synth-live` runs the SAME fixture WITHOUT the extra heading and
        # requires exactly "2/2" -- no suffix. If AMBIG ever fired on a clean
        # corpus, that arm goes red, so this pair cannot both pass vacuously.
        # must be UNMEASURED, not a silent pass, when an input is unreadable.
        _ck "paste-unmeasured" "$(p_paste "$_pq" "$_t/nope.md")" "UNMEASURED"
    fi

    # p_out: stderr must SURVIVE, which is the whole point
    _ck "out-stderr" "$(p_out sh -c 'echo boom >&2; exit 3' | head -2 | tr '\n' '/')" "rc=3/boom/"

    # p_epoch: must be numeric, and must ORDER two files correctly even when the
    # later file carries the earlier clock face. Forced with an explicit backdate.
    : > "$_t/old"; touch -t 202608102141.57 "$_t/old"
    : > "$_t/new"; touch -t 202608261843.56 "$_t/new"
    _eo=$(p_epoch "$_t/old" m); _en=$(p_epoch "$_t/new" m)
    _ck "epoch-numeric" "$(printf '%s' "$_eo" | tr -d '0-9' | wc -c | tr -d ' ')" "0"
    if [ "$_en" -gt "$_eo" ]; then _ck "epoch-order" ok ok; else _ck "epoch-order" bad ok; fi
    # NEGATIVE CONTROL: the clock-face form must get this WRONG, or the arm is vacuous
    _co=$(stat -f '%Sm' -t '%H:%M:%S' "$_t/old"); _cn=$(stat -f '%Sm' -t '%H:%M:%S' "$_t/new")
    if [ "$_cn" \< "$_co" ] 2>/dev/null || awk -v a="$_cn" -v b="$_co" 'BEGIN{exit !(a<b)}'; then
        _ck "clockface-INVERTS-as-expected" ok ok
    else
        _ck "clockface-INVERTS-as-expected" "did-not-invert" ok
    fi

    # p_verdict / p_partition: both branches
    _ck "verdict-match"  "$(p_verdict x 5 5 | /usr/bin/grep -c MATCH)"    "1"
    _ck "verdict-differ" "$(p_verdict x 5 6 | /usr/bin/grep -c DIFFERS)"  "1"
    _ck "part-closes"    "$(p_partition x 1 2 3 6 | /usr/bin/grep -c CLOSES)" "1"
    _ck "part-short"     "$(p_partition x 1 2 3 7 | /usr/bin/grep -c SHORT)"  "1"

    # p_msg_unsafe: must DETECT a real executable in a backticked span, must NOT
    # count a backticked non-command, and must not be fooled by prose without spans.
    printf 'run `git status` then `chmod -R u+w /tmp/x` now\n' > "$_t/danger.md"
    printf 'the `frobnicate_nonexistent_xyz` verb and `not a command here`\n' > "$_t/safe.md"
    printf 'no spans at all in this line\n' > "$_t/plain.md"
    _ck "unsafe-detects"   "$(p_msg_unsafe "$_t/danger.md")" "2"
    _ck "unsafe-ignores"   "$(p_msg_unsafe "$_t/safe.md")"   "0"
    _ck "unsafe-no-spans"  "$(p_msg_unsafe "$_t/plain.md")"  "0"

    # p_rowzero: the arm that matters is that a NON-REPO is INAPPLICABLE, not clean.
    mkdir -p "$_t/norepo"
    _ck "rowzero-norepo"  "$(p_rowzero "$_t/norepo" | sed 's/(.*//')" "UNMEASURED-NOT-A-REPO"
    _ck "rowzero-nodir"   "$(p_rowzero "$_t/absent" | sed 's/(.*//')" "UNMEASURED-NO-DIR"
    # and a REAL repo must report figures, not an UNMEASURED verdict
    ( cd "$_t" && git init -q repo && cd repo && git config user.email t@t \
      && git config user.name t && : > f && git add f && git commit -qm c ) 2>/dev/null
    _ck "rowzero-real"    "$(p_rowzero "$_t/repo" | sed 's/ .*//')" "tags=0"
    # NEGATIVE CONTROL: a tag must move the figure, or the arm proves nothing
    ( cd "$_t/repo" && git tag v9.9-probe ) 2>/dev/null
    _ck "rowzero-sees-tag" "$(p_rowzero "$_t/repo" | sed 's/ .*//')" "tags=1"
    # G-455 (O-WATCH, 2026-08-27): THE CHECK YOU RUN MOST OFTEN IS THE ONE LEAST
    # LIKELY TO HAVE BEEN WATCHED TO FAIL. The arm above moves tags= and at_head=
    # and NEVER MOVES v56= -- so the arm that detects the ACTUAL row-zero emergency
    # was undemonstrated. A v9.9 tag proves the counter counts; it does not prove
    # this counter counts THE THING IT IS FOR. Both tag kinds, because the emergency
    # could be either and an annotated tag is a different object.
    _ck "rowzero-v56-quiet"     "$(p_rowzero "$_t/repo" | sed 's/.*v56=//;s/ .*//')" "0"
    ( cd "$_t/repo" && git tag v5.6-freeze ) 2>/dev/null
    _ck "rowzero-v56-FIRES"     "$(p_rowzero "$_t/repo" | sed 's/.*v56=//;s/ .*//')" "1"
    ( cd "$_t/repo" && git tag -a v5.6.1 -m annotated ) 2>/dev/null
    _ck "rowzero-v56-ANNOTATED" "$(p_rowzero "$_t/repo" | sed 's/.*v56=//;s/ .*//')" "2"
    # and a NEAR-MISS must NOT count: v5.5 is the frozen tag and must never read as v5.6
    ( cd "$_t/repo" && git tag v5.5-freeze ) 2>/dev/null
    _ck "rowzero-v55-EXCLUDED"  "$(p_rowzero "$_t/repo" | sed 's/.*v56=//;s/ .*//')" "2"
    # G-456 (O-WATCH, 2026-08-27): A PATTERN CONTAINING A REGEX METACHARACTER IS TWO
    # DIFFERENT PREDICATES DEPENDING ON WHICH MATCHER READS IT, AND BOTH LOOK THE
    # SAME. The dot in v5.6 is a LITERAL in fnmatch (git tag -l) and a WILDCARD in
    # regex (grep). p_rowzero uses grep with '^v5\.6', so the escape is load-bearing:
    # unescaped it would count v5X6-anything -- AN OVER-REPORT ON THE CHECK WHOSE
    # OVER-REPORTING DISCREDITS THE SEAT. The decoy tag proves the escape works AND
    # the paired unescaped control proves the decoy is capable of being matched, so
    # neither arm is vacuous.
    ( cd "$_t/repo" && git tag v5X6-decoy ) 2>/dev/null
    _ck "rowzero-metachar-LITERAL" "$(p_rowzero "$_t/repo" | sed 's/.*v56=//;s/ .*//')" "2"
    _ck "rowzero-metachar-CONTROL" \
        "$( cd "$_t/repo" 2>/dev/null && git tag | /usr/bin/grep -c '^v5.6' )" "3"
    # nearmiss field: must be 1 when v5.5-freeze is present (it was tagged above),
    # and the SECOND repo proves it can read 0 -- otherwise "1 everywhere" would be
    # indistinguishable from a stuck field.
    _ck "rowzero-nearmiss-PRESENT" "$(p_rowzero "$_t/repo" | sed 's/.*nearmiss=//;s/ .*//')" "1"
    ( cd "$_t" && git init -q bare2 && cd bare2 && git config user.email t@t \
      && git config user.name t && : > g && git add g && git commit -qm c \
      && git tag v1.0.0 ) 2>/dev/null
    _ck "rowzero-nearmiss-ABSENT"  "$(p_rowzero "$_t/bare2" | sed 's/.*nearmiss=//;s/ .*//')" "0"
    # ignored=: must be EMPTY when nothing is ignored and must NAME the path when
    # something is. A count would satisfy the first arm and hide the second, which is
    # exactly the opacity this field exists to remove.
    _ck "rowzero-ignored-EMPTY" "$(p_rowzero "$_t/bare2" | sed 's/.*ignored=//;s/ .*//')" "[]"
    ( cd "$_t/bare2" && printf 'junk.tmp\n' > .gitignore && : > junk.tmp \
      && git add .gitignore && git commit -qm ig ) 2>/dev/null
    _ck "rowzero-ignored-NAMES" "$(p_rowzero "$_t/bare2" | sed 's/.*ignored=//;s/ .*//')" "[junk.tmp]"
    # tracked_modified: never had an arm either -- same gap as v56 had. It must read
    # 0 on a clean tree and MOVE when a TRACKED file is modified. And the paired arm
    # that matters: modifying an IGNORED file must NOT move it, which is the whole
    # distinction the rename exists to make legible.
    _ck "trackmod-CLEAN"  "$(p_rowzero "$_t/bare2" | sed 's/.*tracked_modified=//;s/ .*//')" "0"
    ( cd "$_t/bare2" && printf 'changed\n' > g ) 2>/dev/null
    _ck "trackmod-FIRES"  "$(p_rowzero "$_t/bare2" | sed 's/.*tracked_modified=//;s/ .*//')" "1"
    # THE THIRD CATEGORY, no arm until 2026-08-28: an UNTRACKED file must NOT move
    # tracked_modified. g is already modified above, so the expected value stays 1 --
    # the arm FAILS if the untracked file is counted, which would read 2.
    ( cd "$_t/bare2" && printf 'scratch\n' > untracked_probe.txt ) 2>/dev/null
    _ck "trackmod-UNTRACKED-NOT-COUNTED" "$(p_rowzero "$_t/bare2" | sed 's/.*tracked_modified=//;s/ .*//')" "1"
    ( cd "$_t/bare2" && rm -f untracked_probe.txt ) 2>/dev/null
    # restore by WRITING the original bytes, not by checkout: 'git checkout -- FILE'
    # is a write wearing the word cleanup, it is the first line of my standing
    # hazards, and it has no place in an instrument even on a fixture path.
    ( cd "$_t/bare2" && : > g && printf 'noise\n' >> junk.tmp ) 2>/dev/null
    _ck "trackmod-IGNORES-IGNORED" "$(p_rowzero "$_t/bare2" | sed 's/.*tracked_modified=//;s/ .*//')" "0"
    # ARM 4 is O-WATCH's (#139) and it covers THE INCIDENT rather than the rule: the
    # G-135 residue at 02:10:06 was a .pyc that was CREATED, not modified. My three
    # arms above test modification only.
    ( cd "$_t/bare2" && : > second.tmp && printf 'second.tmp\n' >> .gitignore \
      && git add .gitignore && git commit -qm ig2 ) 2>/dev/null
    _ck "trackmod-NEW-IGNORED-FILE" "$(p_rowzero "$_t/bare2" | sed 's/.*tracked_modified=//;s/ .*//')" "0"
    _ck "ignored-SEES-NEW-FILE" \
        "$(p_rowzero "$_t/bare2" | sed 's/.*ignored=\[//;s/\].*//' | tr ',' '\n' | /usr/bin/grep -c 'second.tmp')" "1"
    # G-459 (O-WATCH, #139): A REPAIR VERIFIED ONLY AGAINST THE CURRENT STATE IS
    # VERIFIED AGAINST THE ONE STATE THAT CANNOT DISTINGUISH IT FROM THE DEFECT. The
    # ignored= arms above all ran on a CLEAN tree, where a status-code filter and a
    # naive field-2 split return identical lists. This arm dirties a TRACKED file
    # first, which is the only state in which the two forms differ.
    ( cd "$_t/bare2" && printf 'dirty\n' > g ) 2>/dev/null
    _ck "ignored-EXCLUDES-DIRTY-TRACKED" \
        "$(p_rowzero "$_t/bare2" | sed 's/.*ignored=\[//;s/\].*//' | tr ',' '\n' | /usr/bin/grep -c '^g$')" "0"
    _ck "ignored-STILL-NAMES-IGNORED" \
        "$(p_rowzero "$_t/bare2" | sed 's/.*ignored=\[//;s/\].*//' | tr ',' '\n' | /usr/bin/grep -c 'junk.tmp')" "1"

    # tagset= : THE ARM THAT SEPARATES A COUNT FROM A SET. A third repo, so these
    # cannot perturb the ordering-sensitive bare2 arms above.
    ( cd "$_t" && git init -q tagpin && cd tagpin && git config user.email t@t \
      && git config user.name t && : > f && git add f && git commit -qm c1 \
      && : > f2 && git add f2 && git commit -qm c2 && git tag t-one HEAD~1 ) 2>/dev/null
    _pin_a=$(p_rowzero "$_t/tagpin" | sed 's/.*tagset=//;s/ .*//')
    _cnt_a=$(p_rowzero "$_t/tagpin" | sed 's/.*tags=//;s/ .*//')
    # STABLE: reading twice must not move it, or every tick is a false alarm.
    _ck "tagset-STABLE" "$(p_rowzero "$_t/tagpin" | sed 's/.*tagset=//;s/ .*//')" "$_pin_a"
    # THE BLINDNESS ARM: t-one is on an ANCESTOR, so at_head= reads 0 while a tag
    # plainly exists. This is the exact shape a ceremony tag on the certified commit
    # would take, and it is asserted rather than described.
    _ck "tagset-at_head-IS-BLIND-ON-ANCESTOR" \
        "$(p_rowzero "$_t/tagpin" | sed 's/.*at_head=//;s/ .*//')" "0"
    # RETARGET: delete and recreate the SAME NAME at a DIFFERENT commit. The count
    # must NOT move and the pin MUST. Both halves are asserted, because the count arm
    # is what makes the pin arm mean something rather than merely pass.
    ( cd "$_t/tagpin" && git tag -d t-one && git tag t-one HEAD ) >/dev/null 2>&1
    _ck "tagset-RETARGET-COUNT-BLIND" "$(p_rowzero "$_t/tagpin" | sed 's/.*tags=//;s/ .*//')" "$_cnt_a"
    if [ "$(p_rowzero "$_t/tagpin" | sed 's/.*tagset=//;s/ .*//')" = "$_pin_a" ]; then
        _ck "tagset-RETARGET-MOVES-PIN" DID-NOT-MOVE MOVED
    else
        _ck "tagset-RETARGET-MOVES-PIN" MOVED MOVED
    fi
    # RENAME: same object, different name. The count is blind to this too.
    _pin_b=$(p_rowzero "$_t/tagpin" | sed 's/.*tagset=//;s/ .*//')
    ( cd "$_t/tagpin" && git tag t-two t-one && git tag -d t-one ) >/dev/null 2>&1
    _ck "tagset-RENAME-COUNT-BLIND" "$(p_rowzero "$_t/tagpin" | sed 's/.*tags=//;s/ .*//')" "$_cnt_a"
    if [ "$(p_rowzero "$_t/tagpin" | sed 's/.*tagset=//;s/ .*//')" = "$_pin_b" ]; then
        _ck "tagset-RENAME-MOVES-PIN" DID-NOT-MOVE MOVED
    else
        _ck "tagset-RENAME-MOVES-PIN" MOVED MOVED
    fi
    # EMPTY SET: a tagless repo must pin to the hash of NOTHING, not of one newline.
    ( cd "$_t/tagpin" && git tag -d t-two ) >/dev/null 2>&1
    _ck "tagset-EMPTY-IS-HASH-OF-NOTHING" \
        "$(p_rowzero "$_t/tagpin" | sed 's/.*tagset=//;s/ .*//')" \
        "$(printf '' | /usr/bin/shasum -a 256 | /usr/bin/cut -c1-12)"
    # ...and that value must NOT equal the hash of a bare newline, or the guard above
    # is untested by the case it was written for.
    if [ "$(printf '' | /usr/bin/shasum -a 256 | /usr/bin/cut -c1-12)" \
       = "$(printf '\n' | /usr/bin/shasum -a 256 | /usr/bin/cut -c1-12)" ]; then
        _ck "tagset-EMPTY-GUARD-IS-LOAD-BEARING" INDISTINGUISHABLE DISTINCT
    else
        _ck "tagset-EMPTY-GUARD-IS-LOAD-BEARING" DISTINCT DISTINCT
    fi
    # p_ground: the LINE must carry both pins and the freeze identity, and the
    # UNMEASURED fallback must be REACHABLE -- fired here against a fixture repo
    # that has no v5.6-freeze, which is the only way to know the branch is not
    # decoration. bare2 exists above and carries v1.0.0 only.
    _gl=$(p_ground "$_t/repo" "$_t/bare2")
    _ck "ground-HAS-PREFIX"   "$(printf '%s' "$_gl" | sed 's/ .*//')" "GROUND"
    _ck "ground-TWO-PINS"     "$(printf '%s' "$_gl" | /usr/bin/grep -o 'tagset=' | /usr/bin/grep -c .)" "2"
    _ck "ground-UNMEASURED-FIRES" \
        "$(printf '%s' "$_gl" | /usr/bin/grep -c 'v5.6-freeze=UNMEASURED/UNMEASURED')" "1"
    # ...and the SAME field must read a real value where the tag exists, or the arm
    # above would pass on a function that always prints UNMEASURED.
    if [ -n "$(_p_spec)" ]; then
        _gr=$(p_ground "$_t/repo" "$(_p_spec)")
        _ck "ground-REAL-FREEZE"  "$(printf '%s' "$_gr" | /usr/bin/grep -c 'v5.6-freeze=tag/')" "1"
    else
        # The control cannot run without the spec tree, and an arm that cannot
        # run must not read as one that passed (WORD 24 RULING A).
        echo "  UNMEASURED ground-REAL-FREEZE: APPNAME_SPEC_ROOT is unset"
        _u=$((_u+1))
    fi

    # pins= : MUST-PASS on the real repos and MUST-FIRE on a fixture. The pair
    # is the point -- a canonical comparison that only ever prints OK is the
    # same decoration as the prose it replaced, and the fixture repos have
    # tagsets that CANNOT equal the canonicals, so DRIFT there is not luck.
    if [ -n "$(_p_spec)" ]; then
        _ck "ground-PINS-OK-ON-REAL" "$(p_ground | sed 's/.*pins=//;s/ .*//')" "OK"
    else
        # Both canonical pins are compared in one field, and one of the two trees
        # is not configured here, so the field cannot be read as a pass either.
        echo "  UNMEASURED ground-PINS-OK-ON-REAL: APPNAME_SPEC_ROOT is unset"
        _u=$((_u+1))
    fi
    _ck "ground-PINS-DRIFT-ON-FIXTURE" \
        "$(p_ground "$_t/fence1" "$_t/repo" | sed 's/.*pins=//;s/(.*//;s/ .*//')" "DRIFT"
    # fence= : EMPTY on a clean tree, PENDING(n) when a TRACKED file is modified --
    # the exact state that printed a false "no review pending" label.
    # ★ THE FIRST VERSION OF THIS ARM WENT RED because I asserted "bare2 is clean at
    # this point" WITHOUT CHECKING and earlier arms had dirtied it. THE ARM WAS RIGHT
    # AND MY PRECONDITION WAS WRONG, so this uses a repo whose cleanliness is
    # established BY CONSTRUCTION rather than assumed from the arms above it.
    ( cd "$_t" && git init -q fence1 && cd fence1 && git config user.email t@t \
      && git config user.name t && : > f && git add f && git commit -qm c ) 2>/dev/null
    # Extract the fence FIELD, not "everything after fence=" -- the ground line
    # grew a `paste=` field after this arm was written and the greedy form
    # silently swallowed it. The arm went red, which is the arm working: a
    # field-scoped extractor cannot be broken by a field appended beside it.
    _ck "ground-FENCE-EMPTY" "$(p_ground "$_t/fence1" "$_t/repo" | sed 's/.*fence=//;s/ .*//')" "EMPTY"
    ( cd "$_t/fence1" && printf 'dirty\n' > f ) 2>/dev/null
    # ★ THE FIELD NOW NAMES ITS POPULATION (entry 801): the fixture's one dirty
    # tracked file is `f`, so the verdict must read PENDING(1:f), not PENDING(1).
    # Asserting the NAME and not just the count is the whole point of the change --
    # an arm that only checked the digit would have passed the old broken field.
    _ck "ground-FENCE-PENDING" "$(p_ground "$_t/fence1" "$_t/repo" | sed 's/.*fence=//;s/ .*//')" "PENDING(1:f)"
    # ★ DIRECTION 2 (O-WATCH #605 §3): a file AUTHORED BUT NEVER `git add`ed.
    # The `-uno` form returned EMPTY here -- not "unnamed", NOT COUNTED -- which is
    # the shape Gate 5's own next artefact will take. The fixture file is named
    # PROVENANCE.md for that reason. An arm that only covered direction 1 would
    # have passed the blind instrument and the seeing one alike.
    # ★ ORDER IS GIT'S, NOT MINE: `status --porcelain` emits tracked changes before
    # untracked, so the verdict reads `f,?PROVENANCE.md`. My first draft of this arm
    # asserted the reverse and WENT RED -- the arm was right and my expectation was
    # wrong, which is the same lesson as §1's fixture. DO NOT "fix" this by sorting:
    # git's order is stable and it puts the tracked half first on purpose.
    ( cd "$_t/fence1" && printf 'new\n' > PROVENANCE.md ) 2>/dev/null
    _ck "ground-FENCE-UNTRACKED" "$(p_ground "$_t/fence1" "$_t/repo" | sed 's/.*fence=//;s/ .*//')" "PENDING(2:f,?PROVENANCE.md)"
    rm -rf "$_t"

    # EXECUTED-ARM CHECKS (O-WATCH #405-A). These run the file as a COMMAND, which
    # is the only way to reach the arm at the bottom -- a sourced selftest cannot
    # observe it. The path is absolute by the same convention p_ground already uses
    # for its repo default; if it is ever wrong, all four arms below go red loudly
    # on the first run rather than skipping silently (a skipped arm is a control
    # that cannot fire, which is the very defect this arm was added to remove).
    # DO NOT ADD `--selftest` HERE: the child would run probe_selftest, which would
    # spawn again. Unbounded fork. The --selftest entry point is verified by hand.
    _self=$(_p_self_dir)/oint-probe.sh
    _ck "exec-self-present" "$([ -f "$_self" ] && echo yes || echo no)" "yes"
    _ck "exec-unknown-VOIDs" "$(bash "$_self" --no-such-flag >/dev/null 2>&1; echo $?)" "3"
    _ck "exec-bare-VOIDs"    "$(bash "$_self" >/dev/null 2>&1; echo $?)" "3"
    _ck "exec-help-passes"   "$(bash "$_self" --help >/dev/null 2>&1; echo $?)" "0"
    # MUST-PASS ARM (10b-L): the sourced path must stay inert even when the CALLER
    # has argv. This is the arm that would have caught the peer's proposed remedy,
    # which aborted a sourcing caller at the source line and left rc=127.
    _ck "source-with-caller-argv" "$(bash -c ". $_self; p_ground >/dev/null && echo ok" x --no-such-flag 2>&1)" "ok"
    # COST, REPORTED BESIDE ITS LOAD -- and deliberately NOT a threshold.
    #
    # 2026-09-03: this selftest was SIGTERM'd at 120s and `guards_selftest` took
    # 910s and still returned 0. Nothing was broken; a peer desk's sweep had the
    # machine at load 24. From inside a run, "hung" and "contended" are the same
    # observation, and this desk had no recorded cost to compare against.
    #
    # ★ A DURATION THRESHOLD WOULD BE THE WRONG INSTRUMENT HERE, and that is
    # measured rather than assumed: this same unchanged selftest has been seen at
    # 17s, 31s and >120s within one hour, purely on other desks' load. A budget
    # tight enough to catch a real hang would fire constantly and benignly -- the
    # cry-wolf shape this desk refused a peer's guard over (G-514).
    #
    # So the arm reports the PAIR and judges nothing. Elapsed alone is unreadable;
    # elapsed beside load is self-explaining, and a reader can attribute without
    # knowing what the number "should" be. The QA lane's runbook works because
    # their grid is a fixed job on a quiet machine; this is a small job on a
    # machine four desks share.
    _cost=$(( $(date +%s) - _t0 ))
    _load=$(uptime | sed 's/.*averages*:* *//' | cut -d' ' -f1 | tr -d ',')
    printf 'probe_selftest: %s checks, %s failed, %s UNMEASURED  [cost %ss @ load %s]\n' \
        "$_n" "$_f" "$_u" "$_cost" "${_load:-?}"
    # UNMEASURED is reported beside the failures and does NOT make the run green
    # by omission: an arm that could not run is named above and counted here.
    [ "$_f" -eq 0 ]
}

# ─────────────────────────────────────────────────────────────────────────────
# EXECUTED-RATHER-THAN-SOURCED ARM. O-WATCH #405-A, 2026-08-30.
#
# THE FILED FINDING WAS: `oint-probe.sh --nonsense-flag` returns a SILENT rc=0
# where build-cert.sh (then named demo-drift.sh) and oint-commit.sh both VOID(3) -- filed as a breach of a
# fleet convention, outstanding four ticks.
#
# ★ THE ACCUSATION IS REFUTED BY DESIGN AND THE REFUTATION IS IN THIS FILE'S OWN
# HEADER: "Source it; do not execute it." (line 17). Two structural markers agree
# -- NO SHEBANG and NO EXECUTABLE BIT. The convention it was measured against
# governs EXECUTABLES; this is a library. G-538: grep the spec for the SUBJECT
# before writing the report, and the subject was seventeen lines from the top.
#
# ★★ AND THE PEER'S REMEDY, ADOPTED VERBATIM, WOULD HAVE BEEN CATASTROPHIC
# (G-527). A top-level `case "$1"` in a sourced file reads THE CALLER'S argv.
# Demonstrated in a fixture rather than argued: a caller invoked as
# `caller.sh somearg` hit the unknown-argument branch AT THE SOURCE LINE, so
# every function below it went undefined -- `myfunc: command not found`, rc=127.
# A cosmetic complaint would have bought a library that breaks whenever its
# caller happens to have arguments.
#
# ★★★ WHAT SURVIVES IS SMALLER AND REAL, AND IT IS WHY THIS ARM EXISTS. A reader
# who TYPES `bash oint-probe.sh --selftest` -- which is exactly how #405-A was
# produced -- got NO OUTPUT and rc=0. That is a control that cannot fire,
# reaching its reader as a pass. The failure is not the missing VOID; it is that
# SILENCE AND SUCCESS ARE THE SAME BYTES. So: executing this file now says what
# it is, and the entry points a reader actually reaches for now work.
#
# THE DISCRIMINATOR IS `$0`, and it was run against every known-good invocation
# form BEFORE it was written here (G-780): `bash f`, `bash f --flag`, `sh f`,
# `bash -c '. f; fn'`, sourced-from-a-script, and -- the one that matters --
# SOURCED BY A CALLER THAT HAS ITS OWN ARGV. All six classify correctly and the
# sourced path stays byte-for-byte inert.
#
# ⛔ DO NOT ADD A `--selftest` ARM TO probe_selftest ITSELF. It would spawn
# `bash <this file> --selftest`, whose child runs probe_selftest, which spawns
# again -- an unbounded fork. The three arms below are the ones that terminate;
# the --selftest entry point is verified by running it once, by hand, which is
# recorded in the ledger rather than in a recursive arm.
# ─────────────────────────────────────────────────────────────────────────────
case "$0" in
*oint-probe.sh)
    case "${1:-}" in
        --selftest) probe_selftest; exit $? ;;
        --ground)   p_ground;       exit $? ;;
        -h|--help|"")
            printf 'oint-probe.sh — a SOURCEABLE LIBRARY of reporting primitives.\n'
            printf '  Normal use (this is the documented form, payload section 1):\n'
            printf "      bash -c '. scripts/instruments/oint-probe.sh; p_ground'\n"
            printf '  Executed directly, only these are accepted:\n'
            printf '      --selftest   run probe_selftest\n'
            printf '      --ground     print the GROUND line\n'
            printf '      --help       this text\n'
            printf 'Exit: 0 OK · 3 VOID (bare invocation, or an argument it does not know)\n'
            [ -z "${1:-}" ] && exit 3
            exit 0 ;;
        *)
            printf 'oint-probe: VOID - unknown argument %s (try --help)\n' "$1" >&2
            exit 3 ;;
    esac
    ;;
esac
