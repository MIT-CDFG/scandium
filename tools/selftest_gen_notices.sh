#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# Activation self-test for tools/gen_notices.py port-obligation records
# AND dep-record-pinned vendored C library texts.
#
# Part 1 — port records. Proves the ACTIVE path works BEFORE any port
# merges: builds a scratch tree containing every landing path named by
# deps/port-*.yaml, regenerates the notices against it (--landing-root),
# and asserts that every record renders ACTIVE with its recorded copyright
# line AND its full license text present in the appendix. Then regenerates
# against an empty scratch tree and asserts every record renders PENDING.
#
# Part 3 — vendored-asset records (recordKind: vendored-assets). Proves,
# BEFORE any vendor drop lands: the ACTIVE path (landing paths + the
# co-located license copy materialized from the record's sha-pinned inline
# text -> full text, copyright lines, notice surfaces emitted), the
# PENDING one-line row, and three red paths: landing without the
# co-located license copy, a drifted co-located copy, and a tampered
# record (inline text vs sha pin) — each must FAIL generation with
# VENDORED-ASSET-DRIFT.
#
# Part 4 — data-package records (recordKind: data-package,
# appname.data-package.v1). Proves: the ACTIVE path (dataPaths + the
# co-located license copy + a payload whose manifest digest matches the
# record's payloadSha256 pin -> full text, copyright, payload-verified
# line, basis, shareAlikeWorkflow, notice surfaces emitted), the PENDING
# one-line row, and four red paths: staged data without the co-located
# license copy, a drifted license copy, a TAMPERED PAYLOAD (one byte
# appended to one staged file — the share-alike workflow tripwire), and a
# tampered record (pending-time text source vs sha pin, red even with
# nothing landed) — each must FAIL generation with DATA-PACKAGE-DRIFT.
# The scratch payload is synthesized per-record so the proof does not
# depend on the real data being staged: a temporary record copy pins the
# synthetic payload's freshly computed manifest (the pin ALGORITHM is the
# real one; the tamper test then proves any byte change breaks it).
#
# Part 5 — shipsViaBuildDep branch (build-edge vendored payloads; first
# case: deps/mlua.yaml pinning lua-src's bundled Lua 5.4.8 lua.h). Proves,
# against the REAL resolver (imported, no reimplementation) and the real
# registry package sources: a flagged entry ACTIVATES when its crate is
# present ONLY in the build/dev (tools) closure — the shipped graph empty —
# with full sha-verified extraction; and an unflagged control copy of the
# same entry stays PENDING in the identical situation (the shipped-graph
# rule is unchanged for everything unflagged). Added at the O-INT fix
# round 2026-08-18: the original branch proof ran scratch-only and left no
# committed regression pin.
#
# Part 2 — vendoredTexts pins. For every deps record with vendoredTexts:
#   * crate SHIPPED (in Cargo.lock): asserts the full vendored text and the
#     "sha256 verified" line are in the generated output, plus any
#     requiredNotice acknowledgment verbatim;
#   * crate NOT shipped yet: asserts the pending one-line row, then proves
#     the ACTIVE path BEFORE the crate lands via --assume-shipped against
#     the real registry archives (text + sha verification + requiredNotice);
#   * drift goes red: a scratch deps dir with a tampered sha256 pin, and one
#     with a moved path, must both make generation FAIL (non-zero exit,
#     VENDORED-TEXT-DRIFT) — the property that a future -sys version bump
#     changing the license text turns the notices gate red rather than
#     silently shipping a stale text.
#
# Nothing is written to the committed THIRD-PARTY-NOTICES.md (--out into
# the scratch dir).
#
# Run from anywhere: tools/selftest_gen_notices.sh

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SCRATCH="$(mktemp -d "${TMPDIR:-/tmp}/gen-notices-selftest.XXXXXX")"
trap 'rm -rf "$SCRATCH"' EXIT

TREE="$SCRATCH/tree"
EMPTY="$SCRATCH/empty"
mkdir -p "$TREE" "$EMPTY"

# Materialize every landing path named by the port records.
python3 - "$ROOT" "$TREE" <<'PY'
import os, sys, yaml
root, tree = sys.argv[1], sys.argv[2]
for f in sorted(os.listdir(os.path.join(root, "deps"))):
    if not f.endswith(".yaml"):
        continue
    r = yaml.safe_load(open(os.path.join(root, "deps", f), encoding="utf-8"))
    if isinstance(r, dict) and r.get("recordKind") == "port-obligation":
        for p in r["landingPaths"]:
            full = os.path.join(tree, p)
            os.makedirs(os.path.dirname(full), exist_ok=True)
            open(full, "w").close()
PY

