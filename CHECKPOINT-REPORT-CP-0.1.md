<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (c) 2026 the Appname contributors -->

# CHECKPOINT-REPORT CP-0.1 — "headless kernel"

| | |
|---|---|
| Tag | `v0.1.0-alpha` |
| Closed | 2026-08-12 (build opened 2026-08-11) |
| Main at close | `3a1f909` (report + tag commits follow) |
| Discipline | REPORT-A style: nothing below that a command didn't print. Evidence lines are verbatim from O-INT's own runs; full trails in `reports/*.md` and the merge commits. |

## 1. Exit gates (plan §2.1) — all run by O-INT on merged main

| Gate | Result | Evidence (O-INT run) |
|---|---|---|
| `scripts/gate.sh` (build, test, deny incl. advisories, SPDX, contract-sync, walled-symbols) | **PASS** | `gate: all steps PASS`, exit 0 (final run on `3a1f909`) |
| Registry three-way equality (registry == `contracts/commands/_index.yaml` == 102) | **PASS** | `test three_way_equality_registry_equals_index_yaml_equals_102 ... ok` |
| Determinism replay byte-identical across two builds | **PASS** | two `CARGO_TARGET_DIR`-isolated builds; `selftest --keep` ×2 each; all four kept packages `sha256 3381811554784c0c95a80ddd74b218e0625f8364b539bb8560422892688f9a7d` (527133 bytes) |
| Package round-trip byte-stable | **PASS** | `package_roundtrip: PASS` (runner, exit 0; save→load→save stable, reject-intact, import-stage) |
| Adversarial suite 100% expected-outcome | **PASS** | `test cp01_corpus_aggregate_165_0_1 ... ok` (165 PASS / 0 FAIL / 1 declared SKIP of 166 engine-runnable; 8/8 runnable CLI D-cases in `cli_adversarial`) |
| Replay determinism end-to-end via CLI | **PASS** | `replay_determinism: PASS` (runner, exit 0; render sha `ed996f6cbbec74dc…` stable across runs) |

Workspace test count at close: **282 passed / 0 failed / 0 ignored** (`cargo test --workspace`).

## 2. Delivered scope (exact, vs the §2.1 table)

- **Repo bootstrap** (O-BUILD): 14-crate workspace, toolchain pin 1.97.1 (== proto), LICENSE
  GPL-3.0-only + LICENSE-SDK Apache-2.0, `contracts/` mirror (117 files, byte-verified vs the
  frozen spec, script-generated sha256 manifest, drift gate), `deny.toml` permissive allowlist,
  `deps/` records ×7 (serde+serde_json, uuid, png, zip, clap, blake3), `scripts/gate.sh` +
  `ci_local.sh`.
- **Tile substrate v1** (O-CORE): `Tile<T,C>` 256², u8/f32 + `depth-u16` feature (f16 paused —
  needs `half` record), premultiplied per §6.4 with debug boundaries, blake3 content identity
  (ES §0.3/§3.8), COW `TileMap` + per-(kind,model) dedup, `Buffer`; no spill (0.5 scope).
- **Document graph v1**: layers/groups skeleton, uuid identities, z-order, selection slot
  (f32 mask type), `document.v1.json`-exact serde (`resolutionPPI` wire spelling).
- **Command engine**: envelope parse/validate (proto taxonomy port; raw-bytes API), mechanically
  generated 102-kind registry (checked-in + regen script + drift tests), pinned 8-stage
  validation order (documented), `error.command.not_implemented` for the 101 future kinds.
