#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors

"""MessageId audit: registry vs catalogs vs what the build emits.

Spec: ES §27 / Appendix G (MessageId registry discipline); plan §2.2 row
"i18n from day 1" (MessageId discipline from `contracts/message_ids.yaml`;
mint-and-log protocol for gaps). Lane O-I18N-A11Y. stdlib-only,
deterministic (sorted walks, sorted output, no timestamps).

What it checks (RED = exit 1):

  RED-1  an id the build EMITS that is in neither the registry nor
         i18n/MINTED.md (unlogged mint — the drift tripwire)
  RED-2  en does not cover 100% of emitted ids
  RED-3  es/ja/pseudo key sets differ from en's (locales cover the same set)
  RED-4  a locale's template placeholder-name set differs from en's for
         any shared key (dropped/renamed {placeholders})
  RED-5  a catalog key that is in neither the registry, nor MINTED, nor
         the `proto.` seed family (untracked key in an owned catalog)
  RED-6  i18n/pseudo.json is stale w.r.t. i18n/tools/pseudo.py --check
  RED-7  a SHELL-EMITTED command kind whose `history.<kind>` label is
         not in the en catalog. The shell's undo/redo path
         (shell/src/document_controller.cpp) speaks
         a11y.history.undone/redone with {label: tr("history."+kind)} for
         WHATEVER kind was just (un)done — a generic emission gated by
         m_catalog->has("history."+kind), so a missing label is a SILENT
         undo/redo (and the visible journal key-echoes). The shell-
         emitted set is therefore every REACHABLE undoable kind; RED-7
         keys on that set = kinds at SHIPPED_MILESTONES (the checkpoint-
         granular reachability proxy, milestone <= SHIPPED_MILESTONES
         from contracts/commands/_index.yaml) UNION EARLY_SHELL_EMITTED
         (kinds wired & reachable in the shell AHEAD of their milestone
         bump). This is the exact echo class the CP-0.5 chrome wave hit
         (12 seeded gaps) and the CP-1.0 A11Y-5 has()-gate silence.
         Bump SHIPPED_MILESTONES at each checkpoint close (O-INT relay);
         drop an EARLY_SHELL_EMITTED entry once its milestone joins it.

Reported, never red:

  - per-locale registry coverage (most registry ids await the shell — the
    gap is a NUMBER in this report, not a failure)
  - registry ids not yet emitted anywhere (same gap, id-level on request)
  - MINTED ids not yet emitted (staged mints, e.g. a lane's pre-merge code)
  - CLI crate seed catalogs vs the canonical top-level i18n/ (the crate
    copy is the frozen CP-0.1 snapshot pending the relocation handoff;
    this lane is fenced out of crates/ — divergences are LISTED for the
    handoff, counted, and never red here)

Emitted-id derivation (mechanical, no hand lists):

  1. every git-visible `crates/**/*.rs` EXCLUDING `crates/*/tests/`
     integration dirs, with inline `#[cfg(test)] mod … { … }` blocks
     stripped by brace matching (test fixtures like "error.x" are
     references, not emissions); PLUS — since the CP-0.5 chrome-wave
     fold-in — `shell/**/*.{qml,cpp,h}` EXCLUDING `shell/tests/` and
     the untracked `shell/build/` (moc copies would double-count);
  2. every fully-quoted dotted literal "seg.seg[.seg…]" whose first
     segment is one of the twelve registry areas + `cli` + `proto` +
     `ui` (the shell's minted area, see MINTED.md) + `color` (the
     CP-1.0 color-pipeline minted area, see MINTED.md; added when the
     §6.2/§5.5 warning ids joined the sweep);
  3. minus literals that are exactly a registered command KIND from
     `contracts/commands/_index.yaml` (kinds are wire data, never
     MessageIds — e.g. `filter.apply`, `history.goto` in the engine's
     registry tables), minus the four §25.2 CAPABILITY tokens
     (CAPABILITY_WIRE_TOKENS — plugin manifest/grant wire identifiers,
     same class as kinds), and minus tokens whose final segment is a
     file extension (e.g. `filter.apply.yaml` filenames in generated
     tables);
  4. plus FAMILY-CONSTRUCTED ids: the shell builds some keys by
     concatenating a quoted prefix with a runtime value —
     `tr("history." + kind)`, `tr("ui.intent." + i)`,
     `tr("ui.resample." + k)`. Every quoted prefix literal ending in
     "." (e.g. "history.") counts each cataloged-AND-tracked id under
     that prefix as emitted (provenance tag `family:`). The expansion
     is bounded by the en catalog ∩ (registry ∪ MINTED), so RED-2 is
     tautological for family ids — RED-7 is the deliberate tripwire
     for missing `history.<kind>` family members.

  Known limitation (recorded): a registry id spelled identically to a
  registered command kind is invisible to this sweep. The full
  registry∩kinds set is FIVE ids: `adjustment.add`, `filter.apply`,
  `filter.smart_add`, `history.goto`, `history.snapshot`
  (reviewer-verified registry rows; none emitted today). The shell-era
  history labels are the 3-segment `history.<kind>` ids, which do not
  collide — but when filter/adjustment surfaces land, coverage for
  these five must be checked deliberately, not assumed from this sweep.
  The same class grew by FOUR at the plugins-tier companion: the §25.2
  capability tokens `filter.register`, `tool.register`,
  `panel.register`, `menu.action` are wire identifiers exempted from
  the census (CAPABILITY_WIRE_TOKENS) AND registered §25.3 rows — the
  registry∩wire inventory is therefore NINE ids; if any of these four
  ever gains a genuine MessageId surface, its coverage must be checked
  deliberately (the inverted self-check in main() pins the inventory
  against registry drift).
  Second limitation: a family-constructed id whose catalog entry is
  MISSING is invisible to the sweep (the expansion is catalog-bounded);
  RED-7 covers the `history.<kind>` family, and the `ui.intent.*` /
  `ui.resample.*` families are enumerated in MINTED.md.

Usage:
  python3 i18n/tools/audit.py            full report + exit code
  python3 i18n/tools/audit.py --gaps     also list uncovered/unemitted ids
  python3 i18n/tools/audit.py --list-ids print the four raw sets and exit
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
I18N = REPO / "i18n"
LOCALES = ("en", "es", "ja", "pseudo")
# The SHELL SEED REGISTER. shell/CMakeLists.txt stages the catalog the
# product actually loads by running shell/i18n/merge_catalogs.py over
# <repo>/i18n/<loc>.json PLUS shell/i18n/ui.<loc>.json (the shell's own
# minted ui.* keys, pending the O-I18N fold-in), canonical winning on
# collision. An id that lives only in the seed IS SHIPPED, so an audit
# that reads the canonical file alone is blind to exactly the ids the
# shell mints for itself. Operator's word, 2026-09-11 (WORD 4 (1)):
# widen the population to what the build ships.
SHELL_SEED_DIR = REPO / "shell" / "i18n"
SHELL_SEED_MERGER = SHELL_SEED_DIR / "merge_catalogs.py"

REGISTRY_AREAS = (
    "app", "menu", "tool", "panel", "dialog", "error", "history",
    "filter", "adjustment", "pref", "a11y", "unit",
)
SWEEP_AREAS = REGISTRY_AREAS + ("cli", "proto", "ui", "color")
# Wire-schema param-path tokens that share a sweep area's prefix but are
# NOT MessageIds: `EngineError::missing(...)` receives the offending
# params path (e.g. doc.new `color.comps`), which reaches the user only
# inside a `{field}`/`{detail}` argument, never as a catalog key. Every
# entry must stay OUT of the registry and MINTED (self-check in main()).
NON_MESSAGE_PARAM_PATHS = frozenset({"color.comps", "color.model"})
# §25.2 CAPABILITY tokens (`Capability::as_str()`, plugin manifest /
# grant strings — wire identifiers, exactly analogous to command
# KINDS, which the sweep already exempts). UNLIKE the param-paths set
# above, all four ARE ALSO registered §25.3 ids (v5.5 rows 1205 /
# 2076 / 1937 / 1829) — the same spelled-identically class as the
# five registry∩kinds ids in the header's known-limitation inventory.
# The self-check in main() is therefore INVERTED for this set: every
# entry MUST remain a registry row (a fold-in rename/drop errors the
# audit and forces re-inventory) and must NEVER be minted. Exempting
# them widens the deliberate blind spot: if a real MessageId surface
# for these four registered rows ever lands, coverage must be checked
# deliberately, not assumed from this sweep (header inventory updated).
CAPABILITY_WIRE_TOKENS = frozenset({
    "filter.register", "tool.register", "panel.register", "menu.action",
})
SEED_FAMILY_PREFIX = "proto."  # catalog-only proto seeds (see MINTED.md)
SHELL_EXTS = (".qml", ".cpp", ".h")
# Bump at each checkpoint close (O-INT relay): kinds at these milestones
# are implemented, so their history.<kind> labels are reachable (RED-7).
SHIPPED_MILESTONES = ("0.1", "0.3", "0.5", "0.7", "1.0")
# Shell-emitted kinds wired & reachable AHEAD of their milestone's bump.
# The undo/redo announcer emits history.<kind> for ANY (un)done kind
# (document_controller.cpp — generic, not a curated list); a kind that is
# implemented before its checkpoint closes is thus shell-emitted while
# still milestone > SHIPPED_MILESTONES, leaving its label OUTSIDE the
# RED-7 net — the exact window the CP-1.0 A11Y-5 relay flagged (the
# has()-gate went silent on three model-door kinds O-UI opened early).
# RED-7's covered set adds these so a dropped/never-landed label REDs
# instead of going silent. Sourced from the O-UI relay (reports/
# O-UI-log.md @ 05e23d3), NOT guessed. Members must be real, undoable
# (dagger:false) kinds — the self-check in main() pins that, and forbids
# an entry already covered by SHIPPED_MILESTONES (a stale entry whose
# milestone bumped — remove it). REMOVE an entry when its milestone
# joins SHIPPED_MILESTONES (then the milestone gate covers it).
EARLY_SHELL_EMITTED = frozenset({
    # EMPTIED 2026-09-04 when "1.0" joined SHIPPED_MILESTONES (Q-D35).
    #
    # Every one of the seven members -- vector.shape_add, vector.path_new,
    # selection.from_path, heal.stroke, smudge.stroke, dodge_burn.stroke,
    # blur_sharpen.stroke -- is milestone "1.0". The comment above requires
    # REMOVING an entry once its milestone joins, and main()'s self-check
    # ERRORS on an entry already covered, so this is a required half of the
    # bump and not tidying: leaving them would red the audit.
    #
    # THE SET STAYS, EMPTY, BECAUSE IT IS THE MECHANISM AND NOT THE MEMBERS.
    # It exists for kinds wired AHEAD of their milestone's bump; with 1.0
    # shipped there is no later milestone left to run ahead of TODAY, but a
    # future one re-opens the same window and this is where its members go.
    # Deleting the set would delete the lesson that bought it -- four stroke
    # tools shipped without history labels while the audit reported 71/71.
}) 

ID_RE = re.compile(r"^[a-z0-9_]+(?:\.[a-z0-9_]+)+$")
REGISTRY_ROW_RE = re.compile(r"^\s*- id: ([a-z0-9_.]+)\s*$", re.M)
KIND_ROW_RE = re.compile(r"- \{ kind: ([a-z0-9_.]+),")
QUOTED_DOTTED_RE = re.compile(r'"([a-z0-9_]+(?:\.[a-z0-9_]+)+)"')
# quoted prefix literal ending in "." — family-constructed keys (rule 4)
QUOTED_PREFIX_RE = re.compile(r'"((?:[a-z0-9_]+\.)+)"')
MINTED_ROW_RE = re.compile(r"^\| `([a-z0-9_.]+)` \|", re.M)
CFG_TEST_RE = re.compile(r"#\[cfg\(test\)\]")


def strip_cxx_comments(text: str) -> str:
    """Blank out // and /* */ comments in C++/QML, NEVER inside a string.

    A quoted dotted id inside a COMMENT is not an emission -- the comment that
    documents why `history.undo` must never be minted was being read as an
    emission demanding that mint (O-UI, 2026-08-25). But a naive `//` strip is
    worse than the bug: 12 tokens in the swept extensions carry `//` INSIDE a
    string literal ("image://layerthumb/", "image://lucide/contrast",
    "https://appname.org/spec/filter/", and a bare "//"), and truncating at
    those would silently drop every real id later on the line. So this walks
    the text tracking string state, and newlines are preserved so reported
    line numbers do not shift.
    """
    out = []
    i, n, quote = 0, len(text), None
    while i < n:
        c = text[i]
        if quote is not None:
            if c == "\\" and i + 1 < n:
                out.append(text[i:i + 2]); i += 2; continue
            out.append(c)
            if c == quote:
                quote = None
            i += 1; continue
        if c in ('"', "'"):
            quote = c; out.append(c); i += 1; continue
        if c == "/" and i + 1 < n:
            if text[i + 1] == "/":
                j = text.find("\n", i)
                if j < 0:
                    break
                out.append("\n"); i = j + 1; continue
            if text[i + 1] == "*":
                j = text.find("*/", i + 2)
                if j < 0:
                    break
                out.append("\n" * text.count("\n", i, j))  # keep line numbers
                i = j + 2; continue
        out.append(c); i += 1
    return "".join(out)
# {name} placeholders and {name, plural, …} argument names, loader grammar
PLACEHOLDER_RE = re.compile(r"\{([A-Za-z0-9_]+)\}")
PLURAL_ARG_RE = re.compile(r"\{([A-Za-z0-9_]+)\s*,\s*plural\s*,")
FILE_EXT_SEGMENTS = {
    "yaml", "yml", "json", "jsonl", "rs", "py", "sh", "md",
    "png", "jpg", "jpeg", "bmp", "txt", "tmp", "toml", "lock", "appname",
}


def read_registry() -> list[str]:
    text = (REPO / "contracts" / "message_ids.yaml").read_text(encoding="utf-8")
    ids = REGISTRY_ROW_RE.findall(text)
    bad = [i for i in ids if not ID_RE.match(i)]
    assert not bad, f"registry ids failing the id grammar: {bad}"
    dupes = {i for i in ids if ids.count(i) > 1}
    assert not dupes, f"registry duplicate ids: {sorted(dupes)}"
    return ids


def read_command_kinds() -> set[str]:
    text = (REPO / "contracts" / "commands" / "_index.yaml").read_text(encoding="utf-8")
    kinds = set(KIND_ROW_RE.findall(text))
    assert len(kinds) == 102, f"expected 102 command kinds, parsed {len(kinds)}"
    return kinds


KIND_MILESTONE_RE = re.compile(
    r'- \{ kind: ([a-z0-9_.]+),.*?milestone: "([0-9.]+)"')


def read_kind_milestones() -> dict[str, str]:
    """kind -> milestone string, from contracts/commands/_index.yaml."""
    text = (REPO / "contracts" / "commands" / "_index.yaml").read_text(encoding="utf-8")
    rows = dict(KIND_MILESTONE_RE.findall(text))
    assert len(rows) == 102, f"expected 102 milestone rows, parsed {len(rows)}"
    return rows


KIND_DAGGER_RE = re.compile(
    r'- \{ kind: ([a-z0-9_.]+),.*?dagger: (true|false)')


def read_kind_daggers() -> dict[str, bool]:
    """kind -> dagger flag, from contracts/commands/_index.yaml. Dagger
    kinds sit OUTSIDE the undo stack (§10.3/§10.4), so only dagger:false
    kinds are undoable and can be shell-emitted on the undo/redo path —
    the EARLY_SHELL_EMITTED self-check uses this."""
    text = (REPO / "contracts" / "commands" / "_index.yaml").read_text(encoding="utf-8")
    rows = {k: (v == "true") for k, v in KIND_DAGGER_RE.findall(text)}
    assert len(rows) == 102, f"expected 102 dagger rows, parsed {len(rows)}"
    return rows


def red7_missing(covered_kinds: set[str], en_keys: set[str]) -> list[str]:
    """history.<kind> labels required for `covered_kinds` but absent from
    the en catalog. Pure (no I/O) so the RED-7 tripwire can be exercised
    by the self-test in main() — see the strict-validation pattern."""
    return sorted(f"history.{k}" for k in covered_kinds
                  if f"history.{k}" not in en_keys)


def read_minted() -> list[str]:
    text = (I18N / "MINTED.md").read_text(encoding="utf-8")
    ids = MINTED_ROW_RE.findall(text)
    ids = [i for i in ids if ID_RE.match(i)]
    dupes = {i for i in ids if ids.count(i) > 1}
    assert not dupes, f"MINTED.md duplicate rows: {sorted(dupes)}"
    return ids


def read_catalog(locale: str, base: Path = I18N) -> dict[str, str]:
    data = json.loads((base / f"{locale}.json").read_text(encoding="utf-8"))
    return {k: v for k, v in data.items() if k != "_meta" and isinstance(v, str)}


def staged_catalog(locale: str, seed_dir: Path = SHELL_SEED_DIR) -> dict[str, str]:
    """The catalog the BUILD SHIPS for `locale` -- canonical + shell seed.

    The merge is DELEGATED to merge_catalogs.py, the build's own script, so
    the audit cannot drift from the build's collision semantics: this is the
    same code shell/CMakeLists.txt runs, not a second implementation of it.

    An ABSENT SEED is normal and yields the canonical catalog unchanged (the
    merger passes on FileNotFoundError, exactly as the build does). An absent
    MERGER is not normal and RAISES rather than quietly returning the
    unwidened catalog -- a silent fallback here would report a narrower
    population as though it were the shipped one, which is a value that was
    never produced.
    """
    if not SHELL_SEED_MERGER.is_file():
        raise FileNotFoundError(
            f"the build's catalog merger is missing: {SHELL_SEED_MERGER} — "
            "the shipped population cannot be derived, so this audit would "
            "report the canonical catalog as if it were what ships")
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / f"{locale}.json"
        proc = subprocess.run(
            [sys.executable, str(SHELL_SEED_MERGER),
             str(I18N / f"{locale}.json"),
             str(seed_dir / f"ui.{locale}.json"),
             str(out)],
            capture_output=True, text=True)
        if proc.returncode != 0:
            raise RuntimeError(
                f"merge_catalogs.py failed for {locale} "
                f"(rc={proc.returncode}): {proc.stderr.strip()}")
        data = json.loads(out.read_text(encoding="utf-8"))
    return {k: v for k, v in data.items() if k != "_meta" and isinstance(v, str)}


def seed_census(seed_dir: Path = SHELL_SEED_DIR) -> dict[str, int]:
    """Per-locale count of ids the seed contributes that canonical lacks."""
    census = {}
    for loc in LOCALES:
        path = seed_dir / f"ui.{loc}.json"
        if not path.is_file():
            census[loc] = 0
            continue
        seed = json.loads(path.read_text(encoding="utf-8"))
        msgs = seed["messages"] if isinstance(seed.get("messages"), dict) else seed
        canon = set(read_catalog(loc))
        census[loc] = sum(1 for k in msgs if k != "_meta" and k not in canon)
    return census


def strip_test_mods(text: str) -> str:
    """Remove #[cfg(test)]-gated blocks (brace-matched from the marker)."""
    out = []
    pos = 0
    while True:
        m = CFG_TEST_RE.search(text, pos)
        if not m:
            out.append(text[pos:])
            break
        out.append(text[pos : m.start()])
        brace = text.find("{", m.end())
        if brace < 0:
            break  # marker without a block: drop the rest
        depth = 0
        i = brace
        while i < len(text):
            if text[i] == "{":
                depth += 1
            elif text[i] == "}":
                depth -= 1
                if depth == 0:
                    break
            i += 1
        pos = i + 1
    return "".join(out)