ACTIVE_MD="$SCRATCH/notices-active.md"
PENDING_MD="$SCRATCH/notices-pending.md"
python3 "$ROOT/tools/gen_notices.py" --landing-root "$TREE" --out "$ACTIVE_MD"
python3 "$ROOT/tools/gen_notices.py" --landing-root "$EMPTY" --out "$PENDING_MD"

# Assertions, driven by the records themselves (no hand-kept list here).
python3 - "$ROOT" "$ACTIVE_MD" "$PENDING_MD" <<'PY'
import os, sys, yaml
root, active_md, pending_md = sys.argv[1], sys.argv[2], sys.argv[3]
active = open(active_md, encoding="utf-8").read()
pending = open(pending_md, encoding="utf-8").read()
PROTO_ROOT = os.environ.get("APPNAME_PROTO_ROOT", "") or next(
    (os.path.abspath(c) for c in (os.path.join(root, os.pardir, "appname-proto"),
                                  os.path.join(root, os.pardir, os.pardir, "appname-proto"))
     if os.path.isdir(c)), os.path.join(root, os.pardir, "appname-proto"))
bad = 0

def check(cond, msg):
    global bad
    print(("PASS " if cond else "FAIL ") + msg)
    if not cond:
        bad = 1

n = 0
for f in sorted(os.listdir(os.path.join(root, "deps"))):
    if not f.endswith(".yaml"):
        continue
    r = yaml.safe_load(open(os.path.join(root, "deps", f), encoding="utf-8"))
    if not (isinstance(r, dict) and r.get("recordKind") == "port-obligation"):
        continue
    n += 1
    k = r["key"]
    if r.get("licenseTextPath"):
        body = open(os.path.join(PROTO_ROOT, r["licenseTextPath"]),
                    encoding="utf-8", errors="replace").read()
    else:
        body = r["licenseTextInline"]
    check(all(c in active for c in r["copyright"]),
          f"{k}: copyright line(s) emitted when ACTIVE")
    check(body.rstrip("\n") in active,
          f"{k}: full license text emitted when ACTIVE")
    check(all(p in active for p in r["landingPaths"]),
          f"{k}: landing path(s) named when ACTIVE")
    for s in r.get("noticeSurfaces") or []:
        check(s in active, f"{k}: additional surface line emitted when ACTIVE")
    check(f"- **{k}** — `{r['license']}` (`deps/{f}`) — activates when" in pending,
          f"{k}: renders as PENDING one-line row in the empty tree")
    # The real invariant is that the record's ACTIVE entry is absent while
    # pending — NOT that its license text is absent from the whole file: the
    # appendix dedups texts by sha256, so a record whose text is byte-equal
    # to a shipped crate's (e.g. an inline Apache-2.0) collides with an
    # unrelated emission. (Fixed 2026-08-17; the text-presence proxy made
    # this assertion red at HEAD once aces-dev landed its inline Apache
    # text — a false alarm, no actual pending leak.)
    check(f"#### {r.get('name', k)}  (`deps/{f}`)" not in pending,
          f"{k}: ACTIVE entry NOT emitted while PENDING")

check(n >= 5, f"record count sanity ({n} port records found, expected >= 5)")
sys.exit(bad)
PY

# ---- Part 2: vendoredTexts pins ------------------------------------------

# Pre-landing activation run: assume every pending pinned crate shipped at
# the version present in the local registry sources (the sealed archives the
# records were read from), so the ACTIVE rendering is proven before the
# crate enters Cargo.lock.
ASSUME="$(python3 - "$ROOT" <<'PY'
import glob, hashlib, os, re, sys, yaml
root = sys.argv[1]
lock = open(os.path.join(root, "Cargo.lock"), encoding="utf-8").read()
pairs = []
for f in sorted(os.listdir(os.path.join(root, "deps"))):
    if not f.endswith(".yaml"):
        continue
    r = yaml.safe_load(open(os.path.join(root, "deps", f), encoding="utf-8"))
    if not (isinstance(r, dict) and r.get("vendoredTexts")):
        continue
    for e in r["vendoredTexts"]:
        c = e["crate"]
        if f'name = "{c}"\n' in lock:
            continue  # really shipped; no assumption needed
        for d in glob.glob(os.path.expanduser(f"~/.cargo/registry/src/*/{c}-*")):
            v = os.path.basename(d)[len(c) + 1:]
            p = os.path.join(d, e["path"])
            if not (re.match(r"^\d", v) and os.path.isfile(p)):
                continue
            body = open(p, encoding="utf-8", errors="replace").read()
            # only the sealed archive the record was read from (sha match);
            # sha-drift redness is proven separately by the tamper tests
            if hashlib.sha256(body.encode()).hexdigest() == e["sha256"]:
                pairs.append(f"{c}={v}")
