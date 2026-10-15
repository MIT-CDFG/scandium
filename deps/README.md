<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (c) 2026 the Appname contributors -->

# deps/ — dependency records

No external dependency lands in this workspace without a record here FIRST
(preamble rule 6; plan §3.2). A record pre-authorizes a dependency for named
consumer crates; `deny.toml` enforces the license policy mechanically and
`scripts/gate.sh` runs `cargo deny` on every gate.

Record shape follows the spec repo's `deps/*.yaml`
(`appname.dep-record.v1`): name, role/purpose, version line, resolved license
expression exactly as published in the crate metadata, elected license where
the expression offers a choice, upstream, linking, which crates may use it,
obligations/redistribution, review owner, risk, notes.

Policy reminders:

- Permissive allowlist only (Apache-2.0, MIT, BSD-2/3-Clause, ISC, Zlib,
  Unicode-3.0/Unicode-DFS-2016). GPL-family dependencies are forbidden inside
  the Apache-licensed crates (`appname-cabi`, `appname-py`).
- LGPL/MPL: link, don't vendor. Records for such deps must say so.
- Every record's obligations column feeds THIRD-PARTY-NOTICES generation
  (CP-0.7 O-BUILD row).
- License expressions come from published crate metadata (mechanically read,
  e.g. `cargo metadata`), never from memory.

## Port-obligation records (`port-*.yaml`, `appname.port-record.v1`)

A second record kind, `recordKind: port-obligation`, covers code PORTED from
censused sources (appname-proto vendor trees / census records) rather than
cargo dependencies. These records are consumed MECHANICALLY by
`tools/gen_notices.py`:

- `key` — the census/provenance key (`appname-proto/vendor/<key>/` or
  `research/code-census/*.yaml`).
- `license` — SPDX id as read first-hand by the census.
- `copyright` — the copyright line(s) the notice must carry, verbatim from
  the census-verified license text.
- `licenseTextPath` (relative to `appname-proto/`) OR `licenseTextInline` —
  where the generator reads the full license text at every generation.
- `landingPaths` — paths in THIS repo where the port lands. While none
  exists in the working tree the obligation renders PENDING (one-line row);
  the moment one exists, regeneration emits the full license text +
  copyright into THIRD-PARTY-NOTICES.md, and the gate's `--check` drift
  stage forces that regeneration post-merge (stale notices = red).
- `noticeSurfaces` — obligations beyond the notices file (About-screen
  credits, courtesy citation lines), emitted verbatim with the entry.

No port lands without its record; records are due-diligence carriage
(V39 terminology discipline), not release claims.

## Vendored-asset records (`recordKind: vendored-assets`)

A third record kind covers non-code assets copied verbatim into this repo
at a pinned upstream commit (first case: `deps/lucide.yaml` — UI icon SVGs
for `shell/`). Never a cargo dependency; cargo-deny never walks them.
Consumed MECHANICALLY by `tools/gen_notices.py`:

- `licenseTextInline` — the upstream license text carried verbatim in the
  record, from a first-hand RAW-BYTE read at the pinned commit;
  `licenseTextSha256` pins it, and the generator verifies inline-vs-pin at
  every load (a tampered record is red even while pending).
- `licenseTextRepoPath` — where the vendoring commit must co-locate a
  LICENSE copy next to the assets. Once any `landingPaths` entry exists,
  the copy is required, re-read at every generation, and sha256-verified
  against the pin — a missing copy or drifted bytes fails generation and
  the gate's notices step.
- `landingPaths` — pending one-line row until the vendor drop lands, then
  the full entry (license text, copyright lines, upstream pin,
  `noticeSurfaces`) activates, exactly like port records.

Pin drift rule: expanding the vendored set later requires a fresh upstream
pin and a fresh first-hand license read recorded census-style.

## Data-package records (`recordKind: data-package`)

A fourth record kind (`appname.data-package.v1`; first case:
`deps/lensfun-db.yaml` — the CC-BY-SA-3.0 lensfun lens profile database
staged under `data/`) covers clearly-separated, user-replaceable DATA
shipped with the application. Never code, never a cargo dependency;
cargo-deny never walks it. Consumed MECHANICALLY by
`tools/gen_notices.py`; the shape merges the port-record and asset-record
precedents with two data-specific additions:

- `dataPaths` — activation triggers: paths in THIS repo where the
  LICENSED DATA is staged. Deliberately NOT the own-code consumer (that
  rides the informational `consumers` field): own code carries none of
  the data license's expression, and its presence alone must never
  activate attribution for data that is not in the tree (the pymatting
  deliberately-not-listed argument, inverted).
- `licenseTextSha256` + `licenseTextRepoPath` + (`licenseTextPath` under
  `appname-proto/` OR `licenseTextInline`) — asset-record license
  carriage: the pending-time source is verified against the pin at every
  load (a tampered record is red even while pending); once staged, the
  co-located copy is required, re-read at every generation, and
  sha256-verified (the license travels with the data, not the binary).
- `payloadSha256` — a manifest digest over every staged payload byte
  (sorted per-file sha256 lines, hashed), re-verified at every generation
  once landed. This is the SHARE-ALIKE WORKFLOW TRIPWIRE: modifying
  shipped data without amending the record in the same commit fails
  generation and the gate's notices step, so a shipped-db change cannot
  bypass the record's `shareAlikeWorkflow` field (mark the changes, keep
  license carriage, contribute back). User-dir contents are out of scope
  by construction — the pin covers only what this repo stages and ships.
- `basis` / `shareAlikeWorkflow` / `noticeSurfaces` — emitted verbatim
  with the notices entry (the addendum's basis-line obligation and the
  workflow note ride the entry itself).
