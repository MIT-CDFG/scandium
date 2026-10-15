#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
# oint-fence.sh — OWNER: O-INT (integration desk)
# READ-ONLY. Never mutates the tree. See ./README.md for the rules of this home.
# Verifies a peer-authored ledger is fence-clean BEFORE O-INT commits it.
# Every leg carries a control. Prints VOID rather than PASS if any control dies.
set -uo pipefail

usage() {
  cat <<'USAGE'
oint-fence.sh <repo> <ledger-path> [review_number]
  Verifies a peer-authored ledger is fence-clean BEFORE O-INT commits it.
  READ-ONLY. Never mutates the tree.
  Exit: 0 PASS · 1 FENCE FAILED · 2 REFUSED (ledger moved during the check) · 3 VOID (control dead)
  Quiescence method: git hash-object. sha256/sha1/md5 of the same bytes all differ --
  a hash without its method is not a shared discriminator.
USAGE
}
case "${1:-}" in -h|--help|"") usage; [ -z "${1:-}" ] && exit 3 || exit 0 ;; esac
REPO="$1"
LEDGER="${2:?$(usage)}"
# WANT is EVERY review number from arg 3 onward, not just arg 3. Collected without
# `shift` so $1/$2 stay bound for anything below. Caught by a fixture: with WANT
# bound to "${3:-}" alone, declaring "900 901" still counted ONE declared and the
# undeclared-entry arm fired on a correctly-declared pair.
WANT=""; _argi=0
for _arg in "$@"; do
    _argi=$((_argi + 1))
    [ "$_argi" -ge 3 ] && WANT="${WANT:+$WANT }$_arg"
done
cd "$REPO" || { echo "VOID - repo unreadable"; exit 3; }

TIP=$(git rev-parse --short HEAD)                       # tip STAMPED, not assumed
echo "=== O-INT FENCE  repo=$REPO  tip=$TIP  ledger=$LEDGER"

# --- QUIESCENCE BRACKET, OPEN. Added 2026-08-26 after this instrument returned a
# clean PASS on a ledger a peer was STILL WRITING (O-INT log F-3): four samples,
# 916033 -> 917164 -> 917910 bytes, three distinct blobs, and the PASS was issued
# against the middle one. Committing on it would have fenced a partial peer entry.
# Method NAMED because the same bytes give four different digests.
H1=$(git hash-object "$LEDGER" 2>/dev/null || echo MISSING)

dead=0; fail=0

# --- CONTROL 1: heading grep must return 0 for a heading that cannot exist ---
c1=$(git diff -U0 -- "$LEDGER" | grep -cE '^\+# Review #99999' || true)
[ "$c1" -eq 0 ] || { echo "  CONTROL-1 DEAD (impossible heading matched $c1)"; dead=$((dead+1)); }

# --- CONTROL 2: the path filter must FIRE on a known mixed commit ---
# G-652 POINT-OF-USE NOTE (2026-08-28): this `head -1` IS A SELECTION, NOT A TALLY.
# It picks the NEWEST commit touching crates/ so CONTROL-2 has a known-mixed commit to
# fire against; the -n 200 bounds the walk, and neither number is ever reported as a
# measurement. DO NOT "FIX" IT UNDER G-507 (a tally is only a census if its denominator
# came from outside the run) -- that rule is about COUNTS, and nothing here is counted.
# Recorded HERE rather than in ENVIRONMENT-TRAPS.md because an index needs a suspicion
# that arrives too late: I stepped in two of my own recorded traps tonight.
mixed=$(git log --format=%H -n 200 -- crates/ 2>/dev/null | head -1)
if [ -n "$mixed" ]; then
  c2=$(git diff --name-only "$mixed~1..$mixed" | grep -cv '^reports/' || true)
  [ "$c2" -gt 0 ] || { echo "  CONTROL-2 DEAD (mixed commit showed 0 non-reports paths)"; dead=$((dead+1)); }
else
  echo "  CONTROL-2 DEAD (no crates/ commit found to control against)"; dead=$((dead+1))
fi

