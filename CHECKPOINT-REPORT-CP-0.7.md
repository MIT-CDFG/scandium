<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (c) 2026 the Appname contributors -->

# CHECKPOINT REPORT — CP-0.7 "power beta" (FINAL — tag candidate pending row zero)

| | |
|---|---|
| **Written by** | O-INT; interim 2026-08-15 under O-WATCH review #7 (adopted in full); **finalized 2026-08-16 after the §12.1 render row landed and the full ledger re-ran** |
| **History** | The `v0.7.0-beta` tag applied 2026-08-15 was DELETED under review #7: it was placed without the O-WATCH pre-tag pass, and two ledger rows had been closed by O-INT self-ruling. Remediation (all complete): §12.1 render row landed (core evaluation slice + render integration, each through full §5 review cycles with one escalated-and-ruled item); the reopened rows now close AS TESTS; the 27-row ledger re-ran fresh (O-QA, zero rulings). **Row zero remains open by design: O-WATCH pre-tag PASS on the exact candidate + the operator's explicit word.** |
| **Discipline** | REPORT-A style: nothing claimed here that a command did not print. Evidence lines are quoted from gate/suite/workflow output recorded in `reports/INTEGRATION-log.md` and the lane logs. |
| **Prior tag** | `v0.5.0-beta` (still the newest tag until row zero closes) |

## 1. Exit-gate ledger (plan §2.4) — ALL ROWS CLOSED AS TESTS (row zero pending)

| Gate | Status | Evidence anchor |
|---|---|---|
| 68/102 kinds | **68 implemented** (registry gate; `assembled kinds: pre 59 -> post 64` + doc.new + 2 daggers + doc.duplicate; `filter.smart_add` live through the chain-insert door) | O-TOOLS log wiring + tail-flip entries; `cli_ni_sweep PASS (35, 66)` |
| OpNode re-render correctness goldens | **CLOSED AS A TEST (2026-08-16)** — the §12.1 render row landed; the tripwire fired its designed handoff and was converted to a live verify; COMPOSITE-level goldens regenerated under O-QA review (four full-byte station composites + composite hashes at every station, two-run byte identity, bite-asserted premises), plus the 4-probe multi-tile seam cross-check suite (byte-strict render-vs-core) | O-QA goldens-round entry |
| PSD subset + compat reports | **PASS** — corpus-subset suite (18+ tests), §42.4 schema-tagged reports, W07 6 byte-level cases green with composite+report goldens | O-IO log; `W07 psd-migration: PASS` |
| W02 non-destructive | **CLOSED AS A TEST (2026-08-16)** — `W02: PASS, 10/10 legs, exit 0` end-to-end including the composite-delta legs (adjustment visible in export, toggle-off == E0 byte, reopen re-render == E2), run by the §12.1 slice, its reviewer, and the O-QA ledger round independently | O-QA ledger row; §12.1 review |
| W07 / W16 / W20 / W25 / W26 | **ALL PASS, re-run fresh in the final ledger round** (W16 now **12/12** including a real composited-content assertion — post-edit export byte-equals a raster-path reference; W20 exit-7 batch semantics; W25 lossless spill, RSS 819≤1280 MiB tier bound; W26 against a committed v0.5.0-beta-binary fixture with provenance). QA-F8 is CLOSED: serve-or-refuse held at every door through the whole §12.1 row | O-QA ledger round |
| W01/03/04/05/06 (CP-0.5 gates) | **ALL PASS, re-run at this boundary** (not carried on faith) | O-QA addendum: verbatim close lines per gate |
| Fuzz soak | Recorded: 2 targets, 868M + 886M execs, 240 s each, **0 findings, 0 OOM** — honestly scoped unsanitized+unguided; `fuzz/` outside the workspace per the ratified NCSA/LLVM-exception posture; nightly entry point committed, host provisioning still owed | O-QA wave-2 entry |
| Ports wave 1 landed WITH notices | **DONE — and wave 2 as well** (ahead of plan, operator-directed): items 1–12 merged with full clean-room review trails; notices MECHANIZED — port-obligation records with landing detection, `active-ports=6 pending-ports=0`, vendored-C texts sha-pinned live-extracted (`vendored-texts(active)=7`), drift red-paths selftest-executed | O-ALGO/O-BUILD logs; gen_notices output lines |
| Registry three-way equality | Standing gate green (contract-sync `117 files verified`; registry == contracts == 102) | every gate run |
| Parity ≤ 1/255 | Standing (CPU/GPU harness in-suite; `gpu_parity` green in the render suite) | O-RENDER log |
| Perf (no regression vs recorded baselines) | **`PERF-GATE PASS: 8 benchmark(s) within +15%`** re-run 2026-08-16 on a quiet host over FINAL main (covering the full §12.1 row): worst +5.0% (fill_apply), recomposite_dirty −45.1%; baselines deliberately unmoved. (The earlier +70% flag, QA-F13, was proven host contamination by a paired A/B/A and closed 2026-08-15) | watcher comparator output, INTEGRATION-log |
| Gate suite | **`gate: all steps PASS`** — 8 steps (build, test, deny, spdx, contract-sync, **notices**, **i18n-audit**, walled-symbols; two steps ADDED this checkpoint) on the final tip; workspace suites 0 failed throughout the close sequence | gate-final.log |
| Calendar (§3.1) | Boundary re-reads 2026-08-15: US8811749B1 "Active, expires 2029-04-27"; US8963960B2 "Active, expires 2032-02-01" (verbatim, Google Patents). On-date checks 2026-08-19 / 2026-08-24 scheduled as durable app-level tasks with a manual backstop | WATCH-log |

