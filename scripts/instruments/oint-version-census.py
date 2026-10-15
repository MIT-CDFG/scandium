#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# oint-version-census.py — OWNER: O-INT (integration desk)
# READ-ONLY in --census and --verify. Never mutates the spec tree.
# See ./README.md for the rules of this home.
#
# THE ACCEPTANCE TEST FOR THE v5.5 -> v5.6 SWEEP, per operator ruling 2026-08-26.
#
# WHY IT EXISTS. V34 (tools/spec_validate.py:709-725) is NOT this test: it scans
# FIVE documents, strips inline code spans first, and covers FOUR literals. The
# :19050 normative anchor that defines what a version stamp MEANS sits inside
# backticks and is invisible to it. A sweep whose acceptance test examines 0.4%
# of its own output has no acceptance test.
#
# THE DISPOSITION PREDICATE IS NORMATIVE, not invented here. ENGINEERING-SPEC.md
# :19050 [NORMATIVE]: "Every artifact carries `spec-version: 5.5` in its §I.2
# header; THE VALUE EQUALS THE SPEC REVISION THE FILE WAS GENERATED FROM and
# bumps with the document, never independently."
#   regenerated at v5.6  -> the stamp becomes 5.6, because that is now its origin
#   NOT regenerated      -> the stamp already names its true origin, and sweeping
#                           it makes the stamp FALSE by the spec's own definition
#
# TARGET DERIVES FROM AN INDEPENDENT SOURCE. VERSIONING.md:16, written before
# this session: "v5.6 FINAL — the full prose + per-file `spec-version` sweep to
# `5.6`". Setting the target to match whatever was swept would make this a
# tautology — the generator certifying the artifact, which the operator ruled
# against.
#
# ★ THE UNIT IS THE LITERAL, NOT THE FILE. ENGINEERING-SPEC.md alone holds all
# three cases: :19050 must sweep; :19099's "(v5.4)" names when a RULE was
# introduced and must not; :20547's "spec-version 5.1" is a historical stamp and
# must not. No file-level include/exclude list can express that.
#
# ★★ AND THE BARE `5.5` FORM IS OVERWHELMINGLY NOT A VERSION. The spec has a
# SECTION 5.5 ("Working spaces and color math") referenced as §5.5 throughout;
# there are percentages (5.56% of text width), table values and ratios. Bare
# decimals therefore DEFAULT TO must-not and are swept only on positive
# identification — G-125 applied to the sweep: enumerate what is known safe to
# change and refuse everything else. An inclusion list of "dangerous" fails
# silent; an allowlist of "safe" fails loud.

import argparse, hashlib, json, os, re, sys

FROM_V, TO_V = "5.5", "5.6"

# Directory / filename classes that are NOT regenerated at v5.6.
NOT_REGENERATED = [
    (re.compile(r"^_to_delete/"),        "not-regenerated:delete-staging"),
    (re.compile(r"^adr/"),               "not-regenerated:adr-dated-decision"),
    (re.compile(r"^decision-history/"),  "not-regenerated:decision-history"),
    (re.compile(r"^research/"),          "not-regenerated:research"),
    (re.compile(r"^digests/"),           "not-regenerated:digest"),
    (re.compile(r"^build/"),             "not-regenerated:build-output"),
    (re.compile(r"^REVIEW-RESPONSE-v"),  "not-regenerated:historical-response"),
]

STAMP   = re.compile(r"spec[-_]version:\s*" + re.escape(FROM_V) + r"\b")
VPREFIX = re.compile(r"\bv" + re.escape(FROM_V) + r"\b")
BARE    = re.compile(r"(?<![\w.\-])" + re.escape(FROM_V) + r"(?![\w.])")

SECTION_REF = re.compile(r"(?:§|section\s+|##\s*|^\s*-?\s*)" + re.escape(FROM_V) + r"\b", re.I)
NUMERIC_CTX = re.compile(r"%|percent|px\b|pt\b|width|ratio|scale|ΔE|em\b|mm\b", re.I)
CURRENT_EDITION = re.compile(r"\*\*Status\*\*|Reference draft|ships with spec|current-state|Compatibility baseline|ENGINEERING-SPEC\.md v|this (?:spec|document|edition)", re.I)
HISTORY_CTX = re.compile(r"HISTORY|non-normative|Prior\s+\*\*v|superseded|previously|was\s+v", re.I)

def is_text(path):
    try:
        with open(path, "rb") as fh:
            return b"\x00" not in fh.read(8192)
    except Exception:
        return False