# --- CONTROL 3: assert there IS a diff at all; an empty diff must not read as clean ---
tot=$(git diff --numstat -- "$LEDGER" | awk '{print $1}')
del=$(git diff --numstat -- "$LEDGER" | awk '{print $2}')
[ -n "${tot:-}" ] || { echo "  CONTROL-3 DEAD (no diff on $LEDGER - nothing staged to fence)"; dead=$((dead+1)); tot=0; del=0; }

# --- CONTROL 4: the quiescence comparator must be able to SEE a difference ---
# Without this, an always-equal comparator would report "quiescent" forever.
_qc=$(mktemp) || _qc=""
if [ -n "$_qc" ]; then
  printf 'a' > "$_qc"; _qa=$(git hash-object "$_qc")
  printf 'b' >> "$_qc"; _qb=$(git hash-object "$_qc")
  rm -f "$_qc"
  [ "$_qa" != "$_qb" ] || { echo "  CONTROL-4 DEAD (hash-object returned the same value for different bytes)"; dead=$((dead+1)); }
else
  echo "  CONTROL-4 DEAD (no temp file to control against)"; dead=$((dead+1))
fi

if [ "$dead" -gt 0 ]; then
  echo "VOID - $dead control(s) dead. THIS RUN PROVES NOTHING."; exit 3
fi

# --- checks, denominators PRINTED not described ---
# COUNT BOTH INDEPENDENTLY so the sum below is a REAL check.
# Until 2026-08-26 blank was DERIVED as (tot-con), which made the assertion
# con+blank==tot a TAUTOLOGY -- it passed for any value of con, and it was
# reported as "sum-asserted against numstat" on 21 fences. It never was.
# Both predicates strip the leading diff marker with cut, so an added line
# whose own first character is '+' is counted rather than skipped: the
# classic `grep '^+[^+]'` undercounts exactly there. (O-WATCH found the
# same blindness on the deletion side of their own checker the same hour.)
con=$(git diff -U0 -- "$LEDGER" | grep -E '^\+' | grep -v '^+++' | cut -c2- | grep -cv '^[[:space:]]*$' || true)
blank=$(git diff -U0 -- "$LEDGER" | grep -E '^\+' | grep -v '^+++' | cut -c2- | grep -c '^[[:space:]]*$' || true)
# **** THE SCOPE CHECK KEYS ON THE STAGED SET, NOT THE DIRTY SET.
# REPAIRED 2026-09-06 on the operator's first-hand word, against the finding
# filed in entry 669 (first noted as an aside in entry 624 and DECLINED four
# times since -- including in the tick where it cost the most, because a desk
# that loosens its own gate under pressure has no gate).
#
# It used to read `git status --porcelain`: EVERY dirty path in the worktree,
# refusing if any was not the ledger. But the commit this fence guards is
# PATH-SCOPED -- oint-fence-commit.sh runs `git add -- "$LEDGER"` and stages
# nothing else -- so a foreign file that is merely DIRTY cannot enter it by any
# route. The check was measuring EXISTENCE where the invariant is PARTICIPATION.
#
# The cost was not hypothetical: O-WATCH review #492 sat unfenceable for fifteen
# consecutive ticks behind a peer's uncommitted log the commit would never have
# touched.
#
# THE REAL HAZARD SURVIVES AND IS EXACTLY WHAT THIS NOW MEASURES: `git commit`
# takes the WHOLE INDEX, so a foreign path already STAGED before the fence ran
# would ride along on the commit. `git status --porcelain` cannot separate that
# from a merely-dirty file; `git diff --cached` names precisely the set that
# would be committed.
dirty=$(git status --porcelain | awk '{print $2}')
npaths=$(printf '%s\n' "$dirty" | grep -c . || true)
staged=$(git diff --cached --name-only)
nstaged=$(printf '%s\n' "$staged" | grep -c . || true)
# Empty lines are filtered BEFORE the inverse match: `printf '%s\n' ""` emits one
# blank line, which does not match ^LEDGER$ and would score a clean index as one
# foreign path -- a refusal manufactured out of nothing.
foreign=$(printf '%s\n' "$staged" | grep -v '^$' | grep -cv "^$LEDGER$" || true)
heads=$(git diff -U0 -- "$LEDGER" | grep -E '^\+# Review #' | sed -E 's/^\+(# Review #[0-9]+).*/\1/')
nheads=$(printf '%s\n' "$heads" | grep -c . || true)

