<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (c) 2026 the Appname contributors -->

# Appname Qt/QML shell (CP-0.5 chrome)

CP-0.5 adds the §2.3 editing chrome: menus (File/Edit/Select/Image),
package Open/Save/Save As, eleven contract-exact dialogs (resize, canvas
size, rotate, trim, crop, fill, selection modify, assign/convert profile,
depth convert, flatten), canvas tools (marquee rect/ellipse, lasso,
polygon, wand, crop drag, move, free-transform frame), layers panel v2
(live visibility/opacity via class-M set_props sessions, rename, add/
delete/duplicate/group/ungroup/merge/reorder), OS-clipboard copy-out, and
a 9-frame tour. Selection rubber bands are in-flight input echo only —
marching ants, layer-tree/mask/blend readback, guides/grid overlays,
codec import/export, paste-in, and the recovery prompt all wait on the
ABI-growth requests 1–5 recorded in `reports/O-UI-log.md` (disabled
controls carry static milestone captions where the container allows —
menu items disable plain; nothing unreadable is displayed as truth). The shell's minted `ui.*` keys ride a seed catalog
(`shell/i18n/ui.en.json`) merged UNDER the canonical repo catalogs at
staging (`merge_catalogs.py` — canonical always wins); the lane-log mint
register remains the O-I18N fold-in source.

# Appname Qt/QML shell v1 (CP-0.3)

Spec: plan §2.2 "Qt/QML shell v1" row; lane orders §3.9. License:
GPL-3.0-only (engine/UI split per DD ④). "Appname" is the internal
name (D-43); the product is Scandium.

The shell is a **pure C-ABI client**: its only view of the engine is
`include/appname.h` plus the `appname-cabi` library — no Rust crate
imports, no engine math in C++ (the round-1 separation audit generalizes;
O-QA re-greps this tree every checkpoint). Every document mutation is an
`appname.command.v1` envelope through `appname_doc_apply_command`; pixels
arrive exclusively through `appname_doc_read_region` readback and are
blitted verbatim.

## Build mechanism

```
cmake -S shell -B target/shell-build -G Ninja \
      -DCMAKE_PREFIX_PATH=/opt/homebrew/opt/qt -DCMAKE_BUILD_TYPE=Debug
cmake --build target/shell-build
ctest --test-dir target/shell-build
```

The build tree lives under `target/` (NOT `shell/build`) deliberately:
the gate's walled-symbols sweep covers `shell/` recursively including
untracked files, and compiled object files can false-positive it with
GENUINE coincidental machine-code byte runs — reproduced as one of
the walled three-letter stems appearing delimited by non-alnum bytes
inside an object file, found by the gate's matcher (/usr/bin/grep,
BSD) and equally visible with -a, never present in any source. (The
exact byte evidence is quoted in reports/O-UI-log.md, which the
mechanical sweep deliberately does not cover — this file is swept, so
the stem is not named here. Review-corrected mechanism: an earlier
version of this note blamed a binary-vs-text matcher divergence in
ugrep — that was the INTERACTIVE shell's grep wrapper, which skips
binaries outright and had measured a different tool than the gate.)
`target/` is already excluded from the sweep and already gitignored.
A sweep-side binary/build-tree exclusion is filed with the lane log —
note -a alone would NOT fix it; the tree can move back when it lands.
(`cargo clean` wipes the CMake tree along with cargo's own output —
rerun the configure step after.)

- A cargo custom target (`appname_cabi_build`) runs
  `cargo build -p appname-cabi` in the repo root before every shell build
  (`~/.cargo/bin` is prepended to PATH for non-interactive shells; disable
  with `-DAPPNAME_CABI_BUILD=OFF` to link a prebuilt artifact).
- The shell then links the produced library **directly by absolute path**
  from `target/<profile>/`. Default `-DAPPNAME_CABI_LINK=shared` links
  `libappname_cabi.dylib` — the cargo-built dylib carries an absolute
  install name, which is fine for dev-machine builds.
  `-DAPPNAME_CABI_LINK=static` links `libappname_cabi.a` for a
  self-contained binary.
- **Flagged to O-BUILD:** corrosion-based cargo/CMake integration is the
  planned refinement of this custom-target mechanism, and bundle/rpath
  packaging is the packaging wave. Qt 6 itself (Homebrew, LGPL dynamic
  link) needs a `deps/qt6.yaml` record before merge.
  qt-advanced-docking-system is deliberately NOT used at v1 (plain QML
  layout); docking + its deps record is a follow-up per the plan.

## Running

```
target/shell-build/appname-shell                 # windowed session
target/shell-build/appname-shell --lang es       # language override (else APPNAME_LANG, else en)
target/shell-build/appname-shell --screenshot-tour /tmp/tour-out
```

## Screenshot tour (standing §3.9 acceptance artifact)

