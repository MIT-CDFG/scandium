#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# oint-e11-check.sh — OWNER: O-INT (integration desk).  v4.  READ-ONLY.
#
# WHY THIS EXISTS (entry 1057, 2026-09-17, freeze day). WORD 14 (d): "ONLY THEN
# E-11: bump Cargo.toml [workspace.package] version to 1.0.2,
# tools/app_version_census.py PASS, gate, and RC0". O-WATCH #885 MINOR-38 found
# that a bump which passes the census still leaves Cargo.lock stale. The census
# does not read Cargo.lock, by design. No cargo call in the RC0 build passes
# --locked, so the build rewrites the lock, and build_dmg.sh's release stage
# then STOPS on the dirty tree after the whole build. Reproduced by O-INT in
# scratch: lock c7c90e8248ac164e -> 3a7c868561db6894, 21 removed / 21 added,
# every one a `version = "0.1.0"` line.
#
# WHAT IT CERTIFIES about a COMMITTED bump (default: HEAD):
#   (1) ORDER    the countersigned cert sha (--through) is an ancestor of the
#                bump's parent, so build-cert5 merged before E-11's bump
#   (2) SCOPE    every path bump^..bump changes belongs to a class E-11 is
#                measured to touch, and the three version files are all there.
#                v2 demanded EXACTLY three files and would therefore have
#                REFUSED the complete bump that landed (O-WATCH #893: "a stale
#                portrait, not a defect"). The measured classes, from the bump
#                on main (ad4fbb98c, 112 files):
#                  version  Cargo.toml · crates/appname-py/pyproject.toml · Cargo.lock
#                  goldens  goldens/** — the packages the engine stamps with
#                           CARGO_PKG_VERSION, their MANIFEST.sha256 and .meta
#                  pin      tests/conformance/src/lib.rs (SELFTEST_REFERENCE_SHA256)
#                  notices  THIRD-PARTY-NOTICES.md (its line 7 is the lock's sha256)
#   (6) NOTICES  tools/gen_notices.py --check passes on the bump's tree, with a
#                control that plants the parent's notices file and must fail
#   (7) PIN      if goldens/selftest/reference.appname moved, the constant in
#                tests/conformance/src/lib.rs is its new sha256 (control: it must
#                NOT be the parent's)
#   (8) STAMP    NO package golden in the tree still carries the FROM version in
#                its manifest, and at the parent they all carried it. v3 gated
#                only strays and the three version files, so a bump that
#                regenerated NOTHING passed SCOPE and — with the notices correct
#                — passed the whole run (O-WATCH #896 §2, rig 2). Q-D84 (A) is
#                "three version files PLUS the regenerated package goldens", and
#                this is the half that was unchecked.
#   (3) LOCK     the Cargo.lock diff is exactly one `version` line per workspace
#                member, FROM -> TO, and nothing else
#   (4) CENSUS   tools/app_version_census.py --expect TO exits 0 on the bump's tree
#   (5) LOCKED   `cargo metadata --offline --locked` exits 0 on the bump's tree,
#                meaning the build will not rewrite the lock; a non-zero exit counts as
#                STALE only when cargo says so, otherwise the run is VOID (v2, MINOR-39)
# CONTROLS, each of which must fire or the verdict is VOID:
#   NEG census --expect FROM exits 1 on the same tree
#   NEG the bump's manifests with the PARENT's Cargo.lock fail --locked, and for
#       the lock reason ("cannot update the lock file"), not a resolution error
#
# It never writes to the repository. The bump's tree is read out with git
# archive into a temporary directory, and every check runs there.
# The bump itself is a hand step (entry 1057 gives the recipe).
#
# usage: oint-e11-check.sh --through <countersigned-cert-sha> [--bump <sha>] [--from 0.1.0] [--to 1.0.2]
# exit:  0 PASS · 1 a certified property fails · 2 REFUSED/VOID (bad input, or a control did not fire)
set -uo pipefail
THROUGH=""; BUMP="HEAD"; FROM="0.1.0"; TO="1.0.2"
while [ $# -gt 0 ]; do
    case "$1" in
        --through) THROUGH="${2:-}"; shift 2 ;;
        --bump)    BUMP="${2:-}"; shift 2 ;;
        --from)    FROM="${2:-}"; shift 2 ;;
        --to)      TO="${2:-}"; shift 2 ;;
        *) echo "oint-e11-check: VOID - unknown argument '$1'"; exit 2 ;;
    esac
