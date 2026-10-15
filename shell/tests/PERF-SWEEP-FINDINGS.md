<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (c) 2026 the Appname contributors -->

# Shell perf sweep — findings record (2026-08-20)

Operator-priority QML/composite-bridge sweep + the masked-redraw probe
(the step-2 gap: the desk's engine benches are non-mask; this is the
shell-side mask cover). Instrument: `tst_perf_probes` (this directory —
see its header for invocation and discipline). Numbers: **Release**
(`target/shell-relbench`, CMAKE_BUILD_TYPE=Release, no bundle flag;
release cabi), Apple M4 Max, square documents on a 512/1024/2048 size
axis (single-call probes at 2048²), median of 7 warm reps
after a discarded warm-up, **A/B/A paired** (config A re-measured after
B; drift bounds machine-load contamination — worst observed 4.1%,
composite legs ≤1.1%). Three full Release runs agreed within ~1% on every
composite leg (the size-axis run is the reported set). Machine-load lines are in the probe output
(load1 5.7-9.1 of 16 cores across runs). Fixes below are PROPOSED SHAPES for
later reviewed slices — nothing here changes product code.

Desk context (engine triage, folded in mid-run): the engine side ruled
NO composite regression (export_png −41% win; recomposite_dirty +0.9%);
this record therefore carries the mask **absolute-cost** story, and its
credible-user-visible findings are shell-side by construction.

## Masked-redraw probe (the authoritative numbers)

Full-composite readback, `readRegionImage` full-doc (the canvas
presentation path — rebuildCache/document-reset shape), per-layer raster
masks from an elliptical selection (§11.2 slot A serving), over the
CANVAS-SIZE axis (aud-perf r2 requirement) × layer count:

| doc | layers | unmasked med | masked med | ratio | A/A′ drift |
|----:|-------:|-------------:|-----------:|------:|-----------:|
| 512² (0.26 MP) | 1 | 3.8 ms | 6.2 ms | 1.64× | 0.967 |
| 512² | 4 | 9.2 ms | 21.0 ms | 2.30× | 0.991 |
| 512² | 8 | 15.5 ms | 40.3 ms | 2.60× | 1.017 |
| 1024² (1.05 MP) | 1 | 13.7 ms | 24.3 ms | 1.78× | 0.955 |
| 1024² | 4 | 28.5 ms | 81.5 ms | 2.86× | 0.990 |
| 1024² | 8 | 44.2 ms | 155.9 ms | 3.53× | 1.008 |
| 2048² (4.19 MP) | 1 | 46.8 ms | 90.5 ms | 1.93× | 1.008 |
| 2048² | 4 | 95.8 ms | 300.0 ms | 3.13× | 0.999 |
| 2048² | 8 | 142.6 ms | 572.3 ms | **4.01×** | 0.989 |

Full readback scales ~linearly with area (per-Mpx cost mildly SUB-linear:
59 → 42 → 34 ms/Mpx unmasked 8L as size grows — fixed overheads
amortize). The mask multiplier grows with BOTH axes (1.64× → 4.01×).

Dirty-region loop (8 layers; small brush-stroke commit + FIXED 56×56
region readback — the interactive redraw shape), split-timed, over the
canvas-size axis. **r2 localization verdict:** at a fixed dirty size the
readback does NOT grow with total canvas px — the O(total-tiles)
recomposite term the desk flagged engine-side is NOT the interactive-
redraw dominator at ≤4 MP as seen from the shell door; per-dirty
compositing + the mask isolate multiplier dominate:

| doc | unmasked readback med | masked readback med | mask ratio | A/A′ drift |
|----:|--------------------:|-------------------:|-----------:|-----------:|
| 512² | 6.31 ms (min 3.20) | 19.17 ms (min 9.60) | 3.04× | 1.001 |
| 1024² | 3.14 ms | 9.88 ms | 3.15× | 1.033 |
| 2048² | 2.92 ms (min 1.61) | 9.75 ms (min 8.83) | 3.34× | 1.014 |

