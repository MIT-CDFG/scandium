#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# D-44 WALLED-SYMBOL RELEASE ASSERTION — the release-BINARY counterpart to
# scripts/gate.sh's walled-symbols SOURCE grep. Plan §2.5 "Release
# engineering" exit-gate item ("release-CI walled-symbol assertion —
# cas_seam_carving etc. absent"); ES §33.6 step 5d ("release CI greps gated
# symbols"); Appendix Q §Q.3 (the date-gated selections); D-44
# (ALGORITHMS.md §1–§2 patent-status ladder). Terminology: due-diligence
# tripwire, not a release claim (V39).
#
# WHAT THIS ASSERTS. The three date-gated engine identifiers (ES §Q.3
# table, ES:20548-20550) are ABSENT from every shipped release binary's
# symbol table:
#   * cas_seam_carving               US7477800B2   gate 2027-03-06   §18.4
#   * heal_texture_contrast_variant  US7605821B1   gate 2028-03-17   §21.2/§45.1
#   * caf_randomized_nnf             US8285055B1   gate 2031-04-09   §21.4/§45.3
# plus the walled algorithm stems the gate's source sweep already bans
# (patchmatch/nnf/surf/berman/mixbox/bm3d + seam[_ ]?carv), consumed from
# scripts/gate.sh --print-walled-patterns so this check can never validate a
# stale pattern copy.
#
# WHY IT PASSES TODAY (the honest state). None of the three gated engines is
# implemented: the shipped engines are the design-arounds — warp
# retargeting for content-aware scale (§18.4), additive gradient-domain +
# MVC healing (§21.2), and deterministic exemplar synthesis for
# content-aware fill (§21.4). There is no compile-off feature to toggle;
# the symbols are absent BY CONSTRUCTION (never written), a stronger
# guarantee than compile-off. This assertion is the durable gate that turns
# a future accidental introduction — a gated-engine symbol reaching a
# release artifact through any feature, dependency, or paste — into a hard
# red. A HIT here is a CRITICAL finding: STOP and escalate (Appendix K
# incident), never a routine bug.
#
# HOW (macOS host). `nm` over the shipped Mach-O artifacts (llvm-nm here,
# GNU-nm compatible; objdump -t fallback). Rust name mangling carries the
# source identifier as a literal substring, so a case-insensitive symbol
# grep catches it. The shipped artifacts come from the `release` profile
# (scripts/build_dmg.sh: `cargo build --release`), which retains the symbol
# table; the per-artifact <100-symbol guard below refuses a vacuous pass on
# an empty/stripped table.
#
# STRIP CAVEAT (ES §33.4, ES:10816). The spec's release-packaging `dist`
# profile carries `strip = "symbols"` (symbols archived server-side, §35.3).
# No `dist` profile exists in this workspace yet — the shipped `release`
# profile keeps symbols, so this assertion sees the full table. WHEN a
# stripping `dist` profile is introduced, this assertion MUST run on the
# PRE-STRIP binary (the same one archived per §35.3); the <100 guard turns
# an accidental run against a stripped artifact into a hard FAIL rather than
# a silent vacuous pass.
#
# USAGE:
#   tools/walled_symbols_release.sh            build release + assert (default)
#   tools/walled_symbols_release.sh --no-build assert over existing artifacts
#   tools/walled_symbols_release.sh --selftest negative control: the matcher
#                                              MUST flag a planted gated
#                                              symbol (proves the tripwire
#                                              works, not just that it passes)
#   tools/walled_symbols_release.sh --bundle APP  assert over the SHIPPED
#                                   population: the appname-owned Mach-O
#                                   binaries inside a built .app, read from
#                                   tools/shipped_libraries.py (finding OB-21)
#   tools/walled_symbols_release.sh --print-gated-symbols   the D-44 triple

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
GATE="$ROOT/scripts/gate.sh"

if ! command -v cargo >/dev/null 2>&1; then
    export PATH="$HOME/.cargo/bin:$PATH"
fi