print(",".join(sorted(set(pairs))))
PY
)"
VEND_MD="$SCRATCH/notices-vendored-active.md"
python3 "$ROOT/tools/gen_notices.py" --landing-root "$EMPTY" \
    --assume-shipped "$ASSUME" --out "$VEND_MD"

# Assertions driven by the records; plus the two red paths on scratch deps.
python3 - "$ROOT" "$PENDING_MD" "$VEND_MD" "$SCRATCH" <<'PY'
import os, shutil, subprocess, sys, yaml
root, pending_md, vend_md, scratch = sys.argv[1:5]
plain = open(pending_md, encoding="utf-8").read()   # no assumptions run
assumed = open(vend_md, encoding="utf-8").read()    # --assume-shipped run
lock = open(os.path.join(root, "Cargo.lock"), encoding="utf-8").read()
bad = 0

def check(cond, msg):
    global bad
    print(("PASS " if cond else "FAIL ") + msg)
    if not cond:
        bad = 1

def registry_body(crate, path):
    import glob
    for d in sorted(glob.glob(os.path.expanduser(f"~/.cargo/registry/src/*/{crate}-*"))):
        full = os.path.join(d, path)
        if os.path.isfile(full):
            return open(full, encoding="utf-8", errors="replace").read()
    return None

vend_recs = []
for f in sorted(os.listdir(os.path.join(root, "deps"))):
    if not f.endswith(".yaml"):
        continue
    r = yaml.safe_load(open(os.path.join(root, "deps", f), encoding="utf-8"))
    if isinstance(r, dict) and r.get("vendoredTexts"):
        vend_recs.append((f, r))

shipped_rec = None  # a record whose crate IS shipped, for the red paths
for f, r in vend_recs:
    name = r.get("name")
    for e in r["vendoredTexts"]:
        c, p = e["crate"], e["path"]
        body = registry_body(c, p)
        check(body is not None, f"{f}: {c} {p} readable in local registry sources")
        if body is None:
            continue
        in_lock = f'name = "{c}"\n' in lock
        target = assumed if not in_lock else plain
        which = "assume-shipped" if not in_lock else "shipped"
        if in_lock and shipped_rec is None:
            shipped_rec = (f, r)
        check(body.rstrip("\n") in target,
              f"{f}: {c} {p} full vendored text emitted ({which})")
        check(f"sha256 verified `{e['sha256'][:16]}…`" in target,
              f"{f}: {c} {p} sha256-verified line emitted ({which})")
        if not in_lock:
            check(f"activates when `{c}` enters the shipped dependency graph" in plain,
                  f"{f}: {c} renders as PENDING row before the crate lands")
            check(body.rstrip("\n") not in plain,
                  f"{f}: {c} {p} text NOT emitted while PENDING")
    if r.get("requiredNotice"):
        check(r["requiredNotice"] in assumed,
              f"{f}: requiredNotice acknowledgment carried verbatim when active")

check(len(vend_recs) >= 5, f"record count sanity ({len(vend_recs)} vendoredTexts records, expected >= 5)")

# Red paths: tampered sha pin and moved path must FAIL generation.
assert shipped_rec, "need at least one shipped vendoredTexts record for red-path tests"
for mode in ("sha", "path"):
    tdir = os.path.join(scratch, f"deps-tampered-{mode}")
    shutil.copytree(os.path.join(root, "deps"), tdir)
    f, r = shipped_rec
    r2 = yaml.safe_load(open(os.path.join(tdir, f), encoding="utf-8"))
    if mode == "sha":
        r2["vendoredTexts"][0]["sha256"] = "0" * 64
    else:
        r2["vendoredTexts"][0]["path"] = "no/such/LICENSE"
    with open(os.path.join(tdir, f), "w", encoding="utf-8") as fh:
        yaml.safe_dump(r2, fh)
    res = subprocess.run(
        [sys.executable, os.path.join(root, "tools", "gen_notices.py"),
         "--deps-dir", tdir, "--landing-root", os.path.join(scratch, "empty"),
         "--out", os.path.join(scratch, f"tampered-{mode}.md")],
        capture_output=True, text=True)
    check(res.returncode != 0 and "VENDORED-TEXT-DRIFT" in res.stderr,
          f"tampered {mode} pin ({f}) turns generation RED (exit {res.returncode})")
sys.exit(bad)
PY

# ---- Part 3: vendored-asset records --------------------------------------

ASSET_TREE="$SCRATCH/asset-tree"
mkdir -p "$ASSET_TREE"

