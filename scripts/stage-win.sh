#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# D-11 staging (O-WIN, WS-D) per reports/D-11-WINDOWS-PACKAGING.md §1–§4.
# Run inside the MSVC env, after cargo build --release and the build-win shell build:
#   scripts\win-env.cmd bash scripts/stage-win.sh <out-dir>     -> <out-dir>/Scandium/
# Build logs go to <out-dir>/logs, never into the staged folder (§5b, B-9).
# windeployqt runs WITHOUT --compiler-runtime and WITHOUT the Mesa software GL,
# nor Microsoft's D3D/DXC compilers: none is recorded in THIRD-PARTY-NOTICES
# (W-27; deps/microsoft-crt-sdk.yaml). Qt Quick's default RHI on Windows is D3D11.
# RR-W1 (O-WIN WORDs 13–16, reversing WORD 6 (1)): the MSVC runtime is NOT
# shipped. The zip carries no vcruntime/msvcp DLLs; the README states the
# prerequisite "Microsoft Visual C++ Redistributable (x64)".
set -euo pipefail
OUT="${1:?usage: stage-win.sh <out-dir>}"
QT="${APPNAME_QT_DIR:-C:/Qt/6.11.1/msvc2022_64}"
CRT="${VCToolsRedistDir:?run through scripts\\win-env.cmd (no VCToolsRedistDir)}x64/Microsoft.VC143.CRT"
[ -d "$CRT" ] || { echo "STAGE crt STOP(no redist folder: $CRT)" >&2; exit 1; }
cd "$(dirname "$0")/.."
S="$OUT/Scandium"; L="$OUT/logs"
rm -rf "$S"; mkdir -p "$S" "$L"
# The shipped engine DLL is built ALONE (-p appname-cabi), never taken from a
# --workspace build: feature unification there turns on appname-script's
# default wasm-plugins and links wasmtime + cranelift into appname_cabi.dll
# (measured on c4f665f35: 40 wasmtime / 3 cranelift strings vs 0 / 0 alone;
# the same 38 exports). deny's wasmtime ignore rests on "in no shipped binary".
cargo build --release -p appname-cabi --target-dir target/ship-cabi > "$L/cargo-cabi.log" 2>&1
cp build-win/appname-shell.exe target/ship-cabi/release/appname_cabi.dll "$S/"
"$QT/bin/windeployqt.exe" --release --qmldir shell/qml --no-compiler-runtime --no-opengl-sw \
    --no-system-d3d-compiler --no-system-dxc-compiler --no-translations "$S/appname-shell.exe" > "$L/windeployqt.log" 2>&1
mkdir -p "$S/i18n"; cp i18n/*.json "$S/i18n/"
cp LICENSE LICENSE-SDK THIRD-PARTY-NOTICES.md README.md "$S/"
mkdir -p "$S/LICENSES"; cp LICENSES/*.txt "$S/LICENSES/"
cp packaging/macos/notices/LGPL-3.0-only.txt "$S/LICENSES/LGPL-3.0-only.txt"
echo "STAGE done $S (windeployqt log: $L/windeployqt.log; no CRT DLLs — RR-W1)"
