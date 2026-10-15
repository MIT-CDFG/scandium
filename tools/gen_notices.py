#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
# REUSE-IgnoreStart  (this generator EMITS/handles SPDX header strings; the
# tokens below are string data, not this file's license tag — real tag above)
#
# THIRD-PARTY-NOTICES.md generator — plan §2.4 "Packaging v1" row.
# Spec: preamble rule 6 (deps records + notice carriage); INTEGRATION-NOTES
# §5.1 obligations discipline (vendor provenance obligations carried verbatim
# as ports land).
#
# Fully mechanical (rule 8): no hand-typed license text, ever.
#   * Rust crates: the workspace resolve graph (cargo metadata --frozen).
#     "Distributed" = transitive closure of NORMAL dependency edges from the
#     workspace members (all targets — conservative, cross-platform-stable).
#     Crates reachable only via build/dev edges are listed as tools, no text
#     obligation (they never enter shipped binaries).
#   * License expressions verbatim from published crate metadata.
#   * License texts read from the local registry sources of the EXACT locked
#     versions (crate root LICENSE*/COPYING*/NOTICE* + one-deep vendor/ texts
#     for -sys crates), deduplicated by sha256. Crates that ship no text in
#     the package reference the appendix's canonical text for their SPDX id
#     (canonical = the first shipped text of that lone-id expression).
#   * Non-cargo components (system links, standalone tools) come from
#     deps/*.yaml records (linking/role fields).
#   * Vendored C library texts: deps records may pin `vendoredTexts` entries
#     (crate, path-inside-package, sha256 recorded from a first-hand archive
#     read). The generic scan above only reaches crate-root + one-deep
#     vendor/ texts; -sys crates that vendor whole C trees (freetype-sys,
#     harfbuzz-sys, lz4-sys, zstd-sys) keep their C license texts deeper.
#     For every pinned entry whose crate is in the SHIPPED graph, the text
#     is re-read at generation time from the exact locked version's package
#     sources and verified against the recorded sha256 — a missing path or
#     a hash mismatch (i.e. a -sys version bump that moved or changed the
#     license text) FAILS generation and therefore the gate's notices step;
#     nothing stale can ship silently. Entries whose crate is not yet in
#     the shipped graph render as PENDING one-line rows and activate
#     mechanically via the --check drift stage when the crate lands. A
#     record-level `requiredNotice` string (e.g. the FTL credit line) is
#     emitted verbatim with the active entry.
#     BUILD-EDGE SHIPPING (`shipsViaBuildDep: true`, per-entry; first case:
#     deps/mlua.yaml pinning lua-src's bundled Lua 5.4.8 lua.h): some
#     vendoring crates are BUILD-dependencies of a -sys crate — they
#     resolve into the build/dev closure, never the normal-edge shipped
#     graph, yet their vendored C payload IS compiled into shipped
#     binaries by the consuming crate's build script. The blanket
#     "tools, no text obligation" rule is wrong for such payloads, so a
#     flagged entry's activation and text extraction ALSO key on the
#     build/dev closure (same sha256 verification against the exact
#     locked package sources; the tools-closure caption carries the
#     caveat).
#   * Port obligations: deps/port-*.yaml records (recordKind:
#     port-obligation, appname.port-record.v1) name the census source, the
#     copyright line(s), the license-text source (a vendored LICENSE under
#     appname-proto/, or census-carried inline text), and the landingPaths
#     in THIS repo. Activation is MECHANICAL: a record whose landing path
#     exists in the working tree is ACTIVE — full license text + copyright
#     + extra notice surfaces are emitted; otherwise it renders as a
#     PENDING one-line row. Post-merge, the --check drift stage forces the
#     regeneration that flips entries active (stale notices = red).
#   * Vendored asset components: deps records with recordKind:
#     vendored-assets (e.g. deps/lucide.yaml — UI icon SVGs copied into
#     shell/ at a pinned upstream commit; never a cargo dependency). The
#     record carries the upstream license text verbatim
#     (licenseTextInline, sha256-pinned from a first-hand raw-byte read at
#     the pinned commit) plus the repo path where the vendoring commit
#     must co-locate a LICENSE copy (licenseTextRepoPath). Activation is
#     landing-detected like ports: while no landingPaths entry exists the
#     record renders as a PENDING one-line row; once the vendor drop
#     lands, the co-located copy MUST exist and is re-read at every
#     generation and verified against the pinned sha256 — a missing copy
#     or drifted bytes FAILS generation (and the gate's notices step).
#     The inline text is checked against the pin at every load, so a
#     tampered record is red even while pending.
#   * Data package components: deps records with recordKind: data-package
#     (appname.data-package.v1; first case: deps/lensfun-db.yaml — the
#     CC-BY-SA-3.0 lensfun lens profile database staged under data/).
#     Clearly-separated, user-replaceable DATA shipped with the
#     application — never code, never a cargo dependency. Activation is
#     landing-detected on `dataPaths` (the licensed data itself; the
#     own-code consumer is informational and never a trigger). Active
#     records verify BOTH the co-located license copy
#     (licenseTextRepoPath vs the sha256 pin) AND the full staged payload
#     (payloadSha256, a manifest digest over every payload byte) at every
#     generation — a shipped-data modification without a record amendment
#     in the same commit is red (the share-alike workflow tripwire). The
#     record's basis and shareAlikeWorkflow fields are emitted verbatim
#     with the entry. Pending records verify their pending-time text
#     source against the pin at every load.
#   * Vendor provenance coverage: appname-proto/vendor/*/PROVENANCE.yaml
#     keys with no port-obligation or data-package record yet are listed
#     so the gap stays visible (a record is required before such a port
#     or data payload lands).
# Deterministic output: stable sort orders, no timestamps; the header pins
# the Cargo.lock sha256 the file was generated from.
#
# Usage: tools/gen_notices.py [--check] [--landing-root DIR] [--out PATH]
#   default: (re)write THIRD-PARTY-NOTICES.md
#   --check: regenerate to memory and diff against the committed file;
#            non-zero exit + diff summary on drift.
#   --landing-root DIR: check port-record landingPaths against DIR instead
#            of the repo root (activation self-test harness; see
#            tools/selftest_gen_notices.sh).
#   --out PATH: write to PATH instead of THIRD-PARTY-NOTICES.md
#            (self-test only; --check ignores it).
#   --deps-dir DIR: read deps records from DIR instead of <repo>/deps
#            (self-test only: proves the sha256 drift check goes red on a
#            tampered pin without touching the real records).
#   --assume-shipped name=version[,name=version...]: treat these crates as
#            present in the shipped graph at the given locked versions
#            (self-test only: proves pending vendoredTexts entries activate
#            correctly — text extraction + sha verification against the real
#            registry archives — BEFORE the crate actually lands).

