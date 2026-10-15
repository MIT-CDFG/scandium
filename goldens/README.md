<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (c) 2026 the Appname contributors -->

# goldens/ — expected-output corpus (LIVE since wave 4)

Spec: plan AGENT-ASSIGNMENTS-full-build.md §2.1 row "Test harness v1" and
§0.6 rule 6 (goldens as cross-lane contract); determinism spine D-48; ES
§34.3 (parity gates), §7.6 (deterministic package writer).

## Status: first goldens landed (wave 4, engine-produced)

- `doc_new/` — the fixed doc.new matrix: 3 modes (rgba, gray_a, lab_a) x
  2 sizes (1x1, 96x64) x 3 backgrounds (transparent, white, black), depth
  u8. 18 package goldens (`.appname`); 6 export goldens (`.png`, rgba
  only — PNG export of non-rgba modes is an engine deferral to the CP-0.5
  §5/§6 conversion pipeline, declared per `.meta`); 18 `.meta` sidecars;
  `MANIFEST.sha256`.
- `selftest/reference.appname` — the CLI selftest fixed-script package
  (CP-0.3: doc.new + brush leg), byte-reproduced by mirroring the script
  on the assembled engine; sha256 matches the O-CORE-log stroke-wiring
  record `2484d11a…82a181`, 527851 bytes, state_hash `71ef71521484…`
  (the pre-stroke 527133-byte reference is retired).
- `stroke/` — CP-0.3 slice B: 4 assembled-engine stroke goldens (brush
  hard/soft, eraser-over-brush, undo-redo with byte-identity evidence),
  packages + exports + metas + `MANIFEST.sha256`; regen bin
  `regen_stroke_goldens` (also pins the runner samples fixtures under
  `fixtures/samples/`). No lock golden: no public surface sets §11.1
  locks until layer.set_props — the `error.command.locked` wire id is
  pinned in the verify suite instead (recorded decision).
- `retouch/` — CP-1.0 golden round for the retouch row (merged d660d3b,
  goldens staged-not-frozen). 6 stations through the ASSEMBLED engine:
  `retouch-clone` (§21.1), `retouch-heal-sampled` (§45.1 Poisson membrane
  commit via appname-algo `poisson_heal`), `retouch-heal-spot` (§45.3
  deterministic exemplar footprint — its seeded stream is the SAME
  contract the `caf/` goldens cite), `retouch-dodge-burn` (§21.5),
  `retouch-smudge`, `retouch-blur-sharpen`. Each: package + export + meta
  (fixed seeds, seed rationale + D-48 undo/redo/replay oracle recorded);
  regen bin `regen_retouch_goldens`.
- `composite/` — CP-1.0 composite SERVE-goldens (the composite walls fell
  @ 3b21d8f). `composite-shape` (two solid-fill vector paths: ellipse +
  rect) and `composite-text` (Door-6 layout→page, deterministic
  §23.5/§44.2 tofu — no font vendored). THE GOLDEN INVARIANT: PNG export
  == `encode_composite_png(composite_plane_f32)` byte-for-byte (the
  export==readback one-truth wall), re-asserted at production and in the
  verify suite. SOLID fills frozen; gradient/pattern are the recorded §22
  gap (typed refusal, not frozen). Regen bin `regen_composite_goldens`.
- Producer: `cargo run -p appname-conformance --bin regen_doc_new_goldens`
  (engine-level; every artifact generated twice through independent
  engines and byte-compared before writing — rule 5 below).
- Enforcement: `tests/conformance/tests/goldens_verify.rs` re-produces
  every golden from a fresh engine on each `cargo test` run and
  byte-compares against the committed files, then `shasum -c`s the
  manifests. A lane that shifts a byte turns it red and routes through
  O-QA review.

## Rules (binding from the first golden onward)

1. **Provenance**: every golden is produced mechanically by a pinned
   engine/CLI revision from a committed fixture + envelope sequence; the
   producing command line and revision are recorded in the golden's `.meta`
   sidecar.
2. **Seeds beside outputs**: any stochastic input (noise fixtures, brush
   scatter) uses a FIXED seed recorded in the sidecar next to the output it
   produced (plan §2.1: "golden fixtures (deterministic, seeded)").
3. **Change control**: goldens change only through O-QA review (plan §0.6
   rule 6). A lane whose diff turns a golden red proposes the change; O-QA
   re-derives, reviews, and lands it. No silent regeneration.
4. **Byte-exactness**: comparisons are byte-exact (`shasum -a 256`) unless
   the spec pins a tolerance (e.g. the ES §34.3 GPU-vs-CPU parity bound of
   ≤ 1/255, which is a parity gate, not a golden).
5. **Determinism proof**: a golden is admissible only after two independent
   runs (fresh temp dirs) produce identical bytes — the same two-run
   discipline as `tests/workflows/replay_determinism.sh`.

## Layout

```
goldens/
  doc_new/           doc.new matrix (live): packages + rgba exports +
                     .meta sidecars + MANIFEST.sha256
  selftest/          selftest reference package (live): reference.appname
                     + .meta + MANIFEST.sha256
  commands/<kind>/   per-kind apply outputs (arrives with each kind's wave)
  blend/             compositor blend-mode goldens vs hand-computed oracles
                     (CP-0.3 row "Golden expansion")
```

Each entry: `<name>.<ext>` + `<name>.meta` (producer, fixed input
envelope, byte sizes, sha256s — deliberately no timestamps, so
regeneration is byte-identical).
