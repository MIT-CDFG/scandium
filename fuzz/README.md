<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (c) 2026 the Appname contributors -->

# appname-fuzz — untrusted-input parser fuzzing (O-QA)

Spec: ES §8.5 ("PSD readers are continuously fuzzed"), §34.2; plan §2.4
("fuzz corpus running without crashes for a recorded soak") and §3.14;
CP-1.0 additions per the O-WATCH review #9 exit-ledger row (fresh fuzz
soak at the close + targets for the checkpoint's new input surfaces).
Posture: **cargo-fuzz layout, OUTSIDE the workspace** (own `[workspace]`
key) so libfuzzer runtime/sanitizer flags never touch shipped artifacts;
deps records `deps/cargo-fuzz.yaml` + `deps/libfuzzer-sys.yaml`. This
crate PROMOTES O-IO's CP-0.7 wave-1 fuzz floor
(`crates/appname-io/tests/psd_fuzz_floor.rs`) to real libFuzzer targets,
per that floor's own migration note.

## Targets

| target | surface |
|---|---|
| `fuzz_psd_import` | the PSD parser via the codec registry (`decode`) |
| `fuzz_codec_import` | the whole codec matrix through sniff-dispatch (`resolve` → `decode`) — the path every import takes |
| `fuzz_vector_sidecar` | the `appname.vector.v1` sidecar surface: `appname_vector::path_from_sidecar_json` (the production wire parser, with strict-writer round-trip asserts) + `appname_core::load_package_bytes_with` over an assembly carrying the REAL `appname_vector::register` service (the production posture since the Door-1 merge — load runs the registered `validate_sidecar` on the fuzzed `vector/{pathId}.json` entry) + `appname_core::load_package_bytes` (the bare-mirror opaque-carry door, kept) |
| `fuzz_pack_manifest` | `appname_ml::manifest::PackRegistry::parse` — serde shape with `deny_unknown_fields` + the discipline validation pass (id grammar, dup ids, revocable refusal, hash-pin hex, lawful fallback) |
| `fuzz_action_v1` | `appname_script::Action::from_json` — the `appname.action.v1` file parser (16 MiB bomb gate, RFC 6901 pointers, base64 assets with content-addressed id verification), with `to_json` re-parse fixpoint asserts |

Bar (the fuzz-floor D-series discipline): every input parses/imports Ok
or returns a **typed error** (`CodecError` / `PathError` / `EngineError`
/ `ManifestError`) — never a panic, never an abort; §8.1/§26.1 bomb
gates hold. The vector and action targets additionally ASSERT wire
round-trip invariants (lenient-reader output satisfies the strict
writer; re-emission re-parses) — a violation is a wanted crash finding.

**Coverage note** (`fuzz_vector_sidecar`): the recorded follow-up is
DISCHARGED (O-QA re-pin round) — the Door-1 `VectorGeometry`
registration merged (TEXTVEC wiring, main e8f5f9b), and the target now
loads over an assembly with the real service registered, so the
registered `validate_sidecar` sits in the fuzzed load path. The
bare-mirror door stays a third surface (it remains a public API and the
service-less opaque-carry path is distinct code).

## Seed corpus

`corpus/fuzz_psd_import/` — the six W07 corpus-subset PSDs, emitted
mechanically by `w07_psd_migration seeds <dir>` (same byte-level emitter,
same psd-corpus.yaml provenance). `corpus/fuzz_codec_import/` — PNG
(engine export + the committed W16 fixture), a PSD, and a `.appname`
package. Regenerate the PSD seeds any time with
`cargo run -p appname-perf --release --bin w07_psd_migration seeds fuzz/corpus/fuzz_psd_import`.

`corpus/fuzz_vector_sidecar/`, `corpus/fuzz_action_v1/`,
`corpus/fuzz_pack_manifest/` — emitted mechanically and DETERMINISTICALLY
by this crate's own `seed_gen` bin (`cargo run --release --bin seed_gen`
from `fuzz/`; byte-stable across re-runs, verified at authoring): real
`VectorPath`s via `appname_vector::primitives` through the strict writer;
one action RECORDED from a live engine journal (pinned ids) plus one
typed-model action covering stops/parameterized/assets via
`Action::to_json`; the staged `packs/packs.json` verbatim plus a
single-pack reduction (seed emission re-parses both — it doubles as a
fixture check).

## Nightly entry point (ci_local tier — no CI system exists)

```
bash fuzz/run_soak.sh [seconds-per-target]   # default 300
```

- With a **nightly toolchain + cargo-fuzz** installed: full engine
  (ASan + coverage) via `cargo +nightly fuzz run`.
- On this **stable-only host**: the same targets run as direct libFuzzer
  binaries — WITHOUT sanitizers AND without coverage feedback (stable
  builds carry no sancov, so mutation is blind from the seed corpus; the
  soak runs on a scratch copy of the seeds so every mutation starts from
  a real structured input — see run_soak.sh). Recorded honestly in every
  soak log; ASan + coverage re-enter when a nightly toolchain is
  provisioned.
- Exit 0 = every target completed with zero crashes/OOMs; any finding
  lands under `findings/<target>/` and fails the run.

O-BUILD handoff (logged in reports/O-QA-log.md CP-0.7 wave 2): add
`bash fuzz/run_soak.sh` to the `scripts/ci_local.sh` nightly section —
the script itself lives in the O-QA fuzz fence.

Soak records live in `reports/O-QA-log.md` (duration, execs, corpus
growth, findings — per checkpoint).
