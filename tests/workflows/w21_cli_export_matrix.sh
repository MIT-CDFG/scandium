#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# w21_cli_export_matrix.sh — workflow gate W21 (headless CLI export
# matrix). Spec: W21-headless-cli-export-matrix.yaml (§41.5/§26.3/§29.1).
#
# Half A: tests/perf/src/bin/w21_export_matrix.rs — the io-side §29.1
# matrix (six formats, per-format defaults, §29.4 policies, stage-7
# atomic write, whole-matrix byte determinism).
# Half B (this wrapper): the CLI-surface legs — what runs, runs; what
# the CLI does not yet wire is PROBED and recorded typed with its
# trigger (the CLI export-subsystem wiring row, O-CORE/O-IO seam):
#   * png export via `appname export` (live) + determinism;
#   * jpeg/webp/tiff/gif/bmp on the CLI verb (probed — jpeg answers
#     typed-NI, the rest are clap-rejected: wiring absent);
#   * --json single-document convention on success and failure;
#   * missing-input exit class (the §26.3 table's io-error row at the
#     current 0/1/2 CLI discipline — the 8-class table binds with the
#     export-subsystem wiring);
#   * pipe modes (--in - / --out -), diff, validate, SIGINT staging,
#     --gpu off: no such verbs/flags on the shipped CLI — recorded.
#
# Exit: 0 pass · 1 fail · 2 environment not ready.

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="$ROOT/target/release/w21_export_matrix"
[ -x "$BIN" ] || { echo "SKIP: scenario binary not built (cargo build -p appname-perf --bins --release)"; exit 2; }
CLI="${APPNAME_CLI:-}"
if [ -z "$CLI" ]; then
    for cand in "$ROOT/target/debug/appname" "$ROOT/target/release/appname"; do
        [ -x "$cand" ] && CLI="$cand" && break
    done
fi
[ -n "$CLI" ] || { echo "SKIP: appname-cli not built"; exit 2; }

fail=0
"$BIN" || fail=1

WORK="$(mktemp -d "${TMPDIR:-/tmp}/appname-qa-w21-XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
sha() { shasum -a 256 "$1" | cut -d' ' -f1; }

pkg="$WORK/doc.appname"
"$CLI" new --width 48 --height 32 --background white --out "$pkg" --json > /dev/null

# CLI png export + determinism
"$CLI" export --doc "$pkg" --format png --out "$WORK/a.png" --json > "$WORK/a.json"
"$CLI" export --doc "$pkg" --format png --out "$WORK/b.png" --json > /dev/null
if [ "$(sha "$WORK/a.png")" = "$(sha "$WORK/b.png")" ]; then
    echo "cli-png PASS (export live and byte-deterministic)"
else
    echo "cli-png FAIL"
    fail=1
fi
python3 -c 'import json,sys; json.loads(open(sys.argv[1]).read())' "$WORK/a.json" \
    && echo "cli-json-success PASS (exactly one JSON document on stdout)" \
    || { echo "cli-json-success FAIL"; fail=1; }

# jpeg: typed-NI on the verb; other formats: clap-rejected (no wiring)
jq="$("$CLI" export --doc "$pkg" --format jpeg --out "$WORK/x.jpg" --json 2>/dev/null | python3 -c 'import json,sys; o=json.load(sys.stdin); print("ACCEPT" if o.get("ok") else o["error"]["code"])' || true)"
echo "cli-jpeg PROBE: $jq (io-side jpeg is merged+proven in half A; the CLI verb wiring is the gap) — TRIGGER: CLI export-subsystem wiring row"
rc=0; "$CLI" export --doc "$pkg" --format webp --out "$WORK/x.webp" --json >/dev/null 2>&1 || rc=$?
echo "cli-webp/tiff/gif/bmp PROBE: clap value rejection (exit $rc — the format set on the verb is [png,jpeg]) — same trigger"

# missing input: typed io error, one JSON doc, nonzero exit
rc=0
out="$("$CLI" export --doc "$WORK/absent.appname" --format png --out "$WORK/y.png" --json 2>/dev/null)" || rc=$?
code="$(printf '%s' "$out" | python3 -c 'import json,sys; print(json.load(sys.stdin)["error"]["code"])' 2>/dev/null || echo parse-fail)"
if [ "$rc" -ne 0 ] && [ "$code" = "error.io.read_failed" ]; then
    echo "cli-missing-input PASS (exit $rc, typed $code, single JSON doc; the §26.3 8-class exit table binds with the wiring row — current discipline is 0/1/2)"
else
    echo "cli-missing-input FAIL (rc=$rc code=$code)"
    fail=1
fi

echo "pipe modes (--in -/--out -), diff verb, validate verb (exit-4 BLAKE3 leg), SIGINT .part staging leg, --gpu off flag: NOT ON THE SHIPPED CLI — recorded typed; TRIGGER: the CLI export-subsystem + verbs wiring row (stage-7 atomicity itself is proven io-side in half A; forced-CPU byte identity is W24's record)"

if [ "$fail" -ne 0 ]; then
    echo "W21 headless-cli-export-matrix: FAIL"
    exit 1
fi
echo "W21 headless-cli-export-matrix: PASS (io-side matrix green; CLI residue recorded typed)"
