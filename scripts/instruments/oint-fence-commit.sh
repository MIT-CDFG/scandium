#!/bin/bash
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
# oint-fence-commit.sh — OWNER: O-INT (integration desk)
#
# THE FENCE'S COMMIT HALF, GATED. oint-fence.sh is READ-ONLY by contract and
# must stay so; this script owns the index and the commit, and it exists
# because of a real incident rather than for symmetry.
#
# WHAT HAPPENED (entry 572, 2026-09-04). I fenced O-WATCH review #430, then
# `git add`ed, then committed a message claiming "blob 808c65d8, +1390/-1".
# The committed blob was e6e824de and the real diff was +1411/-1: the peer
# kept writing in the window BETWEEN the fence run and my `git add` -- 21
# lines -- so the bytes the fence validated were NOT the bytes I committed,
# and the commit message asserted otherwise.
#
# THE FENCE'S QUIESCENCE BRACKET COVERS ITS OWN EXECUTION, NOT THE GAP TO THE
# COMMIT. Section 6 already prescribes what closes that gap -- confirm the blob
# FROM THE INDEX after `git add` and check it against the declared hash -- and
# I performed it: I PRINTED both values and committed anyway. That is section
# 6's own recorded failure ("I piped a fence to tail, it printed DO NOT COMMIT,
# and I committed: the verdict was on the screen and not in the control flow"),
# repeated with a stronger verdict. Discipline had a full session to hold and
# did not; section 4's remedy is the only one with a record of working.
#
# IT DOES NOT PARSE THE FENCE'S OUTPUT. Section 6: never redirect or pipe the
# fence -- the rc must reach the control flow. So the fence runs to the
# terminal where a person can read it, and the blob is derived INDEPENDENTLY
# here, twice, which is a stronger check than reading the fence's own number
# back to it (a value parsed from an instrument cannot disagree with it).
#
# THE GATE: W1 == W2 == INDEX, where W1 is the worktree blob BEFORE the fence
# and W2 AFTER it. W1!=W2 means the peer wrote during the check; W2!=INDEX
# means they wrote between the check and the staging. Both refuse.
set -uo pipefail