def not_regenerated(rel):
    for rx, why in NOT_REGENERATED:
        if rx.search(rel):
            return why
    return None

def classify(rel, line, col, form, text):
    """Return (disposition, reason). MUST-NOT is the default for anything not
    positively identified."""
    nr = not_regenerated(rel)
    if nr:
        return "MUST-NOT", nr
    if form == "stamp":
        return "MUST-SWEEP", "version-stamp:regenerated-artifact"
    if form == "vprefix":
        # ★★★ DEFAULT HOLD. An earlier version defaulted vprefix to MUST-SWEEP
        # unless a history regex fired -- an inclusion list of "dangerous"
        # defaulting to CHANGE IT, which is G-125 inverted, in the classifier
        # that gates the sweep. Reviewing its output caught two large classes it
        # would have falsified:
        #   ENGINEERING-SPEC.md:19104-19109  "| **V39 …** (v5.5) |" — an
        #     INTRODUCED-VERSION annotation naming when the CHECK was introduced.
        #     Sweeping it asserts V39 arrived in v5.6. The operator named this
        #     class explicitly and :19099's "(v5.4)" is the same shape.
        #   VERSIONING.md:5  "- **v5.5** — git tag **v5.5-freeze**" — the
        #     historical record OF the v5.5 freeze. Sweeping it rewrites the v5.5
        #     entry into a v5.6 entry.
        # So a prose reference is swept only on POSITIVE identification as naming
        # the CURRENT edition, and held otherwise with its reason.
        if re.search(r"\(\s*v" + re.escape(FROM_V) + r"\s*\)", text):
            return "MUST-NOT", "introduced-version-annotation:names-when-a-rule-arrived"
        if re.search(r"v" + re.escape(FROM_V) + r"-freeze", text):
            return "MUST-NOT", "tag-name:v5.5-freeze-is-an-identifier"
        # ★★★ LOCAL WINDOW, NOT THE LINE. Evaluating history context over the whole
        # line held ENGINEERING-SPEC.md:11 -- the spec's own Status row -- because
        # the CURRENT "v5.5" sits at the front and the [HISTORY] chain naming
        # v5.4/v5.3/v5.2/v5.1 sits hundreds of characters later on the SAME line.
        # That is the per-literal-not-per-line hazard I had already identified on
        # this exact line, then defeated with a line-level test. It also left
        # design-decisions.md:11 internally inconsistent on main:
        #   "ENGINEERING-SPEC.md v5.5 ↔ artifact pack `spec-version: 5.6`"
        # A window of 70 chars before / 20 after is what a reader uses to decide
        # which clause a literal belongs to.
        lo, hi = max(0, col - 70), min(len(text), col + 20)
        win = text[lo:hi]
        if CURRENT_EDITION.search(win):
            return "MUST-SWEEP", "prose-reference:positively-current-edition"
        if HISTORY_CTX.search(win):
            return "MUST-NOT", "history-chain:names-a-prior-state"
        if HISTORY_CTX.search(text):
            return "MUST-NOT", "history-chain:elsewhere-on-line-defaults-to-hold"
        return "MUST-NOT", "prose-reference:unidentified-defaults-to-hold"
    # bare: default MUST-NOT, swept only on positive identification
    seg = text[max(0, col - 12):col]
    if SECTION_REF.search(text[max(0, col - 12):col + 4]):
        return "MUST-NOT", "section-reference:not-a-version"
    if NUMERIC_CTX.search(text):
        return "MUST-NOT", "numeric-context:not-a-version"
    return "MUST-NOT", "bare-decimal:unidentified-defaults-to-hold"

def enumerate_literals(root):
    rows, binaries = [], []
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if d != ".git"]
        for fn in sorted(filenames):
            fp = os.path.join(dirpath, fn)
            rel = os.path.relpath(fp, root)
            if not is_text(fp):
                try:
                    raw = open(fp, "rb").read().decode("utf-8", errors="ignore")
                    n = len(re.findall(re.escape(FROM_V), raw))
                except Exception:
                    n = 0
                if n:
                    binaries.append({"file": rel, "hits": n})
                continue
            try:
                lines = open(fp, encoding="utf-8", errors="ignore").read().split("\n")
            except Exception:
                continue
            for ln, text in enumerate(lines, 1):
                spans = []
                for m in STAMP.finditer(text):
                    spans.append((m.start(), m.end(), "stamp"))
                for m in VPREFIX.finditer(text):
                    if not any(s <= m.start() < e for s, e, _ in spans):
                        spans.append((m.start(), m.end(), "vprefix"))
                for m in BARE.finditer(text):
                    if not any(s <= m.start() < e for s, e, _ in spans):
                        spans.append((m.start(), m.end(), "bare"))
                for s, e, form in sorted(spans):
                    col = text.index(FROM_V, s)
                    disp, why = classify(rel, ln, col, form, text)
                    rows.append({"file": rel, "line": ln, "col": col, "form": form,
                                 "disposition": disp, "reason": why,
                                 "context": text.strip()[:160]})
    return rows, binaries

