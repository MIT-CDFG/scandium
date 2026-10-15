#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
# oint-route.sh — OWNER: O-INT (integration desk).  v7.
# READ-ONLY. Never mutates the tree. Exits 0 always: this instrument prints, it does not gate.
#
# WHY IT EXISTS (entry 907). O-BUILD's OB-27 was MAJOR and RELEASE-BLOCKING and sat
# four and a half hours on no surface the operator works from. `p_paste` could not
# catch it: it asks whether every OPEN Q-D is LISTED, and OB-27 had no Q-D at all.
# The routing step existed; nothing measured whether it ran.
#
# WHY v2 (OB-34, 2026-09-13, O-BUILD, MAJOR — a finding against v1, filed twenty
# minutes after v1 landed). v1 required a SEVERITY WORD in the heading. Measured
# across the three logs: 859 headings carry an id and that filter saw NINE. OB-27
# was visible ONLY because its author happened to write "(MAJOR, RELEASE-BLOCKING)";
# OB-28, seventy minutes later in their ordinary style, was invisible. THE
# INSTRUMENT BUILT BECAUSE OB-27 STRANDED WOULD NOT HAVE SEEN OB-27 WRITTEN IN THAT
# DESK'S USUAL HAND. Entry 908's "no second OB-27" was a claim about 9 headings, not
# 859. The defect was COVERAGE, and what hid it was a number printed without its
# denominator — so v2 prints the denominator on every line.
#
# AND THE LOGS DO NOT USE IDS THE SAME WAY, which v1 assumed. Measured:
#   OB-nn  FINDING ids ........ 23 distinct, matching O-BUILD's own accounting
#   C-nn   per-tick ENTRY ids .. 619 distinct; routing all of them is meaningless
#   G-nn   item ids ............ 56 distinct, same shape
# So this check APPLIES to O-BUILD and is NOT APPLICABLE to the other two. v2 prints
# UNMEASURED for those rather than a clean zero: an inapplicable check must never
# read as a passing one.
#
# POPULATION RULE: the id must OPEN the heading's title, so the heading IS that
# finding's own entry rather than a cross-reference to it. That rule reproduces
# O-BUILD's own count of 23 exactly, which is the only external check available.
#
# ⛔ AND "NOT ON THE SURFACE" INCLUDES **CLOSED** FINDINGS. v4's first run over the
# QA logs flagged G-2, G-3 and G-6; G-3 reads CRITICAL in its own heading and I was
# one step from reporting that a CRITICAL had never reached the operator. ALL THREE
# ARE CLOSED. Worse, the line that looked like the answer — "G-2 (MINOR) open; G-3
# and G-6 closed" — was itself a STALE SUMMARY, superseded later in the same log by
# "### G-2 — CLOSED". So the read is not "find the id" and not "find a status line":
# it is READ TO THE LATEST STATUS. A closed finding correctly has no Q-D.
#
# WHAT IT IS NOT: a verdict. "Not on the surface" means "has no Q-D", never "needs
# one" — most such findings are the filing desk's own to fix. A count cannot tell
# those apart; only reading can. A desk READS the rows.
#
# WHY v3 (OB-35, 2026-09-13, O-BUILD, MAJOR — a finding against v2, filed within the
# hour). v2's pattern had no trailing boundary, so OB-5a read as OB-5, OB-13a as
# OB-13, OB-10c as OB-10 — and that desk uses sub-lettered findings heavily.
# MEASURED, live: OB-10 has 3 mentions on the surface, OB-10b and OB-10c have ZERO,
# OB-10c IS a heading at O-BUILD-log.md:5233, and v2 did not flag OB-10 at all — it
# called it ROUTED. So OB-10c had never reached the operator and this instrument
# reported it routed because its heading contains the string "OB-10".
#
# v1 failed by NOT SEEING findings — a silence you could notice. v2 failed by seeing
# one finding and ANSWERING ABOUT ANOTHER: a green tick over a question never asked.
# That is the plausible-measurement family appearing INSIDE a routing instrument
# three hours after that family got its own payload section. Remedy: one character
# class, (OB-\d+[a-z]?).
#
# WHY v4 (2026-09-13, after G-17). v1-v3 scanned O-BUILD's, O-UI's and O-WATCH's
# logs. THE THREE QA LOGS WERE NOT IN THE POPULATION AT ALL — so the instrument
# built because OB-27 stranded would not have found G-17 either, and G-17 is a
# PRODUCT a11y finding six days old: a keyboard-only user cannot delete, group,
# ungroup, merge or reorder a layer, measured with a control on each of three
# doors. I found it by accident, chasing something else. Filed as Q-D72.
#
# THE CALIBRATION THAT MADE IT SAFE TO ADD THEM. G-nn is an OVERLOADED namespace:
# ROADMAP-1.0.md defines G-01..G-10 as TASK rows (G-02 gesture net growth, G-06
# the a11y sweep, G-08 the sitting), and the QA desks head their ticks with the
# task id they are working. A raw sweep returns eleven ids that are not eleven
# findings. So v4 DERIVES the task ids from the roadmap's own row table and
# excludes them — derived, never typed, so a new G-row cannot silently become a
# false finding. What remains are ids the QA desks minted themselves.
#
#   usage: bash scripts/instruments/oint-route.sh [repo]
set -uo pipefail
REPO="${1:-$(git rev-parse --show-toplevel)}"
cd "$REPO" || { echo "VOID - repo unreadable"; exit 0; }
python3 - <<'PYEOF'
import io, re, os
Q = io.open('reports/QUESTIONS-FOR-BILL.md', encoding='utf-8', errors='replace').read()
PRE = re.compile(r'^#+\s*(?:\d{4}-\d{2}-\d{2}[^—\-]*[—\-]\s*)?[★⛔✅\s]*')
FINDING_LOGS = {'O-BUILD-log.md': r'(OB-\d+[a-z]?)',
                'QA-q-gesture-log.md': r'(G-\d+[a-z]?)',
                # v6 (entry 997). These two read (G-\d+) from v4 until 2026-09-14 and
                # matched NOTHING: q-scenario adopted lane-qualified ids at its tick 15
                # ("All my findings are q-scenario/Q-N", because q-tutorial numbers its
                # own distinct Q-5 and a bare Q-N collides across lanes), and q-tutorial
                # did the same. Both printed "finding ids 0" in the CLEAN shape over
                # 9,711 and 5,776 lines. The desks announced the change in a heading;
                # the instrument was never re-read against them.
                'QA-q-scenario-log.md': r'(q-scenario/Q-\d+[a-z]?)',
                'QA-q-tutorial-log.md': r'(q-tutorial/Q-\d+[a-z]?)',
                # v5 (entry 959). The rights audit was OUTSIDE this population from v1
                # to v4 -- twenty findings, nineteen of which had never reached the
                # operator, in a log this instrument reported nothing about because it
                # never opened it. Entry 954 REPORTED that gap; this is the fix.
                'AUDIT-aud-rights-log.md': r'(R-\d+[a-z]?)'}