usage() {
  cat <<'USAGE'
oint-fence-commit.sh <repo> <ledger-path> <review_number> [review_number...]
oint-fence-commit.sh <repo> <ledger-path> --sections "<subject prefix>"
  Runs oint-fence.sh, then stages and commits the ledger ONLY if the bytes
  never moved: worktree-before == worktree-after == index.
  Exit: 0 committed · 1 REFUSED · 2 nothing to commit · 3 VOID
USAGE
}
case "${1:-}" in -h|--help|"") usage; [ -z "${1:-}" ] && exit 3 || exit 0 ;; esac
if [ "${1:-}" = "--selftest" ]; then
    _self=$( cd "$(dirname "$0")" && pwd )/$(basename "$0")
    fails=0
    t=$(mktemp -d) || { echo "selftest: VOID - no tmpdir"; exit 3; }
    r="$t/repo"; mkdir -p "$r/reports" "$r/scripts/instruments"
    # THE FENCE MUST BE BESIDE ME. A copy of this script placed elsewhere (the
    # natural way to mutation-test it) has no oint-fence.sh next to it, and the
    # arms then return rc=127 -- a "command not found" wearing a gate's clothes.
    # VOID with an instruction beats four confusing failures.
    _src=$( cd "$(dirname "$0")" && pwd )
    if [ ! -f "$_src/oint-fence.sh" ]; then
        echo "selftest: VOID - no oint-fence.sh beside $0. Put the copy IN scripts/instruments/ (mutation-testing works there); every arm would otherwise measure rc=127." >&2
        rm -rf "$t"; exit 3
    fi
    cp "$_src/oint-fence.sh" "$r/scripts/instruments/" 2>/dev/null
    cp "$_self" "$r/scripts/instruments/" 2>/dev/null
    # SELF-INVOKE BY THE COPIED NAME, NOT A HARD-CODED ONE. O-WATCH #433 tried
    # to re-derive my gate coverage by copying this file to mutA.sh/mutB.sh and
    # mutating the copies; the selftest copied `$_self` under the new name and
    # then invoked the OLD one, so four arms returned rc=127 and measured "can
    # a renamed script find itself" rather than the gates. They read the 127
    # and stopped; had they not, they would have reported both mutations
    # failing arms as proof both gates are covered -- re-asserting a wrong
    # countersign on fabricated evidence.
    _me=$(basename "$_self")
    [ -f "$r/scripts/instruments/$_me" ] || {
        echo "selftest: VOID - could not copy myself as $_me; every arm below would measure rc=127, not a gate" >&2
        rm -rf "$t"; exit 3; }
    # The fixture must satisfy the FENCE's controls, not just this script's.
    # CONTROL-2 needs a crates/ commit to control against; without one the
    # fence returns VOID(3) and every arm below measures the fixture's
    # thinness instead of the property it names. Found by arm 2 failing.
    mkdir -p "$r/crates/x/src"
    ( cd "$r" && git init -q . && git config user.email t@t && git config user.name t \
      && printf '<!-- SPDX-License-Identifier: GPL-3.0-only -->\n<!-- Copyright (c) 2026 the Appname contributors -->\n# O-WATCH log\n' > reports/WATCH-log.md \
      && printf 'fn main() {}\n' > crates/x/src/main.rs \
      && git add -A && git commit -qm base \
      && printf 'fn main() { }\n' > crates/x/src/main.rs \
      && git add crates && git commit -qm 'crates: a commit for CONTROL-2' ) >/dev/null 2>&1

    # ARM 1 must be rc=2: a CLEAN ledger has nothing to commit, and refusing
    # here instead would cry wolf on every quiet tick (G-514).
    ( cd "$r" && bash "scripts/instruments/$_me" "$r" reports/WATCH-log.md 900 ) >/dev/null 2>&1
    [ "$?" -eq 2 ] || { echo "  FAIL arm1-clean-is-rc2"; fails=$((fails+1)); }

    # ARM 2 must PASS: an ordinary quiet review commits, and the blob in the
    # message is the one in the tree.
    ( cd "$r" && printf '\n# Review #900 - x\nbody\n' >> reports/WATCH-log.md )
    ( cd "$r" && bash "scripts/instruments/$_me" "$r" reports/WATCH-log.md 900 ) >/dev/null 2>&1
    _rc2=$?
    _msgblob=$( cd "$r" && git log -1 --format=%s 2>/dev/null | /usr/bin/grep -o 'blob [a-f0-9]*' | cut -d" " -f2 )
    _treeblob=$( cd "$r" && git rev-parse HEAD:reports/WATCH-log.md 2>/dev/null | cut -c1-8 )
    [ "$_rc2" -eq 0 ] || { echo "  FAIL arm2-quiet-mustpass: rc=$_rc2"; fails=$((fails+1)); }
    [ -n "$_msgblob" ] && [ "$_msgblob" = "$_treeblob" ] \
      || { echo "  FAIL arm2-blob-truthful: message says [$_msgblob], tree has [$_treeblob]"; fails=$((fails+1)); }

    # ARM 3 must FIRE on the OUTCOME: a peer writing during the fence is
    # refused. It does NOT isolate gate A -- the fence's own bracket catches
    # this first, measured. Named for what it proves. The incident
    # the script exists for and the one branch a reader cannot test by eye --
    # so it is injected as a real race, not simulated.
    ( cd "$r" && printf '\n# Review #901 - y\nbody\n' >> reports/WATCH-log.md )
    ( cd "$r" && ( sleep 0.4; printf 'peer wrote mid-fence\n' >> reports/WATCH-log.md ) & \
      bash "scripts/instruments/$_me" "$r" reports/WATCH-log.md 901 >/dev/null 2>&1
      echo $? > "$t/rc3"; wait ) >/dev/null 2>&1
    _rc3=$(cat "$t/rc3" 2>/dev/null)
    [ "$_rc3" = "1" ] || { echo "  FAIL arm3-midfence-write-is-refused: rc=$_rc3 (expected 1)"; fails=$((fails+1)); }
    _left=$( cd "$r" && git diff --cached --name-only 2>/dev/null | tr -d "\n" )
    [ -z "$_left" ] || { echo "  FAIL arm3-race-residue: [$_left] left staged"; fails=$((fails+1)); }

    # ARM 4 must FIRE on the ACTUAL entry-572 window: a write landing AFTER the
    # fence completes and BEFORE the staging. Arm 3 does NOT prove this -- with
    # the W1/W2 gate disabled arm 3 still passes, because the fence's own
    # bracket catches a mid-fence write. This arm isolates the gate that is new.
    ( cd "$r" && git reset -q HEAD -- . && git checkout -q -- . ) 2>/dev/null
    ( cd "$r" && printf '\n# Review #902 - z\nbody\n' >> reports/WATCH-log.md )
    # The writer WAITS FOR THE SIGNAL, so the write lands inside the delay
    # window on any machine. A fixed sleep made this arm's verdict depend on
    # the fence being slower or faster than 2s.
    rm -f "$t/sig"
    ( cd "$r" && ( n=0; while [ ! -f "$t/sig" ] && [ $n -lt 600 ]; do sleep 0.05; n=$((n+1)); done
        printf 'peer wrote post-fence\n' >> reports/WATCH-log.md ) & \
      OINT_FC_TEST_SIGNAL="$t/sig" OINT_FC_TEST_DELAY=4 bash "scripts/instruments/$_me" "$r" reports/WATCH-log.md 902 >/dev/null 2>&1
      echo $? > "$t/rc4"; wait ) >/dev/null 2>&1
    _rc4=$(cat "$t/rc4" 2>/dev/null)
    [ "$_rc4" = "1" ] || { echo "  FAIL arm4-postfence-mustfire: rc=$_rc4 (expected 1)"; fails=$((fails+1)); }
    _left4=$( cd "$r" && git diff --cached --name-only 2>/dev/null | tr -d "\n" )
    [ -z "$_left4" ] || { echo "  FAIL arm4-postfence-residue: [$_left4] left staged"; fails=$((fails+1)); }

    rm -rf "$t"
    # ARM 8 — ARM 6's OWN MUST-FIRE ARM. Entry 627 filed this door as the one
    # ARM 5 does not watch; a guard nobody can show fires is not a guard.
    # The header used here is the REUSE-canonical `SPDX-FileCopyrightText:`
    # form -- byte-for-byte the shape that reddened main at 0082aabe -- and
    # NOT an invented bad header, because a guard tested against a defect
    # nobody actually commits has never been tested.
    # The crates/ commit is NOT decoration: without it the fence's CONTROL-2 is
    # DEAD, the fence returns 3 (VOID), and this arm sees rc=1 from a refusal
    # that never reaches ARM 6 -- a FALSE PASS. Measured 2026-09-05: the first
    # version of this arm did exactly that, and the mutation control caught it.
    r8="$t/repo8"
    if mkdir -p "$r8/reports" "$r8/scripts/instruments" "$r8/crates/x/src" 2>/dev/null \
         && ( cd "$r8" && git init -q . && git config user.email t@t \
              && git config user.name t ) 2>/dev/null; then
        cp "$_src/oint-fence.sh" "$r8/scripts/instruments/" 2>/dev/null
        cp "$_self" "$r8/scripts/instruments/" 2>/dev/null
        printf '<!-- SPDX-FileCopyrightText: 2026 the Appname contributors -->\n<!-- SPDX-License-Identifier: GPL-3.0-only -->\n# O-WATCH log\n' \
            > "$r8/reports/WATCH-log.md"
        printf 'fn main() {}\n' > "$r8/crates/x/src/main.rs"
        ( cd "$r8" && git add -A && git commit -qm base \
          && printf 'fn main() { }\n' > crates/x/src/main.rs \
          && git add crates && git commit -qm 'crates: a commit for CONTROL-2' ) >/dev/null 2>&1
        _h8before=$( cd "$r8" && git rev-parse HEAD )
        ( cd "$r8" && printf '\n# Review #908 - w\nbody\n' >> reports/WATCH-log.md )
        ( cd "$r8" && bash "scripts/instruments/$_me" "$r8" reports/WATCH-log.md 908 ) >/dev/null 2>&1
        _rc8=$?
        _h8after=$( cd "$r8" && git rev-parse HEAD )
        [ "$_rc8" -eq 1 ] || { echo "  FAIL arm8-spdx-mustfire: the gate-red header form fenced through, rc=$_rc8"; fails=$((fails+1)); }
        [ "$_h8before" = "$_h8after" ] || { echo "  FAIL arm8-spdx-nocommit: HEAD moved on a refusal"; fails=$((fails+1)); }
    fi

    # ---- ARMS 9 & 10: THE SCOPE CHECK'S OWN ARMS, BOTH DIRECTIONS.
    # The repair of 2026-09-06 narrowed the scope check from the DIRTY set to
    # the STAGED set. A narrowing with no arm proving it still REFUSES is just
    # a loosening, so both directions are pinned here and neither is optional.
    r9="$t/repo9"; mkdir -p "$r9/reports" "$r9/scripts/instruments" "$r9/crates/x/src"
    if ( cd "$r9" && git init -q . && git config user.email t@t && git config user.name t ) 2>/dev/null; then
        cp "$_src/oint-fence.sh" "$r9/scripts/instruments/" 2>/dev/null
        cp "$_self" "$r9/scripts/instruments/" 2>/dev/null
        printf '<!-- SPDX-License-Identifier: GPL-3.0-only -->\n<!-- Copyright (c) 2026 the Appname contributors -->\n# O-WATCH log\n' \
            > "$r9/reports/WATCH-log.md"
        printf 'fn main() {}\n' > "$r9/crates/x/src/main.rs"
        ( cd "$r9" && git add -A && git commit -qm base \
          && printf 'fn main() { }\n' > crates/x/src/main.rs \
          && git add crates && git commit -qm 'crates: a commit for CONTROL-2' ) >/dev/null 2>&1

        # ARM 9 MUST PASS: a foreign file DIRTY IN THE WORKTREE but unstaged.
        # This is the exact fifteen-tick block -- a peer's uncommitted log beside
        # a perfectly in-scope review. The path-scoped commit cannot carry it.
        ( cd "$r9" && printf 'fn main() {  }\n' > crates/x/src/main.rs \
          && printf '\n# Review #909 - w\nbody\n' >> reports/WATCH-log.md )
        ( cd "$r9" && bash "scripts/instruments/$_me" "$r9" reports/WATCH-log.md 909 ) >/dev/null 2>&1
        _rc9=$?
        [ "$_rc9" -eq 0 ] || { echo "  FAIL arm9-dirty-foreign-must-pass: refused an in-scope commit over a merely-dirty foreign file, rc=$_rc9"; fails=$((fails+1)); }

        # ARM 10 MUST FIRE: the same foreign path, but STAGED. `git commit` takes
        # the whole index, so this one really would ride along.
        _h10=$( cd "$r9" && git rev-parse HEAD )
        ( cd "$r9" && printf 'fn main() {   }\n' > crates/x/src/main.rs \
          && git add crates/x/src/main.rs \
          && printf '\n# Review #910 - w\nbody\n' >> reports/WATCH-log.md )
        ( cd "$r9" && bash "scripts/instruments/$_me" "$r9" reports/WATCH-log.md 910 ) >/dev/null 2>&1
        _rc10=$?
        [ "$_rc10" -eq 1 ] || { echo "  FAIL arm10-staged-foreign-must-fire: a STAGED foreign path fenced through, rc=$_rc10"; fails=$((fails+1)); }
        [ "$_h10" = "$( cd "$r9" && git rev-parse HEAD )" ] || { echo "  FAIL arm10-staged-foreign-nocommit: HEAD moved on a refusal"; fails=$((fails+1)); }
    fi

    echo "oint-fence-commit selftest: 10 arms, $fails failed"
    [ "$fails" -eq 0 ]; exit $?
