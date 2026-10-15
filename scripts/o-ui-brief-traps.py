#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
"""Print the O-UI brief traps block, GENERATED from reports/O-UI-MEASURED-FACTS.md.

Why this exists (C-184): the traps block was hand-retyped into every brief and was
wrong six times in one day. The measured-facts index was landed to stop that, and
the retyping continued anyway -- "a citation does not reach fingers". A document
cannot fix that; §G-4 of the index says so itself: *"Nothing in this file is
enforced by a test. It is prose, and prose does not run."*

So this does not cite the index. It PRINTS from it. A correction to a row lands in
the next generated block with no human step and no retyping.

NO TRAP TEXT IS EMBEDDED HERE. Every emitted word is read from the index at run
time. The only things named in this file are STRUCTURE (section headings, the
verdict vocabulary the index defines for itself, the routing keywords, and which
appendix is pinned) -- never a fact. If the index is missing, this exits non-zero
rather than printing a stale copy; if the pinned appendix is missing, likewise.

NO ROW IS DROPPED IN SILENCE (C-187). A table row inside a recognised fact
table that does not parse used to vanish with rc 0, empty stderr and a MOVED
blob id -- so whoever added a fact in the wrong shape read "it landed" off the
one thing that had genuinely changed. That is the intake leak inside the tool
built to close the intake leak. Such a row is now NAMED on stderr with its file
and line; stdout is untouched, because stdout is pasted into a brief and a
warning mixed into it would corrupt the deliverable. `--strict` turns the same
finding into rc 3, and `--selftest` FAILS on it -- the only one of the three a
machine can red on. The check is scoped to tables that head their first column
`#` (exact match, never containment -- C-182), so separators, header rows and
prose tables cannot cry wolf.

NO APPENDIX RULE IS DROPPED IN SILENCE (C-208). The C-187 guard above only ever
watched TABLE rows, so an appendix rule -- a `**G-n — claim**` paragraph -- walked
around the guard built for exactly this class: 20 rules were in the index and 9
reached the block, with rc 0, empty stderr and a PASSING --selftest. Two causes,
both fixed here. (1) Only `F-n` was recognised as an appendix-rule id, so a `G-n`
paragraph was not a fact at all. (2) An appendix with no recognised rule
contributed ONE paragraph, ranked by star count -- so a block carrying four rules
emitted the best-starred one and lost three. Rules are now read per PARAGRAPH,
never per block. The coverage check is INDEPENDENT of the parser it checks:
--selftest rescans the raw index with a deliberately loose id pattern and
compares that set against the set the parser emitted -- because a rule the parser
cannot see is also a rule the parser cannot COUNT, which is exactly how the tally
check kept passing while 11 rules were missing.

Language: Python, not sh, for three reasons. (1) The parse is markdown tables plus
starred prose; sh would need awk and would be harder to audit. (2) The index itself
records that on this host `grep` is a ugrep shell function (A14), `git grep -E`
has no `\b` (A17), zsh eats `$VAR:path` (A6) and hides rc through process
substitution (A10) -- a shell implementation of the traps printer would be exposed
to the very traps it prints. (3) stdlib-only Python is already the repo's tooling
idiom (i18n/tools/audit.py, .claude/hooks/quick_gate.py).

Usage:
  scripts/o-ui-brief-traps.py                 # short default block, paste-ready
  scripts/o-ui-brief-traps.py --build         # add build-freshness rules
  scripts/o-ui-brief-traps.py --readonly      # drop build-freshness, add scope
  scripts/o-ui-brief-traps.py --all           # every non-retracted fact
  scripts/o-ui-brief-traps.py --section A     # one section, uncapped
  scripts/o-ui-brief-traps.py --show-retracted# the DO-NOT-REUSE list, labelled
  scripts/o-ui-brief-traps.py --selftest      # invariants, incl. the tally check
  scripts/o-ui-brief-traps.py --strict        # rc 3 if any row failed to parse
  scripts/o-ui-brief-traps.py --index PATH | --ref lane/ui
"""
import argparse
import hashlib
import os
import re
import subprocess
import sys

RELPATH = "reports/O-UI-MEASURED-FACTS.md"
# A1: bare `git` on this host is 2.15.0 (2017). Always the real one.
GIT = "/usr/bin/git"
STAR = "★"

# The appendix whose content is pinned into EVERY profile. Only the ID is named
# here; every word of it is read from the index. Rationale: C-180 is the only
# entry in the index that stops a brief from DEMANDING A FALSE CLAIM (a data
# mutation's honest NCOMPILE is 0), so it is not build-scoped -- it is
# truth-telling-scoped, and a read-only round can hit it too.
PINNED_APPENDICES = ("C-180",)

# Firing frequency measured across 43 incidents (C-184 audit): control 7,
# shell 4, catalog 4, build 3, scope 1. Default = the top of that ranking.
CATEGORY_ORDER = ["control", "shell", "catalog", "build", "scope", "host"]
CATEGORY_LABEL = {
    "control": "CONTROL DISCIPLINE",
    "shell": "SHELL QUOTING / EXPANSION",
    "catalog": "CATALOG / i18n",
    "build": "BUILD FRESHNESS + GATE SCOPE",
    "scope": "SCOPE / DENOMINATOR",
    "host": "HOST TOOLING",
}
PROFILES = {
    # name: (categories in order, per-category cap)
    # Caps raised 2026-08-26 under operator ruling 9. Roughly +70%: default
    # goes 13 -> 22 rows. Chosen to stay READABLE at a session-start hook --
    # a brief nobody finishes is a cap of zero with extra steps.
    "default": (["control", "shell", "catalog"], {"control": 8, "shell": 10, "catalog": 4}),
    "build": (["control", "shell", "build", "catalog"],
              {"control": 7, "shell": 8, "build": 8, "catalog": 4}),
    "readonly": (["control", "shell", "catalog", "scope"],
                 {"control": 8, "shell": 10, "catalog": 5, "scope": 4}),
}

# Routing keywords. These decide WHERE a fact goes, never WHAT it says, and they
# are matched against the row's own text -- so a row added to the index tomorrow
# is routed without editing this file.
RX_SHELL = re.compile(
    r"zsh|shell function|word-split|nomatch|glob|quot|PIPESTATUS|pipestatus|"
    r"pipeline|pipe\b|substitution|history modifier|\$\{|\$VAR|ugrep|grep|"
    r"expansion", re.I)
