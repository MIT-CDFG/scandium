#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
# oint-roadmap-live.sh — OWNER: O-INT (integration desk).  v4.
# READ-ONLY. Never mutates the tree. Exits 0 always: this instrument prints, it does not gate.
#
# WHY IT EXISTS (entry 940, 2026-09-14). ROADMAP-1.0.md §10.5: "O-INT lands amendments
# as dated blocks ... never by editing the original text." So row cells go stale BY LAW,
# and that is the price of their guarantee that they never move under a desk mid-read.
# In one night four desks quoted a stale cell as current: OB-16 twice, O-WATCH's §2.13b
# ("the Windows zip is due 22 hours from now" — the amendment says 09-21, seven days out),
# and this desk's withdrawn Q-D73, which read the staleness itself as a defect and
# proposed the one act §10.5 forbids.
#
# O-WATCH's §2.13c proposed the derivation and it does not discriminate. Verbatim:
#   awk '/^### AMENDMENT/{a=$0} /<ID>/{print a}' ROADMAP-1.0.md
#   "Empty means the cell is live; non-empty means it is not."
# MEASURED: every id also appears in the ORIGINAL BODY, which precedes the first
# AMENDMENT heading, where `a` is still empty -- so those matches print a BLANK line.
#   D-11 -> 12 lines (11 blank, 1 real, re-based)   F-09 -> 2 lines (both blank, LIVE)
#   H-01 -> 2 lines (both blank, LIVE)              ZZ-999 -> 0 lines (not a row)
# Non-empty for the live cells and the re-based ones alike: the answer does not depend
# on the truth. That is the payload's own family -- never ask a command something its
# shape forces it to answer one way.
#
# THE FIX IS TWO NUMBERS, NOT ONE:
#   (1) does the id EXIST in the file -- else "0 amendments" means "not a row", not "live"
#   (2) how many DISTINCT amendment blocks contain it
# ZZ-999 is carried as a live negative control on every run so a silent match failure
# cannot read as a clean answer.
#
# v2 (entry 941). "CELL LIVE" meant "no amendment has LANDED", and a desk read that as
# "no ruling exists". E-06 is the trap: its cell says 09-14, no amendment touches it, and
# WORD 9 (3) ruled "its date moves to 09-16 18:00 ET by Amendment 10 (paper ...)" -- a
# vehicle the operator still owes. O-BUILD carried "RISK TO A DATE: E-06 (09-14)" six
# times on a date the operator had already moved. So v2 also counts the id across the
# words channels and says so. A word is not an amendment and does not move the row --
# but a desk about to quote the cell needs to know a ruling is in flight.
#
# v3 (entry 942). v1/v2 hard-coded ZZ-999 as the live negative control -- and then
# published it, in this file and in ledger entries 940 and 941. O-WATCH's #777 records
# the same token dying in their report: they published it as a control in SS2.13d and it
# now returns 2 there. A NEGATIVE CONTROL TOKEN IS CHOSEN AT USE AND NEVER WRITTEN DOWN;
# writing it down is precisely what destroys it. v3 mints one per run and PROVES it is
# absent from both corpora before trusting it -- if the minted token is not absent, the
# run says so instead of printing a reassuring zero.
#
# v4 (entry 962). v1-v3 printed "RE-BASED -- n amendment(s) GOVERN". That is STRONGER
# than what this instrument measures, which is only: n amendment blocks CONTAIN this id.
# E-10 is the case that exposed it -- AMENDMENT 9 names E-10 solely to say "E-10's
# release page is UNCHANGED", and the tool labelled it RE-BASED. I was one step from
# telling O-BUILD they had quoted a superseded row when they had quoted a live one.
# Entry 941 stated this limit correctly IN A LEDGER ENTRY nobody reads at runtime; the
# label now states it where it is used.
#
# USAGE: oint-roadmap-live.sh <ID> [ID...]      e.g. oint-roadmap-live.sh D-11 B-10

set -u
F="${ROADMAP:-reports/ROADMAP-1.0.md}"
[ -f "$F" ] || { echo "oint-roadmap-live: VOID - no $F (run from the worktree root)"; exit 0; }

