#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
# REUSE-IgnoreStart
# (The SPDX header above is this file's REAL one. Every occurrence below is the
#  LITERAL TOKEN this instrument greps for or writes into a fixture -- not a second
#  license tag. Same technique, and same reason, as scripts/gate.sh:5. Recorded
#  2026-09-05 against O-WATCH #482, which measured X-11 arm 1 red with these files
#  among the invalid expressions. The token stays BYTE-IDENTICAL to the gate's --
#  obfuscating it would defeat REUSE and destroy the by-eye audit against
#  step_spdx that entries 626 and 638 rest on.)
#
# oint-commit.sh -- stage EXPLICIT paths and refuse the two commit-time
# mistakes O-INT #8 made on 2026-08-29, both against rules that were in
# the payload being read every tick. Discipline did not hold; this does.
#
#   ARM 1  NEVER `git add -A`. Paths are named on the command line. The
#          08-29 lapse: `git add -A` swept O-WATCH's IN-FLIGHT
#          reports/WATCH-log.md into two of my own multi-file commits
#          (#344 in 5cfed3b7, #345/#346 in 62afd496). Ground had said
#          fence=EMPTY at tick start, so I had no reason to fence -- the
#          peer wrote DURING the tick and `-A` took it. The cost is not
#          the commit title: MINOR-17's PREVENTIVE arm never ran on
#          three reviews, which demotes it to merely detected (payload
#          section 6). Staging explicit paths makes the sweep impossible.
#
#   ARM 2  reports/WATCH-log.md is NEVER committed by this script. It has
#          exactly one door -- oint-fence.sh -- and its own path-scoped
#          commit. Naming it here is refused, not silently dropped.
#
#   ARM 3  An INTEGRATION-log entry must carry its ground line. Payload
#          section 1: "PASTE p_ground's LINE INTO EVERY ENTRY." Measured
#          08-29: entries 344-349, SIX CONSECUTIVE, carry no
#          `GROUND appname[` line; only 350 does. The payload already
#          diagnosed this exact shape ("the measurement never lapsed;
#          the record did") and I reproduced it anyway. So the check
#          moves to where the entry lands: if reports/INTEGRATION-log.md
#          is being committed and its ADDED lines carry no ground line,
#          refuse.
#
# Exit: 0 committed - 1 REFUSED (a check fired) - 2 nothing staged - 3 VOID
set -uo pipefail

LEDGER="reports/WATCH-log.md"
INTLOG="reports/INTEGRATION-log.md"
# ARM 3's ANCHOR IS POSITIONAL, AND CONTENT ANCHORS WERE TRIED FIRST.
# This ledger DOCUMENTS ITS OWN INSTRUMENTS, so any literal the guard
# searches for gets reproduced by the paragraph explaining the guard.
# Measured, twice, on this very file:
#   v1 searched `GROUND appname[` -- satisfied by the sentence
#      "...carry NO `GROUND appname[` line. Six consecutive..."
#      i.e. by the COMPLAINT about the absence it was built to detect.
#   v2 required three fields on one line -- satisfied by the sentence
#      explaining v2's anchor, which naturally listed all three.
# Strengthening the literal is NOT a convergent strategy: each stronger
# pattern is reproduced by its own documentation (payload section 11;
# G-508). So the anchor is STRUCTURAL. A ground line must sit on the
# line IMMEDIATELY AFTER the entry's `## NNN.` heading. Prose in a body
# cannot occupy that position, and describing this rule does not put a
# ground line under a heading.
# NOTE the DOUBLED backslashes. `awk -v` processes escape sequences in
# the assignment, so a single `\[` reaches the program as a bare `[` and
# opens an unterminated bracket expression.
#
# ★★★★★ AND THE DIRECTION THIS COMMENT USED TO CLAIM WAS BACKWARDS.
# It said the pattern "matches nothing and the guard fails OPEN". BOTH
# HALVES ARE FALSE, and O-WATCH #395-B measured them:
#   * BSD awk does not match nothing -- it ERRORS and aborts:
#       awk: nonterminated character class GROUND appname[tags=
#   * _ground_after_heading has ONE production caller (:142), and it reads
#       if [ "${_added:-0}" -lt 1 ]; then _die ...
#     so an empty result -- however produced -- REFUSES THE COMMIT.
# ★★★★ The true failure is FAIL-CLOSED OVER-REFUSAL: a broken pattern
# blocks EVERY commit, loudly, on the first one. Not fail-open.
# ★★★ THE TELL WAS INSIDE THIS COMMENT'S OWN NEXT SENTENCE: it said the
# bug was "caught by the must-pass arm". A must-pass arm is exactly what
# catches over-refusal; a fail-OPEN guard is caught by a must-FIRE arm.
# The evidence and the label contradicted each other and the label won for
# two days.
# ★★ WHY THE CORRECTION MATTERS RATHER THAN BEING PEDANTRY: a reader asking
# "could this bug have let an ungrounded entry through?" reads "fails OPEN"
# and answers YES. The right answer is NO -- nothing could have got past it,
# because nothing could get past it at all.
# ★ THE LESSON THE OLD COMMENT DREW IS STILL CORRECT AND STILL EARNED: a
# guard needs an arm that must SUCCEED as well as arms that must fire. It
# was simply attached to the wrong hazard.
HEAD_RE='^\\+## [0-9]+\\.'
GROUND_RE='GROUND appname\\[tags='
# Plain (non-awk) form of the heading pattern, for grep -c. Kept beside
# its awk twin so the two can never drift apart unnoticed.
HEAD_PLAIN='^+## [0-9]\+\.'

