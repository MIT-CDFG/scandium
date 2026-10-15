<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (c) 2026 the Appname contributors -->

# CHECKPOINT-REPORT CP-0.3 — "editor core exists"

| | |
|---|---|
| Tag | `v0.3.0-alpha` |
| Closed | 2026-08-12 (CP-0.1 tagged the same day; build opened 2026-08-11) |
| Main at close | `93a2c2e` (report + tag commits follow) |
| Discipline | REPORT-A: nothing below that a command didn't print. Evidence from O-INT's own final-state runs; full trails in `reports/*.md` and merge commits. |

## 1. Exit gates (plan §2.2) — final-state runs by O-INT on `93a2c2e`

| Gate | Result | Evidence |
|---|---|---|
| All CP-0.1 gates (`scripts/gate.sh`: build, test, deny, SPDX, contract-sync, walled-symbols) | **PASS** | `gate: all steps PASS`, exit 0 |
| Blend goldens | **PASS** | 18 tile + 1 document-walk goldens vs QA's independently hand-derived §4.4/§4.5 oracles, byte-enforced every `cargo test`; the spec's own §4.4 worked example included (its ES:2663 golden-suite mandate) |
| Brush-stroke goldens | **PASS** | 4 assembled-engine stroke goldens + selftest reference (`2484d11a466f4a1a…82a181`, 527 851 B, state_hash `71ef71521484…`), regenerable byte-identically |
| Stroke replay determinism (with asset blobs) | **PASS** | `replay_determinism: PASS` (runner: stroke export change, save-load-save, undo==pre-stroke/redo==stroked byte-exact ×2 runs); `package_roundtrip: PASS` incl. asset GC probes |
| QML app launches, paints, undoes on macOS | **PASS** | windowed by-hand check by the user, reported "pass" verbatim (integration log, 2026-08-12); O-INT's own `--screenshot-tour` on the same state: 6/6 frames, `brush.stroke committed=true`, `eraser.stroke committed=true`, journal-true undo; UI lane log records double-run byte-identical frames |
| C smoke via header | **PASS** | cabi suite 36/36 incl. C99-pedantic smoke against `include/appname.h` (ABI 1.1): asset create → stroke → red-pixel readback → undo-to-white |
| i18n audit (no hardcoded strings) | **PASS** | `AUDIT PASS` — 32/32 emitted ids covered in en/es/ja/pseudo (196 catalog keys each), mint register 23 entries, switch-check 4 locales byte-distinct; shell renders through catalog lookup with key-echo fallback |
| Workspace tests | **499 passed / 0 failed** | `cargo test --workspace` |

Registry three-way equality (== `_index.yaml` == 102) remains a standing green test.
**Kinds implemented: 3/102** (`doc.new`, `brush.stroke`, `eraser.stroke`) — the exact §2.2 delta.

## 2. Delivered scope vs the §2.2 table

- **CPU compositor (O-RENDER):** linear-premul f32 tile compositing, 8 blend modes vs the
  W3C/§4.5 formulas (hand-derived oracles, HDR unclamped per spec arithmetic), dirty-region
  §3.7, cross-checked bit-strict vs core's export. Perf round: bit-exact 256-entry sRGB
  decode LUT, keeper-Arc working-tile memo cache, in-place recomposite + Uniform kernel —
  all byte-equality-proven; zero-alpha uniform skip pinned NORMATIVE (§4.4 identity; the
  CP-0.5 GPU path must inherit it — recorded handoff). This CPU path is the permanent
  §34.3 parity oracle.
- **Brush engine v1 (O-TOOLS):** pure, deterministic (162-cell frozen matrix); centripetal
  Catmull-Rom march, Fritsch–Carlson dynamics, PCG32 (published-vector-pinned), §15.5
  WGSL-mirror stamping; `appname.samples.v1` blob codec; clean-room verified by inspection
  (memo-concepts only; spec-cited public bases). KindImpls hosted in `appname-tools`.
