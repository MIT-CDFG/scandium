#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# separation_greps.sh — the per-checkpoint O-QA separation + purity
# re-grep (plan §3.14: "re-run separation greps on shell/ and purity on
# crates"), codified as a repeatable runner at CP-0.7 wave 2 (the shell
# grew: qml/dialogs, Theme.qml + dark chrome, packaging/bundle,
# 9-frame tour). Walled-identifier sweeps are NOT duplicated here:
# scripts/gate.sh's walled-symbols step already covers crates/ shell/
# tests/ fixtures/ i18n/ (M-6 ruling dir set).
#
# Checks (each prints PASS/FAIL; exit 1 on any FAIL):
#   S1 shell links ONLY the C ABI — no engine-crate names in shell
#      build/link files (HLD: pure C-ABI client, same door as everyone)
#   S2 shell sources/QML reference no Rust crate internals
#   S3 no Adobe marks / trade-dress names in the shell (D-43)
#   P1 engine crates free of Qt/QML reach-back (no UI includes)
#   P2 Apache-licensed crates (cabi/py) do not depend on the GPL shell
#      or reach engine internals beyond their declared deps
#
# Exit: 0 pass · 1 fail.

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
fail=0

check() { # check <name> <expected-zero-count> <desc>
    local name="$1" hits="$2" desc="$3"
    if [ "$hits" = "0" ]; then
        echo "$name PASS ($desc)"
    else
        echo "$name FAIL ($desc: $hits hit[s])"
        fail=1
    fi
}

# S1: engine crate names must not appear in shell build files (corrosion
# imports the cabi staticlib target only).
s1="$( (grep -rEc "appname[-_](core|render|io|color|tools|adjust|filters|algo|text|vector)\b" \
    shell/CMakeLists.txt shell/packaging 2>/dev/null || true) | awk -F: '{s+=$2} END{print s+0}')"
check "S1-shell-links-abi-only" "$s1" "no engine-crate names in shell build/link files"

# S2: shell C++/QML must not name Rust crate internals (the cabi header
# is the whole vocabulary).
s2="$( (grep -rEc "appname_(core|render|io|color|tools|adjust)::" shell/src shell/qml 2>/dev/null || true) | awk -F: '{s+=$2} END{print s+0}')"
check "S2-no-crate-internals" "$s2" "no Rust-internal paths in shell sources"

# S3: D-43 — no Adobe marks or product names anywhere in the shell.
s3="$( (grep -riEc "photoshop|adobe" shell 2>/dev/null || true) | awk -F: '{s+=$2} END{print s+0}')"
check "S3-trade-dress" "$s3" "no Adobe marks in shell/"

# P1: engine crates never reach for Qt/QML.
p1="$( (grep -rEc "#include <Q|QtQuick|QGuiApplication" crates 2>/dev/null || true) | awk -F: '{s+=$2} END{print s+0}')"
check "P1-crates-no-qt" "$p1" "no Qt includes under crates/"

# P2: Apache crates independent of the shell.
p2="$( (grep -rEc "shell/" crates/appname-cabi/Cargo.toml crates/appname-py/Cargo.toml 2>/dev/null || true) | awk -F: '{s+=$2} END{print s+0}')"
check "P2-apache-no-shell" "$p2" "cabi/py manifests reference no shell paths"

if [ "$fail" -ne 0 ]; then
    echo "separation-greps: FAIL"
    exit 1
fi
echo "separation-greps: PASS"
