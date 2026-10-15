#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# REBUILD demo/DEMO.appname FROM COMMITTED INPUTS ONLY.
#
# WHY THIS EXISTS. demo/DEMO.appname is 31.9 MB and gitignored (.gitignore:22) --
# an operator decision to keep it out of the tree. That decision RESTS on the
# recipe being sufficient to reconstruct it, and until 2026-08-28 it was NOT:
# build-demo.jsonl + hide-fills.jsonl carry SEVEN envelopes and the package's own
# journal carries NINE. The two missing were the PRELUDE -- doc.new and the paste
# that makes Background and Pasted -- and nothing committed said how to make them.
#
# ★ THE GAP WAS INVISIBLE BY READING. O-UI 7 found it by counting layers created
# (3) against layers promised (5), then checked referenced-vs-created ids and got
# ZERO dangling -- so the recipe was internally CONSISTENT and externally
# INCOMPLETE, which is exactly the pair that looks finished.
#
# ★★ THE MISSING STEPS WERE RECOVERED FROM THE ARTIFACT, NOT FROM MEMORY: the
# package carries its own journal, and `appname history` names entry 0 doc.new and
# entry 1 edit.paste. A recipe reconstructed from recall is the failure this file
# exists to prevent.
#
# VERIFIED 2026-08-28: running this produced a document byte-comparable in
# structure to the shipped package -- 5 layers, same names, same layer ids, same
# visibility, 9 journal entries. The docId and the Background layer id DIFFER and
# are EXPECTED to: both are generated per-document rather than derived from the
# recipe, so this reproduces the FIXTURE, not the file's identity.
set -euo pipefail
CLI="${CLI:-./target/release/appname}"
OUT="${1:?usage: rebuild-demo.sh <output.appname>}"
HERE="$(cd "$(dirname "$0")" && pwd)"

"$CLI" new --width 1200 --height 800 --out "$OUT" >/dev/null
AID=$("$CLI" asset add-image --doc "$OUT" --file "$HERE/demo-source.png" --json \
      | python3 -c 'import sys,json;print(json.load(sys.stdin)["assetId"])')
"$CLI" apply --doc "$OUT" --command "{\"schema\":\"appname.command.v1\",\
\"id\":\"11111111-0000-4000-8000-000000000001\",\"kind\":\"edit.paste\",\
\"params\":{\"assetId\":\"$AID\",\
\"newLayerId\":\"e0c81653-cbcd-4430-8133-1f8a07a16b52\",\"at\":[0,0]}}" >/dev/null
"$CLI" script --doc "$OUT" --file "$HERE/build-demo.jsonl" >/dev/null
"$CLI" script --doc "$OUT" --file "$HERE/hide-fills.jsonl" >/dev/null
# ★★ SELF-VERIFY. Everything above this line is a recipe; without what follows it
# would be a recipe whose CORRECTNESS lives in the header comment, and a claim
# nothing re-derives is the failure this whole file exists to prevent. These
# asserts run every time anyone rebuilds, so a drifted engine or an edited
# jsonl FAILS HERE rather than producing a different five-layer document that
# looks right.
#
# WHAT IS PINNED AND WHY EACH: the layer COUNT and the four recipe-derived layer
# IDS (a wrong id means the wrong envelope ran); the layer NAMES (a rename in the
# engine would silently change what the demo script points at); the two Fill rows
# being HIDDEN (section 8 turns them on, so shipping them visible breaks the step);
# and the JOURNAL LENGTH (section 14 walks it backwards).
#
# NOT PINNED, DELIBERATELY: docId and the Background layer id. Both are generated
# per-document rather than derived from the recipe, so asserting them would fail
# on every correct rebuild -- the exact shape of a control that cannot pass.
INFO="$("$CLI" info --doc "$OUT")"
fail() { echo "REBUILD VERIFY FAILED: $1" >&2; echo "$INFO" >&2; exit 1; }
echo "$INFO" | grep -q '^5 layers$'                        || fail "expected 5 layers"
echo "$INFO" | grep -q 'entries: 9'                        || fail "expected 9 journal entries"
for want in \
  "Pasted            *e0c81653-cbcd-4430-8133-1f8a07a16b52" \
  "adjustment.invert *0e140000-0000-4000-8000-00000000a001" \
  "Fill              *0e140000-0000-4000-8000-00000000b001" \
  "Fill              *0e140000-0000-4000-8000-00000000b002" ; do
    echo "$INFO" | grep -qE "$want" || fail "missing row: $want"
done
[ "$(echo "$INFO" | grep -c 'visible=false')" -eq 2 ]      || fail "expected exactly 2 hidden layers (the two Fills)"
echo "$INFO"
echo "REBUILD VERIFY OK - 5 layers, 4 recipe ids, 2 hidden fills, 9 journal entries"
