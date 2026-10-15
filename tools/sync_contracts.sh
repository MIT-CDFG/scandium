#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# Contract mirror sync — plan §0.4.
# Spec: the build consumes the frozen v5.5 spec via a VERBATIM byte-identical
# mirror under contracts/. This script is the ONLY writer of that mirror and
# of contracts/MANIFEST.sha256 (preamble rule 8: hashes are script-generated,
# never typed by hand).
#
# Modes:
#   (default)  re-copy every contract from the frozen bogos paths into
#              contracts/ and regenerate contracts/MANIFEST.sha256
#              (shasum -a 256, LC_ALL=C sorted mirror-relative paths,
#              standard shasum two-space format).
#   --check    recompute hashes of BOTH the bogos sources and the mirror,
#              compare each against the manifest, verify no file is missing
#              on either side and nothing extra sits in the mirror; exit
#              non-zero on any drift, printing the offending paths.
#
# Mirror map (source of truth for both modes):
#   bogos/spec/schemas/*                      -> contracts/schemas/
#   bogos/spec/commands/*                     -> contracts/commands/
#   bogos/spec/algorithms/algorithms.yaml     -> contracts/algorithms/
#   bogos/spec/algorithms/patent-register.yaml-> contracts/algorithms/
#   bogos/i18n/message_ids.yaml               -> contracts/message_ids.yaml

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BOGOS="${APPNAME_BOGOS_ROOT:-}"
if [ -z "$BOGOS" ]; then
    # OB-93: no machine path in a shipped file. Same candidate order as
    # gen_attestation.py: a checkout has it one level up, a worktree two.
    for _c in "$ROOT/../bogos" "$ROOT/../../bogos"; do
        [ -d "$_c" ] && { BOGOS="$(cd "$_c" && pwd)"; break; }
    done
    BOGOS="${BOGOS:-$ROOT/../bogos}"
fi
MANIFEST="$ROOT/contracts/MANIFEST.sha256"

if [ ! -d "$BOGOS/spec" ]; then
    echo "sync_contracts: bogos spec root not found at $BOGOS" >&2
    exit 2
fi