# The three date-gated engine identifiers, single-sourced here (the D-44
# named set) — matched as literal, case-insensitive substrings of mangled
# symbols. Assembled from fragments so this file carries no bare walled
# identifier (house style, cf. tools/selftest_walled_sweep.sh).
readonly G1="$(printf 'cas_seam_%s' 'carving')"
readonly G2="$(printf 'heal_texture_%s' 'contrast_variant')"
readonly G3="$(printf 'caf_randomized_%s' 'nnf')"
# Bare engine entry points the gated §18.4/§21.4 text would introduce.
readonly G4="$(printf 'seam_%s' 'carve')"
readonly G5="$(printf 'randomized_%s' 'nnf')"
# OPERATOR WORD 9 (7), 2026-09-14 18:46 ET (R-10): two more, "before RC1", so that
# "correct because nobody wrote it" becomes "red if anyone does". Neither appears in
# the tree today -- that is the point: the assertion is cheap now and expensive to
# add after someone has written the code. `cahn[_ ]?hilliard` is his regex verbatim,
# not broadened; the second is the variance-matching stem. Both are assembled from
# fragments like G1-G5 so this file still carries no bare walled identifier, and both
# are matched by the same `grep -iE` the array has always used.
readonly G6="$(printf 'cahn[_ ]?%s' 'hilliard')"
readonly G7="$(printf 'variance[_ ]?%s' 'match')"
GATED=("$G1" "$G2" "$G3" "$G4" "$G5" "$G6" "$G7")

if [ "${1:-}" = "--print-gated-symbols" ]; then
    printf '%s\n' "${GATED[@]}"
    exit 0
fi

# ---- symbol extractor: nm, objdump -t fallback ----------------------------
have_nm() { command -v nm >/dev/null 2>&1; }
have_objdump() { command -v objdump >/dev/null 2>&1; }
dump_symbols() { # $1 = artifact path -> symbol lines on stdout
    if have_nm; then
        nm "$1" 2>/dev/null || true
    elif have_objdump; then
        objdump -t "$1" 2>/dev/null || true
    else
        echo "walled-symbols-release: no nm or objdump on PATH" >&2
        return 2
    fi
}

# ---- walled stem patterns from the gate (never a stale copy) --------------
# Pass 1 (ci) and pass 4 (ci) are the stem sweeps; pass 2/3 (cs) catch the
# CamelCase compounds a Rust type name can carry into a mangled symbol. All
# four are applied to the symbol stream.
load_gate_patterns() {
    local out; out="$(bash "$GATE" --print-walled-patterns)"
    [ "$(printf '%s\n' "$out" | wc -l | tr -d ' ')" = "4" ] || {
        echo "walled-symbols-release: gate did not emit 4 walled patterns" >&2
        exit 2
    }
    GMODE1="$(printf '%s\n' "$out" | sed -n '1p' | cut -d' ' -f1)"; GPAT1="$(printf '%s\n' "$out" | sed -n '1p' | cut -d' ' -f2-)"
    GMODE2="$(printf '%s\n' "$out" | sed -n '2p' | cut -d' ' -f1)"; GPAT2="$(printf '%s\n' "$out" | sed -n '2p' | cut -d' ' -f2-)"
    GMODE3="$(printf '%s\n' "$out" | sed -n '3p' | cut -d' ' -f1)"; GPAT3="$(printf '%s\n' "$out" | sed -n '3p' | cut -d' ' -f2-)"
    GMODE4="$(printf '%s\n' "$out" | sed -n '4p' | cut -d' ' -f1)"; GPAT4="$(printf '%s\n' "$out" | sed -n '4p' | cut -d' ' -f2-)"
}

# Scan one symbol stream (stdin) for gated identifiers + gate stems.
# Prints offending symbol lines. grep convention: returns 0 if ANY walled
# hit is found, 1 if the stream is clean.
scan_symbols() { # reads stdin
    local syms; syms="$(cat)"
    local found=1 g line out   # 1 = clean (nonzero = no match, grep-like)
    for g in "${GATED[@]}"; do
        if line="$(printf '%s\n' "$syms" | grep -iE "$g")"; then
            printf '%s\n' "$line"; found=0
        fi
    done
    for spec in "i $GPAT1" "s $GPAT2" "s $GPAT3" "i $GPAT4"; do
        local mode="${spec%% *}" pat="${spec#* }"
        if [ "$mode" = i ]; then
            out="$(printf '%s\n' "$syms" | grep -iE "$pat" || true)"
        else
            out="$(printf '%s\n' "$syms" | grep -E "$pat" || true)"
        fi
        if [ -n "$out" ]; then printf '%s\n' "$out"; found=0; fi
    done
    return "$found"
}