amendments_for() {
    awk -v pat="$1" '/^### AMENDMENT /{a=$0} $0~pat && a!=""{print a}' "$F" | sort -u
}

WORDS="${WORDS_DIR:-../ev-scratch-local/words}"

words_hits() {
    [ -d "$WORDS" ] || { echo "0 0"; return; }
    _n=0; _c=0
    for _f in "$WORDS"/*.md; do
        [ -f "$_f" ] || continue
        _k=$(/usr/bin/grep -ac -- "$1" "$_f" 2>/dev/null || true)
        [ -n "$_k" ] || _k=0
        if [ "$_k" -gt 0 ]; then _n=$((_n+_k)); _c=$((_c+1)); fi
    done
    echo "$_n $_c"
}

report_one() {
    _id="$1"
    _ex=$(/usr/bin/grep -ac -- "$_id" "$F")
    _am=$(amendments_for "$_id" | /usr/bin/grep -ac '^### AMENDMENT ')
    if [ "$_ex" -eq 0 ]; then
        printf '  %-8s NOT A ROW (0 mentions) -- the question is malformed, not answered\n' "$_id"
    elif [ "$_am" -eq 0 ]; then
        printf '  %-8s CELL LIVE (%s mentions, 0 amendments)\n' "$_id" "$_ex"
    else
        printf '  %-8s NAMED BY %s AMENDMENT(S) -- they MAY OR MAY NOT move the cell; read them:\n' "$_id" "$_am"
        amendments_for "$_id" | sed 's/^/             /' | cut -c1-104
    fi
    if [ "$_ex" -gt 0 ]; then
        set -- $(words_hits "$_id")
        if [ "${1:-0}" -gt 0 ]; then
            printf '             ⚠ %s operator-word mention(s) in %s channel(s) -- a RULING may be in flight\n' "$1" "$2"
            printf '               that has not landed as an amendment. A word does not move the row; read it.\n'
        fi
    fi
}

echo "=== O-INT ROADMAP-LIVE  file=$F  (§10.5: rows are never edited; amendments govern) ==="
[ "$#" -gt 0 ] || { echo "  (no ids given)"; }
for id in "$@"; do report_one "$id"; done

echo "  --- controls ---"
# NEG token minted at use. Never hard-code one: a published control is a dead control.
NEG=""
_n=0
while [ "$_n" -lt 20 ]; do
    _cand="QZ-$(( (RANDOM % 900) + 100 ))$(( (RANDOM % 900) + 100 ))"
    if [ "$(/usr/bin/grep -ac -- "$_cand" "$F")" -eq 0 ]; then NEG="$_cand"; break; fi
    _n=$((_n+1))
done
if [ -z "$NEG" ]; then
    echo "  ★ COULD NOT MINT AN ABSENT NEG TOKEN IN 20 TRIES -- controls UNMEASURED."
    NEG="QZ-000000"
fi
_ctl=$(amendments_for "$NEG" | /usr/bin/grep -ac '^### AMENDMENT ')
_pos=$(amendments_for 'D-11'  | /usr/bin/grep -ac '^### AMENDMENT ')
_wpos=$(words_hits 'E-06' | cut -d' ' -f1)
_wneg=$(words_hits 'F-09' | cut -d' ' -f1)
printf '  NEG %s (minted at use, absent from the file) amendments %s (want 0)\n' "$NEG" "$_ctl"
printf '  POS D-11 amendments %s (want >=1)\n' "$_pos"
printf '  words arm: POS E-06 %s (want >0)   NEG F-09 %s (want 0)\n' "$_wpos" "$_wneg"
if [ "$_wpos" -lt 1 ] || [ "$_wneg" -ne 0 ]; then
    echo "  ★ WORDS ARM CONTROLS FAILED -- treat every words line above as UNMEASURED."
fi
if [ "$_ctl" -ne 0 ] || [ "$_pos" -lt 1 ]; then
    echo "  ★ CONTROLS FAILED -- this run proves nothing about the ids above."
fi
exit 0
