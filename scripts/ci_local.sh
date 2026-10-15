#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# CI-style local runner — plan §2.1 item 7. Superset of scripts/gate.sh:
# adds formatting and lint discipline on top of the gate. Run before handing
# a branch to O-INT.
#   fmt     cargo fmt --check (all workspace crates)
#   clippy  cargo clippy --workspace --all-targets -- -D warnings
#   switch  i18n/tools/switch_check.sh — O-I18N's runtime language-switch
#           evidence (invocation-only wiring; ci_local tier per the O-INT
#           ruling — heavier than the audit, belongs at lane handoff).
#   notices-selftest  tools/selftest_gen_notices.sh — the notices-mechanism
#           proof harness: port-record activation both ways, vendoredTexts
#           pin carriage (incl. pre-landing --assume-shipped activation for
#           freetype-sys/harfbuzz-sys) and the tampered-pin red paths. Runs
#           several full generations, so ci_local tier, not the gate; the
#           gate's notices step still hard-fails on real drift every merge.
#   gate    scripts/gate.sh (build, test, deny incl. advisories, spdx,
#           contract-sync, notices, i18n-audit, walled-symbols)
# The notices drift check moved INTO gate.sh (CP-0.7 wave-1 review,
# notices-mechanization MAJOR): port-obligation activation is
# landing-detected, so a stale notices file post-merge on main must be red
# in the merge train, not only at lane handoff.
# The deny step inside gate.sh already includes `advisories` (network fetch
# probed live and green at bootstrap 2026-08-11), so no separate advisories
# stage is needed here.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if ! command -v cargo >/dev/null 2>&1; then
    export PATH="$HOME/.cargo/bin:$PATH"
fi

FAILURES=0
SUMMARY=""

note() {
    SUMMARY="${SUMMARY}${1}
"
    echo "$1"
}

run_step() {
    local name="$1"
    shift
    if "$@"; then
        note "CI $name PASS"
    else
        note "CI $name FAIL"
        FAILURES=$((FAILURES + 1))
    fi
}

run_step fmt     cargo fmt --check
run_step clippy  cargo clippy --workspace --all-targets -- -D warnings
run_step switch  bash i18n/tools/switch_check.sh
run_step notices-selftest bash tools/selftest_gen_notices.sh
run_step gate    bash scripts/gate.sh

echo
echo "==== ci_local summary ===="
printf '%s' "$SUMMARY"
if [ "$FAILURES" -ne 0 ]; then
    echo "ci_local: $FAILURES stage(s) FAILED"
    exit 1
fi
echo "ci_local: all stages PASS"
