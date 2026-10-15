#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# Runtime language-switch evidence (plan §2.2 row "i18n from day 1":
# "en/es/ja/pseudo live; runtime switch"). Builds the CLI and proves the
# already-shipped --lang / APPNAME_LANG / APPNAME_I18N_DIR machinery
# renders ONE localized stderr line per locale from THE CANONICAL
# top-level i18n/ catalogs — byte-distinct across all four locales,
# placeholders substituted (no `{code}`/`{message}` survives), the wire
# code verbatim inside the translated frame (wire data is never
# translated), and no key-echo (the catalogs really loaded).
#
# The rejection used is deterministic and filesystem-independent:
# `export --doc <absent>.appname` refuses on the missing input package
# with `error.io.read_failed` (the guaranteed-absent path lives in a
# fresh mktemp dir), exit 2 (the CLI's runtime-error exit).
#
# HISTORY (2026-08-19, A11Y-3 round): the original probe expected
# `export --format jpeg` to refuse with `error.command.not_implemented`
# BEFORE opening the document. That premise is now OBSOLETE upstream —
# jpeg export is IMPLEMENTED (a codec landed; `export --format jpeg` on
# a valid doc now succeeds, "Exported … (jpeg, N B)", exit 0), so no NI
# refusal fires. Retargeted to the absent-doc read failure, which is
# still deterministic and still renders one localized wire-coded error
# line per locale. This is switch EVIDENCE only; the codec/verb-behavior
# change is flagged cross-fence to O-INT (reports/O-I18N-A11Y-log.md).
#
# Also evidenced: the env route (APPNAME_LANG=ja == --lang ja) and
# precedence (--lang es beats APPNAME_LANG=ja).
#
# NOT wired into scripts/gate.sh — that wiring (with a pseudolocalization
# gate) is O-BUILD's, flagged as a handoff in reports/O-I18N-A11Y-log.md.
#
# Usage: bash i18n/tools/switch_check.sh   (exit 0 = PASS, 1 = FAIL)

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

# toolchain self-locate, same quirk note as scripts/gate.sh
if ! command -v cargo >/dev/null 2>&1; then
    export PATH="$HOME/.cargo/bin:$PATH"
fi

echo "switch_check: building appname-cli..."
cargo build -q -p appname-cli
BIN="$ROOT/target/debug/appname"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/appname-switch.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

export APPNAME_I18N_DIR="$ROOT/i18n"
WIRE_CODE="error.io.read_failed"
FAILED=0

fail() {
    echo "FAIL: $1" >&2
    FAILED=1
}

run_rejection() {
    # $1 = output file for the stderr line; remaining args = CLI prefix args
    local out="$1"
    shift
    local rc=0
    "$BIN" "$@" export --doc "$WORK/absent.appname" --format jpeg \
        --out "$WORK/x.png" >"$WORK/stdout.txt" 2>"$out" || rc=$?
    if [ "$rc" -ne 2 ]; then
        fail "expected exit 2 (CLI runtime error), got $rc (args: $*)"
    fi
    if [ -s "$WORK/stdout.txt" ]; then
        fail "human-mode rejection must print to stderr only (args: $*)"
    fi
}

check_line() {
    # $1 = locale label, $2 = file holding the captured stderr
    local loc="$1" file="$2" lines line
    lines="$(grep -c . "$file" || true)"
    if [ "$lines" -ne 1 ]; then
        fail "$loc: expected exactly one non-empty stderr line, got $lines"
        return
    fi
    line="$(cat "$file")"
    case "$line" in
        *"$WIRE_CODE"*) : ;;
        *) fail "$loc: wire code $WIRE_CODE not verbatim in: $line" ;;
    esac
    case "$line" in
        *'{code}'*|*'{message}'*) fail "$loc: unsubstituted placeholder in: $line" ;;
    esac
    case "$line" in
        cli.error*) fail "$loc: key-echo detected (catalog did not load): $line" ;;
    esac
    echo "  $loc: $line"
}

echo "switch_check: APPNAME_I18N_DIR=$APPNAME_I18N_DIR"
for loc in en es ja pseudo; do
    run_rejection "$WORK/$loc.txt" --lang "$loc"
    check_line "$loc" "$WORK/$loc.txt"
done

# byte-distinct across all four locales, pairwise
for a in en es ja pseudo; do
    for b in en es ja pseudo; do
        if [ "$a" != "$b" ] && cmp -s "$WORK/$a.txt" "$WORK/$b.txt"; then
            fail "locales $a and $b render byte-identically"
        fi
    done
done

# env route: APPNAME_LANG=ja (no --lang) matches --lang ja
APPNAME_LANG=ja run_rejection "$WORK/env-ja.txt"
cmp -s "$WORK/env-ja.txt" "$WORK/ja.txt" \
    || fail "APPNAME_LANG=ja differs from --lang ja"
echo "  env route: APPNAME_LANG=ja == --lang ja"

# precedence: --lang es beats APPNAME_LANG=ja
APPNAME_LANG=ja run_rejection "$WORK/prec-es.txt" --lang es
cmp -s "$WORK/prec-es.txt" "$WORK/es.txt" \
    || fail "--lang es under APPNAME_LANG=ja differs from plain --lang es"
echo "  precedence: --lang es beats APPNAME_LANG=ja"

if [ "$FAILED" -ne 0 ]; then
    echo "switch_check: FAIL"
    exit 1
fi
echo "switch_check: PASS — 4 locales byte-distinct, placeholders intact, wire code verbatim, env route + precedence proven"
