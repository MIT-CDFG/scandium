#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# w16_smart_object_nested_edit.sh — workflow gate W16 (CP-0.7 runnable
# subset per plan §2.4; the spec artifact W16-smart-object-nested-edit.yaml
# nests THREE deep with smart filters — single-level smart filters now
# serve through the §12.1 row [chains evaluated at the export door;
# covered by the composite-level opnode goldens + the conformance seam
# suite], THREE-DEEP NESTING remains the named CP-1.0 scope).
#
# LIVE legs (CLI end-to-end, deterministic ids):
#   dedup probe        duplicate of a smart instance adds NO second asset
#                      blob to the package (§11.5 one-AssetId probe)
#   edit-contents      smartobject.edit_contents{assetBefore, assetAfter}
#                      swaps EVERY instance; the receipt dirties both and
#                      both instances' PERSISTED tile surfaces change bytes
#   undo/redo          both instance tile surfaces revert/re-swap
#                      byte-exact across the package boundary
#   typed refusals     stale assetBefore → error.command.bad_target;
#                      replace with unresolvable asset → bad_target
#   reopen             byte-copy reopen: journal + layer structure + tile
#                      surfaces stable
#
# COMPOSITE LEGS LIVE (QA-F8 CLOSED — reopened-CP-0.7 goldens round):
# these legs were HONEST RED while smart-object content was invisible at
# the composite doors (silently absent from exports), then flipped when
# the §12.1 row landed core-side smart-content serving. This revision
# asserts REAL COMPOSITED SMART CONTENT, not just export-hash deltas:
#   convert-preserves-look   export byte-stable across convert
#   edit-changes-look        export changes across the content swap
#   composited-content-real  the post-edit export byte-equals a
#                            reference doc whose visible content IS the
#                            swapped blue fixture (white bg + journaled
#                            import of fixtures/w16/blue-64.png) — the
#                            composite carries the smart content itself,
#                            through both stacked instances
# The serve-or-refuse discipline held end-to-end: no door silently
# omitted at any revision of this gate (refusals were typed; reds were
# honest). Closure evidence: reports/O-QA-log.md goldens-round entry.
#
# Exit: 0 pass · 1 fail · 2 not ready.

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

CLI="${APPNAME_CLI:-}"
if [ -z "$CLI" ]; then
    for cand in "$ROOT/target/debug/appname" "$ROOT/target/release/appname"; do
        [ -x "$cand" ] && CLI="$cand" && break
    done
fi
if [ -z "$CLI" ]; then
    echo "SKIP: appname-cli not built (cargo build -p appname-cli)"
    exit 2
fi

WORK="$(mktemp -d "${TMPDIR:-/tmp}/appname-qa-w16-XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
tile_hash() { # tile_hash <pkg> <layer-uuid>
    unzip -p "$1" "layers/$2/tiles/0/0_0.tile" | shasum -a 256 | cut -d' ' -f1
}
asset_count() { unzip -l "$1" | grep -c "assets/" || true; }

fail=0
pkg="$WORK/doc.appname"
LID="c0000000-0000-4000-8000-00000000bbb1"
SOL="c0000000-0000-4000-8000-00000000bbb2"
DUP="c0000000-0000-4000-8000-00000000bbb3"
# content-derived ids (deterministic: fixed content + fixed fixture bytes;
# asserted below, never assumed)
CONVERT_ASSET="ef89c10d-7f6b-5459-aeaa-ed4bc08828f4"
BLUE_ASSET="44e705e2-8775-514e-acd2-4969998add87"

"$CLI" new --width 64 --height 64 --background white --out "$pkg" --json >/dev/null
"$CLI" apply --doc "$pkg" --json --command "{\"schema\":\"appname.command.v1\",\"id\":\"c0000000-0000-4000-8000-00000000aaa1\",\"kind\":\"layer.add\",\"params\":{\"kind\":\"raster\",\"newLayerId\":\"$LID\",\"name\":\"art\"}}" >/dev/null
"$CLI" apply --doc "$pkg" --json --command "{\"schema\":\"appname.command.v1\",\"id\":\"c0000000-0000-4000-8000-00000000aaa2\",\"kind\":\"fill\",\"target\":{\"layerId\":\"$LID\"},\"params\":{\"content\":\"foreground\"}}" >/dev/null
E_PRE="$("$CLI" export --doc "$pkg" --format png --out "$WORK/pre.png" --json >/dev/null; sha "$WORK/pre.png")"