import hashlib
import json
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
def _resolve_proto_root():
    """Locate the proto tree without hard-coding one machine's layout.

    Same shape as SPEC_TREE_CANDIDATES in gen_attestation.py and as the
    licences-dir search in shell/src/main.cpp: environment first, then the
    sibling positions a checkout and a worktree each put it in. OB-93.
    """
    env = os.environ.get("APPNAME_PROTO_ROOT", "")
    if env:
        return env
    candidates = [os.path.join(ROOT, os.pardir, "appname-proto"),
                  os.path.join(ROOT, os.pardir, os.pardir, "appname-proto")]
    for c in candidates:
        if os.path.isdir(c):
            return os.path.abspath(c)
    return os.path.abspath(candidates[0])


PROTO_ROOT = _resolve_proto_root()
PROTO_VENDOR = os.path.join(PROTO_ROOT, "vendor")
OUT = os.path.join(ROOT, "THIRD-PARTY-NOTICES.md")

try:
    import yaml
except ImportError:
    sys.exit("gen_notices: PyYAML required (system python3 has it on this machine)")


def sh(args, **kw):
    return subprocess.run(args, check=True, capture_output=True, text=True, **kw).stdout


def cargo_metadata():
    env = dict(os.environ)
    env["PATH"] = env.get("PATH", "") + ":" + os.path.expanduser("~/.cargo/bin")
    return json.loads(sh(["cargo", "metadata", "--format-version", "1", "--frozen"], cwd=ROOT, env=env))


def shipped_and_tool_sets(meta):
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    members = set(meta["workspace_members"])
    shipped, seen = set(), set(members)
    frontier = list(members)
    while frontier:  # BFS over NORMAL edges only
        nid = frontier.pop()
        for dep in nodes[nid]["deps"]:
            kinds = dep.get("dep_kinds") or [{"kind": None}]
            if any(k.get("kind") is None for k in kinds):
                if dep["pkg"] not in seen:
                    seen.add(dep["pkg"])
                    shipped.add(dep["pkg"])
                    frontier.append(dep["pkg"])
    all_ext = {p["id"] for p in meta["packages"] if p["source"]}
    shipped_ext = shipped & all_ext
    tool_ext = all_ext - shipped_ext  # reachable only via build/dev edges
    return shipped_ext, tool_ext


def registry_src_dir(name, version):
    for r in sorted(
        __import__("glob").glob(os.path.expanduser("~/.cargo/registry/src/*"))
    ):
        d = os.path.join(r, f"{name}-{version}")
        if os.path.isdir(d):
            return d
    return None


LIC_RE = re.compile(r"(?i)^(license|licence|copying|notice)([-._].*)?$")


def license_files(d):
    out = []
    for f in sorted(os.listdir(d)):
        if LIC_RE.match(f) and os.path.isfile(os.path.join(d, f)):
            out.append(os.path.join(d, f))
    vend = os.path.join(d, "vendor")
    if os.path.isdir(vend):
        for f in sorted(os.listdir(vend)):
            if LIC_RE.match(f) and os.path.isfile(os.path.join(vend, f)):
                out.append(os.path.join(vend, f))
    return out


# E-06: the closed set of linking postures a dep record may declare. Read by the
# classifier below; the prose `linking:` field stays prose and is never parsed.
#   dynamic-system  — linked at runtime as a separate, replaceable shared library
#                     or framework; not compiled into our binaries (qt6, libraw)
#   vendored-pinned — upstream SOURCE vendored at a pinned version and compiled
#                     in-tree by election (freetype, harfbuzz)
#   static          — third-party C/C++ compiled into our binaries, typically a
#                     -sys crate's vendored copy (littlecms2, lz4, zstd)
#   cargo           — an ordinary Cargo.lock dependency built from Rust sources,
#                     whose licence flows through the cargo graph and cargo-deny
# Build tools declare `recordKind: build-tool`; they link nothing, so a linking
# enum is the wrong axis for them and they carry no linkingKind.
#   not-linked      — links nothing at all: build tools, data packages, vendored
#                     assets, ported source. OPERATOR WORD 9 (1), 2026-09-14 18:46 ET
#                     (Amendment 11 (d)). Sixteen records had no honest value in the
#                     four linking kinds; fourteen of them already declared their
#                     nature on the OTHER axis, `recordKind`, and two declared
#                     neither (ceres, microsoft-crt-sdk). This member lets every
#                     record answer the linking question without writing something
#                     false, and leaves `recordKind` doing its own job below.
LINKING_KINDS = frozenset({"dynamic-system", "vendored-pinned", "static", "cargo", "not-linked"})


def load_dep_records(deps_dir):
    recs = []
    for f in sorted(os.listdir(deps_dir)):
        if f.endswith(".yaml"):
            with open(os.path.join(deps_dir, f), encoding="utf-8") as fh:
                recs.append((f, yaml.safe_load(fh)))
    return recs


def load_port_records(deps_dir):
    """deps/port-*.yaml, recordKind: port-obligation (appname.port-record.v1)."""
    recs = []
    for f in sorted(os.listdir(deps_dir)):
        if not f.endswith(".yaml"):
            continue
        with open(os.path.join(deps_dir, f), encoding="utf-8") as fh:
            r = yaml.safe_load(fh)
        if isinstance(r, dict) and r.get("recordKind") == "port-obligation":
            for field in ("key", "license", "copyright", "landingPaths"):
                if not r.get(field):
                    sys.exit(f"gen_notices: deps/{f} missing required field {field!r}")
            if not r.get("licenseTextPath") and not r.get("licenseTextInline"):
                sys.exit(f"gen_notices: deps/{f} needs licenseTextPath or licenseTextInline")
            recs.append((f, r))
    return recs


def load_asset_records(deps_dir):
    """deps/*.yaml, recordKind: vendored-assets (appname.dep-record.v1).

    Non-cargo assets copied verbatim into this repo at a pinned upstream
    commit (e.g. UI icon SVGs into shell/). The record's inline license
    text is verified against its own sha256 pin AT LOAD, every run —
    a tampered record is red even before the vendor drop lands.
    """
    recs = []
    for f in sorted(os.listdir(deps_dir)):
        if not f.endswith(".yaml"):
            continue
        with open(os.path.join(deps_dir, f), encoding="utf-8") as fh:
            r = yaml.safe_load(fh)
        if isinstance(r, dict) and r.get("recordKind") == "vendored-assets":
            for field in ("key", "license", "copyright", "landingPaths",
                          "licenseTextRepoPath", "licenseTextSha256",
                          "licenseTextInline"):
                if not r.get(field):
                    sys.exit(f"gen_notices: deps/{f} missing required field {field!r}")
            actual = hashlib.sha256(r["licenseTextInline"].encode()).hexdigest()
            if actual != r["licenseTextSha256"]:
                sys.exit(f"gen_notices: VENDORED-ASSET-DRIFT deps/{f}: "
                         f"licenseTextInline sha256 {actual} != pinned "
                         f"{r['licenseTextSha256']} — the record's carried text "
                         f"no longer matches its first-hand-read pin")
            recs.append((f, r))
    return recs


