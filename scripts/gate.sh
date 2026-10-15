#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# REUSE-IgnoreStart
# (This file's SPDX header above is the real one; every "SPDX-License-Identifier:"
# below is the literal token this gate GREPS FOR in step_spdx / describes in
# comments — not a second license tag. REUSE would otherwise misparse them.)
#
# THE single gate entry — plan §2.1 item 7; ES §33 CI discipline.
# Every merge and every checkpoint runs this. Steps (each echoed as
# "GATE <name> PASS|FAIL"; any FAIL => non-zero exit):
#   build          cargo build --workspace
#   test           cargo test --workspace
#   deny           cargo deny check advisories bans licenses sources
#                  (advisories network fetch probed live and green at
#                  bootstrap 2026-08-11; if a future environment blocks the
#                  fetch, that is an honest red to raise to O-INT, not a step
#                  to silently skip)
#   spdx           every git-tracked *.rs *.sh *.py *.toml *.md *.yaml *.c
#                  *.h *.cpp *.qml *.wgsl carries BOTH
#                  "SPDX-License-Identifier:" and
#                  "Copyright (c) 2026 the Appname contributors"
#                  (markdown uses HTML-comment form, proto precedent).
#                  *.json stays unswept: JSON has no comment syntax.
#                  Documented exclusions: contracts/** (verbatim frozen-spec
#                  mirror — never edited, never headed), Cargo.lock
#                  (generated), LICENSE* (license texts themselves),
#                  .gitignore (housekeeping). When fixtures/goldens trees
#                  land later, O-QA negotiates their exclusion here.
#   contract-sync  tools/sync_contracts.sh --check (mirror == manifest ==
#                  frozen bogos sources, byte-identical)
#   notices        tools/gen_notices.py --check — THIRD-PARTY-NOTICES.md
#                  drift. PROMOTED from ci_local tier (CP-0.7 wave-1 review,
#                  notices-mechanization MAJOR): port-obligation activation
#                  is landing-detected, so the post-merge state on main with
#                  a stale generated file MUST be red in the merge train —
#                  O-INT regenerates + commits, --check goes green. The same
#                  step also hard-fails on VENDORED-TEXT-DRIFT: dep-record
#                  vendoredTexts pins (sha256 from first-hand archive reads)
#                  are re-verified against the exact locked -sys package
#                  sources at every generation, so a version bump that
#                  changes or moves a vendored C license text (FreeType FTL,
#                  HarfBuzz COPYING, liblz4/zstd/LCMS2 texts) is red here,
#                  never a silently stale shipped text.
#   i18n-audit     python3 i18n/tools/audit.py (O-INT AUTHORIZED wiring,
#                  2026-08-13 ruling): MessageId registry/catalog/emission
#                  audit — the standing cross-lane tripwire. The script is
#                  O-I18N's (their fence); this gate wires the invocation
#                  only. Quiet on pass; full audit report printed on fail.
#                  (switch_check.sh runs at ci_local tier, not here.)
#   fmt            cargo fmt --all --check (ninth step, 2026-08-17: two
#                  drift instances walked in green; introduced with a
#                  one-time mechanical workspace sweep so it lands green)
#   walled-symbols zero hits for rights-walled identifiers over the M-6
#                  ruling dir set: crates/ shell/ tests/ fixtures/ i18n/
#                  (each existence-guarded); contracts/ excluded (frozen
#                  mirror), reports/ excluded (audit trail quotes trip-case
#                  evidence by design — O-WATCH's semantic sweep covers it).
#                  See step for the exact pattern list. At CP-1.0 this
#                  tightens to the compile-off-flag allowlist +
#                  release-build symbol-absence assertion (D-44).

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# (a) toolchain self-locate: non-interactive shells on this machine do not
# have ~/.cargo/bin on PATH.
if ! command -v cargo >/dev/null 2>&1; then
    export PATH="$HOME/.cargo/bin:$PATH"
fi

FAILURES=0
UNMEASURED=0
SUMMARY=""

note() {
    SUMMARY="${SUMMARY}${1}
"
    echo "$1"
    # O-WATCH #297/#298: THE STEP VERDICTS LIVED ONLY IN STDOUT, WHICH NOBODY
    # KEEPS. Scoring a run's arms afterwards, they could re-derive FIVE of seven
    # from retained artifacts and not the two that depend on this line --
    # GATE_RC and the step tally. Their words: "you can score them because you
    # watched your own terminal; I cannot, because no artifact carries them."
    # That is the same auditability gap the retention copy closed for suite
    # output, one field over, and the same remedy: WRITE IT DOWN BESIDE THE RUN.
    #
    # APPEND-ONLY AND FAILURE-SILENT BY DESIGN. This is a RECORD, not a gate: if
    # the path is unwritable the run must proceed exactly as before, because a
    # verdict that depends on its own transcript succeeding is a worse instrument
    # than one with no transcript. No rc is read, nothing branches on it.
    if [ -n "${GATE_STEPLOG:-}" ]; then
        printf '%s\n' "$1" >> "$GATE_STEPLOG" 2>/dev/null || :
    fi
}

# VERDICT CLASSES (operator ruling 2026-08-26, "D"). rc 2 means the step did
# NOT MEASURE the tree; rc 1 means it measured and the tree was wrong. Both
# still fail the gate -- an unmeasured step is not a passing one -- but they
# are no longer the same word in the summary. The cost of conflating them is
# on the record: a stale runner produced a FIDELITY failure that read as a
# regression from a 49-file landing, and three desks spent an hour on the tree
# instead of the binary.
run_step() {
    local name="$1"
    shift
    if "$@"; then
        note "GATE $name PASS"
    else
        local _rc=$?
        if [ "$_rc" -eq 2 ]; then
            note "GATE $name UNMEASURED"
            UNMEASURED=$((UNMEASURED + 1))
        else
            note "GATE $name FAIL"
        fi
        FAILURES=$((FAILURES + 1))
    fi
}

