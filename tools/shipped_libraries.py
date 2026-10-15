#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
"""Enumerate every Mach-O binary in a built macOS bundle — the POPULATION OF RECORD.

B-10 / OB-10, and the reason this exists in one sentence: three instruments
claimed to describe "the third-party libraries this artifact ships" and reported
19, 9 and 63, because each was built to answer a different question and none was
built to answer this one.

    19  the unrecorded count from the notices side (deps/ coverage gap)
     9  cargo-cyclonedx's SBOM naming (the CARGO graph; see sbom-README.md)
    63  a hand-built TSV covering Contents/Frameworks at depth 1

None of the three walked the artifact. This does: it opens every file in the
bundle, keeps the ones that are Mach-O by MAGIC NUMBER rather than by extension,
and reports each with the deps/ record that covers it, if any.

Two rules this tool exists to hold:

  * ENUMERATE THE ARTIFACT, NOT THE TREE THAT BUILT IT. A dependency graph
    describes what was compiled; macdeployqt copies in whatever Qt's link graph
    reaches. Only the bundle knows what shipped.
  * COVERAGE IS DECLARED, NEVER INFERRED FROM A NAME MATCH. Name matching got
    the count wrong four times in three days in BOTH directions: libsharpyuv
    looked unrecorded and ships with the webp formula; libharfbuzz looked
    recorded and the record describes a VENDORED 8.4.0 while the artifact ships
    Homebrew's 12.2.0. A substring match cannot tell you which build of which
    project is in the bundle, so "covered" means a person wrote it down in the
    manifest and a reviewer read it.

Usage:
    tools/shipped_libraries.py --app target/dmg-stage/Appname.app
                               [--manifest packaging/macos/shipped-libraries.tsv]
                               [--deps deps] [--out FILE] [--check]

--check exits non-zero when a shipped third-party Mach-O has no manifest entry,
or when a manifest entry names a deps/ record that does not exist. Both legs are
needed: a manifest pointing at nothing would otherwise pass while asserting
nothing.
"""

import hashlib
import os
import sys

MACHO_MAGICS = {
    b"\xfe\xed\xfa\xce", b"\xce\xfa\xed\xfe",   # 32-bit, both endiannesses
    b"\xfe\xed\xfa\xcf", b"\xcf\xfa\xed\xfe",   # 64-bit
    b"\xca\xfe\xba\xbe", b"\xbe\xba\xfe\xca",   # universal ("fat")
}


def is_macho(path):
    """By MAGIC, never by extension: Qt framework binaries and the Qt plugins
    carry no suffix at all, and a `.dylib` name proves nothing on its own."""
    try:
        with open(path, "rb") as fh:
            return fh.read(4) in MACHO_MAGICS
    except OSError:
        return False


def owner(rel):
    """Ours or third-party. Ours is covered by the project's own LICENSE and
    needs no third-party record; everything else must be declared."""
    base = os.path.basename(rel)
    if base.startswith("libappname_") or base.startswith("appname"):
        return "appname"
    return "third-party"


def bucket(rel):
    # `rel` is relative to the .app, so it has NO leading slash: a "/Contents/…"
    # test silently never fires and every executable falls through to "dylib".
    # Caught by reconciling this tool's own kind counts against the 34/45/82 the
    # bundle was known to hold — discipline (c).
    parts = rel.split("/")
    if parts[:2] == ["Contents", "MacOS"]:
        return "executable"
    if any(p.endswith(".framework") for p in parts):
        return "framework"
    if "PlugIns" in parts or "plugins" in parts:
        return "plugin"
    return "dylib"


def project_key(rel):
    """The unit a licence attaches to. A framework ships many files but is ONE
    project; a plugin directory likewise. Collapsing to the project is what
    makes the count answerable — 45 Qt framework binaries are not 45 projects."""
    parts = rel.split("/")
    for i, p in enumerate(parts):
        if p.endswith(".framework"):
            return p
    return os.path.basename(rel)


def load_manifest(path):
    """basename-or-framework -> (record, note). Tab-separated, # comments."""
    entries = {}
    if not path or not os.path.exists(path):
        return entries, False
    with open(path, encoding="utf-8") as fh:
        for line in fh:
            line = line.rstrip("\n")
            if not line.strip() or line.lstrip().startswith("#"):
                continue
            cols = line.split("\t")
            key = cols[0].strip()
            rec = cols[1].strip() if len(cols) > 1 else ""
            note = cols[2].strip() if len(cols) > 2 else ""
            if key:
                entries[key] = (rec, note)
    return entries, True


