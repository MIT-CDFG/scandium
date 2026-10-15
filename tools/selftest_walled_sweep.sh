#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# Self-test for the gate's walled-symbols sweep (scripts/gate.sh,
# step_walled_symbols) — created with the -I binary-skip fix (O-UI filing
# 2, 2026-08-17: a compiled object file tripped the sweep on a GENUINE
# coincidental machine-code byte run forming a walled stem delimited by
# non-alnum bytes; -a does not help because the bytes are real).
#
# Encodes the O-UI validation discipline permanently:
#   0. MATCHER IDENTITY pinned first — the gate runs under bash where
#      `grep` resolves to /usr/bin/grep (BSD grep). This machine's
#      interactive shell aliases grep to a ugrep wrapper with injected
#      flags (-I among them), which invalidated an earlier cross-check.
#      This selftest refuses to run against a non-BSD-grep resolution.
#   1. REPRODUCE the defect class: the gate's pass-1 pattern WITHOUT -I
#      must flag a synthesized binary carrying the evidence byte shape
#      (proves the false-positive class exists on this exact matcher).
#   2. Bytes-are-real cross-check: -a on the same binary still matches
#      (the corrected O-UI finding; an -a-based "fix" would not work).
#   3. FIXED SWEEP (the gate's exact flags incl. -I, patterns consumed
#      from scripts/gate.sh --print-walled-patterns — never a copy):
#      every planted TEXT identifier (one per pass) is still caught, and
#      a binary-only tree yields zero hits.
#   4. Negative corpus stays clean (surface/SURFACE_ID/seamless/
#      resurface_map/windsurfing per the gate's verified list).
#
# Planted stems are ASSEMBLED AT RUNTIME — this file carries no literal
# walled identifier (tools/ is outside the mechanical sweep scope, but the
# assembled-pattern discipline is house style outside scripts/gate.sh).

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GATE="$ROOT/scripts/gate.sh"
PASS_N=0
FAIL_N=0

ok()   { echo "PASS: $1"; PASS_N=$((PASS_N + 1)); }
fail() { echo "FAIL: $1"; FAIL_N=$((FAIL_N + 1)); }

# --- 0. matcher identity ---------------------------------------------------
GREP_PATH="$(command -v grep)"
GREP_ID="$(grep --version 2>/dev/null | head -1 || true)"
if [ "$GREP_PATH" = "/usr/bin/grep" ] && printf '%s' "$GREP_ID" | /usr/bin/grep -q "BSD grep"; then
    ok "matcher identity: $GREP_PATH ($GREP_ID) — the gate's own resolution"
else
    fail "matcher identity: grep resolves to '$GREP_PATH' ('$GREP_ID'), not the gate's /usr/bin/grep BSD grep — refusing to validate with a wrapper (O-UI lesson)"
    echo "selftest_walled_sweep: $PASS_N pass, $FAIL_N fail"
    exit 1
fi

# --- patterns from the gate itself (never a stale copy) --------------------
PATTERNS_OUT="$(bash "$GATE" --print-walled-patterns)"
[ "$(printf '%s\n' "$PATTERNS_OUT" | wc -l | tr -d ' ')" = "4" ] \
    && ok "gate emits 4 walled patterns via --print-walled-patterns" \
    || fail "gate --print-walled-patterns did not emit exactly 4 lines"
MODE1="$(printf '%s\n' "$PATTERNS_OUT" | sed -n '1p' | cut -d' ' -f1)"
P1="$(printf '%s\n' "$PATTERNS_OUT" | sed -n '1p' | cut -d' ' -f2-)"
MODE2="$(printf '%s\n' "$PATTERNS_OUT" | sed -n '2p' | cut -d' ' -f1)"
P2="$(printf '%s\n' "$PATTERNS_OUT" | sed -n '2p' | cut -d' ' -f2-)"
MODE3="$(printf '%s\n' "$PATTERNS_OUT" | sed -n '3p' | cut -d' ' -f1)"
P3="$(printf '%s\n' "$PATTERNS_OUT" | sed -n '3p' | cut -d' ' -f2-)"
MODE4="$(printf '%s\n' "$PATTERNS_OUT" | sed -n '4p' | cut -d' ' -f1)"
P4="$(printf '%s\n' "$PATTERNS_OUT" | sed -n '4p' | cut -d' ' -f2-)"
[ "$MODE1$MODE2$MODE3$MODE4" = "cicscsci" ] \
    && ok "pass modes as documented (ci cs cs ci)" \
    || fail "pass modes drifted: got '$MODE1 $MODE2 $MODE3 $MODE4', expected 'ci cs cs ci'"