- **`doc.new` + journal + undo**: 1/102 kinds implemented per contract (26-case reject matrix
  green); append-only journal; undo via COW tile deltas (ES §10.4 — deliberately NOT the
  proto's whole-layer snapshots); §40.2 O(1) handle inverse for doc.new.
- **`.appname` package v1**: mimetype first+stored (`application/x-appname` at byte 38), §7.6
  step-4 byte-lex total order + step-5/6 fsync durability, §7.4 chunk codec (bomb gate), §7.8
  journal-in-package (corrupt-tail = warnings, never fatal), §2.8 canonical document.json,
  atomic save; extension `.appname` (ES §7). Validate-on-load typed end-to-end (zero panics
  across the malformed-package corpus).
- **PNG import/export**: recorded `png` crate; linear-f32 boundary under the documented
  CP-0.5-interim sRGB assumption; non-rgba modes refuse typed until §5/§6.
- **Headless CLI v1** (`appname`): ten verbs (new/apply/script/undo/redo/history/export/import/
  info/selftest), exit 0/1/2 per proto, `--json`, transactional `script`, i18n chain
  (env dir → exe-adjacent → embedded en → key-echo; en/es/ja/pseudo seeds).
- **Test harness v1** (O-QA): 178-case adversarial corpus (proto's 41 ported with evidence
  trail; 102/102 kind coverage self-checked; byte-deterministic generator), conformance crate
  (WD-1 raw-byte feeding with per-slice fidelity gate), 24 engine-produced goldens + selftest
  reference (byte-enforced every `cargo test`; 12 non-rgba export goldens explicitly deferred
  to CP-0.5), live property suites, CLI adversarial suite, workflow runners.

## 3. Review trail (builder/reviewer split per §5, all rounds pre-merge)

| Slice | Verdict trail | Defects (all closed pre-merge) |
|---|---|---|
| Bootstrap | R1 0 blockers/2 minors → fixes → R2 +1 residual minor → fix → O-INT independent rig 9/9+8/8 | 3 |
| Tiles+graph | R1 1 blocker (`resolutionPPI`)/5 minors → fixes → delta re-review PASS | 6 |
| Command engine | R1 1 blocker (stage-1 `samples` preempting stage 4)/4 minors → fixes → PASS; corpus harness 165/0/1 | 5 |
| Package/PNG/CLI | R1 PASS 0 blockers/4 minors → fixes → PASS (independent TZ/cwd-varied determinism reruns) | 4 |
| QA activation | R1 PASS 0 blockers/2 minors → fixes (O-INT diff-verified) | 2 |

## 4. Rights & calendar (standing §3.1 duties — statuses read 2026-08-12, Google Patents; select-and-screen due diligence, not a release claim)

- US8811749B1 — quoted status "Active, expires 2029-04-27". The 12th-year-fee grace window
  (ends 2026-08-19) has not yet closed; lapse question undeterminable until after that date.
  Re-check at next checkpoint boundary.
- US8963960B2 — quoted status "Active, expires 2032-02-01" (matches the C5 recheck). 11.5-yr
  fee due 2026-08-24, grace to 2027-02-24 — pending; outcome bears on the 2027-03-06
  seam-gate decision.
- GB2579434B — quoted status "Active" (granted 2021-10-06, anticipated expiration 2039-09-16).
  Renewal due 2026-09-16 — pending.
- US11551337B2 — quoted status "Active legal-status Critical Current" (anticipated expiration
  2038-11-29). First-fee grace to 2027-01-10 — pending. CAF-area discipline unchanged
  (deterministic exemplar only; no NNF structures — enforced by walled-symbol gate + tests).

No date-gated engine is present in the tree (walled-symbol gate green throughout; D-44 flags
arrive only at CP-1.0 scope).

## 5. Upstream / fold-in register (nothing here blocks CP-0.1)

- 12 recorded divergences in `reports/O-CORE-log.md` (taxonomy spellings, §9.6 unknown-key
  ignore-vs-reject, stored-vs-deflate container, §7.7 unknown-kind deferral, etc.) — for the
  v5.6 fold-in session in the spec repo.
- OQ-1: `error.command.not_implemented` absent from v5.5 `message_ids.yaml` — mint-and-log
  candidate (O-I18N-A11Y at CP-0.3).
- Open: OQ-6 (validate-only engine entry), OQ-7 (`state_hash` surface on `info --json`),
  `half`/zstd/lz4/jsonschema/jpeg records when their consumers arrive.

## 6. Honest limits at this tag

Headless kernel only: no compositor, no GUI, no C ABI yet (all CP-0.3 scope). 1/102 command
kinds implemented; color management is the documented interim sRGB assumption; no spill/LRU
(bounded-memory gigapixel work is CP-0.5); PSD/ORA and non-PNG codecs absent by plan. The
windowed by-hand launch check is a CP-0.3 gate, not claimed here.
