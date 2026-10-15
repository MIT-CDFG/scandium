<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (c) 2026 the Appname contributors -->

# fixtures/ — O-QA test-input corpora

Spec: plan AGENT-ASSIGNMENTS-full-build.md §2.1 row "Test harness v1"; ES
§10.2 (envelope), §40.1 (error taxonomy); determinism spine D-48.

Owned by lane O-QA. Fixture and golden corpora are a cross-lane contract:
another lane needing a fixture change routes it through O-QA review (plan
§0.6 rule 6). Consumed by `tests/conformance` (crate `appname-conformance`)
and the `tests/workflows/*.sh` runners.

## Layout

```
fixtures/
  README.md                          this file
  adversarial/
    generate_corpus.py               deterministic generator (see below)
    manifest.json                    counts, taxonomy tiers, id schemes,
                                     input sha256s, open-question pointers
    ported/
      envelope.json                  29 envelope cases ported verbatim from
                                     appname-proto/scripts/adversarial.jsonl
      cli.json                       12 CLI driver cases transcribed from
                                     appname-proto/scripts/adversarial.py
                                     (together: the proven 41-case suite,
                                     REPORT-A §4.2 — 41/41 PASS, no panics)
    generated/
      envelope_rejects.json          envelope-stage rejects derived from
                                     contracts/schemas/command-envelope.v1.json
      kinds/<kind>.json              one file per kind in
                                     contracts/commands/_index.yaml (102):
                                     doc.new carries its param-validation
                                     reject matrix; every other kind carries
                                     its CP-0.1 not-implemented case
```

## Case record (uniform across all corpus files)

| field           | meaning                                                        |
|-----------------|----------------------------------------------------------------|
| `caseId`        | stable id: `P-*` ported, `G-NI-*` not-implemented, `G-DN-*` doc.new matrix, `G-ENV-*` envelope-stage |
| `driver`        | `apply` (envelope through the engine apply entry) or `cli` (driver-level, plan-fixed verb surface) |
| `envelope` / `envelopeSequence` / `cli` | the input; sequences apply in order; CLI cases carry argv with `<DOC>`/`<WORK>`/`<ENVELOPE>`/`<LAYER_ID_FROM_NEW>`/`<I18N_DIR>` placeholders |
| `protoExpect`   | round-1 proto expectation (evidence; `observedCode` from REPORT-A §4.2 for `code=*` cases); null on generated cases |
| `cp01Expect`    | expected outcome against the CP-0.1 engine: `outcome` accept/reject, `exit` (or `exitAnyOf`), `code` (null = any structured error); null when the case premise is void at CP-0.1. `codeAnyOf` is retired: the wave-4 collapse pinned every hedge per the pinned validation order (manifest `hedgeCollapse`; self-check `no_codeanyof_hedges_remain`) |
| `codeTier`      | provenance of the expected code: `contract` / `proto` / `plan` (see manifest `taxonomy`) |
| `cp01Runnable`  | false = the wave-2 runner records SKIP with the `notes` reason instead of executing |
| `provenance`    | mechanical source of the case (rule 7: no hand-invented expected data) |

Rejections are always structured `{"ok":false,...,"error":{code,message}}`
with the envelope id quoted when the input carried one, package bytes
untouched, and no panic — the proto machine contract, asserted per case by
the wave-2 runner exactly as `adversarial.py` did.

## Determinism rules

- `generate_corpus.py` is seedless and stable: sorted iteration everywhere,
  `sort_keys` JSON, fixed positional id schemes (manifest `idSchemes`).
  Re-running against unchanged inputs is byte-identical (verified at
  authoring: two runs, `diff -r` empty).
- The manifest records sha256s of every generator input, so contract or
  proto drift is visible in review, never silent.
- The committed corpus is the artifact of record; CI self-checks
  (`tests/conformance`) validate the committed bytes against
  `contracts/` directly and do NOT re-run the generator (the proto checkout
  is not assumed present in CI).
- Corpus changes land only through O-QA review (plan §0.6 rule 6).

## Regeneration

```
python3 fixtures/adversarial/generate_corpus.py   # needs PyYAML + the
                                                  # appname-proto checkout
```

Then re-run `cargo test -p appname-conformance` — the self-checks must stay
green — and route the diff through O-QA review.

## Binary image fixtures

None under fixtures/ yet — the first binary expected-output corpus landed
under `goldens/` at wave 4 (engine-produced doc.new matrix + selftest
reference; see `goldens/README.md`). Deterministic seeded image INPUT
fixtures (noise/gradient/checker) arrive with the pixel-kind waves; seeds
will be recorded beside the outputs they produce.