# flags per mode, mirroring step_walled_symbols exactly (incl. -I)
sweep() { # $1=mode $2=pattern $3...=paths ; fixed sweep, gate flags
    local mode="$1" pat="$2"; shift 2
    if [ "$mode" = ci ]; then
        grep -rniE -I --exclude-dir=target "$pat" "$@"
    else
        grep -rnE -I --exclude-dir=target "$pat" "$@"
    fi
}

# --- sandbox ---------------------------------------------------------------
T="$(mktemp -d "${TMPDIR:-/tmp}/walled-selftest.XXXXXX")"
trap 'rm -rf "$T"' EXIT
mkdir -p "$T/scan" "$T/scan-bin-only"

# evidence-shaped binary: NUL bytes (binary classification) + a stem run
# delimited by non-alnum bytes, as quoted in the O-UI slice T entries.
STEM_NNF="$(printf 'N%s' 'NF')"
printf '\x00\x01\x7fELFish\x00\xd7\x01\xa0%s\xb4\x00\x00machine\x00' "$STEM_NNF" > "$T/scan/main.cpp.o"
cp "$T/scan/main.cpp.o" "$T/scan-bin-only/other.o"

# planted text identifiers, one per pass, assembled at runtime
S1="$(printf 'patch%s' 'match')"     # pass 1: snake/bare, any case
S2="$(printf 'Nnf%s' 'Field')"       # pass 2: CamelCase compound
S3="$(printf 'NNF%s' 'Field')"       # pass 3: ALLCAPS+CamelCase tail
S4="$(printf 'seam_%s' 'carving')"   # pass 4: seam-carving stem
printf 'fn %s_search_stub() {}\n' "$S1" > "$T/scan/planted1.rs"
printf 'struct %s;\n' "$S2"             > "$T/scan/planted2.rs"
printf 'struct %sTwo;\n' "$S3"          > "$T/scan/planted3.rs"
printf '// %s path\n' "$S4"             > "$T/scan/planted4.rs"

# negative corpus (the gate comment's verified-non-matching list)
cat > "$T/scan/negatives.rs" <<'NEG'
// surface Surface SURFACE SURFACE_ID seamless resurface_map
// windsurfing "seam boundary" allocation
NEG

# --- 1. reproduce: pass 1 WITHOUT -I flags the binary ----------------------
if grep -rniE --exclude-dir=target "$P1" "$T/scan-bin-only" >/dev/null 2>&1; then
    ok "REPRO: pass-1 pattern without -I flags the evidence-shaped binary (the defect class exists on this matcher)"
else
    fail "REPRO: pass-1 without -I did NOT flag the binary — evidence case not reproduced (matcher or synthesis drift; investigate before trusting the fix)"
fi

# --- 2. bytes are real: -a still matches -----------------------------------
if grep -c -aiE "$P1" "$T/scan/main.cpp.o" >/dev/null 2>&1; then
    ok "bytes-are-real: -a matches the binary's stem run (an -a-based fix would not work, as O-UI corrected)"
else
    fail "bytes-are-real: -a found nothing — synthesis no longer carries the stem run"
fi

# --- 3. fixed sweep: binary ignored, planted text still caught -------------
if sweep "$MODE1" "$P1" "$T/scan-bin-only" >/dev/null 2>&1; then
    fail "fixed sweep still flags a binary-only tree"
else
    ok "fixed sweep (-I): binary-only tree clean"
fi
i=1
for spec in "$MODE1 $P1 planted1.rs" "$MODE2 $P2 planted2.rs" "$MODE3 $P3 planted3.rs" "$MODE4 $P4 planted4.rs"; do
    mode="${spec%% *}"; rest="${spec#* }"
    pat="${rest% *}"; file="${rest##* }"
    if sweep "$mode" "$pat" "$T/scan" 2>/dev/null | /usr/bin/grep -q "$file"; then
        ok "fixed sweep pass $i still catches its planted source identifier ($file)"
    else
        fail "fixed sweep pass $i MISSED its planted source identifier ($file) — the fix weakened the tripwire"
    fi
    i=$((i + 1))
done

# --- 4. negatives ----------------------------------------------------------
NEG_HITS=0
for spec in "$MODE1 $P1" "$MODE2 $P2" "$MODE3 $P3" "$MODE4 $P4"; do
    mode="${spec%% *}"; pat="${spec#* }"
    if sweep "$mode" "$pat" "$T/scan" 2>/dev/null | /usr/bin/grep -q "negatives.rs"; then
        NEG_HITS=1
    fi
done
[ "$NEG_HITS" = 0 ] \
    && ok "negative corpus stays clean (surface/SURFACE_ID/seamless/resurface_map/windsurfing/seam boundary)" \
    || fail "negative corpus tripped a pass — pattern regression"

echo "selftest_walled_sweep: $PASS_N pass, $FAIL_N fail"
[ "$FAIL_N" = 0 ]
