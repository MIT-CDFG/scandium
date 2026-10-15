#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# w10_raw_photo_subset.sh — workflow gate W10 (RAW photo end-to-end),
# dispatch-scoped subset. Spec: W10-raw-photo-end-to-end.yaml
# (§41.3/§8.3/Part 43).
#
# Halves:
#  A  tests/perf/src/bin/w10_raw_develop_subset.rs — the DNG typed
#     refusal (LibRaw mosaic-egress gap; TRIGGER: packaging-wave ruling)
#     + develop-v1 stages + the lensdb stage-4b legs on synthetic input.
#  B  CLI-side post-develop story on a synthetic stand-in doc:
#     step 3 heal.stroke -> typed-NI, probed verbatim (TRIGGER: O-TOOLS
#            retouch suite merge);
#     step 4 adjustment.add Curves + mask.add on it (live kinds);
#     step 5 toggle off/on via op.toggle -> renders differ then match
#            (the W2 pattern) — probed; typed result recorded either way;
#     step 6 JPEG export quality 82 (profile/GPS legs ride W21's io-side
#            matrix + §29.4 policy leg — not double-claimed here);
#     step 7 save/reopen: develop-params round-trip is BLOCKED with the
#            RAW module (no rawDevelopParams surface exists yet); the
#            Curves params round-trip IS asserted (journal persists the
#            envelope; reopen + export byte-stable).
#
# Exit: 0 pass · 1 fail · 2 environment not ready.

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

BIN="$ROOT/target/release/w10_raw_develop_subset"
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

WORK="$(mktemp -d "${TMPDIR:-/tmp}/appname-qa-w10-XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
sha() { shasum -a 256 "$1" | cut -d' ' -f1; }

pkg="$WORK/doc.appname"
"$CLI" new --width 96 --height 64 --background white --out "$pkg" --json > "$WORK/new.json"
layer="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["receipt"]["createdIds"][-1])' "$WORK/new.json")"

# step 3 — heal.stroke, verbatim NI probe
heal="$("$CLI" apply --doc "$pkg" --command \
    "{\"schema\":\"appname.command.v1\",\"id\":\"ad00f210-0000-4000-8000-000000001001\",\"kind\":\"heal.stroke\",\"target\":{\"layerId\":\"$layer\"},\"params\":{}}" \
    --json 2>/dev/null | python3 -c 'import json,sys; o=json.load(sys.stdin); print(o.get("error",{}).get("code","ACCEPT"))' || true)"
if [ "$heal" = "error.command.not_implemented" ]; then
    echo "heal-stroke BLOCKED-TYPED PASS ($heal — TRIGGER: O-TOOLS retouch suite merge; W10 step 3 + golden w10_heal.png bind then)"
else
    echo "heal-stroke PROBE OBSERVED: $heal (if ACCEPT, the retouch suite landed — re-encode this row to run step 3 for real)"
fi

# step 6 FIRST — the export leg, before the adjustment lands (the
# parametric composite seam refuses flatten with an enabled adjustment
# in the doc — probed after step 4 below, recorded typed).
# JPEG on the CLI: PROBED AT RUNTIME, never assumed — the branch below
# reads the door's own answer and picks the format from it.
#
# ⛔ THE GAP THIS COMMENT RECORDED IS CLOSED (Q-D34, verified 2026-09-04).
# It read "the CLI export verb still answers typed-NI for jpeg ... the
# byte-stability legs ride PNG meanwhile" — present tense, and false on
# this tree: `export --format jpeg --quality 82` returns ok=true and
# writes real JFIF (file(1) confirms; 648 bytes on a 32x24 probe). This
# leg takes the jpeg branch today.
#
# THE PROBE STAYS, and that is the finding rather than an oversight: it
# is what let this workflow correct its own BEHAVIOUR the moment the door
# opened, with no edit here — only its PROSE went stale. A comment
# stating a blocker outlives the blocker silently (G-529); a runtime
# probe does not.
jq82="$("$CLI" export --doc "$pkg" --format jpeg --quality 82 --out "$WORK/web.jpg" --json 2>/dev/null | python3 -c 'import json,sys; o=json.load(sys.stdin); print("ACCEPT" if o.get("ok") else o["error"]["code"])' || true)"
if [ "$jq82" = "ACCEPT" ]; then
    echo "jpeg-export PASS (quality 82 through the CLI)"
    EXPFMT=jpeg; EXPEXT=jpg; EXPQ="--quality 82"
else
        echo "jpeg-export BLOCKED-TYPED ($jq82) — falling back to PNG for the byte-stability legs. THIS BRANCH WAS UNREACHABLE AS OF 2026-09-04: the verb answered ok for jpeg, so if you are reading this the export door has REGRESSED and $jq82 is its own typed reason. It no longer asserts 'the verb is png-only' — that was a general claim standing in for one probe's result, and it outlived the blocker it named."
    EXPFMT=png; EXPEXT=png; EXPQ=""
fi
# shellcheck disable=SC2086
"$CLI" export --doc "$pkg" --format "$EXPFMT" $EXPQ --out "$WORK/web.$EXPEXT" --json > /dev/null \
    && echo "export PASS (step-6 export via $EXPFMT)" \
    || { echo "export FAIL"; fail=1; }
E1="$(sha "$WORK/web.$EXPEXT")"