step_spdx() {
    local bad=0 f
    while IFS= read -r f; do
        case "$f" in
            contracts/*|Cargo.lock|LICENSE*|.gitignore) continue ;;
        esac
        if ! grep -q 'SPDX-License-Identifier:' "$f"; then
            echo "SPDX missing license identifier: $f" >&2
            bad=1
        fi
        if ! grep -q 'Copyright (c) 2026 the Appname contributors' "$f"; then
            echo "SPDX missing copyright line: $f" >&2
            bad=1
        fi
    done < <(git ls-files -- '*.rs' '*.sh' '*.py' '*.toml' '*.md' '*.yaml' '*.c' '*.h' '*.cpp' '*.qml' '*.wgsl')
    return "$bad"
}

# Walled-symbol patterns, single-sourced: the step below and
# tools/selftest_walled_sweep.sh (via --print-walled-patterns) consume the
# SAME strings, so the selftest can never validate a stale copy. Mode tags:
# ci = case-insensitive pass, cs = case-sensitive pass.
readonly WALLED_MODE1=ci WALLED_PASS1='(^|[^[:alnum:]])(patchmatch|nnf|surf|berman|mixbox|bm3d)([^[:alnum:]]|$)'
readonly WALLED_MODE2=cs WALLED_PASS2='(^|[^[:alnum:]])([Pp]atch[Mm]atch|[Nn]nf|[Ss]urf|[Bb]erman|[Mm]ixbox|[Bb]m3[Dd])[[:upper:]]'
readonly WALLED_MODE3=cs WALLED_PASS3='(^|[^[:alnum:]])(PATCHMATCH|NNF|SURF|BERMAN|MIXBOX|BM3D)[[:upper:]][[:lower:]]'
readonly WALLED_MODE4=ci WALLED_PASS4='seam[_ ]?carv'

if [ "${1:-}" = "--print-walled-patterns" ]; then
    printf '%s %s\n' "$WALLED_MODE1" "$WALLED_PASS1" \
                     "$WALLED_MODE2" "$WALLED_PASS2" \
                     "$WALLED_MODE3" "$WALLED_PASS3" \
                     "$WALLED_MODE4" "$WALLED_PASS4"
    exit 0
fi

step_walled_symbols() {
    # Three passes, explicitly word-boundary aware WITH "_" treated as a
    # DELIMITER. (grep -w was rejected in review R1: its word class includes
    # "_", so snake_case/CamelCase compounds like patchmatch_search_stub or
    # NnfField slipped through.)
    #   pass 1 (any case): stem delimited on both sides by non-alphanumerics
    #           or line edges — catches patchmatch_search_stub, NNF_FIELD,
    #           bare PatchMatch/SURF/bm3d.
    #   pass 2 (case-sensitive): lowercase-carrying stem immediately followed
    #           by an uppercase letter — catches CamelCase compounds
    #           (NnfField, PatchMatchSearch, Bm3dDenoise).
    #   pass 3 (case-sensitive): ALL-CAPS acronym stem followed by an
    #           uppercase-then-lowercase pair (the CamelCase tail) — catches
    #           NNFField, SURFDetector, BM3DDenoise (R2 residual). The
    #           upper-then-LOWER requirement keeps all-caps runs like
    #           SURFACE / SURFACE_ID clean ("SURF" there is followed by two
    #           uppercase letters).
    #   pass 4: seam-carving stem "seam[_ ]?carv" (any case) — catches
    #           seam_carving / seam carving / SeamCarving / seamcarve; the
    #           ALL-CAPS analog (SEAMCARVING / SEAMCarving) is already caught
    #           by this case-insensitive stem, rig-proven, so no seam
    #           equivalent of pass 3 is needed.
    # Verified non-matching: "surface", "Surface", "SURFACE", "SURFACE_ID",
    # "seamless", "seam boundary", "resurface_map", "windsurfing".
    # NOTE: this list tightens to the compile-off-flag allowlist at CP-1.0
    # (date-gated engines get an explicit flag-scoped exemption; release
    # builds assert symbol absence, D-44).
    # Dir scope per the O-INT M-6 ruling (integration log triage, f910d40):
    # mechanical walled sweeps cover crates/ shell/ tests/ fixtures/ i18n/,
    # each guarded for existence. Deliberately EXCLUDED: contracts/ (frozen
    # verbatim mirror) and reports/ (the audit trail quotes trip-case rig
    # evidence by design, so a mechanical grep false-positives there —
    # reports/ is O-WATCH's judgment-bearing semantic sweep, not grep
    # territory).
    #
    # BINARY FILES SKIPPED (-I on every pass; O-UI filing 2, 2026-08-17):
    # the sweep is a SOURCE tripwire, and recursive coverage of untracked
    # build products false-positived on a compiled object file — a GENUINE
    # coincidental machine-code byte run forming a walled stem delimited by
    # non-alnum bytes (reviewer-reproduced with this gate's actual matcher,
    # /usr/bin/grep BSD 2.6.0; -a does NOT help, the bytes are real; O-UI
    # log, slice T entries, quotes the trip bytes). -I
    # (--binary-files=without-match) drops NUL-classified files from the
    # sweep while every text file, tracked or untracked, stays covered.
    # Compiled-artifact coverage is NOT lost overall: release binaries get
    # the D-44 symbol-absence assertion at CP-1.0 (a purpose-built binary
    # check, not a text grep). O-UI's shell-build relocation to
    # target/shell-build/ stands as defense-in-depth. VALIDATION DISCIPLINE
    # (the O-UI wrapper-grep lesson): any change here is validated via
    # tools/selftest_walled_sweep.sh, which pins the matcher identity to
    # the gate's own resolution under bash and consumes these exact
    # patterns via --print-walled-patterns — never an interactive shell's
    # grep, which on this machine is a ugrep wrapper with injected flags.
    # GATE-VACUITY #1 FIX (O-INT, 2026-08-23; three independent confirmations).
    # Two defects, both of which made this step PASS while measuring nothing:
    #   (a) crates/ was seeded UNGUARDED into dirs while the other four were
    #       existence-guarded. Run anywhere crates/ is absent, every grep took a
    #       nonexistent path, exited 2, and `if grep` read 2 as "no match" => PASS.
    #   (b) `if grep` cannot separate rc=1 (searched, found nothing — the real
    #       pass) from rc>=2 (grep FAILED: bad path, unreadable file, broken
    #       pattern). The vacuous case and the clean case returned the same value.
    # A sweep that could not have found anything must not report the same result
    # as a sweep that looked and found nothing. UNMEASURED IS A FAILURE, NOT A PASS.
    local dirs=() d
    for d in crates shell tests fixtures i18n; do
        [ -d "$d" ] && dirs+=("$d")
    done
    if [ "${#dirs[@]}" -eq 0 ]; then
        echo "walled_symbols: UNMEASURED-NO-DIRS (none of crates shell tests fixtures i18n present)" >&2
        return 2
    fi
    local hits=0 rc
    grep -rniE -I --exclude-dir=target "$WALLED_PASS1" "${dirs[@]}"; rc=$?
    if [ "$rc" -eq 0 ]; then hits=1
    elif [ "$rc" -gt 1 ]; then echo "walled_symbols: UNMEASURED-GREP-ERROR pass1 rc=$rc" >&2; hits=1; fi
    grep -rnE -I --exclude-dir=target "$WALLED_PASS2" "${dirs[@]}"; rc=$?
    if [ "$rc" -eq 0 ]; then hits=1
    elif [ "$rc" -gt 1 ]; then echo "walled_symbols: UNMEASURED-GREP-ERROR pass2 rc=$rc" >&2; hits=1; fi
    grep -rnE -I --exclude-dir=target "$WALLED_PASS3" "${dirs[@]}"; rc=$?
    if [ "$rc" -eq 0 ]; then hits=1
    elif [ "$rc" -gt 1 ]; then echo "walled_symbols: UNMEASURED-GREP-ERROR pass3 rc=$rc" >&2; hits=1; fi
    grep -rniE -I --exclude-dir=target "$WALLED_PASS4" "${dirs[@]}"; rc=$?
    if [ "$rc" -eq 0 ]; then hits=1
    elif [ "$rc" -gt 1 ]; then echo "walled_symbols: UNMEASURED-GREP-ERROR pass4 rc=$rc" >&2; hits=1; fi
    return "$hits"
}

# GATE 1 (X-11 "REUSE all-PASS"), operator's WORD 4 (4), 2026-09-11: "the reuse step
# added to gate.sh ... red until the 31 are covered is expected; 09-14 stands."
#
# ★ INAPPLICABLE IS NOT PASSING -- and unlike step_release_engine_bind (:403, whose
# header says this and whose code returns 0 anyway, O-WATCH #660) this step exits 2
# when the tool is absent, which run_step counts as UNMEASURED, not PASS.
#
# The step reports the two figures the row is judged on rather than only an exit code,
# because "reuse lint rc=1" alone cannot tell a repo that got worse from one that got
# better. Both numbers are DERIVED at the gate -- AMENDMENT 8 clause (c).
# ★★★★★ GRACE WINDOW -- OPERATOR WORD 5 (2026-09-12 22:39 ET), RULING ON Q-D62 (a):
# "step_reuse is NON-BLOCKING until 09-14 18:00 ET: it prints its two figures and
# returns 0; after that it blocks."
#
# WHY IT EXISTS. This step landed at f6fba8737 on WORD 4 (4), which said "red until the
# 31 are covered is expected; 09-14 stands". Nobody traced that to SS10.3, which requires
# a GREEN gate for every merge -- so a step added on his word made the landing his OTHER
# word requires impossible. Measured link by link at entry 886: reuse lint rc=1 ->
# step_reuse returns 1 -> run_step increments FAILURES, and it does so in BOTH branches,
# so even a deliberate rc=2 UNMEASURED reds the gate. Q-D62 put the fork to him unchosen;
# this is (a), in his words.
#
# ⛔ A GRACE PASS IS NOT A PASS AND MUST NEVER READ AS ONE. The two figures print in
# EVERY mode, and while in grace the step says so in capitals, names the expiry, and
# carries the LIVE rc -- so neither a reader nor a certificate can mistake "not blocking
# yet" for "compliant". That is this file's own UNMEASURED-vs-PASS standing, applied to a
# window the operator opened rather than to a missing tool.
#
# THE DEADLINE IS A LITERAL EPOCH ON PURPOSE, not a parsed date: `date -j -f` is BSD-only
# and `date -d` is GNU-only, and a step that could not parse its own deadline would have
# to return UNMEASURED -- reddening the gate on the landing day for a reason that has
# nothing to do with rights. DERIVED AND ROUND-TRIPPED 2026-09-12:
#   TZ=America/New_York date -j -f '%Y-%m-%d %H:%M:%S' '2026-09-14 18:00:00' '+%s'
#                                           -> 1789423200
#   TZ=America/New_York date -r 1789423200  -> 2026-09-14 18:00:00 EDT   (round-trip)
#   TZ=UTC              date -r 1789423200  -> 2026-09-14 22:00:00 UTC
REUSE_GRACE_UNTIL=1789423200   # 2026-09-14 18:00:00 EDT -- round-trip above

step_reuse() {
    command -v reuse >/dev/null 2>&1 || {
        echo "reuse: UNMEASURED-NO-TOOL(reuse not on PATH -- inapplicable, not passing)" >&2
        return 2
    }
    local out rc
    out=$(reuse lint 2>&1); rc=$?
    local cov inv
    cov=$(printf '%s\n' "$out" | /usr/bin/sed -n 's/.*Files with copyright information: *//p' | head -1)
    inv=$(printf '%s\n' "$out" | /usr/bin/sed -n 's/.*Invalid SPDX License Expressions: *//p' | head -1)
    echo "reuse: covered ${cov:-?} - invalid-expressions ${inv:-?} - version $(reuse --version 2>&1 | head -1)"
    [ "$rc" -eq 0 ] || {
        local _now _until
        _now=$(date +%s)
        _until=$(date -r "$REUSE_GRACE_UNTIL" '+%Y-%m-%d %H:%M %Z' 2>/dev/null) || _until="the WORD 5 deadline"
        printf '%s\n' "$out" | /usr/bin/sed -n '/^# MISSING COPYRIGHT/,/^# SUMMARY/p' >&2
        if [ "$_now" -lt "$REUSE_GRACE_UNTIL" ]; then
            echo "reuse: NOT-COMPLIANT(rc=$rc) -- NON-BLOCKING until $_until by OPERATOR WORD 5, Q-D62 (a). THIS IS NOT A PASS." >&2
            return 0
        fi
        echo "reuse: NOT-COMPLIANT(rc=$rc) -- the WORD 5 grace window closed at $_until; this step blocks." >&2
        return 1
    }
    return 0
}

step_i18n_audit() {
    # O-I18N's audit script (invocation-only wiring per the O-INT ruling);
    # quiet on pass, full report on failure.
    local out
    if out="$(python3 i18n/tools/audit.py 2>&1)"; then
        return 0
    fi
    printf '%s\n' "$out"
    return 1
}