# Emit "mirror-relative-path<TAB>absolute-source-path" pairs, LC_ALL=C sorted
# by mirror path. Every mode derives everything from this one enumeration.
enumerate() {
    local f
    for f in "$BOGOS"/spec/schemas/*; do
        printf 'contracts/schemas/%s\t%s\n' "$(basename "$f")" "$f"
    done
    for f in "$BOGOS"/spec/commands/*; do
        printf 'contracts/commands/%s\t%s\n' "$(basename "$f")" "$f"
    done
    printf 'contracts/algorithms/algorithms.yaml\t%s\n' "$BOGOS/spec/algorithms/algorithms.yaml"
    printf 'contracts/algorithms/patent-register.yaml\t%s\n' "$BOGOS/spec/algorithms/patent-register.yaml"
    printf 'contracts/message_ids.yaml\t%s\n' "$BOGOS/i18n/message_ids.yaml"
}

sorted_pairs() { enumerate | LC_ALL=C sort; }

do_sync() {
    rm -rf "$ROOT/contracts/schemas" "$ROOT/contracts/commands" "$ROOT/contracts/algorithms"
    rm -f "$ROOT/contracts/message_ids.yaml" "$MANIFEST"
    mkdir -p "$ROOT/contracts/schemas" "$ROOT/contracts/commands" "$ROOT/contracts/algorithms"

    local dest src n=0
    while IFS=$'\t' read -r dest src; do
        cp "$src" "$ROOT/$dest"
        n=$((n + 1))
    done < <(sorted_pairs)

    # Manifest: hash the freshly copied mirror files, paths relative to the
    # repo root, LC_ALL=C sorted (stable format, standard shasum output).
    ( cd "$ROOT" && sorted_pairs | cut -f1 | xargs shasum -a 256 ) > "$MANIFEST"

    echo "sync_contracts: mirrored $n files; manifest $(wc -l < "$MANIFEST" | tr -d ' ') lines -> contracts/MANIFEST.sha256"
}

do_check() {
    local fail=0 dest src want got
    tmp="$(mktemp -d)"   # deliberately not local: the EXIT trap runs at script scope
    trap 'rm -rf "$tmp"' EXIT

    if [ ! -f "$MANIFEST" ]; then
        echo "CONTRACT-DRIFT missing manifest: contracts/MANIFEST.sha256 (run tools/sync_contracts.sh)" >&2
        exit 1
    fi

    sorted_pairs > "$tmp/pairs"

    # 1. The manifest's path set must equal the enumeration's mirror path set.
    awk '{ $1=""; sub(/^ +/,""); print }' "$MANIFEST" | LC_ALL=C sort > "$tmp/manifest_paths"
    cut -f1 "$tmp/pairs" > "$tmp/expected_paths"
    if ! cmp -s "$tmp/manifest_paths" "$tmp/expected_paths"; then
        echo "CONTRACT-DRIFT manifest path set != frozen-source enumeration:" >&2
        diff "$tmp/expected_paths" "$tmp/manifest_paths" >&2 || true
        fail=1
    fi

    # 2. Per file: mirror hash and bogos source hash must both match the manifest.
    while IFS=$'\t' read -r dest src; do
        want="$(awk -v p="$dest" '{ h=$1; $1=""; sub(/^ +/,""); if ($0 == p) print h }' "$MANIFEST")"
        if [ -z "$want" ]; then
            echo "CONTRACT-DRIFT not in manifest: $dest" >&2
            fail=1
            continue
        fi
        if [ ! -f "$ROOT/$dest" ]; then
            echo "CONTRACT-DRIFT mirror file missing: $dest" >&2
            fail=1
        else
            got="$(shasum -a 256 "$ROOT/$dest" | awk '{print $1}')"
            if [ "$got" != "$want" ]; then
                echo "CONTRACT-DRIFT mirror hash mismatch: $dest" >&2
                fail=1
            fi
        fi
        if [ ! -f "$src" ]; then
            echo "CONTRACT-DRIFT frozen source missing: $src" >&2
            fail=1
        else
            got="$(shasum -a 256 "$src" | awk '{print $1}')"
            if [ "$got" != "$want" ]; then
                echo "CONTRACT-DRIFT frozen source hash mismatch (bogos changed?): $src vs manifest entry $dest" >&2
                fail=1
            fi
        fi
    done < "$tmp/pairs"

    # 3. Nothing extra may sit inside the mirror (manifest itself excepted, and the
    #    DECLARED non-mirrored files below).
    #
    # OPERATOR WORD 11 (1), 2026-09-14. bogos is never written (SS10.1). A 1.0-only
    # register addition therefore cannot go into the generated mirror -- O-INT put
    # R-04's row there at 40ffb8dd8 and reddened this step. The row now lives in a
    # NON-MIRRORED file beside the mirror, which "enters bogos at the next ceremony".
    # It is listed EXPLICITLY rather than matched by pattern: a new non-mirrored file
    # must be a deliberate edit here, not something a filename can smuggle in.
    NON_MIRRORED="contracts/algorithms/patent-register-addenda-1.0.yaml"
    ( cd "$ROOT" && find contracts -type f ! -name MANIFEST.sha256 \
        | LC_ALL=C grep -vxF "$NON_MIRRORED" | LC_ALL=C sort ) > "$tmp/actual_files"
    if ! cmp -s "$tmp/actual_files" "$tmp/expected_paths"; then
        while IFS= read -r dest; do
            echo "CONTRACT-DRIFT unexpected file in mirror: $dest" >&2
            fail=1
        done < <(LC_ALL=C comm -23 "$tmp/actual_files" "$tmp/expected_paths")
    fi

    if [ "$fail" -ne 0 ]; then
        echo "sync_contracts --check: DRIFT DETECTED" >&2
        exit 1
    fi
    echo "sync_contracts --check: OK ($(wc -l < "$tmp/pairs" | tr -d ' ') files verified against sources and manifest)"
}

case "${1:-sync}" in
    sync) do_sync ;;
    --check) do_check ;;
    *)
        echo "usage: tools/sync_contracts.sh [--check]" >&2
        exit 2
        ;;
esac