def load_data_package_records(deps_dir):
    """deps/*.yaml, recordKind: data-package (appname.data-package.v1).

    Clearly-separated, user-replaceable DATA shipped with the application
    (first case: deps/lensfun-db.yaml — the CC-BY-SA-3.0 lensfun lens
    profile database, staged under data/). Never code, never a cargo
    dependency; cargo-deny never walks it. Design merges the two landing
    precedents: activation is landing-detected like port records, but the
    triggers (`dataPaths`) are the LICENSED DATA only — the own-code
    consumer never activates attribution for data that is not in the tree
    (the pymatting deliberately-not-listed argument) — and license-text
    carriage follows the vendored-asset shape (sha256 pin + co-located
    repo copy verified at every generation). Two additions specific to
    data: `payloadSha256`, a manifest pin over every staged payload byte,
    re-verified at every generation once landed — a shipped-data
    modification without a record amendment in the same commit is a red
    gate (the share-alike workflow tripwire); and the record's `basis` /
    `shareAlikeWorkflow` fields, emitted verbatim with the entry.

    Load-time verification (every run, landed or not): the pending-time
    license text source (licenseTextPath under appname-proto/, or
    licenseTextInline) is read and sha256-verified against the pin — a
    tampered record or drifted source is red even while pending.
    """
    recs = []
    for f in sorted(os.listdir(deps_dir)):
        if not f.endswith(".yaml"):
            continue
        with open(os.path.join(deps_dir, f), encoding="utf-8") as fh:
            r = yaml.safe_load(fh)
        if isinstance(r, dict) and r.get("recordKind") == "data-package":
            for field in ("key", "license", "copyright", "dataPaths",
                          "licenseTextRepoPath", "licenseTextSha256",
                          "payloadSha256"):
                if not r.get(field):
                    sys.exit(f"gen_notices: deps/{f} missing required field {field!r}")
            if not r.get("licenseTextPath") and not r.get("licenseTextInline"):
                sys.exit(f"gen_notices: deps/{f} needs licenseTextPath or licenseTextInline")
            if r.get("licenseTextPath"):
                full = os.path.join(PROTO_ROOT, r["licenseTextPath"])
                if not os.path.isfile(full):
                    sys.exit(f"gen_notices: deps/{f} licenseTextPath not found: {full}")
                with open(full, encoding="utf-8", errors="replace") as fh:
                    body = fh.read()
                origin = f"data-package {r['key']} ({r['licenseTextPath']})"
            else:
                body = r["licenseTextInline"]
                origin = f"data-package {r['key']} (inline)"
            actual = hashlib.sha256(body.encode()).hexdigest()
            if actual != r["licenseTextSha256"]:
                sys.exit(f"gen_notices: DATA-PACKAGE-DRIFT deps/{f}: license "
                         f"text source ({origin}) sha256 {actual} != pinned "
                         f"{r['licenseTextSha256']} — the record's text source "
                         f"no longer matches its first-hand-read pin")
            recs.append((f, r, body))
    return recs


def payload_manifest(landing_root, paths):
    """Deterministic digest over every payload byte: sha256 of the sorted
    per-file manifest "sha256(bytes)  relpath\\n" across all dataPaths
    (files read as raw bytes; directories walked sorted)."""
    lines = []
    count = 0
    for p in sorted(paths):
        full = os.path.join(landing_root, p)
        files = []
        if os.path.isdir(full):
            for dirpath, dirnames, filenames in os.walk(full):
                dirnames.sort()
                for fn in sorted(filenames):
                    files.append(os.path.join(dirpath, fn))
        elif os.path.isfile(full):
            files.append(full)
        for fp in files:
            with open(fp, "rb") as fh:
                h = hashlib.sha256(fh.read()).hexdigest()
            rel = os.path.relpath(fp, landing_root).replace(os.sep, "/")
            lines.append(f"{h}  {rel}\n")
            count += 1
    digest = hashlib.sha256("".join(lines).encode()).hexdigest()
    return digest, count


def resolve_data_package_records(data_recs, landing_root):
    """Landing-detected activation for data-package records.

    Returns (active, pending):
      active:  [(fname, record, [landed paths], license body, file count)]
               — the co-located license copy (licenseTextRepoPath) re-read
               from the tree and sha256-verified against the pin, AND the
               staged payload re-hashed and verified against payloadSha256;
               a missing copy, drifted license bytes, or a payload change
               without a record amendment is a hard error (gate notices
               step goes red — shipped-data modifications must ride the
               record's share-alike workflow);
      pending: [(fname, record)] — no dataPaths entry in the tree yet.
    """
    active, pending = [], []
    for fname, r, _pending_body in data_recs:
        landed = sorted(p for p in r["dataPaths"]
                        if os.path.exists(os.path.join(landing_root, p)))
        if not landed:
            pending.append((fname, r))
            continue
        lic = os.path.join(landing_root, r["licenseTextRepoPath"])
        if not os.path.isfile(lic):
            sys.exit(f"gen_notices: DATA-PACKAGE-DRIFT deps/{fname}: data "
                     f"staged ({', '.join(landed)}) but the co-located license "
                     f"copy {r['licenseTextRepoPath']} is missing — the license "
                     f"travels with the data (record obligations)")
        with open(lic, encoding="utf-8", errors="replace") as fh:
            body = fh.read()
        actual = hashlib.sha256(body.encode()).hexdigest()
        if actual != r["licenseTextSha256"]:
            sys.exit(f"gen_notices: DATA-PACKAGE-DRIFT deps/{fname}: "
                     f"{r['licenseTextRepoPath']} sha256 {actual} != pinned "
                     f"{r['licenseTextSha256']} — the co-located license copy "
                     f"drifted from the first-hand-read pin")
        digest, count = payload_manifest(landing_root, r["dataPaths"])
        if digest != r["payloadSha256"]:
            sys.exit(f"gen_notices: DATA-PACKAGE-DRIFT deps/{fname}: staged "
                     f"payload manifest sha256 {digest} != pinned "
                     f"{r['payloadSha256']} — the shipped data changed; a "
                     f"shipped-data modification must ride a record amendment "
                     f"in the same commit (share-alike workflow: mark the "
                     f"changes, keep CC-BY-SA carriage, contribute back per "
                     f"the record's shareAlikeWorkflow field)")
        active.append((fname, r, landed, body, count))
    return active, pending