# Materialize landing paths + the co-located license copy (byte-exact from
# the record's sha-pinned inline text) for every vendored-asset record.
python3 - "$ROOT" "$ASSET_TREE" <<'PY'
import os, sys, yaml
root, tree = sys.argv[1], sys.argv[2]
n = 0
for f in sorted(os.listdir(os.path.join(root, "deps"))):
    if not f.endswith(".yaml"):
        continue
    r = yaml.safe_load(open(os.path.join(root, "deps", f), encoding="utf-8"))
    if isinstance(r, dict) and r.get("recordKind") == "vendored-assets":
        n += 1
        for p in r["landingPaths"]:
            os.makedirs(os.path.join(tree, p), exist_ok=True)
        lic = os.path.join(tree, r["licenseTextRepoPath"])
        os.makedirs(os.path.dirname(lic), exist_ok=True)
        with open(lic, "w", encoding="utf-8") as fh:
            fh.write(r["licenseTextInline"])
assert n >= 1, "no vendored-asset records found"
PY

ASSET_MD="$SCRATCH/notices-asset-active.md"
python3 "$ROOT/tools/gen_notices.py" --landing-root "$ASSET_TREE" --out "$ASSET_MD"

python3 - "$ROOT" "$ASSET_MD" "$PENDING_MD" "$ASSET_TREE" "$SCRATCH" <<'PY'
import os, shutil, subprocess, sys, yaml
root, asset_md, pending_md, asset_tree, scratch = sys.argv[1:6]
active = open(asset_md, encoding="utf-8").read()
pending = open(pending_md, encoding="utf-8").read()
bad = 0

def check(cond, msg):
    global bad
    print(("PASS " if cond else "FAIL ") + msg)
    if not cond:
        bad = 1

def gen(landing_root, deps_dir=None, out="asset-red.md"):
    args = [sys.executable, os.path.join(root, "tools", "gen_notices.py"),
            "--landing-root", landing_root, "--out", os.path.join(scratch, out)]
    if deps_dir:
        args += ["--deps-dir", deps_dir]
    return subprocess.run(args, capture_output=True, text=True)

recs = []
for f in sorted(os.listdir(os.path.join(root, "deps"))):
    if not f.endswith(".yaml"):
        continue
    r = yaml.safe_load(open(os.path.join(root, "deps", f), encoding="utf-8"))
    if isinstance(r, dict) and r.get("recordKind") == "vendored-assets":
        recs.append((f, r))

for f, r in recs:
    k = r["key"]
    body = r["licenseTextInline"]
    check(all(c in active for c in r["copyright"]),
          f"{k}: copyright line(s) emitted when ACTIVE")
    check(body.rstrip("\n") in active,
          f"{k}: full license text emitted when ACTIVE")
    check(f"sha256 verified `{r['licenseTextSha256'][:16]}…`" in active,
          f"{k}: sha256-verified line emitted when ACTIVE")
    check(all(p in active for p in r["landingPaths"]),
          f"{k}: landing path(s) named when ACTIVE")
    for s in r.get("noticeSurfaces") or []:
        check(s in active, f"{k}: additional surface line emitted when ACTIVE")
    check(f"- **{k}** — `{r['license']}` (`deps/{f}`) — activates when" in pending,
          f"{k}: renders as PENDING one-line row in the empty tree")
    # Section-scoped negative (see the part-1 note on appendix dedup).
    check(f"#### {r.get('name', k)}  (`deps/{f}`)" not in pending,
          f"{k}: ACTIVE entry NOT emitted while PENDING")

# Red path 1: assets landed but the co-located license copy is missing.
f, r = recs[0]
bare = os.path.join(scratch, "asset-tree-no-license")
for p in r["landingPaths"]:
    os.makedirs(os.path.join(bare, p), exist_ok=True)
res = gen(bare, out="asset-red1.md")
check(res.returncode != 0 and "VENDORED-ASSET-DRIFT" in res.stderr,
      f"{r['key']}: landing without the license copy turns generation RED "
      f"(exit {res.returncode})")

# Red path 2: co-located copy drifted from the pin.
drift = os.path.join(scratch, "asset-tree-drift")
shutil.copytree(asset_tree, drift)
with open(os.path.join(drift, r["licenseTextRepoPath"]), "a",
          encoding="utf-8") as fh:
    fh.write("tampered\n")
res = gen(drift, out="asset-red2.md")
check(res.returncode != 0 and "VENDORED-ASSET-DRIFT" in res.stderr,
      f"{r['key']}: drifted co-located copy turns generation RED "
      f"(exit {res.returncode})")