"$CLI" apply --doc "$pkg" --json --command "{\"schema\":\"appname.command.v1\",\"id\":\"c0000000-0000-4000-8000-00000000aaa3\",\"kind\":\"smartobject.convert\",\"params\":{\"layerIds\":[\"$LID\"],\"newLayerId\":\"$SOL\"}}" >/dev/null
E_CONV="$("$CLI" export --doc "$pkg" --format png --out "$WORK/conv.png" --json >/dev/null; sha "$WORK/conv.png")"
if [ "$(unzip -l "$pkg" | grep -c "assets/$CONVERT_ASSET.image")" = "1" ]; then
    echo "convert PASS (embedded source asset $CONVERT_ASSET aboard)"
else
    echo "convert FAIL (expected content-derived asset $CONVERT_ASSET; have: $(unzip -l "$pkg" | grep assets/ || true))"
    fail=1
fi

"$CLI" apply --doc "$pkg" --json --command "{\"schema\":\"appname.command.v1\",\"id\":\"c0000000-0000-4000-8000-00000000aaa4\",\"kind\":\"layer.duplicate\",\"target\":{\"layerId\":\"$SOL\"},\"params\":{\"idMap\":{\"$SOL\":\"$DUP\"}}}" >/dev/null
n="$(asset_count "$pkg")"
if [ "$n" = "1" ]; then
    echo "dedup-probe PASS (duplicate instance, still 1 asset blob)"
else
    echo "dedup-probe FAIL ($n asset blobs after duplicate)"
    fail=1
fi

T0_A="$(tile_hash "$pkg" "$SOL")"
T0_B="$(tile_hash "$pkg" "$DUP")"
[ "$T0_A" = "$T0_B" ] || { echo "instance-tiles-agree FAIL"; fail=1; }

got="$("$CLI" asset add-image --doc "$pkg" --file "$ROOT/fixtures/w16/blue-64.png" --json | python3 -c 'import json,sys;print(json.load(sys.stdin)["assetId"])')"
if [ "$got" = "$BLUE_ASSET" ]; then
    echo "ingest PASS (content-derived id stable)"
else
    echo "ingest FAIL (assetId $got != recorded $BLUE_ASSET)"
    fail=1
fi

"$CLI" apply --doc "$pkg" --json --command "{\"schema\":\"appname.command.v1\",\"id\":\"c0000000-0000-4000-8000-00000000aaa5\",\"kind\":\"smartobject.edit_contents\",\"target\":{\"layerId\":\"$SOL\"},\"params\":{\"assetBefore\":\"$CONVERT_ASSET\",\"assetAfter\":\"$BLUE_ASSET\"}}" > "$WORK/edit.json"
dirty_layers="$(python3 -c 'import json,sys; r=json.load(open(sys.argv[1]))["receipt"]; print(len(r["dirty"]))' "$WORK/edit.json")"
T1_A="$(tile_hash "$pkg" "$SOL")"
T1_B="$(tile_hash "$pkg" "$DUP")"
if [ "$dirty_layers" = "2" ] && [ "$T1_A" != "$T0_A" ] && [ "$T1_B" != "$T0_B" ]; then
    echo "edit-contents-propagates PASS (receipt dirties both; both persisted tile surfaces changed)"
else
    echo "edit-contents-propagates FAIL (dirty=$dirty_layers, tiles $T0_A->$T1_A / $T0_B->$T1_B)"
    fail=1
fi

"$CLI" undo --doc "$pkg" --json >/dev/null
if [ "$(tile_hash "$pkg" "$SOL")" = "$T0_A" ] && [ "$(tile_hash "$pkg" "$DUP")" = "$T0_B" ]; then
    echo "undo-restores-both PASS (tile surfaces byte-exact)"
else
    echo "undo-restores-both FAIL"
    fail=1
fi
"$CLI" redo --doc "$pkg" --json >/dev/null
if [ "$(tile_hash "$pkg" "$SOL")" = "$T1_A" ] && [ "$(tile_hash "$pkg" "$DUP")" = "$T1_B" ]; then
    echo "redo-reswaps PASS"