RX_BUILD = re.compile(
    r"sha256|ctest|cmake|ninja|dylib|rebuild|relink|NCOMPILE|BUILD_RC|freshness|"
    r"binary|gate\.sh|cargo|build|SPDX|hook|enforcer", re.I)
RX_SCOPE = re.compile(
    r"denominator|without its ref|which of the three|cite which number|"
    r"scoped narrower|per-worktree|not a property of the repo|absolute path|"
    r"names? the tree|moved ref|moved tip|scoping a brief|readable file", re.I)
RX_CONTROL = re.compile(
    r"control|mechanism|infer|both directions|false negative|wolf|detector", re.I)


class Fact(object):
    """One emitted-capable fact: a table row, or an id'd paragraph in an appendix."""

    def __init__(self, fid, section, stars, headline, qualifier, verdict, origin):
        self.fid = fid
        self.section = section
        self.stars = stars
        self.headline = headline
        self.qualifier = qualifier
        self.verdict = verdict          # raw verdict text ("" for §D and appendices)
        self.origin = origin            # e.g. "§A", "C-184"
        self.category = None

    # -- the index's own verdict vocabulary (§ "Verdict key") -------------------
    @property
    def verdict_word(self):
        """The row's verdict class, by the index's own convention: a row-level
        verdict LEADS with its verdict word ("NARROWED — see §E-5", "CONFIRMED
        (already narrowed in C-170)" -> CONFIRMED, not NARROWED). Only when the
        cell does not lead with one -- i.e. the cell retracts some other, quoted,
        line -- does keyword precedence apply.

        This is the classification the index uses for its own TALLY line, and
        --selftest reconciles against that line."""
        v = clean(self.verdict).upper()
        v = re.sub(r"^NEW\s*[—-]\s*", "", v)
        for word in ("RETRACTED", "NARROWED", "CONFIRMED"):
            if v.startswith(word):
                return word
        for word in ("RETRACTED", "NARROWED", "CONFIRMED"):
            if word in v:
                return word
        return "UNCLASSIFIED"

    @property
    def retracted(self):
        """True iff the verdict retracts THIS ROW'S OWN CLAIM.

        NOT the same as `verdict_word == "RETRACTED"`. Four rows carry the word;
        two of them (A7, B7) are the CORRECTIONS -- their verdict says some other,
        quoted, boilerplate line is retracted, and the row itself is the fact that
        replaces it. A leading RETRACTED ("RETRACTED as stated", "RETRACTED as
        universal") retracts the row; a RETRACTED that arrives after a quoted
        antecedent does not. Dropping A7 would delete the single most-used trap in
        the boilerplate (`${VAR}:path`), so this distinction is load-bearing."""
        v = clean(self.verdict).upper()
        v = re.sub(r"^NEW\s*[—-]\s*", "", v)
        return v.startswith("RETRACTED")

    @property
    def supersedes(self):
        if self.retracted or self.verdict_word != "RETRACTED":
            return ""
        m = re.search(r"§(E-\d+)", self.verdict)
        return m.group(1) if m else ""


def clean(text):
    """Markdown -> paste-ready plain text. Backticks are KEPT: the shell forms are
    the fact. Stars are removed from the text but scored for ranking."""
    t = text.replace("\\|", "|")
    t = t.replace(STAR, "")
    t = re.sub(r"\*\*(.+?)\*\*", r"\1", t, flags=re.S)
    t = re.sub(r"(?<![\w*])\*(?!\s)(.+?)(?<!\s)\*(?![\w*])", r"\1", t, flags=re.S)
    t = re.sub(r"\s+", " ", t)
    return t.strip()


def headline_of(cell, maxlen):
    """First bold run if the cell opens with one, else the whole cell; then, if
    that lead is a finished sentence or under 100 chars, as much of the following text as
    fits in whole sentences."""
    s = cell.strip()
    s = re.sub(r"^(?:%s|\s)+" % STAR, "", s)
    lead, rest = s, ""
    m = re.match(r"\*\*(.+?)\*\*", s, re.S)
    if m:
        lead, rest = m.group(1), s[m.end():]
    lead = clean(lead)
    extra = re.sub(r"^[—-]\s*", "", clean(rest))
    if extra and (lead.endswith((".", ":", "!", "?")) or len(lead) < 100):
        lead += (" " if lead.endswith((".", "!", "?")) else " -- ") + extra
    return soft_clip(lead, maxlen)


def truncate(text, maxlen):
    if len(text) <= maxlen:
        return text
    cut = text[:maxlen]
    for mark in (". ", "; "):
        dot = cut.rfind(mark)
        if dot > maxlen * 0.5:
            return cut[:dot + 1]
    sp = cut.rfind(" ")
    return (cut[:sp] if sp > 0 else cut).rstrip(" ,;-") + "…"


# Split on sentence end only before a capital, so "e.g. a" and "2.39.5" survive.
SENT = re.compile(r"(?<=[.!?]) +(?=[A-Z(\"'`])")


def whole_sentences(text, budget):
    """Take WHOLE sentences up to a budget, never a mid-sentence cut.

    Requirement-5 guard: a character truncation of the C-180 stanza landed exactly
    before "would force a false claim", i.e. it silently deleted the clause the
    rule exists for. Whole sentences or nothing."""
    parts = SENT.split(text)
    out = parts[0]
    for p in parts[1:]:
        if len(out) + 1 + len(p) > budget:
            break
        out += " " + p
    return out


def soft_clip(text, budget):
    """Whole sentences up to a budget -- and a first sentence that is already over
    budget is kept WHOLE rather than cut. Half a trap is a wrong trap: a clip at
    150 chars turned C11 into "the C++ doors refuse..." and deleted the half that
    says the QML label does not. Only runaway text (2x budget) is ellipsized."""
    out = whole_sentences(text, budget)
    return truncate(out, 2 * budget) if len(out) > 2 * budget else out


def split_row(line):
    cells = re.split(r"(?<!\\)\|", line.strip())
    if cells and cells[0].strip() == "":
        cells = cells[1:]
    if cells and cells[-1].strip() == "":
        cells = cells[:-1]
    return cells


# A markdown delimiter row: EVERY cell is dashes, with optional alignment colons.
# All cells, not just the first: a body cell that happens to be "-" must not be
# able to make a real row look structural.
DASH_CELL = re.compile(r"^:?-+:?$")