# Red path 3: tampered record (inline text no longer matches the sha pin)
# is RED at load, even with nothing landed.
tdir = os.path.join(scratch, "deps-tampered-asset")
shutil.copytree(os.path.join(root, "deps"), tdir)
r2 = yaml.safe_load(open(os.path.join(tdir, f), encoding="utf-8"))
r2["licenseTextInline"] += "tampered\n"
with open(os.path.join(tdir, f), "w", encoding="utf-8") as fh:
    yaml.safe_dump(r2, fh)
res = gen(os.path.join(scratch, "empty"), deps_dir=tdir, out="asset-red3.md")
check(res.returncode != 0 and "VENDORED-ASSET-DRIFT" in res.stderr,
      f"{r['key']}: tampered record inline text turns generation RED "
      f"even while PENDING (exit {res.returncode})")

sys.exit(bad)
PY

# ---- Part 4: data-package records ----------------------------------------

python3 - "$ROOT" "$PENDING_MD" "$SCRATCH" <<'PY'
import os, shutil, subprocess, sys, yaml
root, pending_md, scratch = sys.argv[1:4]
pending = open(pending_md, encoding="utf-8").read()
sys.path.insert(0, os.path.join(root, "tools"))
import gen_notices as gn  # the REAL manifest algorithm, no reimplementation
bad = 0

def check(cond, msg):
    global bad
    print(("PASS " if cond else "FAIL ") + msg)
    if not cond:
        bad = 1

def gen(landing_root, deps_dir, out):
    return subprocess.run(
        [sys.executable, os.path.join(root, "tools", "gen_notices.py"),
         "--landing-root", landing_root, "--deps-dir", deps_dir,
         "--out", os.path.join(scratch, out)],
        capture_output=True, text=True)

recs = []
for f in sorted(os.listdir(os.path.join(root, "deps"))):
    if not f.endswith(".yaml"):
        continue
    r = yaml.safe_load(open(os.path.join(root, "deps", f), encoding="utf-8"))
    if isinstance(r, dict) and r.get("recordKind") == "data-package":
        recs.append((f, r))
check(len(recs) >= 1, f"record count sanity ({len(recs)} data-package records found, expected >= 1)")

PROTO_ROOT = os.environ.get("APPNAME_PROTO_ROOT", "") or next(
    (os.path.abspath(c) for c in (os.path.join(root, os.pardir, "appname-proto"),
                                  os.path.join(root, os.pardir, os.pardir, "appname-proto"))
     if os.path.isdir(c)), os.path.join(root, os.pardir, "appname-proto"))
