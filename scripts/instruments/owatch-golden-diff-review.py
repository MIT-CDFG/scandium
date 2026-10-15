#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# owatch-golden-diff-review.py -- OWNER: O-WATCH (conformance & performance audit desk)
# READ-ONLY: reads git objects only; never writes to the repository.
"""O-WATCH golden-diff review — the mechanical rule of operator WORD 17 (2) / O-INT WORD 16 (3).

usage: golden_diff_review.py <repo> <base> <head> [--from 0.1.0] [--to 1.0.2]

PASS (exit 0) only if, for EVERY package golden (goldens/**.appname) changed between base and head:
  - the change is a modification (not add / delete / rename)
  - both archives list the same members in the same order
  - every member other than manifest.json is byte-identical
  - manifest.json's bytes equal the base bytes with the version string FROM replaced by TO exactly once,
    and the parsed JSON differs in createdWith.version alone, FROM -> TO
FAIL (exit 1) if any package breaks the rule. VOID (exit 2) if nothing could be judged: no package in the
diff, an unreadable object, or a negative control that did not fail.

NEGATIVE CONTROL, minted per run: one changed package's head bytes, one non-manifest member chosen at random
with one byte flipped, must FAIL the same comparator; and the same package with its minReader altered must
FAIL. If either passes, the run is VOID.

Reported beside the verdict, not part of it: every non-package path in the diff, and the sites the E-11 bump is
measured to touch (Cargo.toml, crates/appname-py/pyproject.toml, Cargo.lock, THIRD-PARTY-NOTICES.md,
tests/conformance/src/lib.rs) that are absent from the diff.
"""
import io, json, random, subprocess, sys, zipfile

def git(repo, *args, binary=False):
    r = subprocess.run(["git", "-C", repo, *args], capture_output=True)
    if r.returncode != 0:
        raise RuntimeError("git %s: %s" % (" ".join(args), r.stderr.decode(errors="replace").strip()))
    return r.stdout if binary else r.stdout.decode()

def members(blob):
    z = zipfile.ZipFile(io.BytesIO(blob))
    return [(i.filename, z.read(i.filename)) for i in z.infolist()]

def judge(old_blob, new_blob, v_from, v_to):
    """Return (ok, reason)."""
    try:
        a, b = members(old_blob), members(new_blob)
    except Exception as e:  # unreadable archive is a failure of the subject, not of the instrument
        return False, "unreadable archive: %s" % e
    if [n for n, _ in a] != [n for n, _ in b]:
        return False, "member list/order differs: -%s +%s" % (
            sorted(set(n for n, _ in a) - set(n for n, _ in b)), sorted(set(n for n, _ in b) - set(n for n, _ in a)))
    reasons, saw_manifest = [], False
    for (n, x), (_, y) in zip(a, b):   # every member is judged; no early return, so the reason list is complete
        if n != "manifest.json":
            if x != y:
                reasons.append("member %s differs (%d -> %d bytes)" % (n, len(x), len(y)))
            continue
        saw_manifest = True
        old_s, new_s = x.decode("utf-8"), y.decode("utf-8")
        needle = '"version": "%s"' % v_from
        try:
            jo, jn = json.loads(old_s), json.loads(new_s)
        except Exception as e:
            reasons.append("manifest.json does not parse: %s" % e); continue
        vo, vn = jo.get("createdWith", {}).get("version"), jn.get("createdWith", {}).get("version")
        if (vo, vn) != (v_from, v_to):
            reasons.append("createdWith.version is %s -> %s, want %s -> %s" % (vo, vn, v_from, v_to))
        if old_s.count(needle) != 1:
            reasons.append("base manifest.json carries %r %d times, want 1" % (needle, old_s.count(needle)))
        elif (vo, vn) == (v_from, v_to) and old_s.replace(needle, '"version": "%s"' % v_to) != new_s:
            jo2 = dict(jo); jo2["createdWith"] = dict(jo.get("createdWith", {}), version=vn)
            reasons.append("manifest.json altered beyond createdWith.version" if jo2 != jn
                           else "manifest.json bytes differ (layout) though its JSON differs in the version alone")
    if not saw_manifest:
        reasons.append("no manifest.json member")
    if reasons:
        return False, " · ".join(reasons)
    return True, "createdWith.version %s -> %s only" % (v_from, v_to)