def bundle_identity(app):
    """A cheap identity of the whole bundle: (relpath, size, mtime_ns) for every
    file, hashed. Sampled BEFORE and AFTER the walk (OB-30).

    Why both ends. O-INT's artifact sweep read 77 binaries where this tool reads
    163, because the executable's mtime moved between two of their own
    observations while O-BUILD rebuilt underneath them; they filed it VOID rather
    than as a disagreement. O-WATCH then named the precise gap: an OPENING bracket
    alone "detects a subject that was already moving, not one that starts moving
    under the measurement." A census that cannot tell whether its subject moved is
    a photograph of a moving thing with the shutter speed unrecorded.

    Cheap on purpose: stat only, no content hashing. 2,006 files, and the walk it
    guards already stats every one of them.
    """
    h = hashlib.sha256()
    n = 0
    for root, dirs, files in os.walk(app):
        dirs.sort()
        for name in sorted(files):
            full = os.path.join(root, name)
            try:
                st = os.lstat(full)
            except OSError:
                continue
            h.update(os.path.relpath(full, app).encode("utf-8", "replace"))
            h.update(b"\0%d\0%d\0" % (st.st_size, st.st_mtime_ns))
            n += 1
    return h.hexdigest(), n


def build_in_progress(app):
    """OB-29's marker, read as the UP-FRONT half of the bracket: refuse to census a
    bundle whose build is still running. RUNNING with a dead pid means the build
    died -- resolvable, and still not a settled artifact."""
    state = os.path.join(os.path.dirname(os.path.dirname(app.rstrip("/"))), "dmg-logs", "build-state")
    try:
        line = open(state, encoding="utf-8").read().strip()
    except OSError:
        return None
    if not line.startswith("RUNNING"):
        return None
    pid = ""
    for tok in line.split():
        if tok.startswith("pid="):
            pid = tok[4:]
    alive = False
    if pid.isdigit():
        try:
            os.kill(int(pid), 0)
            alive = True
        except OSError:
            alive = False
    return f"{line}  (pid {'ALIVE' if alive else 'DEAD'})"


def arg(flag, default=None):
    if flag in sys.argv:
        i = sys.argv.index(flag)
        if i + 1 >= len(sys.argv):
            sys.exit(f"shipped_libraries: {flag} wants a value")
        return sys.argv[i + 1]
    return default


