<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (c) 2026 the Appname contributors -->

# CHECKPOINT-REPORT CP-0.5 — "usable daily beta"

| | |
|---|---|
| Tag | `v0.5.0-beta` — **APPLIED** after O-WATCH pre-tag PASS (0/0/0, last-reviewed `0a1b9c7`) |
| Closed | 2026-08-13 |
| Main at draft | `6c9ea7e` |
| Discipline | REPORT-A: nothing below that a command didn't print. O-INT's own final-state runs; full trails in `reports/*.md` and merge commits. |

## 1. Exit gates (plan §2.3) — final-state runs by O-INT on `6c9ea7e`

| Gate | Result | Evidence |
|---|---|---|
| All prior gates (`scripts/gate.sh`) | **PASS** | all six steps, exit 0, over the full merged tree incl. the wgpu family |
| **50/102 kinds** — stated per O-WATCH's wording: **49 apply-live + the `doc.duplicate` dagger door** (§10.3 † surface = `Engine::duplicate` + CLI verb; its apply path answers `not_implemented` by design, corpus-pinned) | **MET** | registry three-way standing green; corpus rev 10 (48 finalized + the watched dagger arm); NI sweep 53+48=101+doc.new vs the shipped binary |
| Adversarial coverage for implemented kinds | **PASS** | corpus aggregate BareCore + Assembled both 165/0/1; per-kind adversarial suites at the lane bar; conformance 66/0/0 |
| **Parity ≤ 1/255 CPU-vs-GPU on the golden corpus (ES §34.3)** | **PASS — live Metal** | `gpu_parity_gate_on_available_adapter ... ok` (O-INT run, release); render-lane evidence: 162/162 cases, max-abs **0.0603/255**, SSIM 0.9999999997, Dissolve coverage+determinism exact; W24 forced-CPU floor first-class, `APPNAME_REQUIRE_ADAPTER=1` contract for CI tiers |
| W01 / W03 / W04 / W05 / W06 | **ALL PASS** | verbatim outputs in the QA evidence package (`reports/O-QA-log.md`, "CP-0.5 CLOSE" entry): W01 byte-exact export walk; W03 150 spills, resident 0 B over budget, peak RSS 3088 MiB ≤ 4096 tier; W04 assign-untouched / convert-moved / undo-restores; W05 abort-134 / base-untouched / recover-exact-3; W06 four locales + switch evidence |
| Spill bound recorded | **YES** | §10.4 history spill: zstd-3/lz4 tiers, newest-8 floor; W03 numbers above; scratch format BGSCRT01 |
| ORA round-trip | **PASS** | §8.4-normative both directions (16-op map, mergedimage, entry order, 16-bit-for-f32); journaled landing with decoded names; fixtures per `ora-fixtures.yaml` executable subset (PARTIALs registered as typed deviations) |
| Replay determinism + package round-trip (CLI, end-to-end) | **PASS** | `replay_determinism: PASS`, `package_roundtrip: PASS` (O-INT runs) incl. journaled-import undo across the package boundary and the asset-GC pair |
| Perf baseline recorded | **YES** | 8 criterion benches (`=0.8.2`), tier `31-39-73` / Apple M4 Max, comparator 8/8; the >15% wave-gate armed and exercised all checkpoint |
| i18n audit | **PASS** | 100% emitted coverage ×4 locales (audit + switch evidence); mint register current |
| Workspace tests | **866 passed / 0 failed** | `cargo test --workspace` (3 deliberate ignored: two lane helpers + render's nightly soak tier) |

**Daily-drivable claim:** a person can create, paint, fill, select, transform, manage layers,
color-manage (assign/convert), import/export PNG/JPEG/TIFF/WebP/GIF/BMP/ORA, undo/redo
across sessions, autosave-recover, and work on gigapixel documents in bounded memory —
through the CLI, the C ABI, and the Qt shell (CP-0.3-era chrome; the full §2.3 UI chrome
row belongs to the standalone O-UI lane's ongoing arc).

## 2. Delivered scope (by lane, one line each — details in lane logs)

O-COLOR: CMM v1 (spec-verbatim matrix core + deterministic lcms2 backend, NOCACHE at type
level). O-CORE: spill, autosave/hard-kill recovery, selections/masks share + doors, color
wiring, journaled import (**`import_layer` deleted — the spine's last exception is gone**),
layer/guide/grid/merge families, suffix history, `--history-budget`. O-TOOLS: resample
kernels, transforms (DLT/warp), geometry engines, selection tiers (shared wand §22.6),
fill lift + live flood. O-IO: five-format codec matrix + EXIF + ORA, all ingestion
journaled. O-SCRIPT: family conformance rows, six ratified arms, receipt-truth events with
§3.7 ≤8 coalescing (64→1 pinned), framework link companion. O-QA: corpus rev 10, seven
golden sets (37-window blend set with doubly-independent oracles), five live W-gates, the
merge-bake watched-divergence tripwire, perf harness. O-RENDER (standalone): wgpu DAG v1,
27 modes GPU+CPU, groups, the §34.3 parity gate green on live Metal, 60-doc fuzz soak.
O-BUILD: 12 records incl. the pre-swept 124-package wgpu tree; IJG family ruled non-viable
under policy. O-I18N-A11Y: 212 keys/locale ×4, three tripwire catches resolved same-cycle.

## 3. Review + audit trail

Every branch §5-reviewed before merge (builders ≠ reviewers; many zero-finding verdicts);
the standalone render lane ran its own review discipline per slice, independently re-verified
where it crossed shared surfaces. Two incidents on the record, both resolved with standing
rules: one red-main window (corpus-companion sequencing — now gated by the QA-regen rebase
rule) and one security-warning on a V39 wording fix (verified false positive twice: O-INT
raw-diff + operator independent re-verification). O-WATCH reviews #1 and #2: all findings
closed; the pre-tag pass returned **PASS 0/0/0** and the tag was applied on the operator's word.

## 4. Rights & calendar (statuses read 2026-08-13; select-and-screen, no release claim)

US8811749B1 "Active, expires 2029-04-27" (12th-yr grace ends **2026-08-19** — six days out;
re-check scheduled); US8963960B2 "Active, expires 2032-02-01" (fee due **2026-08-24**);
GB2579434 "Active (Granted)" (renewal 2026-09-16); US11551337B2 "Active". No lapse events.
Walled-symbol gate green all checkpoint; no date-gated engine in the tree.

## 5. Fold-in / scheduled register (nothing blocks this checkpoint)

cropRect schema gap · §4.5/§34.3 "branch inputs correctly rounded" note + WGSL VividLight /
HardMix observations · subnormal-alpha working-domain floor question (div_cr) · merge-bake
divergence (tripwire armed for §5 unification) · zip-deflate election (with the
W17-unblocking wave; prerequisite for foreign-ORA interop claims) · stored Dissolve
coverage-mask golden (O-QA wave-2 harness pass) · TILEREF byte-capture escalation path ·
Appendix-B ProPhoto TRC addendum · taxonomy spellings + the accumulated divergence lists in
the lane logs · doc.duplicate's contract `scripting:` example applies it as an envelope while
the build's dagger surface is the engine door (MINOR-12 divergence).

## 6. Honest limits at this draft

§3.4 live-tile page-out NOT landed (history spill bounds delta growth, not resident
documents — the 3.1 GiB W03 characteristic stands recorded); merge-bake rides the interim
storage-space composite until §5 unification (watched, enveloped < 0.5); selection surface
is session-state by spec reading (§14.2 channels are the persistence path — CP-1.0);
foreign deflated ORA typed-refused pending the election; W17 blocked-honest on kinds that
arrive at CP-0.7/1.0; the shell's full editing chrome and shared-texture canvas are the
standalone O-UI lane's arc; CompositeBackend's fallible widening for §4.2
"degrade-never-abort" is a CP-0.7 growth note.
