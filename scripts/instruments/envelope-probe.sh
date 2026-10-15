#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
# envelope-probe.sh — OWNER: O-INT. Applies ONE envelope to a COPY of a document
# (RENAMED 2026-08-31 from demo-probe.sh, operator ruling Q-D17 -- the probe was never
#  demo-scoped; it answers accepted-vs-composite-changed for ANY envelope.)
# and reports whether it was accepted AND whether it changed the composite.
#
# ★★★★★ WHY THIS EXISTS. Three times in one session I measured a command
# while redirecting its errors, and each time the silence read as success:
#   * `asset add` reported an id into a void and the next apply could not
#     resolve it;
#   * a test binary was run after a build that had FAILED, and its "33
#     passed" was the previous binary's;
#   * a gradient `layer.add` was REFUSED, so the composite stayed at the
#     untouched baseline, and comparing it against a rendered fill
#     "confirmed" the claim under test.
# `2>/dev/null turns a fatal into a zero` is already rule §10 in my orders.
# It was written down, restated, and did not bind on any of the three.
#
# ★★★ SO THE REMEDY IS NOT A FOURTH STATEMENT OF THE RULE. It is this: a
# probe that CANNOT be asked to hide the answer, and that refuses to report
# a pixel comparison it has no right to make.
#
# Usage:  envelope-probe.sh <doc> <envelope-json> [label]
#         envelope-probe.sh --selftest
# Exit:   0 accepted+changed · 1 accepted+UNCHANGED · 2 REFUSED · 3 VOID
set -uo pipefail
CLI="${CLI:-./target/release/appname}"

_hash() { shasum -a 256 "$1" 2>/dev/null | cut -d' ' -f1; }

_probe() {
    _doc="$1"; _env="$2"; _label="${3:-envelope}"
    [ -f "$_doc" ] || { echo "VOID - no document at $_doc"; return 3; }
    [ -x "$CLI" ] || { echo "VOID - no CLI at $CLI"; return 3; }
    _work=$(mktemp -d) || return 3
    cp "$_doc" "$_work/d.appname" || { rm -rf "$_work"; return 3; }

    # BEFORE. An export that fails leaves no hash, and an empty hash is
    # never compared -- it is reported as UNMEASURED, because "two empty
    # strings are equal" is exactly how a broken probe says "unchanged".
    "$CLI" export --doc "$_work/d.appname" --format png --out "$_work/b.png" >/dev/null 2>&1
    _b=$(_hash "$_work/b.png")

    # APPLY. stderr is DELIBERATELY joined, never discarded: the refusal
    # text is the most informative line this probe can print.
    _out=$("$CLI" apply --doc "$_work/d.appname" --command "$_env" 2>&1 | grep -v '^warning:' | tail -1)
    case "$_out" in
        Error*|*"error."*)
            echo "$_label: REFUSED - $_out"
            rm -rf "$_work"; return 2 ;;
    esac

    "$CLI" export --doc "$_work/d.appname" --format png --out "$_work/a.png" >/dev/null 2>&1
    _a=$(_hash "$_work/a.png")
    rm -rf "$_work"

    if [ -z "$_b" ] || [ -z "$_a" ]; then
        echo "$_label: UNMEASURED - an export failed, so no pixel comparison is possible"
        return 3
    fi
    if [ "$_b" = "$_a" ]; then
        echo "$_label: ACCEPTED but composite UNCHANGED ($(printf '%s' "$_b" | cut -c1-12)) - accepted is not effect"
        return 1
    fi
    echo "$_label: ACCEPTED and composite CHANGED ($(printf '%s' "$_b" | cut -c1-12) -> $(printf '%s' "$_a" | cut -c1-12))"
    return 0
}

if [ "${1:-}" = "--selftest" ]; then
    fails=0
    _doc="${SELFTEST_DOC:-demo/DEMO.appname}"
    [ -f "$_doc" ] || { echo "envelope-probe selftest: VOID - no $_doc"; exit 3; }
    # ARM 1 - a MALFORMED envelope must report REFUSED, not "unchanged".
    # This is the exact failure the probe exists for: the first draft of my
    # gradient test read a refusal as a clean comparison.
    o=$(_probe "$_doc" '{"schema":"appname.command.v1","id":"not-a-uuid","kind":"layer.add","params":{}}' arm1); r=$?
    case "$o$r" in *REFUSED*2) : ;; *) echo "SELFTEST FAIL arm1: a malformed envelope did not report REFUSED ($o)"; fails=$((fails+1));; esac
    # ARM 2 - a REAL no-op must report ACCEPTED-but-UNCHANGED, rc 1. Heal
    # onto a transparent layer is the canonical one and it is why this
    # distinction exists at all.
    o=$(_probe "$_doc" '{"schema":"appname.command.v1","id":"11111111-2222-4000-8000-000000000009","kind":"layer.add","params":{"kind":"raster","newLayerId":"11111111-2222-4000-8000-00000000000a"}}' arm2); r=$?
    case "$r" in 1) : ;; *) echo "SELFTEST FAIL arm2: an empty new layer changed the composite or errored (rc=$r: $o)"; fails=$((fails+1));; esac
    # ARM 3 - MUST-FAIL control: if _hash of a missing file returned
    # something comparable, arm 2 would pass for the wrong reason.
    [ -z "$(_hash /nonexistent/file.png)" ] || { echo "SELFTEST FAIL arm3: _hash invented a value for a missing file"; fails=$((fails+1)); }
    [ "$fails" -eq 0 ] && { echo "envelope-probe selftest: 3 arms, 0 failed"; exit 0; }
    echo "envelope-probe selftest: $fails FAILED"; exit 3
fi

_probe "${1:?usage: envelope-probe.sh <doc> <envelope-json> [label]}" "${2:?envelope required}" "${3:-envelope}"
exit $?