done
TOP=$(git rev-parse --show-toplevel 2>/dev/null)
# `cd ""` exits 0 in bash, so an empty toplevel must be refused explicitly (O-WATCH #886 NIT).
[ -n "$TOP" ] && cd "$TOP" || { echo "oint-e11-check: VOID - not in a git tree"; exit 2; }
say() { echo "oint-e11-check: $*"; }
VERSION_FILES="Cargo.lock Cargo.toml crates/appname-py/pyproject.toml"
PIN_FILE="tests/conformance/src/lib.rs"
NOTICES_FILE="THIRD-PARTY-NOTICES.md"
SELFTEST_PKG="goldens/selftest/reference.appname"

[ -n "$THROUGH" ] || { say "REFUSED - --through <countersigned cert sha> is required"; exit 2; }
[ "$FROM" != "$TO" ] || { say "REFUSED - --from and --to are both $FROM"; exit 2; }
B=$(git rev-parse --verify --quiet "${BUMP}^{commit}") || { say "REFUSED - --bump '$BUMP' is not a commit"; exit 2; }
git rev-parse --verify --quiet "${THROUGH}^{commit}" >/dev/null || { say "REFUSED - --through '$THROUGH' is not a commit"; exit 2; }
command -v cargo >/dev/null || { say "VOID - cargo not on PATH"; exit 2; }
say "bump $(git rev-parse --short "$B") · through $(git rev-parse --short "$THROUGH") · $FROM -> $TO"

fail=0
# (1) ORDER
if git merge-base --is-ancestor "$THROUGH" "${B}^" 2>/dev/null; then say "OK   (1) ORDER  through is an ancestor of bump^"
else say "FAIL (1) ORDER  $(git rev-parse --short "$THROUGH") is not an ancestor of bump^ — build-cert5 did not merge before E-11's bump"; fail=1; fi

# (2) SCOPE — classify every changed path; nothing outside the measured classes
n_ver=0; n_gold=0; n_pin=0; n_not=0; stray=""
while IFS= read -r f; do
    case "$f" in
        Cargo.toml|Cargo.lock|crates/appname-py/pyproject.toml) n_ver=$((n_ver+1)) ;;
        goldens/*)          n_gold=$((n_gold+1)) ;;
        "$PIN_FILE")        n_pin=$((n_pin+1)) ;;
        "$NOTICES_FILE")    n_not=$((n_not+1)) ;;
        *) stray="$stray $f" ;;
    esac
done <<EOF
$(git diff --name-only "${B}^" "$B")
EOF
if [ -n "$stray" ]; then say "FAIL (2) SCOPE  path(s) outside E-11's measured classes:$stray"; fail=1;
elif [ "$n_ver" -ne 3 ]; then say "FAIL (2) SCOPE  $n_ver of the 3 version files changed"; fail=1;
elif [ "$n_gold" -eq 0 ]; then say "FAIL (2) SCOPE  the version files changed and NOT ONE golden did — Q-D84 (A) is three version files PLUS the regenerated package goldens"; fail=1;
else say "OK   (2) SCOPE  version 3 · goldens $n_gold · pin $n_pin · notices $n_not · stray 0"; fi

# the bump's tree, read out — every remaining check runs here, never in the repository
TMP=$(mktemp -d "${TMPDIR:-/tmp}/oint-e11-check.XXXXXX") || { say "VOID - mktemp failed"; exit 2; }
trap 'rm -rf "$TMP"' EXIT
git archive "$B" | tar -x -C "$TMP" || { say "VOID - git archive of $B failed"; exit 2; }
[ -f "$TMP/tools/app_version_census.py" ] || { say "VOID - tools/app_version_census.py absent at the bump"; exit 2; }

LOCK_T0=$(shasum -a 256 "$TMP/Cargo.lock" | cut -c1-16)
MEMBERS=$(cd "$TMP" && cargo metadata --offline --no-deps --format-version 1 2>/dev/null \
    | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["workspace_members"]))') \
    || { say "VOID - cannot read the workspace member count at the bump"; exit 2; }
[ "${MEMBERS:-0}" -gt 0 ] || { say "VOID - workspace member count is 0"; exit 2; }
# the member count must not have regenerated the lock that (5) is about to judge
[ "$(shasum -a 256 "$TMP/Cargo.lock" | cut -c1-16)" = "$LOCK_T0" ] || { say "VOID - reading the member count rewrote the archived Cargo.lock; (5) would judge a regenerated lock"; exit 2; }

# (3) LOCK
lock_minus=$(git diff -U0 "${B}^" "$B" -- Cargo.lock | /usr/bin/grep -E '^-[^-]' | sort | uniq -c | sed 's/^ *//')
lock_plus=$(git diff -U0 "${B}^" "$B" -- Cargo.lock | /usr/bin/grep -E '^\+[^+]' | sort | uniq -c | sed 's/^ *//')
if [ "$lock_minus" = "$MEMBERS -version = \"$FROM\"" ] && [ "$lock_plus" = "$MEMBERS +version = \"$TO\"" ]; then
    say "OK   (3) LOCK   exactly $MEMBERS member version lines $FROM -> $TO, nothing else"