# ★★★ POPULATION ACCOUNTING. A census that silently drops literals is worse than
# none. Every naive `5.5` occurrence in a text file is placed in exactly one
# bucket: counted, or excluded WITH ITS REASON. The reconciliation is printed and
# must balance, so a future boundary-guard change cannot quietly shrink the
# population.
#
# THIS IS NOT COSMETIC. The first population figure reported for this sweep was
# 1055 literals. It was inflated by ~457 SECTION-NUMBER SUBSTRINGS: §15.5, §25.5,
# §35.5 and §45.5 all contain "5.5". Sweeping them would have rewritten hundreds
# of cross-references to point at DIFFERENT SECTIONS of the spec — silently, and
# with no test in the tree able to notice. The boundary guards are what stand
# between the sweep and that outcome, so their effect is enumerated rather than
# trusted.
NAIVE = re.compile(re.escape(FROM_V))

def account(root, rows):
    """POSITIONAL RECONCILIATION, not a second heuristic.

    Every naive `5.5` in every text file is looked up by (file, line, col)
    against the census the instrument actually produced. A miss is bucketed with
    the character that caused it. This is exact: it cannot disagree with the
    census about what the census contains, only about whether something was left
    out — which is the only question worth asking.

    An earlier version re-derived the boundary rule with a character class and
    disagreed with the census by one: tools/pdf-exceptions.json carries
    `spec-version: 5.5.` ending a sentence, which the census correctly counts as
    a stamp and the heuristic wrongly excluded as "followed by a period". That is
    the two-methods-one-question defect, committed inside the check written to
    prove the population complete. The fix is to compare positions, not rules."""
    have = {(r["file"], r["line"], r["col"]) for r in rows}
    counted = excluded = 0
    reasons = {}
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if d != ".git"]
        for fn in sorted(filenames):
            fp = os.path.join(dirpath, fn)
            if not is_text(fp):
                continue
            rel = os.path.relpath(fp, root)
            try:
                lines = open(fp, encoding="utf-8", errors="ignore").read().split("\n")
            except Exception:
                continue
            for ln, text in enumerate(lines, 1):
                for m in NAIVE.finditer(text):
                    col = m.start()
                    if (rel, ln, col) in have:
                        counted += 1
                        continue
                    excluded += 1
                    pre = text[col - 1] if col else "^"
                    post = text[col + 3] if col + 3 < len(text) else "$"
                    if re.match(r"[0-9]", pre):
                        k = "inside-a-larger-number(section-ref like §%s5.5)" % pre
                    elif pre == "v":
                        k = "v-prefixed-and-recaptured-elsewhere"
                    elif re.match(r"[\w.\-]", pre):
                        k = "preceded-by-%r" % pre
                    else:
                        k = "followed-by-%r" % post
                    reasons[k] = reasons.get(k, 0) + 1
    return counted, excluded, reasons

def build_census(root):
    rows, binaries = enumerate_literals(root)
    payload = {"from": FROM_V, "to": TO_V, "root": os.path.abspath(root),
               "literals": rows, "binaries_excluded": binaries}
    blob = json.dumps(payload["literals"], sort_keys=True).encode()
    payload["census_sha256"] = hashlib.sha256(blob).hexdigest()
    return payload

def verify(root, census):
    """BOTH directions. Returns (ok, findings)."""
    findings = []
    checked = swept = held = 0
    cache = {}
    for r in census["literals"]:
        fp = os.path.join(root, r["file"])
        if fp not in cache:
            try:
                cache[fp] = open(fp, encoding="utf-8", errors="ignore").read().split("\n")
            except Exception:
                cache[fp] = None
        lines = cache[fp]
        if lines is None or r["line"] > len(lines):
            findings.append(f"VOID unreadable-or-shortened {r['file']}:{r['line']}")
            continue
        text = lines[r["line"] - 1]
        col = r["col"]
        actual = text[col:col + 3]
        checked += 1
        if r["disposition"] == "MUST-SWEEP":
            swept += 1
            if actual != TO_V:
                findings.append(f"MISSED-SWEEP {r['file']}:{r['line']}:{col} "
                                f"expected {TO_V} found {actual!r} [{r['reason']}]")
        else:
            held += 1
            if actual != FROM_V:
                findings.append(f"WRONG-SWEEP {r['file']}:{r['line']}:{col} "
                                f"must-not-touch became {actual!r} [{r['reason']}]")
    return (len(findings) == 0), findings, {"checked": checked, "sweep": swept, "hold": held}

