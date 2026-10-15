#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# w26_version_compat.sh — workflow gate W26 (CP-0.7 subset per plan §2.4:
# "a 0.5-era package opens/upgrades under 0.7"; fixture generated from
# the v0.5.0-beta tag binary — fixtures/compat/v050/README.md).
#
# Legs:
#   open           current CLI opens the committed v0.5-era package:
#                  structure exact (journal 5/5, 2 layers), no crash
#   render-stable  current export == the 0.5 binary's own recorded export
#                  (byte-exact cross-version composite stability)
#   upgrade        CP-0.7 kinds apply on the legacy doc (history.snapshot,
#                  smartobject.convert on the 0.5-era layer); undo works
#                  across the version boundary
#   preserve       upgrading does NOT rewrite the untouched background
#                  layer's tile bytes (raw-carry subset of §7.6)
#   minreader      a copy with requires.minReader rewritten to 99.0.0
#                  refuses CLEANLY: typed error, nonzero exit, no crash,
#                  package bytes untouched (§7.7 rule 1 subset)
#   replay         save-load-save byte stability of the upgraded package
#                  under the current binary
#
# Exit: 0 pass · 1 fail · 2 environment not ready.

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
FIX="$ROOT/fixtures/compat/v050"

CLI="${APPNAME_CLI:-}"
if [ -z "$CLI" ]; then
    for cand in "$ROOT/target/debug/appname" "$ROOT/target/release/appname"; do
        [ -x "$cand" ] && CLI="$cand" && break
    done
fi
[ -n "$CLI" ] || { echo "SKIP: appname-cli not built"; exit 2; }
[ -f "$FIX/legacy-basic.appname" ] || { echo "SKIP: fixture missing"; exit 2; }

WORK="$(mktemp -d "${TMPDIR:-/tmp}/appname-qa-w26-XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
sha() { shasum -a 256 "$1" | cut -d' ' -f1; }

fail=0
pkg="$WORK/doc.appname"
cp "$FIX/legacy-basic.appname" "$pkg"

# fixture integrity first (the legs below prove nothing on a drifted fixture)
( cd "$FIX" && shasum -a 256 -c SHA256SUMS >/dev/null 2>&1 ) \
    && echo "fixture-integrity PASS (SHA256SUMS)" \
    || { echo "fixture-integrity FAIL"; fail=1; }

s="$("$CLI" info --doc "$pkg" --json | python3 -c 'import json,sys; i=json.load(sys.stdin)["info"]; print(i["journal"]["length"], i["journal"]["cursor"], len(i["layers"]))')"
[ "$s" = "5 5 2" ] \
    && echo "open PASS (journal 5/5, 2 layers under the 0.7 binary)" \
    || { echo "open FAIL ($s)"; fail=1; }

"$CLI" export --doc "$pkg" --format png --out "$WORK/now.png" --json >/dev/null
if [ "$(sha "$WORK/now.png")" = "$(sha "$FIX/legacy-basic.v050-export.png")" ]; then
    echo "render-stable PASS (0.7 export byte-equals the recorded 0.5 export)"
else
    echo "render-stable FAIL (0.7 render of the 0.5 package diverged)"
    fail=1
fi

bg_id="$("$CLI" info --doc "$pkg" --json | python3 -c 'import json,sys;print(json.load(sys.stdin)["info"]["layers"][0]["id"])')"
bg_hash_before="$(unzip -p "$pkg" "layers/$bg_id/tiles/0/0_0.tile" | shasum -a 256 | cut -d' ' -f1)"

