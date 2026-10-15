<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (c) 2026 the Appname contributors -->

# Lens profile database (lensfun database, DATA ONLY)

This directory stages the **lensfun lens database** — camera/lens
distortion, chromatic-aberration, and vignetting calibration profiles —
as a clearly-separated, user-replaceable DATA package.

- Upstream: <https://github.com/lensfun/lensfun>, `data/db/` subtree at
  commit `6804b5f581f31b10a90a06d811acb37d6877ef32` (census pin,
  2026-08-11), copied **byte-identical** (verified per-file against the
  upstream tree at the pin, 2026-08-17).
- License: **CC-BY-SA-3.0** — the co-located `COPYING.CC_BY-SA_3.0` is
  the license text, staged with the data because the license travels with
  the DATABASE, not with the application binary.
- Record: `deps/lensfun-db.yaml` (`appname.data-package.v1`) — the
  mechanical notice-carriage record; `tools/gen_notices.py` verifies this
  directory's full payload against the record's manifest pin at every
  generation.
- Basis: CC-BY-SA-3.0 is not in the `algorithms.yaml` policy
  `weights_allowed` list — this data package rests on the contract's
  explicit `lens_correction.database` decision (`data_license:
  CC-BY-SA-3.0`, `library_linked: false`). The LGPL-3.0 lensfun LIBRARY
  is not linked, not vendored, and was never consulted: the engine reads
  this data with its own Rust reader
  (`crates/appname-algo/src/lensdb/`), written from the DTD/XSD and the
  observable data only.

## Obligation shape (precisely)

1. **Shipped unmodified (this directory's state):** the database is the
   Work incorporated in a Collection — CC-BY-SA-3.0 §4(a) requires the
   license copy with every distributed copy (the co-located COPYING) and
   all notices kept intact (byte-identical carriage; the profile files'
   contributor-credit comments are preserved verbatim). §4(a)'s
   Collection carve-out means the application itself is NOT subject to
   CC-BY-SA terms.
2. **Shipped modified (any project change to `db/`):** that is an
   Adaptation — §4(b) requires distribution under CC-BY-SA-3.0 (or a
   listed compatible license) and §3(b) requires the changes to be
   clearly marked. Project workflow (elected; beyond the license's
   distribution-terms minimum): contribute the modification back
   upstream, mark it, and amend `deps/lensfun-db.yaml` (payload pin +
   workflow field) in the same commit — the notices gate fails on any
   change to this directory that does not.
3. **User-replaceable directory (outside this repo):** share-alike
   obligations attach on distribution. Profiles a user adds or modifies
   in their own data directory are not distributed by this project — they
   are the user's affair, and no project obligation arises. The reader
   treats that directory as untrusted input (typed errors, never
   panics); per the O-INT 2026-08-17 ruling, the SHIPPED database is read
   strictly (all-or-nothing) while the user directory is wrapped
   degrade-to-warnings-and-skip at the consumer seam.

Due-diligence documentation, not a legal opinion, not a release claim;
OD-7 remains the release gate.