def is_separator_row(line):
    cells = split_row(line)
    return bool(cells) and all(DASH_CELL.match(c.strip()) for c in cells)


ROW_ID = re.compile(r"^\s*\*\*([A-D]\d{1,2})\*\*\s*(%s*)\s*$" % STAR)
SECTION = re.compile(r"^##\s+§([A-G])\s*—\s*(.*)$")
APPENDIX = re.compile(r"^##\s+(%s*)\s*APPENDED[^(]*\((C-\d+)\)\s*—\s*(.*)$" % STAR)
E_ENTRY = re.compile(r"^###\s+(E-\d+)\s*(%s*)\s*(.*)$" % STAR)
# C-467: WIDENED FOR THE `O-UI ` DESK PREFIX. From G-125 onward this desk minted
# `**O-UI G-n — claim**` (the namespace claim of C-340/C-383). All THREE patterns
# below anchored the id immediately after the bold marker, so the prefix hid every
# mint after G-124 -- 45 rules, invisible to the strict parser, to the near-miss
# warner, AND to the "independent" loose rescan, which is why --selftest PASSED
# while declaring 128 of 173. The independence was real in intent and absent in
# fact: all three shared the assumption that the id OPENS the bold span (G-41/G-85
# -- instruments sharing an assumption are one instrument). Only `O-UI ` is
# accepted, never a bare prefix: C-340 ruled the G-namespace NOT shared, so another
# desk's G-n cited in this file must still not parse as a rule of ours.
# An APPENDIX RULE: `**F-n — claim**` or `**G-n — claim**`, ONE PER PARAGRAPH.
# C-208: this read `F` only, so every `G` rule in the index was invisible here and
# the block fell back to one paragraph per appendix -- 11 of 20 rules dropped.
APX_RULE = re.compile(r"^\*\*(?:O-UI\s+)?([FG]-\d{1,3}[a-z]?)\s*—\s*")
APX_RULE_ID = re.compile(r"^[FG]-\d{1,3}[a-z]?$")   # trailing letter = a REFINEMENT of the numbered rule (G-44b refines G-44); widened 2026-08-25 after G-44b/G-45b failed to parse
# Announces itself as an appendix-rule id but is not in APX_RULE's shape.
# Deliberately narrow: it fires only on text that already claims to be a rule, so
# an ordinary appendix paragraph can never trip it.
APX_RULE_NEARMISS = re.compile(r"^\*\*\s*(?:O-UI\s+)?[FG]\s*-?\s*\d+")
# The INDEPENDENT scan behind the coverage check. It deliberately shares no
# pattern with APX_RULE: if the strict one stops matching, this one must still see
# the rule, or the check inherits the very blindness it exists to catch. Loose on
# the bold markers, on which dash, and on leading space -- but ANCHORED AT LINE
# START, so a mention inside a sentence ("O-INT audited G-13 ...") is not a
# declaration. Measured on the real index: 23 hits, 23 real rules, 0 false.
APX_RULE_LOOSE = re.compile(r"^[ \t]*\**[ \t]*(?:O-UI[ \t]+)?([FG]-\d{1,3}[a-z]?)[ \t]*[—–-]", re.M)
# A directive paragraph is what a brief needs out of an appendix that has no
# id'd rule of its own ("REQUIRE BOTH:", "SO:", "USE ...").
DIRECTIVE = re.compile(r"^(%s*\s*)?\*\*(REQUIRE|SO|USE|DO NOT|ALWAYS|NEVER)" % STAR)


def rule_key(fid):
    """Sort G-2 before G-10. Never lexical: the ids are numbered, not named."""
    m = re.match(r"^([FG])-(\d{1,3})([a-z]?)$", fid)
    if not m:                      # never crash the selftest on an id shape it can't order
        return (fid[0], 10**6, fid)
    return (m.group(1), int(m.group(2)), m.group(3))


