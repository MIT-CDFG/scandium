<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (c) 2026 the Appname contributors -->

# W26 compat fixture — v0.5.0-beta era package

Provenance (rule 7 — nothing hand-authored):

- `legacy-basic.appname` was produced MECHANICALLY by the CLI built from
  the repo tag **`v0.5.0-beta`** (scratch worktree build, 2026-08-13;
  the crate version string at that tag reads `0.1.0` — versions were not
  bumped per-tag, the tag itself is the provenance) by running:
  `new --width 96 --height 64 --background white` then
  `script --file gen_script.jsonl` (the committed script, deterministic
  fixed ids: layer.add + fill + layer.set_props + guide.add — all
  milestone-0.5 kinds).
- `legacy-basic.v050-export.png` is that binary's own export of the
  package (the cross-version render-stability reference).
- `SHA256SUMS` pins both. Regeneration: check out `v0.5.0-beta` in a
  scratch worktree, build `appname-cli`, re-run the commands above —
  byte-identical output (fixed envelope ids; content-derived layer ids).

Consumed by `tests/workflows/w26_version_compat.sh` (the CP-0.7 subset
of the W26 spec artifact: a 0.5-era package opens/upgrades under 0.7).
The spec's future-feature and minReader corpus rows (§7.7 scenarios 1/3)
are 1.0-gate scope; the minReader-too-new leg here is synthesized from
this fixture by rewriting `requires.minReader` (see the runner).