# ⛔ v5's OWN DEFECT, FOUND ON ITS FIRST RUN (entry 959). "ON THE SURFACE" means
# ONLY: this id occurs somewhere in QUESTIONS-FOR-BILL.md. It does NOT mean the
# finding was read, summarised, or its substance routed. On the run that added the
# rights log it printed 20/20 ON THE SURFACE -- because the previous tick I had
# LISTED the eleven ids I explicitly said I had NOT read. Naming an absence
# created the presence: the same mechanism Q-D67 records for the row-7 flag, and
# the same one entry 955 warned about ("routing an id I have not read is
# forwarding"). The metric cannot tell naming from routing. Until it can, read the
# ON-THE-SURFACE column as "not stranded by id", never as "handled".
# DERIVED exclusion: the roadmap's own G-nn task rows are not findings.
TASK_IDS = set(re.findall(r'^\|\s*(G-\d+)\s*\|',
               io.open('reports/ROADMAP-1.0.md', encoding='utf-8',
                       errors='replace').read(), re.M))
ENTRY_LOGS = {'O-UI-log.md': 'C-nn are per-tick ENTRY ids, not finding ids',
              'WATCH-log.md': 'G-nn are item ids, not finding ids'}
CONTROL = re.compile(r'\b(ZZ-\d{9})\b')
total = 0
for fn, pat in FINDING_LOGS.items():
    p = 'reports/' + fn
    if not os.path.exists(p):
        print("%-18s MISSING -- UNMEASURED, not clean" % fn); continue
    lines = io.open(p, encoding='utf-8', errors='replace').read().split('\n')
    own, ctl = {}, 0
    for i, l in enumerate(lines):
        if not l.startswith('#'):
            continue
        t = PRE.sub('', l)
        ctl += len(CONTROL.findall(t))
        # Anchored first (bare-id logs stay strict: a heading CITING another
        # desk's OB-nn must not become ours). A LANE-QUALIFIED id names its own
        # desk, so inside that desk's log it is unambiguous wherever it sits --
        # q-scenario heads findings "## 🔴 CRITICAL q-scenario/Q-1", which no
        # anchored match can ever see. (v6, entry 997.)
        m = re.match(r'^\(?' + pat, t) or (re.search(pat, t) if '/' in pat else None)
        if m and m.group(1) not in TASK_IDS:
            own.setdefault(m.group(1), (i + 1, t[:78]))
    if ctl:
        print("%-18s CONTROL DEAD (%d impossible ids) -- VOID" % (fn, ctl)); continue
    # ⛔ v6's REASON FOR EXISTING (entry 997). A declared pattern that matches
    # nothing is a PATTERN FAULT, not a clean desk -- but v4/v5 printed both as
    # "finding ids 0", the same shape as aud-rights' honest 20. This instrument
    # already says "UNMEASURED, not clean" for the two logs it excludes ON
    # PURPOSE; it owed the same honesty to the ones it silently failed to parse.
    # A desk with genuinely zero findings is reported UNMEASURED too: that is the
    # safe direction, and it is indistinguishable from a dead pattern without
    # reading the log -- which is what this line now tells a person to go do.
    if not own:
        body = len(re.findall(pat, '\n'.join(lines)))
        print("%-18s %s -- UNMEASURED, not clean (pattern %s matches %d in body, 0 in headings)"
              % (fn, "SHAPE MISMATCH" if body else "PATTERN DEAD", pat, body))
        continue
    un = sorted(((k, v) for k, v in own.items()
                 if not re.search(r'\b' + re.escape(k) + r'\b', Q)), key=lambda t: -t[1][0])
    total += len(un)
    print("%-18s finding ids %-4d ON THE SURFACE %-4d NOT ON IT %d" %
          (fn, len(own), len(own) - len(un), len(un)))
    for k, (ln, t) in un:
        print("     %-9s :%-7d %s" % (k, ln, t))