class Index(object):
    def __init__(self, text, source, blob, maxlen=150):
        self.text = text
        self.source = source
        self.blob = blob
        self.maxlen = maxlen
        self.host_ref = ""
        self.facts = []          # emit-capable, in document order
        self.retracted_rows = []  # Fact objects whose own claim is retracted
        self.e_entries = []      # (id, stars, quoted line)
        self.f_rows = []         # §F "asserted, not re-verified" (no ids)
        self.appendices = {}     # id -> {"stars", "title", "paras"}
        self.tally = None        # (rows, confirmed, narrowed, retracted) from prose
        self.refs = []           # provenance lines from the header table
        self.problems = []       # (lineno, where, reason, raw) -- did NOT parse
        self._parse()

    # ---------------------------------------------------------------- parsing
    def _parse(self):
        section, appendix = None, None
        para, para_line = [], 0
        in_fact_table = False
        lines = self.text.splitlines()
        for i, line in enumerate(lines):
            lineno = i + 1
            if not line.startswith("|"):
                in_fact_table = False       # a table is contiguous `|` lines
            ms = SECTION.match(line)
            if ms:
                self._flush(appendix, para, para_line)
                para, section, appendix = [], ms.group(1), None
                continue
            ma = APPENDIX.match(line)
            if ma:
                self._flush(appendix, para, para_line)
                para = []
                appendix = ma.group(2)
                self.appendices[appendix] = {
                    "stars": len(ma.group(1)), "title": clean(ma.group(3)), "paras": []}
                section = None
                continue
            if appendix is not None:
                if line.strip():
                    if not para:
                        para_line = lineno
                    para.append(line)
                else:
                    self._flush(appendix, para, para_line)
                    para = []
                continue
            if section in ("A", "B", "C", "D") and line.startswith("|"):
                # WHICH table lines are checkable. Structure only -- the parse
                # itself is unchanged below, so emission cannot move.
                nxt = lines[i + 1] if i + 1 < len(lines) else ""
                if is_separator_row(line):
                    checkable = False                  # the delimiter row
                elif is_separator_row(nxt):
                    # A header row, and it NAMES its table. Every fact table in
                    # §A-§D heads its first column `#`. EXACT match, never
                    # containment (C-182) -- so a prose table with any other
                    # header is never checked and cannot cry wolf.
                    cells = split_row(line)
                    in_fact_table = bool(cells) and cells[0].strip() == "#"
                    checkable = False
                else:
                    checkable = in_fact_table
                self._row(section, line, lineno, checkable)
            elif section == "E":
                me = E_ENTRY.match(line)
                if me:
                    self.e_entries.append(
                        (me.group(1), len(me.group(2)), clean(me.group(3))))
            elif section == "F" and line.startswith("|"):
                cells = split_row(line)
                if len(cells) == 2 and not set(cells[0].strip()) <= set("- :"):
                    if "why not re-measured" not in cells[1]:
                        self.f_rows.append(clean(cells[0]))
            if section is None and appendix is None:
                self._header(line)
        self._flush(appendix, para, para_line)
        self._appendix_facts()
        self._rule_coverage()
        self._route()

    # ------------------------------------------------- appendix-rule coverage
    def rule_ids_declared(self):
        """Every appendix-rule id the RAW TEXT declares, found WITHOUT the
        parser's own pattern (C-208). Ordered by line; value is the line number."""
        out = {}
        for m in APX_RULE_LOOSE.finditer(self.text):
            out.setdefault(m.group(1), self.text.count("\n", 0, m.start()) + 1)
        return out

    def rule_ids_emitted(self):
        """Every appendix rule that became a fact UNDER ITS OWN ID. A rule whose
        words merely ride along inside some other line does NOT count: a brief
        cites by id, and an id that is never printed cannot be cited."""
        return set(f.fid for f in self.facts
                   if f.section == "APX" and APX_RULE_ID.match(f.fid))

    def _rule_coverage(self):
        """A declared rule that is not emitted is the C-187 silent drop one layer
        over. Recorded as a problem, so it reaches the SAME three doors the
        table-row check reaches: named on stderr, rc 3 under --strict, FAIL under
        --selftest. Diagnostics only -- stdout is untouched."""
        emitted = self.rule_ids_emitted()
        for fid, lineno in sorted(self.rule_ids_declared().items(),
                                  key=lambda kv: kv[1]):
            if fid in emitted:
                continue
            self.problems.append(
                (lineno, "appendix",
                 "rule %s is DECLARED in the index and is NOT in the block" % fid,
                 fid))

    def _appendix_facts(self):
        """An appendix that is not pinned contributes ONE line beyond its rules.

        No id'd rule of its own -> its best paragraph, as before. C-208: an
        appendix whose rules are now emitted under their own ids contributes only
        a DIRECTIVE paragraph, and only if it has one -- otherwise this line would
        either restate a rule the block already carries or promote a leftover.
        Discovered, not listed: an appendix added tomorrow shows up untouched."""
        have = set(f.origin for f in self.facts if f.section == "APX")
        for aid in sorted(self.appendices):
            if aid in PINNED_APPENDICES:
                continue
            lines = self.appendix_lines(aid, 1, directive_only=(aid in have))
            if lines:
                self.facts.append(Fact(aid, "APX", self.appendices[aid]["stars"],
                                       truncate(lines[0], self.maxlen + 90),
                                       "", "", aid))

    def _header(self, line):
        mt = re.search(r"TALLY\s*—\s*(\d+)\s+measured rows.*?(\d+)\s+CONFIRMED"
                       r".*?(\d+)\s+NARROWED.*?(\d+)\s+RETRACTED", line)
        if mt:
            self.tally = tuple(int(g) for g in mt.groups())
        if line.startswith("|") and "`" in line:
            cells = split_row(line)
            if len(cells) == 2 and cells[0].strip().startswith("`"):
                branch = clean(cells[0]).strip("`")
                sha = re.search(r"`([0-9a-f]{7,40})`", cells[1])
                self.refs.append((branch, truncate(clean(cells[1]), 90)))
                if sha and not self.host_ref:
                    self.host_ref = "%s@%s" % (branch, sha.group(1))

    def _row(self, section, line, lineno=0, checkable=False):
        """Parse one table row.

        WHAT IT ACCEPTS IS UNCHANGED. Each early return below dropped the row
        before and drops it identically now; the only addition is that a drop
        inside a recognised fact table is RECORDED. C-187: a row in the wrong
        shape vanished with rc 0, empty stderr and a MOVED blob id, so its
        author read "it landed" off the one thing that had in fact changed."""
        need = 3 if section == "D" else 5
        cells = split_row(line)
        if len(cells) < 3:
            return self._dropped(lineno, section, line, checkable,
                                 "%d cell(s); a §%s row needs %d"
                                 % (len(cells), section, need))
        mid = ROW_ID.match(cells[0])
        if not mid:
            return self._dropped(
                lineno, section, line, checkable,
                "first cell is not a bold fact id (**A1**, **D7**): %s"
                % (truncate(clean(cells[0]), 40) or "empty"))
        fid, stars = mid.group(1), len(mid.group(2))
        if section == "D":
            claim, qual, verdict = cells[1], "", ""
        else:
            if len(cells) < 5:
                return self._dropped(
                    lineno, section, line, checkable,
                    "%s has %d cells; a §%s row needs %d "
                    "(| # | claim | evidence | scope | verdict |)"
                    % (fid, len(cells), section, need))
            claim, qual, verdict = cells[1], cells[3], cells[4]
        stars = max(stars, claim.count(STAR))
        qtext = ""
        if STAR in qual or "**" in qual:
            qtext = soft_clip(clean(qual), 95)
        f = Fact(fid, section, stars, headline_of(claim, self.maxlen), qtext,
                 verdict, "§" + section)
        if f.retracted:
            self.retracted_rows.append(f)
        else:
            self.facts.append(f)

    def _dropped(self, lineno, where, line, checkable, reason):
        """Record a row that did not parse. Diagnostics only: it returns None,
        exactly as the bare `return` it replaced, so nothing about emission
        depends on it."""
        if checkable:
            self.problems.append((lineno, "§" + where, reason, line.strip()))
        return None

    def _flush(self, appendix, para, lineno=0):
        if appendix is None or not para:
            return
        text = "\n".join(para).strip()
        self.appendices[appendix]["paras"].append(text)
        mf = APX_RULE.match(text)
        if mf:
            # Re-open the bold run without the id, so headline_of() applies the
            # same lead-plus-following-sentence rule it applies to table rows.
            body = "**" + text[mf.end():]
            self.facts.append(Fact(mf.group(1), "APX", text.count(STAR),
                                   headline_of(body, self.maxlen), "", "", appendix))
        elif APX_RULE_NEARMISS.match(text):
            # The same silent drop, one layer over: a paragraph that announces
            # itself as an appendix rule but is not in the shape that becomes one.
            self.problems.append(
                (lineno, appendix,
                 "opens with an appendix-rule id but not in the "
                 "`**G-n — claim**` shape (em dash U+2014 required)",
                 truncate(clean(text), 80)))

    # --------------------------------------------------------------- routing
    def _route(self):
        for f in self.facts:
            blob = " ".join([f.fid, f.headline, f.qualifier, f.origin])
            if f.section == "D" or (f.section == "APX" and RX_CONTROL.search(blob)):
                f.category = "control"
            elif f.section == "A" and RX_BUILD.search(blob) and not RX_SHELL.search(blob):
                f.category = "build"
            elif f.section == "A" and RX_SHELL.search(blob):
                f.category = "shell"
            elif f.section == "B":
                f.category = "build"
            elif f.section == "C":
                f.category = "catalog"
            elif RX_SCOPE.search(blob):
                f.category = "scope"
            elif RX_BUILD.search(blob):
                f.category = "build"
            else:
                f.category = "host"

    # ------------------------------------------------------- appendix render
    def appendix_lines(self, aid, limit, directive_only=False):
        apx = self.appendices.get(aid)
        if not apx:
            return []
        out, paras = [], []
        for p in apx["paras"]:
            flat = p.strip()
            if re.match(r"^\*\*(Measured on|Denominator)", flat):
                continue
            if APX_RULE.match(flat):
                continue
            paras.append(flat)
        if directive_only:
            paras = [p for p in paras if DIRECTIVE.match(p)]
        # A directive paragraph ("REQUIRE BOTH:", "SO:", "USE ...") is what a brief
        # needs; otherwise fall back to document order, which keeps an antecedent
        # ("THAT IS CORRECT FOR...") attached to the rule it refines.
        if limit == 1:
            ranked = sorted(range(len(paras)),
                            key=lambda i: (0 if DIRECTIVE.match(paras[i]) else 1,
                                           -paras[i].count(STAR), i))
            paras = [paras[i] for i in ranked[:1]]
        else:
            paras = paras[:limit]
        for p in paras:
            body = clean(re.sub(r"^>\s*", "", p, flags=re.M))
            out.append(whole_sentences(body, 380))
        return out