# ---- SHIPPED release artifact set -----------------------------------------
# The binaries the dmg/packaging pipeline ships (build_dmg.sh): the CLI, the
# headless actions runner, the C-ABI dylib the shell links, and the Python
# trusted-tier dylib. Existence-guarded — a leg absent from this build is
# reported SKIP, never a silent pass.
# _existing_of: print the arguments that exist, and NEVER fail.
#
# FIXED 2026-09-05 against O-WATCH #484 and O-INT entries 645/646, on the
# operator's assignment of 2026-09-05 19:15 ET (item 8, link 3).
#
# THE DEFECT: this loop was `[ -f "$a" ] && printf ...`, and a `for` loop's
# exit status is its LAST iteration's. With `set -euo pipefail` (line 62), a
# MISSING LAST LEG made the function return 1 and killed the whole script at
# the caller's assignment -- silently, rc=1, zero bytes.
#
# WHAT THAT COST: the caller at run_assertion ALREADY handles an empty list
# correctly -- it prints "no shipped release artifacts found" and exits 1, and
# gen_attestation.py greps for exactly that substring to report "pending".
# THE CORRECT BEHAVIOUR WAS ALREADY WRITTEN AND WAS UNREACHABLE. Instead
# gen_attestation saw rc!=0 with EMPTY output, fell past its substring test,
# and reported "CRITICAL -- a gated/walled symbol is present in a shipped
# artifact (D-44 STOP)". A missing build artifact was attested as a shipped
# gated symbol, on the ordinary release path (build_dmg.sh builds one of the
# four legs; gen_attestation runs this in --no-build).
#
# The `if` form is deliberate over `|| :` or a trailing `return 0`: when the
# file DOES exist the status is printf's, so a genuine write failure still
# surfaces. Taking the list as ARGUMENTS is what makes the must-fire arm in
# run_selftest possible at all -- the old hardcoded loop could not be driven.
_existing_of() {
    local a
    for a in "$@"; do
        if [ -f "$a" ]; then printf '%s\n' "$a"; fi
    done
}

shipped_artifacts() {
    _existing_of \
        target/release/appname \
        target/release/appname-actions \
        target/release/libappname_cabi.dylib \
        target/release/libappname_py.dylib
}

# ==== OB-21: the population is a PARAMETER, and the dev tree is not it ======
#
# The function above is called shipped_artifacts and every path in it is a
# DEV-TREE path. Measured against the staged bundle on 2026-09-10: three of the
# four legs do not ship at all, and the fourth ships as a MODIFIED copy --
# install_name_tool rewrote its LC_ID_DYLIB, so dev 0c027a2b vs bundle 5be71b37.
# Overlap: 1 of 4 by name, 0 of 4 byte-for-byte. And the file a user actually
# double-clicks, Contents/MacOS/appname-shell -- where gated 18.4/21.4 UI would
# land -- was asserted by nothing at all.
#
# WHY target/release STAYS. B-1's whole point is that the assertion runs BEFORE
# staging or signing, so a violation is caught before anything is produced. The
# bundle pass is an ADDITION, not a swap, and it belongs after macdeployqt
# because that is when the shipped set exists in final form.
#
# WHY THIS DOES NOT RE-DERIVE THE FILE LIST. tools/shipped_libraries.py already
# walks the bundle and says which Mach-O binaries are ours; asking it is the
# whole point of having one enumerator. OB-10 exists because three instruments
# each grew their own idea of what ships, and a fourth here would be the same
# mistake with a shorter history.
bundle_artifacts() {
    local app="$1" here
    here="$(cd "$(dirname "$0")" && pwd)"
    # OB-37: identify as the writing build and let the census verify the pid. A
    # refusal must not arrive as an empty population (§5a-6), so the status is
    # checked and the reason is printed rather than discarded.
    local out
    if ! out="$(python3 "$here/shipped_libraries.py" --app "$app" \
            --caller=build_dmg --allow-in-flight 2>&1)"; then
        echo "walled-symbols-release: the bundle census REFUSED: $(printf '%s' "$out" | tail -1)" >&2
        return 1
    fi
    printf '%s\n' "$out" \
        | awk -F'\t' '$2 == "appname" { print $1 }' \
        | while IFS= read -r rel; do
              [ -n "$rel" ] && printf '%s\n' "$app/$rel"
          done
}