`--screenshot-tour <dir>` runs a deterministic scripted session — fixed
1280×800 window, fixed 640×400 white document, fixed stroke sample lists,
seeded envelope ids/stroke seeds, `Basic` QtQuick Controls style — and
writes numbered PNGs (launch, new-document, zoom-200, brush-stroke,
eraser-stroke, undo), exiting 0 when every frame was captured. It is
headless-friendly: with no explicit environment it defaults itself to
`QT_QPA_PLATFORM=offscreen` and `QT_QUICK_BACKEND=software` (an explicit
environment always wins, so a windowed tour is
`QT_QPA_PLATFORM=cocoa target/shell-build/appname-shell --screenshot-tour …`).

## i18n

Every visible string resolves through the C++ catalog helper
(`src/i18n_catalog.{h,cpp}`), a behavior-parity port of the CLI loader
(`crates/appname-cli/src/i18n.rs`): same JSON catalog format
(`{ "id": "template" }` or nested under `"messages"`, `_meta.plural_rules`),
same ICU-subset grammar (`{name}` placeholders, cardinal plurals with `#`),
same key-echo fallback. Resolution: `APPNAME_I18N_DIR` → executable-adjacent
`i18n/` → key-echo (per catalog file: a locale missing from the env dir
falls through to the exe-adjacent one). The shell embeds NO catalog;
O-I18N-A11Y's repo catalogs (`<repo>/i18n/*.json`, en/es/ja/pseudo) are
**staged exe-adjacent at build time** by CMake, so the default windowed run
and tour resolve registered ids — matching an installed layout. (Under
`-DAPPNAME_BUNDLE=ON` the exe-adjacent staging lands inside the `.app`, so
CMake stages a second identical copy flat in the build dir — the flat test
binaries' exe-adjacent `i18n/`.) Ids absent
from the catalogs (the shell's minted `ui.*` keys, listed in
`reports/O-UI-log.md` for the O-I18N sweep) key-echo *visibly by design* —
the standing honesty mechanism against hardcoded English. A pure key-echo
run remains reproducible by deleting `target/shell-build/i18n/` and running the
binary before rebuilding (the next build re-stages). The language is fixed
at process start in v1 (`--lang es|ja|pseudo` to switch); the runtime
switch surface arrives with the O-I18N audit wave.

## Stroke pipeline

Pointer input is normalized to §15.1 samples (mouse pressure 1.0 — tablet
input is CP-0.5), encoded as canonical `appname.samples.v1` bytes
(16-byte header + 48-byte records; bit-exact unit test vs the documented
layout), registered through **`appname_asset_create`** (the §9.2 ingress
as landed: blob bytes in, content-addressed assetId out), and committed as
a `brush.stroke` / `eraser.stroke` envelope whose `samples` field is that
assetId **string** (§9.6 rule 2 — never inline). Committed strokes render
engine-side and repaint via dirty-rect readback.

- While a stroke is in flight it is echoed on-canvas; if the engine
  rejects (typed, surfaced in the status bar) the echo stays as a visibly
  provisional **preview overlay** (capped at 64; dropped on the first
  successful commit, on document reset, and on document teardown; not part
  of history — a documented v1 limitation). An `appname_asset_create`
  rejection is surfaced the same way; the shell never ships a stroke
  envelope referencing an asset the engine did not acknowledge.
- Historical note: until the cabi flip (assembled stroke kinds + ingress)
  this section described a degraded path — client-uuid envelopes answered
  `error.command.not_implemented`, and the ingress was a dlsym probe for
  the §9.2 draft prototype (record array + count). The landed symbol takes
  the canonical blob BYTES instead; the probe is retired.
- The `brushPreset` param is the engine's built-in soft-round-pressure
  preset id (`7efe96cd-64b6-5e4d-835c-5dd050705583`, a content-independent
  UUIDv5 constant confirmed against the engine; §28.1 minimal resolution).
  A preset picker arrives with the CP-0.5 resource index.

## Event loop

`appname_doc_poll_events` is drained on a 50 ms timer plus immediately
after every mutating call. `appname.event.dirtyRects.v1` repaints ONLY the
named document regions (readback per rect into the canvas cache);
`appname.event.history.v1` refreshes the info/history models (undo/redo
depths, layers panel, history panel). Unknown schemas/fields are ignored
per the ES §2.8 mirror rule.

## v1 scope notes

- Readback happens on the UI thread (v1 ABI is synchronous; regions are
  viewport/dirty-rect sized and the full-canvas rebuild is chunked at
  2048×2048 « the ABI's 2^26 px cap). The §9.4 off-thread guidance and the
  shared-texture path are CP-0.5.
- Layers panel v1: list + visibility toggle + opacity slider. The controls
  render **disabled with a milestone tooltip** ("layer.set_props", 0.5) —
  never faked. Active-layer selection (the stroke target) is pure UI state.
- Trade dress D-43: generic layout and names ("Brush", "Eraser", "Layers",
  "History"), text buttons, no iconography, `Basic` controls style.
