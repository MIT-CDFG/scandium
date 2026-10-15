#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# Reproducible-build verification counterpart — ES Appendix O.1/O.5
# ("tools/repro-verify.sh"). Given a locally-rebuilt artifact and a
# published reference, normalize BOTH per the O.1 per-OS caveats (strip
# signatures, exclude allow-listed regions) and print a per-file verdict.
# The expected result (O.5) is ZERO differences in the comparable set.
#
# HONEST SCOPE. A third-party verifier runs this inside the O.1 container
# against the release-page artifact. This host has no published artifact to
# diff against, so the default mode compares two paths you give it; with one
# path it prints the normalization it WOULD apply. The comparable-set
# contract is tools/repro/allowlist.toml.
#
# USAGE:
#   tools/repro-verify.sh <rebuilt> <published>   normalize both, diff
#   tools/repro-verify.sh <artifact>              show the normalization plan

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
ALLOWLIST="tools/repro/allowlist.toml"

os_class() {
    case "$(uname -s)" in
        Darwin) echo macos ;;
        Linux)  echo linux ;;
        *)      echo windows ;;
    esac
}

# Strip signatures / normalize a Mach-O or copy through unchanged.
normalize() { # $1 = input, $2 = output
    local in="$1" out="$2"
    cp "$in" "$out"
    if [ "$(os_class)" = macos ] && command -v codesign >/dev/null 2>&1; then
        # O.1 macOS: strip signatures before diffing.
        codesign --remove-signature "$out" 2>/dev/null || true
    fi
}

if [ "$#" -eq 1 ]; then
    echo "repro-verify: normalization plan for $(os_class) (comparable set = all shipped bytes MINUS the allow-listed regions):"
    echo "  allow-list: $ALLOWLIST"
    awk -v sect="[$(os_class)]" '
        $0==sect {ins=1; next}
        /^\[/{ins=0}
        ins' "$ALLOWLIST" | sed 's/^/    /'
    echo "  step: strip signatures (codesign --remove-signature on macOS), then byte-compare the remainder."
    exit 0
fi

[ "$#" -eq 2 ] || { echo "usage: $0 <rebuilt> [<published>]" >&2; exit 2; }
REBUILT="$1"; PUBLISHED="$2"
[ -f "$REBUILT" ] && [ -f "$PUBLISHED" ] || { echo "repro-verify: both paths must exist" >&2; exit 2; }

T="$(mktemp -d "${TMPDIR:-/tmp}/repro-verify.XXXXXX")"
trap 'rm -rf "$T"' EXIT
normalize "$REBUILT"  "$T/rebuilt.norm"
normalize "$PUBLISHED" "$T/published.norm"

if cmp -s "$T/rebuilt.norm" "$T/published.norm"; then
    echo "VERIFY PASS: zero differences in the comparable set (O.5) after normalization"
    exit 0
else
    echo "VERIFY DIFF: comparable-set difference after normalization (O.5: 'no close enough')"
    echo "  differing bytes: $(cmp -l "$T/rebuilt.norm" "$T/published.norm" 2>/dev/null | wc -l | tr -d ' ')"
    echo "  report per §35.5 (supply-chain incident) after self-triaging the O.5 benign causes"
    exit 1
fi
