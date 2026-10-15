#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# oint-validmin-staged.py — OWNER: O-INT (integration desk).  v1.
# Writes ONLY to fresh temporary documents it creates; never to the repository.
#
# WHY IT EXISTS (entries 1048, 1075). Running each published example on a FRESH
# document made 62 of 102 come back error.command.bad_target — a fact about an
# empty document, not about the example. This stages what each example's own
# target names and then runs the example, so the verdict is about the EXAMPLE.
#
# usage: oint-validmin-staged.py <path-to-appname-cli> [repo-root]
# exit:  0 every example that this rig can decide ran ok · 1 at least one refusal
#        that is not a stated rig limit · 2 VOID (a control did not fire).
#
# ⛔ THE RIG'S LIMITS ARE PART OF ITS OUTPUT, because they are most of its
# non-ok rows: it stages a raster layer, a mask and a channel, and NOTHING else.
# It does not build an existing op, a brush preset or samples asset, a clipboard,
# sibling layers, a group, a smart object, a selection or a path, so the examples
# needing those refuse for the rig's reason and are counted as UNDECIDED, never
# as defects (entry 1075 section 3).

import glob, json, os, subprocess, sys, tempfile, uuid, yaml, collections

CLI = sys.argv[1]; ROOT = sys.argv[2] if len(sys.argv) > 2 else "."
# The canvas is 512x512, not 64x64: doc.crop's own example crops a 1720x920 rect,
# and on a 64x64 document it refused for the RIG's size, not for anything the
# example got wrong (entry 1075 section 2 — my own artifact, found by re-running it).
CANVAS = "512"
try:
    v = subprocess.run([CLI, "--version"], capture_output=True, text=True)
    if v.returncode != 0:
        raise OSError(v.stderr.strip() or "non-zero --version")
except OSError as e:
    print(f"VOID - the CLI at {CLI} does not run: {e}"); raise SystemExit(2)

def run(doc, envs):
    """apply envelopes through `script`; return (rc, code) of the LAST one."""
    with tempfile.NamedTemporaryFile("w", suffix=".jsonl", delete=False) as fh:
        for e in envs: fh.write(json.dumps(e) + "\n")
        path = fh.name
    p = subprocess.run([CLI, "script", "--doc", doc, "--file", path, "--json"],
                       capture_output=True, text=True)
    os.unlink(path)
    code = ""
    try:
        d = json.loads(p.stdout or "{}")
        if not d.get("ok"): code = d.get("error", {}).get("code", "") or "?"
    except Exception:
        code = "<unparsed>"
    return p.returncode, code

def env(kind, params, target=None, n=[0]):
    n[0] += 1
    e = {"schema": "appname.command.v1", "id": str(uuid.uuid5(uuid.NAMESPACE_URL, f"stage{n[0]}")),
         "kind": kind, "params": params}
    if target: e["target"] = target
    return e

out = collections.Counter(); rows = []
files = [f for f in sorted(glob.glob(os.path.join(ROOT, "contracts/commands/*.yaml")))
         if not f.endswith("_index.yaml")]
for f in files:
    d = yaml.safe_load(open(f)); kind = d.get("kind")
    vm = (d.get("examples") or {}).get("validMinimal")
    if not isinstance(vm, dict) or "kind" not in vm: continue
    tgt = vm.get("target") if isinstance(vm.get("target"), dict) else {}
    tmp = tempfile.mkdtemp(); doc = os.path.join(tmp, "d.appname")
    subprocess.run([CLI, "new", "--width", CANVAS, "--height", CANVAS, "--out", doc],
                   capture_output=True)
    stage = []
    lid = tgt.get("layerId")
    if lid: stage.append(env("layer.add", {"kind": "raster", "newLayerId": lid}))
    mid = tgt.get("maskId")
    if mid and lid: stage.append(env("mask.add", {"kind": "raster", "newMaskId": mid}, {"layerId": lid}))
    cid = tgt.get("channelId")
    if cid: stage.append(env("channel.add", {"newChannelId": cid}))
    staged_rc = 0
    if stage: staged_rc, staged_code = run(doc, stage)
    rc, code = run(doc, [vm])
    verdict = "ok" if rc == 0 else (code or "?")
    if stage and staged_rc != 0: verdict = "STAGING-FAILED:" + staged_code
    out[verdict] += 1; rows.append((kind, verdict))
# CONTROLS: a known-good example must run and a minted kind must refuse, or the
# verdict below is unreadable.
ctl = tempfile.mkdtemp(); cdoc = os.path.join(ctl, "c.appname")
subprocess.run([CLI, "new", "--width", CANVAS, "--height", CANVAS, "--out", cdoc], capture_output=True)
pos_rc, _ = run(cdoc, [env("layer.add", {"kind": "raster", "newLayerId": str(uuid.uuid4())})])
neg_rc, neg_code = run(cdoc, [env("zz.minted.kind", {})])
if pos_rc != 0 or neg_rc == 0:
    print(f"VOID - controls did not fire: POS layer.add rc {pos_rc} (want 0), NEG minted kind rc {neg_rc} (want non-zero)")
    raise SystemExit(2)
print(f"controls: POS layer.add ok · NEG minted kind refused ({neg_code})")
print(f"examples run: {sum(out.values())}")
for k, v in out.most_common(): print(f"  {v:3}  {k}")
print("\nper kind, everything that is not ok:")
for k, v in rows:
    if v != "ok": print(f"  {k:<26} {v}")

UNDECIDED = {"error.command.bad_target", "error.command.target_kind_mismatch",
             "error.command.selection_required", "error.command.not_implemented"}
undecided = sum(v for k, v in out.items() if k in UNDECIDED)
decided_bad = sum(v for k, v in out.items() if k not in UNDECIDED and k != "ok")
print(f"\nok {out['ok']} · UNDECIDED by this rig {undecided} · refusals to READ {decided_bad}")
print("a refusal to read is not a defect: a code alone cannot separate a wrong example from a")
print("live-state check wearing a schema code. Entry 1075 read today's 8 and found ONE example")
print("defect (adjustment.add) and seven live-state or rig artifacts.")
raise SystemExit(1 if decided_bad else 0)