fi

REPO="$1"; LEDGER="${2:?$(usage)}"
shift 2
# ⛔ WHY --sections EXISTS (entry 844, 2026-09-11). This tool's contract is
# "commit ONLY if the fence passed". RELEASE-RIGHTS-1.0.md has no
# `# Review #N` headings, so the review-number check below REFUSED it and I
# hand-assembled fence-then-commit as TWO commands, five times in one night.
# The fifth time the fence printed "FENCE FAILED - DO NOT COMMIT" and the
# next command committed anyway (16cb543e3). The content happened to be
# benign; the gate was simply absent. Discipline is what failed, so the fix
# is apparatus: give the review-less ledger the SAME gated path.
# The subject PREFIX is typed by the desk (judgment); the blob, counts and
# heading count stay DERIVED (the parts that can lie).
MODE=review; SUBJ=""
if [ "${1:-}" = "--sections" ]; then
    MODE=sections; shift
    SUBJ="${1:-}"
    [ -n "$SUBJ" ] || { echo "oint-fence-commit: VOID - --sections needs a subject prefix" >&2; exit 3; }
    shift
    [ "$#" -eq 0 ] || { echo "oint-fence-commit: VOID - --sections takes exactly one subject prefix" >&2; exit 3; }
    WANT=""
else
    [ "$#" -ge 1 ] || { echo "oint-fence-commit: VOID - name at least one review number" >&2; exit 3; }
    WANT="$*"