def emitted_ids(family_bound: set[str]) -> dict[str, list[str]]:
    """id -> sorted list of repo-relative source files referencing it.

    `family_bound`: ids eligible for the quoted-prefix family expansion
    (rule 4 in the header) — pass en-catalog ∩ (registry ∪ MINTED).
    """
    kinds = read_command_kinds()
    hits: dict[str, set[str]] = {}
    sources: list[Path] = list((REPO / "crates").rglob("*.rs"))
    for ext in SHELL_EXTS:
        sources.extend((REPO / "shell").rglob(f"*{ext}"))
    for path in sorted(sources):
        rel = path.relative_to(REPO).as_posix()
        if "/tests/" in rel:
            continue  # test expectations are references, not emissions
        if rel.startswith("shell/build/"):
            continue  # untracked build tree (moc copies double-count)
        text = path.read_text(encoding="utf-8")
        if path.suffix == ".rs":
            text = strip_test_mods(text)
        else:
            text = strip_cxx_comments(text)
        for tok in QUOTED_DOTTED_RE.findall(text):
            if tok.split(".")[0] not in SWEEP_AREAS:
                continue
            if tok in kinds:
                continue  # command kinds are wire data, not MessageIds
            if tok in NON_MESSAGE_PARAM_PATHS:
                continue  # params-path tokens (EngineError::missing)
            if tok in CAPABILITY_WIRE_TOKENS:
                continue  # §25.2 capability tokens (plugin wire data)
            if tok.rsplit(".", 1)[-1] in FILE_EXT_SEGMENTS:
                continue  # filename, not an id
            hits.setdefault(tok, set()).add(rel)
        # family-constructed keys: quoted prefix "area.sub." + value
        for prefix in QUOTED_PREFIX_RE.findall(text):
            if prefix.split(".")[0] not in SWEEP_AREAS:
                continue
            for fam_id in family_bound:
                if fam_id.startswith(prefix):
                    hits.setdefault(fam_id, set()).add(f"family:{rel}")
    return {k: sorted(v) for k, v in sorted(hits.items())}


