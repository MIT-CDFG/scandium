#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# w23_rtl_mid_edit_switch.sh — workflow gate W23 (RTL mid-edit switch,
# extends W6): the ENGINE-SIDE legs now, the ui-harness legs staged.
# Spec: W23-rtl-mid-edit-switch.yaml (§41.6/§27.2/§27.3/§0.3/§7.6).
#
# RUNNABLE NOW (the step-6 byte-identity contract — "locale MUST NOT
# influence pixels or file bytes"):
#   A  the same edit script run under en / ja / pseudo (--lang + the
#      APPNAME_LANG env door) produces BYTE-IDENTICAL exports;
#   B  the saved .appname packages are byte-identical across locales
#      (canonical serializer + ASCII digits, §7.6/§0.3);
#   C  the runtime language-pack surface re-proven via the W06 gate
#      (W23 extends W6), plus --json locale-independence (identical
#      machine error codes under en and pseudo).
#
# STAGED/BLOCKED (typed):
#   * ar pack: i18n/ carries en/es/ja/pseudo — NO ar catalog yet.
#     TRIGGER: O-I18N-A11Y ar pack (the CP-1.0 completion sweep, in
#     flight this wave). The RTL-script byte-identity leg then rides
#     the same harness line below (lang list is data).
#   * steps 1-5+7 (mirroring probe, toast snapshot rule, mid-hover
#     switch, shaped Arabic text run, Eastern-Arabic digit fields,
#     menu-tree qps-ploc dump): ui-harness rows (runner: ui-harness,
#     §41.1). TRIGGER: the O-UI screenshot-tour/probe harness + the
#     a11y/i18n completion merge (in flight). W23 stays PARTIAL until
#     then — never claimed whole.
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
[ -n "$CLI" ] || { echo "SKIP: appname-cli not built"; exit 2; }

WORK="$(mktemp -d "${TMPDIR:-/tmp}/appname-qa-w23-XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
fail=0

LANGS="en ja pseudo"
if [ -f "$ROOT/i18n/ar.json" ]; then
    LANGS="$LANGS ar"
    echo "ar-pack PRESENT (joins the identity matrix)"
else
    echo "ar-pack BLOCKED (i18n/ has en/es/ja/pseudo only) — TRIGGER: O-I18N-A11Y ar catalog; this harness picks it up from i18n/ automatically"
fi

# One edit script (dirty doc: stroke-stand-in fill + a second fill —
# the W23 step-1 two-history-entry shape at the headless surface).
mkops() {
    printf '{"schema":"appname.command.v1","id":"ad00f230-0000-4000-8000-000000002301","kind":"fill","target":{"layerId":"%s"},"params":{"content":"foreground","opacity":0.7}}\n{"schema":"appname.command.v1","id":"ad00f230-0000-4000-8000-000000002302","kind":"fill","target":{"layerId":"%s"},"params":{"content":"background","opacity":0.4}}\n' "$1" "$1"
}

# ONE base document (§41.6 letter: "save .appname under each locale FROM
# THE SAME STATE" — document identity is part of the bytes, so every
# locale edits a copy of the SAME base, never its own doc.new).
base="$WORK/base.appname"
"$CLI" new --width 64 --height 48 --background white --out "$base" --json > "$WORK/base.new.json"
layer="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["receipt"]["createdIds"][-1])' "$WORK/base.new.json")"
mkops "$layer" > "$WORK/ops.jsonl"

first_export=""
first_pkg=""
for lang in $LANGS; do
    pkg="$WORK/$lang.appname"
    cp "$base" "$pkg"
    APPNAME_LANG="$lang" "$CLI" --lang "$lang" script --doc "$pkg" --file "$WORK/ops.jsonl" --json > /dev/null
    "$CLI" --lang "$lang" export --doc "$pkg" --format png --out "$WORK/$lang.png" --json > /dev/null
    if [ -z "$first_export" ]; then
        first_export="$(sha "$WORK/$lang.png")"
        first_pkg="$(sha "$pkg")"
    else
        [ "$(sha "$WORK/$lang.png")" = "$first_export" ] \
            || { echo "export-identity FAIL ($lang differs)"; fail=1; }
        [ "$(sha "$pkg")" = "$first_pkg" ] \
            || { echo "package-identity FAIL ($lang differs)"; fail=1; }
    fi
done
[ "$fail" -eq 0 ] && echo "byte-identity PASS (exports AND .appname packages byte-identical across: $LANGS — locale never touches pixels or file bytes)"

# leg C — the runtime language-pack surface is the W06 gate; W23 extends
# it, so re-run it here rather than re-inventing its asserts.
HERE="$(cd "$(dirname "$0")" && pwd)"
if out="$(APPNAME_CLI="$CLI" bash "$HERE/w06_language_pack.sh" 2>&1)"; then
    echo "catalog-switch PASS (W06 language-pack gate green in this environment)"
else
    rc=$?
    if [ "$rc" -eq 2 ]; then
        echo "catalog-switch SKIP (W06 environment not ready)"
    else
        echo "catalog-switch FAIL (W06 gate red):"
        echo "$out" | tail -3
        fail=1
    fi
fi
J1="$("$CLI" --lang en --json export --doc "$WORK/does-not-exist.appname" --format png --out "$WORK/x.png" 2>/dev/null || true)"
J2="$("$CLI" --lang pseudo --json export --doc "$WORK/does-not-exist.appname" --format png --out "$WORK/x.png" 2>/dev/null || true)"
if [ "$(printf '%s' "$J1" | python3 -c 'import json,sys; print(json.load(sys.stdin)["error"]["code"])')" = \
     "$(printf '%s' "$J2" | python3 -c 'import json,sys; print(json.load(sys.stdin)["error"]["code"])')" ]; then
    echo "json-locale-independence PASS (identical machine error code under en and pseudo)"
else
    echo "json-locale-independence FAIL"
    fail=1
fi

echo "ui-harness legs (mirroring / toast snapshot / mid-hover switch / Arabic shaping / Eastern-Arabic digits / qps-ploc menu dump) STAGED — TRIGGER: O-UI probe harness + i18n-a11y completion merge (runner: ui-harness per §41.1)"

if [ "$fail" -ne 0 ]; then
    echo "W23 rtl-mid-edit-switch: FAIL"
    exit 1
fi
echo "W23 rtl-mid-edit-switch: PASS (engine-side legs; ui legs staged typed)"
