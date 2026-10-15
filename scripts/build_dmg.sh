#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
#
# dmg build + notarization path — plan §2.4 "Packaging v1" row.
# Spec: D-43 naming (generic "Appname" bundle, no trade dress); qt6 record's
# LGPL obligations (Qt bundled as REPLACEABLE dynamic frameworks via
# macdeployqt — relink-ability preserved, never statically absorbed);
# i18n install layout per i18n/README.md (exe-adjacent i18n/*.json).
#
# P5-D1 handoff (O-UI lane log, 2026-08-13): the pipeline consumes the
# SHELL'S OWN bundle (-DAPPNAME_BUNDLE=ON — D-43 placeholder identity
# org.appname-placeholder.shell, CFBundleName "Appname", the checked-in
# placeholder icns, i18n staged Contents/MacOS/i18n) instead of
# hand-assembling a divergent Info.plist here. The icns provenance is
# verified per the P5 reviewer's recipe BEFORE the bundle is accepted:
# regenerate via exactly `make_placeholder_icon.py <iconset>` +
# `iconutil -c icns`, two-run determinism cmp, then cmp against the
# checked-in shell/packaging/appname-placeholder.icns. Byte-identity is
# same-environment (zlib/iconutil versions shape compressed bytes; PIXELS
# are deterministic everywhere — P5 record): a differing environment can
# acknowledge with APPNAME_ICNS_ALLOW_ENV_DIFF=1 (stage reports SKIP, the
# committed reviewer-verified artifact stays authoritative); an
# unacknowledged mismatch is a STOP. iconutil unavailable/blocked (it
# exits 126 under sandboxed build graphs — the recorded reason the
# artifact is committed) reports SKIP with the reason.
#
# DRY-RUN BOUNDARY, documented honestly: this machine holds NO signing
# identity and NO notary credentials. Stages echo "DMG <stage> OK|SKIP|STOP".
#   * codesign: ad-hoc (-s -) unless APPNAME_SIGN_IDENTITY is set.
#   * notarization: STOPS before submission unless APPNAME_NOTARY_PROFILE is
#     set (a keychain profile created with `xcrun notarytool store-credentials`).
#     The real sequence past the boundary is:
#         xcrun notarytool submit "$DMG" --keychain-profile "$PROFILE" --wait
#         xcrun stapler staple "$DMG"
# Exit 0 = all reachable stages completed to the documented boundary.
# Exit 1 = a stage that should work on this machine failed.

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

command -v cargo >/dev/null 2>&1 || export PATH="$HOME/.cargo/bin:$PATH"
QT_BIN="${APPNAME_QT_BIN:-/opt/homebrew/opt/qt/bin}"
STAGE_DIR="target/dmg-stage"
# Build logs live OUTSIDE the staged folder (finding B-9). Stage 7 images the
# WHOLE of $STAGE_DIR (`hdiutil create -srcfolder`), so anything written there
# ships to every recipient. macdeployqt.log did exactly that from the original
# packaging commit onward, carrying 39 absolute /Users/... paths into what would
# have been a public dmg; codesign.err joined it when stage 6 was made to fail
# loudly. Nothing that is not part of the product goes under $STAGE_DIR.
BUILD_LOG_DIR="target/dmg-logs"
mkdir -p "$BUILD_LOG_DIR"
# BUILD STATE, for the desks that read the staged bundle (finding OB-29).
# $STAGE_DIR is a SHARED READ SURFACE -- O-INT and O-WATCH census it -- and until
# now nothing announced that it was being rewritten. In two hours that voided one
# peer's sweep outright and made another infer settledness by watching `ps` for a
# macdeployqt process. Three readers, three invented liveness tests, one of them
# wrong; the surface owner should simply say.
#
# NOT under $STAGE_DIR: B-9's arm asserts the staging directory holds EXACTLY
# Appname.app, and that arm exists because this script once imaged two build logs
# into a dmg. It goes beside the logs.
#
# The trap is the point. A STOP must announce settledness as loudly as a success,
# or the marker becomes a lie exactly when a reader most needs it -- a stage that
# fails without saying so is the B-8 shape, and a state file that never clears is
# the same defect wearing a different hat.
BUILD_STATE="$BUILD_LOG_DIR/build-state"
# OB-37: the census verifies the writer by PID, not by a flag anyone may pass.
# Exported so walled_symbols_release.sh inherits it as a child of this script.
export APPNAME_BUILD_PID=$$
printf 'RUNNING pid=%s since=%s stage_dir=%s\n' "$$" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$STAGE_DIR" > "$BUILD_STATE"
# The EXIT trap ALONE is not enough, and the test that proved it is worth keeping:
# killed with SIGTERM mid-run, bash ran the EXIT trap with $? == 0 and the marker
# recorded "SETTLED exit=0" over a TRUNCATED bundle -- the marker telling a peer that
# a half-written artifact is a finished one, which is the exact lie it exists to
# prevent. The signal traps re-raise as 128+signo so EXIT sees the real status.
trap 'rc=$?; printf "SETTLED at=%s exit=%s stage_dir=%s\n" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$rc" "$STAGE_DIR" > "$BUILD_STATE"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP
# OB-38: WRITTEN HERE, NOT AT STAGE 4. It used to be written beside the stage
# directory creation, which is line ~250 -- after `cargo build --release
# --workspace` and the cmake shell build, roughly 20 minutes of a 25-minute run.
# For all of that window there was no RUNNING marker, the PREVIOUS run's bundle
# was still on disk (rm -rf "$STAGE_DIR" is also stage 4), and the state file
# still carried the previous run's "SETTLED exit=0" -- so a peer asking whether
# it was safe to census was told YES about a bundle about to be deleted. A false
# SETTLED is the exact thing the signal traps above exist to prevent; writing the
# marker late reintroduced it by omission rather than by value.
# OPERATOR WORD 9 (4) + WORD 10 (2026-09-14 20:01 ET): OD-1 is decided, and ONE
# PRODUCT_NAME drives every name site -- dmg, volume, staged .app, tarball name
# and prefix, the release-name arm, and the CMake bundle name, which reads the
# same file. It lived in eight literals; when the name moved it would have moved
# in eight places, and the ninth (CMake) would have been missed.
PRODUCT_NAME="$(cat packaging/PRODUCT_NAME 2>/dev/null | head -1 | tr -d '[:space:]')"
[ -n "$PRODUCT_NAME" ] \
    || { say "product STOP(packaging/PRODUCT_NAME is missing or empty — the artifact name has no source)"; exit 1; }
APP="$STAGE_DIR/${PRODUCT_NAME}.app"
VERSION="$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)"/\1/')"
# AMENDMENT 10 (e), operator 2026-09-14: E-10's artifact name is the PATTERN
# <Name>-<version>-macOS-arm64.dmg, <version> from Cargo.toml [workspace.package]
# (the single source of truth, which this line already derives from -- measured:
# Cargo.toml carries exactly ONE ^version line and it is under [workspace.package]).
# <Name> stays the placeholder until F-01. This closes OB-64.
DMG="target/${PRODUCT_NAME}-${VERSION}-macOS-arm64.dmg"

say() { echo "DMG $1"; }

# license/notice manifest (aud-rights R-01) ---------------------------------
# The texts every recipient of the dmg must receive. STAGED into
# Appname.app/Contents/Resources/licenses/ at step 4b and PRESENCE-GATED both
# here (source, fail-fast) and there (staged into the signed bundle) the same
# fail-loud way icns and i18n are gated — a build that would image the artifact
# without a notice STOPS; it never silently ships un-noticed again (the R-01
# regression). This is the MECHANICAL text carriage only: the About/credits UI
# surface is the separate O-UI item, and WHAT each license legally requires in
# the delivered bundle is the open OD-7 counsel question — no legal
# determination is made here. Staging is deliberately over-inclusive (carry the
# texts the repo already holds; STOP on any we intend to ship but cannot find)
# so nothing ships un-noticed by omission.
NOTICE_FILES=(
    "LICENSE"                 # GPL-3.0-only — the engine/shell license
    "LICENSE-SDK"             # the SDK license
    "THIRD-PARTY-NOTICES.md"  # generated aggregate: carries the four recorded
                              # attribution duties (lucide/lensfun/libmypaint/
                              # rgb2spec) and the required FreeType acknowledgment
)
NOTICE_DIRS=(
    "LICENSES"                # the SPDX license texts (must include the Qt
                              # LGPL-3.0-only text — see QT_LGPL_TEXT gate below)
)
# Qt is dynamically bundled LGPL-3.0-only (deps/qt6.yaml elected arm). Its full
# license text is a REQUIRED carriage obligation (THIRD-PARTY-NOTICES.md qt6
# record) and is NOT in the repo as of this lane. It must NOT be invented: the
# build STOPS, loudly, until the verbatim upstream text is added at this SPDX
# path. (FLAGGED by O-BUILD — the auditor's suggested direction names this text
# explicitly.)
# OPERATOR WORD 10 (1), 2026-09-14: carriage texts -- licence texts carried ONLY to
# give notice of a SHIPPED component and declared by no repo file -- live beside the
# packaging that ships them, NOT in LICENSES/. LICENSES/ is REUSE's registry of what
# repo files declare, and a text nothing declares reads there as an "unused licence"
# and reds `reuse lint` (F-06, never-cut). Moving it clears that. The obligation is
# unchanged: the text still ships, staged explicitly below.
QT_LGPL_TEXT="packaging/macos/notices/LGPL-3.0-only.txt"

