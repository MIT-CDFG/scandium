#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# run_ks.py — the KS harness (W09 / §49.1 keyboard-scripts), O-QA fence.
# Spec: ES Part 49 §49.1 (appname.keyboardScripts.v1); §31.3 live-region
# policy; the A11Y-5 KS-junction contract frozen in reports/O-UI-log.md.
#
# WHAT THIS HARNESS DOES (and does not) — recorded honestly per preamble
# rule 9. The frozen v1.0.0-beta shell (commit 904c369) exposes exactly
# two entry points (`--lang`, `--screenshot-tour`); it lands the A11Y-5
# HOOKS (A: the APPNAME_A11Y_LOG JSONL sink; B: the F10/Ctrl+F2 menu
# access with real-Space arming; C: the tst_focus_probes scenes) but NO
# external KS-script-runner door — there is no in-process key-injection
# entry for driving an arbitrary KS-1..8 step sequence from outside the
# process. This harness therefore:
#   1. LIVE-EXERCISES hook A on the candidate binary: runs the shell's
#      one drivable announcing surface (--screenshot-tour, offscreen)
#      under APPNAME_A11Y_LOG and captures the real JSONL sink.
#   2. Validates the SINK CONTRACT mechanically against the frozen
#      A11Y-5 shape (one file/run; {source,id,text,ms}; ms
#      construction-relative + weakly-monotonic — never read absolute;
#      append-per-emit; JSON escaping round-trips).
#   3. Validates the ANNOUNCE-GOLDEN / D-43 LABEL CONTRACT: every KS
#      announceExpect id is resolved against the SHIPPED en.json; the
#      SHIPPED D-43 tool labels ("Rect Select"/"Text") are asserted
#      authoritative (O-INT ruling), and the frozen §49 goldens
#      ("Rectangular Marquee"/"Type" — Adobe trade dress) are recorded
#      as the parked QUESTIONS-FOR-BILL discrepancy, not a failure.
#   4. LIVE CROSS-CONFIRMS the cataloged announce ids the tour emits
#      (their live text matches the catalog template shape on the
#      candidate).
#   5. Emits the honest KS-1..8 PER-STEP LEDGER: each step is one of
#      ASSERTED-CONTRACT (announce/label contract verified against the
#      shipped catalog, live-confirmed where the tour emits the id — the
#      key sequence itself is NOT driven; see the structural note below) /
#      SKIP-UNSAT (KS-3 s2, unsatisfiable under the 3-pane mapping) /
#      SKIP-UNCATALOGED (announce id not yet minted in en.json — silent
#      by the has()-gate design) / STATE-ONLY (announceExpect null: no
#      sink-observable output by design).
#      STRUCTURAL GAP (reported to O-INT, NEVER a fabricated pass): the
#      frozen shell has no external KS-script-runner door, so no step's
#      key sequence is executed end-to-end by this harness.
#
# Exit 0 = the sink contract and the cataloged-label contract hold on
#          the candidate (typed-skips are recorded gaps, not failures).
# Exit 1 = a CONTRACT VIOLATION (malformed sink, monotonicity break,
#          absolute-clock leak, or a cataloged label that mismatches the
#          shipped catalog outside the ruled D-43 override).
# Exit 2 = environment not ready (no shell binary / no catalog).

import json
import os
import pathlib
import subprocess
import sys
import tempfile

import yaml

ROOT = pathlib.Path(__file__).resolve().parents[2]
KS_YAML = ROOT / "tests" / "a11y" / "keyboard-scripts.yaml"
EN_JSON = ROOT / "i18n" / "en.json"