(The apparent INVERSION — 512² slower than 2048² — is CONTENT, not
canvas size: the probed positions at 512² sit where the recipe's
diagonal strokes cross (Dense tiles), at 2048² mostly on flat fill
(`TileView::Uniform` fast path, `png_io.rs`); 512² medians carry a 2×
min/med spread — treat them INDICATIVE-ONLY. The structural conclusion
rides the whole curve: no growth with total canvas px.)

Stroke-commit half: 0.75-0.88 ms per small stroke at every size (mask
ratio 1.06-1.18× — mask cost lives in the readback, not the apply).

Mechanism (derived from code, RULE 8): a masked layer takes the §4.7
row-4 **isolate** arm in the render compositor
(`crates/appname-render/src/compositor.rs` posture table) — per-tile
intermediate + `layer_effective_coverage` per pixel — vs direct
source-over when unmasked; no occlusion culling exists (absent tiles
even run the arithmetic for bit-exactness, `png_io.rs
composite_frame_tiled`), so the ratios are honest mask math, not a
culling artifact. Absolute interactive cost stays single-digit ms per
edit; the full-doc masked rebuild (572 ms at 8L/2048²) is the number to watch
wherever `rebuildCache` fires (document reset / geometry change).

## Findings table

| # | Finding | Code cite | Measured (Release) | Severity | Proposed fix SHAPE (not implemented) |
|---|---------|-----------|--------------------|----------|--------------------------------------|
| F1 | Marching-ants refresh does a FULL-canvas selection-mask readback + revectorization INSIDE `paint()`, and `m_selectionRevision` bumps on EVERY history event — so while a selection is active, *any* command (each brush-stroke commit included) re-pays it on the next frame | `canvas_view.cpp:334` (`refreshAntsIfStale` in `paint`), `:1611-1658`; `document_controller.cpp:1370` (revision bump on any journal transition) | full-doc `readSelectionImage` = **48.2 ms** (vectorize cost additional, unmeasured separately) | **credible-user-visible** (per-edit frame hitch with a live selection; scales with doc area) | bump the revision only on selection-mutating kinds (or read selection stamp from info); vectorize off the paint path (cache path per revision, compute on commit not on paint); window the mask read to the viewport |
| F2 | Post-commit full-viewport repaint defeats the partial dirty path: `infoChanged → update()` (full item) fires on every applied command, subsuming `refreshRegion`'s `update(viewRect)` | `canvas_view.cpp:77-78`; partial path `:1105-1122`; `pollNow`/`refreshInfo` emit chain `document_controller.cpp:1367-1382,1504` | 40×40-doc-px commit repainted **full_frac = 1.000** of a 1280×800 item (probe `paint-small-commit`); ~2.1 ms/full software repaint (INDICATIVE — offscreen software SG) | **credible-user-visible** (every edit repaints the whole canvas item; cost scales with window size and hides the dirty-rect machinery that already works) | emit a scoped `overlayChanged` (guides/grid/selection deltas) instead of blanket `infoChanged→update()`; let composite dirt ride the existing partial path; full update only when overlay truth changed |
| F3 | Move-tool PRESS does a synchronous full-doc per-layer readback for the drag echo | `canvas_view.cpp:530-538` (item-10 capture); door `layerThumbImage` `document_controller.cpp:650-661` | **93.3 ms** per press (8L doc, unmasked layer, full 2048² read) | **credible-user-visible** (press-latency stall; worse on larger docs) | read the viewport-intersected rect only; capture lazily on first move past slop; keep the full read off the press path |
| F3b | …and on a MASKED layer that read always refuses typed → silent fallback to whole-composite echo | engine truth: `crates/appname-cabi/src/composite.rs:240-246` (`layer_region_rgba8` refuses "mask application is the render row") | refusal itself ~0 µs (cheap, typed) | theoretical (echo fidelity, not perf) — recorded for the ABI ask ledger | the exclude-layer/masked-readback ABI ask already filed (item-10 note); no shell fix meanwhile |
| F4 | Selection-bounds announcement scans the FULL canvas mask synchronously on every applied selection.* command | `document_controller.cpp:284-315` (`announceSelectionState`, strip reads) | **52.5 ms** per selection commit (scan equivalent; announcer dispatch not included) | credible-user-visible when stacked: marquee release ≈ applyCommand 35.8 ms + scan 52.5 ms + next-paint ants read 48.2 ms ⇒ **~140 ms** release stall at 2048² | share ONE per-revision mask read/bounds between announcer + ants (cache on DocumentController); or bounds-from-info when the engine grows selection.bounds |
| F5 | Every gesture mouse-move calls full-item `update()`; no partial invalidation for overlay deltas | `canvas_view.cpp` mouseMove arms `:606-741`, echo `:2087-2100`, hover `:762-769` | 60-move drag → 28-30 paints (scene graph coalesces ~2 moves/paint) but **every paint = full item** (full_frac 1.000, marquee AND brush) | moderate (coalescing caps paint rate; each paint still full-item — cost grows with window size; software-SG per-paint ~2.1 ms INDICATIVE) | `update(prev∪curr overlay bbox)` per move for marquee/lasso/pen/echo arms; keep full update for zoom/pan |
| F6 | Per-commit UI-thread chain re-parses info + FULL history JSON every history event | `document_controller.cpp:1333-1382` (`pollNow`), `:1514-1530` (`refreshHistory`, O(history)) | applyCommand(selection.set) round trip **35.8 ms** (engine rasterize dominates); +300 history entries moved the median +2.1%/+1.2%/+8.9% across three runs — **in-noise at this scale, INDICATIVE** | theoretical at v1 session lengths (linear growth exists in code; not measurable at 300 entries) | incremental history (append from receipt; full re-fetch only on undo/redo); no urgency |
| F7 | `commandAvailability()` rebuilt per query (keypress chords, twice per layer switch) | `document_controller.cpp:485-587`; call sites `canvas_view.cpp:907-975`, `document_controller.cpp:75-79` | **4 µs** per call | **cleared** — not a finding | none needed |
| F8 | Stroke input path is client-buffered — NO per-move ABI call or engine recomposite during drags | `document_controller.cpp:1222-1227` (`strokeAppend` buffers), engine apply once at `strokeEnd` `:1229-1305` | stroke commit 0.75-0.88 ms per small stroke | **cleared** (suspect 5's worst case absent) | none needed |

## Not measurable in this harness (honest list)

- Live cocoa/Metal frame pacing and the QQuickPaintedItem texture-upload
  cost of full-item repaints — probes run offscreen with the software
  scene graph; per-paint µs are INDICATIVE, counts/areas are structural.
- Input-to-photon latency (no display); the A/B/A discipline bounds
  machine-load only for the ABI-call timings.
- Announcer dispatch cost in-app (harness wires no Announcer; F4 measures
  the scan the announcer performs, not the a11y emission).
- Thumbnail-provider traffic (`layer_thumb_provider` is QML-driven; no
  QML here). Its door is the same `readLayer` as F3 — per-call cost
  equals F3's number; trigger frequency unmeasured.
- Vector masks / multiple masks per layer (probe covers one raster mask
  per layer, §11.2 slot A); >8 layers; >4.2 Mpx documents (the size axis
  tops out at 2048² — the r2 curve extrapolates flat for the dirty loop,
  but 16-64 MP documents are unprobed first-hand).
- Debug-config numbers for the heavy composite probes (unoptimized
  engine exceeds QTest's 300 s function watchdog — see harness header).
  Config-dependence WAS verified first-hand on the structural probe:
  Debug reproduces the identical repaint structure (29 paints/60 moves,
  full_frac 1.000 both gestures, small commit = full-item repaint);
  Debug latencies are non-reportable by policy.

## Severity ranking (which fixes deserve slices)

1. **F1 + F2 together** (one slice candidate: "commit-path repaint
   scoping") — the pair makes every edit-with-selection pay full-canvas
   mask read + full-viewport repaint; both are shell-side and
   mechanical.
2. **F4** (fold into the same slice or a small follow-up: share the
   per-revision mask read) — pure duplication of F1's read.
3. **F3** (move-press echo windowing) — one call site, big constant.
4. **F5** (gesture partial invalidation) — moderate, wide but mechanical.
5. **F6/F3b** — recorded, no slice now.

Masked-composite absolutes (the desk's criterion input): ratios above;
interactive dirty-region masked cost stays ~10 ms/edit at 8L across
the size axis, canvas-size-independent — the full-doc masked rebuild
(0.57 s at 8L/2048²) is the one to keep rare.
