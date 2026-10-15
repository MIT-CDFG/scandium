#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# BOUND A, two-arm form (operator ruling 2026-08-25). Implemented by O-UI,
# reviewed and landed by O-INT -- because this modifies a certification
# instrument at the moment it blocked its own proposer, and the desk that finds
# a gate inconvenient is the last desk that should get to change it alone.
#
# THE PROBLEM IT REPLACES. Bound A originally required EVERY ctest binary's
# mtime to postdate the run's start marker T0. That conflates two different
# things: "stale because something FAILED to rebuild it" and "unchanged because
# correctly NOTHING NEEDED TO". An incremental build's correct behaviour is to
# leave untouched binaries untouched, so the old bound could only ever be
# satisfied by a full rebuild -- it reds on every honest incremental run.
# Measured 2026-08-25: 20 of 38 binaries predated T0, none of them link
# libappname_cabi.dylib, and ninja had correctly relinked exactly the 23
# cabi-linked edges on an unchanged source tree.
#
#   ARM 1  Any binary in the DEPENDENCY CLOSURE of the change being certified
#          MUST be newer than T0. No exceptions.
#   ARM 2  A binary OUTSIDE the closure may predate T0 ONLY IF the build
#          system's own dry-run reports it up-to-date -- and it is NAMED in the
#          report as "current-by-graph, not rebuilt". Never silently passed.
#
# Bound B (sha256 of libappname_cabi.dylib identical either side of the
# battery) is unchanged and is NOT checked here.
#
# Usage: o-ui-bound-a.sh <build-dir> <T0-epoch> <changed-file>...
set -uo pipefail
BUILD="${1:?build dir}"; T0="${2:?T0 epoch}"; shift 2
CHANGED=("$@")
[ "${#CHANGED[@]}" -gt 0 ] || { echo "BOUND-A ERROR: no changed files given -- refusing to certify a closure of unknown size"; exit 2; }

BUILD_ABS=$(cd "$BUILD" && pwd)
REPO_ROOT=$(git -C "$BUILD_ABS" rev-parse --show-toplevel 2>/dev/null)
[ -n "$REPO_ROOT" ] || { echo "BOUND-A ERROR: cannot derive repo root from $BUILD_ABS -- refusing to run, because a wrong root silently makes ARM 2 pass everything"; exit 2; }
cd "$BUILD" || { echo "BOUND-A ERROR: build dir unreadable"; exit 2; }
# bash 3.2 on macOS has no `mapfile`; a while-read loop is the portable form.
BINS=()
while IFS= read -r _l; do [ -n "$_l" ] && BINS+=("$_l"); done < <(ctest -N 2>/dev/null | sed -n 's/^ *Test *#[0-9]*: *//p' | sort -u)
[ "${#BINS[@]}" -gt 0 ] || { echo "BOUND-A ERROR: ctest declared no tests -- instrument floor failed"; exit 2; }

arm1_fail=0; arm2_fail=0; closure=0; outside=0
declare -a NAMED_CURRENT=() FAIL1=() FAIL2=()

for b in "${BINS[@]}"; do
    [ -x "$b" ] || continue
    mt=$(stat -f %m "$b" 2>/dev/null || echo 0)
    ins=$(ninja -t inputs "$b" 2>/dev/null)
    inclosure=0
    for c in "${CHANGED[@]}"; do
        # Anchor on the REPO-RELATIVE PATH. A basename substring match is
        # over-broad in both directions: a changed `mod.rs` matches every input
        # containing that string, and `main.cpp` matches `domain.cpp`. It fails
        # SAFE (over-inclusion pushes binaries under arm 1, which is stricter)
        # but it makes the closure/outside split reported to a reader meaningless.
        if printf '%s\n' "$ins" | grep -qF -- "$REPO_ROOT/$c"; then inclosure=1; break; fi
    done
    if [ "$inclosure" -eq 1 ]; then
        closure=$((closure+1))
        if [ "$mt" -le "$T0" ]; then FAIL1+=("$b (mtime $mt <= T0 $T0)"); arm1_fail=1; fi
    else
        outside=$((outside+1))
        if [ "$mt" -le "$T0" ]; then
            # ARM 2's instrument is NOT `ninja -n`. Measured 2026-08-25: a queued
            # CMake re-run short-circuits the dry-run, so `ninja -n <target>`
            # emits only "Re-running CMake..." and reports ZERO pending edges
            # even with an input freshly touched -- which would make this arm a
            # guard that cannot fire. Caught only because the operator required
            # both controls demonstrated RED before the bound is trusted.
            # Instead: compare the binary against its OWN INPUTS directly, which
            # is the property the bound actually means -- "not older than its
            # own inputs" -- and depends on no dry-run behaviour at all.
            # ARM 2's instrument, THIRD form. Two earlier ones were watched and
            # both were wrong in opposite directions:
            #   `ninja -n <target>`      -- CANNOT FIRE. A queued CMake re-run
            #     short-circuits the dry-run, which then reports zero pending
            #     edges even with an input freshly touched.
            #   max(ALL input mtimes)    -- ALWAYS FIRES. `ninja -t inputs`
            #     includes GENERATED inputs (e.g. <target>_auto under the build
            #     dir) that the CMake re-run makes newer than every binary.
            # Only SOURCE inputs can legitimately red this arm: a binary may be
            # older than its own generated config, but never older than a source
            # file it compiles. Inputs are ABSOLUTE paths in three shapes --
            # repo sources, generated files under the build dir, and external
            # toolchain (/opt/homebrew) -- so a source is "under the repo AND not
            # under the build dir".
            newest=0; newest_src=""
            while IFS= read -r _in; do
                case "$_in" in
                    "$REPO_ROOT"/*) ;;          # under the repo: candidate
                    *) continue ;;              # external toolchain: not ours
                esac
                case "$_in" in
                    "$BUILD_ABS"/*) continue ;; # generated: excluded by design
                esac
                [ -f "$_in" ] || continue
                _m=$(stat -f %m "$_in" 2>/dev/null || echo 0)
                if [ "$_m" -gt "$newest" ]; then newest=$_m; newest_src="$_in"; fi
            done < <(ninja -t inputs "$b" 2>/dev/null)
            if [ "$newest" -gt "$mt" ]; then
                FAIL2+=("$b (predates T0 AND SOURCE input newer: ${newest_src##*/} $newest > binary $mt)"); arm2_fail=1
            else
                NAMED_CURRENT+=("$b")
            fi
        fi
    fi
done

echo "BOUND A (two-arm) -- ${#BINS[@]} declared binaries: $closure in closure, $outside outside"
echo "  ARM 1 (closure must postdate T0): $([ "$arm1_fail" -eq 0 ] && echo PASS || echo FAIL)"
for f in ${FAIL1[@]+"${FAIL1[@]}"}; do echo "      *** ARM 1 VIOLATION: $f"; done
echo "  ARM 2 (outside closure, no SOURCE input newer): $([ "$arm2_fail" -eq 0 ] && echo PASS || echo FAIL)"
for f in ${FAIL2[@]+"${FAIL2[@]}"}; do echo "      *** ARM 2 VIOLATION: $f"; done
echo "  NAMED current-by-graph, not rebuilt (${#NAMED_CURRENT[@]}):"
for n in ${NAMED_CURRENT[@]+"${NAMED_CURRENT[@]}"}; do echo "      $n  -- current-by-graph, NOT REBUILT"; done
[ "$arm1_fail" -eq 0 ] && [ "$arm2_fail" -eq 0 ] && { echo "  => BOUND A SATISFIED"; exit 0; }
echo "  => BOUND A NOT SATISFIED"; exit 1