def resolve_asset_records(asset_recs, landing_root):
    """Landing-detected activation for vendored-asset records.

    Returns (active, pending):
      active:  [(fname, record, [landed paths], body)] — the co-located
               license copy (licenseTextRepoPath) re-read from the tree and
               sha256-verified against the record's pin; a missing copy or
               drifted bytes is a hard error (gate notices step goes red);
      pending: [(fname, record)] — no landing path in the tree yet.
    """
    active, pending = [], []
    for fname, r in asset_recs:
        landed = sorted(p for p in r["landingPaths"]
                        if os.path.exists(os.path.join(landing_root, p)))
        if not landed:
            pending.append((fname, r))
            continue
        lic = os.path.join(landing_root, r["licenseTextRepoPath"])
        if not os.path.isfile(lic):
            sys.exit(f"gen_notices: VENDORED-ASSET-DRIFT deps/{fname}: assets "
                     f"landed ({', '.join(landed)}) but the co-located license "
                     f"copy {r['licenseTextRepoPath']} is missing — the "
                     f"vendoring commit must carry it (record obligations)")
        with open(lic, encoding="utf-8", errors="replace") as fh:
            body = fh.read()
        actual = hashlib.sha256(body.encode()).hexdigest()
        if actual != r["licenseTextSha256"]:
            sys.exit(f"gen_notices: VENDORED-ASSET-DRIFT deps/{fname}: "
                     f"{r['licenseTextRepoPath']} sha256 {actual} != pinned "
                     f"{r['licenseTextSha256']} — the co-located license copy "
                     f"drifted from the first-hand-read pin; re-read upstream "
                     f"at the recorded commit and update record + copy together")
        active.append((fname, r, landed, body))
    return active, pending


def port_license_text(fname, r):
    """Full license text, re-read at every generation (no hand-typed text
    outside the record's census-carried inline block)."""
    p = r.get("licenseTextPath")
    if p:
        full = os.path.join(PROTO_ROOT, p)
        if not os.path.isfile(full):
            sys.exit(f"gen_notices: deps/{fname} licenseTextPath not found: {full}")
        with open(full, encoding="utf-8", errors="replace") as fh:
            return fh.read(), f"port record {r['key']} ({p})"
    return r["licenseTextInline"], f"port record {r['key']} (inline, census-carried)"


def resolve_vendored_texts(dep_records, pkgs, shipped, assumed=None, tools=None):
    """Dep-record-pinned vendored C library texts (see module docstring).

    Returns (active, pending):
      active:  [(fname, record, [(crate, version, path, sha256, body,
               covers)], [unshipped crate names])] for records with at least
               one entry whose crate is SHIPPED — every shipped entry
               verified (sha256) or the process exits red;
      pending: [(fname, record, [crate names not yet shipped])] for records
               whose pinned crates are all outside the shipped graph.
    Verification failures are hard errors: the notices gate step goes red
    rather than shipping a missing or silently-changed license text.

    Entries flagged `shipsViaBuildDep: true` additionally resolve against
    the build/dev closure (`tools`): the pinned crate is a BUILD-dependency
    whose vendored payload is compiled into shipped binaries by its
    consuming -sys crate's build script (module docstring; first case
    lua-src via deps/mlua.yaml), so the normal-edge shipped graph alone
    would never activate it.
    """
    by_name = {}
    for pid in shipped:
        by_name.setdefault(pkgs[pid]["name"], []).append(pkgs[pid]["version"])
    for n, vs in (assumed or {}).items():  # self-test hook (--assume-shipped)
        by_name.setdefault(n, []).extend(vs)
    tool_by_name = {}
    for pid in tools or ():
        tool_by_name.setdefault(pkgs[pid]["name"], []).append(pkgs[pid]["version"])
    active, pending = [], []
    for fname, r in dep_records:
        entries = r.get("vendoredTexts") if isinstance(r, dict) else None
        if not entries:
            continue
        for e in entries:
            for field in ("crate", "path", "sha256"):
                if not e.get(field):
                    sys.exit(f"gen_notices: deps/{fname} vendoredTexts entry "
                             f"missing required field {field!r}")
        resolved, unshipped = [], []
        for e in entries:
            versions = by_name.get(e["crate"])
            if not versions and e.get("shipsViaBuildDep"):
                versions = tool_by_name.get(e["crate"])
            if not versions:
                unshipped.append(e["crate"])
                continue
            for v in sorted(versions):  # every locked version must verify
                d = registry_src_dir(e["crate"], v)
                if not d:
                    sys.exit(f"gen_notices: deps/{fname} vendoredTexts: "
                             f"{e['crate']} {v} package sources not found under "
                             f"~/.cargo/registry/src (run cargo build/fetch first)")
                full = os.path.join(d, e["path"])
                if not os.path.isfile(full):
                    sys.exit(f"gen_notices: VENDORED-TEXT-DRIFT deps/{fname}: "
                             f"{e['crate']} {v} has no {e['path']} — the crate "
                             f"layout changed at a version bump; re-read the "
                             f"archive first-hand and update the record")
                with open(full, encoding="utf-8", errors="replace") as fh:
                    body = fh.read()
                actual = hashlib.sha256(body.encode()).hexdigest()
                if actual != e["sha256"]:
                    sys.exit(f"gen_notices: VENDORED-TEXT-DRIFT deps/{fname}: "
                             f"{e['crate']} {v} {e['path']} sha256 {actual} != "
                             f"recorded {e['sha256']} — the vendored license "
                             f"text changed; first-hand re-read and record "
                             f"update required before notices can regenerate")
                resolved.append((e["crate"], v, e["path"], e["sha256"], body,
                                 str(e.get("covers", "")).strip()))
        if resolved:
            active.append((fname, r, resolved, sorted(set(unshipped))))
        else:
            pending.append((fname, r, sorted(set(unshipped))))
    return active, pending


def load_provenance():
    out = []
    # W-31: an absent vendor tree used to return [] here and silently drop the
    # provenance rows. STOP instead; a host without appname-proto points
    # APPNAME_PROTO_ROOT at the vendor subset O-BUILD carries (win-transfer/).
    if not os.path.isdir(PROTO_VENDOR):
        sys.exit(f"gen_notices: no vendor provenance source: {PROTO_VENDOR} does not exist "
                 f"(set APPNAME_PROTO_ROOT to the proto tree or to O-BUILD's vendor subset)")
    for k in sorted(os.listdir(PROTO_VENDOR)):
        p = os.path.join(PROTO_VENDOR, k, "PROVENANCE.yaml")
        if os.path.isfile(p):
            with open(p, encoding="utf-8") as fh:
                out.append(yaml.safe_load(fh))
    return out


def arg_value(flag):
    if flag in sys.argv:
        i = sys.argv.index(flag)
        if i + 1 >= len(sys.argv):
            sys.exit(f"gen_notices: {flag} needs a value")
        return sys.argv[i + 1]
    return None