## 2. Delivered beyond the §2.4 rows (operator-directed accelerations)

- **ABI 1.2 + 1.3** (release-block discipline restored under the "checkpoint tags are releases" ruling; census partition 20/1/7/1 machine-pinned): export door, selection readback, five-symbol recovery surface, info_json v2 + treeChanged, raster ingress, `appname_doc_apply_action`.
- **Actions/macros v1 + headless batch (W20/W21 groundwork) + appname-py v2** (§25.1 trusted tier, pyo3 0.29.2 under record).
- **UI**: full ABI-1.2 chrome (import/paste/drag-drop, export dialog, marching ants, layers-panel truth, W05 recovery prompt), P-round (dark theme AA-pinned, cursors, .app bundle with D-43 placeholder identity + icns provenance stage).
- **i18n**: catalogs 502 ids/locale ×4; audit machinery with shell sweep, family-construction census, RED-7 shipped-kind labels; registry coverage 120/419.
- **Text/vector (CP-1.0 groundwork pulled forward, standalone lane)**: HB 8.4.0 bring-up, §20.3 booleans, §23.3 layout, §44.2 fallback chain, glyph cache, §23.4 style model, §20.4 primitive constructors.
- **Ports wave 2 complete** (CP-1.0 pre-build): grabcut + own min-cut, MLS, edge-aware quartet (incl. the paper-only LLF), ACES translation, pano classical tier.

## 3. Honest limits (each with its gating row)

1. **§12.1 render row: LANDED (2026-08-16)** — core evaluation slice (ChainEvaluator, §12.1 formula verbatim, export-door serving, door-level dirty/T flip, R1 dirty regions per the contract letter with a pinned alpha-preservation guard) + render integration (composite_document serves proxies/chains/adjustments per-tile with the segment pipeline; refuse/serve asymmetry reviewed SOUND under the strict-subset single-truth invariant, pinned both directions). Residuals, each named and recorded: CLI `export` still routes parametric docs to core's narrower arm (a §4.5-faded smart doc renders but refuses at export; closure = route export through the compositor, riding the existing multi-raster precedent); GPU pairs for parametric content are FORCED-CPU with the deferral recorded (W24 first-class; no fake parity); isolated-group adjustment reach stays typed (§11.4's isolated clipping is future render work); the masked-adjustment dirty region is a recorded conservative superset (fold-in).
2. **Smart-object placement transforms** (§18.2) not landed: `doc.resize`/`doc.rotate`/non-origin `doc.canvas_resize` refuse typed on smart-bearing documents (guard at apply entry; replay identity preserved by construction).
3. **Findings register open**: QA-F2..F7, F9 (new-kind contract divergences → core reconciliation queue), F10 (cdylib stray-export census gap, failing-by-design pin, blocks SDK artifacts not this tag), F11 (per-arena id uniqueness), F12 (op.reorder clamp vs contract). All alarm-pinned in corpus rev 14.
4. **Core reconciliation queue** (next core slice): mergeKey §10.5 (three consumers), layer.duplicate idMap, op-stack entry ids, effective_locks transitivity arbitration, selection-not-persisted-across-CLI-invocations, CodecError direction observation follow-ups.
5. **Fold-in register** (spec v5.6 reconciliation, non-blocking): §48.1 HB/FT pins, §17.2 gamma formula, Maximum/Minimum gap, OQ-3 stage-3 mapping, §7.8 fsync divergence (SCRIPT-D1), SCRIPT-D2..D7, capture-position carriage design, Vatti arm election, layout pins, tofu metrics, mid-stack goto design, batch step_failed second id, milestone-caption designators, stale spec_refs (3 areas), §16.3-vs-ES:2617 doc reconciliation.
6. **Deferred rulings** (CP-1.0 wave-open, texts in hand): ES §13.3 region-grow vs ALGORITHMS §8.B; ES photo.merge posture vs plan §2.5 Photomerge/W12.
7. **Pre-guard scratch packages** from unreleased builds may refuse replay-as-recovery typed (no tagged build produced such journals).
8. Fuzz nightly host provisioning owed; sanitized/guided soak is future work.
9. lensfun-db (INTEGRATION-NOTES item 7) deferred by ruling to the CP-1.0 RAW wave with its CC-BY-SA-3.0 surface.

## 4. Rights posture

All rights work this checkpoint is select-and-screen due diligence with basis + source + read date (V39 discipline audited by O-WATCH: six consecutive PASS reviews through #6). Date-gated engines remain compile-off (walled-symbols gate green at every merge). The notices pipeline is mechanized end-to-end (ports, vendored-C texts, asset records) with drift enforced at the gate. OD-7 (counsel FTO) remains the release gate; nothing herein is a release claim.

## 5. Process note

This checkpoint absorbed three orchestrator session boundaries and four builder/agent deaths mid-round; the §3.15 recovery drill (plan + lane logs + git history) reconstructed full state each time with zero lost work. Review discipline findings: 2 CLI-reproduced data-loss blockers, 2 shipped-pixel-wrongness majors, 1 composed-traversal security breach, 1 latent CP-0.5 bounds bug, and 1 ABI-versioning unsoundness were all caught by independent reviewers before merge — none survived to this tag.