def blob_sha(data):
    """git's own blob id -- content-addressed, so the header is stable and does
    not need a timestamp (C-182: content-addressed beats path-diff)."""
    h = hashlib.sha1()
    h.update(b"blob %d\0" % len(data))
    h.update(data)
    return h.hexdigest()


def load(args):
    if args.index:
        path = os.path.abspath(args.index)
        if not os.path.exists(path):
            die("index not found: %s" % path)
        with open(path, "rb") as fh:
            raw = fh.read()
        return raw, path
    here = os.path.dirname(os.path.abspath(__file__))
    root = os.path.dirname(here)
    path = os.path.join(root, RELPATH)
    if os.path.exists(path) and not args.ref:
        with open(path, "rb") as fh:
            raw = fh.read()
        return raw, path
    # The index lives on lane/ui and is NOT on main. Reading it out of the object
    # store means a worktree on another branch can still generate a block.
    ref = args.ref or "lane/ui"
    try:
        raw = subprocess.check_output([GIT, "-C", root, "show", "%s:%s" % (ref, RELPATH)],
                                      stderr=subprocess.PIPE)
    except (OSError, subprocess.CalledProcessError) as exc:
        die("index not on disk at %s and `%s show %s:%s` failed (%s).\n"
            "       The index is not on main; pass --index PATH or --ref BRANCH."
            % (path, GIT, ref, RELPATH, exc))
    return raw, "%s:%s" % (ref, RELPATH)


def die(msg):
    sys.stderr.write("o-ui-brief-traps: ERROR: %s\n" % msg)
    raise SystemExit(2)


def report_problems(idx, strict):
    """Diagnostics go to STDERR, NEVER stdout.

    stdout is the deliverable -- it is pasted verbatim into a brief -- so a
    warning mixed into it would corrupt the thing this tool exists to produce.
    Exit stays 0 by default, so a pipeline still gets a clean block and an
    unrelated round is not broken by someone else's bad row; --strict turns the
    same finding into rc 3 for anything that wants to fail on it."""
    if not idx.problems:
        return 0
    sys.stdout.flush()          # keep terminal ordering: block first, then this
    for lineno, where, reason, raw in idx.problems:
        sys.stderr.write(
            "o-ui-brief-traps: WARNING: %s:%d: %s row DID NOT PARSE and is NOT in "
            "the block: %s\n" % (idx.source, lineno, where, reason))
        sys.stderr.write("o-ui-brief-traps:          %s\n" % truncate(raw, 110))
    sys.stderr.write(
        "o-ui-brief-traps: %d row(s) dropped. A dropped row is a fact that is NOT "
        "in the block above -- copy a NEIGHBOURING row's shape exactly. "
        "(--strict = rc 3; --selftest fails on this)\n" % len(idx.problems))
    sys.stderr.flush()
    return 3 if strict else 0


def select(idx, args):
    cats, caps = PROFILES[args.profile]
    picked, by_cat = [], {}
    for f in idx.facts:
        by_cat.setdefault(f.category, []).append(f)
    for c in cats:
        pool = by_cat.get(c, [])
        # The author's own danger marker first (2+ stars), then document order.
        pool = sorted(pool, key=lambda f: (0 if f.stars >= 2 else 1,
                                           idx.facts.index(f)))
        picked.extend(pool[:caps.get(c, 3)])
    # Some rows exist to FIX another row (A7 fixes A6). Printing the trap without
    # its fix is worse than printing neither -- it pushes a round away from a
    # working idiom (§E-1's own diagnosis). The link is read out of the row text,
    # so it is not a hardcoded pair list. Exact id match, never substring: "A1"
    # is a substring of "A16" and "A17".
    ids = set(f.fid for f in picked)
    out = []
    for f in picked:
        out.append(f)
        for g in idx.facts:
            if g.fid in ids:
                continue
            m = re.search(r"\bFIXES\s+([A-D]\d{1,2})\b", g.headline, re.I)
            if m and m.group(1) == f.fid:
                out.append(g)
                ids.add(g.fid)
    return out


def render_fact(f):
    tag = "[%s]" % f.fid
    text = f.headline
    if f.verdict_word == "NARROWED":
        text = "NARROWED: " + text
    if f.qualifier:
        text += " — " + f.qualifier
    if f.supersedes:
        text += " [supersedes the retracted line in §%s]" % f.supersedes
    return "  %-7s %s" % (tag, text)