# The KS contract is a MIRROR of the frozen bogos spec artifact
# (bogos/tests/a11y/keyboard-scripts.yaml). To satisfy the repo's SPDX
# gate (scripts/gate.sh step_spdx requires an SPDX + copyright line on
# every tracked .yaml outside contracts/) the mirror carries exactly the
# two-line comment header below; STRIPPING those two lines must
# reproduce the bogos source byte-for-byte. Drift teeth: the SHA256 of
# the header-stripped mirror content is pinned here, and — when the
# frozen spec tree is reachable — also compared live to the bogos bytes.
# A real edit to the mirror's contract content flips the sha and FAILS.
MIRROR_SPDX_HEADER = (
    # WORD 10 (4): split token -- the MIRROR carries the tag, this file only
    # describes it. Adjacent literals concatenate; the sha-compared bytes are
    # unchanged.
    "# SPDX-License" "-Identifier: GPL-3.0-only\n"
    "# Copyright (c) 2026 the Appname contributors\n"
)
# sha256 of the bogos spec artifact content (== mirror minus the 2 header
# lines). A spec re-mirror is a deliberate O-QA act that re-pins this.
EXPECTED_SPEC_SHA256 = (
    "e5b1e7edcfec816f53f04f2a1333c8e48d63061e4c8328d2a0d992f040f9a31e"
)
# The frozen specification tree is not derivable from this repository and is not
# named here (operator WORD 24, RULING A): point APPNAME_SPEC_ROOT at it to have
# the live-bytes half of the parity check run. Unset is not a failure — the
# sha256 pin above is the drift gate either way — but an EMPTY variable must not
# resolve to this tree, or the mirror would be compared with itself and always
# agree, which is the silent pass this guard exists to prevent.
_SPEC_ROOT = os.environ.get("APPNAME_SPEC_ROOT", "").strip()
BOGOS_SPEC = (
    pathlib.Path(_SPEC_ROOT, "tests", "a11y", "keyboard-scripts.yaml")
    if _SPEC_ROOT
    else None
)


def validate_mirror_parity(violations):
    """Assert the SPDX-headed mirror still equals the frozen bogos spec."""
    import hashlib
    checks = []
    raw = KS_YAML.read_bytes()
    lines = raw.split(b"\n")
    got_header = b"\n".join(lines[:2]) + b"\n"
    if got_header != MIRROR_SPDX_HEADER.encode():
        violations.append(
            "KS mirror header is not the expected 2-line SPDX block "
            f"(got {got_header!r})")
        return checks
    content = b"\n".join(lines[2:])  # mirror MINUS the 2 SPDX lines
    got_sha = hashlib.sha256(content).hexdigest()
    if got_sha != EXPECTED_SPEC_SHA256:
        violations.append(
            f"KS mirror content drift: sha256 {got_sha} != pinned "
            f"{EXPECTED_SPEC_SHA256}")
    else:
        checks.append(("mirror content sha256 == pinned spec sha", got_sha[:16] + "…"))
    if BOGOS_SPEC is not None and BOGOS_SPEC.exists():
        if content != BOGOS_SPEC.read_bytes():
            violations.append(
                "KS mirror (minus SPDX header) != live bogos spec bytes")
        else:
            checks.append(("mirror (minus header) == live bogos spec", "byte-identical"))
    else:
        checks.append(("live spec tree not reachable (APPNAME_SPEC_ROOT)",
                       "sha-pin is the drift gate"))
    return checks

# The frozen §49 tool goldens that O-INT ruled superseded by the shipped
# D-43 catalog labels; the discrepancy is a parked QUESTIONS-FOR-BILL item.
D43_OVERRIDES = {
    # KS step announceText (frozen §49) -> shipped catalog label id (D-43)
    "Rectangular Marquee": "ui.tool.select_rect",
    "Type": "ui.tool.text",
}

# KS steps whose announce leg is UNSATISFIABLE on the shipped surface
# (frozen contract, reports/O-UI-log.md A11Y-5 junction brief item 1).
UNSAT = {("KS-3", 2): "Shift+F6 x2 to a separate tool OPTIONS pane — "
                      "unsatisfiable under the shipped 3-pane mapping (§31.2 "
                      "5->3 collapse); parked QUESTIONS-FOR-BILL"}


def find_shell():
    for cand in (ROOT / "shell" / "build" / "appname-shell",
                 ROOT / "shell" / "build" / "Release" / "appname-shell"):
        if cand.is_file() and os.access(cand, os.X_OK):
            return cand
    return None