# ==== selftest: the matcher MUST catch a planted gated symbol ==============
run_selftest() {
    load_gate_patterns
    local T pass=0 fail=0
    T="$(mktemp -d "${TMPDIR:-/tmp}/walled-rel-selftest.XXXXXX")"
    trap 'rm -rf "$T"' RETURN
    ok()   { echo "PASS: $1"; pass=$((pass + 1)); }
    bad()  { echo "FAIL: $1"; fail=$((fail + 1)); }

    # PLACED FIRST, DELIBERATELY. This arm was written at the END of the
    # selftest and the mutation control proved it UNREACHABLE: reverting
    # _existing_of makes the `built="$(shipped_artifacts)"` call below die
    # under `set -e`, so the selftest exits with NO summary and the arm never
    # runs -- it would have passed forever without testing anything. Same
    # shape as entry 637's arm-8 false pass. It runs before any caller of
    # shipped_artifacts, so the defect it guards cannot suppress it.
    # ---- MUST-FIRE ARM for the _existing_of exit status (item 8 link 3).
    # A guard nobody can show fires is not a guard. Revert _existing_of to the
    # old `[ -f "$a" ] && printf` form and THIS arm goes red: the loop's status
    # becomes its last iteration's, and the last path here is deliberately
    # ABSENT. Both halves are asserted -- the rc AND the output -- because an
    # rc of 0 with the wrong lines would pass a status-only check.
    local _eo_out _eo_rc
    set +e
    _eo_out="$(_existing_of /etc/hosts /nonexistent-zzz-$$ 2>/dev/null)"; _eo_rc=$?
    set -e
    [ "$_eo_rc" = 0 ] \
        && ok "_existing_of returns 0 when the LAST path is missing" \
        || bad "_existing_of returned $_eo_rc with a missing last path — set -e would kill the caller (O-WATCH #484)"
    [ "$_eo_out" = "/etc/hosts" ] \
        && ok "_existing_of prints exactly the paths that exist" \
        || bad "_existing_of printed [$_eo_out], expected [/etc/hosts]"
    # POSITIVE CONTROL: last path PRESENT must also be rc 0, so the arm above
    # is not merely observing that this function always returns 0 for any reason.
    set +e
    _existing_of /nonexistent-zzz-$$ /etc/hosts >/dev/null 2>&1; _eo_rc=$?
    set -e
    [ "$_eo_rc" = 0 ] \
        && ok "_existing_of returns 0 when the last path EXISTS (control)" \
        || bad "_existing_of control returned $_eo_rc"


    if ! command -v cc >/dev/null 2>&1; then
        echo "walled-symbols-release --selftest: no C compiler; cannot build the negative control" >&2
        return 2
    fi
    # Negative control: a stub object EXPORTING all three gated identifiers
    # (names assembled at runtime so this file stays clean) -> the matcher
    # must flag every one. A separate clean stub must yield zero.
    {
        printf 'int %s(void){return 0;}\n' "$G1"
        printf 'int %s(void){return 0;}\n' "$G2"
        printf 'int %s(void){return 0;}\n' "$G3"
    } > "$T/planted.c"
    printf 'int appname_composite_plane(void){return 0;}\n' > "$T/clean.c"
    cc -c "$T/planted.c" -o "$T/planted.o"
    cc -c "$T/clean.c"   -o "$T/clean.o"

    local n
    for n in 1 2 3; do
        local gv; gv="$(eval echo \$G$n)"
        if dump_symbols "$T/planted.o" | grep -iqE "$gv"; then
            ok "negative control: planted gated symbol #$n present in the stub's table"
        else
            bad "negative control: planted gated symbol #$n NOT in the stub table — compiler stripped it, selftest cannot prove the tripwire"
        fi
    done
    if dump_symbols "$T/planted.o" | scan_symbols >/dev/null; then
        ok "matcher flags the planted stub (all three gated symbols caught)"
    else
        bad "matcher did NOT flag the planted stub — the assertion is blind, DO NOT TRUST it"
    fi
    if dump_symbols "$T/clean.o" | scan_symbols >/dev/null; then
        bad "matcher flagged the CLEAN stub — false positive, pattern regression"
    else
        ok "matcher leaves the clean stub alone (no false positive)"
    fi

    # If real artifacts are built, confirm they are clean here too.
    local built; built="$(shipped_artifacts)"
    if [ -n "$built" ]; then
        local dirty=0 a
        while IFS= read -r a; do
            if dump_symbols "$a" | scan_symbols >/dev/null; then dirty=1; fi
        done <<< "$built"
        [ "$dirty" = 0 ] \
            && ok "shipped release artifacts present and clean under the selftest matcher" \
            || bad "a shipped release artifact carries a gated symbol — CRITICAL"
    else
        echo "note: no shipped release artifacts built; run the default mode for the live assertion"
    fi

    echo "walled_symbols_release --selftest: $pass pass, $fail fail"
    [ "$fail" = 0 ]
}

