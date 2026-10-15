#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# w08_plugin_safety.sh — workflow gate W08 (plugin safety), TRANSLATED +
# STAGED. Spec: W08-plugin-safety.yaml (§34.6 W8/§25.3/§25.5).
#
# STAGED STATUS (typed, per the CP-1.0 wave dispatch): the sandboxed
# plugins tier (wasmtime + mlua hosts, manifest permissions, capability
# gates) is IN BUILD this wave at O-SCRIPT — no wasm/lua host exists on
# main yet (probed below, not assumed). This script is the W8 harness
# ready to run the moment the tier merges; until then it exits 2 with
# the staged record. TRIGGER: the O-SCRIPT sandboxed-tier merge landing
# a plugin-host door (binary `appname-plugin-host`, a `plugin`
# subcommand on the CLI, or a crates/appname-script wasm feature).
#
# ENCODED STEPS (bind on merge; W8 letter):
#   1  install a test WASM plugin whose manifest requests no fs permission
#   2  plugin reads $HOME        -> APPNAME_ERR_PERMISSION + logged denial
#   3  plugin makes a net call   -> denied (net=deny default)
#   4  second plugin, scoped fs grant -> in-scope read OK, out-of-scope FAIL
#   5  crash the plugin          -> host document intact (state-hash)
#
# Exit: 0 pass · 1 fail · 2 staged/environment not ready.

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

CLI="${APPNAME_CLI:-}"
if [ -z "$CLI" ]; then
    for cand in "$ROOT/target/debug/appname" "$ROOT/target/release/appname"; do
        [ -x "$cand" ] && CLI="$cand" && break
    done
fi

# Probe every plausible host door, so the staged status is OBSERVED.
HOST=""
for cand in "$ROOT/target/debug/appname-plugin-host" "$ROOT/target/release/appname-plugin-host"; do
    [ -x "$cand" ] && HOST="$cand" && break
done
if [ -z "$HOST" ] && [ -n "$CLI" ] && "$CLI" plugin --help >/dev/null 2>&1; then
    HOST="$CLI plugin"
fi
if [ -z "$HOST" ] && grep -q "wasmtime\|mlua" "$ROOT/crates/appname-script/Cargo.toml" 2>/dev/null; then
    HOST="feature-present-but-undoored"
fi

if [ -z "$HOST" ]; then
    echo "W08 plugin-safety: STAGED (probed: no appname-plugin-host binary, no CLI plugin subcommand, no wasmtime/mlua dep in crates/appname-script — the sandboxed tier is IN BUILD at O-SCRIPT this wave)"
    echo "TRIGGER: the O-SCRIPT wasmtime+mlua merge; steps 1-5 are encoded above and this script grows their drivers the same wave the door lands (O-QA re-encode, same-day)"
    exit 2
fi

# The CP-1.0 tier landed in EMBEDDED LIBRARY form (crates/appname-script
# SandboxHost: wasmtime + mlua hosts, manifest permissions, capability
# gates) — there is deliberately NO CLI `plugin` verb / plugin-host
# binary door on the shipped surface (probed above; the sandbox is an
# in-process host, not a CLI subcommand). The W8 letter's steps 1-5 are
# therefore driven at the host boundary by the committed cargo suites,
# which ARE the real drivers (not CLI stand-ins):
#   plugin_hosts.rs  — wasm_tier + lua_tier: fs-of-$HOME denied (step 2),
#                      net denied (step 3), scoped grant in/out (step 4),
#                      one-door journal + fuel/deadline/memory traps
#                      (step 5 crash-isolation), typed bad-entry.
#   plugin_safety.rs — the hostile-plugin corpus (step 1 install/scan),
#                      no-permission $HOME denial, symlink/traversal
#                      escapes denied, the sealed-closure registration
#                      gate, one-door history landing.
# O-QA fence: run them here so W08 is genuinely runnable+green on the
# candidate, gating on real test output (never a hand-authored pass).
if [ "$HOST" = "feature-present-but-undoored" ]; then
    echo "W08 plugin-safety: library-form sandbox tier detected (no CLI plugin door by design) — driving steps 1-5 through the committed host boundary suites:"
    if cargo test --manifest-path "$ROOT/Cargo.toml" -p appname-script \
            --test plugin_hosts --test plugin_safety; then
        echo "W08 plugin-safety: PASS (plugin_hosts + plugin_safety suites green — capability gates, hostile corpus, sealed Lua closure, crash isolation)"
        exit 0
    fi
    echo "W08 plugin-safety: FAIL (a plugin host-boundary suite reported a failure — see output above)"
    exit 1
fi

echo "plugin host door FOUND ($HOST) — a CLI/binary plugin door shipped; the CLI-level step 1-5 drivers must now be encoded against it. Failing loudly rather than green-lighting an undriven CLI tier:"
echo "W08 plugin-safety: FAIL (CLI host door merged but its CLI drivers not yet encoded — O-QA action required)"
exit 1