# MAJOR-25(b): the shell-behaviour leg. The comparator has been on main
# since 2026-08-23 in scripts/battery_guards.sh AND WAS CALLED BY NOTHING —
# 0 callers against 36 references to this file. A guard that is present but
# unwired is indistinguishable, AT THE GATE, from a guard nobody wrote.
# This step is the wiring. Operator ruling it serves: "G-5 must be the last
# rendering regression a green gate misses."
#
# Two independent facts, both floored:
#   tour_ok      the 32-frame screenshot tour must be DETERMINISTIC (run A
#                vs run B) and FAITHFUL (run A vs the committed goldens in
#                artifacts/tour-chrome).
#                ⛔ THIS COMMENT HAS NOW DRIFTED TWICE, AND THE SECOND TIME WAS
#                MINE. It read "24-frame" until 2026-08-31, when the Q-D9 re-pin
#                moved the floor to 31; I corrected the headline number that day
#                AND LEFT "Floor 24" standing three lines below it -- a second
#                copy of the same value, in the block that complains about a
#                number not moving with its subject. Both are corrected here.
#                ★ The number the gate ENFORCES is the literal in the tour_ok
#                call below, never this prose. Two numbers in one comment is one
#                number too many; a successor moving the floor must grep this
#                whole block, not read its first line.
#                DATED RECORD, kept because it stays true of its own date:
#                floor 24 = the golden count MEASURED 2026-08-23, not carried
#                from an older baseline. 24 -> 31 (2026-08-30 re-pin, 7 frames
#                added) -> 32 (2026-09-01, the discard-confirm frame).
#   partition_ok the declared ctest suite set must not silently shrink.
#                Floor 34 = the declared count MEASURED 2026-08-23 in the
#                same change, per the standing "re-measure, do not carry"
#                instruction attached to this item.
#
# ⚠ AN UNBUILT SHELL IS A FAILURE, NOT A PASS. If target/shell-build has no
# binary this step reds with UNMEASURED-NO-SHELL rather than skipping. That
# is deliberate and it is the whole lesson of gate-vacuity #1: a leg that
# cannot run must not return the same value as a leg that ran and found
# nothing. If a host legitimately cannot build the shell, that is a CI
# configuration fact to surface, not to hide behind a green.
# RELINK, AS A VISIBLE STAGE (operator ruling 2026-08-26, "A").
# gate.sh's step 1 is `cargo build --workspace`, which re-makes
# libappname_cabi.dylib. The shell LINKS that dylib and the gate did not build
# the shell -- so the pre-built shell required by ruling 7's :264 amendment was
# stale before step_shell_behavior ever ran, by construction, on every run
# where crates/ changed. The tour then rendered with a stale binary and its
# output was attributed to the tree.
#
# THIS IS A VISIBLE STAGE, NOT A LINE INSIDE step_shell_behavior. Hiding it
# inside the step is how the staleness became invisible in the first place.
#
# TWO GUARDS ON "the gate builds its own subject", per the ruling:
#   1. IT FAILS LOUDLY. A relink error is rc 1 -- a gate FAIL -- never
#      skip-and-continue. The gate does not proceed to draw tour conclusions
#      from a binary it could not produce.
#   2. INCREMENTAL ONLY, NO CONFIGURE. `cmake --build` on an already-configured
#      dir relinks; it does not configure. A broken build CONFIGURATION still
#      fails where it should -- in the real build lane -- rather than being
#      papered over here. A missing or unconfigured build dir is UNMEASURED
#      (rc 2), not a pass.
#
# And shell_fresh_ok in step_shell_behavior is the CROSS-VERIFIER: this stage
# claims to have produced a current binary, and the next stage measures whether
# it did. The relink and the check verify each other; neither is trusted alone.
step_shell_relink() {
    local bdir=target/shell-build
    [ -d "$bdir" ] || { echo "shell-relink: UNMEASURED-NO-BUILD-DIR($bdir)" >&2; return 2; }
    [ -f "$bdir/build.ninja" ] || [ -f "$bdir/Makefile" ] || {
        echo "shell-relink: UNMEASURED-NOT-CONFIGURED($bdir -- incremental only, this step does NOT configure)" >&2
        return 2; }
    local _out
    # ★★★★★ BUILDS EVERY TARGET, NOT JUST appname-shell. FIXED 2026-08-29.
    #
    # Until now this said `--target appname-shell`, so the gate built the
    # APP and then ran ctest over test binaries NOTHING HAD BUILT. Measured
    # on the df638143 run: all 43 test binaries in this directory were
    # older than the newest shell source -- tst_palette_pins was SEVEN
    # HOURS stale -- and the suite added that same night had never been
    # compiled here at all, so ctest reported it "Not Run".
    #
    # ★★★ THE CONSEQUENCE IS THE WHOLE POINT: A FAILING SUITE WAS A
    # PHANTOM AND A PASSING ONE PROVED NOTHING. tst_palette_pins "failed"
    # against a fresh catalog with a stale binary's expectation, and the
    # operator-pinned tst_import_filter result said nothing about whether
    # the day's ingress work had closed it, because that binary predated
    # the work. A gate that tests binaries it does not build is not
    # measuring the tree it names in its own certificate.
    #
    # The step is still INCREMENTAL and still does NOT configure; dropping
    # `--target` builds the default target, which is every target this
    # directory is configured for.
    if ! _out=$(cmake --build "$bdir" 2>&1); then
        echo "shell-relink: FAILED (incremental build of target/shell-build)" >&2
        printf '%s\n' "$_out" | tail -8 >&2
        return 1
    fi
    echo "shell-relink: OK"
}

step_quarantine() {
    # QUARANTINE REGISTER (operator's word, 2026-08-28). A quarantined test's failure is
    # CAPTURED AND TOLERATED PENDING CLASSIFICATION -- not an expected-fail, which has a
    # known cause and a recorded decision.
    #
    # PREDICTIONS REGISTERED BEFORE THIS EXISTED (INTEGRATION-log 241.1):
    #   Q-P1 prints QUARANTINED: <n> on EVERY run, INCLUDING n=0. A step that printed only
    #        when the register is non-empty would be INDISTINGUISHABLE FROM A STEP THAT DID
    #        NOT RUN, which is why the operator specified "every run".
    #   Q-P2 NEVER changes GATE_RC. run_step marks FAIL on any non-zero, so this function
    #        RETURNS 0 ON EVERY PATH -- including a missing or unreadable register, which
    #        is reported as a value and not as a failure. NON-BLOCKING IS A PROPERTY OF THE
    #        EXIT CODE, NOT AN INTENTION IN A COMMENT.
    #   Q-P3 an UNCLASSIFIED row is COUNTED AND NAMED. A count whose purpose is to block a
    #        tag is useless if the next reader cannot tell WHICH row to classify.
    #   Q-P4 THE TAG BLOCK IS RECORDED HERE, NOT ENFORCED HERE. This step has NO tag
    #        authority and must never acquire one: row zero's order is PASS first, word
    #        second, and a gate step that could refuse a tag would be a third key nobody
    #        granted. IT PRODUCES A FACT; THE PRE-TAG REVIEW CONSUMES IT.
    local reg=reports/QUARANTINE.md n u names
    if [ ! -r "$reg" ]; then
        echo "quarantine: QUARANTINED: 0 · REGISTER-ABSENT($reg) -- reported, not failed (Q-P2)"
        return 0
    fi
    n=$(/usr/bin/grep -c '^QROW ' "$reg" || true)
    u=$(awk -F'|' '/^QROW/ && $3 ~ /^[[:space:]]*$/ {c++} END{print c+0}' "$reg")
    names=$(awk -F'|' '/^QROW/ && $3 ~ /^[[:space:]]*$/ {gsub(/^ +| +$/,"",$2); printf "%s ", $2}' "$reg")
    echo "quarantine: QUARANTINED: $n · unclassified: $u"
    if [ "$u" -gt 0 ]; then
        # Q-P3: NAMED, not merely counted.
        echo "quarantine: UNCLASSIFIED-ROWS: ${names% }"
        echo "quarantine: an unclassified row BLOCKS THE NEXT APPNAME TAG -- a fact for the"
        echo "quarantine: pre-tag review to consume. THIS STEP DOES NOT AND MUST NOT ENFORCE IT."
    fi
    return 0
}


# P1-3 (operator ruling, 2026-08-28): A RELEASE SHELL MUST LOAD A RELEASE ENGINE.
#
# The reported symptom -- "performance is shit" in the release shell -- had this as
# its prime suspect, and MEASUREMENT DISPROVED IT: target/shell-release/appname-shell
# links target/release/deps/libappname_cabi.dylib, at an ABSOLUTE install path with
# no rpath indirection, so dyld has no substitution to make. The debug dylib's fresh
# mtime that made the suspicion plausible was a concurrent gate run rebuilding debug.
#
# The guard is added ANYWAY, and the reason is the operator's: the class can recur
# even though this instance is clean. A release shell silently bound to a debug
# engine is invisible in every artifact anyone looks at -- both dylibs exist side by
# side, both are named libappname_cabi.dylib, and only the LINK distinguishes them.
#
# ★ INAPPLICABLE IS NOT PASSING. If no release shell has been built, this step must
# say so rather than return 0 -- a check that silently passes when its subject is
# absent is the A21 shape, and it is why the panic arm below exists at all.
#
# ⛔ v2 (O-INT, OPERATOR WORD 14 (3), 2026-09-17; findings #837 #838 #840, entries
# 1002-1004 1022). BOTH HALVES, because either alone is wrong:
#   (1) EXIT CODE. v1 printed "inapplicable, not passing" and then `return 0`, and
#       run_step reads the code, so an absent subject was tallied PASS -- the exact
#       shape the comment above forbids. Absent now returns 2 -> UNMEASURED.
#   (2) PATH. v1 named target/shell-release/appname-shell. The only `cmake -S shell`
#       in the tree (build_dmg.sh:297) builds with -DAPPNAME_BUNDLE=ON, so the release
#       executable lives INSIDE a .app. A clean tree never had v1's path (green by
#       absence); a desk with a leftover from an older build convention had a stale
#       one (green by a 12-day-old artifact). Fixing (1) alone would turn both into a
#       PERMANENT UNMEASURED. The subject is now discovered exactly as build_dmg.sh:305
#       discovers it, and the executable read from the bundle's own CFBundleExecutable.
# Measured before changing it: the build-dir .app binary carries the absolute
# .../target/release/deps/libappname_cabi.dylib load command (build_dmg.sh rewrites to
# @executable_path only on the STAGED copy), so the /debug/ vs /release/ arms below
# still classify it. NOT addressed, and stated: FRESHNESS. A present-but-stale .app is
# still measured as-is; this step checks the link, not the build's age.
step_release_engine_bind() {
    local app exe shell_bin
    app=$(find target/shell-release -maxdepth 2 -type d -name '*.app' 2>/dev/null | head -1)
    if [ -z "$app" ]; then
        echo "release-engine-bind: NOT-BUILT(no *.app under target/shell-release -- UNMEASURED, not passing)"
        return 2
    fi
    exe=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$app/Contents/Info.plist" 2>/dev/null || true)
    shell_bin="$app/Contents/MacOS/$exe"
    if [ -z "$exe" ] || [ ! -x "$shell_bin" ]; then
        echo "release-engine-bind: NOT-BUILT($app has no executable CFBundleExecutable -- UNMEASURED, not passing)"
        return 2
    fi
    local links
    links=$(otool -L "$shell_bin" 2>/dev/null | grep 'libappname_cabi') || {
        echo "release-engine-bind: NO-CABI-LINK($shell_bin links no libappname_cabi -- the shell cannot be running the engine)" >&2
        return 1
    }
    if printf '%s\n' "$links" | grep -q '/debug/'; then
        echo "release-engine-bind: DEBUG-ENGINE-IN-RELEASE-SHELL" >&2
        printf '%s\n' "$links" >&2
        return 1
    fi
    if ! printf '%s\n' "$links" | grep -q '/release/'; then
        echo "release-engine-bind: UNRECOGNISED-ENGINE-PATH(neither /debug/ nor /release/ -- classify before trusting)" >&2
        printf '%s\n' "$links" >&2
        return 1
    fi
    echo "release-engine-bind: OK(release shell -> release engine)"
    return 0
}