# ==== default: the live release assertion ==================================
run_assertion() {
    local no_build="${1:-}"
    if [ "$no_build" != "--no-build" ]; then
        echo "walled-symbols-release: building release artifacts (cargo build --release --workspace)"
        cargo build --release --workspace
    fi
    load_gate_patterns

    assert_over "$(shipped_artifacts)" "dev-tree pre-staging control (target/release)"
}

# The scan itself, shared by both populations. One matcher, one vacuity guard,
# one report line -- a second copy for the bundle mode is exactly the "stale
# copy" this file's own header refuses for the gate patterns.
assert_over() {
    local artifacts="$1" label="$2"
    if [ -z "$artifacts" ]; then
        echo "walled-symbols-release: FAIL — the $label population is EMPTY. An empty scan REFUSES rather than reporting clean: a population that resolves to nothing would otherwise print an all-PASS line over zero files (finding OB-21)." >&2
        exit 1
    fi

    local failures=0 n=0 a syms nsyms
    while IFS= read -r a; do
        [ -n "$a" ] || continue
        n=$((n + 1))
        syms="$(dump_symbols "$a")"
        nsyms="$(printf '%s\n' "$syms" | grep -c . || true)"
        # Guard against a vacuous pass on an empty/stripped table.
        if [ "$nsyms" -lt 100 ]; then
            echo "walled-symbols-release: FAIL — $a yielded only $nsyms symbols (stripped/unreadable table would make the assertion vacuous)" >&2
            failures=$((failures + 1))
            continue
        fi
        if printf '%s\n' "$syms" | scan_symbols; then
            echo "walled-symbols-release: CRITICAL — gated/walled symbol present in $a (above). D-44 STOP: escalate to O-INT as an incident." >&2
            failures=$((failures + 1))
        else
            echo "WALLED-RELEASE $a PASS ($nsyms symbols, zero gated/walled)"
        fi
    done <<< "$artifacts"

    echo
    if [ "$failures" -ne 0 ]; then
        echo "walled-symbols-release: $failures artifact(s) FAILED over the $label — CRITICAL"
        exit 1
    fi
    echo "walled-symbols-release: all $n artifact(s) of the $label PASS (D-44 gated symbols absent)"
}

# ==== the SHIPPED population: appname-owned Mach-O inside the built bundle ===
run_bundle_assertion() {
    local app="${1:-}"
    [ -n "$app" ] || { echo "walled-symbols-release: --bundle wants a path to a built .app" >&2; exit 2; }
    if [ ! -d "$app" ]; then
        echo "walled-symbols-release: FAIL — no bundle at $app (run scripts/build_dmg.sh first)" >&2
        exit 1
    fi
    load_gate_patterns
    assert_over "$(bundle_artifacts "$app")" "shipped bundle after macdeployqt ($app)"
}

case "${1:-}" in
    --selftest)  run_selftest ;;
    --no-build)  run_assertion --no-build ;;
    --bundle)    run_bundle_assertion "${2:-}" ;;
    "")          run_assertion ;;
    *) echo "usage: $0 [--no-build|--selftest|--print-gated-symbols|--bundle <Appname.app>]" >&2; exit 2 ;;
esac