def emit(idx, args):
    out = []
    # THE COUNT MUST BE ABLE TO MOVE (C-208). It read "72 facts" through every
    # append of the night because it counted only §A-§D TABLE ROWS, and the
    # night's 20 rules were appendix paragraphs -- a population it did not
    # measure. A number that cannot change when the file grows is not a signal.
    # Kept rather than removed, and split, because the two halves are exactly the
    # two populations --selftest reconciles: the row half against the index's own
    # TALLY line, the rule half against the independent declared-id scan. So each
    # number here has a machine that reds when it is wrong.
    n_rows = len([f for f in idx.facts + idx.retracted_rows
                  if f.section in ("A", "B", "C", "D")])
    n_rules = len(idx.rule_ids_emitted())
    # C-627: the header and the footer counted DIFFERENT populations, and both
    # were "verified" -- the header against the tally and the declared-id scan
    # it was DERIVED from (circular), the footer against nothing. Two errors
    # that partly cancelled: +2 retracted rows counted as rows, and -4 appendix
    # DIRECTIVE paragraphs emitted under their own ids but called nothing at
    # all. Net residual 2, small enough to read as a rounding nit. The pair has
    # sat adjacent and disagreeing in the payload's own birth table since O-UI
    # 7 (74+128 printed beside 211 emit-safe; gap 9 then, 2 now -- so never a
    # constant that could be normalised away). Print all three categories and
    # make them sum to the footer.
    n_rows_emitsafe = len([f for f in idx.facts
                           if f.section in ("A", "B", "C", "D")])
    n_retr = n_rows - n_rows_emitsafe
    n_apx = len([f for f in idx.facts if f.section == "APX"])
    n_direct = n_apx - n_rules
    # PROVENANCE: report where THIS RUN read the index, never a ref scraped from
    # the index's own rows. The scraped value is "the ref a fact was measured on"
    # and printing it as the source made a fully-current block look ~30 commits
    # stale — a false alarm from the very header that exists to make the block
    # auditable. The blob id below is the load-bearing identifier; it is
    # content-addressed and cannot drift from what was actually read.
    ref = getattr(args, "ref", None) or "working tree"
    out.append("O-UI TRAPS — GENERATED, DO NOT RETYPE. "
               "source %s blob %s · %d rows (%d emit-safe + %d retracted) "
               "+ %d appendix (%d rules + %d directives) · %d emit-safe total "
               "· read from %s · regenerate: scripts/o-ui-brief-traps.py%s"
               % (RELPATH, idx.blob[:8], n_rows, n_rows_emitsafe, n_retr,
                  n_apx, n_rules, n_direct, n_rows_emitsafe + n_apx, ref,
                  "" if args.profile == "default" else " --" + args.profile))

    if args.show_retracted:
        out.append("  DO NOT REUSE — these lines are RETRACTED by the index:")
        for f in idx.retracted_rows:
            out.append("  %-7s %s  [%s]" % ("[%s]" % f.fid, f.headline,
                                            truncate(clean(f.verdict), 60)))
        for eid, _stars, line in idx.e_entries:
            out.append("  %-7s %s" % ("[§%s]" % eid, line))
        return out

    if args.all or args.section:
        groups = {}
        for f in idx.facts:
            if args.section and f.section != args.section:
                continue
            groups.setdefault(f.category, []).append(f)
        for c in CATEGORY_ORDER:
            if c not in groups:
                continue
            out.append("  -- %s --" % CATEGORY_LABEL[c])
            out.extend(render_fact(f) for f in groups[c])
    else:
        for f in select(idx, args):
            out.append(render_fact(f))

    for aid in PINNED_APPENDICES:
        if aid not in idx.appendices:
            die("pinned appendix %s is not in the index. Refusing to print a block "
                "without it: it is the only entry that stops a brief from demanding "
                "a false claim." % aid)
        lines = idx.appendix_lines(aid, 3)
        if not lines:
            die("pinned appendix %s parsed empty." % aid)
        out.append("  -- %s: %s --" % (aid, idx.appendices[aid]["title"]))
        for ln in lines:
            out.append("  %-7s %s" % ("[%s]" % aid, ln))

    excl = ", ".join(f.fid for f in idx.retracted_rows) or "none"
    # NO SILENT CAPS: a block that bounds its own coverage must say what it dropped,
    # or 16 rules read as "all of them". Added 2026-08-25 after the default was
    # measured at 16 of 156 with a footer that never mentioned the other 140.
    shown = len(set(re.findall(r"^\s*\[([A-Z]+-?\d*[a-z]?)\]", "\n".join(out), re.M)))
    total = len(set(f.fid for f in idx.facts))   # the emit-safe population, same set the selftest counts
    if shown < total:
        out.append("  SHOWING %d of %d emit-safe facts — the rest are NOT in this block; "
                   "run --all for the full set." % (shown, total))
    out.append("  excluded as RETRACTED: %s · §E do-not-reuse lines: %d "
               "· more: --all --build --readonly --section X --show-retracted"
               % (excl, len(idx.e_entries)))
    return out


# Structure only -- no fact text. Two rows that MUST parse and two that must
# not, plus a prose table that must stay silent.
PROBE_DOC = [
    "# synthetic fixture -- never emitted",                             # 1
    "",                                                                 # 2
    "## §A — SELFTEST FIXTURE",                                         # 3
    "",                                                                 # 4
    "| # | claim | evidence | denominator / scope | verdict |",         # 5
    "|---|---|---|---|---|",                                            # 6
    "| **A1** | **well formed.** | ev | scope | **CONFIRMED** |",       # 7  good
    "| **A2** | **wrong shape.** | ev | **CONFIRMED** |",               # 8  4 cells
    "| A3 | **id not bolded.** | ev | scope | **CONFIRMED** |",         # 9  bad id
    "",                                                                 # 10
    "| note | why |",                                                   # 11 prose hdr
    "|---|---|",                                                        # 12
    "| a prose row in a recognised section | must NOT warn |",          # 13 prose
    "",                                                                 # 14
    "## ★★★★★ APPENDED 2026-01-01 00:00 (C-999) — SELFTEST APPENDIX",   # 15
    "",                                                                 # 16
    "**G-98 — well formed appendix rule.** body.",                      # 17 good
    "",                                                                 # 18
    "**G-97 - hyphen where the shape needs an em dash.** body.",        # 19 bad
    "",                                                                 # 20
    "an ordinary appendix paragraph, declaring no rule at all.",        # 21 inert
]
# Line 19 is named TWICE on purpose -- by the near-miss shape check and, again,
# by the independent declared-but-not-emitted scan. Two detectors, one fault.
PROBE_BAD = [8, 9, 19]
PROBE_SILENT = [13, 21]
PROBE_GOOD = ["A1", "G-98"]


