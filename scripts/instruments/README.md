<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (c) 2026 the Appname contributors -->

# `scripts/instruments/` — the fleet's self-checking probes

**Authorized by the operator, 2026-08-25**, on a question two desks reached independently within
twenty minutes and both declined to answer for themselves: *should these probes live in the repo so
they outlive the sessions that wrote them?*

They exist because of a measured distinction:

```
MEMORY      fails WITHOUT A TRACE
CHECKLIST   fails WHEN UNREAD          — better than memory, worse than an instrument
INSTRUMENT  fails LOUDLY or not at all
```

On 2026-08-25 four desks logged twelve instrument failures in one evening. **Eight were caught by
reading output, one by a pre-existing guard, one by an incentive.** The guard is the only one that
would have worked while nobody was paying attention. A scratchpad probe fixes one session; a probe
in the tree outlives its author.

## Rules of the home (operator ruling, verbatim in substance)

5. ★★★ **INSTRUMENT REVIEW ASKS TWO QUESTIONS: "IS IT CORRECT?" AND "WHAT CONSUMES IT?"**
   *A guard that is landed but wired to nothing is recorded as* **NOT IN SERVICE** *— not as a guard.*
   **Operator ruling 2026-08-26.** *Measured that day: `o-ui-bound-a.sh` was written, reviewed,
   gate-checked and landed — and the gate step most dependent on its rule never called it. A stale
   binary then produced a FIDELITY failure that read as a regression from a 49-file landing.*
   - **A MENTION IS NOT A CONSUMER.** *This file's own inventory row for `oint-fence.sh` was counted
     as service by the desk that wrote both. Match on INVOCATION (`bash`/`sh`/`source`/`./`), not on
     the name appearing somewhere.* ★ **A false ALIVE leaves a dead guard with a tick beside it, and
     the census exists to find dead guards — its failure mode points the same way as its purpose.**
   - ★★ **A CALLER-COUNT OF ZERO IS A FINDING ONLY FOR INSTRUMENTS MEANT TO RUN AUTOMATICALLY.**
     *For a HAND-RUN CLI TOOL — one with required positional arguments and a usage string — zero
     callers is the EXPECTED shape, and the questions are instead: is it documented, is it
     reachable, and HAS IT ACTUALLY BEEN RUN?* ★ **Narrowed by O-WATCH on this rule's FIRST real
     subject, which was `oint-fence.sh`: as written the rule condemned a tool that had been
     invoked 22 times that day.** *Their words: "a rule I invented two ticks ago and applied once
     has already been wrong once, and I would rather say that than let it stand and start
     condemning your tools."* **A census rule needs its own subject-class test before it grades
     anything.**
   - **RULE CONSUMED IS NOT SCRIPT CONSUMED.** *A later guard may reimplement an instrument's rule.
     The rule is then in service and the script still is not. Record the script.* (O-UI)
   - **"IN SERVICE" IS WORKTREE-DEPENDENT** when the consumer is a machine-local settings file or
     hook. *A per-machine consumer makes it a property of the PAIR, not of the file — so a census
     must name WHICH TREE it was taken in, exactly as a count over a moving range needs its tip.*
   - **A GUARD EXERCISED ONLY WITH WELL-FORMED INPUTS DOES NOT TEST ITS CALLER.** *A guard passed
     five selftests while its caller handed it a relative path into an absolute-path comparison;
     every synthetic case was absolute on both sides, so the bug could not appear.* (O-UI)


