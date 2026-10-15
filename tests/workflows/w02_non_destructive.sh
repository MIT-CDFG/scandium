#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# w02_non_destructive.sh — workflow gate W02 (non-destructive edit,
# spec artifact W02-non-destructive-edit.yaml §34.6; CP-0.7 exit-gate
# row per plan §2.4).
#
# FINALIZED (reopened-CP-0.7 §12.1 slice, O-CORE; flagged for O-QA
# ratification): the export door now EVALUATES parametric content — the
# core-side §12.1 chain evaluator consumes the assembly-registered
# kernels (adjustments + filters ride the same construction-only wraps
# the KindImpls do), so the former typed-refusal TRIPWIRE legs are
# re-encoded as the real §34.6 composite-DELTA legs, exactly as the
# tripwire's finalize instructions demanded. Gate flips to exit 0.
#
# Changed expectations vs the pending-armed revision (BY DESIGN, each
# listed in reports/O-CORE-log.md for ratification):
#   - enabled-adjustment export: typed refusal → SERVES, E1 != E0
#   - op.toggle off: refusal-unchanged pin → export == E0 BYTE-IDENTICAL
#     (§12.4 disabled == never-applied now holds AT THE EXPORT DOOR too;
#     the recorded layer-level-routing divergence is RESOLVED)
#   - op.toggle back on: re-render match, export == E1
#   - re-edit (op.set_params V2): E2 != E1
#   - reopen: params JSON-equal AND re-render == E2 (byte)
#   - content layer fills at opacity 0.5 (mid-gray composite) so the
#     midtone curve points produce visible deltas — pure black/white
#     content would alias the curve to identity and void the premise
#
# The render-lane tripwire (appname_render::composite_document, QA's
# conformance suite) is UNTOUCHED and stays armed for O-RENDER's slice.
#
# Exit: 0 pass · 1 fail · 2 pending (assembly lacks adjustment.curves).

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

CLI="${APPNAME_CLI:-}"
if [ -z "$CLI" ]; then
    for cand in "$ROOT/target/debug/appname" "$ROOT/target/release/appname"; do
        [ -x "$cand" ] && CLI="$cand" && break
    done
fi
[ -n "$CLI" ] || { echo "SKIP: appname-cli not built"; exit 2; }

WORK="$(mktemp -d "${TMPDIR:-/tmp}/appname-qa-w02-XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
exp() { "$CLI" export --doc "$pkg" --format png --out "$1" --json >/dev/null; sha "$1"; }

fail=0
pkg="$WORK/doc.appname"
LID="02000000-0000-4000-8000-00000000bbb1"
ADJL="02000000-0000-4000-8000-00000000bbb2"
OPID="02000000-0000-4000-8000-00000000bbb3"
# Registry-owned schema spelling (top-level point arrays) — the
# contract example's `channels` nesting is ignored-unknown live
# (identity), which would void the curve premise (fold-in note).
CURVE_V1='{"master":[[0,0],[0.5,0.25],[1,1]]}'
CURVE_V2='{"master":[[0,0],[0.5,0.75],[1,1]]}'

"$CLI" new --width 64 --height 64 --background white --out "$pkg" --json >/dev/null
"$CLI" apply --doc "$pkg" --json --command "{\"schema\":\"appname.command.v1\",\"id\":\"02000000-0000-4000-8000-00000000aaa1\",\"kind\":\"layer.add\",\"params\":{\"kind\":\"raster\",\"newLayerId\":\"$LID\",\"name\":\"content\"}}" >/dev/null
# Mid-gray composite (fill at opacity 0.5) — see header note.
"$CLI" apply --doc "$pkg" --json --command "{\"schema\":\"appname.command.v1\",\"id\":\"02000000-0000-4000-8000-00000000aaa2\",\"kind\":\"fill\",\"target\":{\"layerId\":\"$LID\"},\"params\":{\"content\":\"foreground\",\"opacity\":0.5}}" >/dev/null
E0="$(exp "$WORK/e0.png")"

# ---- stage guard: the §12.1 registry must carry adjustment.curves --------
add_out="$("$CLI" apply --doc "$pkg" --json --command "{\"schema\":\"appname.command.v1\",\"id\":\"02000000-0000-4000-8000-00000000aaa3\",\"kind\":\"adjustment.add\",\"target\":{\"layerId\":\"$LID\"},\"params\":{\"opKind\":\"adjustment.curves\",\"newLayerId\":\"$ADJL\",\"newOpId\":\"$OPID\",\"params\":$CURVE_V1}}" || true)"
code="$(printf '%s' "$add_out" | python3 -c 'import json,sys
o=json.load(sys.stdin)
print(o.get("error",{}).get("code","") if not o.get("ok") else "OK")')"
if [ "$code" = "error.schema.unknown_op_kind" ]; then
    echo "W02 PENDING: adjustment.curves is not in this assembly's §12.1 op registry"
    exit 2
fi
[ "$code" = "OK" ] || { echo "adjustment.add FAIL ($code)"; echo "W02 non-destructive: FAIL"; exit 1; }
echo "adjustment-add PASS (adjustment.curves live through the §12.1 registry)"

