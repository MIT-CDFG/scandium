#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# oint-decoys.sh — OWNER: O-INT (integration desk).  v1.
#
# WHY THIS EXISTS (entry 1006, 2026-09-15). I reported `CFBundleName "Appname"`
# as main's state. My source was a COMMENT, and when I looked for corroboration
# my own worktree's built plist agreed with it -- because that bundle was built
# 2026-08-28, before the rename. Two independent-looking confirmations, both
# photographs of the same dead moment. O-WATCH #843 then measured the scale:
# SIX of the EIGHT .app bundles under appname-wt/ answer with the old identity,
# including the one in int/, which is the worktree I board from every tick.
#
# THE HAZARD IS WORSE TOMORROW THAN TODAY. Every stale bundle currently answers
# `org.appname-placeholder.shell` for CFBundleIdentifier -- which is ALSO what
# the shipped bundle answers. So a reader who checks a decoy gets the right
# value BY ACCIDENT. The moment E-02 lands, the shipped bundle moves and the
# decoys do not: the same read starts returning the wrong answer while looking
# exactly as authoritative as it does today.
#
# A REMEMBERED DISCIPLINE FAILS ON THE TICK THAT MATTERS. This is apparatus, so
# it does not depend on my noticing. It READS ONLY -- it never deletes or moves
# a bundle, because build output is another desk's input and `target/` staleness
# is itself evidence other reviews are citing.
#
# Exit: 0 always. This is a PROMPT, never a verdict -- the same contract as
# oint-route.sh. A stale bundle is not a defect; reading one as state is.
set -u
REPO="${1:-.}"
cd "$REPO" || { echo "oint-decoys: VOID - repo unreadable"; exit 0; }
ROOT=$(cd .. && pwd)
HEAD_EPOCH=$(git log -1 --format=%ct 2>/dev/null || echo 0)
printf 'HEAD %s (%s)\n' "$(git rev-parse --short HEAD)" "$(git log -1 --format=%ad --date=format:'%Y-%m-%d %H:%M')"
found=0; stale=0
while IFS= read -r plist; do
    app=${plist%/Contents/Info.plist}
    found=$((found + 1))
    name=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleName' "$plist" 2>/dev/null || echo '(none)')
    ident=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$plist" 2>/dev/null || echo '(none)')
    mt=$(stat -f %m "$plist" 2>/dev/null || echo 0)
    age=$(( (HEAD_EPOCH - mt) / 86400 ))
    flag="  "
    if [ "$mt" -lt "$HEAD_EPOCH" ]; then stale=$((stale + 1)); flag="⛔"; fi
    printf '%s %-58s %-12s %-32s %sd older than HEAD\n' \
        "$flag" "${app#"$ROOT"/}" "$name" "$ident" "$age"
# ONLY a bundle's OWN top-level plist. v1 matched '*.app/Contents/*' and returned
# 143 -- it had descended into every vendored Qt framework's Resources/Info.plist
# (org.qt-project.*), which are third-party bundles inside our bundle and answer
# for Qt, not for us. #843 found EIGHT by matching the top level. Calibrated here
# before the number was used for anything.
done < <(find "$ROOT" -path '*.app/Contents/Info.plist' 2>/dev/null \
         | grep -v '\.framework/' | sort)
echo "oint-decoys: $found bundle(s), $stale older than HEAD."
echo "  A bundle older than HEAD cannot answer for HEAD. Read the SOURCE for state;"
echo "  read a bundle only to learn what that build produced. (entry 1006, #843)"