1. **READ-ONLY.** An instrument never mutates the tree.
2. **Each carries its own controls**, and **prints `VOID` when a control dies** — never a green
   number. *(q-scenario's design: "0 findings" is indistinguishable from "can no longer see a
   finding.")*
3. **Each names its owner.**
4. **Landing or changing an instrument gets the same peer review as changing a gate**, because they
   are certification surface.

## Conventions earned the night these were written

> **House style, and it is load-bearing rather than decorative: EVERY CONVENTION HERE CARRIES THE
> MEASUREMENT THAT PRODUCED IT, not just the maxim.** *A maxim invites agreement; a maxim with its
> failure attached invites checking — and only the second survives a reader who does not already
> believe it. Every rule below is here because someone got something wrong in a way that was cheap
> to write down, and the writing-down is the part that transferred.*
>
> ★ **Evidence this works, from 2026-08-26:** a convention added here at 00:3x — *a control is only a
> control if you have MEASURED what it must return* — was load-bearing in its own author's check
> **twenty minutes later**, on a negative control they would otherwise have assumed. They had the same
> lesson in their ledger and in their head, and neither is what made them measure it. **The file did,
> because it sits on the PATH OF THE CHECK rather than in the MEMORY OF THE CHECKER.**


- **Stamp the tip**, never assume it — a count over a moving range is meaningless without it.
- **Print denominators, do not describe them.** *A description of a population is itself a claim
  that can be wrong; the only thing that cannot be wrong is the number beside the number.*
- **Verify a mutation LANDED before interpreting the control's result.** An unapplied control and a
  passing subject both produce a clean run.
- **Anchor any process predicate so it cannot match itself.** Four waiters once spun ~16h on
  `pgrep -f` patterns their own command lines satisfied.
- **VALIDATE A METHOD ON THE CASE WHERE THE CANDIDATES MUST DISAGREE.** A wrong method becomes a
  house habit by being checked against an input where its bug CANCELS. *Measured 2026-08-26:
  `git diff main..BRANCH` (two-dot, wrong for a carry) versus `main...BRANCH` (three-dot, correct)
  returned **49/47 under BOTH forms** on `lane/ui`, and **22/10 versus 3/2** on `qa/q-gesture`.
  They differ only when the base has newer commits touching shared paths.* ★ **Anyone who had
  validated the two-dot habit against `lane/ui` would have certified it correct.** Pick the input
  where the two candidate methods CANNOT agree, or the check proves nothing.
- **A CORRECTION IS A MEASUREMENT AND GOES STALE LIKE ONE.** *Stamp its tip too.* Within twenty
  minutes of correcting a branch-carry figure from 83/71 to 53/48, the true value was 49/47 —
  main had advanced. **The rule that binds the error binds the fix.**
- **Assert ANCESTRY before interpreting a commit range.** `git diff <gated>..HEAD` is only evidence
  that a gate's verdict carries if `<gated>` is genuinely an ancestor — `git merge-base --is-ancestor`,
  with a control that must fail (a lane tip). *Without it, a green run and a commit are two facts that
  merely happened near each other. Contributed by O-WATCH: it is what turns "a battery went green"
  into "a battery went green on THIS", and it costs one command.*
  ★★ **CHOOSE THE CONTROL SO IT CAN FAIL AGAINST THE ACTUAL SUBJECT.** *Reversing the direction
  (`--is-ancestor HEAD <gated>`) works only while `<gated>` is STRICTLY BEHIND HEAD. **When the
  gated commit IS HEAD — the normal case for a gate run on the tip — reverse also SUCCEEDS, because
  a commit is its own ancestor, and the control is INAPPLICABLE rather than failing.** Use an
  unrelated ref (a lane tip) instead. Found 2026-08-26 01:0x by this rule's own first user, one hour
  after banking it, on gate7 where gated == HEAD.*
- **THE TWO SIDES OF A CONTROL MUST DIFFER ONLY IN THE SUBJECT, NEVER IN THE QUESTION.** A control
  is a comparison, and a comparison asking two different questions on its two sides answers
  neither. *O-WATCH checked whether a marker pre-existed using one pattern set before the fix and
  a different one after, and got **24 → 1**. Caught only because the numbers were nonsensical: a
  marker being ADDED cannot make a count FALL. Redone with one pattern both sides: **0 → 1**.*
  ★ **A wrong number in the right direction would have passed unread.**
- **DO NOT SHIP A RULE FROM THE EXCEPTION.** Check a contributed control against the shape of
  subject its USER will have, not the one you happened to hold. *O-WATCH's ancestry control worked
  because their gated commit was strictly behind HEAD; the normal case is a gate run on the tip,
  where `gated == HEAD` makes it inapplicable. Their words: "I gave you a control that works on
  the shape of subject I happened to have."*
- **A control is only a control if you have MEASURED what it must return.** A positive control plus an
  ASSUMED negative is a half-control passing for a whole one. *O-WATCH used their own ledger as a
  "must NOT carry SPDX" control; it returned 1, because that file does carry one — so the pair proved
  nothing in either direction.* **The rule applied to zeros applies identically to ones.**
- **A control that spans TWO VERSIONS must be keyed on CONTENT, never on POSITION.** Line numbers
  are display; blob and context are identity. A diff is precisely the thing that moves positions,
  so a line window carried across a before/after pair reads the wrong content and answers
  confidently. *Contributed by O-WATCH after their own control — checking whether a marker
  pre-existed — ran a post-fix line window against the pre-fix file and returned "already there"
  for a marker that did not exist. Caught by the SHAPE of the answer: a control claiming the marker
  pre-existed contradicted a fix whose entire content was that marker.*
- **No `PIPESTATUS` after an intervening command** — capture `rc=$?` with no pipe at all.

## Two practices that outperformed every instrument here

Measured on 2026-08-26 by tallying one engagement's catches BY MECHANISM rather than by author:

```
reading output        many — every one luck-dependent
a pre-existing guard  ONE
an incentive          ONE
PRE-ANNOUNCEMENT      FOUR
```

- **THE AUDIT TRIGGER IS IMPLAUSIBILITY, NOT DIRECTION.** *A wrong number gets challenged when it
  is SATURATED or otherwise impossible — 31-of-31, 100%, all-or-nothing — not when it is alarming.*
  ★★ **So the unkillable error is the one that is both PLAUSIBLE AND REASSURING**, and a
  plausible-but-alarming figure (83 where the truth was 53) survives just as long, because
  direction only decides whether it gets challenged ONCE SOMEONE LOOKS. *Contributed by O-WATCH
  against their own case: they caught a broken check in one tick because it read 31 of 31, not
  because it read badly — **at 2-of-31 it would still be running.***
- **A SUFFICIENT CAUSE IS THE MOST EFFECTIVE WAY TO STOP LOOKING FOR THE SECOND ONE.** *After
  finding a cause, ask whether removing it **ALONE** would have prevented the failure — then ask the
  same of everything else on that line.* ★★ **Where two fixes each suffice, there were TWO defects,
  and finding either one first hides the other.** *O-WATCH's broken check had a zero-permitting
  quantifier AND an unguarded `head -1` first-hit selection; either fix alone would have worked.
  They reported one because it explained the failure COMPLETELY, and the `head -1` was their own
  code on the line they were staring at.* ★★★ ***A terminating diagnosis with no emotional colour
  at all — not flattering, not costly, just COMPLETE, which is the best disguise available.***
  **The countermeasure is arithmetic, not doubt.**
- **A POPULATION-SCANNING CHECK MUST PRINT `VOID` WHEN `n == 0`, AND THE GUARD MUST BE DEMONSTRATED.**
  *A control proves the test can tell a hit from a miss. **It says nothing about whether the test was
  applied to anything at all.*** ★★ **Zero-population success is the purest fail-into-plausible-value
  there is: a green line and a count of zero that reads as "nothing wrong" when it means "nothing
  looked at."** *O-WATCH built a staged-entry check with a positive control (a known id must be found)
  and a negative control (an impossible id must be absent), and it returned `0 scanned, 0 unlanded` —
  GREEN — because the directory variable was never defined and the glob matched nothing. **Both
  controls guarded the PREDICATE and left the POPULATION unguarded, and both passed honestly.***
- **A QUANTIFIER THAT PERMITS ZERO TURNS A "FIND THE VALUE" PATTERN INTO A "MATCH THE DELIMITER" ONE.**
  *`grep -o '#[0-9]*'` against `# Review #60` yields `#` as well as `#60` — the zero-digit match on the
  delimiter comes FIRST, so anything taking the first hit gets `#`.* **No error, no empty output, and a
  token that looks like a plausible answer.** Use `+`. ★ *It had been broken every tick since it was
  written, because it lived only in shell history and was retyped each time — **a fix that lives in
  shell history is not a fix.***

★★★ **1. ANNOUNCE A SEQUENCE BEFORE YOU RUN IT.** *Say what you are about to do, in what order, and
what transient state it will open.* It caught more than every guard and incentive combined, costs no
code and no maintenance, and — decisively — **it is the only mechanism that works in the window
BETWEEN an auditor's ticks, which is exactly where instruments cannot reach.**
> **A detective guard SAMPLES; it does not OBSERVE.** An hourly watch cannot witness a five-minute
> red. ★★ *And the gap is not empty: **an auditor whose cadence cannot reach a window has, for that
> window, delegated verification to the SUBJECT.** It is invisible precisely because everything comes
> out green — what fills the gap is not nothing, it is SELF-REPORT.*

★★★ **2. MARK EVERY CLAIM `MEASURED` OR `RELAYED` AT THE POINT OF WRITING.** *A reader cannot recover
the distinction afterwards.* **A relay AMPLIFIES.** ★★ *Amended 2026-08-26 by O-WATCH, against a rule I wrote:* ***THE HAZARD
OF A RELAY IS NOT THAT IT IS LESS ACCURATE — IT IS THAT ITS ACCURACY IS UNEXAMINED, AND THAT CUTS
BOTH WAYS.*** *The original form said a relay is WORSE because it arrives pre-endorsed. Then a
pre-endorsed relay turned out to be ACCURATE while the first-hand claim it repeated was not.* ★ **The
hazard it exposed is the one nobody was watching for: a relay that merely repeats your own finding
back to you invites waving through, which is the same failure as swallowing one because it sounds
authoritative.** *(Case-history note, corrected 2026-08-26: O-WATCH did NOT wave it through — they
measured, which is how the inaccuracy was found at all. An earlier version of this entry said
otherwise. **A correct rule taught with a false case history gets discredited by the first person
who checks it against the record.**)*
*Worked example, one desk in one night: a peer's account of a mechanism carried unread into a ledger,
then into an operator-facing recommendation, then into a discriminator — three defects, one shape.
The mechanism in question was a hardcoded literal eleven lines above the line that had been read.*

## Open verification items

*Instruments this home does not yet have, recorded with a NAMED DESIGN so the gap is a specification
rather than a shrug. **An item here is UNVERIFIED, not unverifiable** — the distinction matters
because "unverifiable" is a terminating diagnosis: it does not explain the failure to verify, it
dissolves the obligation to try.*

### FI-22 ordering clause — the stage-tap
**Status: HALF-CLOSED, trails the freeze rather than blocking it (operator ruling 2026-08-26).**

*What IS measured, on two axes with boundaries accepted and one-past refused, key named every time:
the engine refuses out-of-range effects parameters with `error.schema.out_of_range` —
`altitudeDeg` 90 in / 91 out, `angleDeg` 180 in / 181 out.*

★ **What is NOT measured: that the refusal occurs AT STAGE 3, nor that "the engine never computes
with the offending value."** *§11.3's clause 4 says that ordering is NORMATIVE and not an
implementation detail.* ***A REFUSAL WITH THE RIGHT CODE IS NOT EVIDENCE OF THE RIGHT ORDERING** — a
correct code observed at the wrong stage is indistinguishable, from the outside, from a correct code
observed at the right one.*

**NAMED DESIGN — a stage-tap proving the offending value never reaches the reach arithmetic.** *The
observable must be that the ROI/reach computation was NEVER ENTERED for the refused command, not that
it returned early: those differ exactly where the FI-22 hazard lives, since `IRect::inflate` panics
INSIDE the arithmetic and an early return after entry would still have saturated.* **Two controls it
must carry: an in-range command that DOES reach the arithmetic (or the tap proves nothing), and a
refused command that does not.**

## Inventory

| instrument | owner | what it certifies |
|---|---|---|
| `oint-fence.sh` | O-INT | a peer-authored ledger is fence-clean before O-INT commits it |
| `oint-e11-check.sh` | O-INT | a committed E-11 version bump is complete: order after the certificate, scope by measured class, the lock, the census, `--locked`, the notices file, the selftest pin, and that **no package golden still carries the old version** |
| `owatch-golden-diff-review.py` | O-WATCH | the regenerated package goldens differ in `createdWith.version` **and nothing else** |
| `oint-validmin-contract.py` | O-INT | every kind's published `validMinimal` satisfies its own contract-declared schema, at the levels decidable without a JSON-Schema engine |
| `oint-validmin-staged.py` | O-INT | those same examples RUN, with what each one's target names staged first; states its own rig limits as UNDECIDED rather than as verdicts |

★ **The two golden checkers are complementary and neither is a superset of the other** (O-WATCH #898 §2, recorded here so that nobody retires one for the other): `oint-e11-check.sh` (8) asks *does every golden carry the new version*; `owatch-golden-diff-review.py` asks *did anything else inside those archives move*. A regeneration that bumped the version **and** perturbed a tile passes the first and fails the second.

★ **The two validMinimal sweeps are two halves of one question** (entries 1073, 1075): the contract half reads the published schema, the staged half runs the engine. A contract that DELEGATES a nested object — `adjustment.add`'s `params.params` says "re-validate against that opKind's registered schema" — is invisible to the first and decided by the second. That is how the one wrong example in 102 was found.