PLATFORMS = ("macos", "windows")
PLATFORM_LABEL = {"macos": "macOS", "windows": "Windows"}


def resolve_platform():
    """Which artifact's notice set to emit (WORD 16 (4); Amendment 11 (c) overtaken).

    One tag ships two artifacts, and each carries its own THIRD-PARTY-NOTICES.
    The Rust closure is identical on both (it is taken over all targets); what
    differs is the NATIVE set bundled beside the binary, which on macOS is the
    22 `deps/shipped-*.yaml` records that macdeployqt copies in, and on Windows
    the `deps/shipped-win-*.yaml` records of the code inside the Qt DLLs that
    windeployqt stages (ruling (w) (3)).
    """
    plat = (arg_value("--platform") or "macos").lower()
    if plat not in PLATFORMS:
        sys.exit("gen_notices: --platform must be one of %s (got %r)"
                 % (", ".join(PLATFORMS), plat))
    return plat


def is_bundled(r):
    """A BUNDLED record (ruling (x) (2)): dynamic-system AND it names the artifacts
    that ship it (`platform:`). shipped-*, shipped-win-*, shipped-macqt-* are; qt6,
    libraw, ort (no `platform:`) are truly system-linked and keep the old section."""
    return r.get("linkingKind") == "dynamic-system" and bool(r.get("platform"))


def on_platform(r, platform):
    return platform in [str(x).lower() for x in (r.get("platform") or list(PLATFORMS))]


def resolve_bundled(system_rows, platform):
    """Every bundled record of this artifact, with each pinned text re-read and
    sha256-verified. A bundled record with no `bundledTexts` or no `copyright`, a
    missing text file, or drifted bytes STOPS generation — the R-01 posture, so a
    bundled component can never again render without its licence text."""
    out = []
    for fname, r in system_rows:
        if not (is_bundled(r) and on_platform(r, platform)):
            continue
        entries, crs = r.get("bundledTexts") or [], r.get("copyright") or []
        if not entries or not crs:
            sys.exit(f"gen_notices: BUNDLED-TEXT-MISSING deps/{fname}: a bundled record must "
                     f"pin its licence text(s) (bundledTexts) and carry its copyright lines "
                     f"(copyright) — ruling (x). Record them; do not emit notices without them.")
        resolved = []
        for e in entries:
            full = os.path.join(ROOT, e["path"])
            if not os.path.isfile(full):
                sys.exit(f"gen_notices: BUNDLED-TEXT-DRIFT deps/{fname}: {e['path']} is missing")
            with open(full, encoding="utf-8", errors="replace") as fh:
                body = fh.read()
            raw = open(full, "rb").read()
            if hashlib.sha256(raw).hexdigest() != e["sha256"]:
                sys.exit(f"gen_notices: BUNDLED-TEXT-DRIFT deps/{fname}: {e['path']} sha256 "
                         f"{hashlib.sha256(raw).hexdigest()} != pinned {e['sha256']}")
            resolved.append((e["path"], e["sha256"], body, str(e.get("covers", "")).strip()))
        out.append((fname, r, resolved))
    return out


def require_native_set(platform, dep_records):
    """STOP rather than emit a notice set with no native records for its platform.

    Every record that names platforms names its own; a platform that no record
    names has an unrecorded native set, and emitting would silently omit a
    bundled library's licence — the defect B-3 was filed for.
    """
    if not any(platform in [str(x).lower() for x in (r.get("platform") or [])]
               for _, r in dep_records):
        sys.exit(
            f"gen_notices: --platform {platform}: the {PLATFORM_LABEL[platform]} native set "
            f"is NOT RECORDED. No deps/*.yaml record is tagged `platform: [{platform}]`. "
            "Emitting now would produce notices that silently omit a bundled library's "
            "licence, which is the defect B-3 was filed for. Record the manifest first, "
            "then re-run.")