else
    echo "redo-reswaps FAIL"
    fail=1
fi

# typed refusals (error-record legs)
code="$("$CLI" apply --doc "$pkg" --json --command "{\"schema\":\"appname.command.v1\",\"id\":\"c0000000-0000-4000-8000-00000000aaa6\",\"kind\":\"smartobject.edit_contents\",\"target\":{\"layerId\":\"$SOL\"},\"params\":{\"assetBefore\":\"$CONVERT_ASSET\",\"assetAfter\":\"$BLUE_ASSET\"}}" | python3 -c 'import json,sys;print(json.load(sys.stdin)["error"]["code"])' || true)"
[ "$code" = "error.command.bad_target" ] \
    && echo "stale-session-refusal PASS (bad_target)" \
    || { echo "stale-session-refusal FAIL ($code)"; fail=1; }
code="$("$CLI" apply --doc "$pkg" --json --command "{\"schema\":\"appname.command.v1\",\"id\":\"c0000000-0000-4000-8000-00000000aaa7\",\"kind\":\"smartobject.replace\",\"target\":{\"layerId\":\"$DUP\"},\"params\":{\"source\":{\"assetId\":\"c0000000-0000-4000-8000-00000000feed\"}}}" | python3 -c 'import json,sys;print(json.load(sys.stdin)["error"]["code"])' || true)"
[ "$code" = "error.command.bad_target" ] \
    && echo "replace-unresolvable-refusal PASS (bad_target)" \
    || { echo "replace-unresolvable-refusal FAIL ($code)"; fail=1; }

# reopen (byte-copy): structure + tile surfaces stable
cp "$pkg" "$WORK/copy.appname"
s="$("$CLI" info --doc "$WORK/copy.appname" --json | python3 -c 'import json,sys; i=json.load(sys.stdin)["info"]; print(i["journal"]["length"], i["journal"]["cursor"], len(i["layers"]))')"
if [ "$s" = "6 6 3" ] && [ "$(tile_hash "$WORK/copy.appname" "$SOL")" = "$T1_A" ]; then
    echo "reopen PASS (journal 6/6, 3 layers, tiles stable)"
else
    echo "reopen FAIL ($s)"
    fail=1
fi

# ---- composite legs: LIVE (QA-F8 closed; content-asserted) ---------------
E_POST="$("$CLI" export --doc "$pkg" --format png --out "$WORK/post.png" --json >/dev/null; sha "$WORK/post.png")"
if [ "$E_CONV" = "$E_PRE" ]; then
    echo "convert-preserves-look PASS (export byte-stable across convert)"
else
    echo "convert-preserves-look FAIL (smart-object content diverged at the export door)"
    echo "  repro: pre-convert export $E_PRE != post-convert export $E_CONV"
    fail=1
fi
if [ "$E_POST" != "$E_CONV" ]; then
    echo "edit-changes-look PASS (export changed across the content swap)"
else
    echo "edit-changes-look FAIL (export byte-identical across a smart-object content swap)"
    echo "  repro: post-convert export == post-edit export == $E_CONV"
    fail=1
fi
# Content assertion: the composited pixels ARE the swapped blue fixture.
# Reference doc: white bg + journaled import (edit.paste) of the same
# fixture — its export is the fixture's visible content through the
# ordinary raster path; the smart-object doc must composite to the SAME
# BYTES (both stacked instances carry the opaque blue source).
ref="$WORK/ref.appname"
"$CLI" new --width 64 --height 64 --background white --out "$ref" --json >/dev/null
"$CLI" import --doc "$ref" --image "$ROOT/fixtures/w16/blue-64.png" --json >/dev/null
E_REF="$("$CLI" export --doc "$ref" --format png --out "$WORK/ref.png" --json >/dev/null; sha "$WORK/ref.png")"
if [ "$E_POST" = "$E_REF" ]; then
    echo "composited-content-real PASS (post-edit export byte-equals the raster-path blue reference)"
else
    echo "composited-content-real FAIL (post-edit export $E_POST != blue reference $E_REF —"
    echo "  the composite does not carry the swapped smart content)"
    fail=1
fi

if [ "$fail" -ne 0 ]; then
    echo "W16 smart-object-nested-edit: FAIL"
    exit 1
fi
echo "W16 smart-object-nested-edit: PASS"