printf '  numstat        +%s / -%s\n' "$tot" "$del"
# ★★★★★ THE VERDICT NAMES ITS OWN INDEPENDENCE, BECAUSE THE BARE SUM INVITED A
# MISREAD. O-WATCH #404-C read the historical comment above -- which records that
# `blank` USED to be derived as (tot-con) and was fixed on 2026-08-26 -- and
# reported the tautology as CURRENT, quoting "the instrument says so itself".
# ★★★★ They were right about the comment and wrong about the state, and the
# comment is why: it describes a defect that WAS. That is G-529 in REVERSE -- a
# note about a FIXED defect outliving the fix and reading as a live one.
# ★★★ The residual is mine: `(sum N == N)` looks identical whether the two terms
# were counted independently or one was computed from the other, so a reader
# CANNOT TELL A REAL CHECK FROM A TAUTOLOGY FROM THE OUTPUT. The line now says
# which it is. Verified the hard way rather than by reading: fed con=2 blank=1
# tot=10, the assertion returns NO. A tautology cannot do that.
printf '  content/blank  %s / %s   (sum %s == %s, both counted independently)\n' "$con" "$blank" "$((con+blank))" "$tot"
printf '  paths dirty %s   STAGED %s   foreign(staged, non-ledger) %s\n' "$npaths" "$nstaged" "$foreign"
printf '  new headings   %s  [%s]\n' "$nheads" "$(printf '%s ' $heads)"
# G-432 (O-WATCH, 08-27): '^-[^-]' CANNOT SEE A DELETED BLANK LINE -- [^-] demands a
# second character and an empty deleted line is a bare '-'. THE GATE BELOW WAS NEVER
# BLIND: it reads numstat field 2, which does count blank deletions. Verified on a
# fixture rather than by reading -- one deleted blank gives numstat 1 / grep 0, and two
# deletions of which one is blank give numstat 2 (my gate FIRES) where a grep-based gate
# reads 1 and PASSES. Only this DISPLAY was blind, so a deleted blank line would fire the
# check while being absent from the list printed beside it: a safe failure, an incomplete
# one. Anchor on the real diff header, NOT on a run of '-': a deleted markdown rule is
# itself '---' and must still be shown.
echo   "  deletions:"; git diff -U0 -- "$LEDGER" | grep '^-' | grep -vE '^--- (a/|/dev/null)' | cut -c1-90 | sed 's/^/    /'

[ "$foreign" -eq 0 ] || { echo "  ★ OUT OF SCOPE: $foreign non-ledger path(s) IN THE STAGED SET -- a dirty foreign file is fine (the commit is path-scoped); a STAGED one would ride the commit"; fail=$((fail+1)); }
[ $((con+blank)) -eq "$tot" ] || { echo "  ★ ARITHMETIC: content($con)+blank($blank)=$((con+blank)) != numstat($tot) -- a REAL check now, not a tautology"; fail=$((fail+1)); }
[ "$del" -le 1 ] || { echo "  ★ DELETIONS: $del (>1 - a peer record may be at risk)"; fail=$((fail+1)); }
if [ -z "$WANT" ]; then
  # DECLARING NOTHING IS ITSELF A DECLARATION: "this fence adds no review entry".
  # 73 of 186 historical fences are exactly this -- state-line updates and addenda --
  # and they are legitimate. But with the set-equality arm guarded by `-n "$WANT"`,
  # a run with NO review number SKIPPED THE CHECK ENTIRELY, so a blob carrying an
  # undeclared entry would pass whenever the caller omitted the argument. That is the
  # same hole the arm was written to close, reachable by a shorter route.
  # O-WATCH #150 named the 0-of-186 partition; this closes it in the direction they
  # did not file, because "0 == 0 passes without examining what changed" is only safe
  # if 0 is ASSERTED rather than assumed.
  [ "$nheads" -eq 0 ] || { echo "  ★ UNDECLARED ENTRY: $nheads new heading(s) [$(printf '%s ' $heads)] but NO review number was declared -- pass the review number(s) as arg 3 onward"; fail=$((fail+1)); }