step_shell_behavior() {
    # C-410 RESIDUAL (O-UI, 2026-08-28): the stamp was minted once but THE SHA WAS STILL
    # DERIVED TWICE -- :529 fell back to the literal UNKNOWN while :566's tree_sha fell
    # back to a LIVE `git rev-parse`. Same defect C-410 was filed for, in the same two
    # lines, in the OTHER field: two artifacts of one run could carry DIFFERENT sha
    # components. Unreachable today (GATE_START_SHA is set before any run_step and this
    # function has no external caller) -- THEY AGREED BY CONSTRUCTION, NOT BY EXPRESSION.
    # ONE derivation, both consumers, so the agreement is now structural.
    local run_sha
    run_sha="${GATE_START_SHA:-$(git rev-parse --short HEAD 2>/dev/null || echo UNKNOWN)}"
    local sh=target/shell-build/appname-shell
    [ -x "$sh" ] || { echo "shell-behavior: UNMEASURED-NO-SHELL($sh not built)" >&2; return 2; }

    . scripts/battery_guards.sh >/dev/null 2>&1 || {
        echo "shell-behavior: UNMEASURED-NO-GUARDS(scripts/battery_guards.sh did not source)" >&2; return 2; }

    # RUN THE GUARDS' OWN SELFTEST BEFORE TRUSTING THE GUARDS.
    # battery_guards.sh's header says "a guard you have not watched refuse is
    # not in service" and then NOTHING INVOKED guards_selftest -- three
    # references in the whole repo, all of them its own definition or prose
    # about it. By its own standard none of its guards were in service.
    # Found by O-INT's landed-but-unconsumed census under the operator's new
    # rule: instrument review asks "is it correct?" AND "what consumes it?".
    # This is the consumer.
    local _gs
    if ! _gs=$(guards_selftest 2>&1); then
        echo "shell-behavior: UNMEASURED-GUARDS-SELFTEST-FAILED" >&2
        printf '%s\n' "$_gs" | sed -n 's/^  FAIL/    guard-selftest FAIL/p' >&2
        return 2
    fi

    # FRESHNESS OF THE RUNNER, before it is trusted to render the tree.
    # Existence was already checked; staleness was not, and a stale runner's
    # output was attributed to the tree under test -- turning an innocent
    # golden re-pin into a NO-ARTIFACT-DIFFER that read as a regression.
    # shell_fresh_ok lives in battery_guards.sh so it inherits guards_selftest:
    # a guard nobody has watched refuse is not in service. (O-INT's review.)
    local _root _bdir _fresh
    _root=$(git rev-parse --show-toplevel 2>/dev/null) || _root=""
    # ABSOLUTE build dir. ninja emits absolute input paths, so a RELATIVE
    # build dir here makes the exclusion a silent no-op -- it matched nothing
    # and every generated input counted as a source. Caught end-to-end
    # (nsrc=88 where the closure has 59 sources); the selftest did NOT catch
    # it, because its synthetic paths were absolute on both sides.
    _bdir=$(cd "$(dirname "$sh")" 2>/dev/null && pwd) || _bdir=""
    _fresh=$( (cd "$(dirname "$sh")" && ninja -t inputs "$(basename "$sh")" 2>/dev/null) \
              | shell_fresh_ok "$sh" "$_root" "$_bdir" )
    case "$_fresh" in
        YES*) echo "shell-behavior: runner $_fresh" ;;
        *)    echo "shell-behavior: runner $_fresh" >&2; return 2 ;;
    esac

    # CALLER-SIDE NON-VACUITY (O-WATCH's requirements 3 and 4). The selftest
    # exercises shell_fresh_ok with SYNTHETIC inputs that are absolute on both
    # sides -- well-formed by construction -- so it CANNOT see a caller that
    # passes a path shape the exclusion never matches. That is exactly the bug
    # that shipped in v2: a RELATIVE build dir against ABSOLUTE ninja paths,
    # the filter matched nothing, and it reported a clean YES.
    #
    # This binary's build dir demonstrably contains graph inputs (moc/autogen
    # output). So excl=0 here does not mean "nothing to exclude" -- it means
    # THE FILTER DID NOT MATCH, and the nsrc figure above is inflated with
    # generated files. Refuse rather than report a number built on a dead
    # filter.
    case "$_fresh" in
        *excl=0\)) echo "shell-behavior: UNMEASURED-EXCLUSION-VACUOUS($_fresh -- build dir holds inputs, so the filter matched nothing)" >&2; return 2 ;;
    esac

    local work rc=0
    work=$(mktemp -d) || { echo "shell-behavior: UNMEASURED-NO-TMPDIR" >&2; return 2; }
    "$sh" --screenshot-tour "$work/A" >/dev/null 2>&1
    "$sh" --screenshot-tour "$work/B" >/dev/null 2>&1

    local tour part declared
    # ★★★★★ FLOOR PINNED EXACT AT 31 ON THE OPERATOR'S WORD, 2026-08-30.
    # I argued a floor equal to its own population "can never fire". HIS RULING
    # CORRECTS THE FRAMING AND IS RIGHT: "a completeness pin is supposed to trip
    # consciously on every future addition, that's its job."
    # ★★★★ It is a COMPLETENESS pin, not a loss tripwire. Tripping on the next
    # added frame is the intended behaviour, not a defect -- whoever adds frame
    # 32 must move this number deliberately and say why.
    # ★★★ Was 24, matching the golden count set by RULING 3 on 2026-08-25.
    # Now 31, matching the re-pin of 2026-08-30 (23 changed + 1 identical +
    # 0 orphaned = 24, plus 7 new = 31).
    # 31 -> 32 ON 2026-09-01, DELIBERATELY, WHICH IS WHAT THE OPERATOR'S RULING
    # REQUIRES OF WHOEVER MOVES IT: the discard-confirm dialog shipped with
    # standing priority (4)-3 and had ZERO photographic coverage -- a
    # user-visible surface whose strings, had they failed to resolve, would
    # render as raw MessageIds that throw nothing and that no assertion in
    # tst_dirty_flag can name. The frame is purely ADDITIVE: measured before
    # copying, new=1 changed=0 orphaned=0, and runs A and B were byte-identical
    # with the modal open. INTEGRATION-log 455 carries the why.
    # 32 -> 39 ON 2026-09-10 (O-UI, C-756), DELIBERATELY AND WITH THE WHY THE
    # OPERATOR'S RULING REQUIRES -- and the first half of this is MY OWN
    # OMISSION BEING PAID. At 3e8a46c20 I added FOUR frames (32-mask-panel and
    # three adjustment dialogs) taking the population 32 -> 36 and DID NOT
    # move this number. The ruling of 2026-08-30 is explicit that "whoever
    # adds frame 32 must move this number deliberately and say why", and the
    # convention had held exactly until then: 24/24, 31/31, 32/32. Four frames
    # rode in under a pin that could no longer trip on them.
    # NOW 36 -> 39: B-03's acceptance clause is "one tour frame each" for SIX
    # adjustment dialogs, all by 09-15; Exposure, Threshold and Curves were
    # the three without photographic coverage. Measured BEFORE copying, on the
    # same build, against the run that preceded the change: changed=0, new=3,
    # orphaned=0 -- purely additive -- and runs A and B were byte-identical
    # both before and after.
    # 39 -> 40 ON 2026-09-10 (O-UI, C-759), the ruling's deliberate move
    # again: B-09's DONE-WHEN is "About shows all", VERIFIED-BY an O-WATCH
    # READ, and until now there was nothing to read but source. Measured
    # before copying, same build, against the run that preceded the step:
    # changed=0, new=1, orphaned=0, with runs byte-identical both sides.
    # ★ The frame BLANKS the build-commit row: AboutDialog renders
    # "Build <sha>-dirty", which changes every commit, so a golden holding
    # it could never match a fresh run again and would break this very arm
    # permanently one commit after any re-pin. Found by looking at the first
    # capture. The row's rendering stays covered by tst_about_render.
    # ★★★★★ MEASURED 2026-09-11 (O-UI, C-790), AND IT CORRECTS THE MECHANISM
    # DESCRIBED ABOVE WITHOUT TOUCHING THE RULING. The line above reads "It is
    # a COMPLETENESS pin, not a loss tripwire." That is the OPERATOR'S INTENT
    # and it stays printed. The IMPLEMENTED BEHAVIOUR is the other way round,
    # measured by driving tour_ok with synthetic directories rather than by
    # reasoning about it:
    #
    #   40/40/40, floor 40                    -> YES
    #   run 41, goldens 40 (addition uncopied) -> NO-ARTIFACT-DIFFER  <- the DIFF
    #   41/41/41, floor still 40               -> YES                 <- floor need not move
    #   39/39/39, floor 40                     -> UNMEASURED-BELOW-FLOOR <- the FLOOR
    #
    # So what trips on an ADDITION is `diff -r A artifacts`, which refuses until
    # the new golden is deliberately copied in -- that is where the conscious
    # move actually lives. The FLOOR catches LOSS. Both halves of the ruling are
    # honoured by the step as a whole; the sentence just names the wrong arm.
    # ★★★★ WHY THIS MATTERS TO A SUCCESSOR RATHER THAN BEING PEDANTRY: read as
    # printed, the comment says a committed addition will trip this number, so
    # someone who adds frame 41, copies the golden and sees GREEN concludes the
    # pin is broken. It is not. Moving the literal is a CONVENTION that keeps
    # the floor equal to the population so a future loss is caught at the right
    # height -- which is why every move above is recorded with its why.
    # ★★★ AMENDMENT 8 (c) OF 2026-09-11, "EVERY LITERAL COUNT BECOMES A
    # RELATION", extends Q-D49 (b) to tour_ok as "every golden matches; the
    # count is derived from the tree at the gate and is never lower than the
    # previous landing's". CHECKED AGAINST THIS CALL AND IT ALREADY CONFORMS:
    # tour_ok derives A, B and ART with `ls | wc -l` AT THE GATE and reports
    # all three; "every golden matches" is the `diff -r A artifacts` arm, exact
    # and unconditional; and the literal below IS the previous landing's count
    # (40 = the population at the last re-pin). I expected a conflict with the
    # 2026-08-30 ruling and went looking for one; the measurement says there is
    # none, so nothing here changes and no question is filed.
    # 40 -> 35 ON 2026-10-11 (O-UI, O-WATCH F-7), DELIBERATELY AND WITH THE
    # WHY: corrigendum (l) removed Text, Image > Mode, Warp, Puppet Warp and
    # Liquify from 1.0, so the five frames that photographed them through
    # engine commands (16-text, 18-mode-convert, 20-warp, 21-puppet,
    # 23-liquify) left tour.cpp with them -- a golden of a state no 1.0
    # control can reach would show the release doing what a user cannot.
    # This is a LOSS by ruling, so the floor moves down with it.
    tour=$(tour_ok "$work/A" "$work/B" artifacts/tour-chrome 35)
    case "$tour" in
        YES*) echo "shell-behavior: tour $tour" ;;
        # ★★★★★ PRESERVE THE EVIDENCE, BECAUSE THIS STEP DESTROYED IT ONCE AND
        # THE INVESTIGATION DIED WITH IT. On 2026-09-01 the fidelity arm returned
        # NO-ARTIFACT-DIFFER inside a gate; the same tree then passed twice, and
        # ten hand re-runs (four idle, six concurrent) reproduced nothing. The
        # differing PNG had existed, in $work, for the length of one `case`
        # statement -- and every path below this point does `rm -rf "$work"`.
        #
        # ★★ NAMING THE MEMBER WAS HALF THE FIX (battery_guards.sh now does).
        # THIS IS THE OTHER HALF: a name tells you WHICH file, and only the file
        # itself tells you WHAT CHANGED -- a wrong colour, a stale label, a
        # half-drawn modal all render, and the instrument for that is a person
        # looking at a picture.
        #
        # ★ Copies rather than skipping the cleanup: the rm's live on several
        # exit paths and re-plumbing them all to conditionally spare $work is a
        # bigger change than this warrants. A failed run costs a few MB, kept
        # beside the gate's other retained logs; a passing run costs nothing.
        *)    echo "shell-behavior: tour $tour" >&2
              _tkeep="target/shell-build/gate-tour-fail-${GATE_START_SHA:-UNKNOWN}-${GATE_START_STAMP:-UNKNOWN}"
              if [ -d "$work/A" ] || [ -d "$work/B" ]; then
                  mkdir -p "$_tkeep" 2>/dev/null \
                    && cp -R "$work/A" "$work/B" "$_tkeep"/ 2>/dev/null \
                    && echo "shell-behavior: tour EVIDENCE KEPT -> $_tkeep (compare against artifacts/tour-chrome)" \
                    || echo "shell-behavior: tour EVIDENCE-KEEP-FAILED($_tkeep)" >&2
              else
                  echo "shell-behavior: tour NO-EVIDENCE-TO-KEEP(no run dirs -- the verdict is about the count, not the pixels)"
              fi
              rc=1 ;;
    esac

    # Capture the NAMES and derive the count from them -- ONE `ctest -N`, not two.
    # The names are needed later for the pin presence-precondition, and that check
    # must NOT invoke `ctest -N` itself: A21 -- `ctest -N` TRUNCATES
    # Testing/Temporary/LastTest.log, so a presence probe run AFTER the suites
    # would destroy the only record of the run that just executed. Taking the
    # names from BEFORE the run costs nothing and cannot destroy anything.
    local _declared_names
    _declared_names=$(ctest --test-dir target/shell-build -N 2>/dev/null \
                 | sed -n 's/^  Test *#[0-9]*: *//p' | sort -u)
    declared=$(printf '%s\n' "$_declared_names" | grep -c . || true)

    # C-354 — OPERATOR RULING, 2026-08-26: "A SUITE THE GATE NAMES, THE GATE RUNS."
    # The Qt suites are MANDATORY on every run that feeds a certification, a
    # merge, or anything O-WATCH signs; the ~12 min is accepted cost. Tiering is
    # permitted for quick iteration ONLY with an explicit UNMEASURED verdict
    # PRINTED -- never silently, and never on a certifying run.
    #
    # This replaces the C-339 tautology. partition_ok's signature is
    # (RANSUM, declared, floor) and it now gets a REAL ransum, extracted by
    # `ctest_ran` -- which was written for exactly this and had zero call sites
    # in this file until now. Its "No tests were found!!!" arm matters: ctest
    # EXITS 0 on a name that matches nothing (battery_guards.sh:183), which is
    # how tst_g5_frameprobe sat unregistered and unnoticed (C-353).
    #
    # It printed "shell-behavior: suites YES(38/38)" on 2026-08-26 while
    # ctest on the SAME build dir measured 37/38 -- a real red the line read
    # as a pass. A reader reads the report, not the inputs (C-207), so the
    # report now says which of the two it measured and states the gap
    # (G-25). WHETHER THE GATE SHOULD EXECUTE THE SUITES is a desk question
    # with a real cost (~12 min of Qt suites per gate run) and is routed,
    # not decided here.
    # ── LEG A: REGISTRATION COMPLETENESS (operator ruling 2026-08-26) ──
    # Lands BEFORE execution is wired, and the order is load-bearing. The gate
    # NAMES suites with `ctest -N`; a BUILT-but-unregistered binary is absent
    # from that naming set, so execution alone would run every named suite,
    # pass, and never touch the missing one. And once execution is wired the
    # direction of harm INVERTS: `NOT-RUN` is honest (a reader sees a hole),
    # while a green over an incomplete naming set HIDES the absence.
    # Checks BOTH directions. DOCUMENTED_EXCLUSIONS must name its expiry
    # condition beside each entry (G-124), or an exclusion outlives its reason.
    local _regx _regdelta
    # tst_perf_probes: bench-class, wall-clock numbers flake the default suite;
    #   registered only under APPNAME_BENCH_CTEST. EXPIRES if it stops being
    #   timing-based. tst_g5_frameprobe is DELIBERATELY NOT LISTED HERE: its
    #   stated reason ("reds by design until the repaint scoping is corrected")
    #   EXPIRED when the scoping merged at 907ee4c5, so it must register.
    _regx="tst_perf_probes"
    _regdelta=$(ctest --test-dir "$_bdir" -N 2>/dev/null \
        | grep -oE 'Test +#[0-9]+: +[a-zA-Z0-9_]+' | awk '{print $NF}' \
        | registration_complete_ok "$_bdir" "$_regx" "shell/tests")
    case "$_regdelta" in
        YES*)        echo "shell-behavior: registration $_regdelta" ;;
        UNMEASURED*) echo "shell-behavior: registration $_regdelta" >&2; rm -rf "$work"; return 2 ;;
        *)           echo "shell-behavior: registration $_regdelta" >&2; rc=1 ;;
    esac

    # ── LEG B: EXECUTION (operator ruling 2026-08-26, "a suite the gate names,
    # the gate runs"). Ordered AFTER Leg A deliberately, and the order is the
    # whole point: wiring execution over an incomplete naming set INVERTS the
    # direction of harm -- NOT-RUN is honest because a reader sees a hole, while
    # a green over 38 executed suites hides the absence. Leg A above is what
    # makes this green mean what it says.
    # Authored by O-UI 6 on lane/ui. Their competing registration leg was REMOVED
    # by them, not disabled, after they read main's and measured it better on
    # both directions, population guards, shell-independence and diagnostics.
    # Merged with BOTH legs kept and registration FIRST. Rule 12: this goes to
    # O-WATCH before any run counts as certifying.
    # SCOPE, per the same ruling: a suite result offered toward the tag must name
    # the TREE, the BUILD DIR and the COMMIT. All three are printed below.
    # C-357: MY REGISTRATION-COMPLETENESS LEG IS REMOVED, NOT DISABLED.
    # O-INT #7 built the same thing independently and theirs is better on every
    # axis I could measure: BOTH directions (built-but-unregistered AND
    # registered-but-unbuilt, which found a real phantom on main --
    # tst_auto_adjust_menu, registered with no resolvable command, a class mine
    # could not see at all); population guards so a zero on either side reads
    # UNMEASURED rather than agreement; newline-separated lists joined explicitly
    # because space-separated ones depend on the CALLER'S shell word-splitting
    # (bash and zsh gave them different answers -- A4, and my version had that
    # exposure); two distinct phantom labels because the remedies differ; 35
    # selftest arms; twice through rule 12 with O-WATCH. It is on main already.
    # Mine was one revision old and mixed authorities -- filesystem for the built
    # side, ctest for the registered side -- which is what forced the orphan
    # split I had to add. Dropping it is not deference; it is the measurement.

    local tier="${APPNAME_GATE_TIER:-certify}"
    if [ "$tier" != "certify" ]; then
        echo "shell-behavior: UNMEASURED-SUITES-SKIPPED(APPNAME_GATE_TIER=$tier -- the Qt suites are mandatory on a certifying run; this verdict is NOT a pass)" >&2
        rm -rf "$work"
        return 2
    fi

    # EXPECTED-FAIL REGISTER. A suite held here does not red the gate, but the
    # hold is at SUITE granularity while the licence is at CASE granularity --
    # so holding a suite would hide any OTHER failure inside it. The check below
    # therefore verifies that the failing CASES in a held suite are exactly the
    # named ones; anything else reds. Entries carry owner and close condition
    # (G-122): a standing accepted failure is named IN the bar or the bar is
    # simply lower.
    #   RETIRED 2026-08-30 ON THE OPERATOR'S WORD -- THE REGISTER IS NOW EMPTY.
    #   The entry was:
    #     tst_import_filter :: everyAdvertisedExtensionIsIngestible
    #       operator-directed widening 2026-08-25 (third request); the code comment
    #       at shell/qml/Main.qml:1427 said it WILL red and must not be "fixed" by
    #       widening the accepted set. Closed with operator item 22 / engine P2.
    #
    #   ★★★ THE PIN DIED WITH ITS CAUSE, AND THE CAUSE WAS MEASURED BEFORE IT DIED.
    #   That comment's own predicate was "Codec::decode has ZERO callers in
    #   appname-cabi, appname-cli or shell". Measured 2026-08-30: THREE callers --
    #   appname-cabi/src/doc.rs:438, appname-cli/src/main.rs:993 and :1166. Ruling
    #   D5 wired the ingress, and all EIGHT advertised extensions were then driven
    #   through the real path one file at a time (png jpg jpeg tif tiff webp gif
    #   bmp -> all "Imported layer"), with a firing negative control (random bytes
    #   named .png -> error.command.bad_target).
    #   ★★ The line number in the old entry had ALSO drifted: Main.qml:1427 is now
    #   an unrelated Theme.warnText comment; the real one is :1602. A pin whose
    #   justification cites a moved line is a pin nobody can re-audit cheaply.
    #
    #   ★ THE MECHANISM BELOW IS LEFT WHOLE AND SIMPLY NOT ENGAGED. Re-arming is
    #   two assignments. Every guard is gated on a NON-EMPTY register, explicitly
    #   rather than by relying on `grep -x ""` happening to match nothing -- an
    #   empty-string pattern behaving correctly by accident is the same class of
    #   luck this file exists to refuse.
    local EXPECTED_SUITE=""
    local EXPECTED_CASE=""

    # FRESHNESS OF THE SUITES THEMSELVES -- rule 12, O-WATCH 3.
    #
    # Placed BEFORE the ctest call deliberately: you want to know the binaries are
    # current before you trust what they report, not after. On certifying run #1
    # EIGHT of forty test binaries were stale -- one missing the very case the
    # registration line named -- and the gate printed `suites-ran YES(39/39)`
    # while executing suites built from day-old code. `shell_fresh_ok` said
    # "runner YES" throughout, because it checks the RUNNER, not these forty.
    #
    # ★ THE GATE DETECTS A STALENESS IT CANNOT FIX. `step_shell_relink` builds
    # appname-shell ONLY; nothing in this gate builds the test binaries. A
    # STALE-SUITES red is correct and has NO IN-GATE REMEDY -- the responder must
    # go build them. Said here and printed in the message, because otherwise the
    # first person to hit it goes looking for a gate defect.
    local _fresh
    _fresh=$(suites_fresh_ok target/shell-build shell/tests)
    case "$_fresh" in
        YES*)        echo "shell-behavior: suites-fresh $_fresh" ;;
        UNMEASURED*) echo "shell-behavior: suites-fresh $_fresh" >&2; rm -rf "$work"; return 2 ;;
        *)           echo "shell-behavior: suites-fresh $_fresh -- NO IN-GATE REMEDY: this gate builds appname-shell only, never the test binaries; rebuild them (ninja -C target/shell-build) and re-run" >&2; rc=1 ;;
    esac

    local ctlog ransum ctrc
    ctlog=$(mktemp) || { echo "shell-behavior: UNMEASURED-NO-TMPFILE" >&2; rm -rf "$work"; return 2; }
    # `|| rc=$?` rather than `cmd; rc=$?`. NOT because errexit would abort here --
    # I wrote that explanation first and it is FALSE, retracted at C-355 before it
    # shipped: `run_step` invokes every step as `if "$@"; then`, a CONDITION
    # CONTEXT, which makes `set -e` inert for the whole dynamic extent of the
    # call. That is why the bare `grep …; rc=$?` lines in step_walled_symbols
    # have worked all along, and checking them is what falsified the theory.
    # The form below is used because it holds the status without DEPENDING on
    # that fact -- a step lifted into a plain call site would otherwise change
    # behaviour silently.
    ctrc=0
    ctest --test-dir target/shell-build --output-on-failure > "$ctlog" 2>&1 || ctrc=$?

    # C-386: PRESERVE CTEST'S OWN LOG, HERE, BEFORE ANYTHING ELSE RUNS CTEST.
    # $ctlog is what the GATE PRINTED, and --output-on-failure prints per-suite
    # output only for FAILURES. The complete per-suite record -- every passing
    # suite included -- is ctest's LastTest.log, and the hold re-run below
    # overwrites it minutes later. On the 10:46:53 run 6350 bytes of $ctlog were
    # retained and 50646 bytes of LastTest.log were lost. (O-WATCH 3 measured both.)
    #
    # Retaining $ctlog faithfully is not the same as retaining the right object:
    # cmp proves the copy matches its source, and says nothing about whether the
    # source was what a stranger needs. Two files, two purposes -- what the gate
    # CONCLUDED, and what the suites SAID.
    local detail_src detail_keep detailnote
    detail_src="target/shell-build/Testing/Temporary/LastTest.log"
    detail_keep="target/shell-build/gate-suites-detail-${run_sha}-${GATE_START_STAMP:-UNKNOWN}.log"
    if [ ! -s "$detail_src" ]; then
        detailnote="DETAIL-ABSENT($detail_src is missing or empty -- ctest wrote no per-suite log)"
    elif ! cp "$detail_src" "$detail_keep" 2>/dev/null; then
        detailnote="DETAIL-RETAIN-FAILED(could not copy $detail_src)"
    elif ! cmp -s "$detail_src" "$detail_keep"; then
        detailnote="DETAIL-RETAIN-CORRUPT($detail_keep differs from $detail_src)"
    else
        detailnote="detail $detail_keep"
    fi

    ransum=$(ctest_ran < "$ctlog")
    part=$(partition_ok "$ransum" "$declared" 34)
    # ★★★★ `partition` COUNTS SUITES ctest ENUMERATED, NOT SUITES THAT RAN.
    # O-WATCH #322, traced to source: ctest_ran (battery_guards.sh:142)
    # extracts the DENOMINATOR of "N tests failed out of M" -- M is what
    # ctest was ASKED to run. A suite whose executable does not exist is
    # reported "(Not Run)" and still sits INSIDE that M.
    #
    # ★★★ SO ON THE df638143 RUN THE SIDECAR SAID `partition YES(43/43)`
    # WHILE tst_layers_blend_row HAD NO EXECUTABLE. The function is doing
    # its documented job -- "the declared ctest suite set must not silently
    # shrink" -- and by that purpose it is CORRECT. What misleads is the
    # field sitting beside `ctest_rc` in a provenance sidecar, where a
    # reader takes it for "all suites ran". O-WATCH read it that way and
    # cited it as completeness at their #304; so did I.
    #
    # ★★ ADDING A FIELD RATHER THAN RENAMING ONE, which is this file's own
    # standing instruction (battery_guards.sh: "add a field instead of
    # relying on the shape"). `partition` keeps its meaning and its
    # parsers; `executed` and `not_run` say the thing it was being read as.
    _notrun=$(sed -n 's/^[[:space:]]*[0-9]* - [A-Za-z0-9_]* (Not Run)$/x/p' "$ctlog" | grep -c . || true)
    _executed=$(( ransum - _notrun ))

    local tree_sha tree_now treenote
    tree_sha="$run_sha"
    tree_now=$(git rev-parse --short HEAD 2>/dev/null || echo UNKNOWN)
    if [ "$tree_now" = "$tree_sha" ]; then
        treenote=""
    else
        # Not a failure: the suites ran against the tree as it was built. But the
        # result must not be filed under a commit that did not exist at the time.
        treenote=" · TREE-MOVED-DURING-RUN(started $tree_sha, HEAD is now $tree_now -- this result is filed under the START commit)"
    fi

    # C-381: RETAIN THE FULL-RUN EVIDENCE. $ctlog is an mktemp and line ~586
    # deletes it unconditionally, so on every certifying run the only complete
    # record of what the 39 suites did was destroyed at the end of the step --
    # including the runs whose partition or expected-fail verdict someone would
    # later want to re-read. The hold re-run no longer clobbers it (C-357 gave
    # that its own $holdlog), but deletion alone was enough to lose it.
    #
    # NEVER PRINT A PATH THAT IS NOT THERE (G-119: the output line is part of
    # the guard). If the copy fails, the line says RETAIN-FAILED and names the
    # errno, because a path that does not resolve is worse than no path: it
    # reads as evidence to anyone who does not try to open it.
    local keep keepnote prov provnote gatesha
    keep="target/shell-build/gate-suites-${tree_sha}-${GATE_START_STAMP:-UNKNOWN}.log"
    if [ ! -s "$ctlog" ]; then
        # The copy is not the problem here and must not be blamed for it: ctest
        # ran and produced NOTHING. That is a louder fact than a failed cp.
        keepnote="RETAIN-EMPTY(ctest wrote no output to $ctlog -- there was nothing to retain, which is itself a red)"
    elif ! cp "$ctlog" "$keep" 2>/dev/null; then
        keepnote="RETAIN-FAILED(could not copy $ctlog to $keep -- this run's suite output is NOT recoverable)"
    elif ! cmp -s "$ctlog" "$keep"; then
        # -s would pass here. Non-empty is not faithful: verify the artifact
        # against the thing it claims to be, not against a property it shares.
        keepnote="RETAIN-CORRUPT($keep differs from $ctlog -- do not read it as this run's output)"
    else
        keepnote="log $keep"
        # G-743 (O-WATCH 3, 2026-08-28): A FILENAME'S SHA IS THE PRODUCER'S
        # ASSERTION ABOUT WHICH TREE WAS TESTED, NEVER A MEASUREMENT OF IT.
        # $tree_sha is computed here and echoed to STDOUT, which nobody keeps
        # -- so a retained log's ONLY tie to its subject was its own name.
        # A complete 39-suite run was found this way and nearly used to narrow
        # a row-zero auditability exclusion it could not testify to.
        #
        # SIDECAR, NOT A TRAILER: $keep is verified byte-identical to $ctlog
        # three lines above, and appending to it would falsify the one property
        # that check establishes. The provenance goes BESIDE the artifact.
        #
        # RECORDS THE GATE ITSELF, not only the tree. scripts/gate.sh moved
        # today (12 steps -> 13) and two reviews were spent establishing that a
        # certificate must name WHICH gate produced it. An artifact that names
        # its tree and not its instrument answers half the question.
        gatesha=$(git hash-object scripts/gate.sh 2>/dev/null) || gatesha=""
        [ -n "$gatesha" ] || gatesha="UNMEASURED-GIT-HASH-OBJECT-FAILED"
        prov="${keep}.provenance"
        if printf 'tree_sha %s\ntree_at_end %s\nstamp %s\nctest_rc %s\npartition %s\nenumerated %s\nexecuted %s\nnot_run %s\ngate_sh_blob %s\nartifact %s\n' \
                "$tree_sha" "$tree_now" "${GATE_START_STAMP:-UNKNOWN}" "$ctrc" "$part" \
                "$ransum" "$_executed" "$_notrun" "$gatesha" "$keep" \
                > "$prov" 2>/dev/null; then
            provnote=" · provenance $prov"
        else
            # G-119 again: never print a path that is not there.
            provnote=" · PROVENANCE-FAILED(could not write $prov -- $keep cannot testify to its own subject)"
        fi
        keepnote="$keepnote$provnote"
    fi

    # ★ SAID ON THE HUMAN LINE TOO, not only in the sidecar: the sidecar is
    # read when something has already gone wrong, and this is the number a
    # reader takes for "the suites ran" while skimming a green gate.
    if [ "${_notrun:-0}" -gt 0 ]; then
        # NO BACKTICKS. The first draft of this line wrapped the word
        # partition in backticks for emphasis INSIDE a double-quoted string,
        # so the shell ran it as a command: "command not found" on stderr and
        # the word itself REPLACED BY NOTHING, leaving "-- counts them as
        # present" with no subject. G-512, in a diagnostic whose whole job is
        # to stop a field being misread.
        echo "shell-behavior: NOT-RUN($_notrun suite(s) enumerated but never executed -- the partition field counts them as present)" >&2
    fi
    echo "shell-behavior: suites-ran $part [enumerated $ransum · executed $_executed · not-run $_notrun] · tree $(pwd) · build target/shell-build · commit $tree_sha · ctest rc=$ctrc · $keepnote · $detailnote$treenote"
    case "$part" in
        YES*) : ;;
        *)    echo "shell-behavior: suites $part" >&2; rc=1 ;;
    esac

    if [ "$ctrc" -ne 0 ]; then
        local failed unexpected_suites failing_cases
    # ★★★★★ MATCHES EVERY TERMINAL STATUS, NOT JUST "(Failed)". FIXED 2026-08-29.
    #
    # This pattern used to end `(Failed)$`, so ctest's OTHER terminal
    # statuses were invisible -- "(Not Run)", "(Timeout)", "(Subprocess
    # aborted)", "(Exception)". On the df638143 run ctest's own summary
    # listed THREE failures and this extraction reported TWO: the missing
    # one was `tst_layers_blend_row (Not Run)`, a suite whose executable
    # did not exist.
    #
    # ★★★ A SUITE THAT COULD NOT RUN IS THE ONE GUARANTEED TO CATCH
    # NOTHING, AND IT WAS THE ONE STATUS THE GATE COULD NOT SEE. This is
    # "use a pattern that can match the thing you are counting" -- already
    # in the standing hazard list, from a step-list grep that could not
    # match `i18n-audit` -- recurring in the gate's own suite extraction.
    failed=$(sed -n 's/^[[:space:]]*[0-9]* - \([A-Za-z0-9_]*\) (\([A-Za-z][A-Za-z ]*\))$/\1/p' "$ctlog" | sort -u)
    # ★★ CROSS-CHECK AGAINST ctest'"'"'S OWN TALLY, IN THE SAME FILE.
    # The old extraction disagreed with a number printed a few lines above
    # it and NOTHING COMPARED THEM. A count is only evidence when a second,
    # independently-produced count can contradict it.
    _ct_claimed=$(sed -n 's/^.*[0-9]*% tests passed, \([0-9]*\) tests* failed out of [0-9]*.*$/\1/p' "$ctlog" | tail -1)
    _ct_extracted=$(printf '%s\n' "$failed" | grep -c . || true)
    if [ -n "$_ct_claimed" ] && [ "$_ct_claimed" != "$_ct_extracted" ]; then
        echo "shell-behavior: EXTRACTION-DISAGREES(ctest says $_ct_claimed failed, this step extracted $_ct_extracted -- a status this pattern cannot match)" >&2
        rc=1   # WIRED 2026-09-08 (entry 689 / O-WATCH #505): this
               # cross-check printed and continued for its whole life. A
               # second, independently-produced count that cannot contradict
               # the first is not evidence -- which is the very sentence the
               # comment above states. Latent, never fired: 0 occurrences
               # across 8 recorded gate runs, so no past certificate moves.
    fi
        if [ -n "$EXPECTED_SUITE" ]; then
            unexpected_suites=$(printf '%s\n' "$failed" | grep -v -x "$EXPECTED_SUITE" | grep -v '^$' || true)
        else
            # EMPTY REGISTER: every failing suite is unexpected. Stated as its own
            # branch because `grep -v -x ""` drops only blank lines, which reaches
            # the same answer for the wrong reason.
            unexpected_suites=$(printf '%s\n' "$failed" | grep -v '^$' || true)
        fi

        # Q-P5 (registered INTEGRATION-log 244.2, BEFORE this code): A RED FROM A REGISTERED
        # QUARANTINED CASE DOES NOT RED THE GATE, AND A RED FROM ANY OTHER CASE IN THE SAME
        # SUITE STILL DOES. Falsifier: either half alone -- suppressing the whole suite hides
        # the 21 unquarantined cases beside it; suppressing nothing leaves the ruling unmet.
        #
        # GRANULARITY MISMATCH (O-UI, 2026-08-28): the register is CASE-granular, ctest's
        # failure list is SUITE-granular. Same bridge the expected-fail hold uses after C-357:
        # re-run the suite ALONE so cross-suite attribution is IMPOSSIBLE, then subtract by CASE.
        #
        # NO HEREDOC HERE ON PURPOSE: the first draft used one and its terminator picked up this
        # block's indentation, which is a syntax error. IFS-newline splitting is indentation-proof
        # and this file is #!/bin/bash, where unquoted splitting is defined (it is NOT in zsh).
        if [ -n "$unexpected_suites" ] && [ -r reports/QUARANTINE.md ]; then
            local _qkeep="" _qs _qcases _qlog _qfail _qstray _qifs
            _qifs="$IFS"; IFS=$'\n'
            for _qs in $unexpected_suites; do
                [ -n "$_qs" ] || continue
                _qcases=$(awk -F'|' -v S="$_qs" '/^QROW/ {split($2,a,"::");
                    gsub(/^[ \t]+|[ \t]+$/,"",a[1]); gsub(/^[ \t]+|[ \t]+$/,"",a[2]);
                    if (a[1]==S && a[2]!="") print a[2]}' reports/QUARANTINE.md)
                if [ -z "$_qcases" ]; then _qkeep="$_qkeep$_qs"$'\n'; continue; fi
                _qlog=$(mktemp) || { _qkeep="$_qkeep$_qs"$'\n'; continue; }
                ctest --test-dir target/shell-build -R "^${_qs}$" --output-on-failure > "$_qlog" 2>&1 || true
                _qfail=$(sed -n 's/^FAIL!  *: *[A-Za-z0-9_]*::\([A-Za-z0-9_]*\)().*/\1/p' "$_qlog" | sort -u)
                rm -f "$_qlog"
                _qstray=$(printf '%s\n' "$_qfail" | /usr/bin/grep -vxF -- "$_qcases" | /usr/bin/grep -v '^$' || true)
                if [ -n "$_qstray" ]; then
                    echo "shell-behavior: QUARANTINE-STRAY($_qs also failed: $(printf '%s' "$_qstray" | tr '\n' ' '))" >&2
                    _qkeep="$_qkeep$_qs"$'\n'
                else
                    echo "shell-behavior: QUARANTINE-SUPPRESSED($_qs -- only registered quarantined case(s) failed)"
                fi
            done
            IFS="$_qifs"
            unexpected_suites=$(printf '%s' "$_qkeep" | /usr/bin/grep -v '^$' || true)
        fi
        if [ -n "$unexpected_suites" ]; then
            echo "shell-behavior: SUITES-FAILED($(printf '%s' "$unexpected_suites" | tr '\n' ' '))" >&2
            rc=1
        fi
        if [ -n "$EXPECTED_SUITE" ] && printf '%s\n' "$failed" | grep -q -x "$EXPECTED_SUITE"; then
            # Suite-granular hold, case-granular check: the ONLY failing cases in
            # the held suite must be the named one.
            #
            # C-357 — THIS SCRAPED THE WHOLE LOG AND MIS-ATTRIBUTED. On its first
            # faithful run it reported EXPECTED-FAIL-HOLD-TOO-WIDE naming
            # `resizePreviewMatchesEngine`, which belongs to TstReviewDialogProbes
            # (shell/tests/tst_review_dialog_probes.cpp:876), not to the held
            # suite -- a failure anywhere in the run was charged to the suite on
            # hold. Re-running the held suite ALONE makes cross-suite attribution
            # impossible rather than unlikely; it costs ~1.2 s.
            local holdlog
            holdlog=$(mktemp) || { echo "shell-behavior: UNMEASURED-NO-TMPFILE(hold re-run)" >&2; rm -f "$ctlog"; rm -rf "$work"; return 2; }
            ctest --test-dir target/shell-build -R "^${EXPECTED_SUITE}$" --output-on-failure > "$holdlog" 2>&1 || true
            failing_cases=$(sed -n 's/^FAIL!  *: *[A-Za-z0-9_]*::\([A-Za-z0-9_]*\)().*/\1/p' "$holdlog" | sort -u)
            rm -f "$holdlog"
            local stray
            stray=$(printf '%s\n' "$failing_cases" | grep -v -x "$EXPECTED_CASE" | grep -v '^$' || true)
            if [ -n "$stray" ]; then
                echo "shell-behavior: EXPECTED-FAIL-HOLD-TOO-WIDE($EXPECTED_SUITE also failed: $(printf '%s' "$stray" | tr '\n' ' ') -- the hold covers one case and is hiding others)" >&2
                rc=1
            else
                echo "shell-behavior: EXPECTED-FAIL($EXPECTED_SUITE::$EXPECTED_CASE -- operator-directed 2026-08-25; closes with operator item 22 / engine P2)"
            fi
        fi
    fi
    # PIN OBSOLESCENCE PANIC -- rule 12, O-WATCH 3's finding, verified by O-INT.
    #
    # The whole expected-fail block above sits inside `if [ "$ctrc" -ne 0 ]`. If the
    # engine conforms and the pinned case starts PASSING, ctrc is 0, that block
    # never executes, and THE GATE GOES GREEN HAVING SAID NOTHING ABOUT THE PIN.
    # It fails SAFE, which is what makes it dangerous: a stale pin that REDS is
    # fixed within a day; one that goes QUIET is cited in certificates
    # indefinitely, and every future PASS carries EXPECTED-FAIL language for a
    # condition that silently resolved. G-139, and G-124's family -- a rationale
    # that outlives its subject with nothing to notice.
    #
    # ★ ABSENT IS NOT PASSING. `ctest -R "^<name>$"` EXITS 0 printing "No tests
    # were found!!!" when the name is gone, so a naive "did it fail?" reads a
    # DROPPED suite as "the defect is fixed, remove the stale pin" -- A21
    # reproduced inside the fix for the pin. The presence precondition and the
    # thing it guards are deliberately IN THE SAME FUNCTION, five lines apart:
    # separating a guard from its guarded is what produced :1195 and A21 both.
    if [ -z "$EXPECTED_SUITE" ]; then
        : # register empty -- nothing pinned, so nothing can go stale. The
          # obsolescence panic below exists to catch a pin outliving its cause;
          # with no pin it has no subject, and leaving the ABSENT arm live would
          # red every run forever (G-514, a guard red for benign reasons).
    elif printf '%s\n' "$_declared_names" | grep -q -x "$EXPECTED_SUITE"; then
        if [ "$ctrc" -eq 0 ]; then
            echo "shell-behavior: EXPECTED-FAIL-NO-LONGER-REPRODUCES($EXPECTED_SUITE::$EXPECTED_CASE -- the run was fully green, so the pinned case PASSED; the pin is stale, remove it and close operator item 22)" >&2
            rc=1
        fi
    else
        echo "shell-behavior: EXPECTED-FAIL-SUITE-ABSENT($EXPECTED_SUITE is not registered -- the pin names a suite that no longer exists. ABSENT IS NOT PASSING: do not read this as the defect being fixed)" >&2
        rc=1
    fi

    rm -f "$ctlog"

    rm -rf "$work"
    return "$rc"
}