else
    say "FAIL (3) LOCK   the Cargo.lock diff is not exactly the $MEMBERS member version lines:"
    printf '       %s\n' "${lock_minus:-(no removed lines)}" "${lock_plus:-(no added lines)}"; fail=1
fi

# (4) CENSUS, with its control
python3 "$TMP/tools/app_version_census.py" --expect "$FROM" >/dev/null 2>&1; neg=$?
[ "$neg" -eq 1 ] || { say "VOID - NEG census --expect $FROM exits $neg on the bump (want 1): the census cannot see a stale version here"; exit 2; }
python3 "$TMP/tools/app_version_census.py" --expect "$TO" > "$TMP/.census.out" 2>&1; rc=$?
if [ "$rc" -eq 0 ]; then say "OK   (4) CENSUS --expect $TO exit 0 (NEG --expect $FROM exit 1)"
else say "FAIL (4) CENSUS --expect $TO exit $rc:"; sed 's/^/       /' "$TMP/.census.out"; fail=1; fi

# (5) LOCKED, with its control
# v2 (O-WATCH #886 MINOR-39): exit 101 alone does not mean "stale lock". Cargo also exits 101 when it
# cannot resolve the workspace offline (measured with an empty CARGO_HOME: "no matching package
# named `blake3` found"), and that error MASKS a stale lock. Cargo's refusal names itself, so this
# check classifies by stderr: 0 = lock current · STALE_PHRASE = lock stale · anything else = VOID,
# not judged, with the error printed. The control must fail FOR THE LOCK REASON, or it is dead.
STALE_PHRASE="cannot update the lock file"
lockrun() {  # lockrun <label> -> sets LR_RC, LR_CLASS (OK|STALE|UNRESOLVED), LR_ERR (first error line)
    (cd "$TMP" && cargo metadata --offline --locked --format-version 1 >/dev/null 2>"$TMP/.lock.$1.err"); LR_RC=$?
    LR_ERR=$(/usr/bin/grep -m1 -E '^error' "$TMP/.lock.$1.err" | cut -c1-160)
    if [ "$LR_RC" -eq 0 ]; then LR_CLASS=OK
    elif /usr/bin/grep -qF -- "$STALE_PHRASE" "$TMP/.lock.$1.err"; then LR_CLASS=STALE
    else LR_CLASS=UNRESOLVED; fi
}
lockrun bump; rc=$LR_RC; cls=$LR_CLASS; err=$LR_ERR
git show "${B}^:Cargo.lock" > "$TMP/.parent.lock" || { say "VOID - cannot read bump^:Cargo.lock"; exit 2; }
cp "$TMP/Cargo.lock" "$TMP/.bump.lock"; cp "$TMP/.parent.lock" "$TMP/Cargo.lock"
lockrun parent; neg=$LR_RC; ncls=$LR_CLASS; nerr=$LR_ERR
cp "$TMP/.bump.lock" "$TMP/Cargo.lock"
case "$ncls" in
    STALE) ;;
    OK)    say "VOID - NEG the parent's lock under the bumped manifests passes --locked: the check cannot see a stale lock"; exit 2 ;;
    *)     say "VOID - NEG the parent's lock failed --locked (exit $neg) but NOT for the lock reason — cargo cannot resolve the workspace offline here, so no lock can be judged:"
           say "       $nerr"; exit 2 ;;
esac
case "$cls" in
    OK)    say "OK   (5) LOCKED cargo metadata --offline --locked exit 0 (NEG parent lock: exit $neg, \"$STALE_PHRASE\")" ;;
    STALE) say "FAIL (5) LOCKED --locked exit $rc, \"$STALE_PHRASE\" — the build would rewrite Cargo.lock and stage 9 would stop on the dirty tree"; fail=1 ;;
    *)     say "VOID - (5) --locked exit $rc NOT for the lock reason; the bump's lock was not judged:"
           say "       $err"; exit 2 ;;
esac