def probe_detector():
    """D4: a detector never shown firing is free, and RED = none is then
    meaningless. C-182: it must also be shown STAYING SILENT on a clean case,
    with exact matching. Both directions, same batch (D3), no fixture file."""
    ok = True
    probe = Index("\n".join(PROBE_DOC), "<selftest fixture>", "0" * 40)
    fired = sorted(set(pr[0] for pr in probe.problems))
    emitted = sorted(f.fid for f in probe.facts)
    if fired == PROBE_BAD:
        print("  PASS  malformed-row detector FIRED, on exactly lines %s of the fixture"
              % ", ".join(str(x) for x in PROBE_BAD))
    else:
        print("  FAIL  detector should fire on lines %s; fired on %s"
              % (PROBE_BAD, fired or "nothing"))
        ok = False
    noise = [ln for ln in PROBE_SILENT if ln in fired]
    if noise:
        print("  FAIL  detector cried wolf on line(s) %s -- legitimate prose"
              % ", ".join(str(x) for x in noise))
        ok = False
    else:
        print("  PASS  detector SILENT on the header, the separator, the prose "
              "table and an ordinary appendix paragraph (lines %s)"
              % ", ".join(str(x) for x in PROBE_SILENT))
    if emitted == PROBE_GOOD:
        print("  PASS  the well-formed row AND the well-formed appendix rule "
              "survived and are emit-capable (%s)" % ", ".join(PROBE_GOOD))
    else:
        print("  FAIL  well-formed row/rule did not survive: emitted %s"
              % (emitted or "nothing"))
        ok = False
    return ok


