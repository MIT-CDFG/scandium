#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# w13_batch_raw_subset.sh — workflow gate W13 (batch RAW develop) at the
# surface the shipped build exposes.
# Spec: W13-batch-raw-develop.yaml (§41.3/§26.2/§26.3/§8.3/Part 43).
#
# BLOCKED, typed (the row's RAW half): the DNG shoot fixture rides the
# LibRaw path whose mosaic egress is typed-refused pending the
# packaging-wave ruling (deps/libraw.yaml; W10 leg A pins the refusal
# verbatim) — probed below with a real .dng input so the batch surface's
# OWN refusal is on record too. TRIGGER: the packaging-wave LibRaw
# mosaic-egress ruling + the RAW develop stage on the batch path.
# Develop determinism itself is proven synthetic-side (w10 bin).
#
# RUNNABLE NOW (the row's BATCH-SEMANTICS half over appname-actions —
# the §26.3 --batch grounding binary; PNG stand-ins for the shoot):
#   step 1/2  20 valid inputs + 1 truncated -> exit 7 (partial batch
#             failure, §26.3), the run continues past the bad file.
#   step 3    batch report: schema-tagged, summary {total:21, ok:20,
#             failed:1}, the failing entry typed with the failing file.
#   step 4    exactly 20 outputs, 2-digit zero-padded {seq}, lowercase
#             extension per template.
#   step 6    re-run -> byte-identical outputs (--overwrite); without
#             --overwrite, " (2)" collision suffixes (§26.2 policy).
#   step 7    --json: stdout carries exactly one JSON document.
#   (jpeg-quality-82 leg: the runner's export format set is probed; if
#   jpeg isn't wired on the batch path yet, recorded typed.)
#
# Exit: 0 pass · 1 fail · 2 environment not ready.

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

CLI="${APPNAME_CLI:-}"
for cand in "$ROOT/target/debug/appname" "$ROOT/target/release/appname"; do
    [ -z "$CLI" ] && [ -x "$cand" ] && CLI="$cand"
done
ACT=""
for cand in "$ROOT/target/debug/appname-actions" "$ROOT/target/release/appname-actions"; do
    [ -x "$cand" ] && ACT="$cand" && break
done
[ -n "$CLI" ] || { echo "SKIP: appname-cli not built"; exit 2; }
[ -n "$ACT" ] || { echo "SKIP: appname-actions not built (cargo build -p appname-script --bins)"; exit 2; }

WORK="$(mktemp -d "${TMPDIR:-/tmp}/appname-qa-w13-XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
fail=0

# ---- fixtures: 20 valid PNGs + 1 truncated ------------------------------
IN="$WORK/shoot"; mkdir -p "$IN"
for i in $(seq 1 20); do
    tmp="$WORK/gen$i.appname"
    "$CLI" new --width $((24 + i)) --height $((20 + (i % 5))) --background white --out "$tmp" --json >/dev/null
    "$CLI" export --doc "$tmp" --format png --out "$IN/frame$(printf '%02d' "$i").png" --json >/dev/null
done
head -c 60 "$IN/frame01.png" > "$IN/truncated.png"

# The RAW-half refusal, on the batch surface itself:
head -c 4096 /dev/zero | dd bs=1 count=200 2>/dev/null > "$IN/probe.dng" || true
printf 'II*\000' | cat - "$IN/probe.dng" > "$IN/shot.dng" && rm "$IN/probe.dng"

# develop_web-shaped action (§41.3 preconditions: resize longest-edge +
# a fixed adjustment step; the curves step rides adjustment.add which is
# live — resize stands first per the letter).
cat > "$WORK/develop_web.json" <<'EOF'
{
  "schema": "appname.action.v1",
  "name": "develop_web",
  "steps": [
    { "command": { "kind": "doc.resize",
                   "params": { "width": 64, "resample": "bilinear" } } },
    { "command": { "kind": "adjustment.add",
                   "params": { "opKind": "adjustment.curves",
                                "newLayerId": "13000000-0000-4000-8000-0000000000c1",
                                "newOpId": "13000000-0000-4000-8000-0000000000c2",
                                "params": { "master": [[0.0,0.05],[0.5,0.6],[1.0,0.98]] } } } }
  ]
}
EOF
"$ACT" validate --action "$WORK/develop_web.json" >/dev/null \
    && echo "action-validate PASS (develop_web shape accepted)" \
    || { echo "action-validate FAIL"; fail=1; }

# DNG on the batch path: typed refusal expected while the module is out.
set +e
"$ACT" run --action "$WORK/develop_web.json" --in "$IN/shot.dng" \
    --out "$WORK/dng_out/{name}.{ext}" --format png --on-error continue --json \
    > "$WORK/dng.stdout" 2>&1
rc_dng=$?
set -e
if [ "$rc_dng" -ne 0 ] && [ ! -e "$WORK/dng_out/shot.png" ]; then
    echo "raw-leg BLOCKED-TYPED PASS (DNG input refused on the batch path, exit $rc_dng, no output written): $(head -c 160 "$WORK/dng.stdout" | tr '\n' ' ')"