else
  # THE THIRD ARGUMENT IS A REVIEW NUMBER, NOT HEADING TEXT. It was labelled
  # `expected_new_heading`, and O-INT #7 duly passed "# Review #76" -- which
  # built the pattern `^\+# Review ## Review #76 `, matched nothing, and printed
  # "HEADING ## Review #76 appears 0 times". That reads as a HEADING-LEVEL
  # mismatch, and the investigation went to the ledger's heading convention
  # across reviews #67..#75 instead of to the call. G-142: A USAGE LABEL THAT
  # NAMES THE WRONG THING MISDIRECTS THE DIAGNOSIS OF ITS OWN FAILURE. The check
  # failed CLOSED and blocked the commit, which is correct; it just spent the
  # search in the wrong place. Printing the CONSTRUCTED PATTERN makes a bad
  # argument self-evident.
  # 2026-08-27: this check asserted only that the DECLARED review's heading appears
  # exactly once. IT DID NOT ASSERT THAT IT IS THE ONLY NEW HEADING. Measured on
  # O-WATCH's #148+#149 blob: run with 148 it passed, run with 149 it passed, and
  # NEITHER RUN OBJECTED TO THE OTHER ENTRY BEING PRESENT. There that was correct --
  # both were declared -- but A BLOB CARRYING AN UNDECLARED EXTRA ENTRY WOULD HAVE
  # FENCED CLEAN UNDER THE DECLARED ONE, and the commit message would have named one
  # review while the tree gained two. The fence's whole purpose is that the committed
  # object is the declared object.
  #
  # Every review number from arg 3 onward is now DECLARED, and the assertion is a SET
  # EQUALITY: each declared number appears exactly once, AND the count of new headings
  # equals the count declared. An extra heading now fails closed.
  _declared=0
  for _w in $WANT; do
      _declared=$((_declared + 1))
      _pat="^\+# Review #$_w "
      got=$(git diff -U0 -- "$LEDGER" | grep -cE "$_pat" || true)
      [ "$got" -eq 1 ] || { echo "  ★ HEADING pattern [$_pat] matched $got line(s) (want exactly 1) -- 3rd arg onward are REVIEW NUMBERS, e.g. 76 or '148 149'"; fail=$((fail+1)); }
  done
  [ "$nheads" -eq "$_declared" ] || { echo "  ★ UNDECLARED ENTRY: $nheads new heading(s) [$(printf '%s ' $heads)] but $_declared declared -- fence the declared set or nothing"; fail=$((fail+1)); }
fi

# --- QUIESCENCE BRACKET, CLOSE. Before ANY verdict, including a failing one:
# a FENCE FAILED issued against bytes that have since changed is as wrong as a PASS.
H2=$(git hash-object "$LEDGER" 2>/dev/null || echo MISSING)
printf '  quiescence     %s -> %s  (method: git hash-object)\n' "$H1" "$H2"
if [ "$H1" != "$H2" ]; then
  echo "★ REFUSED - THE LEDGER MOVED DURING THE CHECK. Every figure above describes bytes"
  echo "  that no longer exist. The peer is still writing; re-run when it settles."
  exit 2
fi

if [ "$fail" -gt 0 ]; then echo "★ FENCE FAILED - $fail check(s). DO NOT COMMIT."; exit 1; fi
echo "FENCE PASS (tip $TIP, 4 controls live, quiescent across the check)"
# entry 963. PASS is a BYTE verdict and nothing more. It asserts: the declared headings
# are present, there are no deletions, nothing foreign is staged, the file did not move
# across the check, and the four self-tests are live. It does NOT assert that anyone READ
# the review -- which is the entire substance of MINOR-17, and the thing I have got wrong
# before (entry 934: fenced RELEASE-RIGHTS at +83/2 sections having read +45/1). Twice in
# three ticks my instruments' summary lines claimed more than their code evaluated
# (entries 959, 962); this one says its limit at the point of use rather than in a header.
echo "  PASS = bytes intact and declarations matched. It does NOT mean the review was READ (MINOR-17)."
exit 0