# 0. license/notice source pre-flight (fail fast, before the costly builds) -
for f in "${NOTICE_FILES[@]}"; do
    [ -f "$f" ] || { say "notices STOP(required notice text '$f' missing from the repo — aud-rights R-01: the dmg must not ship without it)"; exit 1; }
done
for d in "${NOTICE_DIRS[@]}"; do
    [ -d "$d" ] || { say "notices STOP(required notice dir '$d/' missing from the repo — aud-rights R-01)"; exit 1; }
done
if [ ! -f "$QT_LGPL_TEXT" ]; then
    say "notices STOP(Qt LGPL-3.0-only license text absent at '$QT_LGPL_TEXT' — Qt is bundled LGPL-3.0-only (deps/qt6.yaml) and its full text is a REQUIRED carriage obligation (THIRD-PARTY-NOTICES.md qt6 record). Not in-repo; do NOT invent it — add the verbatim upstream GNU LGPL v3 text at this SPDX path, then re-run. FLAGGED, aud-rights R-01)"
    exit 1
fi
say "notices OK(source texts present: ${NOTICE_FILES[*]} ${NOTICE_DIRS[*]}/ incl. $QT_LGPL_TEXT)"

# 0b. entitlements parse pre-flight (OB-7b; O-INT routed the arm here, O-WATCH
# #501 §6) ----------------------------------------------------------------
# BOTH entitlement files must PARSE before anything is built. codesign's AMFI
# parser rejects XML that `plutil -lint` accepts -- that is finding B-7, where
# two consecutive hyphens inside a comment made the no-jit file unparseable and
# the run died at stage 6, AFTER the workspace and the shell were built, having
# produced no signature, no dmg and no notarization.
#
# plistlib sees exactly what codesign sees, at the same line and column, in
# milliseconds and with no signing identity. B-7's own comment claimed a plist
# check could not catch this and that codesign was the only real test; both
# claims were false and both misled a peer desk. Measured on the broken form:
#   plutil -lint OK (blind) · xml.etree ERROR 10:50 · plistlib ERROR 10:50
#   · codesign AMFIUnserializeXML error near line 10
# So the check belongs HERE, in the fail-fast pre-flight beside the notices,
# not at stage 6 after the expensive work.
for ENT_FILE in packaging/macos/appname.entitlements packaging/macos/appname-no-jit.entitlements; do
    [ -f "$ENT_FILE" ] || { say "entitlements STOP(missing $ENT_FILE)"; exit 1; }
    if ! ENT_ERR="$(python3 -c 'import plistlib,sys; plistlib.load(open(sys.argv[1],"rb"))' "$ENT_FILE" 2>&1)"; then
        say "entitlements STOP($ENT_FILE does not parse: $(printf '%s' "$ENT_ERR" | tail -1). codesign will reject it at stage 6 -- note that plutil -lint is BLIND to this class, see finding B-7)"
        exit 1
    fi
done
say "entitlements-parse OK(both entitlement files parse; checked before any build, not at stage 6)"

# 0c. reproducibility normalization (OB-11) ---------------------------------
# The release path used to build with whatever environment it inherited, while
# tools/repro-build.sh built the SAME code under a normalized one. Only the
# second was normalized, so the shipped libappname_cabi.dylib carried 115
# distinct absolute paths out of the maintainer's home directory
# (/Users/.../.cargo/registry/src/.../<crate>/src/*.rs) into a public artifact.
# The same crates built WITH the normalization carry none.
#
# SOURCED, NOT COPIED. The knobs come from repro-build.sh --print-env-sh, so the
# release build and the reproducibility build cannot drift. Copying them here
# would re-create the very defect this fixes, one file over: OB-12 found the
# release path sharing NOTHING with the project's own instruments, and a
# duplicated constant is how that starts.
if [ -x tools/repro-build.sh ]; then
    eval "$(bash tools/repro-build.sh --print-env-sh)"
    say "repro-env OK(normalization sourced from tools/repro-build.sh: SOURCE_DATE_EPOCH, RUSTFLAGS remap-path-prefix, LC_ALL, TZ, umask)"
else
    say "repro-env STOP(tools/repro-build.sh missing or not executable — the release build would embed absolute paths; see finding OB-11)"
    exit 1
fi

# 1. release binaries + the D-44 assertion ---------------------------------
# WAS: `cargo build --release -p appname-cabi`. That built ONE of the four
# artifacts tools/walled_symbols_release.sh asserts over
# (appname, appname-actions, libappname_cabi.dylib, libappname_py.dylib --
# four DIFFERENT packages: appname-cli, appname-script, appname-cabi,
# appname-py), and the tripwire was never invoked from this script at all.
#
# THE DEFECT (O-BUILD findings B-1/B-1c/B-1d; O-WATCH #484-#485; O-INT entries
# 645/646). Before the instrument fixes at 7a6bc4fd/713f11e2, a missing LAST
# leg killed the tripwire silently and gen_attestation.py attested a FALSE
# CRITICAL. Those two links are fixed and are not this stage's business. What
# they did NOT fix -- because it is not in their files -- is the leg set: with
# one leg present the tripwire honestly asserts over the ONE artifact it was
# handed and prints "all shipped artifacts PASS", and gen_attestation returns
# "pass". Measured against the FIXED instrument: 1-of-4 legs -> rc 0 -> pass.
# So the release path's failure mode became a FALSE PASS -- three legs never
# inspected, a green D-44 line in the document a release ships with. A false
# CRITICAL stops a release; a false PASS ships one.
#
# THE FIX, and why DEFAULT mode rather than --no-build: default mode runs
# `cargo build --release --workspace` ITSELF and then asserts, so this stage
# cannot go blind again if someone later narrows the build. `--no-build` would
# be faster by one no-op cargo invocation and would re-create exactly the
# coupling that caused B-1 -- a distant assertion silently depending on THIS
# script's build scope. Self-sufficiency is worth the seconds.
#
# Placed FIRST among the build stages so a gated symbol stops the run before
# any bundle is staged or signed -- the artifact is never assembled, let alone
# signed, around a symbol that must not ship. A HIT is an Appendix K incident
# (D-44 STOP), never a routine bug.
#
# NOTE, recorded rather than left implicit: the dmg itself ships only the
# shell .app and libappname_cabi.dylib. The other three legs are release
# binaries of OTHER channels (CLI, actions runner, Python module), so this
# stage deliberately builds three artifacts this dmg does not carry. D-44 asks
# whether a gated symbol reached ANY release artifact; a tripwire that only
# inspected the one binary this dmg happens to ship would be the narrower
# question. See also finding B-1e: the tool's line-145 comment promises a SKIP
# report for an absent leg that run_assertion does not implement -- routed to
# O-INT. This stage makes the promise moot by guaranteeing all four exist.
if bash tools/walled_symbols_release.sh; then
    say "walled-release OK(D-44 asserted over all four shipped legs, before staging or signing)"
else
    say "walled-release STOP(D-44 assertion failed above -- a CRITICAL is an Appendix K incident, not a routine bug; escalate to O-INT)"
    exit 1
fi
CABI_DYLIB="$(ls target/release/libappname_cabi.dylib 2>/dev/null || true)"
if [ -z "$CABI_DYLIB" ]; then
    say "cabi STOP(no cdylib produced — appname-cabi crate-type; flag to O-SCRIPT)"
    exit 1
fi
say "cabi OK($CABI_DYLIB)"