def run_live_sink(shell):
    """Run the candidate shell's announcing surface under the sink; return rows."""
    with tempfile.TemporaryDirectory() as td:
        sink = pathlib.Path(td) / "ks_sink.jsonl"
        tourdir = pathlib.Path(td) / "frames"
        tourdir.mkdir()
        env = dict(os.environ)
        env["APPNAME_A11Y_LOG"] = str(sink)
        env["APPNAME_I18N_DIR"] = str(ROOT / "i18n")
        proc = subprocess.run(
            [str(shell), "--screenshot-tour", str(tourdir)],
            env=env, capture_output=True, text=True, timeout=120)
        raw = sink.read_text() if sink.exists() else ""
    return proc.returncode, raw


def validate_sink(raw, violations):
    """Mechanical checks against the frozen A11Y-5 sink contract."""
    checks = []
    lines = [ln for ln in raw.splitlines() if ln.strip()]
    if not lines:
        violations.append("sink produced ZERO rows (hook A did not emit)")
        return checks, []
    rows = []
    for i, ln in enumerate(lines):
        try:
            obj = json.loads(ln)  # JSON escaping / round-trip check
        except json.JSONDecodeError as e:
            violations.append(f"sink row {i} is not valid JSON: {e}")
            continue
        rows.append(obj)
    checks.append(("rows parse as JSONL (escaping round-trips)", f"{len(rows)} rows"))

    # shape: exactly {id, ms, source, text}
    want = {"id", "ms", "source", "text"}
    bad = [i for i, r in enumerate(rows) if set(r) != want]
    if bad:
        violations.append(f"sink rows with wrong key-set: {bad} (want {sorted(want)})")
    else:
        checks.append(("every row shape == {id,ms,source,text}", "OK"))

    ms = [r["ms"] for r in rows if "ms" in r]
    # weakly monotonic (non-decreasing)
    mono = all(ms[i] <= ms[i + 1] for i in range(len(ms) - 1))
    if not mono:
        violations.append(f"ms NOT weakly-monotonic: {ms}")
    else:
        checks.append(("ms weakly-monotonic (non-decreasing)",
                       f"{ms[0]}..{ms[-1]}"))
    # construction-relative: first ms is a small offset, NOT a wall-clock
    # epoch (~1.7e12 ms). This asserts the harness treats ms as relative.
    if ms and ms[0] < 60_000:
        checks.append(("ms construction-relative (not absolute epoch)",
                       f"first ms={ms[0]}"))
    elif ms:
        violations.append(f"first ms={ms[0]} looks absolute, not construction-relative")
    return checks, rows


def render(template, **kw):
    out = template
    for k, v in kw.items():
        out = out.replace("{" + k + "}", str(v))
    return out