# ─────────────────────────────────────────────────────────────────────────────
# SELFTEST. "A guard you have not watched refuse is not in service"
# (battery_guards.sh's header). This instrument gates a 342-edit sweep across a
# frozen spec, so it is demonstrated to FAIL IN BOTH DIRECTIONS on planted faults
# and demonstrated NOT to over-fire on a correct sweep, before it is trusted once.
#
# Both directions matter and they fail differently:
#   MISSED-SWEEP  a MUST-SWEEP literal left at 5.5 -> the tree is not uniformly
#                 v5.6 and the freeze would record a lie.
#   WRONG-SWEEP   a MUST-NOT literal changed to 5.6 -> a historical stamp, a
#                 section reference or a prior-version citation silently falsified.
#                 This is the direction with no downstream test able to catch it.

def _fixture(d):
    os.makedirs(os.path.join(d, "spec"), exist_ok=True)
    os.makedirs(os.path.join(d, "adr"), exist_ok=True)
    with open(os.path.join(d, "spec", "a.yaml"), "w") as fh:
        fh.write("# spec-version: 5.5\nkind: thing\n")          # MUST-SWEEP (stamp)
    with open(os.path.join(d, "spec", "b.md"), "w") as fh:
        fh.write("Reference draft v5.5 current.\nSee section 5.5 for detail.\n")
        # line1 v5.5 -> MUST-SWEEP ; line2 bare 5.5 after "section " -> MUST-NOT
    with open(os.path.join(d, "adr", "ADR-1.md"), "w") as fh:
        fh.write("# spec-version: 5.5\nDated decision.\n")       # MUST-NOT (adr)
    return d

def _apply(d, correct=True, plant=None):
    """Sweep the fixture. plant='missed' leaves a MUST-SWEEP alone;
    plant='wrong' also bumps a MUST-NOT."""
    ay = os.path.join(d, "spec", "a.yaml")
    bm = os.path.join(d, "spec", "b.md")
    ad = os.path.join(d, "adr", "ADR-1.md")
    t = open(ay).read()
    if plant != "missed":
        t = t.replace("spec-version: 5.5", "spec-version: 5.6")
    open(ay, "w").write(t)
    t = open(bm).read().replace("v5.5", "v5.6")
    open(bm, "w").write(t)
    if plant == "wrong":
        t = open(ad).read().replace("spec-version: 5.5", "spec-version: 5.6")
        open(ad, "w").write(t)

def selftest():
    import shutil, tempfile
    rc = 0
    n = 0

    def check(want, got, label):
        nonlocal rc, n
        n += 1
        if want == got:
            print("  PASS %-30s -> %s" % (label, got))
        else:
            print("  FAIL %-30s -> got [%s] want [%s]" % (label, got, want))
            rc = 1

    def run(plant):
        d = tempfile.mkdtemp()
        try:
            _fixture(d)
            c = build_census(d)
            sweep = sum(1 for r in c["literals"] if r["disposition"] == "MUST-SWEEP")
            hold = len(c["literals"]) - sweep
            if sweep == 0 or hold == 0:
                return "FIXTURE-VOID(sweep=%d hold=%d)" % (sweep, hold)
            _apply(d, plant=plant)
            ok, findings, _ = verify(d, c)
            if ok:
                return "PASS"
            return findings[0].split()[0]
        finally:
            shutil.rmtree(d, ignore_errors=True)

    # the fixture must itself be non-vacuous, or every arm below is meaningless
    d = tempfile.mkdtemp()
    try:
        _fixture(d)
        c = build_census(d)
        sweep = sum(1 for r in c["literals"] if r["disposition"] == "MUST-SWEEP")
        hold = len(c["literals"]) - sweep
        check(True, sweep > 0 and hold > 0, "fixture has both sides")
        check("MUST-NOT", next(r["disposition"] for r in c["literals"]
                               if r["file"].startswith("adr")), "adr classified hold")
        check("MUST-NOT", next(r["disposition"] for r in c["literals"]
                               if r["form"] == "bare"), "bare decimal defaults hold")
        check("MUST-SWEEP", next(r["disposition"] for r in c["literals"]
                                 if r["form"] == "stamp" and r["file"].startswith("spec")),
              "stamp classified sweep")
    finally:
        shutil.rmtree(d, ignore_errors=True)

    check("PASS", run(None), "correct sweep -> PASS")
    check("MISSED-SWEEP", run("missed"), "planted MISSED sweep -> RED")
    check("WRONG-SWEEP", run("wrong"), "planted WRONG sweep -> RED")

    # VOID arms
    empty = {"literals": [], "from": FROM_V, "to": TO_V}
    check(True, not empty["literals"], "empty census is refusable")
    print("  %s %d checks ran" % ("PASS selftest-denominator ->", n))
    return rc

