#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# Reproducible-build recipe + local two-build determinism check.
# ES Appendix O.1 ("tools/repro-build.sh") + §33.4 reproducible-build knobs.
# Plan §2.5 "Release engineering": the reproducible-build CHECK.
#
# HONEST SCOPE. The full O.1 recipe is a release-pipeline artifact: an
# offline `cargo --offline` build from the `cargo vendor`ed source archive,
# inside a pinned CI container with a recorded image digest, across six
# OS/arch legs. This host has none of that infrastructure. What this script
# IS: (1) the committed environment-normalization recipe (the O.1
# "Environment normalization" table, exported before any tool runs), so a
# release pipeline and a third-party verifier run the SAME knobs; and (2) a
# runnable LOCAL determinism check — build the same shipped binary TWICE
# into separate target dirs with the normalized environment and compare
# byte-for-byte, recording exactly what is and is not reproducible on this
# host. An honest "these bytes differ" recorded here beats a claimed pass.
#
# The verification counterpart is tools/repro-verify.sh (O.1); the
# comparable-set allow-list is tools/repro/allowlist.toml (O.1/O.5).
#
# USAGE:
#   tools/repro-build.sh                 two-build determinism check (default)
#   tools/repro-build.sh --print-env     print the normalized environment only
#   tools/repro-build.sh --leg <os-arch> (accepted for O.5 CLI shape; this
#                                         host builds the host leg only)

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if ! command -v cargo >/dev/null 2>&1; then
    export PATH="$HOME/.cargo/bin:$PATH"
fi

# ---- O.1 environment normalization ----------------------------------------
# SOURCE_DATE_EPOCH: the release tag's commit timestamp. No tag here, so the
# HEAD commit time stands in (a release run passes the tag's %ct).
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct HEAD 2>/dev/null || echo 0)}"
# RUSTFLAGS: strip absolute paths from binaries + debug info (O.1; moving to
# Cargo `trim-paths` when stable). CARGO_HOME remap keeps registry paths out.
CARGO_HOME_DIR="${CARGO_HOME:-$HOME/.cargo}"
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$PWD=/build --remap-path-prefix=$CARGO_HOME_DIR=/cargo"
export LC_ALL=C.UTF-8
export TZ=UTC
umask 022

print_env() {
    echo "SOURCE_DATE_EPOCH=$SOURCE_DATE_EPOCH"
    echo "RUSTFLAGS=$RUSTFLAGS"
    echo "LC_ALL=$LC_ALL"
    echo "TZ=$TZ"
    echo "umask=$(umask)"
    echo "profile=release (no dist profile defined in this workspace yet; §33.4 dist adds strip/codegen-units/LTO)"
}

# --print-env-sh: the SAME values as --print-env, emitted shell-evaluable so a
# consumer can `eval` them instead of copying the knobs (finding OB-11). The
# release path and the reproducibility path must not drift: build_dmg.sh reads
# its normalization from here rather than restating it, because a value copied
# into a second place is a value that will eventually disagree with the first.
# Deliberately omits `profile=`, which is prose, and emits umask as a command.
print_env_sh() {
    echo "export SOURCE_DATE_EPOCH=$(printf '%q' "$SOURCE_DATE_EPOCH")"
    echo "export RUSTFLAGS=$(printf '%q' "$RUSTFLAGS")"
    echo "export LC_ALL=$(printf '%q' "$LC_ALL")"
    echo "export TZ=$(printf '%q' "$TZ")"
    echo "umask 022"
}
if [ "${1:-}" = "--print-env" ]; then
    print_env
    exit 0
fi
if [ "${1:-}" = "--print-env-sh" ]; then
    print_env_sh
    exit 0
fi
if [ "${1:-}" = "--leg" ]; then
    echo "repro-build: --leg ${2:-?} accepted; this host builds the host leg ($(uname -s)-$(uname -m)) only"
fi

# ---- two-build determinism check ------------------------------------------
# The shipped CLI binary `appname` is the comparison subject: a real release
# artifact, self-contained, fastest of the shipped set to relink.
SUBJECT_BIN="appname"
DIR_A="target/repro-a"
DIR_B="target/repro-b"

echo "==== repro-build: normalized environment ===="
print_env
echo

echo "==== build A ($DIR_A) ===="
cargo build --release --target-dir "$DIR_A" -p appname-cli --bin "$SUBJECT_BIN" 2>&1 | tail -3
echo "==== build B ($DIR_B) ===="
cargo build --release --target-dir "$DIR_B" -p appname-cli --bin "$SUBJECT_BIN" 2>&1 | tail -3

A="$DIR_A/release/$SUBJECT_BIN"
B="$DIR_B/release/$SUBJECT_BIN"
[ -f "$A" ] && [ -f "$B" ] || { echo "repro-build: FAIL — a build did not produce $SUBJECT_BIN"; exit 1; }

echo
echo "==== compare ===="
SHA_A="$(shasum -a 256 "$A" | cut -d' ' -f1)"
SHA_B="$(shasum -a 256 "$B" | cut -d' ' -f1)"
echo "A sha256: $SHA_A  ($(wc -c < "$A" | tr -d ' ') bytes)"
echo "B sha256: $SHA_B  ($(wc -c < "$B" | tr -d ' ') bytes)"

if [ "$SHA_A" = "$SHA_B" ]; then
    echo "REPRO PASS: $SUBJECT_BIN is byte-identical across two normalized builds on this host ($(uname -s)-$(uname -m))"
    exit 0
fi

# Characterize the difference honestly (O.5 self-triage). On macOS the usual
# benign cause is LC_UUID; report the differing extent + whether the diff is
# UUID-only.
echo "REPRO DIFF: $SUBJECT_BIN differs between the two normalized builds — characterizing:"
DIFF_BYTES="$(cmp -l "$A" "$B" 2>/dev/null | wc -l | tr -d ' ')"
echo "  differing bytes: $DIFF_BYTES of $(wc -c < "$A" | tr -d ' ')"
if command -v otool >/dev/null 2>&1; then
    UUID_A="$(otool -l "$A" 2>/dev/null | awk '/LC_UUID/{getline; print $2}')"
    UUID_B="$(otool -l "$B" 2>/dev/null | awk '/LC_UUID/{getline; print $2}')"
    echo "  LC_UUID A: ${UUID_A:-<none>}"
    echo "  LC_UUID B: ${UUID_B:-<none>}"
    [ "$UUID_A" != "$UUID_B" ] && echo "  -> LC_UUID differs (Apple ld64 content hash; investigate per O.5 — NOT auto-excluded, see allowlist.toml macOS note)"
fi
echo
echo "repro-build: NON-REPRODUCIBLE on this host — recorded honestly (O.5). This is the"
echo "  local mechanism's finding; the normative comparable-set verdict comes from"
echo "  tools/repro-verify.sh under the O.1 container against a published artifact."
# A local diff is a RECORDED finding, not a gate failure here — the release
# gate is O.5 verify-repro. Exit 0 so the check reports without blocking, but
# print the finding loud. (A release pipeline wires the fail policy per O.4
# reproVerify.status.)
exit 0