def main():
    if not KS_YAML.exists() or not EN_JSON.exists():
        print(f"KS harness: STAGED (missing {KS_YAML if not KS_YAML.exists() else EN_JSON})")
        return 2
    shell = find_shell()
    if shell is None:
        print("KS harness: STAGED (no candidate appname-shell binary under shell/build "
              "— build it: cmake --build shell/build)")
        return 2

    ks = yaml.safe_load(KS_YAML.read_text())
    catalog = json.load(open(EN_JSON))
    violations = []

    print("== 0. MIRROR PARITY (SPDX-headed mirror vs frozen bogos spec) ==")
    for name, detail in validate_mirror_parity(violations):
        print(f"   PASS  {name}: {detail}")

    print(f"\n== KS harness on the candidate shell: {shell.relative_to(ROOT)}")
    rc, raw = run_live_sink(shell)
    print(f"-- hook A live run (--screenshot-tour, offscreen): exit {rc}")

    print("\n== 1. SINK CONTRACT (frozen A11Y-5 shape, live rows) ==")
    checks, rows = validate_sink(raw, violations)
    for name, detail in checks:
        print(f"   PASS  {name}: {detail}")
    live_ids = {r["id"] for r in rows}

    print("\n== 2. LIVE CROSS-CONFIRM (cataloged ids the tour emitted render to shape) ==")
    for r in rows:
        tmpl = catalog.get(r["id"])
        if tmpl is None:
            violations.append(f"live row id {r['id']} has NO catalog entry")
            print(f"   VIOL  {r['id']}: uncataloged but emitted live")
            continue
        # structural: a templated field ({...}) means the live text is a
        # substitution of the catalog template; assert the fixed prefix.
        prefix = tmpl.split("{")[0]
        if prefix and not r["text"].startswith(prefix):
            violations.append(f"live text {r['text']!r} != template {tmpl!r}")
            print(f"   VIOL  {r['id']}: {r['text']!r} vs {tmpl!r}")
        else:
            print(f"   PASS  {r['id']}: {r['text']!r} ~ {tmpl!r}")

    print("\n== 3. KS-1..8 PER-STEP LEDGER (announce/label contract + drive disposition) ==")
    print("  NOTE: no step's KEY SEQUENCE is executed — the frozen shell has no external")
    print("        KS-script-runner door. ASSERTED-CONTRACT = the step's announce/label")
    print("        contract is verified against the shipped catalog (live-confirmed where")
    print("        the tour emits the id); the keyboard drive is the structural gap below.")
    tally = {}
    for script in ks["scripts"]:
        sid = script["id"]
        if sid == "KS-9":
            continue  # AI Copilot / W27 — post-1.0, out of this checkpoint
        print(f"\n  {sid}: {script['title']}")
        for step in script["steps"]:
            n = step["n"]
            ann = step.get("announceExpect")
            text = step.get("announceText")
            disp, note = None, ""

            if (sid, n) in UNSAT:
                disp, note = "SKIP-UNSAT", UNSAT[(sid, n)]
            elif ann in (None, "null"):
                disp, note = "STATE-ONLY", "announceExpect null (no live-region output by design)"
            else:
                ann = str(ann)
                tmpl = catalog.get(ann)
                if tmpl is None:
                    disp = "SKIP-UNCATALOGED"
                    note = f"announce id {ann} not minted in shipped en.json (silent by has()-gate)"
                else:
                    # cataloged: assert the label / golden contract.
                    if text and text in D43_OVERRIDES:
                        shipped = catalog[D43_OVERRIDES[text]]
                        disp = "ASSERTED-CONTRACT"
                        note = (f"D-43 OVERRIDE: shipped label {shipped!r} authoritative; "
                                f"§49 golden {text!r} is trade dress -> QUESTIONS-FOR-BILL")
                    elif text:
                        # golden with concrete text: assert it is derivable
                        # from the catalog template (fixed prefix present).
                        prefix = tmpl.split("{")[0]
                        if prefix and prefix not in text:
                            disp = "VIOLATION"
                            note = f"golden {text!r} inconsistent with catalog {tmpl!r}"
                            violations.append(f"{sid} s{n}: {note}")
                        else:
                            disp = "ASSERTED-CONTRACT"
                            live = " (live-confirmed)" if ann in live_ids else ""
                            note = f"golden {text!r} consistent with catalog {tmpl!r}{live}"
                    else:
                        disp = "ASSERTED-CONTRACT"
                        live = " (live-confirmed)" if ann in live_ids else ""
                        note = f"announce id {ann} cataloged as {tmpl!r}{live}"
                    # tool.selected steps: assert the D-43 shipped tool label
                    if ann == "a11y.tool.selected" and text and text not in D43_OVERRIDES:
                        # non-overridden tool names (Crop) must match a ui.tool.* label
                        if text not in {v for k, v in catalog.items() if k.startswith("ui.tool.")}:
                            note += f" [tool label {text!r} not found among ui.tool.*]"
            tally[disp] = tally.get(disp, 0) + 1
            print(f"     s{n:<2} {disp:<18} {note}")

    print("\n== SUMMARY ==")
    for k in sorted(tally):
        print(f"   {k}: {tally[k]}")
    if violations:
        print(f"\nKS harness: FAIL — {len(violations)} contract violation(s):")
        for v in violations:
            print(f"   VIOLATION: {v}")
        return 1
    print("\nKS harness: PASS — sink contract + cataloged D-43 label contract hold on the "
          "candidate.\n  NOTE (reported to O-INT): no step's KEY SEQUENCE is executed — the "
          "frozen shell lands the A11Y-5 hooks (sink/menu-arming/probes) but no external "
          "KS-script-runner door, so full KS-1..8 keyboard execution is a structural gap "
          "(ASSERTED-CONTRACT verifies the announce/label contract only). KS-3 s2 is typed "
          "SKIP-UNSAT; KS-4 s2 + KS-5 announce ids are SKIP-UNCATALOGED (unminted). No leg "
          "is faked.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
