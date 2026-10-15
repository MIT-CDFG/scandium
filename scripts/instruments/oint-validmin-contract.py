#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# oint-validmin-contract.py — OWNER: O-INT (integration desk).  v1.  READ-ONLY.
#
# WHY IT EXISTS (entry 1073, operator WORD 14 (4): "run every kind's validMinimal
# through the validator in one script and report the true count"). There IS no
# validator in this tree — registry.rs:23 and schema_shape.rs:39 both record the
# jsonschema crate as unelected, and python3 has none — so this checks the subset
# a reader can decide and NAMES the rest rather than implying a full validation.
#
# Exit: 0 every example satisfies its own contract-declared schema · 1 one or more
# do not · 2 VOID (its own planted violation went undetected, or nothing was read).
"""validMinimal, contract half: does each kind's own published example satisfy its own
published paramsSchema, at the levels decidable without a JSON-Schema engine?

CHECKED: unknown keys under additionalProperties:false · missing required keys · the type of
a declared property when the schema gives one simple "type" · the same three one level down,
for a declared object property that itself declares properties.
NOT CHECKED: oneOf/anyOf/allOf · $ref · format · enum · numeric bounds · array item schemas.
A contract that delegates a nested object to another surface (adjustment.add's params.params
says "re-validate at stage 3 against that opKind's REGISTERED schema") is NOT judged here:
that surface is the engine's, and oint-validmin-staged.py is what runs it.
"""
import sys, glob, os, yaml

TYPES = {"object": dict, "array": list, "string": str, "boolean": bool,
         "integer": int, "number": (int, float), "null": type(None)}

def check(schema, inst, path, out):
    if not isinstance(schema, dict) or not isinstance(inst, dict):
        return
    props = schema.get("properties") or {}
    for r in schema.get("required") or []:
        if r not in inst:
            out.append((path + r, "MISSING-REQUIRED"))
    if schema.get("additionalProperties") is False:
        for k in inst:
            if k not in props:
                out.append((path + k, "UNKNOWN-KEY"))
    for k, v in inst.items():
        sub = props.get(k)
        if not isinstance(sub, dict):
            continue
        t = sub.get("type")
        if isinstance(t, str) and t in TYPES and not isinstance(v, TYPES[t]):
            if not (t == "integer" and isinstance(v, bool) is False and isinstance(v, int)):
                out.append((path + k, f"TYPE {type(v).__name__} != {t}"))
        if isinstance(v, dict) and ("properties" in sub or "additionalProperties" in sub):
            check(sub, v, path + k + ".", out)

root = sys.argv[1] if len(sys.argv) > 1 else "."
files = sorted(glob.glob(os.path.join(root, "contracts/commands/*.yaml")))
files = [f for f in files if not f.endswith("_index.yaml")]
bad = novm = 0
rows = []
for f in files:
    d = yaml.safe_load(open(f))
    kind = d.get("kind", os.path.basename(f))
    vm = (d.get("examples") or {}).get("validMinimal")
    if not isinstance(vm, dict) or "params" not in vm:
        novm += 1
        rows.append((kind, "NO-validMinimal-params", ""))
        continue
    out = []
    check(d.get("paramsSchema") or {}, vm["params"], "params.", out)
    if out:
        bad += 1
        for p, why in out:
            rows.append((kind, why, p))
# CONTROL: a planted violation must be seen, or the run is VOID (this is the whole
# reason a "0 violations" line can be read at all).
probe = {"required": ["mustHave"], "additionalProperties": False,
         "properties": {"declared": {"type": "string"}}}
seen = []
check(probe, {"undeclared": 1, "declared": 2}, "probe.", seen)
kinds = {w for _, w in seen}
if not {"UNKNOWN-KEY", "MISSING-REQUIRED"} <= kinds or not any(w.startswith("TYPE") for w in kinds):
    print(f"VOID - the planted control was not fully detected: {sorted(kinds)}")
    raise SystemExit(2)
if not files:
    print("VOID - no contracts read"); raise SystemExit(2)
print(f"control: planted unknown key, missing required and wrong type all detected")
print(f"contracts read {len(files)} · with a validMinimal.params {len(files)-novm} · without {novm}")
print(f"VIOLATE THEIR OWN SCHEMA: {bad} of {len(files)-novm}")
for k, why, p in rows:
    print(f"  {k:<28} {why:<22} {p}")

raise SystemExit(1 if bad else 0)