for f, r in recs:
    k = r["key"]
    # PENDING proof rides the real record against the empty tree (part 1 run).
    check(f"- **{k}** — `{r['license']}` (`deps/{f}`) — activates when" in pending,
          f"{k}: renders as PENDING one-line row in the empty tree")
    if r.get("licenseTextPath"):
        body = open(os.path.join(PROTO_ROOT, r["licenseTextPath"]),
                    encoding="utf-8", errors="replace").read()
    else:
        body = r["licenseTextInline"]
    # Section-scoped negative (see the part-1 note on appendix dedup).
    check(f"#### {r.get('name', k)}  (`deps/{f}`)" not in pending,
          f"{k}: ACTIVE entry NOT emitted while PENDING")

    # ACTIVE + red-path proofs on a SYNTHESIZED payload with a re-pinned
    # scratch record copy — the pin algorithm is the real one (imported),
    # so the proof holds before any real staging lands.
    tree = os.path.join(scratch, f"data-tree-{k}")
    for p in r["dataPaths"]:
        d = os.path.join(tree, p)
        os.makedirs(d, exist_ok=True)
        for name, content in (("alpha.xml", b"<synthetic a='1'/>\n"),
                              ("beta.xml", b"<synthetic b='2'/>\n")):
            with open(os.path.join(d, name), "wb") as fh:
                fh.write(content)
    lic = os.path.join(tree, r["licenseTextRepoPath"])
    os.makedirs(os.path.dirname(lic), exist_ok=True)
    with open(lic, "w", encoding="utf-8") as fh:
        fh.write(body)
    digest, count = gn.payload_manifest(tree, r["dataPaths"])
    ddeps = os.path.join(scratch, f"deps-data-{k}")
    shutil.copytree(os.path.join(root, "deps"), ddeps)
    r2 = yaml.safe_load(open(os.path.join(ddeps, f), encoding="utf-8"))
    r2["payloadSha256"] = digest
    with open(os.path.join(ddeps, f), "w", encoding="utf-8") as fh:
        yaml.safe_dump(r2, fh, allow_unicode=True)
    res = gen(tree, ddeps, f"data-active-{k}.md")
    check(res.returncode == 0, f"{k}: generation succeeds on the synthetic staged tree")
    active = open(os.path.join(scratch, f"data-active-{k}.md"), encoding="utf-8").read() if res.returncode == 0 else ""
    check(all(c in active for c in r["copyright"]),
          f"{k}: copyright line(s) emitted when ACTIVE")
    check(body.rstrip("\n") in active,
          f"{k}: full license text emitted when ACTIVE")
    check(f"sha256 verified `{r['licenseTextSha256'][:16]}…`" in active,
          f"{k}: license sha256-verified line emitted when ACTIVE")
    check(f"payload manifest sha256 verified `{digest[:16]}…` ({count} files)" in active,
          f"{k}: payload-manifest-verified line emitted when ACTIVE")
    if r.get("basis"):
        check(str(r["basis"]).strip() in active,
              f"{k}: basis line emitted verbatim when ACTIVE")
    if r.get("shareAlikeWorkflow"):
        check(str(r["shareAlikeWorkflow"]).strip() in active,
              f"{k}: share-alike workflow note rides the entry when ACTIVE")
    for s in r.get("noticeSurfaces") or []:
        check(s in active, f"{k}: additional surface line emitted when ACTIVE")

    # Red path 1: data staged but the co-located license copy is missing.
    os.rename(lic, lic + ".hidden")
    res = gen(tree, ddeps, f"data-red1-{k}.md")
    check(res.returncode != 0 and "DATA-PACKAGE-DRIFT" in res.stderr,
          f"{k}: staging without the license copy turns generation RED (exit {res.returncode})")
    os.rename(lic + ".hidden", lic)

    # Red path 2: drifted co-located license copy.
    with open(lic, "a", encoding="utf-8") as fh:
        fh.write("tampered\n")
    res = gen(tree, ddeps, f"data-red2-{k}.md")
    check(res.returncode != 0 and "DATA-PACKAGE-DRIFT" in res.stderr,
          f"{k}: drifted license copy turns generation RED (exit {res.returncode})")
    with open(lic, "w", encoding="utf-8") as fh:
        fh.write(body)

    # Red path 3: tampered payload — one byte appended to one staged file
    # (the share-alike workflow tripwire).
    victim = os.path.join(tree, r["dataPaths"][0], "alpha.xml")
    with open(victim, "ab") as fh:
        fh.write(b"!")
    res = gen(tree, ddeps, f"data-red3-{k}.md")
    check(res.returncode != 0 and "DATA-PACKAGE-DRIFT" in res.stderr,
          f"{k}: modified shipped data without a record amendment turns generation RED (exit {res.returncode})")

    # Red path 4: tampered record pin is RED at load, even with nothing landed.
    tdeps = os.path.join(scratch, f"deps-data-tampered-{k}")
    shutil.copytree(os.path.join(root, "deps"), tdeps)
    r3 = yaml.safe_load(open(os.path.join(tdeps, f), encoding="utf-8"))
    r3["licenseTextSha256"] = "0" * 64
    with open(os.path.join(tdeps, f), "w", encoding="utf-8") as fh:
        yaml.safe_dump(r3, fh, allow_unicode=True)
    res = gen(os.path.join(scratch, "empty"), tdeps, f"data-red4-{k}.md")
    check(res.returncode != 0 and "DATA-PACKAGE-DRIFT" in res.stderr,
          f"{k}: tampered record pin turns generation RED even while PENDING (exit {res.returncode})")

sys.exit(bad)
PY

# ---- Part 5: shipsViaBuildDep branch (build-edge vendored payloads) ------

python3 - "$ROOT" <<'PY'
import copy, glob, hashlib, os, sys, yaml
root = sys.argv[1]
sys.path.insert(0, os.path.join(root, "tools"))
import gen_notices as gn  # the REAL resolver, no reimplementation
bad = 0

def check(cond, msg):
    global bad
    print(("PASS " if cond else "FAIL ") + msg)
    if not cond:
        bad = 1

flagged = []
for f in sorted(os.listdir(os.path.join(root, "deps"))):
    if not f.endswith(".yaml"):
        continue
    r = yaml.safe_load(open(os.path.join(root, "deps", f), encoding="utf-8"))
    if isinstance(r, dict) and any(e.get("shipsViaBuildDep")
                                   for e in r.get("vendoredTexts") or []):
        flagged.append((f, r))
check(len(flagged) >= 1,
      f"record count sanity ({len(flagged)} shipsViaBuildDep records found, expected >= 1)")