# Counts ground lines that sit IMMEDIATELY AFTER an entry heading.
# awk, not `grep -A1`: -A on a filtered stream re-inserts separators and
# counts context lines as matches.
_ground_after_heading() {
    awk -v h="$HEAD_RE" -v g="$GROUND_RE" '
        $0 ~ h { want = 1; next }
        want == 1 { if ($0 ~ g) c++; want = 0 }
        END { print c + 0 }'
}

# _STAGED_PATHS is empty until this invocation actually stages something.
# _die consults it so that EVERY refusal path -- not just the one I remembered
# to wrap -- returns the index to the state the entry check requires (G-796).
# The first version of this reset sat in an `if ! _check_groundline` branch and
# NEVER RAN, because _check_groundline reaches _die and _die exits the script.
# A cleanup placed after the call cannot run when the callee exits.
_STAGED_PATHS=()
_unstage() {
    [ "${#_STAGED_PATHS[@]}" -gt 0 ] || return 0
    git reset -q HEAD -- "${_STAGED_PATHS[@]}" 2>/dev/null
}
_die()  { _unstage; echo "oint-commit: REFUSED - $1" >&2; exit 1; }
_void() { echo "oint-commit: VOID - $1" >&2; exit 3; }

# ARM 2. A PATHSPEC IS NOT A PATH -- corrected 2026-08-29 on O-WATCH #352,
# reproduced here first with the control run before the test.
#
# This header used to claim "staging explicit paths makes the sweep
# impossible." THAT WAS FALSE, and it is my own G-526: a stated safe
# direction is a claim about EVERY arm of the pattern, not the arm its
# author had in mind. `oint-commit.sh msg reports` names an explicit
# argument, matches neither `$LEDGER` nor `*/$LEDGER` as a STRING, and
# git expands it to the whole directory -- sweeping the peer's ledger
# into my commit, which is the exact failure this script exists to stop.
#
# The fix is to stop comparing the strings I typed and ask GIT what the
# pathspecs resolve to. `ls-files -m -o` is the set `git add` would newly
# stage (modified + untracked); a ledger that is CLEAN resolves to nothing
# and must NOT be refused, or the guard cries wolf on every `reports/`
# commit and trains its reader to stop reading it (G-514).
_check_paths() {
    for p in "$@"; do
        case "$p" in
            "$LEDGER"|*/"$LEDGER")
                _die "$LEDGER goes through oint-fence.sh in its own path-scoped commit, never here" ;;
        esac
    done
    # Resolution check: what would these pathspecs ACTUALLY stage?
    _hits=$(git ls-files -m -o --exclude-standard -- "$@" 2>/dev/null \
            | /usr/bin/grep -x "$LEDGER")
    if [ -n "$_hits" ]; then
        _die "a pathspec here resolves to $LEDGER (it has uncommitted changes). \
That file goes through oint-fence.sh in its own commit -- name the other paths explicitly."
    fi
    return 0
}

