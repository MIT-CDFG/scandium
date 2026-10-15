#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# REUSE-IgnoreStart
# (The SPDX header above is this file's real one; the token below is the
# literal string the spdx step greps for.)
#
# D-06 — the Windows gate subset (O-WIN, WS-D; ROADMAP Appendix A "Gate subset").
# Run from any prompt:   scripts\win-env.cmd bash scripts/gate-win.sh
# (win-env.cmd provides the MSVC toolchain, the W-03 zlib include and
# PYTHONUTF8=1; this script refuses to run without it.)
#
# Steps, in gate.sh's order, with gate.sh's names and verdict lines
# ("GATE <name> PASS|FAIL|UNMEASURED"; any non-PASS => non-zero exit):
#   build       cargo build --workspace
#   ctest       the shell suites in build-win (Release, Qt 6.11.1 msvc2022_64);
#               every registered suite must pass — the count is derived from
#               `ctest -N` (Amendment 8: relations, not literals)
#   deny        cargo deny check advisories bans licenses sources
#   spdx        the same sweep and exclusions as gate.sh step_spdx
#   notices     python tools/gen_notices.py --check
#   i18n-audit  python i18n/tools/audit.py
#   fmt         cargo fmt --all --check
#   tour        D-09: A==B, every frame == its Windows golden (tour-chrome-win/) else the macOS golden
# A step whose tool is absent is UNMEASURED (rc 2), never PASS — gate.sh's
# verdict classes (operator ruling 2026-08-26, "D").

set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if [ -z "${VCToolsInstallDir:-}" ]; then
    echo "gate-win: run through scripts\\win-env.cmd (no MSVC environment)" >&2
    exit 2
fi
# Git-Bash puts its coreutils `link` ahead of MSVC's; pin MSVC's first.
export PATH="$(cygpath -u "$VCToolsInstallDir")bin/Hostx64/x64:$PATH"
QT_BIN="${APPNAME_QT_BIN:-/c/Qt/6.11.1/msvc2022_64/bin}"
BUILD_DIR="${APPNAME_WIN_BUILD_DIR:-build-win}"

STAMP="$(date '+%Y%m%d-%H%M%S')"
LOGDIR="${GATE_WIN_LOGDIR:-target/gate-win/$STAMP}"
mkdir -p "$LOGDIR"
GATE_STEPLOG="$LOGDIR/steps.txt"

FAILURES=0
UNMEASURED=0

note() {
    echo "$1"
    printf '%s\n' "$1" >> "$GATE_STEPLOG" 2>/dev/null || :
}

run_step() {
    local name="$1"
    shift
    if "$@" > "$LOGDIR/$name.log" 2>&1; then
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

step_build() {
    cargo build --workspace
}

step_ctest() {
    if [ ! -f "$BUILD_DIR/CTestTestfile.cmake" ]; then
        echo "ctest: $BUILD_DIR is not configured -- UNMEASURED, not PASS" >&2
        return 2
    fi
    cmake --build "$BUILD_DIR" || return 1
    local declared
    declared=$(ctest --test-dir "$BUILD_DIR" -N | sed -n 's/^Total Tests: //p')
    echo "ctest: declared $declared"
    PATH="$QT_BIN:$PATH" ctest --test-dir "$BUILD_DIR" --output-on-failure --timeout 600
    local rc=$?
    echo "ctest: rc $rc of $declared declared"
    [ "$rc" -eq 0 ] || return 1
}

step_deny() {
    if ! cargo deny --version >/dev/null 2>&1; then
        echo "deny: cargo-deny is not installed -- UNMEASURED, not PASS" >&2
        return 2
    fi
    cargo deny check advisories bans licenses sources
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

step_notices() {
    if ! python -c 'import yaml' >/dev/null 2>&1; then
        echo "notices: PyYAML is not installed -- UNMEASURED, not PASS" >&2
        return 2
    fi
    python tools/gen_notices.py --check
}

step_i18n_audit() {
    python i18n/tools/audit.py
}

step_fmt() {
    cargo fmt --all --check
}

# D-09, WORD 2 (4): the tour must be DETERMINISTIC (run A vs run B) and
# FAITHFUL — each frame byte-equal to its Windows golden in
# artifacts/tour-chrome-win/ where glyph rasterization differs, else to the
# macOS golden in artifacts/tour-chrome/. Counts are relations derived here
# (Amendment 8): A, B and the macOS golden set must be the same size, and a
# Windows golden with no macOS counterpart is stale.
step_tour() {
    local sh="$BUILD_DIR/appname-shell.exe" work="$LOGDIR/tour" f gold
    [ -x "$sh" ] || { echo "tour: no $sh -- UNMEASURED, not PASS" >&2; return 2; }
    [ -d artifacts/tour-chrome-win ] || { echo "tour: no artifacts/tour-chrome-win -- UNMEASURED, not PASS" >&2; return 2; }
    rm -rf "$work"; mkdir -p "$work"
    PATH="$QT_BIN:$PATH" "$sh" --screenshot-tour "$work/A" > "$work/A.log" 2>&1 || { echo "tour: run A rc $?" >&2; return 1; }
    PATH="$QT_BIN:$PATH" "$sh" --screenshot-tour "$work/B" > "$work/B.log" 2>&1 || { echo "tour: run B rc $?" >&2; return 1; }
    local na nb nmac nwin
    na=$(ls "$work/A" | wc -l); nb=$(ls "$work/B" | wc -l)
    nmac=$(ls artifacts/tour-chrome/*.png | wc -l); nwin=$(ls artifacts/tour-chrome-win/*.png | wc -l)
    echo "tour: A=$na B=$nb macOS goldens=$nmac Windows goldens=$nwin"
    diff -rq "$work/A" "$work/B" || { echo "tour: NOT deterministic (A != B)" >&2; return 1; }
    [ "$na" -eq "$nmac" ] || { echo "tour: frame count $na != golden count $nmac" >&2; return 1; }
    for f in artifacts/tour-chrome-win/*.png; do
        [ -f "artifacts/tour-chrome/$(basename "$f")" ] \
            || { echo "tour: stale Windows golden with no macOS frame: $f" >&2; return 1; }
    done
    local bad=0 win=0 mac=0
    for f in "$work/A"/*.png; do
        f=$(basename "$f")
        if [ -f "artifacts/tour-chrome-win/$f" ]; then gold="artifacts/tour-chrome-win/$f"; win=$((win + 1))
        else gold="artifacts/tour-chrome/$f"; mac=$((mac + 1)); fi
        cmp -s "$work/A/$f" "$gold" || { echo "tour: MISMATCH $f vs $gold" >&2; bad=$((bad + 1)); }
    done
    echo "tour: $((na - bad))/$na frames match ($win vs Windows goldens, $mac vs macOS goldens)"
    [ "$bad" -eq 0 ]
}

note "GATE-WIN start $(date '+%Y-%m-%d %H:%M:%S %Z') HEAD $(git rev-parse --short HEAD) logs $LOGDIR"
run_step build      step_build
run_step ctest      step_ctest
run_step deny       step_deny
run_step spdx       step_spdx
run_step notices    step_notices
run_step i18n-audit step_i18n_audit
run_step fmt        step_fmt
run_step tour       step_tour
note "GATE_RC=$FAILURES (unmeasured $UNMEASURED)"
[ "$FAILURES" -eq 0 ]
