#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
"""E-11's acceptance predicate for THIS repository.

WHY IT EXISTS. ROADMAP:230 accepts E-11 on `oint-version-census.py PASS`. That
instrument is the v5.5 -> v5.6 SPEC sweep: its --root defaults to another
repository which exists on this machine, so at freeze it would walk a spec tree
elsewhere and pass, marking done the row that bumps this project's version
(O-INT entry 1015; O-BUILD OB-65, OB-79). AMENDMENT 10 (e) already rules the
substance -- "E-11's census measures the APP version, not the spec revision" --
and this is that predicate, written so the operator has something to name.

WHAT IT CHECKS, and the exclusion is half the point:
  BUMP        Cargo.toml [workspace.package] version   -- the single source of truth
              crates/appname-py/pyproject.toml version -- independent; derives from nothing
  DERIVED     shell/CMakeLists.txt, scripts/build_dmg.sh must carry NO version literal
  MUST NOT    READER_VERSION and the "minReader" strings are a BACKWARD-COMPATIBILITY
  MOVE        FLOOR, not a product version. They read 0.1.0 today by coincidence of
              being early. "Bump every version-bearing file" executed literally rewrites
              51 goldens/fixtures/corpus documents and breaks what they exist to prove.

ROOT is derived from this file's location. It is never a path to somewhere else.
"""
import argparse, os, re, sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))

ap = argparse.ArgumentParser(description="E-11 acceptance: the APP version across this repository")
ap.add_argument("--expect", required=True, help="the version every app-version site must carry, e.g. 1.0.2")
a = ap.parse_args()

def read(rel):
    p = os.path.join(ROOT, rel)
    if not os.path.isfile(p):
        print(f"app-version-census REFUSED: missing {rel} under {ROOT}"); sys.exit(2)
    return open(p, encoding="utf-8").read()

fails, notes = [], []

# --- the two that must carry the expected version -------------------------
m = re.search(r"^\[workspace\.package\][^\[]*?^version\s*=\s*\"([^\"]+)\"", read("Cargo.toml"), re.M | re.S)
if not m:
    fails.append("Cargo.toml: no [workspace.package] version found")
else:
    notes.append(f"Cargo.toml [workspace.package] = {m.group(1)}")
    if m.group(1) != a.expect:
        fails.append(f"Cargo.toml [workspace.package] is {m.group(1)}, expected {a.expect}")

m = re.search(r"^version\s*=\s*\"([^\"]+)\"", read("crates/appname-py/pyproject.toml"), re.M)
if not m:
    fails.append("crates/appname-py/pyproject.toml: no version found")
else:
    notes.append(f"crates/appname-py/pyproject.toml   = {m.group(1)}")
    if m.group(1) != a.expect:
        fails.append(f"crates/appname-py/pyproject.toml is {m.group(1)}, expected {a.expect} "
                     "(it derives from nothing; the workspace bump does not reach it)")

# --- the derived sites must stay derived ----------------------------------
for rel, token in (("shell/CMakeLists.txt", "APPNAME_VERSION"),
                   ("scripts/build_dmg.sh", "VERSION")):
    body = read(rel)
    if token not in body:
        fails.append(f"{rel}: no longer reads the version through {token}")
    else:
        notes.append(f"{rel} derives via {token}")
    for lit in re.findall(r"VERSION[_A-Z]*\s*[:=]?\s*\"(\d+\.\d+(?:\.\d+)?)\"", body):
        fails.append(f"{rel}: carries the version literal \"{lit}\" — it must derive, not restate")

# --- the floor that must NOT move -----------------------------------------
pkg = read("crates/appname-core/src/io/package.rs")
m = re.search(r"READER_VERSION:\s*\(u32,\s*u32,\s*u32\)\s*=\s*\(([^)]*)\)", pkg)
rv = "".join((m.group(1) if m else "?").split())
notes.append(f"READER_VERSION = ({rv})   [floor — must not move]")
if rv != "0,1,0":
    fails.append(f"READER_VERSION moved to ({rv}); it is a compatibility floor, not the product version")
n_floor = sum(1 for _ in re.finditer(r'"minReader":\s*"0\.1\.0"', pkg))
if n_floor == 0:
    fails.append('package.rs: the "minReader": "0.1.0" floor string is gone')
notes.append(f'package.rs carries "minReader": "0.1.0" x{n_floor}   [floor]')

print(f"app-version-census: root {ROOT}")
for n in notes: print(f"  {n}")
print(f"app-version-census: expect {a.expect} · mismatches {len(fails)}")
for f in fails: print(f"  MISMATCH {f}")
sys.exit(1 if fails else 0)