def main():
    check = "--check" in sys.argv
    platform = resolve_platform()
    landing_root = arg_value("--landing-root") or ROOT
    out_path = arg_value("--out") or OUT
    deps_dir = arg_value("--deps-dir") or os.path.join(ROOT, "deps")
    meta = cargo_metadata()
    pkgs = {p["id"]: p for p in meta["packages"]}
    shipped, tools = shipped_and_tool_sets(meta)

    texts = {}       # sha256 -> {"tag": Tn, "body": str, "origin": crate}
    order = []       # tag emission order
    canonical = {}   # lone SPDX id -> tag (first shipped text seen for it)

    def intern(body, origin, carriage=False):
        # carriage: first seen as a bundled record's carriage text (ruling (x)).
        h = hashlib.sha256(body.encode()).hexdigest()
        if h not in texts:
            tag = f"T{len(texts) + 1}"
            texts[h] = {"tag": tag, "body": body, "origin": origin, "carriage": carriage}
            order.append(h)
        return texts[h]["tag"]

    crate_rows, missing_text = [], []
    for pid in sorted(shipped, key=lambda i: (pkgs[i]["name"], pkgs[i]["version"])):
        p = pkgs[pid]
        d = registry_src_dir(p["name"], p["version"])
        tags = []
        if d:
            for lf in license_files(d):
                with open(lf, encoding="utf-8", errors="replace") as fh:
                    tags.append(intern(fh.read(), f'{p["name"]} {p["version"]} ({os.path.relpath(lf, d).replace(os.sep, "/")})'))
        expr = p["license"] or "NO-EXPRESSION"
        if tags and expr not in canonical and ("OR" not in expr and "AND" not in expr and "/" not in expr):
            canonical[expr] = tags[0]
        crate_rows.append((p["name"], p["version"], expr, sorted(set(tags))))
        if not tags:
            missing_text.append((p["name"], p["version"], expr))

    # second pass: crates without shipped texts reference canonical texts
    resolved_missing = {}
    for name, version, expr in missing_text:
        arms = [a.strip() for a in re.split(r"\bOR\b|/", expr)]
        tag = next((canonical[a] for a in arms if a in canonical), None)
        resolved_missing[(name, version)] = tag

    dep_records = load_dep_records(deps_dir)
    require_native_set(platform, dep_records)
    assumed = {}
    for pair in (arg_value("--assume-shipped") or "").split(","):
        if pair:
            n, _, v = pair.partition("=")
            if not (n and v):
                sys.exit("gen_notices: --assume-shipped wants name=version pairs")
            assumed.setdefault(n, []).append(v)
    vend_active, vend_pending = resolve_vendored_texts(dep_records, pkgs, shipped,
                                                       assumed, tools)
    # E-06: classify from STRUCTURED keys, never from the prose.
    #
    # What this replaced was two exact substring tests against free-text fields,
    # and each was short by exactly one record. `"dynamic system library" in
    # linking` matched deps/qt6.yaml and missed deps/libraw.yaml, which writes
    # the same posture as "dynamic-system, optional module" -- the one dropped
    # record being the one carrying an LGPL carriage obligation (O-WATCH #533).
    # `role.startswith("BUILD TOOL")` plus `"standalone build tool" in linking`
    # caught four tools and missed deps/reuse-tool.yaml, whose role reads
    # "BUILD/CI TOOL:" (the slash defeats the prefix) and whose linking reads
    # "standalone Python CLI" (not the phrase). So the defect was on BOTH arms,
    # not one.
    #
    # The fix is not a looser grep: "harfbuzz" is hyphenated too and is
    # correctly excluded, so widening buys a false positive. The records carry a
    # closed enum instead and this reads it. An UNKNOWN value is fatal -- that is
    # enum rot, and a classifier that silently ignores a value it does not know
    # is how a record stops being read without anyone noticing. An ABSENT value
    # on a record that still describes its posture in prose is a different thing:
    # an open election, reported and never guessed.
    system_rows, toolrec_rows, unclassified = [], [], []
    for fname, r in dep_records:
        kind = r.get("linkingKind")
        if kind is not None and kind not in LINKING_KINDS:
            sys.exit(f"gen_notices: deps/{fname} carries linkingKind {kind!r}, which is "
                     f"not one of {sorted(LINKING_KINDS)}. The enum is closed: fix the "
                     f"record, or amend LINKING_KINDS deliberately.")
        if kind == "dynamic-system":
            system_rows.append((fname, r))
        elif r.get("recordKind") == "build-tool":
            toolrec_rows.append((fname, r))
        elif kind is None and not r.get("recordKind") and str(r.get("linking", "")).strip():
            unclassified.append(fname)
    if unclassified:
        print("gen_notices: WARNING — these records describe a linking posture in prose but "
              "carry no linkingKind, so this generator cannot classify them: "
              + ", ".join("deps/" + f for f in unclassified)
              + ". Each is an open election (the record says so itself), not a typo — they "
                "are reported rather than guessed. When they are ruled, this becomes fatal.",
              file=sys.stderr)

    bundled = resolve_bundled(system_rows, platform)

    prov = load_provenance()
    port_recs = load_port_records(deps_dir)
    asset_recs = load_asset_records(deps_dir)
    active_assets, pending_assets = resolve_asset_records(asset_recs, landing_root)
    data_recs = load_data_package_records(deps_dir)
    active_data, pending_data = resolve_data_package_records(data_recs, landing_root)

    # Mechanical activation: a port record is ACTIVE iff any landing path
    # exists in the working tree (landing_root defaults to the repo root).
    active_ports, pending_ports = [], []
    for fname, r in port_recs:
        landed = sorted(p for p in r["landingPaths"]
                        if os.path.exists(os.path.join(landing_root, p)))
        (active_ports if landed else pending_ports).append((fname, r, landed))
    covered_keys = {r["key"] for _, r in port_recs}
    covered_keys |= {r["key"] for _, r, _b in data_recs}
    uncovered_prov = [r for r in prov if r.get("key") not in covered_keys]

    lock_hash = hashlib.sha256(open(os.path.join(ROOT, "Cargo.lock"), "rb").read()).hexdigest()

    L = []
    L.append("<!-- SPDX-License-Identifier: GPL-3.0-only -->")
    L.append("<!-- Copyright (c) 2026 the Appname contributors -->")
    L.append("")
    L.append("# Third-Party Notices")
    L.append("")
    L.append("GENERATED FILE — `tools/gen_notices.py`; do not edit by hand.")
    L.append(f"Generated from `Cargo.lock` sha256 `{lock_hash}`, `deps/*.yaml`, and")
    L.append("vendor provenance records. License expressions are verbatim published")
    L.append("crate metadata; license texts are read from the exact locked package")
    L.append("versions and deduplicated by sha256.")
    L.append("")
    L.append(f"**Platform: {PLATFORM_LABEL[platform]}.** One tag ships two artifacts and each")
    L.append("carries its own copy of this file. The Rust closure below is identical on")
    L.append("both (it is taken over all targets); the native set under \"System-linked")
    L.append("components\" is the one bundled with THIS artifact.")
    L.append("")
    L.append("## Distributed Rust crate components")
    L.append("")
    L.append("Transitive closure of normal dependency edges from the workspace")
    L.append("(all targets; the dependency closure, not every listed crate is compiled into release binaries).")
    L.append("")
    for name, version, expr, tags in crate_rows:
        ref = ", ".join(f"[{t}]" for t in tags) if tags else None
        if not ref:
            t = resolved_missing.get((name, version))
            ref = (f"no text in package; canonical [{t}]" if t
                   else "no text in package; expression-only (see upstream)")
        L.append(f"- **{name} {version}** — `{expr}` — {ref}")
    L.append("")
    L.append("## Vendored C library license texts (dep-record pinned)")
    L.append("")
    L.append("Some -sys crates vendor whole C library trees that are compiled into")
    L.append("release binaries; their license texts live deeper than the generic")
    L.append("crate-root scan reaches. Each text below is pinned by a `vendoredTexts`")
    L.append("entry in its `deps/` record (sha256 recorded from a first-hand archive")
    L.append("read), re-read at every generation from the exact locked crate")
    L.append("version's package sources, and verified against the recorded sha256.")
    L.append("A crate version bump that changes or moves a text fails generation and")
    L.append("the gate's notices step — stale texts cannot ship silently. Pinned")
    L.append("entries whose crate has not yet entered the shipped dependency graph")
    L.append("are listed as pending and activate mechanically when the crate lands.")
    L.append("Entries marked ships-via-build-dep cover BUILD-dependency vendoring")
    L.append("crates whose bundled sources are compiled into shipped binaries by a")
    L.append("consuming crate's build script (e.g. lua-src under mlua-sys); they")
    L.append("activate from the build/dev closure with the same verification.")
    L.append("")
    for fname, r, resolved, unshipped in vend_active:
        L.append(f"### {r.get('name')}  (`deps/{fname}`)")
        L.append(f"- elected (as recorded): {r.get('licenseElected')}")
        for crate, v, path, sha, body, covers in resolved:
            tag = intern(body, f"{crate} {v} ({path}, dep-record pinned)")
            cov = f" — {covers}" if covers else ""
            L.append(f"- `{crate} {v}` `{path}` — sha256 verified `{sha[:16]}…` — "
                     f"full text [{tag}]{cov}")
        if r.get("requiredNotice"):
            L.append(f"- required acknowledgment, carried verbatim per the record's "
                     f"obligations: “{r['requiredNotice']}”")
        for c in unshipped:
            L.append(f"- pinned entry for `{c}` pending (crate not in the shipped graph yet)")
        L.append("")
    if vend_pending:
        L.append("### Pending (recorded and pinned; crate not in the shipped graph yet)")
        L.append("")
        for fname, r, unshipped in vend_pending:
            crates = ", ".join(f"`{c}`" for c in unshipped)
            n = len(r.get("vendoredTexts") or [])
            L.append(f"- **{r.get('name')}** (`deps/{fname}`) — {n} pinned vendored "
                     f"text(s) — activates when {crates} enters the shipped "
                     f"dependency graph")
        L.append("")
    L.append("## Bundled third-party libraries and components (shipped in this artifact)")
    L.append("")
    L.append(f"Each library or component below ships inside the {PLATFORM_LABEL[platform]} "
             "artifact: a separate shared library copied into it, or third-party code that")
    L.append("Qt compiles into the frameworks/DLLs it ships. Each entry gives the shipped")
    L.append("version, the upstream, the declared and elected licence, every copyright line,")
    L.append("and the full licence text(s), sha256-pinned in its `deps/` record and verified")
    L.append("at every generation (ruling (x)).")
    L.append("")
    for fname, r, resolved in bundled:
        L.append(f"### {r.get('name')} {str(r.get('candidateVersion', '')).split()[0]}  (`deps/{fname}`)")
        L.append(f"- upstream: {str(r.get('upstream', '')).strip()}")
        L.append(f"- declared license (verbatim): `{r.get('license')}`")
        L.append(f"- elected: {str(r.get('licenseElected', '')).strip()}")
        if r.get("sourceLocation"):
            L.append(f"- source for this exact version: {r['sourceLocation']}")
        for c in r.get("copyright") or []:
            L.append(f"- {c}")
        if r.get("requiredNotice"):
            L.append(f"- required acknowledgment: “{r['requiredNotice']}”")
        for path, sha, body, covers in resolved:
            tag = intern(body, f"{r.get('name')} ({path}, dep-record pinned)", carriage=True)
            cov = f" — {covers}" if covers else ""
            L.append(f"- `{path}` — sha256 verified `{sha[:16]}…` — full text [{tag}]{cov}")
        L.append("")
    L.append("## System-linked components (not shipped in-repo)")
    L.append("")
    # A record with no `platform:` is cross-platform and always carried; a
    # record that names platforms is carried only for those artifacts.
    for fname, r in [(f, r) for (f, r) in system_rows
                     if on_platform(r, platform) and not is_bundled(r)]:
        L.append(f"### {r.get('name')}  (`deps/{fname}`)")
        L.append(f"- declared license (verbatim): `{r.get('license')}`")
        L.append(f"- elected: {r.get('licenseElected')}")
        if r.get("sourceLocation"):
            L.append(f"- shipped version {str(r.get('candidateVersion', '')).split()[0]}; "
                     f"source: {r['sourceLocation']}")
        L.append(f"- obligations: {str(r.get('obligations', '')).strip()}")
        L.append("")
    L.append("## Build and development tools (not distributed; no text carriage)")
    L.append("")
    for fname, r in toolrec_rows:
        L.append(f"- **{r.get('name')}** (`deps/{fname}`) — `{r.get('license')}` — {str(r.get('obligations','')).strip()[:160]}")
    dev_names = sorted({f'{pkgs[i]["name"]} {pkgs[i]["version"]}' for i in tools})
    if dev_names:
        L.append("")
        L.append(f"Cargo build/dev-only closure ({len(dev_names)} crates; no code of")
        L.append("these crates enters shipped binaries EXCEPT where a dep record pins")
        L.append("a ships-via-build-dep vendored text, carried in the vendored-C")
        L.append("section above):")
        L.append("")
        L.append("  " + "; ".join(dev_names))
    L.append("")
    L.append("## Ported third-party components (port-obligation records)")
    L.append("")
    L.append("Driven by `deps/port-*.yaml` (`appname.port-record.v1`). Activation is")
    L.append("mechanical: a record whose landing path exists in the working tree is")
    L.append("ACTIVE (full license text + copyright below); otherwise it is PENDING.")
    L.append("The gate's `--check` drift stage forces regeneration when a landing")
    L.append("merges. License identifiers and copyright lines are as recorded by the")
    L.append("round-1 census first-hand reads (due-diligence records, not release")
    L.append("claims); texts are re-read at every generation from the vendored")
    L.append("license files (or the record's census-carried inline text).")
    L.append("")
    if active_ports:
        L.append("### Active (port landed in this repo)")
        L.append("")
        for fname, r, landed in active_ports:
            body, origin = port_license_text(fname, r)
            tag = intern(body, origin)
            L.append(f"#### {r.get('name', r['key'])}  (`deps/{fname}`)")
            L.append(f"- license (as recorded): `{r['license']}` — full text [{tag}]")
            for c in r["copyright"]:
                L.append(f"- {c}")
            L.append(f"- upstream: {r.get('upstream')} @ `{str(r.get('commit'))[:12]}`"
                     f" (census key `{r['key']}`)")
            L.append("- ported into: " + ", ".join(f"`{p}`" for p in landed))
            for s in r.get("noticeSurfaces") or []:
                L.append(f"- additional surface: {s}")
            L.append("")
    if pending_ports:
        L.append("### Pending (recorded; no landing path in this tree yet)")
        L.append("")
        for fname, r, _ in pending_ports:
            paths = ", ".join(f"`{p}`" for p in r["landingPaths"])
            L.append(f"- **{r['key']}** — `{r['license']}` (`deps/{fname}`) — "
                     f"activates when {paths} lands")
        L.append("")
    if uncovered_prov:
        L.append("### Vendor provenance entries without a port-obligation record")
        L.append("")
        L.append("From `appname-proto/vendor/*/PROVENANCE.yaml`; a `deps/` record")
        L.append("(port-obligation for code, data-package for data) is required")
        L.append("before any of these lands in this repo.")
        L.append("")
        for r in uncovered_prov:
            L.append(f"- **{r.get('key')}** — `{r.get('license_spdx_as_read')}` — upstream {r.get('upstream')} @ `{str(r.get('commit'))[:12]}` (verdict: {r.get('verdict')})")
        L.append("")
    L.append("## Vendored asset components (vendored-asset records)")
    L.append("")
    L.append("Non-cargo assets copied verbatim into this repository at a pinned")
    L.append("upstream commit (never a cargo dependency; cargo-deny never walks")
    L.append("them). Driven by `deps/*.yaml` records with `recordKind:")
    L.append("vendored-assets`; activation is landing-detected. Once the vendor")
    L.append("drop lands, the co-located license copy named by the record is")
    L.append("re-read at every generation and sha256-verified against the pin")
    L.append("recorded from a first-hand raw-byte read of the upstream text at")
    L.append("the pinned commit — a missing copy or drifted bytes fails")
    L.append("generation and the gate's notices step.")
    L.append("")
    if active_assets:
        L.append("### Active (assets landed in this repo)")
        L.append("")
        for fname, r, landed, body in active_assets:
            tag = intern(body, f"{r['key']} ({r['licenseTextRepoPath']}, "
                               f"vendored-asset record pinned)")
            L.append(f"#### {r.get('name', r['key'])}  (`deps/{fname}`)")
            L.append(f"- license (as recorded): `{r['license']}` — full text [{tag}] "
                     f"— sha256 verified `{r['licenseTextSha256'][:16]}…`")
            for c in r["copyright"]:
                L.append(f"- {c}")
            L.append(f"- upstream: {r.get('upstream')} @ `{str(r.get('commit'))[:12]}`")
            L.append("- vendored at: " + ", ".join(f"`{p}`" for p in landed)
                     + f" (license copy `{r['licenseTextRepoPath']}`)")
            for s in r.get("noticeSurfaces") or []:
                L.append(f"- additional surface: {s}")
            L.append("")
    if pending_assets:
        L.append("### Pending (recorded and pinned; vendor drop not landed yet)")
        L.append("")
        for fname, r in pending_assets:
            paths = ", ".join(f"`{p}`" for p in r["landingPaths"])
            L.append(f"- **{r['key']}** — `{r['license']}` (`deps/{fname}`) — "
                     f"activates when {paths} lands (with the license copy at "
                     f"`{r['licenseTextRepoPath']}`)")
        L.append("")
    if data_recs:
        L.append("## Data package components (data-package records)")
        L.append("")
        L.append("Clearly-separated, user-replaceable DATA shipped with the application")
        L.append("(never code; never a cargo dependency — cargo-deny never walks it).")
        L.append("Driven by `deps/*.yaml` records with `recordKind: data-package`")
        L.append("(`appname.data-package.v1`); activation is landing-detected on the")
        L.append("staged data paths — the licensed data itself, never its own-code")
        L.append("consumer. Once staged, the co-located license copy is re-read and")
        L.append("sha256-verified at every generation, and the ENTIRE staged payload")
        L.append("is re-hashed and verified against the record's manifest pin — a")
        L.append("modification of shipped data without a record amendment in the same")
        L.append("commit fails generation and the gate's notices step (the")
        L.append("share-alike workflow tripwire).")
        L.append("")
    if active_data:
        L.append("### Active (data staged in this repo)")
        L.append("")
        for fname, r, landed, body, count in active_data:
            tag = intern(body, f"{r['key']} ({r['licenseTextRepoPath']}, "
                               f"data-package record pinned)")
            L.append(f"#### {r.get('name', r['key'])}  (`deps/{fname}`)")
            L.append(f"- license (as recorded): `{r['license']}` — full text [{tag}] "
                     f"— sha256 verified `{r['licenseTextSha256'][:16]}…`")
            for c in r["copyright"]:
                L.append(f"- {c}")
            L.append(f"- upstream: {r.get('upstream')} @ `{str(r.get('commit'))[:12]}`"
                     f" (provenance key `{r['key']}`)")
            L.append("- staged at: " + ", ".join(f"`{p}`" for p in landed)
                     + f" (license copy `{r['licenseTextRepoPath']}`)")
            L.append(f"- payload manifest sha256 verified "
                     f"`{r['payloadSha256'][:16]}…` ({count} files)")
            if r.get("basis"):
                L.append(f"- basis: {str(r['basis']).strip()}")
            if r.get("shareAlikeWorkflow"):
                L.append(f"- share-alike workflow (rides this entry): "
                         f"{str(r['shareAlikeWorkflow']).strip()}")
            for c in r.get("consumers") or []:
                L.append(f"- consumed by: {c}")
            for s in r.get("noticeSurfaces") or []:
                L.append(f"- additional surface: {s}")
            L.append("")
    if pending_data:
        L.append("### Pending (recorded and pinned; data not staged yet)")
        L.append("")
        for fname, r in pending_data:
            paths = ", ".join(f"`{p}`" for p in r["dataPaths"])
            L.append(f"- **{r['key']}** — `{r['license']}` (`deps/{fname}`) — "
                     f"activates when {paths} lands (with the license copy at "
                     f"`{r['licenseTextRepoPath']}`)")
        L.append("")
    L.append("## Appendix: license texts (deduplicated)")
    L.append("")
    for h in order:
        t = texts[h]
        L.append(f"### [{t['tag']}] sha256 {h[:16]}… (first seen: {t['origin']})")
        L.append("")
        # A bundled component's verbatim carriage text may print its OWN SPDX tag
        # (pcre2's LICENCE.md does). It states the component's terms, not this
        # file's, so REUSE must not read it as a declaration of THIRD-PARTY-NOTICES.md.
        fence = t.get("carriage") and "SPDX-License-Identifier" in t["body"]
        if fence:
            L.append("<!-- REUSE-IgnoreStart -->")
        L.append("```text")
        L.append(t["body"].rstrip("\n"))
        L.append("```")
        if fence:
            L.append("<!-- REUSE-IgnoreEnd -->")
        L.append("")
    content = "\n".join(L) + "\n"

    stats = (f"crates(shipped)={len(crate_rows)} tools/dev={len(dev_names)} "
             f"system={len(system_rows)} toolrecs={len(toolrec_rows)} "
             f"vendored-texts(active)={sum(len(a[2]) for a in vend_active)} "
             f"vendored-texts(pending)={len(vend_pending)} "
             f"active-ports={len(active_ports)} pending-ports={len(pending_ports)} "
             f"asset-records(active)={len(active_assets)} "
             f"asset-records(pending)={len(pending_assets)} "
             f"data-packages(active)={len(active_data)} "
             f"data-packages(pending)={len(pending_data)} "
             f"uncovered-provenance={len(uncovered_prov)} unique-texts={len(order)} "
             f"no-text-crates={len(missing_text)}")

    if check:
        try:
            committed = open(OUT, encoding="utf-8").read()
        except FileNotFoundError:
            print("NOTICES-DRIFT: THIRD-PARTY-NOTICES.md missing", file=sys.stderr)
            sys.exit(1)
        if committed != content:
            print("NOTICES-DRIFT: regenerated content differs from committed file "
                  "(run tools/gen_notices.py and commit)", file=sys.stderr)
            sys.exit(1)
        print(f"gen_notices --check: OK ({stats})")
    else:
        with open(out_path, "w", encoding="utf-8") as fh:
            fh.write(content)
        print(f"gen_notices: wrote {os.path.relpath(out_path, ROOT)} ({stats})")


if __name__ == "__main__":
    main()
# REUSE-IgnoreEnd