for f, r in flagged:
    for e in r["vendoredTexts"]:
        if not e.get("shipsViaBuildDep"):
            continue
        c, p = e["crate"], e["path"]
        # Locate the registry package version whose pinned path+sha match
        # (the sealed archive the record was read from; Part 2's registry
        # expectation, same machine reality).
        version = None
        for d in sorted(glob.glob(os.path.expanduser(f"~/.cargo/registry/src/*/{c}-*"))):
            full = os.path.join(d, p)
            if os.path.isfile(full):
                body = open(full, encoding="utf-8", errors="replace").read()
                if hashlib.sha256(body.encode()).hexdigest() == e["sha256"]:
                    version = os.path.basename(d)[len(c) + 1:]
                    break
        check(version is not None,
              f"{f}: {c} {p} readable + sha-matched in local registry sources")
        if version is None:
            continue
        pkgs = {"selftest-tool-id": {"name": c, "version": version}}
        # Flagged entry: crate ONLY in the tools closure -> must ACTIVATE.
        a, pend = gn.resolve_vendored_texts([(f, r)], pkgs,
                                            shipped=set(),
                                            tools={"selftest-tool-id"})
        got = [t for _, _, resolved, _ in a for t in resolved
               if t[0] == c and t[2] == p]
        check(bool(got) and got[0][3] == e["sha256"],
              f"{f}: flagged {c} entry ACTIVATES from the tools closure "
              f"(sha-verified extraction)")
        # Unflagged control: identical situation, flag stripped -> PENDING
        # (the shipped-graph rule unchanged for everything unflagged).
        r2 = copy.deepcopy(r)
        for e2 in r2["vendoredTexts"]:
            e2.pop("shipsViaBuildDep", None)
        a2, pend2 = gn.resolve_vendored_texts([(f, r2)], pkgs,
                                              shipped=set(),
                                              tools={"selftest-tool-id"})
        check(not a2 and any(fn == f for fn, _, _ in pend2),
              f"{f}: unflagged control stays PENDING from the tools closure")
sys.exit(bad)
PY


# ---- Part 6: E-06 linkingKind -- the classifier reads the enum, not prose ---
#
# The defect this guards against is the one it replaced: two exact substring
# tests over free-text fields, each short by exactly one record (libraw on the
# system arm, reuse-tool on the tool arm). The decisive assertion is (3) --
# mutate ONLY the prose and the classification must not move.

python3 - "$ROOT" <<'PY'
import os, shutil, subprocess, sys, tempfile, yaml
root = sys.argv[1]
sys.path.insert(0, os.path.join(root, "tools"))
import gen_notices as gn          # for LINKING_KINDS; nothing reimplemented
bad = 0

def check(cond, msg):
    global bad
    print(("PASS " if cond else "FAIL ") + msg)
    if not cond:
        bad = 1

scratch = tempfile.mkdtemp(prefix="selftest-e06-")
DEPS = os.path.join(root, "deps")

def gen(deps_dir, out):
    return subprocess.run(
        [sys.executable, os.path.join(root, "tools", "gen_notices.py"),
         "--deps-dir", deps_dir, "--out", os.path.join(scratch, out)],
        capture_output=True, text=True, cwd=root)

def copy_deps(name):
    d = os.path.join(scratch, name)
    shutil.copytree(DEPS, d)
    return d

def load(d, f):
    return yaml.safe_load(open(os.path.join(d, f), encoding="utf-8"))

def dump(d, f, r):
    yaml.safe_dump(r, open(os.path.join(d, f), "w", encoding="utf-8"),
                   sort_keys=False, allow_unicode=True)

# (1) Every record is classifiable: a valid enum value, or a recordKind, or one
#     of the open elections that say so in their own prose. No silent fourth
#     category -- this is what catches a NEW record added without a posture.
OPEN_ELECTIONS = {"ceres.yaml", "microsoft-crt-sdk.yaml"}
stray = []
for f in sorted(os.listdir(DEPS)):
    if not f.endswith(".yaml"):
        continue
    r = load(DEPS, f)
    if not isinstance(r, dict):
        continue
    k, rk = r.get("linkingKind"), r.get("recordKind")
    if k is not None and k not in gn.LINKING_KINDS:
        stray.append(f + ": invalid linkingKind " + repr(k))
    elif k is None and not rk and f not in OPEN_ELECTIONS:
        stray.append(f + ": no linkingKind and no recordKind")
check(not stray, "every deps record carries a valid linkingKind, a recordKind, "
                 "or is a named open election" + (" -- " + str(stray) if stray else ""))

# (2) An unknown enum value is FATAL. A classifier that ignores a value it does
#     not know is how a record stops being read with nobody noticing.
d = copy_deps("rot")
r = load(d, "qt6.yaml"); r["linkingKind"] = "dynamic_system"; dump(d, "qt6.yaml", r)
res = gen(d, "e06-rot.md")
check(res.returncode != 0 and "linkingKind" in res.stderr,
      "an unknown linkingKind turns generation RED (exit %s)" % res.returncode)

