#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# PROSE-vs-INSTRUMENT CROSS-CHECK (C-351).
#
# O-WATCH 3's observation, made about a real miss of mine at C-347: an entry
# that discusses `crates/appname-core/src/engine/registry_gen.rs` at length
# while its own instrument prints four zeros for that same window is
# internally inconsistent ON ITS FACE -- and the author has both halves in
# front of them at write time. "Prose-vs-instrument disagreement is a free
# cross-check we all currently discard."
#
# This stops discarding it. Given a window and a ledger entry, it reports:
#
#   PROSE-ONLY  a repo path the entry names that did NOT change in the window
#               -- the entry is describing something outside its own window,
#               which may be fine (a citation) or may be the C-339 class
#               (a claim about the wrong object). Reported, never failed on.
#   WINDOW-ONLY a path that changed in the window and the entry never names
#               -- the C-347 class. THIS is the one that reds.
#
# It is deliberately noisy in the PROSE-ONLY direction and strict in the
# other: an unmentioned change is the failure that produced four confident
# zeros; a mentioned non-change is usually a reference.
#
# rc 0 OK · rc 1 WINDOW-ONLY paths exist · rc 2 UNMEASURED (bad args/refs)

set -u
SINCE=""; UNTIL="main"; ENTRY=""
while [ $# -gt 0 ]; do
    case "$1" in
        --since) SINCE="${2:-}"; shift 2 ;;
        --until) UNTIL="${2:-}"; shift 2 ;;
        --entry) ENTRY="${2:-}"; shift 2 ;;
        *) echo "usage: $0 --since <ref> [--until <ref>] --entry <C-nnn>" >&2; exit 2 ;;
    esac
done
[ -n "$SINCE" ] && [ -n "$ENTRY" ] || { echo "prose-check: UNMEASURED-BAD-ARGS(--since and --entry are required)" >&2; exit 2; }
for r in "$SINCE" "$UNTIL"; do
    git rev-parse --verify --quiet "${r}^{commit}" >/dev/null || { echo "prose-check: UNMEASURED-BAD-REF($r)" >&2; exit 2; }
done
git merge-base --is-ancestor "$SINCE" "$UNTIL" 2>/dev/null || { echo "prose-check: UNMEASURED-ANCHOR-NOT-ANCESTOR($SINCE not on ${UNTIL}'s history)" >&2; exit 2; }

LOG=reports/O-UI-log.md
[ -r "$LOG" ] || { echo "prose-check: UNMEASURED-NO-LOG($LOG)" >&2; exit 2; }

git diff --name-only "${SINCE}..${UNTIL}" > /tmp/pc-window.$$ ; WRC=$?
[ "$WRC" -eq 0 ] || { echo "prose-check: UNMEASURED-DIFF-FAILED(rc=$WRC)" >&2; rm -f /tmp/pc-window.$$; exit 2; }

python3 - "$ENTRY" "$LOG" /tmp/pc-window.$$ "$UNTIL" "$SINCE" <<'PY'
import re, sys
entry, logpath, winpath = sys.argv[1], sys.argv[2], sys.argv[3]
text = open(logpath, errors="replace").read()
start = text.find(f"## {entry} —")
if start < 0:
    print(f"prose-check: UNMEASURED-NO-ENTRY({entry} not found in the ledger)", file=sys.stderr)
    sys.exit(2)
nxt = text.find("\n## C-", start + 1)
body = text[start: nxt if nxt > 0 else len(text)]

# C-352: THE AIMING GUARD. This checker takes its window on the command line,
# so it can be pointed at the wrong entry/window pair and will answer
# confidently -- I did exactly that one tick after writing it, aiming C-351 at
# C-350's window and reading the red as a finding. Every STATE block in this
# ledger names the main sha it was measured against; if the entry names one and
# it is not the endpoint given, the pairing is wrong and there is no answer to
# give. Same shape as o-ui-tick's anchor-not-ancestor refusal (C-344).
import subprocess
# Preferred, exact: an entry that DECLARES its own window. The main-sha check
# below is the fallback and it is WEAKER -- it cannot separate two entries
# written at the same main, which is exactly the mis-aim it failed to catch at
# C-352 (C-350 and C-351 both name main 07438ad5). Entries from C-352 on carry
# a WINDOW line so the pairing is checkable rather than inferable.
decl_win = re.search(r"WINDOW:\s*([0-9a-f]{7,40})\.\.([0-9a-f]{7,40})", body)
if decl_win:
    def rp(r):
        return subprocess.run(["git","rev-parse",r],capture_output=True,text=True).stdout.strip()
    if rp(decl_win.group(1)) != rp(sys.argv[5]) or rp(decl_win.group(2)) != rp(sys.argv[4]):
        print(f"prose-check: UNMEASURED-WRONG-PAIRING({entry} DECLARES window "
              f"{decl_win.group(1)}..{decl_win.group(2)}, not the one given)", file=sys.stderr)
        sys.exit(2)
declared = [] if decl_win else re.findall(r"`main`\s+`([0-9a-f]{7,40})`", body)
if declared:
    want = subprocess.run(["git","rev-parse",sys.argv[4]],capture_output=True,text=True).stdout.strip()
    ok = False
    for d in declared:
        got = subprocess.run(["git","rev-parse",d],capture_output=True,text=True).stdout.strip()
        if got and got == want:
            ok = True
            break
    if not ok:
        print(f"prose-check: UNMEASURED-WRONG-PAIRING({entry}'s STATE block names main "
              f"{'/'.join(declared)}, not the endpoint given)", file=sys.stderr)
        sys.exit(2)

window = {l.strip() for l in open(winpath) if l.strip()}
# A path is "named" if the entry mentions the full path or its basename.
named = set()
for p in window:
    base = p.rsplit("/", 1)[-1]
    if p in body or base in body:
        named.add(p)
missed = sorted(window - named)
print(f"prose-check: entry {entry} · window {len(window)} changed file(s) · named {len(named)} · unnamed {len(missed)}")
for p in missed:
    print(f"  WINDOW-ONLY (changed, and the entry never names it): {p}")
sys.exit(1 if missed else 0)
PY
RC=$?
rm -f /tmp/pc-window.$$
[ "$RC" -eq 0 ] && echo "prose-check: OK (every changed path in the window is named in the entry)"
exit "$RC"