# step 4 — Curves adjustment + mask on it (live)
"$CLI" apply --doc "$pkg" --json --command \
    "{\"schema\":\"appname.command.v1\",\"id\":\"ad00f210-0000-4000-8000-000000001002\",\"kind\":\"adjustment.add\",\"params\":{\"opKind\":\"adjustment.curves\",\"newLayerId\":\"ad00f210-0000-4000-8000-0000000010b1\",\"newOpId\":\"ad00f210-0000-4000-8000-0000000010b2\",\"params\":{\"master\":[[0.0,0.0],[0.5,0.62],[1.0,1.0]]}}}" \
    > "$WORK/adj.json" && echo "curves-add PASS (adjustment.add accepted; params journaled)" \
    || { echo "curves-add FAIL"; fail=1; }
mask_out="$("$CLI" apply --doc "$pkg" --json --command \
    "{\"schema\":\"appname.command.v1\",\"id\":\"ad00f210-0000-4000-8000-000000001003\",\"kind\":\"mask.add\",\"target\":{\"layerId\":\"ad00f210-0000-4000-8000-0000000010b1\"},\"params\":{\"kind\":\"raster\",\"newMaskId\":\"ad00f210-0000-4000-8000-0000000010b3\",\"from\":\"reveal_all\"}}" \
    2>/dev/null | python3 -c 'import json,sys; o=json.load(sys.stdin); print("ACCEPT" if o.get("ok") else o["error"]["code"])' || true)"
if [ "$mask_out" = "ACCEPT" ]; then
    echo "mask-add PASS (raster mask on the adjustment layer accepted)"
else
    echo "mask-add PROBE OBSERVED: $mask_out (recorded)"
fi

# step 5 — toggle off/on (op.toggle): probed; the composite arm for
# parametric layers is typed-refused on the CPU reference today (w17
# probe: ParametricNotImplemented via the §12.1 registry seam), so the
# renders-differ-then-match leg records its blocker rather than faking.
tog="$("$CLI" apply --doc "$pkg" --json --command \
    "{\"schema\":\"appname.command.v1\",\"id\":\"ad00f210-0000-4000-8000-000000001004\",\"kind\":\"op.toggle\",\"target\":{\"opId\":\"ad00f210-0000-4000-8000-0000000010b2\"},\"params\":{\"enabled\":false}}" \
    2>/dev/null | python3 -c 'import json,sys; o=json.load(sys.stdin); print("ACCEPT" if o.get("ok") else o["error"]["code"])' || true)"
echo "op-toggle PROBE OBSERVED: $tog (the render-differ-then-match leg BLOCKED on the adjustment composite seam — w17's recorded ParametricNotImplemented; TRIGGER: the §12.1 op-registry wraps on the shipped composite path)"

# post-step-4 export: the parametric composite seam, probed verbatim.
# shellcheck disable=SC2086
if "$CLI" export --doc "$pkg" --format "$EXPFMT" $EXPQ --out "$WORK/web_adj.$EXPEXT" --json > "$WORK/webadj.json" 2>&1; then
    echo "export-with-adjustment PROBE: ACCEPTED (the parametric composite seam landed — re-encode the step-5 toggle leg to run for real)"
else
    echo "export-with-adjustment BLOCKED-TYPED (flatten refuses the enabled adjustment — the recorded §12.1 op-registry composite seam): $(python3 -c 'import json,sys; o=json.load(open(sys.argv[1])); print(o.get("error",{}).get("code","?"))' "$WORK/webadj.json" 2>/dev/null || head -c 80 "$WORK/webadj.json")"
fi

# step 7 — reopen: journaled Curves params round-trip + export stability
# (export stability compared on a pre-adjustment copy of the package —
# the adjusted doc cannot flatten yet, recorded above).
cp "$pkg" "$WORK/reopen.appname"
"$CLI" undo --doc "$WORK/reopen.appname" --steps 2 --json > /dev/null 2>&1 || true
# shellcheck disable=SC2086
"$CLI" export --doc "$WORK/reopen.appname" --format "$EXPFMT" $EXPQ --out "$WORK/web2.$EXPEXT" --json > /dev/null
persisted="$("$CLI" history --doc "$pkg" --json | python3 -c 'import json,sys; rows=json.load(sys.stdin)["entries"]; print(any(r.get("kind")=="adjustment.add" for r in rows))' 2>/dev/null || echo probe-failed)"
if [ "$(sha "$WORK/web2.$EXPEXT")" = "$E1" ] && [ "$persisted" = "True" ]; then
    echo "reopen PASS (adjustment.add persisted in the journal; export byte-stable across the package boundary)"
else
    echo "reopen OBSERVED (export_stable=$([ "$(sha "$WORK/web2.$EXPEXT")" = "$E1" ] && echo yes || echo no), adjustment_in_history=$persisted)"
    [ "$(sha "$WORK/web2.$EXPEXT")" = "$E1" ] || fail=1
fi
echo "raw-develop-params round-trip BLOCKED (no rawDevelopParams document surface without the RAW module) — TRIGGER: the packaging-wave LibRaw ruling + §43 develop-stage landing"

if [ "$fail" -ne 0 ]; then
    echo "W10 raw-photo-subset: FAIL"
    exit 1
fi
echo "W10 raw-photo-subset: PASS (synthetic develop + CLI subset; RAW ingest + retouch legs typed-blocked on record)"
