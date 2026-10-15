<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (c) 2026 the Appname contributors -->

# i18n/ — message catalogs (lane O-I18N-A11Y)

Spec: ES §27 (strings/i18n), Appendix G (MessageId registry); plan §2.2
row "i18n from day 1", §3.12. Registry of record:
`contracts/message_ids.yaml` (v5.5 freeze, 419 ids). This directory is
the CANONICAL catalog home for the whole build; the format is the
proto's, unchanged, so the shipped Rust CLI loader
(`crates/appname-cli/src/i18n.rs`) reads these files as-is.

## Ownership (goldens-style)

Catalog and MINTED changes route through lane O-I18N-A11Y, the way
golden changes route through O-QA: another lane needing a new string
adds the emitter in its own crate and hands the id + en wording to this
lane (via O-INT), which lands the catalog entries + MINTED row and runs
the audit. `i18n/pseudo.json` is GENERATED — never edit it by hand.

## Files

| Path | What |
|---|---|
| `en.json` `es.json` `ja.json` | hand-maintained catalogs, same key set (610 keys today) |
| `pseudo.json` | generated from `en.json` by `tools/pseudo.py` |
| `MINTED.md` | mint-and-log register: every id the build emits that the v5.5 registry lacks, with rationale + upstream fold-in destination |
| `tools/audit.py` | registry-vs-catalog-vs-emitted audit (the lane's tripwire; see below) |
| `tools/pseudo.py` | deterministic pseudolocalizer (`--check` = drift guard) |
| `tools/switch_check.sh` | runtime switch evidence: builds the CLI, proves en/es/ja/pseudo render byte-distinct via `APPNAME_I18N_DIR` |

## Catalog format

Flat JSON object, `"messageid": "template"`, UTF-8, plus a `_meta`
block the loader reads for `plural_rules` (`one_other` en/es/pseudo,
`other_only` ja) and humans read for `locale`/`direction` (`ltr` — RTL
catalogs arrive with the CP-1.0 mirroring work). SPDX headers are
IMPOSSIBLE in JSON (no comment syntax): `scripts/gate.sh` exempts
`*.json` from the SPDX sweep by design (proto precedent) — the
`.py`/`.sh`/`.md` files here carry normal headers.

Template grammar is the loader's ICU-MessageFormat subset, exactly:

- `{name}` placeholders (`[A-Za-z0-9_]+`); args a template does not
  consume are appended by the loader as ` name=value` (that is how
  optional error context like `field`/`id`/`line` surfaces);
- cardinal plurals `{n, plural, one {# thing} other {# things}}` with
  `#` = the number; selectors limited to `one`/`other`; the catalog's
  own `_meta.plural_rules` picks the category (an en fallback string
  pluralizes with en's rule even under `--lang ja`);
- wire data (command kinds, error codes, uuids, PASS/FAIL, `--json`
  output) is NEVER translated — it appears verbatim inside translated
  frames.

Language resolution (shipped, evidenced by `tools/switch_check.sh`):
`--lang` flag > `APPNAME_LANG` env > `en`; catalog directory:
`APPNAME_I18N_DIR` > executable-adjacent `i18n/` > embedded `en` >
key-echo.

## Plural policy (state of today, stated per the activation brief)

Full ICU plural coverage does NOT exist yet: exactly three ids carry
plural blocks (`cli.info.layers`, `cli.undo.ok`, `cli.redo.ok`), using
only the minimal `one|other` subset the CLI loader implements; every
other string is deliberately phrased plural-free (label-style counts:
"entries: {length}"). Full CLDR categories (`few`, `many`, …) and richer
ICU arrive with the shell wave alongside the C++/QML loader. Variant ids
(`…_one`, `…_many`) are forbidden by the registry rules — grammar lives
in catalog values, never in ids.

## Mint-and-log protocol

Short form (full text + tables in `MINTED.md`): an id the build emits
must be in `contracts/message_ids.yaml` or in `MINTED.md`, in the same
commit as its emitter — the audit REDs anything else. Minted ids follow
the registry naming rules and carry rationale + upstream destination
(the v5.6 fold-in session; §36.5 PR i18n note). 320 ids are minted
today (6 error-taxonomy wire codes, 19 CLI ids, 4 `dialog.recovery.*`
family members, 7 `color.*` pipeline warning ids (CP-1.0 backlog
round, spec-named §6.2/§5.5), 284 shell `ui.*` ids — 180 from the
CP-0.5 chrome-wave fold-in, minus the dropped `ui.status.zoom_percent`,
+ 43 staged in the CP-0.7 W05/About/blend/export coordination round
+ 2 staged in the post-ui-P-round queue close + 9 staged in the
CP-1.0 slice-V vector-tools mint round + 2 About-credits rows staged in
the CP-1.0 backlog round per the lucide/lensfun deps records
+ 49 staged in the CP-1.0 chrome-vocabulary mint rotation); the 129 `proto.*` catalog
keys are
catalog-only shell seeds, not mints, and shrink to zero as O-UI
surfaces land.

## Running the audit

```
python3 i18n/tools/audit.py           # report + exit 0/1
python3 i18n/tools/audit.py --gaps    # + id-level uncovered-registry list
python3 i18n/tools/audit.py --list-ids# raw sets for downstream tooling
```

Deterministic, stdlib-only. RED conditions: an emitted id in neither
registry nor MINTED; en below 100% of emitted ids; es/ja/pseudo key set
differing from en; placeholder-name drift vs en; an untracked catalog
key; stale `pseudo.json`; a shipped command kind (milestone ≤ the
audit's `SHIPPED_MILESTONES`, bumped at checkpoint close) whose
`history.<kind>` label is missing from en (RED-7 — history labels are
family-constructed in the shell, invisible to the literal sweep). The
registry-vs-emitted and registry-vs-catalog gaps are REPORTED numbers,
not failures — most registry ids await the shell. The emitted-id census
is mechanical over `crates/**/*.rs` AND `shell/**/*.{qml,cpp,h}`
(sweep + family-expansion rules documented in the script header); the
CLI-crate seed comparison is informational pending the relocation
handoff below.

## Pseudolocalization

`tools/pseudo.py` regenerates `pseudo.json` from `en.json`:
`[!!! Àççẽñŧẽđ ŧẽẋŧ ~~~ !!!]` — accented so untranslated literals stand
out, `~`-padded ~30% (capped) so clipped layouts show, bracket-wrapped so
concatenation is obvious. Placeholders, plural machinery, `#`, and file
glob patterns `(*.ext)` survive verbatim; `proto.lang.*` endonyms pass
through so the Language menu stays navigable. `--check` fails when
`pseudo.json` is stale (the audit calls it).

## Open coordination (tracked in `reports/O-I18N-A11Y-log.md`)

- **Gate wiring (O-BUILD): DONE** — `audit.py` (which embeds
  `pseudo.py --check`) runs as gate step `i18n-audit` and
  `switch_check.sh` at the `ci_local` tier (O-BUILD `d6f561a`, O-INT
  authorized). The audit is a standing gate: catalog changes land with
  their emitters or main goes red.
- **Catalog relocation (O-CORE via O-INT):** `crates/appname-cli/i18n/`
  is the frozen CP-0.1 seed snapshot (179 ids; the loader's embedded-en
  `include_str!` points there). Proposed install layout: the packaged
  binary ships `<bindir>/i18n/*.json` copied from THIS directory
  (exe-adjacent resolution already works), and the crate seed dir
  becomes a build-time copy of `i18n/` so the embedded en and the
  canonical catalogs cannot diverge. Until then the audit lists the
  delta (14 added ids per locale + the es `cli.error` reconciliation).
- **C++/QML loader:** Qt toolchain now available; the loader half of the
  §2.2 row lands next wave, coordinated with O-UI through O-INT.