def rezip(pairs):
    out = io.BytesIO()
    with zipfile.ZipFile(out, "w", compression=zipfile.ZIP_STORED) as z:
        for n, d in pairs:
            z.writestr(n, d)
    return out.getvalue()

def main():
    argv = sys.argv[1:]
    if len(argv) < 3:
        print(__doc__); return 2
    repo, base, head = argv[0], argv[1], argv[2]
    v_from = argv[argv.index("--from") + 1] if "--from" in argv else "0.1.0"
    v_to = argv[argv.index("--to") + 1] if "--to" in argv else "1.0.2"
    try:
        rows = [l.split("\t") for l in git(repo, "diff", "--name-status", "-M", base, head).splitlines() if l]
    except RuntimeError as e:
        print("golden-review: VOID - %s" % e); return 2
    pkgs = [r for r in rows if r[-1].startswith("goldens/") and r[-1].endswith(".appname")]
    others = [r for r in rows if r not in pkgs]
    print("golden-review: %s..%s · %d changed paths · %d package goldens · %s -> %s" % (
        base, head, len(rows), len(pkgs), v_from, v_to))
    if not pkgs:
        print("golden-review: VOID - no package golden in the diff; nothing to judge (not a PASS)"); return 2
    fails, judged, sample = 0, 0, None
    for r in pkgs:
        st, path = r[0], r[-1]
        if st != "M":
            print("  FAIL %s  status %s (a regeneration modifies; it does not add, delete or rename)" % (path, st))
            fails += 1; continue
        try:
            ob = git(repo, "show", "%s:%s" % (base, path), binary=True)
            nb = git(repo, "show", "%s:%s" % (head, path), binary=True)
        except RuntimeError as e:
            print("golden-review: VOID - %s" % e); return 2
        ok, why = judge(ob, nb, v_from, v_to)
        judged += 1
        print("  %s %s  %s" % ("OK  " if ok else "FAIL", path, why))
        if not ok:
            fails += 1
        elif sample is None:
            sample = (path, ob, nb)
    # negative control, minted per run, through the same comparator
    if sample is None:
        print("golden-review: NEG control not runnable (no package passed); verdict stands on the FAILs above")
    else:
        path, ob, nb = sample
        pairs = members(nb)
        idx = [i for i, (n, d) in enumerate(pairs) if n != "manifest.json" and len(d) > 0]
        k = random.choice(idx)
        n, d = pairs[k]
        pos = random.randrange(len(d))
        flipped = list(pairs); flipped[k] = (n, d[:pos] + bytes([d[pos] ^ 0xFF]) + d[pos + 1:])
        neg1, why1 = judge(ob, rezip(flipped), v_from, v_to)
        man = [(nn, dd.replace(b'"minReader": "0.1.0"', b'"minReader": "9.9.9"') if nn == "manifest.json" else dd)
               for nn, dd in pairs]
        neg2, why2 = judge(ob, rezip(man), v_from, v_to)
        print("golden-review: NEG minted on %s — byte %d of %s flipped -> %s (%s) · minReader altered -> %s (%s)" % (
            path, pos, n, "PASS" if neg1 else "FAIL", why1, "PASS" if neg2 else "FAIL", why2))
        if neg1 or neg2:
            print("golden-review: VOID - a negative control passed; the comparator cannot see a defect"); return 2
    expected = ["Cargo.toml", "crates/appname-py/pyproject.toml", "Cargo.lock", "THIRD-PARTY-NOTICES.md",
                 "tests/conformance/src/lib.rs"]
    changed = set(r[-1] for r in rows)
    print("golden-review: other paths in the diff (%d):" % len(others))
    for r in others:
        print("    %s\t%s" % (r[0], r[-1]))
    absent = [p for p in expected if p not in changed]
    print("golden-review: measured E-11 sites ABSENT from the diff: %s" % (", ".join(absent) if absent else "none"))
    if fails:
        print("golden-review: FAIL - %d of %d package goldens break the rule" % (fails, len(pkgs))); return 1
    print("golden-review: PASS - %d of %d package goldens: createdWith.version %s -> %s only, every other member byte-identical" % (
        judged, len(pkgs), v_from, v_to))
    return 0

if __name__ == "__main__":
    sys.exit(main())