# (3) THE ONE THAT MATTERS: mutate ONLY the prose; classification must not move.
d = copy_deps("prose")
# The baseline is MEASURED, not written down. This arm asserted the literal
# "system=3", which was true the day it was written and stopped being true the
# moment E-06's shipped-library records landed (22 of them, 2026-09-14 14:5x,
# taking the count to 25). A test that hardcodes a population count fails for the
# one reason it is not about. Take the count BEFORE gutting the prose and compare.
import re as _re
_base = gen(d, "e06-prose-base.md")
_m = _re.search(r"system=(\d+)", _base.stdout)
_before = _m.group(1) if _m else "?"
r = load(d, "qt6.yaml")
r["linking"] = "this prose says nothing a grep could match"
dump(d, "qt6.yaml", r)
res = gen(d, "e06-prose.md")
body = ""
if res.returncode == 0:
    body = open(os.path.join(scratch, "e06-prose.md"), encoding="utf-8").read()
check(res.returncode == 0 and _before != "?" and ("system=%s" % _before) in res.stdout
      and "(`deps/qt6.yaml`)" in body,
      "gutting the prose `linking:` field does NOT change the classification "
      "(the enum is what is read; system count unchanged at %s)" % _before)

# (4) The converse: removing the enum value drops the record from the system
#     rows even though its prose still says "dynamic system library".
d = copy_deps("noenum")
r = load(d, "qt6.yaml"); r.pop("linkingKind", None); dump(d, "qt6.yaml", r)
res = gen(d, "e06-noenum.md")
# Exact count, not a substring: "system=2" in the output passed by accident while
# the real count was 24 ("system=24" contains it) and failed once records grew.
_after = _re.search(r"\bsystem=(\d+)\b", res.stdout)
check(res.returncode == 0 and _before != "?" and _after is not None
      and int(_after.group(1)) == int(_before) - 1,
      "removing linkingKind drops the record from the system rows, prose "
      "notwithstanding (system %s -> %s)" % (_before, _after.group(1) if _after else "?"))

# (6) ruling (x): a BUNDLED record (dynamic-system + platform) must pin its texts
#     and carry its copyright; generation STOPS otherwise, and on a drifted text.
d = copy_deps("bundled-notext")
r = load(d, "shipped-zstd.yaml"); r.pop("bundledTexts", None); dump(d, "shipped-zstd.yaml", r)
res = gen(d, "bundled-notext.md")
check(res.returncode != 0 and "BUNDLED-TEXT-MISSING" in (res.stdout + res.stderr),
      "a bundled record with no bundledTexts STOPS generation (BUNDLED-TEXT-MISSING)")
d = copy_deps("bundled-nocr")
r = load(d, "shipped-zstd.yaml"); r.pop("copyright", None); dump(d, "shipped-zstd.yaml", r)
res = gen(d, "bundled-nocr.md")
check(res.returncode != 0 and "BUNDLED-TEXT-MISSING" in (res.stdout + res.stderr),
      "a bundled record with no copyright lines STOPS generation (BUNDLED-TEXT-MISSING)")
d = copy_deps("bundled-drift")
r = load(d, "shipped-zstd.yaml"); r["bundledTexts"][0]["sha256"] = "0" * 64
dump(d, "shipped-zstd.yaml", r)
res = gen(d, "bundled-drift.md")
check(res.returncode != 0 and "BUNDLED-TEXT-DRIFT" in (res.stdout + res.stderr),
      "a bundled text whose bytes do not match the pin STOPS generation (BUNDLED-TEXT-DRIFT)")
res = gen(DEPS, "bundled-real.md")
body = open(os.path.join(scratch, "bundled-real.md"), encoding="utf-8").read()
check(res.returncode == 0 and "## Bundled third-party libraries and components" in body
      and "(`deps/shipped-zstd.yaml`)" in body and "(`deps/shipped-macqt-tinycbor.yaml`)" in body
      and "(`deps/shipped-win-zlib.yaml`)" not in body,
      "macOS notices carry the bundled section with a Homebrew record and a Qt-internal "
      "record, and no Windows-only record")

# (5) The two records the prose predicates dropped are now carried.
res = gen(DEPS, "e06-real.md")
body = open(os.path.join(scratch, "e06-real.md"), encoding="utf-8").read()
check("(`deps/libraw.yaml`)" in body,
      "libraw is carried in the notices (the LGPL record #533 showed dropped)")
check("(`deps/reuse-tool.yaml`)" in body,
      "reuse-tool is carried in the notices (the BUILD/CI TOOL role the "
      "startswith predicate missed)")

shutil.rmtree(scratch, ignore_errors=True)
sys.exit(bad)
PY

echo "selftest_gen_notices: all assertions PASS"