# upgrade: CP-0.7 kinds on the legacy document
"$CLI" apply --doc "$pkg" --json --command '{"schema":"appname.command.v1","id":"26000000-0000-4000-8000-000000000101","kind":"history.snapshot","params":{"name":"pre-upgrade"}}' > "$WORK/snap.json"
ok="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["ok"])' "$WORK/snap.json")"
"$CLI" apply --doc "$pkg" --json --command '{"schema":"appname.command.v1","id":"26000000-0000-4000-8000-000000000102","kind":"smartobject.convert","params":{"layerIds":["26000000-0000-4000-8000-0000000000b1"],"newLayerId":"26000000-0000-4000-8000-0000000000c1"}}' > "$WORK/conv.json"
ok2="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["ok"])' "$WORK/conv.json")"
if [ "$ok" = "True" ] && [ "$ok2" = "True" ]; then
    echo "upgrade PASS (history.snapshot + smartobject.convert applied on the 0.5-era doc)"
else
    echo "upgrade FAIL (snapshot ok=$ok convert ok=$ok2)"
    fail=1
fi
"$CLI" undo --doc "$pkg" --json >/dev/null \
    && echo "undo-across-boundary PASS (convert undone on the upgraded package)" \
    || { echo "undo-across-boundary FAIL"; fail=1; }
"$CLI" redo --doc "$pkg" --json >/dev/null || { echo "redo FAIL"; fail=1; }

bg_hash_after="$(unzip -p "$pkg" "layers/$bg_id/tiles/0/0_0.tile" | shasum -a 256 | cut -d' ' -f1)"
if [ "$bg_hash_after" = "$bg_hash_before" ]; then
    echo "preserve PASS (untouched background tile bytes carried verbatim through the upgrade)"
else
    echo "preserve FAIL (layers/$bg_id tile rewritten: $bg_hash_before -> $bg_hash_after)"
    fail=1
fi
# minReader-too-new: synthesized from the fixture (mimetype stays first + stored)
python3 - "$FIX/legacy-basic.appname" "$WORK/too-new.appname" <<'EOF'
import json, sys, zipfile
src, dst = sys.argv[1], sys.argv[2]
zin = zipfile.ZipFile(src)
names = zin.namelist()
assert names[0] == "mimetype"
with zipfile.ZipFile(dst, "w") as zout:
    for n in names:
        data = zin.read(n)
        if n == "manifest.json":
            m = json.loads(data)
            m["requires"]["minReader"] = "99.0.0"
            data = json.dumps(m).encode()
        comp = zipfile.ZIP_STORED if n == "mimetype" else zipfile.ZIP_DEFLATED
        zout.writestr(zipfile.ZipInfo(n), data, comp)
EOF
before="$(sha "$WORK/too-new.appname")"
set +e
out="$("$CLI" info --doc "$WORK/too-new.appname" --json 2>&1)"
rc=$?
set -e
code="$(printf '%s' "$out" | python3 -c 'import json,sys
try: print(json.load(sys.stdin)["error"]["code"])
except Exception: print("NO-JSON-ERROR")' 2>/dev/null || echo PARSE-FAIL)"
if [ "$rc" -ne 0 ] && printf '%s' "$code" | grep -q "^error\."; then
    echo "minreader PASS (refused: $code, exit $rc, no crash)"
else
    echo "minreader FAIL (exit $rc, code $code, out: $(printf '%s' "$out" | head -c 120))"
    fail=1
fi
[ "$(sha "$WORK/too-new.appname")" = "$before" ] \
    && echo "minreader-intact PASS (bytes untouched)" \
    || { echo "minreader-intact FAIL"; fail=1; }

# replay: save-load-save byte stability of the upgraded package
"$CLI" duplicate --doc "$pkg" --out "$WORK/dup1.appname" --json >/dev/null
"$CLI" duplicate --doc "$WORK/dup1.appname" --out "$WORK/dup2.appname" --json >/dev/null
if [ "$(sha "$WORK/dup1.appname")" = "$(sha "$WORK/dup2.appname")" ]; then
    echo "save-load-save PASS (upgraded package byte-stable)"
else
    echo "save-load-save FAIL"
    fail=1
fi

if [ "$fail" -ne 0 ]; then
    echo "W26 version-compat: FAIL"
    exit 1
fi
echo "W26 version-compat: PASS"