def placeholder_names(template: str) -> frozenset[str]:
    return frozenset(PLACEHOLDER_RE.findall(template)) | frozenset(
        PLURAL_ARG_RE.findall(template)
    )


def main() -> int:
    ap = argparse.ArgumentParser(description="MessageId audit")
    ap.add_argument("--gaps", action="store_true",
                    help="also list uncovered registry ids id-by-id")
    ap.add_argument("--list-ids", action="store_true",
                    help="print raw sets (registry/minted/emitted/catalog-en)")
    args = ap.parse_args()

    registry = read_registry()
    registry_set = set(registry)
    # extension-blocklist self-check: no real id may be excludable by it
    assert not [i for i in registry
                if i.rsplit(".", 1)[-1] in FILE_EXT_SEGMENTS], \
        "FILE_EXT_SEGMENTS collides with a registry id"
    minted = read_minted()
    minted_set = set(minted)
    # param-path-blocklist self-check: no real id may be excludable by it
    assert not (NON_MESSAGE_PARAM_PATHS & (registry_set | minted_set)), \
        "NON_MESSAGE_PARAM_PATHS collides with a registry or minted id"
    # capability-token self-check (INVERTED membership, see the set's
    # comment): every token must still be the registered §25.3 row it
    # was inventoried as, and must never be minted — either drift means
    # the exemption no longer matches its rationale and must be
    # re-inventoried before the audit runs green.
    assert not (CAPABILITY_WIRE_TOKENS & minted_set), \
        "CAPABILITY_WIRE_TOKENS collides with a minted id — re-inventory"
    assert CAPABILITY_WIRE_TOKENS <= registry_set, \
        "CAPABILITY_WIRE_TOKENS entry is no longer a registry id — re-inventory"
    # `canonical` is what the REPO stores; `catalogs` is what the BUILD
    # SHIPS (canonical + shell seed). Every coverage question is asked of
    # the shipped population; the crate-seed handoff arm below is asked of
    # the canonical one, because that arm's subject IS the canonical file.
    canonical = {loc: read_catalog(loc) for loc in LOCALES}
    catalogs = {loc: staged_catalog(loc) for loc in LOCALES}
    seeded = seed_census()
    family_bound = set(catalogs["en"]) & (registry_set | minted_set)
    emitted = emitted_ids(family_bound)
    emitted_set = set(emitted)

    if args.list_ids:
        for name, ids in (
            ("registry", registry),
            ("minted", minted),
            ("emitted", sorted(emitted_set)),
            ("catalog-en", sorted(catalogs["en"])),
        ):
            for i in sorted(ids):
                print(f"{name}\t{i}")
        return 0

    failures: list[str] = []
    print("== MessageId audit (i18n/tools/audit.py) ==")

    # -- registry census ---------------------------------------------------
    by_area: dict[str, int] = {}
    for i in registry:
        by_area[i.split(".")[0]] = by_area.get(i.split(".")[0], 0) + 1
    census = ", ".join(f"{a} {n}" for a, n in sorted(by_area.items()))
    print(f"registry: {len(registry)} ids ({census})")
    print(f"minted (i18n/MINTED.md): {len(minted)} ids")
    if any(seeded.values()):
        detail = ", ".join(f"{loc} +{n}" for loc, n in seeded.items() if n)
        print(f"shell seed merged into the audited population "
              f"(shell/i18n/ui.*.json, as shell/CMakeLists.txt stages it): "
              f"{detail}")
    else:
        print("shell seed (shell/i18n/ui.*.json): none present — "
              "the audited population is the canonical catalog alone")

    # -- emitted census ----------------------------------------------------
    n_family = sum(1 for i in emitted_set
                   if all(f.startswith("family:") for f in emitted[i]))
    print(f"emitted by the build today: {len(emitted_set)} ids "
          f"(mechanical sweep of crates/*/src + shell C++/QML, test code "
          f"stripped; {n_family} family-constructed)")
    for i in sorted(emitted_set):
        cls = ("registry" if i in registry_set
               else "MINTED" if i in minted_set else "UNLOGGED")
        print(f"  {i}  [{cls}]  <- {', '.join(emitted[i])}")

    # RED-1: emitted but unlogged
    unlogged = sorted(emitted_set - registry_set - minted_set)
    if unlogged:
        failures.append(
            f"RED-1 emitted ids in neither registry nor MINTED.md: {unlogged}")

    # -- per-locale coverage ----------------------------------------------
    en_keys = set(catalogs["en"])
    print("\nper-locale coverage:")
    print(f"  {'locale':<8} {'emitted':>14} {'registry':>16} {'keys':>6}")
    for loc in LOCALES:
        keys = set(catalogs[loc])
        cov_e = len(emitted_set & keys)
        cov_r = len(registry_set & keys)
        print(f"  {loc:<8} {cov_e:>3}/{len(emitted_set)} "
              f"({100 * cov_e // max(1, len(emitted_set)):>3}%) "
              f"{cov_r:>4}/{len(registry_set)} "
              f"({100 * cov_r // max(1, len(registry_set)):>3}%) "
              f"{len(keys):>6}")
        if loc == "en" and emitted_set - keys:
            failures.append(
                f"RED-2 en missing emitted ids: {sorted(emitted_set - keys)}")
        if loc != "en" and keys != en_keys:
            failures.append(
                f"RED-3 {loc} key set differs from en "
                f"(missing {sorted(en_keys - keys)}, "
                f"extra {sorted(keys - en_keys)})")

    # RED-4: placeholder parity against en
    for loc in ("es", "ja", "pseudo"):
        for key in sorted(en_keys & set(catalogs[loc])):
            want = placeholder_names(catalogs["en"][key])
            got = placeholder_names(catalogs[loc][key])
            if want != got:
                failures.append(
                    f"RED-4 {loc}:{key} placeholders {sorted(got)} != "
                    f"en {sorted(want)}")

    # -- catalog keys not in the registry ---------------------------------
    non_registry = sorted(en_keys - registry_set)
    n_minted = [k for k in non_registry if k in minted_set]
    n_seed = [k for k in non_registry
              if k not in minted_set and k.startswith(SEED_FAMILY_PREFIX)]
    n_untracked = [k for k in non_registry
                   if k not in minted_set and not k.startswith(SEED_FAMILY_PREFIX)]
    print(f"\ncatalog keys not in the registry: {len(non_registry)} "
          f"= {len(n_minted)} minted + {len(n_seed)} proto seed "
          f"+ {len(n_untracked)} untracked")
    if n_untracked:
        failures.append(f"RED-5 untracked catalog keys: {n_untracked}")

    # -- gaps (reported, never red) ---------------------------------------
    reg_unemitted = sorted(registry_set - emitted_set)
    print(f"registry ids not yet emitted anywhere: {len(reg_unemitted)} "
          f"(expected — most UI ids await the shell)")
    reg_uncovered = sorted(registry_set - en_keys)
    print(f"registry ids not yet in catalogs: {len(reg_uncovered)} "
          f"(covered: {len(registry_set & en_keys)}/{len(registry_set)})")
    staged = sorted(minted_set - emitted_set)
    print(f"minted ids not yet emitted (staged/dormant): {len(staged)} "
          f"-> {staged}")
    if args.gaps:
        print("\n-- uncovered registry ids (--gaps) --")
        for i in reg_uncovered:
            print(f"  {i}")

    # -- crate seed sync (informational; relocation handoff) --------------
    crate_dir = REPO / "crates" / "appname-cli" / "i18n"
    print("\nCLI crate seed vs canonical i18n/ (relocation handoff):")
    for loc in LOCALES:
        if not (crate_dir / f"{loc}.json").exists():
            print(f"  {loc}: crate seed absent")
            continue
        seed = read_catalog(loc, crate_dir)
        mine = canonical[loc]
        shared = set(seed) & set(mine)
        diverged = sorted(k for k in shared if seed[k] != mine[k])
        only_canonical = len(set(mine) - set(seed))
        only_seed = sorted(set(seed) - set(mine))
        line = (f"  {loc}: {len(shared)} shared keys, "
                f"{len(diverged)} value-diverged, "
                f"canonical adds {only_canonical}")
        if only_seed:
            line += f", seed-only {only_seed}"
        print(line)
        for k in diverged:
            print(f"    diverged: {k}: seed {seed[k]!r} != canonical {mine[k]!r}")

    # RED-7: shell-emitted-kind history labels (family-constructed, so the
    # literal sweep cannot see a missing one — this is the deliberate
    # check). Covered set = shipped-milestone kinds UNION the
    # EARLY_SHELL_EMITTED supplement (kinds the shell announces ahead of
    # their milestone bump). See the module header + the set's comment.
    milestones = read_kind_milestones()
    daggers = read_kind_daggers()
    shipped = {k for k, m in milestones.items() if m in SHIPPED_MILESTONES}
    # EARLY_SHELL_EMITTED self-check (strict-validation pattern): every
    # supplement entry must be a real, UNDOABLE (dagger:false) kind — else
    # RED-7 would demand a label for a kind that can never be (un)done, a
    # false positive — and must be a genuine SUPPLEMENT, not a stale entry
    # whose milestone already bumped into SHIPPED_MILESTONES (remove those).
    assert EARLY_SHELL_EMITTED <= set(milestones), \
        (f"EARLY_SHELL_EMITTED has non-kinds: "
         f"{sorted(EARLY_SHELL_EMITTED - set(milestones))}")
    _dagger_entries = sorted(k for k in EARLY_SHELL_EMITTED if daggers.get(k))
    assert not _dagger_entries, \
        (f"EARLY_SHELL_EMITTED holds dagger (non-undoable) kinds — RED-7 "
         f"would false-positive on them: {_dagger_entries}")
    _stale = sorted(EARLY_SHELL_EMITTED & shipped)
    assert not _stale, \
        (f"EARLY_SHELL_EMITTED entries are already at SHIPPED_MILESTONES "
         f"(milestone bumped — drop them): {_stale}")
    # RED-7 tripwire negative+positive self-test: prove the detector FIRES
    # on a covered kind whose label is absent, and PASSES when present.
    assert red7_missing({"__probe.kind"}, set()) == ["history.__probe.kind"], \
        "RED-7 tripwire failed to flag a missing covered-kind label"
    assert red7_missing({"__probe.kind"}, {"history.__probe.kind"}) == [], \
        "RED-7 tripwire false-flagged a present covered-kind label"
    covered = shipped | EARLY_SHELL_EMITTED
    label_missing = red7_missing(covered, en_keys)
    print(f"shell-emitted history labels (milestones "
          f"{'/'.join(SHIPPED_MILESTONES)} + {len(EARLY_SHELL_EMITTED)} "
          f"early): {len(covered) - len(label_missing)}/{len(covered)} "
          f"cataloged")
    if label_missing:
        failures.append(
            f"RED-7 shell-emitted kinds missing history.<kind> labels in "
            f"en: {label_missing}")

    # RED-6: pseudo drift
    check = subprocess.run(
        [sys.executable, str(Path(__file__).with_name("pseudo.py")), "--check"],
        capture_output=True, text=True,
    )
    print(f"\npseudo drift guard: {check.stdout.strip() or check.stderr.strip()}")
    if check.returncode != 0:
        failures.append("RED-6 i18n/pseudo.json is stale (pseudo.py --check)")

    # -- verdict -----------------------------------------------------------
    print()
    if failures:
        for f in failures:
            print(f"AUDIT FAIL: {f}")
        return 1
    print("AUDIT PASS: every emitted id is registered or minted; en covers "
          "100% of emitted ids; es/ja/pseudo cover the same set; "
          "placeholders intact; no untracked catalog keys; pseudo fresh.")
    return 0