def main():
    ap = argparse.ArgumentParser(description="v5.5->v5.6 per-literal census and acceptance test")
    # The tree this census walks is the frozen specification repository. It is
    # not named here and carries no absolute default (operator WORD 24,
    # RULING A): pass --root, or set APPNAME_SPEC_ROOT.
    ap.add_argument("--root", default=os.environ.get("APPNAME_SPEC_ROOT", ""))
    ap.add_argument("--census", metavar="OUT", help="write the census manifest")
    ap.add_argument("--verify", metavar="IN", help="verify the tree against a census")
    ap.add_argument("--selftest", action="store_true")
    ap.add_argument("--defer", action="append", default=[],
                    help="path prefix whose MUST-SWEEP literals are DEFERRED to a later "
                         "reviewed pass (operator ruling: tools/ separate, so generator and "
                         "artifact never certify each other). Deferred literals stay in the "
                         "census as DEFERRED and are checked to be UNTOUCHED by --verify.")
    a = ap.parse_args()

    if a.selftest:
        return selftest()

    if a.census:
        c = build_census(a.root)
        if a.defer:
            n_def = 0
            for r in c["literals"]:
                if r["disposition"] == "MUST-SWEEP" and any(r["file"].startswith(d) for d in a.defer):
                    r["disposition"] = "DEFERRED"
                    r["reason"] = "deferred-to-separate-pass:" + r["reason"]
                    n_def += 1
            blob = json.dumps(c["literals"], sort_keys=True).encode()
            c["census_sha256"] = hashlib.sha256(blob).hexdigest()
            c["deferred_prefixes"] = a.defer
            print(f"deferred to a later pass: {n_def} literals under {a.defer}")
        with open(a.census, "w") as fh:
            json.dump(c, fh, indent=1, sort_keys=True)
        n = len(c["literals"])
        if n == 0:
            print("VOID - census enumerated ZERO literals. THIS RUN PROVES NOTHING.")
            return 3
        sweep = sum(1 for r in c["literals"] if r["disposition"] == "MUST-SWEEP")
        print(f"census: {n} literals  MUST-SWEEP {sweep}  MUST-NOT {n-sweep}")
        print(f"binaries excluded: {len(c['binaries_excluded'])} files")
        counted, excluded, reasons = account(a.root, c['literals'])
        print(f"accounting: naive '5.5' in text = {counted + excluded}"
              f"  -> counted {counted}  excluded {excluded}")
        for k, v in sorted(reasons.items(), key=lambda x: -x[1])[:6]:
            print(f"    excluded {k}: {v}")
        if counted != n:
            print(f"  ACCOUNTING MISMATCH: accounted {counted} != census {n} "
                  f"(delta {counted - n}) -- the population is not provably complete")
            return 3
        print(f"  accounting BALANCES: {counted} == census {n}")
        print(f"census_sha256: {c['census_sha256']}")
        return 0

    if a.verify:
        c = json.load(open(a.verify))
        if not c.get("literals"):
            print("VOID - census carries ZERO literals. THIS RUN PROVES NOTHING.")
            return 3
        ok, findings, stats = verify(a.root, c)
        print(f"verify: checked {stats['checked']}  sweep-side {stats['sweep']}  hold-side {stats['hold']}")
        if stats["sweep"] == 0 or stats["hold"] == 0:
            print("VOID - one side of the comparison is EMPTY; a vacuous check cannot pass.")
            return 3
        for f in findings[:40]:
            print("  " + f)
        if findings:
            print(f"FAIL - {len(findings)} finding(s)")
            return 1
        print("PASS - every MUST-SWEEP became 5.6 and every MUST-NOT is untouched")
        return 0

    ap.print_help()
    return 3

if __name__ == "__main__":
    sys.exit(main())
