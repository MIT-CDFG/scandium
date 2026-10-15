<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (c) 2026 the Appname contributors -->

# packs/ — model pack manifests (O-ML fence)

Registry: `packs.json`, shape `appname.aiModelPack.v1`
(`contracts/schemas/ai-model-pack.v1.json`; ES Part 53 §53.9, ALGORITHMS.md
§1.4.3, review ML-001). Parsed and discipline-checked by
`crates/appname-ml/src/manifest.rs`; loadability + sha256 verification at
load by `crates/appname-ml/src/verify.rs`.

## Feed discipline (normative for this directory)

- **Exclusive feed:** every pin (repo, revision, filename, bytes, sha256,
  license reads, recipes) is copied MECHANICALLY from
  `appname-proto/research/pins/models.v5.6-candidate.yaml`. No value in this
  directory is ever invented, re-derived, or "fixed up" by hand (plan §1
  rule 8; the pins file's own RECORD DISCIPLINE header governs upstream).
- **`pending-packaging` is honest state, not a placeholder to fill in:**
  converted-artifact hashes are stamped only by the packaging pipeline with a
  recorded deterministic recipe; the loader refuses such files typed
  (`PackRefusal::ConversionUnpinned`) until then. Conversion/export of
  weights is NOT performed in this lane (D-49 — no fetching, no conversion
  without a recorded basis; wave-3 exports are O-ALGO pulls).
- **Offline default (D-49):** `download_url` fields are provenance metadata.
  Nothing in the runtime dereferences them; packs load from local files
  only.

## Current inventory (CP-1.0 slice 1, 2026-08-16)

| pack id | slot | state |
|---|---|---|
| `bgremove-birefnet` | subject-select high-quality tier (13a) | staged; source pinned; converted `pending-packaging` → not loadable yet |
| `matting-modnet` | portrait matte (13c) | staged; source pinned; converted `pending-packaging` → not loadable yet |

## Recorded gaps (routed to O-INT via reports/O-ML-log.md)

- **u2net (13a salient tier) and dis (13a dichotomous tier): NO manifest.**
  The exclusive pins feed carries no u2net row and no dis row (19 pack ids,
  checked 2026-08-16; the frozen `bogos/spec/ai/models.yaml` id set is
  identical). INTEGRATION-NOTES §6 rows 13a say "weights per models.yaml
  row" — those rows do not exist. Writing a manifest would mean inventing
  pins; both tiers are typed-unbound in `runtime::slot_binding` until a
  sanctioned pin lands (v5.6 fold-in territory). The ALGORITHMS.md §9 ladder
  still serves: BiRefNet tier → §8 classical fallback.
- **dehazeformer/retinexformer** (LATER waves, gate closed): spec-owner
  elections confirmed via operator relay on the INTEGRATION-log record
  (2026-08-16): dehazeformer = reside6k, retinexformer = LOL_v2_real, as B3
  elected. Their manifests are staged only when their waves open.
- **MI-GAN stays `not-distributed`** (upstream issue Picsart-AI-Research/
  MI-GAN#25 open, NVIDIA-header finding): the loader enforces the gate
  structurally (`PackRefusal::DistributionGated`); no manifest staged.
- **denoise-nafnet**: own-training program, out of this plan.
- **GeoCalib** (later wave): CC-BY-4.0 attribution must render in the UI —
  the manifest pipeline already flags it
  (`LicenseDisplay::attribution_required_in_ui`).