# ARM 5. THE GATE'S OWN spdx PREDICATE, ON THE STAGED SET. ADDED 2026-09-05
# against entry 625: commit 0082aabe put reports/RELEASE-RIGHTS-1.0.md on main
# carrying the REUSE-canonical `SPDX-FileCopyrightText:` form, which step_spdx
# does not accept -- and turned the release gate RED for every desk. The check
# this desk ran before committing was `grep -c 'SPDX'` returning 2: a PROXY, not
# the predicate. Nothing noticed for twenty minutes, until a whole gate run.
#
# THE TWO GREPS BELOW ARE COPIED FROM step_spdx, INCLUDING ITS EXCLUSIONS, and
# they are the point: an arm that approximates the gate can disagree with it,
# and a commit guard that disagrees with the gate is worse than none -- it
# teaches the desk the tree is clean when it is not.
#
# /usr/bin/grep, not bare `grep`. THE PIN IS LATENT, NOT ACTIVE, and saying so
# is the point of the arm: MEASURED 2026-09-05, /usr/bin/grep is the ONLY grep
# on PATH, and a non-interactive bash resolves `grep` straight to it -- so this
# pin changes NOTHING against today's gate. It guards two real things anyway:
# a second grep arriving earlier on PATH (as /usr/local/bin/git 2.15 already
# does for git, an ENVIRONMENT-TRAPS line), and this desk hand-checking a header
# in the INTERACTIVE shell, where `grep` IS a function wrapping ugrep with
# --ignore-files and -I (payload SS10) and a script's result would not apply.
_check_spdx() {
    local f bad=0
    while IFS= read -r f; do
        case "$f" in
            contracts/*|Cargo.lock|LICENSE*|.gitignore) continue ;;
            *.rs|*.sh|*.py|*.toml|*.md|*.yaml|*.c|*.h|*.cpp|*.qml|*.wgsl) ;;
            *) continue ;;
        esac
        # A DELETION stages a path with no file on disk; it cannot fail a header
        # check and must not be read.
        [ -f "$f" ] || continue
        /usr/bin/grep -q 'SPDX-License-Identifier:' "$f" \
            || { echo "  spdx: missing license identifier: $f" >&2; bad=1; }
        /usr/bin/grep -q 'Copyright (c) 2026 the Appname contributors' "$f" \
            || { echo "  spdx: missing copyright line: $f" >&2; bad=1; }
    done < <(git diff --cached --name-only)
    [ "$bad" -eq 0 ] || _die "a staged file fails the GATE's own spdx predicate \
(step_spdx). Fix the header, do not weaken this arm -- it is the gate's text, not an approximation."
    return 0
}

_check_groundline() {
    # ARM 3: only fires when the integration log is actually being committed.
    _named=0
    for p in "$@"; do
        [ "$p" = "$INTLOG" ] && _named=1
    done
    [ "$_named" = "1" ] || return 0
    # Added lines only. A ground line already in the file does not count --
    # the requirement is that THIS entry carries one.
    # An entry AMENDED across commits presents no heading in the diff, so
    # this arm has nothing to anchor to. That is UNMEASURED, not PASS --
    # a check that cannot evaluate must SAY SO rather than fall through
    # quietly (payload section 10: a claim of absence must name its
    # population; G-518: granularity of check must match granularity of
    # edit). Silence here is exactly how a guard becomes decorative.
    _heads=$(git diff --cached -U0 -- "$INTLOG" | grep -c "$HEAD_PLAIN")
    if [ "${_heads:-0}" -lt 1 ]; then
        echo "oint-commit: arm3 UNMEASURED - $INTLOG is being changed but this diff adds no '## NNN.' heading, so there is no new entry to ground. Verify by hand if you meant to add one." >&2
        return 0
    fi
    _added=$(git diff --cached -U0 -- "$INTLOG" | grep '^+' | _ground_after_heading)
    if [ "${_added:-0}" -lt 1 ]; then
        _die "$INTLOG is being committed but its added lines carry no ground line (payload section 1). Paste p_ground's output into the entry."
    fi
    return 0
}

# ARM 4. THE LEDGER IS APPEND-ONLY, SO IT MUST NEVER SHRINK. ADDED
# 2026-09-04 AFTER I TRUNCATED IT TO ZERO AND COMMITTED THAT (entry 563).
#
# WHAT HAPPENED: `io.open(p,'w').write(EXPR)` evaluates `open` FIRST, so the
# file was emptied; then EXPR raised and never refilled it. 58,474 lines,
# 1179 entries, committed as `f23dc77b`. Recovered from git only because a
# commit nine minutes earlier held the bytes.
#
# WHY THIS IS A NEW ARM AND NOT A CHANGE TO ARM 3. Arm 3 said UNMEASURED and
# let it through, and that was CORRECT for what arm 3 measures: an entry
# amended across commits legitimately presents no `## NNN.` heading in its
# diff, so making "no heading" refuse would fire on the right answer and
# train its reader to stop looking (G-514). The heading is the wrong
# discriminator. APPEND-ONLY-NESS is the right one.
#
# THE PREDICATE, and why net-lines rather than a byte floor: a reword is
# +N/-N and must PASS; a marker added to a dated record (section 5c-2, this
# desk's own rule) is +N/-0 and must PASS; a truncation is +0/-58474 and must
# REFUSE. Net decrease separates all three with no threshold to tune.
#
# THIS ARM REFUSES. Arm 3 reported and I read past it -- section 6's own
# lesson, which I had quoted in the same session: "the verdict was on the
# screen and not in the control flow."
_check_ledger_shrink() {
    _named=0
    for p in "$@"; do
        [ "$p" = "$INTLOG" ] && _named=1
    done
    [ "$_named" = "1" ] || return 0
    git diff --cached --quiet -- "$INTLOG" 2>/dev/null && return 0
    _head_n=$(git show "HEAD:$INTLOG" 2>/dev/null | _entry_region_lines)
    _idx_n=$(git show ":$INTLOG"     2>/dev/null | _entry_region_lines)
    [ -n "$_head_n" ] || return 0
    [ -n "$_idx_n" ]  || return 0
    if [ "$_idx_n" -lt "$_head_n" ]; then
        _die "$INTLOG's ENTRY REGION would SHRINK from $_head_n to $_idx_n lines. Entries are APPEND-ONLY: a commit that removes entry text is a truncation, not an edit. (The unblock paste is EXCLUDED from this count and may be regenerated freely.) If a deletion is genuinely intended, commit it by hand and say why."
    fi
    return 0
}

# The ENTRY REGION: everything after the unblock paste's CLOSING fence.
#
# SCOPED 2026-09-04, one tick after arm 4 landed, because arm 4 refused a
# CORRECT and ROUTINE commit (entry 566): the paste lives INSIDE this file and
# G-542 requires REGENERATING it whenever the questions move, which removes
# rows. Measured on the very commit it blocked -- whole file +37/-179, while
# the ENTRY region GREW 58142 -> 58200.
#
# "Append-only" was true of the ENTRIES and false of the FILE; I wrote the
# guard over the file because that is the shape the incident handed me
# (G-527). A guard that refuses a routine correct action trains its reader to
# use the documented bypass as a habit, and the bypass then LOOKS like
# compliance -- worse than no guard, because it is deniable.
#
# FALLBACK IS WHOLE-FILE, deliberately: a ledger with no paste heading has no
# region to exclude, so the STRICTEST reading applies rather than the loosest.
_entry_region_lines() {
    awk '/^# .*UNBLOCK PASTE/ { seen=1 }
        seen && f<2 && /^```[ \t]*$/ { f++; next }
        { total++; if (f>=2) n++ }
        END { if (f>=2) print n+0; else print total+0 }'
}

_selftest() {
    fails=0
    t=$(mktemp -d) || { echo "selftest: VOID - no tmpdir"; return 3; }

    # ARM 2 must fire on the ledger, by bare and by nested path.
    for p in "$LEDGER" "x/$LEDGER"; do
        if ( _check_paths "$p" ) 2>/dev/null; then
            echo "  FAIL arm2: accepted $p"; fails=$((fails+1))
        fi
    done
    # ARM 2 must NOT fire on an innocent path -- a control that cannot
    # pass is as useless as one that cannot fire.
    if ( _check_paths "reports/INTEGRATION-log.md" ) 2>/dev/null; then :; else
        echo "  FAIL arm2-control: refused an innocent path"; fails=$((fails+1))
    fi
    # ARM 2 must not match a path that merely CONTAINS the name.
    if ( _check_paths "reports/WATCH-log.md.bak" ) 2>/dev/null; then :; else
        echo "  FAIL arm2-substring: refused WATCH-log.md.bak"; fails=$((fails+1))
    fi

    # ARM 3, positional. _g3 feeds diff-shaped lines through the real helper.
    _g3() { _ground_after_heading; }
    REAL='+`GROUND appname[tags=7 at_head=0] bogos[tags=2 at_head=1] v5.6-freeze=tag/33b8d562`'
    # must PASS: a ground line directly under a new entry heading
    o=$(printf '+## 351. TITLE\n%s\n' "$REAL" | _g3)
    if [ "$o" != "1" ]; then
        echo "  FAIL arm3-one: ground line under a heading not counted (got '$o')"; fails=$((fails+1))
    fi
    # must FAIL: heading with no ground line under it
    z=$(printf '+## 351. TITLE\n+some prose\n' | _g3)
    if [ "$z" != "0" ]; then
        echo "  FAIL arm3-zero: expected 0, got '$z'"; fails=$((fails+1))
    fi
    # must FAIL: a real ground line far from any heading
    f=$(printf '+prose\n%s\n' "$REAL" | _g3)
    if [ "$f" != "0" ]; then
        echo "  FAIL arm3-floating: a ground line not under a heading counted"; fails=$((fails+1))
    fi
    # MUST-FAIL CONTROLS -- THE TWO SENTENCES THAT DEFEATED v1 AND v2,
    # VERBATIM. If either arm ever goes green the guard has regressed to
    # matching the ledger talking about itself.
    D1='+348 and 349 carry NO `GROUND appname[` line. Six consecutive. Only 350 does.**'
    D2='+`GROUND appname[tags=`, `bogos[tags=` and `v5.6-freeze=` **on one added line.** Prose quotes a'
    for d in "$D1" "$D2"; do
        r=$(printf '+## 351. TITLE\n+intervening prose\n%s\n' "$d" | _g3)
        if [ "$r" != "0" ]; then
            echo "  FAIL arm3-selfdiscussion: prose about the guard satisfied it"; fails=$((fails+1))
        fi
    done
    # ARM 2 RESOLUTION — the directory-pathspec sweep (O-WATCH #352), in a
    # throwaway git repo, CONTROL FIRST. Two arms and they must disagree:
    # a DIRTY ledger under a directory pathspec must REFUSE, and a CLEAN one
    # must PASS. Without the second, the guard would refuse every `reports/`
    # commit forever and train its reader to stop reading it (G-514).
    r="$t/repo"
    if mkdir -p "$r/reports" 2>/dev/null && ( cd "$r" && git init -q . \
         && git config user.email t@t && git config user.name t ) 2>/dev/null; then
        printf '<!-- SPDX-License-Identifier: GPL-3.0-only -->\n<!-- Copyright (c) 2026 the Appname contributors -->\ne\n' > "$r/reports/INTEGRATION-log.md"
        printf 'l\n' > "$r/$LEDGER"
        ( cd "$r" && git add -A && git commit -qm base ) 2>/dev/null
        printf 'peer\n' >> "$r/$LEDGER"
        printf 'mine\n' >> "$r/reports/INTEGRATION-log.md"
        # ABSOLUTE, because these arms `cd` into the fixture before invoking
        # this script: `$0` is relative to the ORIGINAL cwd and resolves to
        # nothing from inside `$r`. A path is not an identity without its base.
        _self=$( cd "$(dirname "$0")" && pwd )/$(basename "$0")
        # DIRTY ledger + directory pathspec -> must refuse, and must not commit.
        ( cd "$r" && bash "$_self" "m" reports >/dev/null 2>&1 )
        if [ "$?" -eq 1 ]; then :; else
            echo "  FAIL arm2-dir-dirty: a directory pathspec did not refuse"; fails=$((fails+1))
        fi
        n=$( cd "$r" && git rev-list --count HEAD 2>/dev/null )
        if [ "$n" != "1" ]; then
            echo "  FAIL arm2-dir-leak: the refusal still produced a commit"; fails=$((fails+1))
        fi
        # CLEAN ledger + directory pathspec -> must PASS and stage only mine.
        ( cd "$r" && git checkout -q -- "$LEDGER" \
          && bash "$_self" "m" reports >/dev/null 2>&1 )
        if [ "$?" -eq 0 ]; then :; else
            echo "  FAIL arm2-dir-clean: refused a directory pathspec with a clean ledger"; fails=$((fails+1))
        fi
        # The sweep check must read the commit the CLEAN run made, not the
        # base commit -- which legitimately contains the ledger. Assert a
        # second commit exists first, or this arm reports the fixture's own
        # setup as a sweep.
        n2=$( cd "$r" && git rev-list --count HEAD 2>/dev/null )
        if [ "$n2" != "2" ]; then
            echo "  FAIL arm2-dir-nocommit: the clean run produced no commit"; fails=$((fails+1))
        else
            got=$( cd "$r" && git show --name-only --format= HEAD 2>/dev/null | tr '\n' ' ' )
            case "$got" in
                *WATCH-log*) echo "  FAIL arm2-dir-swept: the ledger reached the commit"; fails=$((fails+1)) ;;
            esac
        fi
    fi

    # THE INDEX ARMS (O-WATCH #354). The guard's earlier checks all asked
    # "what would MY pathspecs stage?"; `git commit` takes the WHOLE INDEX.
    # KG1/KG2 are their known-good controls and MUST NOT FIRE -- a guard that
    # refuses the correct answer trains its reader to stop reading it (G-514).
    r2="$t/repo2"
    if mkdir -p "$r2/reports" 2>/dev/null && ( cd "$r2" && git init -q . \
         && git config user.email t@t && git config user.name t ) 2>/dev/null; then
        printf '<!-- SPDX-License-Identifier: GPL-3.0-only -->\n<!-- Copyright (c) 2026 the Appname contributors -->\n## 1. E\n`GROUND appname[tags=1]`\n' > "$r2/reports/INTEGRATION-log.md"
        printf '<!-- SPDX-License-Identifier: GPL-3.0-only -->\n<!-- Copyright (c) 2026 the Appname contributors -->\no\n' > "$r2/reports/OTHER.md"
        printf 'l\n' > "$r2/$LEDGER"
        ( cd "$r2" && git add -A && git commit -qm base ) 2>/dev/null
        _self2=$( cd "$(dirname "$0")" && pwd )/$(basename "$0")

        # KG1 must PASS: an innocent path, clean ledger, empty index.
        printf 'x\n' >> "$r2/reports/OTHER.md"
        ( cd "$r2" && bash "$_self2" "kg1" reports/OTHER.md >/dev/null 2>&1 )
        [ "$?" -eq 0 ] || { echo "  FAIL idx-KG1: refused the correct answer"; fails=$((fails+1)); }

        # HOLE (their PROBE A) must FIRE: ledger pre-staged by anything else.
        printf 'peer\n' >> "$r2/$LEDGER"
        printf 'y\n' >> "$r2/reports/OTHER.md"
        ( cd "$r2" && git add "$LEDGER" && bash "$_self2" "hole" reports/OTHER.md >/dev/null 2>&1 )
        [ "$?" -eq 1 ] || { echo "  FAIL idx-prestaged: a pre-staged ledger reached a commit"; fails=$((fails+1)); }
        # COUNT commits, do not inspect HEAD. HEAD here is KG1's own legitimate
        # commit, so a name-match against it fires on the arm above succeeding.
        # This is the SECOND time in one session that a harness of mine read a
        # prior commit as the one under test; the discriminator is always
        # "did a NEW commit appear", never "what is in the newest one".
        [ "$( cd "$r2" && git rev-list --count HEAD 2>/dev/null )" -eq 2 ] \
          || { echo "  FAIL idx-prestaged-landed: the refusal still produced a commit"; fails=$((fails+1)); }
        ( cd "$r2" && git reset -q HEAD -- . && git checkout -q -- "$LEDGER" ) 2>/dev/null

        # ---- ARM 4 (append-only): the truncation that produced entry 563 ----
        # ORDER MATTERS AND IT IS NOT COSMETIC. The must-PASS arms run FIRST so
        # the fixture GROWS before the must-FIRE arms try to shrink it. Run the
        # other way round, a truncation that slips through (arm 4 disabled)
        # leaves an EMPTY fixture, and the partial-shrink arm then has nothing
        # to delete and cannot fire -- a must-fire arm silently satisfied by
        # the damage the previous arm did. Measured: in that order the mutation
        # control reported ONE failure where it should report TWO.
        ( cd "$r2" && git reset -q HEAD -- . && git checkout -q -- . ) 2>/dev/null

        # MUST PASS: an ordinary grounded append (10b-L -- a guard that can
        # never go green is indistinguishable from one that never fires).
        printf '## 2. APPEND\n`GROUND appname[tags=1]`\n' >> "$r2/reports/INTEGRATION-log.md"
        ( cd "$r2" && bash "$_self2" "append" reports/INTEGRATION-log.md >/dev/null 2>&1 ) \
            || { echo "  FAIL arm4-append-mustpass: refused an ordinary append"; fails=$((fails+1)); }
        ( cd "$r2" && git reset -q HEAD -- . && git checkout -q -- . ) 2>/dev/null

        # MUST PASS: an equal-size reword (+N/-N) on a NON-heading line. This
        # is why the predicate is net-lines and not "no deletions allowed":
        # section 5c-2 permits correcting a LIVE line in place. Rewording a
        # HEADING is arm 3's subject, not arm 4's, and would confound this.
        sed 's/tags=1/tags=2/' "$r2/reports/INTEGRATION-log.md" > "$r2/.x" \
            && mv "$r2/.x" "$r2/reports/INTEGRATION-log.md"
        ( cd "$r2" && bash "$_self2" "reword" reports/INTEGRATION-log.md >/dev/null 2>&1 ) \
            || { echo "  FAIL arm4-reword-mustpass: refused a +N/-N in-place correction"; fails=$((fails+1)); }
        ( cd "$r2" && git reset -q HEAD -- . && git checkout -q -- . ) 2>/dev/null

        # MUST FIRE on a PARTIAL shrink, not only on zero -- a guard tuned to
        # the incident that produced it catches only that incident. Deleting
        # trailing lines adds NO heading, so arm 3 goes UNMEASURED and arm 4
        # is the only thing that can refuse: this arm ISOLATES arm 4.
        _n4=$(wc -l < "$r2/reports/INTEGRATION-log.md")
        awk -v k=$((_n4 - 2)) 'NR<=k' "$r2/reports/INTEGRATION-log.md" > "$r2/.x" \
            && mv "$r2/.x" "$r2/reports/INTEGRATION-log.md"
        ( cd "$r2" && bash "$_self2" "partial" reports/INTEGRATION-log.md >/dev/null 2>&1 ) \
            && { echo "  FAIL arm4-partial: a net line LOSS was committed"; fails=$((fails+1)); }
        ( cd "$r2" && git reset -q HEAD -- . && git checkout -q -- . ) 2>/dev/null

        # MUST FIRE: the exact catastrophe of entry 563 -- emptied and committed.
        : > "$r2/reports/INTEGRATION-log.md"
        ( cd "$r2" && bash "$_self2" "truncate" reports/INTEGRATION-log.md >/dev/null 2>&1 ) \
            && { echo "  FAIL arm4-truncate: a ledger emptied to zero was COMMITTED"; fails=$((fails+1)); }
        ( cd "$r2" && git reset -q HEAD -- . && git checkout -q -- . ) 2>/dev/null

        # ---- ARM 4 REGION SPLIT: the false positive of entry 566 ----
        # A fixture WITH an unblock paste, so the region logic is exercised
        # rather than the whole-file fallback the arms above use.
        ( cd "$r2" && git reset -q HEAD -- . && git checkout -q -- . ) 2>/dev/null
        {
            printf '<!-- SPDX-License-Identifier: GPL-3.0-only -->\n'
            printf '<!-- Copyright (c) 2026 the Appname contributors -->\n\n'
            printf '# UNBLOCK PASTE - O-INT #9\n\n'
            printf '```\n'
            printf 'Q-D1 alpha\nQ-D2 beta\nQ-D3 gamma\nQ-D4 delta\n'
            printf '```\n\n'
            printf '## 1. E\n`GROUND appname[tags=1]`\n\nbody line\n'
        } > "$r2/reports/INTEGRATION-log.md"
        ( cd "$r2" && git add reports/INTEGRATION-log.md \
          && git commit -qm pastebase ) 2>/dev/null

        # MUST PASS: remove rows from INSIDE the paste fence. This is what
        # G-542 regeneration does every time the questions move, and it is
        # exactly the commit arm 4 wrongly refused in entry 566.
        ( cd "$r2" && sed '/^Q-D2 beta$/d;/^Q-D3 gamma$/d' reports/INTEGRATION-log.md > .x \
          && mv .x reports/INTEGRATION-log.md )
        ( cd "$r2" && bash "$_self2" "pasteregen" reports/INTEGRATION-log.md >/dev/null 2>&1 ) \
            || { echo "  FAIL arm4-pasteregen-mustpass: refused a paste-only regeneration"; fails=$((fails+1)); }
        ( cd "$r2" && git reset -q HEAD -- . && git checkout -q -- . ) 2>/dev/null

        # MUST FIRE: remove a line from AFTER the fence. Same file, same size
        # change, OTHER side of the boundary -- this is what proves the split
        # is a SPLIT and not a way of letting everything through.
        ( cd "$r2" && sed '/^body line$/d' reports/INTEGRATION-log.md > .x \
          && mv .x reports/INTEGRATION-log.md )
        ( cd "$r2" && bash "$_self2" "entryloss" reports/INTEGRATION-log.md >/dev/null 2>&1 ) \
            && { echo "  FAIL arm4-entryloss: entry text was removed and COMMITTED"; fails=$((fails+1)); }
        ( cd "$r2" && git reset -q HEAD -- . && git checkout -q -- . ) 2>/dev/null

        # PROBE B must leave NO RESIDUE: a refusal returns the index to empty.
        printf '## 2. NO GROUND\nprose\n' >> "$r2/reports/INTEGRATION-log.md"
        ( cd "$r2" && bash "$_self2" "ungrounded" reports/INTEGRATION-log.md >/dev/null 2>&1 )
        left=$( cd "$r2" && git diff --cached --name-only 2>/dev/null | tr -d '\n' )
        [ -z "$left" ] || { echo "  FAIL idx-residue: a refusal left [$left] staged"; fails=$((fails+1)); }
    fi

    # ---- ARM 5's OWN ARMS. A guard that has never fired is not a guard, and
    # this one exists because entry 625's proxy check let a header-less file
    # reach main and redden the release gate.
    r3="$t/repo3"
    if mkdir -p "$r3/reports" 2>/dev/null && ( cd "$r3" && git init -q . \
         && git config user.email t@t && git config user.name t ) 2>/dev/null; then
        _hdr='<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (c) 2026 the Appname contributors -->'
        printf '%s\nseed\n' "$_hdr" > "$r3/reports/SEED.md"
        printf 'l\n' > "$r3/$LEDGER"
        ( cd "$r3" && git add -A && git commit -qm base ) 2>/dev/null
        _self3=$( cd "$(dirname "$0")" && pwd )/$(basename "$0")

        # KG must PASS: a .md carrying BOTH lines is committed normally.
        printf 'more\n' >> "$r3/reports/SEED.md"
        ( cd "$r3" && bash "$_self3" "spdx-kg" reports/SEED.md >/dev/null 2>&1 )
        [ "$?" -eq 0 ] || { echo "  FAIL arm5-KG: refused a correctly headered file"; fails=$((fails+1)); }

        # MUST FIRE: the REUSE-canonical copyright form, which step_spdx does
        # NOT accept. This is byte-for-byte the shape that reddened main.
        printf '<!-- SPDX-FileCopyrightText: 2026 the Appname contributors -->\n<!-- SPDX-License-Identifier: GPL-3.0-only -->\nbody\n' \
            > "$r3/reports/REUSEFORM.md"
        ( cd "$r3" && bash "$_self3" "spdx-fire" reports/REUSEFORM.md >/dev/null 2>&1 )
        [ "$?" -eq 1 ] || { echo "  FAIL arm5-mustfire: the gate-red header form reached a commit"; fails=$((fails+1)); }

        # MUST NOT FIRE on an extension step_spdx does not scan: a .json with
        # no header at all is none of this arm's business.
        printf '{}\n' > "$r3/reports/data.json"
        ( cd "$r3" && bash "$_self3" "spdx-scope" reports/data.json >/dev/null 2>&1 )
        [ "$?" -eq 0 ] || { echo "  FAIL arm5-scope: refused a file the gate never scans"; fails=$((fails+1)); }
    fi

    rm -rf "$t"
    echo "oint-commit selftest: 28 arms, $fails failed"
    [ "$fails" -eq 0 ]
}

case "${1:-}" in
    --selftest) _selftest; exit $? ;;
    "") _void "usage: oint-commit.sh <message> <path> [path...]  |  --selftest" ;;
esac

MSG="$1"; shift
[ "$#" -ge 1 ] || _void "name at least one path -- this script never stages with -A"

# ENTRY: THE INDEX MUST BE EMPTY. Added 2026-08-29 on O-WATCH #354, both
# probes reproduced here first.
#
# Every check below asks "what would MY pathspecs stage?" -- but `git commit`
# takes THE WHOLE INDEX, and nothing was looking at it. Two live holes:
#   PROBE A  the ledger staged by anything else, then an innocent path named
#            -> committed, WITH the ledger. The script even PRINTED the sweep
#            in its own "staging ->" list and committed it anyway.
#   PROBE B  arm 3 refuses an ungrounded entry and LEAVES THE INDEX STAGED;
#            the next run, naming only an innocent path, commits the very
#            entry that was refused. THE GUARD DID THIS TO ITSELF.
# Probe B is O-WATCH's G-796: a refusal that leaves its side effects in place
# is a POSTPONEMENT wearing a refusal's exit code. Exit 1 was true, and the
# state it left made it false one invocation later.
if ! git diff --cached --quiet 2>/dev/null; then
    echo "oint-commit: REFUSED - the index is not empty at entry:" >&2
    git diff --cached --name-only | sed 's/^/    /' >&2
    echo "    This script owns the index. Commit or unstage those first --" >&2
    echo "    a leftover index is how a refused entry lands on the next run." >&2
    exit 1
fi

_check_paths "$@" || exit 1
git add -- "$@" || _void "git add failed"
_STAGED_PATHS=("$@")

# POST-ADD, ON THE STAGED SET ITSELF -- the property that actually matters is
# what the COMMIT will contain, not what my pathspecs matched.
if git diff --cached --name-only | /usr/bin/grep -qx "$LEDGER"; then
    _die "$LEDGER is in the index and would ride along in this commit. It goes through oint-fence.sh alone."
fi

# A REFUSAL MUST LEAVE NO RESIDUE (G-796). Unstage exactly what this
# invocation staged, so a refusal returns the index to the empty state the
# entry check requires. `reset -q HEAD -- <paths>` is INDEX-ONLY and works on
# the 2.15 bare git (§9); `git restore` needs 2.23 and is not available.
_check_ledger_shrink "$@" || exit 1
_check_groundline "$@" || exit 1
_check_spdx || exit 1

if git diff --cached --quiet; then
    echo "oint-commit: nothing staged (paths clean)"; exit 2
fi
echo "oint-commit: staging ->"
git diff --cached --name-only | sed 's/^/    /'
git commit -q -m "$MSG" || _void "git commit failed"
git log --oneline -1
# REUSE-IgnoreEnd