# C-383: STAMP THE TREE AT GATE START, NOT AT THE STEP THAT HAPPENS TO PRINT.
# My own run named its retained log for commit 93180658 -- a commit I made an
# hour AFTER the run began, because tree_sha was computed at the suites step.
# Six of my commits landed while that gate ran. The artifact therefore attributed
# a suite run to a tree it never tested. G-134 applied to the executing script and
# NOT to the repository the run describes, which was too narrow a reading of it.
GATE_START_SHA=$(git rev-parse --short HEAD 2>/dev/null || echo UNKNOWN)
export GATE_START_SHA
# C-410 (cloud ruling, 2026-08-28): the stamp is minted ONCE, here, beside the sha.
# :529 and :566 each ran their own $(date), so ONE RUN'S TWO ARTIFACTS COULD CARRY TWO
# CLOCKS -- a run straddling a second boundary produced filenames no reader could pair,
# and the pairing is the artifact's whole purpose. THE RUN'S SHA REMAINS THE PRIMARY
# PAIRING KEY; the stamp is DISPLAY METADATA, now one value both consumers read rather
# than two values they each derive.
GATE_START_STAMP=$(date +%Y%m%dT%H%M%S)
# The step transcript's home, named from the SAME sha+stamp pair the suites log
# and its provenance sidecar use, so all three artifacts of one run sort together
# and a reader can pair them without guessing.
GATE_STEPLOG="target/shell-build/gate-steps-${GATE_START_SHA}-${GATE_START_STAMP}.log"
mkdir -p target/shell-build 2>/dev/null || :
export GATE_START_STAMP