# D-emission (chain-edit dirty flip, §10.3): the add receipt carries a
# non-empty dirty list (below-stack extent).
dirty="$(printf '%s' "$add_out" | python3 -c 'import json,sys
o=json.load(sys.stdin)
r=o.get("receipt",o)
print(len(r.get("dirty",[]) or []))')"
[ "$dirty" -gt 0 ] \
    && echo "adjustment-add-dirty PASS ($dirty rect(s) — chain-edit D emission live)" \
    || { echo "adjustment-add-dirty FAIL (dirty empty)"; fail=1; }

# ---- §34.6 composite-delta legs (re-encoded live) ------------------------
E1="$(exp "$WORK/e1.png")"
[ "$E1" != "$E0" ] \
    && echo "enabled-adjustment-delta PASS (E1 != E0 — the curve renders)" \
    || { echo "enabled-adjustment-delta FAIL (E1 == E0)"; fail=1; }
[ "$(exp "$WORK/e1b.png")" = "$E1" ] \
    && echo "re-render-determinism PASS (two exports byte-identical)" \
    || { echo "re-render-determinism FAIL"; fail=1; }

"$CLI" apply --doc "$pkg" --json --command "{\"schema\":\"appname.command.v1\",\"id\":\"02000000-0000-4000-8000-00000000aaa4\",\"kind\":\"op.toggle\",\"params\":{\"opId\":\"$OPID\",\"enabled\":false}}" >/dev/null
[ "$(exp "$WORK/e0b.png")" = "$E0" ] \
    && echo "toggle-off-identity PASS (== E0 byte — §12.4 at the export door)" \
    || { echo "toggle-off-identity FAIL"; fail=1; }
"$CLI" apply --doc "$pkg" --json --command "{\"schema\":\"appname.command.v1\",\"id\":\"02000000-0000-4000-8000-00000000aaa5\",\"kind\":\"op.toggle\",\"params\":{\"opId\":\"$OPID\",\"enabled\":true}}" >/dev/null
[ "$(exp "$WORK/e1c.png")" = "$E1" ] \
    && echo "toggle-on-render-match PASS (== E1 byte)" \
    || { echo "toggle-on-render-match FAIL"; fail=1; }

"$CLI" apply --doc "$pkg" --json --command "{\"schema\":\"appname.command.v1\",\"id\":\"02000000-0000-4000-8000-00000000aaa7\",\"kind\":\"layer.set_props\",\"target\":{\"layerId\":\"$ADJL\"},\"params\":{\"visible\":false}}" >/dev/null
[ "$(exp "$WORK/ehidden.png")" = "$E0" ] \
    && echo "hidden-adjustment-exports-identity PASS (== E0 byte-identical)" \
    || { echo "hidden-adjustment-exports-identity FAIL"; fail=1; }
"$CLI" apply --doc "$pkg" --json --command "{\"schema\":\"appname.command.v1\",\"id\":\"02000000-0000-4000-8000-00000000aaa8\",\"kind\":\"layer.set_props\",\"target\":{\"layerId\":\"$ADJL\"},\"params\":{\"visible\":true}}" >/dev/null

"$CLI" apply --doc "$pkg" --json --command "{\"schema\":\"appname.command.v1\",\"id\":\"02000000-0000-4000-8000-00000000aaa6\",\"kind\":\"op.set_params\",\"params\":{\"opId\":\"$OPID\",\"params\":$CURVE_V2}}" >/dev/null
E2="$(exp "$WORK/e2.png")"
[ "$E2" != "$E1" ] \
    && echo "re-edit-delta PASS (op.set_params V2 re-renders, E2 != E1)" \
    || { echo "re-edit-delta FAIL (E2 == E1)"; fail=1; }

# save/reopen: params round-trip JSON-equal + re-render matches byte-true
cp "$pkg" "$WORK/copy.appname"
roundtrip="$(python3 - "$WORK/copy.appname" "$CURVE_V2" <<'EOF'
import json, sys, zipfile
# Ops persist as ops/<opId>.json (appname.op.v1: kind + params) —
# document.json carries the id references only (finalize observation).
z = zipfile.ZipFile(sys.argv[1])
want = json.loads(sys.argv[2])
found = [json.loads(z.read(n)) for n in z.namelist()
         if n.startswith("ops/") and n.endswith(".json")]
curves = [o.get("params") for o in found
          if o.get("kind") == "adjustment.curves"]
print("EQUAL" if any(p == want for p in curves) else f"DIFF {curves}")
EOF
)"
[ "$roundtrip" = "EQUAL" ] \
    && echo "params-roundtrip PASS (curve params JSON-equal after save/reopen)" \
    || { echo "params-roundtrip FAIL ($roundtrip)"; fail=1; }
pkg="$WORK/copy.appname"
[ "$(exp "$WORK/e2reopen.png")" = "$E2" ] \
    && echo "reopen-rerender PASS (byte-copy reopen re-renders == E2)" \
    || { echo "reopen-rerender FAIL"; fail=1; }

if [ "$fail" -ne 0 ]; then
    echo "W02 non-destructive: FAIL"
    exit 1
fi
echo "W02 non-destructive: PASS (every §34.6 leg live — the §12.1"
echo "  parametric re-render serves at the export door; changed legs"
echo "  listed in reports/O-CORE-log.md for O-QA ratification)"
exit 0