fi

cd "$REPO" || { echo "oint-fence-commit: VOID - repo unreadable" >&2; exit 3; }
_here=$( cd "$(dirname "$0")" && pwd )

git diff --quiet -- "$LEDGER" && {
  echo "oint-fence-commit: nothing to commit ($LEDGER is clean)" >&2; exit 2; }

W1=$(git hash-object "$LEDGER") || exit 3

# The fence runs to the TERMINAL. Not piped, not redirected, rc gated.
bash "$_here/oint-fence.sh" "$REPO" "$LEDGER" $WANT
FENCE_RC=$?
if [ "$FENCE_RC" -ne 0 ]; then
    echo "oint-fence-commit: REFUSED - fence returned $FENCE_RC; nothing staged" >&2
    exit 1
fi

# GATE A. ⛔ NOT INDEPENDENT COVERAGE, AND NO ARM ISOLATES IT — stated here
# because O-WATCH #432 countersigned this file as carrying "a must-fire arm
# for EACH window" and that is true of gate B only. MEASURED, twice: with
# this comparison disabled the selftest still reports 7 arms 0 FAILED,
# because oint-fence.sh's OWN quiescence bracket refuses a mid-fence write
# first and this script gates on FENCE_RC. The mid-fence window IS protected;
# it is not protected by THIS line.
#
# KEPT, not deleted, and the reason is narrow: the fence's bracket is the
# thing actually covering that window, and if it ever changes this catches
# the same case at a second point. But it is CORRELATED defence -- both
# compute `git hash-object` on the same file across overlapping instants --
# so it must not be counted as a second independent arm (§6: correlated
# failure conditions read as redundancy and are not).
W2=$(git hash-object "$LEDGER") || exit 3
if [ "$W1" != "$W2" ]; then
    echo "oint-fence-commit: REFUSED - $LEDGER CHANGED DURING the fence ($W1 -> $W2). A peer is still writing; re-run when quiet." >&2
    exit 1