run_step build         cargo build --workspace
run_step test          cargo test --workspace
run_step deny          cargo deny check advisories bans licenses sources
run_step reuse         step_reuse
run_step spdx          step_spdx
run_step contract-sync bash tools/sync_contracts.sh --check
run_step notices       python3 tools/gen_notices.py --check
run_step i18n-audit    step_i18n_audit
run_step walled-symbols step_walled_symbols
# fmt: ninth step (O-QA observation 2026-08-17: two rustfmt drift instances
# walked into main green — formatting is now gate-enforced, not
# convention-enforced). Introduced together with a one-time mechanical
# workspace-wide `cargo fmt` sweep in the same branch so the step lands
# green; per-lane drift henceforth reds the merge train at the offending
# branch, never silently on main.
run_step fmt           cargo fmt --all --check
run_step shell-relink   step_shell_relink
run_step quarantine     step_quarantine
run_step release-engine-bind step_release_engine_bind
# ARM VERDICTS REACH THE TRANSCRIPT, NOT ONLY THE TERMINAL. O-WATCH #389:
# they could read GATE_RC and the step tally from gate-steps-*.log, and the
# reason for the one FAIL from NOTHING -- "shell-behavior writes its arm
# verdicts to the gate STDOUT, and only its PASS/FAIL reaches a retained file
# ... I record it as their measurement, not as mine."
#
# THIS IS THE THIRD ITERATION OF ONE GAP, AND note()'s own comment above holds
# the second. Suite output was retained after #76; step verdicts after
# #297/#298; the ARM verdicts inside a step were still terminal-only. Each fix
# covered the layer above the one that mattered next, so "why is the gate red"
# stayed unauditable while "is the gate red" became well recorded.
#
# rc COMES FROM PIPESTATUS[0], AND THE FIRST VERSION OF THIS COMMENT HAD THE
# REASON BACKWARDS. It said a bare `| tee` "would hand the gate tee's exit
# status and this step could never fail again." ★★★★ THAT IS FALSE HERE:
# gate.sh runs under `set -euo pipefail` (:64), so a bare pipe reports the
# RIGHTMOST NON-ZERO and the step fails perfectly well. Measured, both
# wrappers, four conditions:
#
#                    callee0/logOK  callee1/logOK  callee0/logBAD  callee2/logOK
#   PIPESTATUS[0]         0              1              0               2
#   bare pipe             0              1              1               2
#
# ★★★★★ THEY DIFFER IN EXACTLY ONE COLUMN: the step SUCCEEDS and the
# transcript is unwritable. A bare pipe turns that into a GATE FAILURE -- the
# gate going red because it could not write its own log. PIPESTATUS[0] keeps
# it silent, which is EXACTLY the design note() states for the same file:
# "APPEND-ONLY AND FAILURE-SILENT BY DESIGN ... a verdict that depends on its
# own transcript succeeding is a worse instrument than one with no
# transcript."
# ★★★ So the construct is right and the ORIGINAL JUSTIFICATION WAS NOT. The
# hazard is not a step that cannot fail; it is a step that fails for a benign
# reason (G-514). Kept, with the real reason, because a guard defended by a
# false argument is one re-reading away from being removed.
# The wrapper is watched by sb_retained_selftest, immediately below, which
# GATES the step -- and its must-fail arm is that one differing column.
#
# 2>&1 MERGES THE STREAMS AND THAT IS STATED RATHER THAN HIDDEN: several arms
# print to stderr. The step's VERDICT is its rc and never its stream, so
# nothing that reads the verdict changes; a reader who kept only stdout now
# sees the failing arms too, which is the entire point.
sb_retained() {
    step_shell_behavior 2>&1 | tee -a "${GATE_STEPLOG:-/dev/null}"
    return "${PIPESTATUS[0]}"
}
# ★★★★★ THE ARM THIS COMMENT USED TO PROMISE. O-WATCH #391-D: the text said
# "watched by the selftest arms below" and NOTHING BELOW WATCHED IT. The
# verification had genuinely happened -- by execution, both directions,
# recorded at 1635e853 -- but a comment in the PRESENT TENSE describes a
# standing guard, and an act is not a guard. Their sentence, one level in:
# testimony describing a safeguard is not the safeguard.
#
# ★★★★ THIS EXERCISES THE REAL sb_retained, NOT A REPLICA OF ITS PATTERN. A
# synthetic copy of `| tee` + PIPESTATUS would pass forever while the shipped
# function drifted away from it -- the two-copy problem in miniature, inside
# the guard meant to prevent a silent step. So the callee is substituted and
# the ACTUAL wrapper is called.
#
# ★★★ BOTH DIRECTIONS, AND THE MUST-PASS ONE IS NOT DECORATION: a wrapper that
# can never return 0 is indistinguishable from one that never fires, and only
# a must-succeed arm separates them (G-541). rc 2 is checked too, because
# UNMEASURED is a distinct verdict class that a naive `return 1` would erase.
sb_retained_selftest() {
    local _saved _rc _bad=0 _want _keep
    _saved=$(declare -f step_shell_behavior) || return 1
    _keep="${GATE_STEPLOG:-}"

    # ★★★★★ THE ARMS WRITE TO /dev/null, NOT TO THE REAL TRANSCRIPT. O-WATCH
    # #393-A: the first version left GATE_STEPLOG pointing at the retained log
    # while calling sb_retained four times, so THE GUARD BUILT TO PROTECT THE
    # TRANSCRIPT WROTE FOUR JUNK LINES INTO THE TRANSCRIPT IT PROTECTS.
    # ★★★★ That is the shape I named at INTEGRATION-log 405 -- the tour's own
    # compositeHash() putting a temp path into the frames it measures --
    # arriving one turn later in the guard I wrote about it. An instrument
    # that contaminates its own artifact does not announce itself: the lines
    # are well-formed and land where real lines land.
    # PASS-THROUGH ARMS. rc 2 is here on purpose: UNMEASURED is a distinct
    # verdict class, and a naive `return 1` would erase it.
    GATE_STEPLOG=/dev/null
    for _want in 0 1 2 7; do
        eval "step_shell_behavior() { echo 'selftest arm'; return $_want; }"
        set +e; sb_retained >/dev/null 2>&1; _rc=$?; set -e
        [ "$_rc" -eq "$_want" ] || {
            echo "gate: SB-WRAPPER-BROKEN(callee returned $_want, wrapper reported $_rc)" >&2
            _bad=1
        }
    done

    # ★★★★★ THE ARM THAT ACTUALLY DISCRIMINATES, and the only one a bare pipe
    # fails: the step SUCCEEDS while the transcript is UNWRITABLE. A bare pipe
    # reports 1 here and reds the gate over its own logging; PIPESTATUS[0]
    # reports 0. Without this arm the guard cannot tell the two constructs
    # apart under pipefail and would never fire -- which is what the first
    # version of it did, and why it was rewritten before it ever shipped.
    eval "step_shell_behavior() { echo 'selftest arm'; return 0; }"
    GATE_STEPLOG=/nonexistent-dir-for-selftest/x.log
    set +e; sb_retained >/dev/null 2>&1; _rc=$?; set -e
    GATE_STEPLOG="$_keep"
    [ "$_rc" -eq 0 ] || {
        echo "gate: SB-WRAPPER-NOT-FAILURE-SILENT(step passed but wrapper reported $_rc" \
             "because the transcript was unwritable -- see note(), which is" \
             "append-only and failure-silent by design)" >&2
        _bad=1
    }

    eval "$_saved"
    return "$_bad"
}
if ! sb_retained_selftest; then
    note "GATE shell-behavior UNMEASURED"
    UNMEASURED=$((UNMEASURED + 1)); FAILURES=$((FAILURES + 1))
else
    run_step shell-behavior sb_retained
fi

echo
echo "==== gate summary ===="
printf '%s' "$SUMMARY"
if [ "$FAILURES" -ne 0 ]; then
    # UNMEASURED is reported SEPARATELY but still fails the gate: a step that
    # did not measure the tree has not certified it.
    # O-WATCH #299: THESE TWO LINES SAT OUTSIDE THE CHOKEPOINT. The step
    # verdicts became readable at 9735bccb and the SUMMARY did not, so a
    # stranger could count the tally (arm 3) and still only INFER the exit
    # status (arm 1) from the presence of a FAIL line. Routed through note()
    # so both are retained; SUMMARY has already been printed above, so
    # appending to it here changes no output.
    #
    # GATE_RC IS WRITTEN AS A VALUE, NOT LEFT TO INFERENCE. It is the process
    # exit status and lived nowhere -- derivable is not recorded, and the
    # difference is exactly what cost two arms their scoreability.
    note "gate: $FAILURES step(s) not PASS ($UNMEASURED UNMEASURED, $((FAILURES - UNMEASURED)) FAILED)"
    note "GATE_RC 1"
    exit 1
fi
note "gate: all steps PASS"
note "GATE_RC 0"
# REUSE-IgnoreEnd