# 2. icns provenance (P5-D1: reviewer's recipe, see header) -----------------
SHIPPED_ICNS="shell/packaging/appname-placeholder.icns"
[ -f "$SHIPPED_ICNS" ] || { say "icns STOP(checked-in $SHIPPED_ICNS missing)"; exit 1; }
ICNS_SCRATCH="target/icns-provenance"
rm -rf "$ICNS_SCRATCH" && mkdir -p "$ICNS_SCRATCH"
if command -v iconutil >/dev/null 2>&1; then
    for run in 1 2; do
        python3 shell/packaging/make_placeholder_icon.py "$ICNS_SCRATCH/run$run.iconset" \
            || { say "icns STOP(make_placeholder_icon.py failed — pure-stdlib, environment-independent)"; exit 1; }
        if ! iconutil -c icns -o "$ICNS_SCRATCH/run$run.icns" "$ICNS_SCRATCH/run$run.iconset" 2>"$ICNS_SCRATCH/iconutil.err"; then
            say "icns SKIP(iconutil blocked in this environment [$(cat "$ICNS_SCRATCH/iconutil.err" | head -1)] — the recorded P5 sandbox limitation; committed reviewer-verified artifact stays authoritative)"
            ICNS_VERIFIED=skip
            break
        fi
    done
    if [ "${ICNS_VERIFIED:-}" != "skip" ]; then
        cmp -s "$ICNS_SCRATCH/run1.icns" "$ICNS_SCRATCH/run2.icns" \
            || { say "icns STOP(two-run regeneration NOT byte-identical — determinism regression in the recipe)"; exit 1; }
        if cmp -s "$ICNS_SCRATCH/run1.icns" "$SHIPPED_ICNS"; then
            say "icns OK(provenance verified: regenerated per the recorded recipe, two-run deterministic, byte-identical to $SHIPPED_ICNS)"
        elif [ -n "${APPNAME_ICNS_ALLOW_ENV_DIFF:-}" ]; then
            say "icns SKIP(regeneration differs from shipped bytes; APPNAME_ICNS_ALLOW_ENV_DIFF acknowledged — same-environment byte-identity caveat per the P5 record, pixels deterministic)"
        else
            say "icns STOP(regeneration differs from the shipped $SHIPPED_ICNS — on the recorded machine this means the script or artifact changed without re-verification; on a different zlib/iconutil environment set APPNAME_ICNS_ALLOW_ENV_DIFF=1 to acknowledge)"
            exit 1
        fi
    fi
else
    say "icns SKIP(iconutil not available — committed reviewer-verified artifact stays authoritative)"
fi

# 3. shell build — the SHELL'S bundle (-DAPPNAME_BUNDLE=ON, P5-D1) ----------
if ! command -v cmake >/dev/null 2>&1; then
    say "shell STOP(cmake not installed on this machine)"
    exit 1
fi
SHELL_BUILD="target/shell-release"
if cmake -S shell -B "$SHELL_BUILD" -DCMAKE_BUILD_TYPE=Release -DAPPNAME_BUNDLE=ON \
        -DCMAKE_PREFIX_PATH="$(dirname "$QT_BIN")" >"$SHELL_BUILD-configure.log" 2>&1 \
   && cmake --build "$SHELL_BUILD" --parallel >"$SHELL_BUILD-build.log" 2>&1; then
    say "shell OK"
else
    say "shell STOP(cmake configure/build failed — see $SHELL_BUILD-*.log)"
    exit 1
fi
BUILT_APP="$(find "$SHELL_BUILD" -maxdepth 2 -type d -name '*.app' | head -1)"
[ -n "$BUILT_APP" ] || { say "shell STOP(no .app produced — APPNAME_BUNDLE option missing from shell/?)"; exit 1; }

# 4. bundle staging (consume, verify, complete — never re-author) -----------
rm -rf "$STAGE_DIR" && mkdir -p "$STAGE_DIR" "$BUILD_LOG_DIR"
cp -R "$BUILT_APP" "$APP"    # volume/app-folder name "${PRODUCT_NAME}.app" (D-43, OD-1)
PLIST="$APP/Contents/Info.plist"
[ -f "$PLIST" ] || { say "bundle STOP(shell bundle has no Info.plist)"; exit 1; }
BUNDLE_ID="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$PLIST" 2>/dev/null || true)"
if [ "$BUNDLE_ID" != "org.scandium-editor.Scandium" ]; then
    say "bundle STOP(CFBundleIdentifier '$BUNDLE_ID' != the E-02 identity org.scandium-editor.Scandium (WORD 11, final) — identity changes route via OD-1, not packaging)"
    exit 1
fi
BUNDLE_EXE="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$PLIST" 2>/dev/null || true)"
[ -n "$BUNDLE_EXE" ] && [ -f "$APP/Contents/MacOS/$BUNDLE_EXE" ] \
    || { say "bundle STOP(CFBundleExecutable missing from the shell bundle)"; exit 1; }
cmp -s "$APP/Contents/Resources/appname-placeholder.icns" "$SHIPPED_ICNS" \
    || { say "bundle STOP(bundle icns differs from the checked-in $SHIPPED_ICNS)"; exit 1; }
# OB-19. The catalog that SHIPS must BE the catalog that was reviewed and merged.
# Existence is the wrong predicate here: shell/build/i18n/ is an untracked build
# directory that can sit days behind tracked i18n/ (measured six days and sixteen
# keys behind in a sibling tree on 09-09), and a stale copy satisfies -f perfectly.
# Every notice the release owes out of the About box -- the licence line, the
# trademark line, and the no-affiliation sentence that must name Adobe in order to
# deny affiliation with it -- is a string in these files. Ship a stale catalog and
# the product states the wrong legal text while the gate reports OK.
[ -f "$APP/Contents/MacOS/i18n/en.json" ] \
    || { say "bundle STOP(shell bundle carries no staged i18n — Contents/MacOS/i18n/en.json missing)"; exit 1; }