# (6) NOTICES, with a control that must fail
if [ -f "$TMP/tools/gen_notices.py" ]; then
    (cd "$TMP" && python3 tools/gen_notices.py --check >/dev/null 2>&1); rc=$?
    git show "${B}^:$NOTICES_FILE" > "$TMP/.parent.notices" 2>/dev/null || : 
    if [ -s "$TMP/.parent.notices" ]; then
        cp "$TMP/$NOTICES_FILE" "$TMP/.bump.notices"
        cp "$TMP/.parent.notices" "$TMP/$NOTICES_FILE"
        (cd "$TMP" && python3 tools/gen_notices.py --check >/dev/null 2>&1); neg=$?
        cp "$TMP/.bump.notices" "$TMP/$NOTICES_FILE"
    else neg=0; fi
    if [ "$neg" -eq 0 ]; then
        say "VOID - NEG the parent's $NOTICES_FILE passes --check on the bumped tree: this check cannot see a stale notices file"; exit 2
    fi
    [ "$rc" -eq 0 ] && say "OK   (6) NOTICES gen_notices.py --check exit 0 (NEG parent's notices exit $neg)" \
        || { say "FAIL (6) NOTICES gen_notices.py --check exit $rc — gate step notices would red"; fail=1; }
else
    say "VOID - tools/gen_notices.py absent at the bump"; exit 2
fi

# (7) PIN — only when the selftest reference moved
if git diff --name-only "${B}^" "$B" | /usr/bin/grep -qx "$SELFTEST_PKG"; then
    want=$(git show "$B:$SELFTEST_PKG" | shasum -a 256 | cut -d' ' -f1)
    parent=$(git show "${B}^:$SELFTEST_PKG" | shasum -a 256 | cut -d' ' -f1)
    pinned=$(git show "$B:$PIN_FILE" | /usr/bin/grep -A1 'SELFTEST_REFERENCE_SHA256' | /usr/bin/grep -oE '[0-9a-f]{64}' | head -1)
    if [ -z "$pinned" ]; then say "VOID - no SELFTEST_REFERENCE_SHA256 found in $PIN_FILE at the bump"; exit 2; fi
    if [ "$want" = "$parent" ]; then say "VOID - the selftest package's bytes did not change, so this check cannot discriminate"; exit 2; fi
    [ "$pinned" = "$want" ] && say "OK   (7) PIN    SELFTEST_REFERENCE_SHA256 is the new reference's sha256 (parent's differs)" \
        || { say "FAIL (7) PIN    pinned ${pinned} but the reference hashes to ${want}"; fail=1; }
else
    say "--   (7) PIN    $SELFTEST_PKG unchanged in this bump; nothing to check"
fi

# (8) STAMP — the property itself: no golden left holding the old version
stamp=$(git -C "$TOP" ls-tree -r --name-only "$B" goldens | /usr/bin/grep '\.appname$' \
    | python3 -c '
import io, json, subprocess, sys, zipfile
top, bump, parent, vfrom, vto = sys.argv[1:6]
paths = [l.strip() for l in sys.stdin if l.strip()]
def ver(rev, path):
    blob = subprocess.run(["git","-C",top,"show",rev+":"+path], capture_output=True).stdout
    try:
        return json.loads(zipfile.ZipFile(io.BytesIO(blob)).read("manifest.json"))["createdWith"]["version"]
    except Exception:
        return None
bad  = [p for p in paths if ver(bump, p) != vto]
base = [p for p in paths if ver(parent, p) == vfrom]
print("%d %d %d %s" % (len(paths), len(bad), len(base), " ".join(bad[:3])))
' "$TOP" "$B" "${B}^" "$FROM" "$TO")
set -- $stamp; total=$1; bad=$2; base=$3; shift 3
if [ "${total:-0}" -eq 0 ]; then say "VOID - no package goldens found in the tree at the bump; (8) cannot judge"; exit 2; fi
if [ "${base:-0}" -eq 0 ]; then say "VOID - NEG at the parent NOT ONE of $total package goldens carried $FROM: (8) cannot tell a regenerated tree from any other"; exit 2; fi
if [ "${bad:-0}" -eq 0 ]; then say "OK   (8) STAMP  all $total package goldens carry $TO (NEG at the parent: $base carried $FROM)"
else say "FAIL (8) STAMP  $bad of $total package goldens do not carry $TO, e.g. $*"; fail=1; fi

if [ "$fail" -ne 0 ]; then say "FAIL"; exit 1; fi
say "PASS lock $(git show "${B}^:Cargo.lock" | shasum -a 256 | cut -c1-16) -> $(git show "$B:Cargo.lock" | shasum -a 256 | cut -c1-16)"
exit 0