def main():
    app = arg("--app")
    if not app:
        sys.exit(__doc__.strip().splitlines()[0] +
                 "\nshipped_libraries: --app is required (path to a built .app)")
    if not os.path.isdir(app):
        sys.exit(f"shipped_libraries: no bundle at {app} — build one first "
                 f"(scripts/build_dmg.sh stages it at target/dmg-stage/Appname.app)")
    manifest_path = arg("--manifest", "packaging/macos/shipped-libraries.tsv")
    deps_dir = arg("--deps", "deps")
    check = "--check" in sys.argv

    manifest, manifest_found = load_manifest(manifest_path)

    # OB-30, the up-front half: never census a bundle that is still being written.
    #
    # OB-37's FULL remedy, per the operator's WORD 7 (2026-09-13 09:33 ET). The
    # minimal fix -- an unconditional --allow-in-flight -- was refused, and rightly:
    # a flag any caller may pass turns OB-30's guard off for everyone who reads the
    # manual. THE WRITER IS EXEMPT, NOBODY ELSE IS, and the census proves the caller
    # IS the writer rather than taking its word: --caller=build_dmg must arrive with
    # --allow-in-flight, and the pid in the state file must match the build's own,
    # which build_dmg.sh exports as APPNAME_BUILD_PID. Every other caller still meets
    # the refusal (OB-21's rule stands).
    inflight = build_in_progress(app)
    if inflight:
        # WORD 7 specifies the EQUALS form, `--caller=build_dmg`. arg() reads the
        # space-separated form only, so the first scratch run refused the writer
        # itself with "no --caller=build_dmg" -- the flag was there and unparsed.
        # Both forms are accepted now; the test that caught it is kept below.
        caller = arg("--caller")
        if caller is None:
            for a in sys.argv:
                if a.startswith("--caller="):
                    caller = a.split("=", 1)[1]
        claimed = os.environ.get("APPNAME_BUILD_PID", "")
        state_pid = ""
        for tok in inflight.split():
            if tok.startswith("pid="):
                state_pid = tok[4:]
        writer = ("--allow-in-flight" in sys.argv and caller == "build_dmg"
                  and claimed and claimed == state_pid)
        if not writer:
            why = "no --caller=build_dmg" if caller != "build_dmg" else (
                  "APPNAME_BUILD_PID unset" if not claimed else
                  f"APPNAME_BUILD_PID={claimed} does not match the state file's pid={state_pid}")
            sys.exit(f"shipped_libraries: REFUSING to census a bundle whose build is in "
                     f"progress — {inflight}. A census of a tree being written describes a "
                     f"state that never existed whole. The writing build may read its own "
                     f"output (--caller=build_dmg --allow-in-flight, pid-verified); this "
                     f"caller is not it ({why}).")

    ident_before, n_before = bundle_identity(app)

    rows = []
    for root, _dirs, files in os.walk(app):
        for name in files:
            full = os.path.join(root, name)
            if os.path.islink(full) or not is_macho(full):
                continue
            rel = os.path.relpath(full, app)
            rows.append(rel)
    rows.sort()

    # NON-VACUITY. A bundle with no Mach-O in it is not a clean bundle, it is a
    # failed walk — the same arm OB-21 put on the walled-symbol scan.
    if not rows:
        sys.exit(f"shipped_libraries: found NO Mach-O files under {app}. That is a "
                 f"broken walk, not an empty artifact — refusing to report clean.")

    out_lines = [
        "# GENERATED — tools/shipped_libraries.py; the POPULATION OF RECORD for B-10,",
        "# E-05's coverage report and F-06. Walked off the built bundle, never off a",
        "# dependency graph. Coverage is declared in the manifest, never inferred.",
        "#",
        "# path\towner\tkind\tproject\trecord\tnote",
    ]
    undeclared, badrec = [], []
    projects = {}
    for rel in rows:
        own = owner(rel)
        proj = project_key(rel)
        rec, note = "", ""
        if own == "third-party":
            rec, note = manifest.get(proj, manifest.get(os.path.basename(rel), ("", "")))
            if not rec:
                undeclared.append(rel)
            elif not os.path.exists(os.path.join(deps_dir, rec)):
                badrec.append(f"{proj}->{rec}")
        else:
            rec, note = "(ours)", "covered by the project's own LICENSE"
        projects.setdefault((own, proj), []).append(rel)
        out_lines.append(f"{rel}\t{own}\t{bucket(rel)}\t{proj}\t{rec}\t{note}")

    # OB-30, the closing half: if the subject moved under the walk, the result is
    # VOID. Refuse it rather than reporting a confident number taken across two
    # different artifacts.
    ident_after, n_after = bundle_identity(app)
    if ident_after != ident_before:
        sys.exit(f"shipped_libraries: CENSUS VOID — the bundle changed while it was "
                 f"being walked ({n_before} files before, {n_after} after; identity "
                 f"{ident_before[:12]} -> {ident_after[:12]}). The numbers this walk "
                 f"produced span two different artifacts and describe neither. Re-run "
                 f"once the build reports SETTLED.")

    third = [r for r in rows if owner(r) == "third-party"]
    ours = [r for r in rows if owner(r) == "appname"]
    third_projects = sorted({p for (o, p) in projects if o == "third-party"})

    # "identities", not "projects": 45 Qt framework binaries are ONE project, and
    # only the manifest knows that. Naming this count "projects" would be the
    # name-matching mistake this tool exists to refuse.
    summary = (f"shipped_libraries: {len(rows)} Mach-O files "
               f"({len(ours)} appname, {len(third)} third-party) in "
               f"{len(third_projects)} distinct third-party binary identities; "
               f"declared={len(third) - len(undeclared)} undeclared={len(undeclared)}; "
               f"manifest={'found' if manifest_found else 'ABSENT'} "
               f"({len(manifest)} entries); bad-record-refs={len(badrec)}")

    out = arg("--out")
    if out:
        with open(out, "w", encoding="utf-8") as fh:
            fh.write("\n".join(out_lines) + "\n")
    else:
        print("\n".join(out_lines))
    print(summary, file=sys.stderr)

    if check:
        bad = False
        if undeclared:
            print(f"SHIPPED-LIBRARY-COVERAGE: {len(undeclared)} shipped third-party "
                  f"Mach-O files have no manifest entry, so no recorded licence and "
                  f"no notice: {', '.join(sorted({project_key(r) for r in undeclared}))}",
                  file=sys.stderr)
            bad = True
        if badrec:
            print(f"SHIPPED-LIBRARY-COVERAGE: manifest entries naming a deps/ record "
                  f"that does not exist: {', '.join(sorted(set(badrec)))}", file=sys.stderr)
            bad = True
        if not manifest_found:
            print(f"SHIPPED-LIBRARY-COVERAGE: no manifest at {manifest_path}",
                  file=sys.stderr)
            bad = True
        if bad:
            sys.exit(1)
    return 0


if __name__ == "__main__":
    sys.exit(main())
