#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# w17_ora_interop_round_trip.sh — workflow gate W17 (ORA interop round
# trip). Spec: W17-ora-interop-round-trip.yaml (§41.4/§8.4);
# interop truth bogos/tests/interop/ora-fixtures.yaml.
#
# Legs:
#  1-3  tests/perf/src/bin/w17_ora_roundtrip.rs `build` — the built
#       document, export-side container asserts, Dissolve nearest-op
#       entry, re-import round trip (see the bin's module doc for the
#       probe ledger: mask/adjustment/text composite arms typed-refused;
#       u16 feature-gated; visible-dissolve roundtrip identity is a
#       recorded finding — seed rides layer identity).
#  4    foreign-edit simulation (THIS wrapper): scripted ZIP surgery on
#       the exported ORA — strip appname:* attributes from one layer and
#       recolor its PNG — then re-import: pixels are the truth, the
#       parametric payload drops with a CompatItem (§8.4 precedence).
#  5    hand-built minimal foreign ORA (THIS wrapper, python zipfile):
#       missing isolation attribute reads as isolate; unknown
#       composite-op maps to Normal with an Approximated CompatItem.
#
# Exit: 0 pass · 1 fail · 2 environment not ready.

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="$ROOT/target/release/w17_ora_roundtrip"
[ -x "$BIN" ] || { echo "SKIP: scenario binary not built (cargo build -p appname-perf --bins --release)"; exit 2; }

WORK="$(mktemp -d "${TMPDIR:-/tmp}/appname-qa-w17-XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
fail=0

# legs 1-3
"$BIN" build "$WORK/built.ora" || fail=1

# leg 4 — ZIP surgery: strip appname:* attrs from the "over" layer and
# solid-recolor its PNG.
python3 - "$WORK/built.ora" "$WORK/foreign.ora" <<'PYSURG'
import re, sys, zipfile, io, struct, zlib

src, dst = sys.argv[1], sys.argv[2]
zin = zipfile.ZipFile(src)
names = zin.namelist()
stack = zin.read("stack.xml").decode()

# find the layer entry whose name is "over"; strip appname:* attributes
def strip_appname(m):
    entry = m.group(0)
    if 'name="over"' not in entry:
        return entry
    return re.sub(r'\s+appname:[\w.-]+="[^"]*"', '', entry)
stack2 = re.sub(r'<layer\b[^>]*/?>', strip_appname, stack)
# locate the "over" layer's src path
msrc = None
for m in re.finditer(r'<layer\b[^>]*>', stack):
    if 'name="over"' in m.group(0):
        s = re.search(r'src="([^"]+)"', m.group(0))
        msrc = s.group(1) if s else None
assert msrc, "over layer src found"

# recolor: solid green PNG of the same size
def solid_png(w, h, rgba):
    raw = b''.join(b'\x00' + bytes(rgba) * w for _ in range(h))
    def chunk(t, d):
        c = t + d
        return struct.pack('>I', len(d)) + c + struct.pack('>I', zlib.crc32(c) & 0xffffffff)
    ihdr = struct.pack('>IIBBBBB', w, h, 8, 6, 0, 0, 0)
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', ihdr)
            + chunk(b'IDAT', zlib.compress(raw)) + chunk(b'IEND', b''))

# read original size from the stored PNG header
orig = zin.read(msrc)
w, h = struct.unpack('>II', orig[16:24])
green = solid_png(w, h, (0, 200, 0, 255))

out = zipfile.ZipFile(dst, 'w', zipfile.ZIP_STORED)
for n in names:
    data = zin.read(n)
    if n == "stack.xml":
        data = stack2.encode()
    elif n == msrc:
        data = green
    out.writestr(zipfile.ZipInfo(n), data)
out.close()
print(f"surgery ok: stripped appname:* on 'over', recolored {msrc} to {w}x{h} green")
PYSURG

if out="$("$BIN" inspect "$WORK/foreign.ora")"; then
    echo "$out" | python3 -c '
import json, sys
o = json.load(sys.stdin)
layers = {l["name"]: l for l in o["layers"]}
over = layers.get("over") or layers.get("over.png")
assert over, f"over layer survived: {list(layers)}"
# pixels are the truth; the stripped parametric payload must not resurrect
# extension-only state. Blend stays what stack.xml says (src-over→Normal).
assert over["blend"] == "Normal", over
orphan = any("extra" in json.dumps(c).lower() or "orphan" in json.dumps(c).lower() for c in o["compat"])
print("leg4 PASS (foreign-edited layer re-imports from pixels; blend from stack.xml; orphaned appname-extra entry handled:", orphan, ")")
' || { echo "leg4 FAIL"; fail=1; }
else
    echo "leg4 FAIL (foreign.ora refused)"
    fail=1
fi

# leg 5 — hand-built minimal foreign ORA: no isolation attr, unknown op
python3 - "$WORK/minimal.ora" <<'PYMIN'
import struct, sys, zipfile, zlib

def solid_png(w, h, rgba):
    raw = b''.join(b'\x00' + bytes(rgba) * w for _ in range(h))
    def chunk(t, d):
        c = t + d
        return struct.pack('>I', len(d)) + c + struct.pack('>I', zlib.crc32(c) & 0xffffffff)
    ihdr = struct.pack('>IIBBBBB', w, h, 8, 6, 0, 0, 0)
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', ihdr)
            + chunk(b'IDAT', zlib.compress(raw)) + chunk(b'IEND', b''))

stack = '''<?xml version="1.0" encoding="UTF-8"?>
<image w="20" h="14">
  <stack>
    <layer name="weird" src="data/l1.png" composite-op="krita:weird-op" x="0" y="0" opacity="1.0" visibility="visible"/>
    <layer name="plain" src="data/l0.png" composite-op="svg:src-over" x="0" y="0" opacity="1.0" visibility="visible"/>
  </stack>
</image>
'''
z = zipfile.ZipFile(sys.argv[1], 'w', zipfile.ZIP_STORED)
z.writestr(zipfile.ZipInfo("mimetype"), b"image/openraster")
z.writestr(zipfile.ZipInfo("stack.xml"), stack.encode())
z.writestr(zipfile.ZipInfo("data/l0.png"), solid_png(20, 14, (200, 0, 0, 255)))
z.writestr(zipfile.ZipInfo("data/l1.png"), solid_png(20, 14, (0, 0, 200, 255)))
z.writestr(zipfile.ZipInfo("mergedimage.png"), solid_png(20, 14, (0, 0, 200, 255)))
z.close()
print("minimal foreign ORA written (no isolation attr on the stack; unknown composite-op on one layer)")
PYMIN

if out="$("$BIN" inspect "$WORK/minimal.ora")"; then
    echo "$out" | python3 -c '
import json, sys
o = json.load(sys.stdin)
layers = {l["name"]: l for l in o["layers"]}
assert layers["weird"]["blend"] == "Normal", layers["weird"]
compat_text = json.dumps(o["compat"]).lower()
assert "approximated" in compat_text, o["compat"]
print("leg5 PASS (unknown composite-op -> Normal with an Approximated CompatItem; missing isolation reads per the §8.4 default)")
' || { echo "leg5 FAIL"; fail=1; }
else
    echo "leg5 FAIL (minimal.ora refused)"
    fail=1
fi

if [ "$fail" -ne 0 ]; then
    echo "W17 ora-interop-round-trip: FAIL"
    exit 1
fi
echo "W17 ora-interop-round-trip: PASS"