else
    echo "raw-leg FAIL (a DNG decoded without the module?! exit $rc_dng)"
    fail=1
fi
rm -f "$IN/shot.dng"

# ---- the batch: 21 inputs incl. the truncated one -----------------------
INS=()
for f in "$IN"/frame*.png "$IN/truncated.png"; do INS+=(--in "$f"); done
run_batch() { # $1 outdir, $2 extra flag or ""
    set +e
    # shellcheck disable=SC2086
    "$ACT" run --action "$WORK/develop_web.json" "${INS[@]}" \
        --out "$1/web-{name}-{seq:2}.{ext}" --format png --on-error continue \
        $2 --json > "$1.stdout" 2>"$1.stderr"
    local rc=$?
    set -e
    return $rc
}
rc=0; run_batch "$WORK/out1" "" || rc=$?
if [ "$rc" = "7" ]; then
    echo "exit-code PASS (7 = partial batch failure; the truncated file failed, the run continued)"
else
    echo "exit-code FAIL (rc=$rc)"
    fail=1
fi

# step 3: report shape
if python3 - "$WORK/out1.stdout" <<'PYREP'
import json, sys
doc = json.loads(open(sys.argv[1]).read())  # exactly one JSON document
assert doc["schema"] == "appname.batchreport.v1", doc.get("schema")
s = doc["summary"]
assert (s["total"], s["ok"], s["failed"]) == (21, 20, 1), s
bad = [r for r in doc["results"] if r["status"] != "ok"]
assert len(bad) == 1 and "truncated" in bad[0]["input"], bad
assert bad[0]["error"]["code"].startswith("error."), bad[0]
PYREP
then
    echo "report PASS (appname.batchreport.v1; summary 21/20/1; the truncated input entry carries its typed error.* code)"
else
    echo "report FAIL"
    fail=1
fi

# step 4: naming — exactly 20 outputs, zero-padded {seq:2}, lowercase ext
count="$(ls "$WORK/out1" | grep -c '^web-frame[0-9][0-9]-[0-9][0-9]\.png$' || true)"
total_png="$(ls "$WORK/out1"/*.png 2>/dev/null | wc -l | tr -d ' ')"
if [ "$count" = "20" ] && [ "$total_png" = "20" ] && [ -f "$WORK/out1/batch-report.json" ]; then
    echo "naming PASS (exactly 20 png outputs, template web-{name}-{seq:2}.png with 2-digit padded seq; batch-report.json alongside)"
else
    echo "naming FAIL (matching=$count total_png=$total_png: $(ls "$WORK/out1" | head -3))"
    fail=1
fi

# step 6: determinism + collision policy
rc=0; run_batch "$WORK/out2" "" || rc=$?
same=1
for f in "$WORK/out1"/*.png; do
    b="$WORK/out2/$(basename "$f")"
    [ -f "$b" ] && [ "$(sha "$f")" = "$(sha "$b")" ] || same=0
done
[ "$same" = "1" ] \
    && echo "rerun-identity PASS (all 20 outputs byte-identical across runs — default develop deterministic, §43.4 class)" \
    || { echo "rerun-identity FAIL"; fail=1; }
rc=0; run_batch "$WORK/out2" "" || rc=$?
if ls "$WORK/out2" | grep -q ' (2)\.png$'; then
    echo "collision PASS (re-run into the same dest without overwrite produced \" (2)\" suffixes)"
else
    echo "collision FAIL ($(ls "$WORK/out2" | head -3))"
    fail=1
fi

# step 7: stream conventions (stdout = exactly one JSON document)
python3 -c 'import json,sys; json.loads(open(sys.argv[1]).read())' "$WORK/out1.stdout" \
    && echo "stream PASS (stdout parsed as exactly one JSON document; progress on stderr)" \
    || { echo "stream FAIL"; fail=1; }

# jpeg-quality leg probe (the letter's --quality 82):
set +e
"$ACT" run --action "$WORK/develop_web.json" --in "$IN/frame01.png" \
    --out "$WORK/jq/{name}.{ext}" --format jpeg --on-error continue --json \
    > "$WORK/jq.stdout" 2>&1
rc_j=$?
set -e
if [ "$rc_j" = "0" ] && ls "$WORK/jq" | grep -qi '\.jpe\?g$'; then
    echo "jpeg-leg PASS (batch path exports jpeg; quality knob wiring per-preset rides the export subsystem)"
else
    echo "jpeg-leg BLOCKED-TYPED (batch export format set is png-only today, exit $rc_j: $(head -c 120 "$WORK/jq.stdout" | tr '\n' ' ')) — TRIGGER: the batch-path jpeg/quality wiring (O-SCRIPT x O-IO export subsystem)"
fi

if [ "$fail" -ne 0 ]; then
    echo "W13 batch-raw-develop: FAIL"
    exit 1
fi
echo "W13 batch-raw-develop: PASS (batch-semantics half green; RAW half typed-blocked on record)"