I18N_N=0
for SRC_CAT in "$ROOT"/i18n/*.json; do
    [ -e "$SRC_CAT" ] \
        || { say "bundle STOP(no catalogs at $ROOT/i18n/*.json — the source of truth is missing)"; exit 1; }
    CAT_NAME="$(basename "$SRC_CAT")"
    STAGED_CAT="$APP/Contents/MacOS/i18n/$CAT_NAME"
    [ -f "$STAGED_CAT" ] \
        || { say "bundle STOP(staged i18n is missing $CAT_NAME, which tracked i18n/ carries)"; exit 1; }
    cmp -s "$SRC_CAT" "$STAGED_CAT" \
        || { say "bundle STOP(staged $CAT_NAME differs from tracked i18n/$CAT_NAME — the bundle is carrying a stale build copy, not the reviewed catalog)"; exit 1; }
    I18N_N=$((I18N_N + 1))
done
# E-06's About row, gated on the artifact rather than on the source tree: the
# disclaimer is what answers a trade-dress charge, so its absence is a release stop.
# OB-73. This list is the E-06 gate -- its own STOP message says so -- and it
# asserted three keys while E-06's row names "Qt LGPL text + RELINK NOTICE". The
# text ships (LICENSES/LGPL-3.0-only.txt, gated at 4b); the notice is
# `ui.dialog.about.qt_lgpl`, which main carries and this lane does not, so three
# green builds today imaged a bundle missing the one notice LGPL-3.0 §4(d) is
# about. A gate whose key list is a SUBSET of what its row requires passes the
# artifact it exists to stop.
NOTICE_KEYS=(ui.dialog.about.no_affiliation ui.dialog.about.trademarks ui.dialog.about.license ui.dialog.about.qt_lgpl)
for NOTICE_KEY in "${NOTICE_KEYS[@]}"; do
    grep -q "\"$NOTICE_KEY\"" "$APP/Contents/MacOS/i18n/en.json" \
        || { say "bundle STOP(staged en.json carries no $NOTICE_KEY — E-06 owes this notice in the About box)"; exit 1; }
done
# The count is DERIVED. It read "3" while the loop above asserted four, from 18:3x
# when I added qt_lgpl and left the literal -- the arm was right and its label
# understated it, which is the defect this desk filed against three other files
# today. The first full build printed it.
say "bundle OK(i18n: $I18N_N catalogs staged byte-identical to tracked i18n/; ${#NOTICE_KEYS[@]} About notice keys present)"
# Packaging completes what the shell's dev-machine layout leaves absolute:
# the cabi dylib rides in Frameworks with the load command rewritten.
mkdir -p "$APP/Contents/Frameworks"
cp "$CABI_DYLIB" "$APP/Contents/Frameworks/"
# The dylib's OWN id (LC_ID_DYLIB) is cargo's absolute build path -- e.g.
# /Users/<user>/.../target/release/deps/libappname_cabi.dylib. Rewriting only the
# executable's reference to it (below) leaves that id inside the shipped file, so a
# build-machine path reaches every recipient. It loads correctly either way, because
# the executable's own LC_LOAD_DYLIB is what resolves; this is a PROVENANCE leak, not
# a load failure, which is exactly why it survived so long.
# Found by the stage-5b path scan (OB-18) AFTER OB-11 had reported the cabi dylib
# clean -- OB-11 measured with `strings`, which does not surface load commands, while
# `grep -a` and `otool -D` both do. Discipline (c): vary the instrument.
install_name_tool -id "@executable_path/../Frameworks/libappname_cabi.dylib" \
    "$APP/Contents/Frameworks/libappname_cabi.dylib"
# DERIVE THE REFERENCE, DO NOT GUESS IT (finding OB-17a). This used to be two
# install_name_tool calls against two GUESSED paths -- "@rpath/…" and
# "$ROOT/target/release/libappname_cabi.dylib" -- each with `|| true`. cargo's
# install_name actually points into target/release/DEPS/, so the absolute guess
# never matched, and shell/CMakeLists.txt:37 says plainly that "the cargo-built
# dylib carries an absolute install name". A third guess would fail the same way
# the first two did the moment the layout moved again.
# So: read the reference the executable really carries and rewrite exactly that.
CABI_OLD="$(otool -L "$APP/Contents/MacOS/$BUNDLE_EXE" 2>/dev/null | grep -o '[^[:space:]]*libappname_cabi\.dylib' | head -1)"
if [ -n "$CABI_OLD" ] && [ "$CABI_OLD" != "@executable_path/../Frameworks/libappname_cabi.dylib" ]; then
    install_name_tool -change "$CABI_OLD" \
        "@executable_path/../Frameworks/libappname_cabi.dylib" "$APP/Contents/MacOS/$BUNDLE_EXE"
fi
# VERIFY THE REWRITE (finding OB-17). The two calls above are each allowed to fail
# because exactly one is expected to match -- the shell links the dylib either by
# @rpath or by its absolute build path. What was missing is the assertion that AT
# LEAST ONE worked. Without it, a shell that linked it a third way would leave both
# calls as no-ops, "bundle OK" would print, the dmg would image, and the app would
# ship pointing at a path that does not exist on the user's machine -- surfacing as
# "it doesn't launch" on a clean Mac (D-13) with nothing in the build log to explain
# it. This desk had been running exactly this otool check BY HAND on every dmg since
# 2026-09-07 and never noticed the script did not; a human performing an automated
# check is the same defect as a check that cannot fail.
CABI_REF="$(otool -L "$APP/Contents/MacOS/$BUNDLE_EXE" 2>/dev/null | grep -o '[^[:space:]]*libappname_cabi\.dylib' | head -1)"
if [ -z "$CABI_REF" ]; then
    say "bundle STOP(the shell executable does not link libappname_cabi.dylib at all, so staging it into Frameworks is dead weight and the load-command rewrite was meaningless — see finding OB-17)"
    exit 1
fi
if [ "$CABI_REF" != "@executable_path/../Frameworks/libappname_cabi.dylib" ]; then
    say "bundle STOP(cabi load command is '$CABI_REF', not @executable_path/../Frameworks/libappname_cabi.dylib — neither install_name_tool rewrite matched, so the app would ship pointing outside its own bundle; finding OB-17)"
    exit 1
fi
# OPERATOR WORD 10, 2026-09-14 20:01 ET, verbatim: "do not build a release-named
# DMG from a tree that still carries `org.appname-placeholder.shell`". The guard
# above keeps the placeholder until E-02 lands (09-16 18:00 ET), when it re-points
# to `org.scandium-editor.Scandium` (WORD 11, final). So the lane may build today
# -- version 0.1.0 is not a release version -- and may NOT image an artifact whose
# name says 1.0 while its identity still says placeholder.
case "$VERSION" in
    1.0*)
        if [ "$BUNDLE_ID" = "org.appname-placeholder.shell" ]; then
            say "bundle STOP(WORD 10: refusing to build a release-named DMG (${PRODUCT_NAME}-${VERSION}-macOS-arm64.dmg) from a tree whose CFBundleIdentifier is still the placeholder '$BUNDLE_ID' — E-02 re-points it to org.scandium-editor.Scandium)"
            exit 1
        fi ;;
esac
say "bundle OK($APP consumed from shell build; identity+icns+i18n verified; cabi load command verified $CABI_REF)"

# 4b. license/notice staging (aud-rights R-01) ------------------------------
# Land the notice texts where a macOS recipient can find them, INSIDE the
# bundle and BEFORE codesign --deep (step 6) so the signature covers them.
# Destination Resources/licenses/ per the auditor's suggested direction. Then
# gate on their presence in the staged bundle exactly as icns/i18n are gated
# above — a staging miss STOPS rather than imaging an un-noticed dmg.
NOTICE_DEST="$APP/Contents/Resources/licenses"
rm -rf "$NOTICE_DEST" && mkdir -p "$NOTICE_DEST"
for f in "${NOTICE_FILES[@]}"; do
    cp "$f" "$NOTICE_DEST/" || { say "notices STOP(failed to stage '$f' into $NOTICE_DEST)"; exit 1; }
done
for d in "${NOTICE_DIRS[@]}"; do
    cp -R "$d" "$NOTICE_DEST/" || { say "notices STOP(failed to stage '$d/' into $NOTICE_DEST)"; exit 1; }
done
# OPERATOR WORD 10 (1). The loop above copies whole DIRECTORIES, and copying LICENSES/
# is the only thing that ever put the Qt text in the bundle. Now that the text lives
# outside LICENSES/, it must be staged explicitly or :276 below STOPS the build at
# stage 4b -- AFTER stage 0 passes (OB-64; O-INT sheet amendment 2 step 4).
cp "$QT_LGPL_TEXT" "$NOTICE_DEST/LICENSES/" \
    || { say "notices STOP(failed to stage '$QT_LGPL_TEXT' into $NOTICE_DEST/LICENSES/)"; exit 1; }
for f in "${NOTICE_FILES[@]}"; do
    [ -f "$NOTICE_DEST/$(basename "$f")" ] \
        || { say "notices STOP(staged bundle is missing '$(basename "$f")' — never image an un-noticed artifact)"; exit 1; }
done
[ -d "$NOTICE_DEST/LICENSES" ] \
    || { say "notices STOP(staged bundle is missing the LICENSES/ SPDX texts)"; exit 1; }
[ -f "$NOTICE_DEST/LICENSES/$(basename "$QT_LGPL_TEXT")" ] \
    || { say "notices STOP(staged bundle is missing the Qt LGPL-3.0-only text — aud-rights R-01)"; exit 1; }
say "notices OK(staged $(find "$NOTICE_DEST" -type f | wc -l | tr -d ' ') texts into ${NOTICE_DEST#"$APP/"}: top-level ${NOTICE_FILES[*]} + LICENSES/ SPDX incl. $(basename "$QT_LGPL_TEXT"))"

# 4c. SBOM staging (E-05) ---------------------------------------------------
# gen_sbom.sh's header has claimed since it was written that "release packaging
# attaches the fresh SBOM to the artifacts". It did not. Before this stage
# existed this script named sbom/cdx ZERO times against twenty for notices and
# licences, and no *.cdx.json appeared anywhere in the bundle (E-05 screen
# 2026-09-10; O-WATCH #554). Beside the licences, and BEFORE codesign --deep
# (stage 6) so the signature covers it, gated on presence and non-emptiness
# exactly as the notices are gated above.
#
# COVERAGE — stated in the stage line and in the shipped note, because
# ATTACHING IS NOT COVERING. cargo-cyclonedx walks the CARGO graph, so these
# documents describe the Rust dependency tree and NOT the third-party binaries
# macdeployqt puts in Contents/Frameworks: qt, glib, openssl, brotli, pcre and
# dbus are absent from every one of them. The population of record for "what
# does this artifact actually ship" is the shipped-library enumerator walked off
# the STAGED BUNDLE, never this SBOM. The OK line says so itself so that no
# reader of a green log can record E-05 as artifact coverage.
SBOM_DEST="$APP/Contents/Resources/sbom"
SBOM_NOTE="packaging/macos/sbom-README.md"
[ -f "$SBOM_NOTE" ] \
    || { say "sbom STOP(coverage note '$SBOM_NOTE' missing from the repo — the SBOM does not ship without the statement of what it does not cover)"; exit 1; }
./scripts/gen_sbom.sh >"$BUILD_LOG_DIR/gen_sbom.log" 2>&1 \
    || { say "sbom STOP(gen_sbom.sh failed — $(tail -1 "$BUILD_LOG_DIR/gen_sbom.log" 2>/dev/null); full log at $BUILD_LOG_DIR/gen_sbom.log)"; exit 1; }
rm -rf "$SBOM_DEST" && mkdir -p "$SBOM_DEST"
SBOM_N=0
for f in sbom/*.cdx.json; do
    [ -e "$f" ] || break
    [ -s "$f" ] \
        || { say "sbom STOP(generated '$f' is zero bytes — an empty document is not an SBOM)"; exit 1; }
    cp "$f" "$SBOM_DEST/" \
        || { say "sbom STOP(failed to stage '$f' into $SBOM_DEST)"; exit 1; }
    SBOM_N=$((SBOM_N + 1))
done
# NON-VACUITY (OB-21's arm; discipline (j)). Zero documents must REFUSE, never
# report clean: `find | wc -l` over an empty directory prints 0 quite happily and
# the OK line below would print beside it, green, over a bundle with no SBOM.
[ "$SBOM_N" -gt 0 ] \
    || { say "sbom STOP(gen_sbom.sh reported success and produced NO sbom/*.cdx.json — refusing to image a dmg whose SBOM directory is empty)"; exit 1; }
cp "$SBOM_NOTE" "$SBOM_DEST/README.md" \
    || { say "sbom STOP(failed to stage the coverage note into $SBOM_DEST)"; exit 1; }
# NORMALIZE THE BUILD-MACHINE PATHS OUT OF THE STAGED COPIES. cargo-cyclonedx
# writes each workspace member's bom-ref/ref/dependsOn as an ABSOLUTE source
# path, so all 21 documents carried "$ROOT/crates/..." into the bundle. Caught
# on the first full pipeline run by stage 5b's path arm (OB-11/OB-18) two stages
# later; a stage that gates its own output would have caught it here, so it now
# does. This is OB-11's remedy one file over: the dev-tree sbom/ output is left
# untouched, only the SHIPPED copies are rewritten, and the rewrite is a prefix
# strip that turns them into repo-relative refs -- the three keys involved are
# internal cross-references, not licence or provenance data, and all three are
# rewritten together so referential integrity holds.
SBOM_LEAK_N=0
for f in "$SBOM_DEST"/*.cdx.json; do
    [ -e "$f" ] || break
    if grep -q "$ROOT/" "$f" 2>/dev/null; then
        SBOM_LEAK_N=$((SBOM_LEAK_N + 1))
        python3 - "$f" "$ROOT/" <<'NORM' || { say "sbom STOP(failed to normalize build-machine paths out of $(basename "$f"))"; exit 1; }
import io, sys
path, root = sys.argv[1], sys.argv[2]
body = io.open(path, encoding="utf-8").read()
io.open(path, "w", encoding="utf-8").write(body.replace(root, ""))
NORM
    fi
done
# Gate the stage's OWN output: no build-machine path may survive here.
# `|| true` IS LOAD-BEARING, and its absence cost a whole pipeline run. grep -l
# exits 1 when it matches NOTHING, which here is the good case; under this
# script's `set -e` a bare command substitution in an assignment propagates that
# status and kills the run WITH NO STAGE LINE AT ALL -- the B-8 shape, and a
# success-path one: the cleaner the artifact, the more certainly the build died.
SBOM_STILL="$(grep -l "$ROOT/" "$SBOM_DEST"/*.cdx.json 2>/dev/null | tr '\n' ' ' || true)"
[ -z "$SBOM_STILL" ] \
    || { say "sbom STOP(build-machine paths still present in the staged SBOM after normalization: $SBOM_STILL)"; exit 1; }
# Re-gate IN THE STAGED BUNDLE, not in sbom/ — the same reason stage 4b re-checks
# its own destination: what was copied is the only thing that ships.
SBOM_STAGED=$(find "$SBOM_DEST" -name '*.cdx.json' -type f | wc -l | tr -d ' ')
[ "$SBOM_STAGED" -eq "$SBOM_N" ] \
    || { say "sbom STOP(staged $SBOM_STAGED of $SBOM_N generated documents — a staging miss; never image a partial SBOM)"; exit 1; }
[ -s "$SBOM_DEST/README.md" ] \
    || { say "sbom STOP(the staged coverage note is missing or empty in the bundle)"; exit 1; }
say "sbom OK(staged $SBOM_STAGED CycloneDX documents + README.md into ${SBOM_DEST#"$APP/"}, before codesign; build-machine paths normalized out of $SBOM_LEAK_N of them — COVERAGE: Rust dependency graph ONLY; the shipped-library enumerator off the staged bundle is the population of record, NOT this SBOM)"

# 5. Qt frameworks (LGPL-compliant: replaceable dynamic frameworks) ---------
if [ -x "$QT_BIN/macdeployqt" ]; then
    "$QT_BIN/macdeployqt" "$APP" -qmldir=shell/qml >"$BUILD_LOG_DIR/macdeployqt.log" 2>&1 \
        && say "qt-deploy OK(dynamic frameworks bundled; LGPL relink-ability preserved)" \
        || { say "qt-deploy STOP(macdeployqt failed — see $BUILD_LOG_DIR/macdeployqt.log)"; exit 1; }

# 5b. THE TWO CHECKS THIS DESK HAD BEEN RUNNING BY HAND (discipline (h), OB-17a).
# Both appeared in every dmg inspection from 2026-09-07 onward and in neither case
# did the script look. A verification a person performs repeatedly is a verification
# the pipeline does not have.
#
# (i) Qt must be linked as REPLACEABLE FRAMEWORKS, never statically absorbed. This is
#     E-03's DONE-WHEN in the ROADMAP ("otool -L shows framework links, not static")
#     and it is deps/qt6.yaml's elected arm: LGPL-3.0-only, dynamic linking only,
#     relink-ability preserved per LGPL 4(d). F-06 re-checks it at 09-22. If Qt ever
#     linked statically the licence election would be violated by the artifact while
#     every stage line still read OK.
QT_REFS="$(otool -L "$APP/Contents/MacOS/$BUNDLE_EXE" 2>/dev/null | grep -oE '[^[:space:]]*Qt[A-Za-z0-9]*\.framework[^[:space:]]*' || true)"
QT_N=$(printf '%s\n' "$QT_REFS" | grep -c . || true)
QT_BAD=$(printf '%s\n' "$QT_REFS" | grep -v '^@executable_path/' | grep -c . || true)
if [ "$QT_N" -eq 0 ]; then
    say "qt-link STOP(the executable references no Qt framework at all — either Qt was statically absorbed, violating deps/qt6.yaml's LGPL-3.0-only dynamic-linking election, or the bundle is not what this script thinks it is)"
    exit 1
fi
if [ "$QT_BAD" -ne 0 ]; then
    say "qt-link STOP($QT_BAD of $QT_N Qt references are not @executable_path-relative, so the app would load Qt from outside its bundle and LGPL 4(d) relink-ability is not preserved: $(printf '%s' "$QT_REFS" | grep -v '^@executable_path/' | tr '\n' ' '))"
    exit 1
fi
# (ii) No build-machine path may reach a public artifact. OB-11 removed 115 of them
#     from the cabi dylib by normalizing the release build; nothing guarded against
#     their return. Scoped to /Users deliberately: third-party Homebrew dylibs carry
#     /opt/homebrew paths of their own, which are upstream's provenance and not ours
#     to strip — measured, so this arm cannot false-positive on them.
LEAKS="$(grep -rl "/Users/" "$APP" 2>/dev/null || true)"
if [ -n "$LEAKS" ]; then
    say "paths STOP(build-machine paths inside the staged bundle — a public artifact must not carry them (finding OB-11): $(printf '%s' "$LEAKS" | sed "s|$APP/||" | tr '\n' ' '))"
    exit 1
fi
say "qt-link OK($QT_N Qt framework references, all @executable_path-relative — LGPL 4(d) relink preserved; no build-machine paths anywhere in the bundle)"
else
    say "qt-deploy STOP(macdeployqt not found at $QT_BIN)"
    exit 1
fi

# 5c. D-44 OVER THE SHIPPED POPULATION (finding OB-21) ----------------------
# Stage 1 already ran the tripwire over target/release, and that stays: B-1's
# point is that a violation is caught BEFORE anything is staged or signed. But
# the function it scans is called shipped_artifacts and every path in it is a
# dev-tree path. Measured 2026-09-10 against this very bundle: three of its four
# legs do not ship at all, the fourth ships as a copy install_name_tool rewrote,
# and Contents/MacOS/appname-shell -- the file a user double-clicks, and where
# gated §18.4/§21.4 UI would land -- was scanned by nothing.
#
# So this is an ADDITION, not a swap, and it runs here because after macdeployqt
# is when the shipped set exists in final form. It is also not a second matcher:
# the population comes from tools/shipped_libraries.py and the patterns from
# scripts/gate.sh, exactly as stage 1's pass does. An empty population REFUSES.
if tools/walled_symbols_release.sh --bundle "$APP" >"$BUILD_LOG_DIR/walled-bundle.log" 2>&1; then
    say "walled-bundle OK(D-44 asserted over the SHIPPED population: $(grep -c '^WALLED-RELEASE' "$BUILD_LOG_DIR/walled-bundle.log" | tr -d ' ') appname-owned Mach-O in the staged bundle, including Contents/MacOS/appname-shell)"
else
    say "walled-bundle STOP(D-44 assertion failed over the staged bundle -- see $BUILD_LOG_DIR/walled-bundle.log. A CRITICAL here is an Appendix K incident, not a routine bug; escalate to O-INT: $(tail -1 "$BUILD_LOG_DIR/walled-bundle.log" 2>/dev/null))"
    exit 1
fi
# 5d. RPATH HYGIENE (finding OB-28) -----------------------------------------
# The shipped executable carried LC_RPATH /opt/homebrew/opt/qt/lib -- the BUILD
# MACHINE's Qt directory -- and neither existing arm could see it. Arm (i) reads
# `otool -L`, which lists LC_LOAD_DYLIB and not LC_RPATH. Arm (ii) greps for
# /Users/ only, scoped that way deliberately because third-party Homebrew dylibs
# carry /opt/homebrew paths that are upstream's provenance. The defect fell
# between two arms, each individually correct. (O-WATCH #706 §4 / Q-D66 addendum.)
#
# It is dead weight on a complete bundle -- measured: the executable makes zero
# @rpath loads of its own, the 64 binaries that do load via @rpath are resolved by
# their own @loader_path entries, and deleting it left the app launching. It is not
# harmless: a fallback search path in a public artifact means a failed in-bundle
# @rpath load falls through to the USER's /opt/homebrew/opt/qt/lib, silently
# loading a foreign Qt and masking a missing-framework bug on any machine that has
# Homebrew Qt. Invisible on the build machine by construction.
#
# OURS is stripped; UPSTREAM's is reported and not touched -- editing third-party
# binaries is a separate decision and only our own deletion has been measured safe.
# Runs before codesign because install_name_tool invalidates the signature.
# OB-37, and §5a-6 with it: an empty result and a REFUSED one are the same bytes,
# so the exit code is captured and a refusal STOPs with its reason instead of
# degrading into "the enumerator returned no binaries". 2>/dev/null here would
# throw away the only sentence that distinguishes the two.
if ! RPATH_RAW="$(python3 tools/shipped_libraries.py --app "$APP" \
        --caller=build_dmg --allow-in-flight 2>"$BUILD_LOG_DIR/rpath-census.err")"; then
    say "rpath STOP(the bundle census REFUSED rather than returned empty — $(tail -1 "$BUILD_LOG_DIR/rpath-census.err" 2>/dev/null))"
    exit 1
fi
RPATH_POP="$(printf '%s\n' "$RPATH_RAW" | tail -n +6 || true)"
[ -n "$RPATH_POP" ] \
    || { say "rpath STOP(the bundle enumerator returned no binaries — an empty walk REFUSES rather than reporting clean)"; exit 1; }

# EVERY absolute rpath, not only ours (operator's WORD 7, 2026-09-13 09:33 ET,
# ruling (3)). OB-28 stripped appname-owned rpaths and REPORTED upstream's three,
# on the ground that editing third-party binaries is a separate decision and only
# my own deletion had been measured safe. He ruled: strip them all.
#
# MEASURED BEFORE STRIPPING, because "he told me to" is not evidence: each of the
# three upstream carriers -- libdbus-1.3, libjasper.7, libjpeg.8 -- makes ZERO
# @rpath loads of its own, so its rpath is dead weight exactly as the shell's was
# and deleting it cannot break resolution. install_name_tool invalidates the
# signature; stage 6's codesign --deep --force re-signs everything after this.
RPATH_STRIPPED=0; RPATH_FILES=0
while IFS=$'\t' read -r rel owner _rest; do
    [ -n "$rel" ] || continue
    BIN="$APP/$rel"
    # `|| true`: grep exits 1 on no match, which here is the good case, and under
    # `set -e` that would kill the run on a clean binary (finding OB-26).
    ABS="$(otool -l "$BIN" 2>/dev/null | awk '/LC_RPATH/{f=1} f&&/ path /{print $2; f=0}' | grep '^/' || true)"
    [ -n "$ABS" ] || continue
    RPATH_FILES=$((RPATH_FILES + 1))
    for RP in $ABS; do
        install_name_tool -delete_rpath "$RP" "$BIN" 2>/dev/null \
            || { say "rpath STOP(failed to delete build-machine rpath '$RP' from $rel)"; exit 1; }
        RPATH_STRIPPED=$((RPATH_STRIPPED + 1))
    done
done <<EOF
$RPATH_POP
EOF
say "rpath OK(stripped $RPATH_STRIPPED absolute rpath(s) from $RPATH_FILES binaries — appname-owned AND upstream, per WORD 7 (3))"

# 5e. THE LC_RPATH ARM — a SEPARATE assertion that reds on ANY absolute rpath.
# Separate on purpose (WORD 7 (3): "a separate LC_RPATH arm that reds on any
# absolute path"). The strip above is an ACTION; this is the CHECK, and it walks
# the bundle again rather than trusting the loop's own counter. A fix that also
# grades itself is the shape B-1 was: a green line over one leg of four.
RPATH_LEFT=""
while IFS=$'\t' read -r rel owner _rest; do
    [ -n "$rel" ] || continue
    L="$(otool -l "$APP/$rel" 2>/dev/null | awk '/LC_RPATH/{f=1} f&&/ path /{print $2; f=0}' | grep '^/' || true)"
    [ -n "$L" ] && RPATH_LEFT="$RPATH_LEFT $(basename "$rel"):$(printf '%s' "$L" | tr '\n' ',')"
done <<EOF
$RPATH_POP
EOF
[ -z "$RPATH_LEFT" ] \
    || { say "rpath-arm STOP(absolute LC_RPATH still present in the staged bundle — a public artifact must not carry build-machine search paths:$RPATH_LEFT)"; exit 1; }
say "rpath-arm OK(re-walked $(printf '%s\n' "$RPATH_POP" | grep -c . || true) binaries with otool -l: ZERO absolute LC_RPATH anywhere in the bundle)"

# 6. codesign ---------------------------------------------------------------
# Entitlement selection (ES §33.4 / §25.3 / §48.3): the allow-jit
# entitlement exists ONLY when the Wasmtime plugin tier is compiled in. We
# do not guess from feature flags — we read the shipped dylib: if it carries
# wasmtime/cranelift runtime symbols, the JIT is present and needs
# allow-jit; otherwise the tightest (empty) entitlement set applies. This
# ties the entitlement surface to the actual compiled-in tier, so the §48.3
# interim mitigation (disable the WASM tier) mechanically shrinks the
# entitlement set with no packaging edit.
ENT_JIT="packaging/macos/appname.entitlements"
ENT_NOJIT="packaging/macos/appname-no-jit.entitlements"
if command -v nm >/dev/null 2>&1 && nm "$CABI_DYLIB" 2>/dev/null | grep -qiE 'wasmtime|cranelift'; then
    ENTITLEMENTS="$ENT_JIT"
    say "entitlements OK(wasm-plugins tier detected in $CABI_DYLIB — allow-jit selected, §33.4/§25.3)"
else
    ENTITLEMENTS="$ENT_NOJIT"
    say "entitlements OK(no wasmtime/cranelift symbols — no-jit empty entitlement set, §48.3)"
fi
[ -f "$ENTITLEMENTS" ] || { say "entitlements STOP(selected $ENTITLEMENTS missing)"; exit 1; }
# FAIL LOUDLY LIKE EVERY OTHER STAGE (finding B-8, 2026-09-07). codesign used to
# be the ONE stage that failed without naming itself: it ran bare, so a failure
# killed the script through `set -e` with no "DMG codesign STOP(...)" line. The
# stage list simply ended after "entitlements OK", which reads like a finished
# run -- and under a pipeline (`| tee`) the shell reports the LAST command's
# status, so the failure surfaced as exit 0. That is exactly how finding B-7
# (an unparseable entitlements file) hid: the dmg was never built and nothing
# said so. The parser's own message is quoted into the STOP because it is the
# only thing that names the real cause.
CS_LOG="$BUILD_LOG_DIR/codesign.err"
if [ -n "${APPNAME_SIGN_IDENTITY:-}" ]; then
    if codesign --deep --force --options runtime --entitlements "$ENTITLEMENTS" -s "$APPNAME_SIGN_IDENTITY" "$APP" 2>"$CS_LOG"; then
        say "codesign OK(identity; hardened runtime + $ENTITLEMENTS)"
    else
        say "codesign STOP(codesign failed with $ENTITLEMENTS: $(head -1 "$CS_LOG"))"
        exit 1
    fi
else
    # Ad-hoc, but with the SAME hardened-runtime + entitlements wiring the
    # real Developer ID path uses, so the dry run exercises the entitlement
    # application rather than skipping it.
    # NO HARDENED RUNTIME ON THE AD-HOC PATH. The entitlements stay.
    # Operator's WORD 7, 2026-09-13 09:33 ET, ruling Q-D69 (a).
    #
    # This branch used to pass --options runtime so the dry run would exercise
    # entitlement application rather than skip it. That was right about the
    # entitlements and fatal in combination with `-s -`: the hardened runtime
    # turns LIBRARY VALIDATION on, an ad-hoc signature carries NO Team ID, and
    # dyld then refuses every bundled Qt framework. The artifact was correctly
    # assembled and could not start -- `open` returned 0, nothing ran, and the
    # binary gave exit 134 (OB-27 / Q-D69 CRITICAL). Two individually correct
    # flags, fatal together.
    #
    # Hardened runtime is a NOTARIZATION requirement and Amendment 9 deferred
    # notarization, so here it currently buys nothing and costs the launch. IT
    # RETURNS WITH A DEVELOPER ID -- the identity branch above still sets it, and
    # there every nested binary shares one Team ID, which is what library
    # validation actually needs.
    if codesign --deep --force --entitlements "$ENTITLEMENTS" -s - "$APP" 2>"$CS_LOG"; then
        say "codesign SKIP(no APPNAME_SIGN_IDENTITY — ad-hoc signature, NO hardened runtime (Q-D69 (a) — it turns library validation ON and an ad-hoc signature has no Team ID, so dyld refused every bundled Qt framework: finding OB-27), entitlements $ENTITLEMENTS still applied; distribution requires a Developer ID identity)"
    else
        say "codesign STOP(ad-hoc codesign failed with $ENTITLEMENTS: $(head -1 "$CS_LOG")). An entitlements file that plutil accepts can still be REJECTED here -- codesign's AMFI parser is stricter. See finding B-7.)"
        exit 1
    fi
fi

# 7. dmg --------------------------------------------------------------------
rm -f "$DMG"
# AMENDMENT 10 (e) renamed this artifact and E-11's freeze bump renames it again.
# `rm -f "$DMG"` cleans only the CURRENT name, so each rename leaves the previous
# build's dmg sitting in target/ looking exactly like a release artifact -- one was
# found there on 2026-09-14 and removed by hand. The B-9 arm below asserts what goes
# INTO the image; this asserts that nothing is left lying beside it.
#
# `[ ... ] && continue` is NOT used here on purpose: under `set -euo pipefail` a
# false test makes the compound return 1 and kills the run, which is the defect the
# OB-69 leak arm shipped with for one commit.
for _stale in target/"${PRODUCT_NAME}"-*.dmg; do
    if [ -e "$_stale" ] && [ "$_stale" != "$DMG" ]; then
        rm -f "$_stale"
        say "dmg NOTE(removed stale artifact '$_stale' — a previous name for this build's output; only '$(basename "$DMG")' ships)"
    fi
done
# B-9 arm: image ONLY the product. Everything in $STAGE_DIR reaches the recipient,
# so assert its contents rather than trusting that nothing strayed in. Ten green
# stage lines above described what this script decided to say, not what the volume
# holds -- that is exactly how two build logs shipped unnoticed.
STAGED="$(ls -A "$STAGE_DIR")"
if [ "$STAGED" != "${PRODUCT_NAME}.app" ]; then
    say "dmg STOP(staging dir must contain exactly ${PRODUCT_NAME}.app before imaging; found: $(printf '%s' "$STAGED" | tr '\n' ' ')). Anything here ships to every recipient — see finding B-9.)"
    exit 1
fi
hdiutil create -volname "${PRODUCT_NAME} ${VERSION}" -srcfolder "$STAGE_DIR" -ov -format UDZO "$DMG" >/dev/null
say "dmg OK($DMG; staging asserted to hold only ${PRODUCT_NAME}.app)"

# 7b. attestation, PERSISTED (OB-20, O-WATCH #514's routed fix) ---------------
# tools/gen_attestation.py had no caller anywhere in the tree: five mentions
# under scripts/ and tools/, every one of them a comment ABOUT it. So R2's
# leg 3 -- "gen_attestation reports no CRITICAL" -- rested on a sentence in my
# ledger, with no artifact for a countersigning desk to re-read. #514 asked for
# the file. The file alone would only prove EXISTENCE, which is the predicate
# OB-19 was filed against this morning, so the arm reads what it just wrote:
# rc, then CRITICAL grepped out of the PERSISTED bytes, then the sha of those
# same bytes so a second desk can prove it read mine.
ATTEST="$BUILD_LOG_DIR/attestation.txt"
if ! python3 tools/gen_attestation.py --emit --out "$ATTEST" >/dev/null 2>"$BUILD_LOG_DIR/attestation.err"; then
    say "attestation STOP(gen_attestation.py --emit failed — see $BUILD_LOG_DIR/attestation.err)"
    exit 1
fi
[ -s "$ATTEST" ] \
    || { say "attestation STOP(--out wrote nothing to $ATTEST)"; exit 1; }
if grep -q "CRITICAL" "$ATTEST"; then
    say "attestation STOP(D-44: the persisted attestation carries CRITICAL — $(grep -c CRITICAL "$ATTEST") line(s) in $ATTEST)"
    exit 1
fi
say "attestation OK($ATTEST, $(wc -c < "$ATTEST" | tr -d ' ') bytes, sha256 $(shasum -a 256 "$ATTEST" | cut -c1-16)…; no CRITICAL in the persisted file)"

# 8. notarization (dry-run boundary) ---------------------------------------
if [ -n "${APPNAME_NOTARY_PROFILE:-}" ]; then
    xcrun notarytool submit "$DMG" --keychain-profile "$APPNAME_NOTARY_PROFILE" --wait
    xcrun stapler staple "$DMG"
    say "notarize OK"
else
    say "notarize STOP(dry-run boundary: no notary credentials on this machine; real run = notarytool submit --keychain-profile + stapler staple, see header)"
fi

# 9. RELEASE SET (E-10 - E-08 - X-16; OPERATOR WORD 8 (1), 2026-09-14 13:35 ET) --
# His words: "two producers as release stages in your pipeline - SHA256SUMS over
# the whole release set, and the source tarball by `git archive` at the release
# sha - plus release-page copies of the SBOMs; measured, countersigned."
#
# ORDER IS LOAD-BEARING. SHA256SUMS is written LAST, over the finished directory,
# so X-16's "covers every artifact" is true BY CONSTRUCTION and not by care. The
# manifest is then verified with `shasum -c` against the bytes just written: a
# manifest nobody checked is a claim, and this desk has filed that finding twice.
RELEASE_DIR="target/release-set"   # NAME NOT YET RULED - decision sheet, X-16
RELEASE_SHA="$(git rev-parse --verify --quiet HEAD)" \
    || { say "release STOP(HEAD does not resolve - cannot stamp a release set)"; exit 1; }

# A tarball is taken from the SHA; if the tree is dirty the tarball does not
# contain the bits that were just built, and the set would carry a false
# provenance. Refuse rather than ship a mismatch. (Reversible by one word.)
RELEASE_DIRTY="$(git status --porcelain | wc -l | tr -d ' ')"
[ "$RELEASE_DIRTY" -eq 0 ] \
    || { say "release STOP(working tree has $RELEASE_DIRTY modified path(s) - a source tarball taken from $RELEASE_SHA would not match the artifact built from this tree)"; exit 1; }

rm -rf "$RELEASE_DIR" && mkdir -p "$RELEASE_DIR"

# (a) the dmg
cp "$DMG" "$RELEASE_DIR/" \
    || { say "release STOP(failed to copy '$DMG' into $RELEASE_DIR)"; exit 1; }

# AMENDMENT 10 (e) made E-10's name a PATTERN rather than the literal
# `Appname-1.0-macOS-arm64.dmg`, so this arm is LIVE AT EVERY VERSION -- there is
# no version at which the pattern does not apply, and the version-gated form this
# replaced was INERT for exactly the versions the tree actually builds (OB-64).
RELEASE_WANT="${PRODUCT_NAME}-${VERSION}-macOS-arm64.dmg"
[ "$(basename "$DMG")" = "$RELEASE_WANT" ] \
    || { say "release STOP(OB-64 / AMENDMENT 10 (e): dmg is '$(basename "$DMG")', E-10's pattern <Name>-<version>-macOS-arm64.dmg requires '$RELEASE_WANT')"; exit 1; }

# (b) release-page copies of the SBOMs -- FROM THE STAGED BUNDLE, NOT FROM sbom/.
# OB-69, found on the first real release set: stage 4c normalizes build-machine
# paths out of the documents it stages into the bundle, and sbom/ keeps the RAW
# ones. Copying from sbom/ published 21 of 22 documents carrying
# /Users/<user>/... absolute paths that had been deliberately scrubbed from the
# shipped copy -- the OB-11/OB-18 defect, one directory over, on the surface the
# public actually downloads. $SBOM_DEST also carries the README whose coverage
# statement E-05 (ii) requires, so the caveat now travels with the documents.
REL_SBOM_N=0
for f in "$SBOM_DEST"/*; do
    [ -e "$f" ] || continue
    [ -s "$f" ] || { say "release STOP(staged SBOM file '$f' is zero bytes)"; exit 1; }
    cp "$f" "$RELEASE_DIR/" || { say "release STOP(failed to copy '$f' into $RELEASE_DIR)"; exit 1; }
    REL_SBOM_N=$((REL_SBOM_N + 1))
done
[ "$REL_SBOM_N" -gt 0 ] \
    || { say "release STOP(E-10 lists SBOMs on the release page and none were copied - $SBOM_DEST is empty)"; exit 1; }

# The arm that would have caught OB-69 at the point of PUBLICATION rather than in
# a ledger entry a day later. Normalization verified in one place and copied from
# another is not verified: assert on the bytes about to be published.
# `grep` exits 1 when it finds NOTHING, and under `set -euo pipefail` that killed
# the run at exactly the moment the release set was clean -- the arm fired on
# success and printed not one word. `|| true` keeps the pipeline's status out of it.
REL_LEAK="$(grep -rlF '/Users/' "$RELEASE_DIR" 2>/dev/null | head -5 || true)"
[ -z "$REL_LEAK" ] \
    || { say "release STOP(OB-69: build-machine paths in the release set — $(printf '%s' "$REL_LEAK" | tr '\n' ' '))"; exit 1; }

# (c) the source tarball, by git archive at the release sha.
# gzip -n keeps the wall clock out of the header and git archive stamps members
# with the COMMIT's time, so two runs at one sha are byte-identical.
REL_TAR="$RELEASE_DIR/${PRODUCT_NAME}-${VERSION}-src.tar"
git archive --format=tar --prefix="${PRODUCT_NAME}-${VERSION}/" "$RELEASE_SHA" -o "$REL_TAR" \
    || { say "release STOP(git archive failed at $RELEASE_SHA)"; exit 1; }
REL_TAR_N="$(tar -tf "$REL_TAR" | grep -c .)"
[ "$REL_TAR_N" -gt 0 ] \
    || { say "release STOP(source tarball covers 0 entries - an empty archive is not a tarball)"; exit 1; }
gzip -n -9 "$REL_TAR" || { say "release STOP(gzip failed on $REL_TAR)"; exit 1; }

# (c2) artifacts built elsewhere for THIS SAME TAG — operator WORD 16 (3).
# Windows rejoins 1.0 under Amendment 16: one tag, one date, TWO artifacts. The
# zip is produced by O-WIN on the Windows laptop, so it cannot be built here —
# but it must be IN the set before (d) runs, because SHA256SUMS is written over
# the finished DIRECTORY and the attestation is re-emitted from that same
# directory. Stage 9 wipes $RELEASE_DIR at its head, so anything staged in
# advance is destroyed: this inlet is the only moment a foreign artifact can
# join the set and still be covered by both producers.
# Unset, nothing changes and this is a no-op (the macOS-only path is untouched).
if [ -n "${APPNAME_EXTRA_ARTIFACTS:-}" ]; then
    [ -d "$APPNAME_EXTRA_ARTIFACTS" ] \
        || { say "release STOP(APPNAME_EXTRA_ARTIFACTS is not a directory: $APPNAME_EXTRA_ARTIFACTS)"; exit 1; }
    EXTRA_N=0
    for xf in "$APPNAME_EXTRA_ARTIFACTS"/*; do
        [ -f "$xf" ] || continue
        cp "$xf" "$RELEASE_DIR/" \
            || { say "release STOP(failed to copy extra artifact '$xf' into $RELEASE_DIR)"; exit 1; }
        EXTRA_N=$((EXTRA_N + 1))
    done
    [ "$EXTRA_N" -gt 0 ] \
        || { say "release STOP(APPNAME_EXTRA_ARTIFACTS holds no files: $APPNAME_EXTRA_ARTIFACTS - an empty inlet is a typo, not a choice)"; exit 1; }
    say "release OK(carried $EXTRA_N foreign artifact(s) from $APPNAME_EXTRA_ARTIFACTS into the set, before SHA256SUMS)"
fi

# (d) SHA256SUMS over the FINISHED set - written last, then verified.
( cd "$RELEASE_DIR" \
  && find . -maxdepth 1 -type f ! -name 'SHA256SUMS' | sed 's|^\./||' | sort \
     | while IFS= read -r rf; do shasum -a 256 "$rf"; done > SHA256SUMS ) \
    || { say "release STOP(failed to write SHA256SUMS)"; exit 1; }
REL_N="$(wc -l < "$RELEASE_DIR/SHA256SUMS" | tr -d ' ')"
[ "$REL_N" -gt 0 ] \
    || { say "release STOP(E-08/X-16: SHA256SUMS covers 0 artifacts - an empty manifest is not a pass)"; exit 1; }
( cd "$RELEASE_DIR" && shasum -a 256 -c SHA256SUMS >/dev/null 2>&1 ) \
    || { say "release STOP(SHA256SUMS does not verify against the bytes just written)"; exit 1; }

# OPERATOR WORD 9 (2) / AMENDMENT 11 (e). Stage 7b emitted the attestation before
# this set existed, so it named no artifact -- OB-70. Re-emit it HERE, against the
# finished directory, and re-run 7b's two assertions on the new bytes: an
# attestation that is regenerated but not re-checked is 7b's gate spent on a file
# that no longer exists.
if ! python3 tools/gen_attestation.py --emit --out "$ATTEST" --artifacts-dir "$RELEASE_DIR" >/dev/null 2>>"$BUILD_LOG_DIR/attestation.err"; then
    say "release STOP(WORD 9 (2): re-emitting the attestation with the artifacts block failed — see $BUILD_LOG_DIR/attestation.err)"
    exit 1
fi
[ -s "$ATTEST" ] || { say "release STOP(re-emitted attestation is empty at $ATTEST)"; exit 1; }
if grep -q "CRITICAL" "$ATTEST"; then
    say "release STOP(D-44: the re-emitted attestation carries CRITICAL — $(grep -c CRITICAL "$ATTEST") line(s))"
    exit 1
fi
# Compare the two records by NAME SET, not by count. My first form asserted
# equality of counts and would have stopped every build: SHA256SUMS excludes
# itself by design and the attestation includes it, so the numbers differ by
# exactly one, forever, for a reason that has nothing to do with correctness.
# (Same lesson as OB-75 one hour earlier: do not assert a population count.)
REL_ATT_N="$(grep -c '^  - name: ' "$ATTEST")"
REL_MISSING="$(comm -23 \
    <(awk '{ $1=""; sub(/^ /,""); print }' "$RELEASE_DIR/SHA256SUMS" | sort) \
    <(sed -n 's/^  - name: //p' "$ATTEST" | sort) | tr '\n' ' ')"
[ -z "$(printf '%s' "$REL_MISSING" | tr -d '[:space:]')" ] \
    || { say "release STOP(SHA256SUMS manifests artifacts the attestation does not name: $REL_MISSING)"; exit 1; }
say "attestation OK(re-emitted with the artifacts block: $REL_ATT_N artifacts named with size and sha256, matching SHA256SUMS; sha256 $(shasum -a 256 "$ATTEST" | cut -c1-16))"

say "release OK($RELEASE_DIR: $REL_N artifacts manifested and verified; dmg $(basename "$DMG") + $REL_SBOM_N sbom + src tarball $REL_TAR_N entries at ${RELEASE_SHA}; SHA256SUMS sha256 $(shasum -a 256 "$RELEASE_DIR/SHA256SUMS" | cut -c1-16))"

say "done"
