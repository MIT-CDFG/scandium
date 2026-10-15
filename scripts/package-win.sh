#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# D-11 presence gate for the Windows zip (O-WIN, WS-D) — the counterpart of
# build_dmg.sh stages 0 and 4b, verbatim from reports/D-11-WINDOWS-PACKAGING.md
# §5 + §5b. Gate the STAGED folder; never zip an un-noticed artifact.
#   bash scripts/package-win.sh <staged-folder>      (the folder that becomes Scandium\)
set -euo pipefail
STAGE="${1:?usage: package-win.sh <staged-folder>}"
say() { echo "ZIP $1"; }
fail() { say "$1"; exit 1; }

# The folder itself first — a typo'd path must not report "missing 'LICENSE'".
[ -d "$STAGE" ] || fail "stage STOP(no such staged folder: $STAGE)"

for f in LICENSE LICENSE-SDK THIRD-PARTY-NOTICES.md; do
    [ -f "$STAGE/$f" ] || fail "notices STOP(staged folder missing '$f' — aud-rights R-01)"
done
# D-11 §4 (corrected 2026-10-06): each artifact carries its own notices. The
# repo-root file is the macOS one; the zip must carry gen_notices.py --platform
# windows output.
grep -qx '\*\*Platform: Windows\.\*\* .*' "$STAGE/THIRD-PARTY-NOTICES.md" \
    || fail "notices STOP(THIRD-PARTY-NOTICES.md is not the Windows notice set — generate it with tools/gen_notices.py --platform windows; the repo-root file is macOS's)"
[ -d "$STAGE/LICENSES" ] || fail "notices STOP(staged folder missing LICENSES/ SPDX texts — R-01)"
[ -f "$STAGE/LICENSES/LGPL-3.0-only.txt" ] \
    || fail "notices STOP(missing Qt LGPL-3.0-only text — Qt is bundled LGPL-3.0-only per deps/qt6.yaml; REQUIRED carriage. Do NOT invent it)"
[ -f "$STAGE/appname-shell.exe" ] || fail "exe STOP(no appname-shell.exe in the staged folder)"
[ -f "$STAGE/i18n/en.json" ]      || fail "i18n STOP(i18n/ is not exe-adjacent — the app would silently fall back to English)"
# LGPL §4(d): Qt must be present as REPLACEABLE DLLs, never absorbed.
ls "$STAGE"/Qt6Core.dll >/dev/null 2>&1 \
    || fail "qt-deploy STOP(no Qt6Core.dll beside the exe — windeployqt did not run, or Qt was statically absorbed; LGPL relink-ability REQUIRES separate DLLs)"
# RR-W1 (O-WIN WORDs 13–16, reversing WORD 6 (1)): the MSVC runtime is NOT
# shipped — the README states the Visual C++ Redistributable (x64) prerequisite.
# A CRT DLL in the folder is a stray, never a requirement.
CRTSTRAY="$(find "$STAGE" \( -iname 'vcruntime140*.dll' -o -iname 'msvcp140*.dll' \) -print)"
[ -z "$CRTSTRAY" ] || fail "crt STOP(MSVC runtime DLLs in the staged folder — RR-W1 ships none: $(printf '%s' "$CRTSTRAY" | tr '\n' ' '))"
grep -q 'Visual C++ Redistributable' "$STAGE/README.md" 2>/dev/null \
    || fail "crt STOP(README.md does not state the Visual C++ Redistributable (x64) prerequisite — RR-W1)"
# WORD 7 (1): deny's dated wasmtime/cranelift ignore holds only while neither is
# in a shipped binary. Byte search over every staged PE; serde_json (linked in
# the engine) is the control that proves the search sees linked crates.
PE="$(find "$STAGE" \( -iname '*.exe' -o -iname '*.dll' \))"
CTRL=0; WASM=""
while IFS= read -r f; do
    grep -aqi 'serde_json' "$f" && CTRL=1
    grep -aqiE 'wasmtime|cranelift' "$f" && WASM="$WASM $f"
done <<< "$PE"
[ "$CTRL" = 1 ] || fail "wasm STOP(control failed: serde_json found in no staged binary — the search is blind)"
[ -z "$WASM" ] || fail "wasm STOP(wasmtime/cranelift linked into a shipped binary — deny's ignore premise fails:$WASM)"
# §5b (B-9): nothing that is not the product may reach the recipient. Recursive;
# WORD 6 (1) keeps vc_redist, opengl32sw and d3dcompiler_47 out, W-27 (a) DXC.
STRAY="$(find "$STAGE" \( -name '*.log' -o -name '*.pdb' -o -name '*.ilk' -o -name '*.exp' -o -name '*.lib' \
        -o -name 'Thumbs.db' -o -name 'desktop.ini' -o -iname 'vc_redist*' -o -iname 'opengl32sw.dll' \
        -o -iname 'd3dcompiler_47.dll' -o -iname 'dxcompiler.dll' -o -iname 'dxil.dll' \) -print)"
[ -z "$STRAY" ] || fail "stage STOP(non-product files in the staging folder — these would ship: $(printf '%s' "$STRAY" | tr '\n' ' '))"
say "notices OK($(find "$STAGE/LICENSES" -type f | wc -l | tr -d ' ') SPDX texts + 3 top-level, incl. the Qt LGPL text)"
say "layout OK(exe + exe-adjacent i18n/ + replaceable Qt DLLs; no CRT DLLs, prerequisite in README — RR-W1)"
say "done"