def selftest_comment_strip() -> int:
    """CONTROL for strip_cxx_comments, BOTH DIRECTIONS (operator ruling, 2026-08-25).

    The fix exists because an id inside a COMMENT was read as an emission. The
    danger of the fix is the mirror: eating real ids. So this asserts that a real
    id in CODE still trips, that a comment-only id does not, and that a `//`
    inside a STRING is untouched -- twelve such tokens exist in the swept
    extensions ("image://layerthumb/", "image://lucide/contrast", a bare "//"),
    and a naive stripper would truncate their lines and drop every id after them.
    """
    fixture = (
        'setStatus(QStringLiteral("ui.error.no_document"));   // real id in CODE\n'
        '// a comment naming "history.undo", which must NOT count as emitted\n'
        '/* block comment naming "history.redo" as well */\n'
        'QUrl u(QStringLiteral("image://layerthumb/abc"));\n'
        'QString s = QStringLiteral("ui.menu.image.crop");\n'
    )
    out = strip_cxx_comments(fixture)
    toks = set(QUOTED_DOTTED_RE.findall(out))
    ok = True
    for must in ("ui.error.no_document", "ui.menu.image.crop"):
        hit = must in toks
        print("  %s  CODE id survives: %s" % ("PASS" if hit else "FAIL", must))
        ok &= hit
    for gone in ("history.undo", "history.redo"):
        hit = gone not in toks
        print("  %s  COMMENT id removed: %s" % ("PASS" if hit else "FAIL", gone))
        ok &= hit
    url = "image://layerthumb/abc" in out
    print("  %s  '//' inside a STRING untouched" % ("PASS" if url else "FAIL"))
    ok &= url
    nl = out.count("\n") == fixture.count("\n")
    print("  %s  line numbers preserved (newline count unchanged)" % ("PASS" if nl else "FAIL"))
    ok &= nl
    return 0 if ok else 1


