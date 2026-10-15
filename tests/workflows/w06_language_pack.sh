#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# w06_language_pack.sh — workflow gate W06 (LIVE): the 4 shipped locale
# catalogs drive human-mode output; wire constants are never translated;
# locale switching leaves no state behind (switch_check pattern).
#
# Spec: plan workflow matrix W06; CLI i18n chain (env dir -> exe-adjacent
# -> embedded en -> key-echo); catalogs crates/appname-cli/i18n/{en,es,ja,
# pseudo}.json.
#
# Checks, per locale: a human-mode rejection prints exactly ONE stderr
# line carrying the wire code and the envelope id. Across locales: the
# ja/pseudo lines DIFFER from en (localization observably applied; the
# es `cli.error` template LEGITIMATELY coincides with en — "Error" is
# the same word in Spanish — so es asserts presence, not difference; the
# catalog-consultation proof rides on ja/pseudo). switch_check: en ->
# ja -> en on the same doc; the two en lines are byte-identical (no
# locale bleed across invocations).
#
# Exit: 0 pass · 1 fail · 2 environment not ready.

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

CLI="${APPNAME_CLI:-}"
if [ -z "$CLI" ]; then
    for cand in "$ROOT/target/debug/appname" "$ROOT/target/release/appname"; do
        [ -x "$cand" ] && CLI="$cand" && break
    done
fi
if [ -z "$CLI" ] && command -v appname >/dev/null 2>&1; then
    CLI="$(command -v appname)"
fi
if [ -z "$CLI" ]; then
    echo "SKIP: appname-cli not built (cargo build -p appname-cli)"
    exit 2
fi
I18N="$ROOT/crates/appname-cli/i18n"
[ -d "$I18N" ] || { echo "SKIP: i18n catalogs not found"; exit 2; }

WORK="$(mktemp -d "${TMPDIR:-/tmp}/appname-qa-w06-XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

fail=0
pkg="$WORK/doc.appname"
"$CLI" new --width 32 --height 32 --out "$pkg" --json > /dev/null

ENVELOPE='{"schema":"appname.command.v1","id":"ad00f060-0000-4000-8000-00000000f060","kind":"layer.bogus_kind","params":{}}'
CODE="error.command.unknown_kind"

reject_line() { # reject_line <lang> -> the single localized stderr line
    local rc=0 out
    out="$(APPNAME_I18N_DIR="$I18N" "$CLI" apply --doc "$pkg" \
        --command "$ENVELOPE" --lang "$1" 2>&1 >/dev/null)" || rc=$?
    [ "$rc" -eq 1 ] || { echo "<exit $rc>"; return; }
    [ "$(printf '%s\n' "$out" | sed '/^\s*$/d' | wc -l | tr -d ' ')" = "1" ] \
        || { echo "<multi-line>"; return; }
    printf '%s' "$out"
}

declare -a LINES
i=0
for lang in en es ja pseudo; do
    line="$(reject_line "$lang")"
    LINES[$i]="$line"
    i=$((i + 1))
    case "$line" in
        "<"*) echo "locale-$lang FAIL ($line)"; fail=1; continue ;;
    esac
    if printf '%s' "$line" | grep -q "$CODE" \
        && printf '%s' "$line" | grep -q "ad00f060-0000-4000-8000-00000000f060"; then
        echo "locale-$lang PASS (one line, wire code + id quoted)"
    else
        echo "locale-$lang FAIL (missing wire constants): $line"
        fail=1
    fi
done

# LINES indices: 0=en, 1=es, 2=ja, 3=pseudo
for i in 2 3; do
    lang="$(printf 'ja\npseudo' | sed -n "$((i - 1))p")"
    if [ "${LINES[$i]}" = "${LINES[0]}" ]; then
        echo "localized-differs-$lang FAIL (identical to en)"
        fail=1
    else
        echo "localized-differs-$lang PASS"
    fi
done
echo "locale-es note: template coincides with en by language (asserted present above)"

# switch_check: en -> ja -> en, same doc; the en lines are byte-identical
en1="$(reject_line en)"
ja1="$(reject_line ja)"
en2="$(reject_line en)"
if [ "$en1" = "$en2" ] && [ "$en1" != "$ja1" ]; then
    echo "switch_check PASS (en stable across a ja interleave)"
else
    echo "switch_check FAIL"
    fail=1
fi

if [ "$fail" -ne 0 ]; then
    echo "W06 language-pack: FAIL"
    exit 1
fi
echo "W06 language-pack: PASS"