fi

# TEST HOOK, and it is here because the gate below is otherwise UNTESTABLE.
# The window between W2 and `git add` is microseconds in production, so a race
# cannot be injected into it from outside. Measured 2026-09-04: with the W1/W2
# gate DISABLED the selftest still passed every arm, because oint-fence.sh's
# OWN quiescence bracket catches a mid-fence write first -- so that gate is
# redundant defence, and THIS gate is the only new coverage. An untestable
# guard is one nobody can show fires; the hook is the smaller hazard.
# The signal makes the handshake DETERMINISTIC instead of a race. Arm 4 used
# to have its writer sleep a fixed 2s and hope that landed after the fence;
# whether it did depends on how long the fence takes, which varies with cache
# warmth and load. That is why two desks measured this arm differently and
# BOTH were right about their own machine (entry 577). The writer now waits
# for this file rather than guessing.
[ -n "${OINT_FC_TEST_SIGNAL:-}" ] && : > "$OINT_FC_TEST_SIGNAL"
[ -n "${OINT_FC_TEST_DELAY:-}" ] && sleep "$OINT_FC_TEST_DELAY"
# ARM 6. THE GATE'S OWN spdx PREDICATE ON THE FENCED PATH. ADDED 2026-09-05
# against entry 627, which filed this door as the one ARM 5 (oint-commit.sh)
# does not watch: WATCH-log.md is a tracked .md and therefore inside
# step_spdx's population, but everything this script verifies is IDENTITY --
# worktree-before == worktree-after == index. That proves the bytes are the
# ones O-WATCH wrote. It says NOTHING about what is in them, and a
# header-less filing would have landed here in byte-for-byte the 0082aabe
# shape that reddened main's release gate for every desk.
#
# The two greps are COPIED from step_spdx, exclusions and all, for the reason
# ARM 5 states: an arm that approximates the gate can disagree with it, and a
# commit guard that disagrees is worse than none. /usr/bin/grep because a
# bare `grep` is a ugrep wrapper in an INTERACTIVE shell (payload SS10) --
# latent inside a script, and pinned so it stays that way.
_fc_spdx() {
    case "$LEDGER" in
        contracts/*|Cargo.lock|LICENSE*|.gitignore) return 0 ;;
        *.rs|*.sh|*.py|*.toml|*.md|*.yaml|*.c|*.h|*.cpp|*.qml|*.wgsl) ;;
        *) return 0 ;;
    esac
    [ -f "$LEDGER" ] || return 0
    _sp=0
    /usr/bin/grep -q 'SPDX-License-Identifier:' "$LEDGER" || _sp=1
    /usr/bin/grep -q 'Copyright (c) 2026 the Appname contributors' "$LEDGER" || _sp=1
    [ "$_sp" -eq 0 ] && return 0
    echo "oint-fence-commit: REFUSED - $LEDGER fails the GATE's own spdx predicate (step_spdx). Nothing staged. Fix the header; do not weaken this arm." >&2
    return 1
}
_fc_spdx || exit 1

git add -- "$LEDGER" || exit 3
IDX=$(git ls-files -s -- "$LEDGER" | awk '{print $2}')
if [ "$IDX" != "$W2" ]; then
    git reset -q HEAD -- "$LEDGER"
    echo "oint-fence-commit: REFUSED - index blob $IDX != fenced blob $W2. The file moved between the check and the staging; index restored." >&2
    exit 1
fi

_stat=$(git diff --cached --numstat -- "$LEDGER" | head -1)
_add=$(printf '%s' "$_stat" | cut -f1); _del=$(printf '%s' "$_stat" | cut -f2)
if [ "$MODE" = sections ]; then
    _heads=$(git diff --cached -U0 -- "$LEDGER" | /usr/bin/grep -c '^+### ')
    _plural=section; [ "$_heads" -eq 1 ] || _plural=sections
    git commit -q -m "${SUBJ} (blob $(printf '%s' "$IDX" | cut -c1-8), +${_add}/-${_del}, ${_heads} ${_plural})" \
      -m "Blob, counts and section count are DERIVED at commit time, not typed: worktree-before == worktree-after == index, all $(printf '%s' "$W1" | cut -c1-8). oint-fence-commit.sh gates on that identity because the fence's quiescence bracket covers its own execution and not the gap to the commit (entry 572). --sections path added at entry 844, after the ungated by-hand sequence committed over a FENCE FAILED." \
      -m "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
    _rc=$?
    [ "$_rc" -eq 0 ] && echo "oint-fence-commit: committed $(git log -1 --format=%h) (blob $(printf '%s' "$IDX" | cut -c1-8), +${_add}/-${_del}, ${_heads} ${_plural})"
    exit "$_rc"
fi
_heads=$(git diff --cached -U0 -- "$LEDGER" | /usr/bin/grep -c '^+# Review')
_first=$(printf '%s' "$WANT" | cut -d' ' -f1)
# ⛔ SUBJECT NAMED ONE REVIEW WHILE THE COMMIT CARRIED TWO. Filed by O-WATCH
# #583 §2 against 9483d839d: "fence O-WATCH review #581 ... 2 heading" --
# the number came from $_first and the count from $_heads, so the subject
# CONTRADICTED ITSELF in its own line. The fence is the durable record
# (§6); a reader doing archaeology on that sha sees #581 and cannot learn
# that #582 is inside it. Fixed by naming EVERY declared review.
# The prefix "fence O-WATCH review #<first>" is preserved BYTE-FOR-BYTE
# because extractions already parse it -- my own entry-738 sweep used
# `fence O-WATCH review #(\d+)` over 267 commits. Extra ids are appended,
# so old greps keep working and new readers see the whole set.
_all=$(printf '%s' "$WANT" | tr ' ' '\n' | /usr/bin/grep -c .)
_rest=$(printf '%s' "$WANT" | cut -d' ' -f2- -s | tr ' ' '\n' \
        | /usr/bin/sed '/^$/d; s/^/ #/' | tr -d '\n')
_plural=heading; [ "$_heads" -eq 1 ] || _plural=headings

git commit -q -m "WATCH-log: fence O-WATCH review #${_first}${_rest} (blob $(printf '%s' "$IDX" | cut -c1-8), +${_add}/-${_del}, ${_heads} ${_plural})" \
  -m "Blob, counts and heading count are DERIVED at commit time, not typed: worktree-before == worktree-after == index, all $(printf '%s' "$W1" | cut -c1-8). oint-fence-commit.sh gates on that identity because the fence's quiescence bracket covers its own execution and not the gap to the commit (entry 572)." \
  -m "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
_rc=$?
[ "$_rc" -eq 0 ] && echo "oint-fence-commit: committed $(git log -1 --format=%h) (blob $(printf '%s' "$IDX" | cut -c1-8), +${_add}/-${_del})"
exit "$_rc"
# REUSE-IgnoreEnd
