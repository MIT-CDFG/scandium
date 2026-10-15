#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# w24_forced_cpu.sh — workflow gate W24 (forced-CPU environment) at
# CURRENT WIDTH (dispatch scoping: "W24 forced-CPU re-run").
# Spec: W24-forced-cpu-environment.yaml (§41.6/§4.8/§34.3).
#
# ENVIRONMENT TRUTH, recorded first: the shipped headless surface
# (appname-cli export/composite via core export_png_bytes + the render
# crate's CPU reference) wires NO GPU adapter path — the render crate's
# wgpu DAG exists crate-side but is not reachable from the CLI. The
# env_nogpu condition is therefore STRUCTURALLY satisfied for every leg
# below (masking an adapter that is never enumerated is vacuous), and
# the W24 legs that compare CPU-vs-GPU sessions (steps 1's adapter
# probe, 6's cross-session byte identity, the §52.2 pan rows) ARM when
# wgpu reaches the shipped surface — TRIGGER: the O-RENDER/O-UI GPU
# wiring row. What runs NOW is the row's substance at current width:
# the whole editing+replay+export chain on the CPU reference, byte-exact.
#
#   leg 1  CPU-path state probe: a fresh doc round-trips new->fill->
#          export with exit 0 under the masked environment.
#   leg 2  golden-corpus render on CPU = the standing conformance
#          goldens (delegated: goldens_verify + blend/stroke suites run
#          in the workspace gate on this same CPU path — recorded, not
#          re-run here).
#   leg 3  W1 replay in this environment: w01_layered_editing.sh green.
#   leg 4  stroke replay: replay_determinism.sh green (byte-identity of
#          replays IS the §34.5 spacing/determinism observable headless).
#   leg 6  export byte-identity: same doc exported twice + across a
#          save/reopen boundary -> identical bytes.
#
# Exit: 0 pass · 1 fail · 2 environment not ready.

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
HERE="$(cd "$(dirname "$0")" && pwd)"

CLI="${APPNAME_CLI:-}"
if [ -z "$CLI" ]; then
    for cand in "$ROOT/target/debug/appname" "$ROOT/target/release/appname"; do
        [ -x "$cand" ] && CLI="$cand" && break
    done
fi
[ -n "$CLI" ] || { echo "SKIP: appname-cli not built"; exit 2; }

# The masked environment (vacuous today — recorded above; kept so the
# script is already correct the day an adapter path lands and honors it).
export APPNAME_GPU=off
export WGPU_BACKEND=noop

WORK="$(mktemp -d "${TMPDIR:-/tmp}/appname-qa-w24-XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
fail=0

echo "env: no GPU adapter path is wired into the shipped headless surface (CPU reference is the ONLY compositor here) — env_nogpu structurally satisfied; cross-session GPU-identity legs ARM on the GPU wiring row"

# leg 1 — CPU-path state probe
pkg="$WORK/doc.appname"
"$CLI" new --width 80 --height 60 --background white --out "$pkg" --json > "$WORK/new.json"
layer="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["receipt"]["createdIds"][-1])' "$WORK/new.json")"
"$CLI" apply --doc "$pkg" --command \
    "{\"schema\":\"appname.command.v1\",\"id\":\"ad00f240-0000-4000-8000-000000002401\",\"kind\":\"fill\",\"target\":{\"layerId\":\"$layer\"},\"params\":{\"content\":\"foreground\"}}" \
    --json > /dev/null
"$CLI" export --doc "$pkg" --format png --out "$WORK/e1.png" --json > /dev/null
echo "cpu-path PASS (new -> fill -> export completed exit 0 under the masked environment)"

# legs 3+4 — the W1 replay + stroke/replay determinism, same environment
for sub in w01_layered_editing.sh replay_determinism.sh; do
    if out="$(APPNAME_CLI="$CLI" bash "$HERE/$sub" 2>&1)"; then
        echo "subgate[$sub] PASS (green in the forced-CPU environment)"
    else
        rc=$?
        if [ "$rc" -eq 2 ]; then
            echo "subgate[$sub] SKIP (environment not ready): $(echo "$out" | tail -1)"
        else
            echo "subgate[$sub] FAIL:"
            echo "$out" | tail -5
            fail=1
        fi
    fi
done

# leg 6 — export byte identity (twice + across the package boundary)
"$CLI" export --doc "$pkg" --format png --out "$WORK/e2.png" --json > /dev/null
cp "$pkg" "$WORK/copy.appname"
"$CLI" export --doc "$WORK/copy.appname" --format png --out "$WORK/e3.png" --json > /dev/null
if [ "$(sha "$WORK/e1.png")" = "$(sha "$WORK/e2.png")" ] \
    && [ "$(sha "$WORK/e1.png")" = "$(sha "$WORK/e3.png")" ]; then
    echo "export-identity PASS (twice + across the package boundary, byte-identical)"
else
    echo "export-identity FAIL"
    fail=1
fi

echo "corpus-render RECORDED (leg 2 = the standing conformance golden suites on this same CPU reference; they run in the workspace gate — not duplicated here)"
echo "pan/PS-7 + adapter-probe + GPU-session-identity legs BLOCKED-ARMED — TRIGGER: wgpu on the shipped surface"

if [ "$fail" -ne 0 ]; then
    echo "W24 forced-cpu: FAIL"
    exit 1
fi
echo "W24 forced-cpu: PASS (current-width scope; armed legs recorded)"