for fn, why in ENTRY_LOGS.items():
    print("%-18s UNMEASURED -- %s" % (fn, why))
# ⛔ v7 (entry 1014). The footer warned in ONE direction only -- that ON THE SURFACE
# does not mean handled -- and a reader (me, entry 1013) took the reciprocal for
# granted: that NOT ON IT means OPEN. It does not. Measured on O-BUILD's 45: at
# least 12 carry explicit closure language in their own log, OB-37 among them --
# labelled RELEASE-BLOCKING and long discharged. I was one sentence from telling
# the operator a closed release-blocker was sitting unrouted. Both directions now
# print, because the asymmetric warning is what let the wrong half be assumed.
print("  ON THE SURFACE = the id OCCURS in QUESTIONS-FOR-BILL.md. It does NOT mean read,\n"
      "  summarised, or routed -- naming an id satisfies this test (entry 959).")
print("  NOT ON IT = the id never reached that sheet. It does NOT mean the finding is\n"
      "  OPEN -- a desk may have discharged it in its own log. This sweep cannot tell\n"
      "  open from closed, and never could. (v7, entry 1014)")
print("ROUTE not-on-surface %d of the finding-id population above. The other two logs are "
      "UNMEASURED, not clean. A PROMPT to read, never a verdict." % total)
PYEOF