- **C ABI v1 (O-SCRIPT):** 21 symbols, ABI **1.1** (N.5 additive bump for
  `appname_asset_create`, the §9.2-normative ingress); assembled engines (stroke kinds live
  through the ABI); §9.5 events derived from receipt truth (`ApplyReceipt.dirty` /
  `HistoryRow.dirty`, multi-rect documented); WD-1 raw bytes end-to-end (ABI wire byte-equal
  to in-process twins); 20/20→21/21 unwind-guard census, leak-clean, opaque-cookie hardened.
- **Qt/QML shell v1 (O-UI, standalone session):** pure C-ABI client (no engine math —
  separation swept); canvas readback blit, zoom/pan, brush/eraser with live echo +
  committed strokes, layers/history panels, undo/redo, i18n'd with key-echo fallback,
  deterministic `--screenshot-tour`. Qt 6.11.1 dynamic-link under `deps/qt6.yaml`
  (LGPL-3.0-only arm elected, link-don't-vendor).
- **i18n from day 1 (O-I18N-A11Y):** 419-id registry census, catalogs en/es/ja/pseudo,
  mint-and-log register (23), registry-vs-emitted audit with drift tripwires (fired once in
  production on the stroke wave and was resolved same-day — working as designed),
  runtime-switch evidence.
- **Golden expansion (O-QA):** blend + stroke + selftest corpora, engine-produced and
  byte-enforced; cp03 expectation overlays (BareCore AND Assembled legs both 165/0/1);
  101-kind NI sweep through the shipped binary; runners live incl. asset-GC probes.
- **Beyond the table (arbitrated additions):** receipt dirty rects; samples as
  content-addressed package assets with journal-referencing GC; suffix-aligned reopened
  history (now the engine default; five-row behavior table pinned through the ABI);
  criterion perf baselines (`=0.8.2`, Apple M4 Max tier) + the standing >15% wave-gate;
  per-edit quick-gate hook; walled-symbol sweep extended to tests/fixtures/i18n.

## 3. Review + audit trail

Every lane branch passed a §5.2 review before merging (builders ≠ reviewers throughout);
several rounds returned **zero findings** (brush engine, stroke wiring, QA slice B, suffix
history, wave-2c). Notable episode, recorded in full in the lane logs: a mandated tripwire
test **falsified its own review's reachability premise** (zero-alpha Uniforms are loader-
reachable today); the builder strengthened rather than softened, and the re-review confirmed.
**O-WATCH** (independent auditor, `reports/WATCH-log.md`, committed with this report):
Review #1 verdict PASS — 0 CRITICAL / 3 MAJOR / 5 MINOR; its two tag blockers were resolved
before this tag (MAJOR-1 straight-f32 export fix — landed and fixture-pinned; MAJOR-2
`import_layer` journal-bypass — arbitration ratified in the integration log and mirrored in
the core lane log, with the binding CP-0.5 reconciliation plan); MAJOR-3 (sRGB powf) landed
as LUTs in both halves; all five MINORs closed.

## 4. Rights & calendar (statuses read 2026-08-12; select-and-screen due diligence, not a release claim)

Quoted statuses: US8811749B1 "Active, expires 2029-04-27"; US8963960B2 "Active, expires
2032-02-01"; GB2579434 "Active (Granted)", anticipated expiration 2039-09-16; US11551337B2
"Active". No lapse events. The two August fee dates (08-19 grace end; 08-24 fee due)
remain in the future — outcomes re-checked at the next checkpoint boundary. Walled-symbol
gate green throughout; no date-gated engine exists in the tree.

## 5. Honest limits at this tag

3/102 kinds; color management remains the documented interim sRGB assumption (O-COLOR
activates at CP-0.5); CPU compositing only (wgpu, full 27 modes, pass-through groups at
CP-0.5); §11.1 locks unreachable from public surfaces until `layer.set_props`; §6.4
reader-sanitize scheduled (CP-0.5, grounded analysis on file); `import_layer` stays the one
ratified out-of-spine IO door until its CP-0.5 command-form reconciliation; shared-texture
canvas path, qt-ads docking, and corrosion build refinement deferred with records pending;
tour PNGs are acceptance artifacts, not goldens. Deps: every external carries a `deps/`
record; THIRD-PARTY-NOTICES generation is the CP-0.7 packaging wave.