def selftest(idx):
    """§G-4 said nothing in the index is enforced by a test. These are the checks
    that can be derived FROM the index, so the index now checks itself."""
    ok = True
    counts = {}
    rows = [f for f in idx.facts + idx.retracted_rows if f.section in ("A", "B", "C")]
    for f in rows:
        counts[f.verdict_word] = counts.get(f.verdict_word, 0) + 1
    got = (len(rows), counts.get("CONFIRMED", 0), counts.get("NARROWED", 0),
           counts.get("RETRACTED", 0))
    print("parsed: %d rows in A/B/C -- %d CONFIRMED, %d NARROWED, %d RETRACTED" % got)
    if idx.tally:
        print("index TALLY line says:   %d rows -- %d CONFIRMED, %d NARROWED, "
              "%d RETRACTED" % idx.tally)
        if got == idx.tally:
            print("  PASS  parse reconciles with the index's own tally")
        else:
            print("  FAIL  parse disagrees with the index's own tally")
            ok = False
    else:
        print("  FAIL  no TALLY line found in the index")
        ok = False
    per = {}
    for f in idx.facts + idx.retracted_rows:
        per[f.section] = per.get(f.section, 0) + 1
    print("rows per section: " + ", ".join("%s=%d" % kv for kv in sorted(per.items())))
    emitted = set(f.fid for f in idx.facts)
    excluded = set(f.fid for f in idx.retracted_rows)
    overlap = emitted & excluded                      # EXACT ids, never substrings
    print("emit-safe ids: %d · retracted-and-excluded: %s"
          % (len(emitted), ", ".join(sorted(excluded)) or "none"))
    if overlap:
        print("  FAIL  a retracted id is also emit-safe: %s" % ", ".join(sorted(overlap)))
        ok = False
    else:
        print("  PASS  no retracted row is emit-safe (set intersection empty)")
    # C-208. The check the old --selftest could not make: it reconciled the parse
    # against the index's own TALLY line, and a rule the parser cannot SEE is a
    # rule the parser does not COUNT, so both sides moved together and the check
    # stayed green with 11 rules missing. This side is scanned out of the RAW TEXT
    # by a pattern the emitter does not use, so the two can disagree -- which is
    # the only way a comparison can fail.
    declared = idx.rule_ids_declared()
    emitted_rules = idx.rule_ids_emitted()
    missing = [f for f in declared if f not in emitted_rules]
    extra = sorted(emitted_rules - set(declared), key=rule_key)
    print("appendix rules DECLARED in the file: %d -- %s"
          % (len(declared), ", ".join(sorted(declared, key=rule_key)) or "none"))
    print("appendix rules EMITTED by the parser: %d -- %s"
          % (len(emitted_rules),
             ", ".join(sorted(emitted_rules, key=rule_key)) or "none"))
    if missing:
        print("  FAIL  %d rule(s) are in the file and NOT in the block: %s"
              % (len(missing),
                 ", ".join("%s (%s:%d)" % (f, idx.source, declared[f])
                           for f in missing)))
        ok = False
    if extra:
        print("  FAIL  emitted a rule the file does not declare: %s"
              % ", ".join(extra))
        ok = False
    if not missing and not extra:
        print("  PASS  declared and emitted rule ids are the SAME SET "
              "(difference empty in both directions)")
    if idx.problems:
        print("  FAIL  %d row(s)/rule(s) did not parse or did not reach the block "
              "-- these facts are NOT in it:" % len(idx.problems))
        for lineno, where, reason, _raw in idx.problems:
            print("          %s:%d: %s %s" % (idx.source, lineno, where, reason))
        ok = False
    else:
        print("  PASS  every row of every fact table parsed (0 silently dropped)")
    ok = probe_detector() and ok
    for aid in PINNED_APPENDICES:
        if aid in idx.appendices and idx.appendix_lines(aid, 3):
            print("  PASS  pinned appendix %s present and non-empty" % aid)
        else:
            print("  FAIL  pinned appendix %s missing or empty" % aid)
            ok = False

    # THE PRODUCTION PATH GETS AN ARM AT LAST (C-529). Traced under --selftest,
    # six functions never executed: die, emit, render_fact, report_problems,
    # select, supersedes. Four of those ARE the block -- select picks the facts,
    # render_fact draws each line, emit writes the header a session actually
    # reads. Every arm above this one stops at the PARSER.
    #
    # And emit's own comment claimed "each number here has a machine that reds
    # when it is wrong". Half true. The rule half (n_rules) is reconciled
    # against the independent declared-id scan. The row half was NOT: emit
    # counts sections A/B/C/D (74) and the only row arm reconciles A/B/C
    # against the TALLY line (60). DIFFERENT POPULATIONS -- so nothing could
    # red when emit's printed row count went wrong, which is C-208's exact
    # failure mode living on inside the function whose comment records C-208.
    emit_args = argparse.Namespace(
        index=None, ref=None, build=False, readonly=False, all=False,
        section=None, show_retracted=False, selftest=False, strict=False,
        maxlen=200, profile="default")
    block = emit(idx, emit_args)
    m = re.search(r"(\d+) rows \((\d+) emit-safe \+ (\d+) retracted\) "
                  r"\+ (\d+) appendix \((\d+) rules \+ (\d+) directives\) "
                  r"· (\d+) emit-safe total", block[0] if block else "")
    if not m:
        print("  FAIL  emit() produced no parsable header line")
        ok = False
    else:
        said_rows, said_rules = int(m.group(1)), int(m.group(5))
        said_emitsafe, said_retr = int(m.group(2)), int(m.group(3))
        said_apx = int(m.group(4))
        said_direct, said_total = int(m.group(6)), int(m.group(7))
        # Derived a DIFFERENT way than emit derives them: the tally-reconciled
        # A/B/C count plus a separately-measured D, and the declared-id scan.
        d_rows = len([f for f in idx.facts + idx.retracted_rows
                      if f.section == "D"])
        want_rows = (idx.tally[0] if idx.tally else len(rows)) + d_rows
        want_rules = len(idx.rule_ids_declared())
        if said_rows == want_rows and said_rules == want_rules:
            print("  PASS  emit()'s OWN header reconciles: %d rows "
                  "(= %d tallied A/B/C + %d D) and %d appendix rules"
                  % (said_rows, want_rows - d_rows, d_rows, said_rules))
        else:
            print("  FAIL  emit()'s header disagrees: says %d rows / %d rules, "
                  "independently derived %d / %d"
                  % (said_rows, said_rules, want_rows, want_rules))
            ok = False
        # NON-CIRCULAR, and the reason C-627 existed: every check above derives
        # its expectation from the SAME source emit() derived the number from,
        # so it can only catch a parse bug -- never a population mismatch. This
        # one compares two numbers built by different routes: the footer counts
        # the fact SET (len(set(fid))), the header sums per-section counts. They
        # agree only if no fid is duplicated and no fact carries a section
        # outside the five. Nothing checked this while the two disagreed.
        want_total = len(set(f.fid for f in idx.facts))
        if said_total == want_total == said_emitsafe + said_apx:
            print("  PASS  header total reconciles with the emit-safe fact SET "
                  "(%d = %d rows + %d appendix; set-counted %d), and "
                  "%d rows = %d emit-safe + %d retracted, %d appendix = "
                  "%d rules + %d directives"
                  % (said_total, said_emitsafe, said_apx, want_total,
                     said_rows, said_emitsafe, said_retr,
                     said_apx, said_rules, said_direct))
        else:
            print("  FAIL  header total %d != %d rows + %d appendix, or != "
                  "set-counted %d -- the header and the footer are over "
                  "different populations again"
                  % (said_total, said_emitsafe, said_apx, want_total))
            ok = False
        # THIS GUARD TOOK THREE FORMS AND MUTATION TESTING KILLED THE FIRST TWO.
        #   1. len(block) < 2      -- emit appends FOOTER lines after the fact
        #      loop, so the block never shrinks below 2 even at zero facts.
        #   2. count fact-SHAPED lines -- the pinned appendix renders with the
        #      same [id] shape, so an emptied select() loop still "drew 3".
        # Both were vacuous guards inside the arm written to end vacuous guards,
        # and both LOOKED right. Shape is not provenance. So bind the block to
        # select()'s OWN output, per fact: every fact select picks must appear.
        picked = select(idx, emit_args)
        absent = [f.fid for f in picked
                  if not any(("[%s]" % f.fid) in l for l in block)]
        if picked and not absent:
            print("  PASS  every one of select()'s %d picked facts reached the "
                  "block through render_fact()" % len(picked))
        elif not picked:
            print("  FAIL  select() picked nothing to render")
            ok = False
        else:
            print("  FAIL  %d fact(s) select() picked never reached the block: %s"
                  % (len(absent), ", ".join(absent[:6])))
            ok = False

    return ok


def main(argv=None):
    p = argparse.ArgumentParser(
        description="Print the O-UI traps block, generated from " + RELPATH,
        epilog="Embeds no traps. Every emitted word is read from the index.")
    p.add_argument("--index", help="path to the index (default: ../%s)" % RELPATH)
    p.add_argument("--ref", help="read the index from a git ref instead of disk "
                                 "(it is on lane/ui, not on main)")
    p.add_argument("--build", action="store_true", help="build/verification round")
    p.add_argument("--readonly", action="store_true", help="read-only round: no "
                                                           "build-freshness rules")
    p.add_argument("--all", action="store_true", help="every non-retracted fact")
    p.add_argument("--section", choices=list("ABCD"), help="one section, uncapped")
    p.add_argument("--show-retracted", action="store_true",
                   help="print the DO-NOT-REUSE list instead of the block")
    p.add_argument("--selftest", action="store_true")
    p.add_argument("--strict", action="store_true",
                   help="exit 3 if any table row failed to parse (stdout is "
                        "unaffected; the diagnostics are on stderr either way)")
    p.add_argument("--maxlen", type=int, default=200, help="max chars per fact line")
    args = p.parse_args(argv)
    if args.build and args.readonly:
        die("--build and --readonly are contradictory")
    args.profile = "build" if args.build else ("readonly" if args.readonly else "default")

    raw, source = load(args)
    idx = Index(raw.decode("utf-8"), source, blob_sha(raw), args.maxlen)
    if not idx.facts:
        die("parsed 0 facts from %s -- the index format changed; fix the parser "
            "rather than trusting an empty block." % source)
    if args.selftest:
        return 0 if selftest(idx) else 1
    print("\n".join(emit(idx, args)))
    return report_problems(idx, args.strict)


if __name__ == "__main__":
    sys.exit(main())