def selftest_seed_population() -> int:
    """CONTROL for staged_catalog, BOTH DIRECTIONS (operator's WORD 4 (1)).

    The widening exists because an id that ships ONLY in the shell seed was
    invisible to the audit. The mirror danger is a population that widens
    when nothing was seeded -- or, worse, one that silently NARROWS back to
    the canonical catalog when the build's merger cannot be run, reporting a
    value it never produced. So this asserts: a seeded id appears; a
    canonical value WINS over a seed value of the same key (the build's own
    collision rule); an absent seed leaves the population byte-equal to the
    canonical catalog; and an absent MERGER raises instead of narrowing.
    """
    ok = True
    canon_en = read_catalog("en")
    collide = next(iter(sorted(canon_en)))

    with tempfile.TemporaryDirectory() as td:
        empty = Path(td) / "empty"
        empty.mkdir()
        same = staged_catalog("en", empty) == canon_en
        print("  %s  no seed: population equals the canonical catalog"
              % ("PASS" if same else "FAIL"))
        ok &= same

        seeded_dir = Path(td) / "seeded"
        seeded_dir.mkdir()
        (seeded_dir / "ui.en.json").write_text(json.dumps({
            "ui.selftest.seed_only": "an id that ships only in the seed",
            collide: "SEED VALUE THAT MUST LOSE",
        }), encoding="utf-8")
        got = staged_catalog("en", seeded_dir)
        grew = "ui.selftest.seed_only" in got
        print("  %s  seeded id enters the audited population" % ("PASS" if grew else "FAIL"))
        ok &= grew
        won = got.get(collide) == canon_en[collide]
        print("  %s  canonical value wins on collision: %s" % ("PASS" if won else "FAIL", collide))
        ok &= won
        untouched = len(got) == len(canon_en) + 1
        print("  %s  exactly one id added, nothing displaced" % ("PASS" if untouched else "FAIL"))
        ok &= untouched

    global SHELL_SEED_MERGER
    keep = SHELL_SEED_MERGER
    SHELL_SEED_MERGER = keep.parent / "merge_catalogs_absent_by_design.py"
    try:
        staged_catalog("en")
        raised = False
    except FileNotFoundError:
        raised = True
    finally:
        SHELL_SEED_MERGER = keep
    print("  %s  absent merger RAISES rather than reporting the narrow population"
          % ("PASS" if raised else "FAIL"))
    ok &= raised
    return 0 if ok else 1


if __name__ == "__main__":
    if "--selftest" in sys.argv:
        # BOTH run, always: `a or b` would let a comment-strip failure
        # hide the seed-population arms entirely.
        rc_strip = selftest_comment_strip()
        rc_seed = selftest_seed_population()
        sys.exit(max(rc_strip, rc_seed))
    sys.exit(main())
