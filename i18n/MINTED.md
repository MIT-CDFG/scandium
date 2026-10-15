<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (c) 2026 the Appname contributors -->

# MINTED — mint-and-log register (ids not in the v5.5 registry)

Lane: O-I18N-A11Y (plan §3.12). Registry of record:
`contracts/message_ids.yaml` (v5.5 freeze, 419 ids — mirror of
`bogos/i18n/message_ids.yaml`, READ-ONLY upstream). The build may not
invent MessageIds silently: **any id the build emits that is absent from
the registry is minted HERE, in the same commit that introduces the
emitter.** `i18n/tools/audit.py` treats the ids in this file's tables as
known-not-in-registry (no red) and FAILS on any emitted id that is in
neither the registry nor this file — that is the drift tripwire.

Every entry carries the minting rationale and the upstream fold-in
destination: the v5.6 fold-in session in `bogos/` (a separate,
validator-gated session per the plan's honest-limits note) consumes this
file; upstream registration happens via the spec's §36.5 PR i18n note,
never by editing the frozen v5.5 artifacts.

Protocol for future mints (all lanes, brokered through O-INT):

1. Need a user-surfaced string or wire code with no registry id? First
   look for a registered id that names the meaning (registry rule: an id
   names the meaning, not the wording).
2. None fits: mint the id following the registry's naming rules
   (lowercase ASCII + underscores, `area.context.key`, no abbreviations,
   no variant ids), add a row to the matching table below WITH rationale
   + destination, and add catalog entries (en/es/ja; pseudo regenerates).
3. Catalog changes route through this lane (goldens-style, README
   ownership note); the audit gates the result.

## Minted wire codes (error taxonomy)

The registry's own preamble excludes wildcard families ("`error.io.*` are
families, not ids"), so spec-sanctioned families have NO registered
concrete members; concrete codes the build emits under them are minted.

| Id | Minted by | Rationale | Upstream destination |
|---|---|---|---|
| `error.command.not_implemented` | O-CORE, CP-0.1 wave 1 close (arbitration OQ-1; INTEGRATION-log 2026-08-11) | Plan §0.4 mandates all 102 kinds registered from CP-0.1 with unimplemented kinds rejecting under this code; v5.5 registry lacks it. Emitters: engine stage-4 dispatch (`appname-core`), CLI jpeg-export refusal | v5.6 fold-in: register under `error.command` (O-CORE-log fold-in item 1) |
| `error.api.internal` | O-SCRIPT, CP-0.3 wave 1 (O-SCRIPT-log flag item 5: "same OQ-1 route") | Panic-at-boundary / poisoned-state payloads at the C ABI (`appname-cabi`). The spec sanctions the `error.api.*` family (ES ABI error tables) but registers no concrete member. EMITTED: `lane/script` merged — emitter live on main (`crates/appname-cabi/src/lib.rs`); catalogs pre-covered it, so the post-merge audit shows 29 emitted, 29/29 coverage in all four locales, staged 1 (O-INT review run, 2026-08-12) | v5.6 fold-in: register concrete id under the `error.api` family |
| `error.api.invalid_arg` | O-CORE, CP-0.1 (proto taxonomy port, arbitration OQ-2) | ES maps `APPNAME_ERR_INVALID_ARG` cases (envelope > 1 MiB, invalid UTF-8, §9.6; null/foreign handle, §1.6) to the `error.api.*` FAMILY only — this concrete spelling appears nowhere in the spec text and is proto-inherited (REPORT-A precedent) | v5.6 fold-in: register concrete id under the `error.api` family |
| `error.schema.wrong_schema` | O-CORE, CP-0.1 (proto taxonomy port, arbitration OQ-2) | Proto-added spelling for a `schema` field present but not the `appname.command.v1` const; the adversarial corpus pins it (REPORT-A precedent); absent from spec text and registry | v5.6 fold-in: register, or alias to a spelling the spec owner prefers (O-CORE-log fold-in item 2) |
| `error.history.nothing_to_undo` | O-CORE, CP-0.1 (proto taxonomy port) | Proto-added reject for undo with nothing to undo; the registry's error area has no history-reject codes (`history.*` kinds arrive at 0.7) | v5.6 fold-in: register under an `error.history` sub-family (O-CORE-log fold-in item 3) |
| `error.history.nothing_to_redo` | O-CORE, CP-0.1 (proto taxonomy port) | Proto-added reject for redo with nothing to redo; same gap as above | v5.6 fold-in: register under an `error.history` sub-family (O-CORE-log fold-in item 3) |

## Minted CLI ids

Proto Agent A2 mints (see `appname-proto/i18n/PROTO-IDS.md`), ported with
the CLI at CP-0.1. The registry's twelve areas include no CLI area, so
none can be mapped onto an existing id. Upstream absorption needs either
a new `cli` area (the PROTO-IDS suggestion, via the §36.5 PR i18n note)
or an `app.cli.*` compromise — spec owner's call at the fold-in.

15 of the 16 proto ids are emitted by today's CLI; `cli.info.selection`
is a dormant seed (the build's `info` verb omits the human selection
line — a recorded CP-0.1 divergence from the proto surface).
`cli.asset.add.ok` is the first BUILD-minted CLI id (CP-0.3 stroke
wave: the `asset add` verb registers `appname.samples.v1` blobs as
package assets for stroke envelopes); same upstream destination as the
proto set.

| Id | Args (`?` = optional, auto-appended when unconsumed) | Status |
|---|---|---|
| `cli.error` | `{code}` `{message}` `{field}?` `{id}?` `{line}?` | emitted |
| `cli.new.ok` | `{out}` `{width}` `{height}` `{docId}` `{layerId}` | emitted |
| `cli.apply.ok` | `{kind}` `{id}` `{revision}` | emitted |
| `cli.asset.add.ok` | `{assetId}` `{bytes}` `{doc}` | emitted (build mint, CP-0.3) |
| `cli.script.ok` | `{applied}` `{doc}` | emitted |
| `cli.undo.ok` | `{performed}` (plural) `{cursor}` `{length}` | emitted |
| `cli.redo.ok` | `{performed}` (plural) `{cursor}` `{length}` | emitted |
| `cli.history.header` | `{length}` `{cursor}` | emitted |
| `cli.export.ok` | `{out}` `{format}` `{bytes}` | emitted |
| `cli.import.ok` | `{layer}` `{id}` `{doc}` | emitted |
| `cli.duplicate.ok` | `{doc}` `{out}` | emitted (build mint, CP-0.5 wave 4: the §10.3 doc.duplicate dagger verb) |
| `cli.recover.ok` | `{replayed}` `{out}` `{doc}` | emitted (build mint, CP-0.5 wave 2: §10.6 `recover` verb replays a `script --autosave` journal onto its base package) |
| `cli.info.doc` | `{docId}` `{width}` `{height}` | emitted |
| `cli.info.layers` | `{count}` (plural) | emitted |
| `cli.info.selection` | `{active}` `{coveragePx}` | dormant seed |
| `cli.info.journal` | `{length}` `{cursor}` `{canUndo}` `{canRedo}` | emitted |
| `cli.selftest.header` | — | emitted |
| `cli.selftest.pass` | `{passed}` `{total}` `{golden}` | emitted |
| `cli.selftest.fail` | `{passed}` `{total}` `{golden}` | emitted |

## Minted `dialog.recovery.*` family members

The registry's preamble treats `dialog.recovery.*` as a FAMILY reference
("namespaces, not ids — members are registered individually"); 11
members are registered, and ES §51.4's flows require four more rendered
strings no registered member names. Concrete family-member mints follow
the `error.api.*` precedent above (spec-sanctioned family, absent
concrete id). Minted STAGED by lane O-I18N-A11Y from the O-UI ABI-1.2
READY handoff (2026-08-13, "dialog.recovery.* rows unblock the W05
prompt"); the emitters are O-UI's W05 recovery prompt, next `shell/`
touch. Args documented here are the wording contract for that emitter.

| Id | Minted by | Rationale | Upstream destination |
|---|---|---|---|
| `dialog.recovery.discard_confirm_never_saved` | O-I18N-A11Y, CP-0.7 W05-coordination round | §51.4 flow R step 4: for a never-saved document the confirm step "additionally states that the document exists nowhere else and will be deleted entirely" — a distinct rendered string (§27.1 one-id-per-string); no args | v5.6 fold-in: register under the `dialog.recovery` family (§36.5 PR i18n note) |
| `dialog.recovery.discard_permanently` | O-I18N-A11Y, CP-0.7 W05-coordination round | §51.4 button-semantics table names the step-two `danger`-token button ("Discard permanently") distinctly from step-one `dialog.recovery.discard` ("Discard…"); registered set carries one id for two rendered strings; no args | same |
| `dialog.recovery.last_change` | O-I18N-A11Y, CP-0.7 W05-coordination round | §51.4 flow R row metadata: "last journaled timestamp (locale-aware, §27.4)" needs a label template; args `{time}` (pre-formatted per §27.4 by the shell) | same |
| `dialog.recovery.unsaved_commands` | O-I18N-A11Y, CP-0.7 W05-coordination round | §51.4 flow R row metadata: "unsaved-command count" label, phrased label-style per the house plural policy; args `{count}` | same |

Registered-member args (catalog coverage this round, not mints —
recorded here so the emitter contract sits in one place):
`dialog.recovery.discard_confirm` `{title}` `{commands}` ·
`dialog.recovery.replay_failed` `{index}` ·
`dialog.recovery.corrupt_opened` `{tiles}` `{layers}` ·
`dialog.recovery.newer_version` `{required}` `{current}` ·
`dialog.export.est_size` `{size}` (registered `dialog` id cataloged the
same round) · all other registered members take no args.

## Minted color-pipeline ids (`color.` area)

ES §6.2 (mode-convert warning matrix) and §5.5 (CMYK mathematical
fallback) name these seven ids EXPLICITLY in normative text, but the
v5.5 registry carries no `color` area and none of the seven as a row —
spec-named-but-unregistered, the same class as the §51.4 twelfth-button
finding. Minted from O-COLOR's cross-fence flag (reports/O-COLOR-log.md
"CROSS-FENCE FLAGS: O-I18N (mint-and-log): 7 spec-named ids absent from
the 420-id v5.5 catalog"), all seven verified absent from
`contracts/message_ids.yaml` mechanically before minting. Emitters are
live on main (`crates/appname-color/src/mode_convert.rs` `warn::*`
consts); the audit's sweep gained the `color` area in the same commit
(with the `NON_MESSAGE_PARAM_PATHS` exclusion for the two
`EngineError::missing` params-path tokens `color.comps`/`color.model`,
which are wire data, not MessageIds). All seven are argless persistent
warnings/notices surfaced by the mode-convert and §5.5 fallback UI.
`color.convert.cmyk_fallback_warning`'s en wording is the spec's own
[NORMATIVE] sentence pair, VERBATIM (ES:3329).

| Id | Minted by | Rationale | Upstream destination |
|---|---|---|---|
| `color.mode.to_cmyk_gamut` | O-I18N-A11Y, CP-1.0 backlog round (O-COLOR flag) | §6.2 matrix rows Gray→CMYK / RGB→CMYK: gamut compression into the destination separation is lossy; spec names the id, registry lacks it | v5.6 fold-in: register a new `color` area (or spec owner's placement) via the §36.5 PR i18n note |
| `color.mode.to_gray_lossy` | O-I18N-A11Y, CP-1.0 backlog round (O-COLOR flag) | §6.2 rows RGB→Gray / CMYK→Gray / Lab→Gray: chroma (and separation) discarded | same |
| `color.mode.no_cmyk_profile` | O-I18N-A11Y, CP-1.0 backlog round (O-COLOR flag) | §6.2 RGB→CMYK row "if no profile": surfaced BEFORE the §5.5 fallback engages | same |
| `color.mode.cmyk_roundtrip_lossy` | O-I18N-A11Y, CP-1.0 backlog round (O-COLOR flag) | §6.2 rows CMYK→RGB / CMYK→Lab: K/ink information folded; re-separation won't match | same |
| `color.mode.lab_u8_precision` | O-I18N-A11Y, CP-1.0 backlog round (O-COLOR flag) | §6.2 RGB→Lab row u8 caveat: precision loss at 8-bit storage depth | same |
| `color.mode.lab_clip` | O-I18N-A11Y, CP-1.0 backlog round (O-COLOR flag) | §6.2 Lab→RGB row: out-of-working-gamut colors clipped per intent | same |
| `color.convert.cmyk_fallback_warning` | O-I18N-A11Y, CP-1.0 backlog round (O-COLOR flag) | §5.5 [NORMATIVE]: persistent warning whenever the mathematical separation runs (forward or inverse); export written untagged; en VERBATIM from the spec sentence | same |

## Minted shell UI ids (`ui.` area)

The registry's twelve areas include no plain `ui` area; O-UI minted its
shell-chrome ids under a deliberate `ui.` prefix so unfolded mints
key-echo visibly (the 0.3 honesty mechanism) and never squat the
registry namespace. Minted by O-UI across CP-0.3 waves 2–4 (31 ids,
mint register in `reports/O-UI-log.md`) and the CP-0.5 chrome wave
(enumerated seed register `shell/i18n/ui.en.json`), folded canonical by
this lane 2026-08-13. Where a registered id names the meaning, the row
carries an alias candidate — the swap (and dropping the mint) is O-UI's
next-touch work, brokered by O-INT; until then the emitted spelling is
the minted one. Upstream destination for the whole family: v5.6
fold-in via the §36.5 PR i18n note — either a new `ui` area or
absorption into the existing `menu`/`dialog`/`panel`/`tool` areas
(spec owner's call; alias candidates pre-marked below).

Fold-in census notes (mechanical, 2026-08-13): the seed file holds 194
keys (13 canonical-register + 181 `ui.*`), not the 196 the handoff
said; 2 seed keys (`ui.fill.content.background`, `.foreground`) are
DEAD — their emitter was removed in the chrome wave's fix round ("fill
fg/bg entries removed") — and are NOT minted or cataloged (O-UI deletes
them from the seed); 1 emitted id (`ui.error.no_document`) was missing
from the seed and is minted here. Net: 180 minted ui.* ids, all
emitted (169 as quoted literals in shell C++/QML, 10 via quoted-prefix
family construction, 1 wire code).

CP-0.7 W05/About/blend/export coordination round (2026-08-13): 43
further `ui.*` ids minted STAGED by lane O-I18N-A11Y from the O-UI
ABI-1.2 READY handoff queue (`reports/O-UI-log.md` close entry) —
26 `ui.blend.*` display names (completing the 27-mode set around the
already-minted `ui.blend.normal`), 7 `ui.dialog.about.*`, 8
`ui.dialog.file.filter_*`, `ui.status.exported`,
`ui.status.applied_not_durable`. Emitters land at O-UI's next `shell/`
touch (`error.api.internal` staged→emitted precedent). SHELL CONTRACT
NOTE for the blend family: the wire enum is camelCase
(`colorBurn`, `linearBurn`, … per the contracts blendMode enum) while
these ids are snake_case per the registry naming rules — a
family-constructed lookup must map camelCase→snake_case (e.g.
`m.replace(/([A-Z])/g, "_$1").toLowerCase()`) before concatenating onto
the quoted `"ui.blend."` prefix; the audit's family expansion is
bounded by the cataloged snake spellings.

CP-1.0 slice-V vector-tools mint round (2026-08-16): 9 further `ui.*`
ids minted STAGED by lane O-I18N-A11Y from the O-UI slice-V mint
request (`reports/O-UI-log.md` @ fb0195f, same-wave audit rule:
catalogs before the QML lands) — 3 tool names (`ui.tool.pen`,
`ui.tool.shape_rect`, `ui.tool.shape_ellipse`), 4 options-bar labels
(`ui.options.commit_as`, `.commit_shape`, `.commit_path`,
`.stroke_width`), 1 action (`ui.action.path_make_selection`), 1 menu
item (`ui.menu.edit.stroke_path`, disabled-NI carrier). Emitters land
at O-UI's slice-V `shell/` touch (`error.api.internal`
staged→emitted precedent). Registered `tool.path_select.hint` was
surveyed with the batch and left uncataloged: slice V builds no
path-select tool surface, so it has no joining emitter — it catalogs
when its surface arrives.

CP-1.0 chrome-vocabulary mint rotation (2026-08-18): 49 further `ui.*`
ids minted STAGED by lane O-I18N-A11Y from O-UI's accumulated build
queue (`reports/O-UI-log.md`, doors-flip through filters/liquify
entries — the census build queue's chrome vocabulary, every id
cross-checked against the registry + catalogs first; the queue's
registry-row members — the 8 filter names, `menu.edit.puppet_warp`,
`menu.filter.last`, `history.*`/`a11y.layers.*` labels — were cataloged
as registered rows, NOT minted). The batch: 3 Select-menu save/load
items (exact ids from the doors-flip entry), 7 mode-convert dialog ids
incl. the `ui.mode.<to>` wire-enum family (intent names REUSE the
cataloged `ui.intent.*` family — nothing re-minted), 4 puppet
options-bar labels, 14 warps-row ids incl. the `ui.warp.<kind>` preset
family (11 kinds verbatim from the `transform.warp` contract enum) +
bend label, 15 filters-chrome ids (`ui.menu.filter`, 9 shared
param-label ids named for the ENGINE's real params — the sigmaPx
dead-name lesson — the `ui.noise.<distribution>` value pair, and the
3-id §10.3 fade-pass dialog family whose mode values reuse
`ui.blend.*`), 5 slice-T chrome ids (`ui.tool.text` as filed +
font-option labels + `ui.action.text_to_shape`; further text chrome
arrives by named ask per protocol), and 1 About-credits completion row
(`ui.dialog.about.credit.cgats21_profiles`, LOCALE-INVARIANT,
mandated by `deps/cgats21-cmyk-profiles.yaml` noticeSurfaces — queued
"with the O-UI About mints alongside the lucide and lensfun-db
surfaces"). Emitters land at O-UI's chrome adoption waves
(`error.api.internal` staged→emitted precedent; the audit's flip check
stands armed on all 49).

CP-1.0 A11Y-3 a11y catalog/mint round (2026-08-19): the a11y
catalog/mint of O-UI's A11Y-3 queue (`reports/O-UI-log.md` @ d9ff9e9 +
aab9d53, read first-hand; RE-DISPATCH of the sleep-killed run, started
fresh). 11 REGISTERED `a11y.*` ids cataloged x4 with NO mints
(`a11y.canvas.selection_bounds`/`.selection_cleared`/`.crop_bounds`,
`a11y.history.undone`/`.redone`, `a11y.file.saved`, `a11y.tool.selected`/
`.brush_size`, `a11y.job.completed`, `a11y.zoom.level`,
`a11y.color.foreground` — every one verified present in
`contracts/message_ids.yaml` FIRST) + 6 `ui.*` mints STAGED (the 5
queue name-mints below + the ROW-17 fill-swatch DISTINCT-id decision,
`ui.dialog.fill.color_swatch`). Golden wordings verified first-hand
against the spec: `a11y.canvas.selection_bounds` = §31.3 golden
"Selection: {width} by {height} pixels"; `a11y.tool.brush_size` = §31.3
"Brush size {size} pixels"; `a11y.file.saved` = §31.3 "Document saved";
`a11y.history.undone` = §49.1 KS-1 golden "Undone: {label}";
`a11y.tool.selected` = the BARE `{name}` (§49.1/§49.2 KS scripts show
the bare tool name, e.g. KS-2 s1 "Rectangular Marquee" — the §49
"indicative" %1-tool table is non-binding, the KS golden wins);
`a11y.canvas.crop_bounds`/`a11y.history.redone`/`a11y.job.completed`/
`a11y.color.foreground` per the §49 New-MessageIds indicative table
("Crop: %1 by %2 pixels", "Redone: %1", "%1 finished", "Foreground
color: %1, %2") + §49.2 (a11y.color.foreground carries name+hex, the
§31.1 color-well value format); `a11y.zoom.level` = "Zoom {value}
percent" (§49.2 "announces the settled percentage", spelled-unit
parallel to brush_size). es/ja are real translations (tokens preserved
exactly; register drawn from the cataloged parallels — a11y.layers.*,
ui.status.undone/redone, ui.dialog.crop/fill/canvas_size). NOT cataloged
(recorded absence, no emitting surface — O-UI census "NO surface"):
`a11y.text.editing_started` (text.add commits empty text; typing surface
is a later slice-T mint), `a11y.adjustments.curve_point` (no §17 curves
editor in this shell) — both stay registered-uncataloged until their
surfaces exist. Emitters land at A11Y-3 wave B (`error.api.internal`
staged→emitted precedent; wave A on lane/ui @ 7a83360 is NOT on main, so
the a11y rows are covered-unemitted = audit-green by construction).

CP-1.0 A11Y-5 history-label + brush-swatch round (2026-08-19): the a11y
follow-on for O-UI's A11Y-5 relay (`reports/O-UI-log.md` @ 05e23d3, the
wave-B `document_controller.cpp` has()-gate finding). ONE `ui.*` mint,
`ui.options.brush_color` (below), for the toolbar brush swatch — minted
NOT as `a11y.color.foreground` (which exists REGISTERED, line 93, and is
the wrong meaning: O-INT ruled the false-name class — there is no app
foreground state at CP-0.5; the swatch is a plain options-bar color
label). Alongside it, THREE `history.<kind>` labels were cataloged x4 as
REGISTERED rows, NOT minted (`history.selection.from_path`,
`history.vector.path_new`, `history.vector.shape_add` — each verified
present in `contracts/message_ids.yaml` FIRST, status
`derived-from-catalog`): they are the `{label}` for the §49 "Undone:
{label}"/"Redone: {label}" pattern on the three CP-1.0 model-door kinds
O-UI opened, whose undo/redo announcement stayed deliberately SILENT via
the announcer's `m_catalog->has("history."+kind)` gate until the labels
landed. Wordings follow the history family convention (the sibling
`history.mask.to_selection` = "Mask to Selection" gives "Path to
Selection"; `history.doc.new` = "New Document" gives "New Path";
`history.layer.add` = "Add Layer" gives "Add Shape"). es/ja are real
translations (tokens n/a — these labels are token-free; register drawn
from the cataloged siblings). Emitter for `ui.options.brush_color` lands
at O-UI's swatch adoption (`error.api.internal` staged→emitted
precedent). This round also carries the RED-7 tightening (see
`i18n/tools/audit.py` `EARLY_SHELL_EMITTED`): the three shell-emitted
kinds are pinned into the RED-7 net ahead of the CP-1.0 milestone bump,
so a dropped label REDs the audit instead of going silent.

CP-1.0 v1.0.1 New-Document presets/units mint round (2026-08-19): 14
`ui.*` ids minted STAGED by lane O-I18N-A11Y from O-UI's New Document
dialog feedback items 2+3 (preset picker + units), so the New Document
and Resize dialogs can bind them — Resize REUSES these same rows, no
Resize-specific duplicates. The batch: `ui.dialog.new.preset` (picker
label), `ui.dialog.new.dpi` (DPI field label), `ui.dialog.new.computed_px`
(the computed-size readout template), the 7-member `ui.preset.<name>`
family (`custom`/`a4`/`a5`/`letter`/`hd`/`uhd4k`/`square_social`), and the
4-member `ui.unit.<u>` family (`px`/`in`/`cm`/`mm`). `computed_px` follows
the project's existing templated-row convention (ICU `{name}`
placeholders, × = U+00D7) — shape `{width} × {height} px`, byte-parallel to
the cataloged `ui.status.doc_size`/`cli.info.doc` rows, so O-UI
interpolates width/height the same way. Paper/technical tokens
(`a4`/`a5`/`hd`/`uhd4k`) and all four unit symbols are LOCALE-INVARIANT
(`ui.profile.*` precedent); `custom`/`letter`/`square_social` and the
`preset`/`dpi` labels are real es/ja translations. The `ui.unit.*` family
key-echoes under the `ui.` prefix per the shell-chrome convention rather
than squatting the registry `unit.*` area; alias candidates to the partial
registry set (`unit.px.abbrev`, `unit.cm.abbrev`, `unit.millimeter.name`,
`unit.pixel.short`) are pre-marked in the rows for O-UI's next-touch swap
(no registry member names an inch abbrev — `ui.unit.in` is a clean mint).
Emitters land at O-UI's New Document + Resize dialog binding
(`error.api.internal` staged→emitted precedent); all 14 are covered in
en/es/ja + regenerated pseudo, staged pending that bind.

CP-1.0 tier-2 tool-palette + item-13 transform-wall mint round
(2026-08-19): 24 ids minted STAGED by lane O-I18N-A11Y unblocking O-UI's
11c tool-palette tier-2 queue + the item-13 transform-wall work (the
zero-invented-ids gate correctly excluded those rows until their ids
existed; this thin follow-up mints them so the rows land). 23 `ui.*` (21
`ui.tool.*` tier-2 palette names + `ui.menu.select.color_range` +
`ui.status.transform_wall`) in the shell-UI table below + 1 `a11y.*`
(`a11y.transform.wall`, new section — the FIRST minted a11y id). Structure
calls: color-range keyed under the Select MENU (`ui.menu.select.color_range`
= "Color Range…", dialog-"…" per `ui.menu.select.modify`) taking O-INT's
ruling nuance — Photoshop's Color Range is a Select-menu command, not a
palette tool, so `ui.tool.color_range` would name a non-existent surface;
the slice family takes TWO rows (`ui.tool.slice` + `ui.tool.slice_select`)
per the multi-mode row-per-mode precedent
(`ui.tool.select_rect`/`select_ellipse`); the fg/bg swatch pair is
`ui.tool.swatch_foreground`/`.swatch_background`, NAMES matching the
a11y-wave `a11y.color.foreground` register (es "Color frontal"/"Color de
fondo", ja "前景色"/"背景色") — these name the swatch CONTROLS and so
legitimately read "foreground", distinct from the O-INT false-name ruling
that barred labeling the CP-0.5 brush swatch "foreground".
`ui.tool.shape_polygon` carries a Shape marker to disambiguate the
existing `ui.tool.polygon` (polygonal-lasso selection), the
`ui.tool.shape_ellipse` precedent; `shape_line`/`shape_rounded_rect` are
bare (unambiguous). Tool names are real es/ja translations (standard
Photoshop tool names); technical values stay invariant. `ui.status.transform_wall`
is worded as a §11.1 lock message (a lock-class wall, NOT an NI). Every id
verified absent from `contracts/message_ids.yaml` first. Emitters land at
O-UI's tier-2 + item-13 shell touch (`error.api.internal` staged→emitted
precedent); all 24 covered in en/es/ja + regenerated pseudo, staged
pending that bind.

CP-1.0 v1.0.1 item-11d Phase-2 options-bar mint round (2026-08-20): 13
`ui.*` ids minted STAGED by lane O-I18N unblocking O-UI's 11d options-bar
Phase-2 slice (the mint-gated batch of the desk-reviewed 11d design entry,
`reports/O-UI-log.md` @ 739d846 §4, read first-hand; the zero-invented-ids
gate holds the Phase-2 rows out of the shell until their ids exist). The
batch: the selection-op segmented control — `ui.options.selection_op`
(label) + its 4 value ids `.replace`/`.add`/`.subtract`/`.intersect`
(KS-1's named §32.3 expectation) — plus `ui.options.antialias`,
`ui.options.feather`, `ui.options.sample_all_layers` (selection/wand
family), `ui.options.opacity`, `ui.options.flow` (brush/eraser), and
`ui.options.blend` / `ui.options.resample` / `ui.options.scale_effects`
(the combo LABELS only — see the reuse note). Wordings come from the
`proto.options.*` seed family per the replace-and-drop policy and the
project's existing option/dialog register: `selection_op` is the label
`proto.options.selection_mode` gives (colon dropped per the `ui.options.*`
convention, "Size" not "Size:"), and `.replace`/`.add`/`.subtract` reuse
`proto.options.mode.*` verbatim (Replace/Add/Subtract · Reemplazar/Añadir/
Restar · 置換/追加/減算) — `.intersect` is the ONE genuinely new value mint
(Intersect · Intersecar · 交差, the short-single-word register of its three
siblings). `feather` is `proto.options.feather` (colon dropped);
`antialias`/`opacity`/`blend` reuse the cataloged Fill-dialog register
(`ui.dialog.fill.antialias` = Anti-alias/Suavizado/アンチエイリアス,
`ui.dialog.fill.opacity` = Opacity/Opacidad/不透明度, `ui.dialog.fill.blend`
= Blend/Mezcla/ブレンド — matching `ui.panel.layers.blend`); `resample` and
`scale_effects` reuse the Resize-dialog register verbatim
(`ui.dialog.resize.resample` = Resample/Remuestreo/再サンプル,
`ui.dialog.resize.scale_effects` = Scale effects/Escalar efectos/効果を拡大縮小),
so the transform family reads identically to the dialog; `flow` is a clean
mint with no project parallel (Flow/Flujo/流量, the Photoshop sibling of
opacity); `sample_all_layers` is a clean mint (Sample All Layers/Muestrear
todas las capas/全レイヤーを対象, the Show-Mesh title-case register). REUSE
DISCIPLINE per the design + O-INT dispatch: the blend combo REUSES the 27
`ui.blend.*` values and the resample combo REUSES the `ui.resample.*`
values — only the two labels are minted here, ZERO value re-mints in those
two families. `ui.options.blend` ("Blend") does not collide in meaning with
`ui.options.selection_op` ("Mode"): one names the brush/eraser blend-mode
combo (the project's blend register is "Blend", not "Mode"), the other the
selection-operation segment (§27.1 one-id-per-meaning). Every id verified
absent from `contracts/message_ids.yaml` (no `ui.` area in the frozen v5.5
registry) AND from all four catalogs FIRST; 0 registered ids cataloged, 0
re-minted; no ICU placeholders in the batch (nothing to preserve).
Emitters land at O-UI's 11d Phase-2 `shell/` touch (`error.api.internal`
staged→emitted precedent); all 13 covered in en/es/ja + regenerated
pseudo, staged pending that bind.

CP-1.0 v1.0.1 item-14b mint-queue additions (2026-08-20): 3 ids minted
STAGED unblocking O-UI's 14b consuming slices (the delete-confirm template
upgrade, the reselect disabled-tooltip, the nudge announcement) — read
first-hand from O-UI's 14b mint-queue in `reports/O-UI-log.md` @ 6270a45
(the "MINT-QUEUE/CARRIES from the build" line). 2 `ui.*` here + 1 `a11y.*`
(`a11y.layers.nudged`, in the a11y section — the THIRD minted a11y id). (1)
`ui.dialog.delete_confirm.message` is the §32.5 named-object destructive-
confirm TEMPLATE (message body) O-UI's `DeleteConfirmDialog.qml` recorded
as its mint-queue candidate ("until it lands the dialog names the object
bare"); it carries the `{name}` ICU placeholder — PRESERVED identical in
all four locales (en/es/ja + pseudo), the RED-4 parity requirement. Wording
follows the project's destructive-confirm register verbatim
(`dialog.recovery.discard_confirm`: en typographic “{…}”, es inverted-¿…?,
ja 「…」＋full-width ？), and the Delete verb reuses the cataloged
`ui.menu.edit.delete` register (Delete/Eliminar/削除). (2)
`ui.error.nothing_to_reselect` is the 14a reselect row's disabled-tooltip
reason (the "nothing to reselect" reasonId O-UI's `commandAvailability`
left absent — `document_controller.cpp` records it as a mint-queue
candidate, noting `ui.error.no_document` would MISNAME the memory
conjunct); it also names the reason the unreachable belt arm should carry
(14b reviewer minor-2). Minted in the shell's disabled-reason namespace
`ui.error.*` (precedent `ui.error.no_document`; the reasonId contract takes
an EXISTING bare cataloged id — `error.command.selection_required` is the
registry sibling), wording from the `error.history.nothing_to_undo`/
`nothing_to_redo` "Nothing to …" register (Nothing to reselect / Nada que
volver a seleccionar / 再選択できる選択範囲はありません). O-UI's log names no
exact id for #2 or #3 (descriptive only) — both minted to the established
pattern and recorded here. Every id verified absent from
`contracts/message_ids.yaml` (no `ui.` area, no `a11y.layers.nudged`), from
all four catalogs, and from this file FIRST; 0 registered ids cataloged, 0
re-minted. Emitters land at O-UI's 14b consuming slices (`error.api.internal`
staged→emitted precedent); all 3 covered in en/es/ja + regenerated pseudo,
staged pending that bind. `bogos/` and `contracts/message_ids.yaml`
UNTOUCHED.

CP-1.0 v1.0.1 puppet-density value mint round (2026-08-20): 3 `ui.*`
value ids minted STAGED by lane O-I18N unblocking the §45.5 puppet
density combo, which stayed RESERVED because its enum VALUES were never
minted (the LABEL `ui.options.puppet_density` = "Density" was cataloged
in the 2026-08-18 chrome-vocabulary rotation, but a segmented/combo
control needs its value ids to render). The three values are the
fewer / normal / more mesh-density (rigidity) scale: minted as siblings
under the label id — `ui.options.puppet_density.fewer`/`.normal`/`.more`
— matching the just-landed 11d `ui.options.selection_op.*` value-id
convention (value siblings keyed directly under their combo-label id,
c98f6c3). Wordings use the project's short-single-word value register
(the selection_op values, `ui.noise.*`, `ui.blend.*`): en Fewer/Normal/
More; es Menos/Normal/Más (Normal matches the cataloged `ui.blend.normal`
es); ja 少ない/標準/多い — 少ない/多い are the natural comparative register,
and Normal is 標準 (the scale-MIDPOINT "standard", deliberately NOT the
blend-mode 通常 which names the default-blend meaning, §27.1
one-id-per-meaning). No ICU placeholders in the batch (nothing to
preserve). Every id verified absent from `contracts/message_ids.yaml`
(no `ui.` area in the frozen v5.5 registry) AND from all four catalogs
AND from this file FIRST; 0 registered ids cataloged, 0 re-minted.
Emitters land at O-UI's puppet options-bar `shell/` touch
(`error.api.internal` staged→emitted precedent); all 3 covered in
en/es/ja + regenerated pseudo, staged pending that bind. `bogos/` and
`contracts/message_ids.yaml` UNTOUCHED.

CP-1.0 v1.0.1 item-25 census pending-reason mint round (2026-08-20): 5
ids minted STAGED by lane O-I18N unblocking O-UI's item-25 census "do-
nothing controls" honesty conversion — the 18 ARMED-SILENT tool rows (a
control wired to a handler that silently no-ops) become ARMED-HONEST (a
control that isn't wired says so). Read first-hand from O-UI's item-25
census (the item-25a surface sweep, message-shape mint proposal grouping
the pending-reason classes). 1 `ui.status.*` + 4 `ui.pending.*` (a new
`ui.` sub-context for the not-yet-wired reason family — semantically
distinct from `ui.error.*`, which names genuine can't-proceed states like
"No document is open", and from `ui.status.*` transient outcomes). THE
HIGH-LEVERAGE ONE is (1) `ui.status.tool_pending` = "{tool} is not wired
to the canvas yet" — ONE `{tool}` template converts all 18 armed-silent
rows with zero engine work, surfaced on the existing `ui.status.*` path on
canvas press (sibling of `ui.status.stroke_preview_pending`); its `{tool}`
placeholder is PRESERVED identical in all four locales (en/es/ja +
pseudo), the RED-4 parity requirement. (2) `ui.pending.needs_engine_arm`
is a BARE sibling of the milestone-tooltip reason
(`ui.panel.layers.milestone_tooltip`) usable as a `reasonId` where that
parameterized id cannot be — the `document_controller.cpp` gap (reasonId
policy attaches only an EXISTING bare cataloged id; the milestone tooltip
is parameterized-only, so those disabled rows carry NO reason today);
serves the class-G engine-arm rows (multi-layer wand/quick sampling,
composite heal/clone, smudge sampleAllLayers) + class-N info-v2 controls.
(3) `ui.pending.option_required` ({tool}/{option}) is the class-D control-
whose-option-isn't-served reason (sides, corner_radius today; grows with
11d Phase 2), parameterized like `milestone_tooltip` on the direct-QML
tooltip surface. (4) `ui.pending.session` ({tool}) is the class-E session-
shaped wording for warp + selection.transform, distinct from the stroke-
shaped preview note; one `{tool}` template serves both. (5)
`ui.pending.history_goto` is the HistoryPanel dead-row reason (rows are
ListItems with no click handler today though the `history.goto` contract
exists) — minted BARE (usable as a reasonId), staged per O-INT dispatch so
it's ready whether or not `history.goto` wiring lands in the same wave.
Reason wordings follow the "not wired yet / arrives at a later milestone"
register of `milestone_tooltip` + `error.unsupported.*` ("aún no es
compatible" / "まだサポートされていません"); es/ja are real translations with
tokens preserved exactly (canvas = lienzo/キャンバス, history = historial/
履歴, options bar = barra de opciones/オプションバー). O-UI's census names no
exact ids (working labels tool-pending / needs-engine-arm-pending /
option-required-pending / session-pending / history-goto-pending, wordings
descriptive) — all 5 minted to the established `ui.` conventions and
recorded here; the bare-vs-parameterized split honors the reasonId
contract (bare ids for the reasonId path #2/#5, parameterized for the
status/direct-tooltip surfaces #1/#3/#4). Every id verified absent from
`contracts/message_ids.yaml` (no `ui.` area in the frozen v5.5 registry)
AND from all four catalogs AND from this file FIRST; 0 registered ids
cataloged, 0 re-minted. Emitters land at O-UI's item-25 wiring waves
(`error.api.internal` staged→emitted precedent); all 5 covered in en/es/ja
+ regenerated pseudo, staged pending that bind. `bogos/` and
`contracts/message_ids.yaml` UNTOUCHED.

CP-1.0 v1.0.1 item-4 brush-hardness mint round (2026-08-20): 1 `ui.*` id
minted STAGED by lane O-I18N unblocking O-UI's item-4 hardness slider (the
shell half of FI-9; O-INT dispatch, carriage-confirmed in
`reports/O-UI-log.md` — "the ui.options.brush_hardness mint routed").
`ui.options.brush_hardness` is the options-bar label for the now-SERVED
`brush.stroke.hardness` param — the FI-9 arc closed end-to-end (bogos
b0f309e → mirror 811965d → registry eb015e8 → engine ff96a54, hardness
SERVES), which LIFTS the standing DO-NOT-MINT reservation on hardness
(the "hold until a v5.6 fold-in row exists" rule — the row now exists via
the sanctioned register channel). Wording from the `proto.options.hardness`
seed per the replace-and-drop policy, colon dropped per the `ui.options.*`
convention ("Size" not "Size:"): Hardness / Dureza / 硬さ — the seed's own
es/ja are the standard Photoshop register and are kept verbatim (minus
colon); the id takes the `brush_` prefix matching its bar sibling
`ui.options.brush_size`. A11Y SIBLING DECISION — surveyed, NOT minted (no
`a11y.tool.brush_hardness`): (1) `a11y.tool.brush_size` is a REGISTERED id
(§31.3/§39.4/§39.6 sources) whose §31.3 golden exists for the CANVAS-CHORD
path — the spec's Brush (T-14) A11y row names "`a11y.tool.brush_size`
announcements on `[`/`]`" ONLY and covers everything else with "bar
controls fully labeled (§31.1)", i.e. the control's own accessible value;
where the spec wants a NEW announcement id it says so explicitly
("`a11y.tool.moved` (new id)", "`a11y.tool.crop_committed` (new id)") and
it names NONE for the `{`/`}` hardness chords; (2) shell precedent: every
options-bar scalar minted since (opacity, flow, puppet_rigidity,
wand_tolerance) shipped with NO announcement mint — the declarative
slider row announces through its own accessible value, and the bespoke
`onBrushSizeChanged` announcer in `shell/qml/Main.qml` exists solely for
the registered golden (toolbar-focus double-speak guarded); (3) no
emitting surface exists today — the shell wires NO `[`/`]`/`{`/`}` chords
yet (the 11e shortcuts slice is queued), and every minted `a11y.*` id so
far had a concrete identified surface. If 11e lands the `{`/`}` chords and
the canvas-context change proves silent, that is a one-id mint-queue
follow-up with a then-real emitter (O-UI's own recorded route: "silence
or mint-queue, never key-echo"). No ICU placeholders in the batch
(nothing to preserve). The id verified absent from
`contracts/message_ids.yaml` (no `ui.` area in the frozen v5.5 registry;
no hardness member anywhere) AND from all four catalogs AND from this
file FIRST; 0 registered ids cataloged, 0 re-minted. Emitter lands at
O-UI's item-4 hardness-row `shell/` touch, has()-gated
(`error.api.internal` staged→emitted precedent); covered in en/es/ja +
regenerated pseudo, staged pending that bind. `bogos/` and
`contracts/message_ids.yaml` UNTOUCHED.

| Id | Args (`?` = optional) | Note |
|---|---|---|
| `ui.action.path_make_selection` | — | slice-V transient Make-Selection action on a committed path |
| `ui.action.redo` | — | — |
| `ui.action.text_to_shape` | — | text.to_shape action (Door 9 one-time bake) |
| `ui.action.undo` | — | — |
| `ui.action.zoom_fit` | — | — |
| `ui.action.zoom_in` | — | — |
| `ui.action.zoom_one_to_one` | — | — |
| `ui.action.zoom_out` | — | — |
| `ui.bg.background_color` | — | — |
| `ui.bg.black` | — | — |
| `ui.bg.transparent` | — | — |
| `ui.bg.white` | — | — |
| `ui.blend.color` | — | — |
| `ui.blend.color_burn` | — | — |
| `ui.blend.color_dodge` | — | — |
| `ui.blend.darken` | — | — |
| `ui.blend.darker_color` | — | — |
| `ui.blend.difference` | — | — |
| `ui.blend.dissolve` | — | — |
| `ui.blend.divide` | — | — |
| `ui.blend.exclusion` | — | — |
| `ui.blend.hard_light` | — | — |
| `ui.blend.hard_mix` | — | — |
| `ui.blend.hue` | — | — |
| `ui.blend.lighten` | — | — |
| `ui.blend.lighter_color` | — | — |
| `ui.blend.linear_burn` | — | — |
| `ui.blend.linear_dodge` | — | — |
| `ui.blend.linear_light` | — | — |
| `ui.blend.luminosity` | — | — |
| `ui.blend.multiply` | — | — |
| `ui.blend.normal` | — | — |
| `ui.blend.overlay` | — | — |
| `ui.blend.pin_light` | — | — |
| `ui.blend.saturation` | — | — |
| `ui.blend.screen` | — | — |
| `ui.blend.soft_light` | — | — |
| `ui.blend.subtract` | — | — |
| `ui.blend.vivid_light` | — | — |
| `ui.canvas.no_document` | — | — |
| `ui.depth.f16` | — | — |
| `ui.depth.f32` | — | — |
| `ui.depth.u16` | — | — |
| `ui.depth.u8` | — | — |
| `ui.dialog.about.acknowledgments` | — | heading only — LICENSE-FIXED credit lines rendered beneath it (FTL line per `deps/freetype.yaml` requiredNotice; rgb2spec/libmypaint credits per the port records) are verbatim untranslated content, never catalog values; the lucide/lensfun credits are the exception BY THEIR OWN RECORDS (noticeSurfaces route them through the About mint batch) — id-backed, LOCALE-INVARIANT rows below. **★ CORRECTED 2026-09-12 (O-UI, C-802) — THE ORIGINAL SENTENCE STAYS AS PRINTED AND IS WRONG ABOUT ONE COMPONENT.** It groups libmypaint with rgb2spec as "verbatim untranslated content, never catalog values", and that is why this table carried ELEVEN of O-UI's twelve About/Licenses ids and not the twelfth. **The product routes libmypaint's credit THROUGH THE CATALOG**: `AboutDialog.qml:116` emits `tr("ui.dialog.about.credit." + modelData)` over a model whose rows are `libmypaint`, `lucide_icons`, `lensfun_database`. Adopting the twelve with this row absent makes `audit.py` fire **RED-5 untracked catalog keys** — measured by dry-running the adoption across all four locales and restoring them byte-identical. rgb2spec is untouched by this correction and remains verbatim content: it is emitted NOWHERE. |
| `ui.dialog.about.commit` | {commit} | B-09 About: the build commit row; hidden when empty, and BLANKED for the tour capture (C-759) because a golden holding a sha can never match a fresh run |
| `ui.dialog.about.credit.cgats21_profiles` | — | LOCALE-INVARIANT (ui.profile.* precedent); deps/cgats21-cmyk-profiles.yaml attribution surface: X-Rite copyright + Idealliance availability line |
| `ui.dialog.about.credit.lensfun_database` | — | CP-1.0 backlog round: `deps/lensfun-db.yaml` obligations ("attribution surface in credits", CC-BY-SA-3.0 §4(c)) + noticeSurfaces credits-half, which queues this line "with the O-UI About mints". Value satisfies §4(c): work title (lensfun database), attribution party (the record's contributors line, verbatim), license notice (CC-BY-SA-3.0), URI. LOCALE-INVARIANT (identical bytes en/es/ja, `ui.profile.*` precedent): attribution content, not prose to translate |
| `ui.dialog.about.credit.libmypaint` | — | **ADDED 2026-09-12 (O-UI, C-802) — the row this table was missing, found by DRY-RUNNING the adoption rather than by reading.** With all four locales carrying the twelve, `audit.py` clears RED-2 and RED-3 and then fires **RED-5 untracked catalog keys: ['ui.dialog.about.credit.libmypaint']** — the catalog would carry a key the registry∪MINTED set does not track. `deps/port-libmypaint.yaml` noticeSurfaces requires it: *"About-screen credit line (memo §2 over-attribution posture)"*, with obligations *"ISC — keep the copyright notice + permission notice … About-surface credit either way"*. The shipped value carries the ISC copyright line verbatim, which the obligations require and the record's own one-line wording does not. LOCALE-INVARIANT on the `lensfun_database` / `lucide_icons` basis: attribution content, not prose to translate — identical bytes en/es/ja. |
| `ui.dialog.about.credit.lucide_icons` | — | CP-1.0 backlog round: `deps/lucide.yaml` noticeSurfaces ("Lucide icon acknowledgment (ISC AND MIT, both copyright lines) in the shell About dialog's third-party credits surface (O-UI About mints)"). Value carries both record copyright lines verbatim + both license names + upstream. LOCALE-INVARIANT, same basis |
| `ui.dialog.about.license` | — | — |
| `ui.dialog.about.no_affiliation` | — | OQ-I18N-3 acknowledgment surface (pairs with `.trademarks`) |
| `ui.dialog.about.qt_lgpl` | — | B-09 About: the Qt LGPL-3.0 relink sentence; states the user's §4(d) RIGHT rather than a packaging claim (16e45b2aa) |
| `ui.dialog.about.third_party_notices` | — | — |
| `ui.dialog.about.title` | — | — |
| `ui.dialog.about.trademarks` | — | OQ-I18N-3 acknowledgment line |
| `ui.dialog.about.version` | {version} | — |
| `ui.dialog.apply` | — | — |
| `ui.dialog.assign_profile.profile` | — | — |
| `ui.dialog.assign_profile.title` | — | — |
| `ui.dialog.cancel` | — | — |
| `ui.dialog.canvas_size.anchor` | — | — |
| `ui.dialog.canvas_size.anchor_cell` | {row} {column} | A11Y-3 wave-B mint: ONE parameterized accessible NAME for the nine 3×3 anchor Buttons (visible text is the glyph pair "●"/"·", meaningless to AT); "row {row}, column {column}"-shaped per the queue |
| `ui.dialog.canvas_size.background` | — | — |
| `ui.dialog.canvas_size.height` | — | — |
| `ui.dialog.canvas_size.relative` | — | — |
| `ui.dialog.canvas_size.title` | — | — |
| `ui.dialog.canvas_size.width` | — | — |
| `ui.dialog.class_c_note` | — | — |
| `ui.dialog.close` | — | shared dialog close action |
| `ui.dialog.convert_profile.bpc` | — | — |
| `ui.dialog.convert_profile.dither` | — | — |
| `ui.dialog.convert_profile.intent` | — | — |
| `ui.dialog.convert_profile.profile` | — | — |
| `ui.dialog.convert_profile.title` | — | — |
| `ui.dialog.crop.allow_growth` | — | — |
| `ui.dialog.crop.delete_pixels` | — | registry alias `tool.crop.option.delete_cropped` NOW CATALOGED (2026-08-13) — swap unblocked; this mint drops at O-UI's swap commit |
| `ui.dialog.crop.height` | — | — |
| `ui.dialog.crop.straighten` | — | — |
| `ui.dialog.crop.title` | — | — |
| `ui.dialog.crop.width` | — | — |
| `ui.dialog.crop.x` | — | — |
| `ui.dialog.crop.y` | — | — |
| `ui.dialog.delete_confirm.message` | `{name}` | 14b: §32.5 named-object destructive-confirm TEMPLATE (message body) — the `DeleteConfirmDialog.qml` mint-queue candidate; `{name}` = the layer's own name (info truth), PRESERVED in all four locales; register from `dialog.recovery.discard_confirm` (Delete verb = `ui.menu.edit.delete`) |
| `ui.dialog.depth.dither` | — | — |
| `ui.dialog.depth.title` | — | — |
| `ui.dialog.depth.to` | — | — |
| `ui.dialog.discard_confirm.title` | — | O-INT #8, standing priority (4)-3 confirm-on-replace (operator ruling 2026-08-31: "the dirty flag ships WITH its readers in one change, or it is decoration"). MINTED RATHER THAN COMPOSED, and the check that forced it: `dialog.recovery.discard_confirm_never_saved` was recorded in the spec seed as "the exact sentence a confirm-on-replace needs" — its VALUE is "This document has never been saved: it exists nowhere else and will be deleted entirely", which is about deleting a RECOVERY LOG, and its `dialog.recovery.` namespace says so. The id existed; the string said something else. `DeleteConfirmDialog`'s precedent composes from cataloged values where the ACTION NAME carries the meaning — here the MESSAGE IS THE FEATURE, so there was nothing to compose from. Cancel reuses `ui.dialog.cancel`; no fourth id minted. Readers: `DiscardConfirmDialog.qml` via `Main.qml` `confirmDiscard()` — File > New, File > Open (covering the recovery branch), and the window close (dialog title); v5.6 fold-in: register under `ui.dialog` beside `delete_confirm` |
| `ui.dialog.discard_confirm.message` | — | O-INT #8, standing priority (4)-3 confirm-on-replace (operator ruling 2026-08-31: "the dirty flag ships WITH its readers in one change, or it is decoration"). MINTED RATHER THAN COMPOSED, and the check that forced it: `dialog.recovery.discard_confirm_never_saved` was recorded in the spec seed as "the exact sentence a confirm-on-replace needs" — its VALUE is "This document has never been saved: it exists nowhere else and will be deleted entirely", which is about deleting a RECOVERY LOG, and its `dialog.recovery.` namespace says so. The id existed; the string said something else. `DeleteConfirmDialog`'s precedent composes from cataloged values where the ACTION NAME carries the meaning — here the MESSAGE IS THE FEATURE, so there was nothing to compose from. Cancel reuses `ui.dialog.cancel`; no fourth id minted. Readers: `DiscardConfirmDialog.qml` via `Main.qml` `confirmDiscard()` — File > New, File > Open (covering the recovery branch), and the window close (the proposition; no ICU placeholders — the document is not named, because the shell is single-document and the title bar already names it); v5.6 fold-in: register under `ui.dialog` beside `delete_confirm` |
| `ui.dialog.discard_confirm.discard` | — | O-INT #8, standing priority (4)-3 confirm-on-replace (operator ruling 2026-08-31: "the dirty flag ships WITH its readers in one change, or it is decoration"). MINTED RATHER THAN COMPOSED, and the check that forced it: `dialog.recovery.discard_confirm_never_saved` was recorded in the spec seed as "the exact sentence a confirm-on-replace needs" — its VALUE is "This document has never been saved: it exists nowhere else and will be deleted entirely", which is about deleting a RECOVERY LOG, and its `dialog.recovery.` namespace says so. The id existed; the string said something else. `DeleteConfirmDialog`'s precedent composes from cataloged values where the ACTION NAME carries the meaning — here the MESSAGE IS THE FEATURE, so there was nothing to compose from. Cancel reuses `ui.dialog.cancel`; no fourth id minted. Readers: `DiscardConfirmDialog.qml` via `Main.qml` `confirmDiscard()` — File > New, File > Open (covering the recovery branch), and the window close (the destructive button; deliberately NOT `dialog.recovery.discard` "Discard…", whose ellipsis promises a further step this dialog does not have); v5.6 fold-in: register under `ui.dialog` beside `delete_confirm` |
| `ui.dialog.fade.mode` | — | fade blend chooser; values reuse ui.blend.* (camelCase→snake map) |
| `ui.dialog.fade.opacity` | — | fade opacity [0,1] wire |
| `ui.dialog.fade.title` | — | filter.apply §10.3 fade pass dialog |
| `ui.dialog.file.filter_appname` | — | — |
| `ui.dialog.file.filter_bmp` | — | — |
| `ui.dialog.file.filter_gif` | — | — |
| `ui.dialog.file.filter_images` | — | import dialog; extend when further import codecs (e.g. PSD) reach the shell ingress |
| `ui.dialog.file.filter_jpeg` | — | — |
| `ui.dialog.file.filter_ora` | — | — |
| `ui.dialog.file.filter_png` | — | — |
| `ui.dialog.file.filter_tiff` | — | — |
| `ui.dialog.file.filter_webp` | — | — |
| `ui.dialog.fill.antialias` | — | — |
| `ui.dialog.fill.blend` | — | — |
| `ui.dialog.fill.color_swatch` | — | A11Y-3 ROW-17 DECISION — DISTINCT id, NOT shared `a11y.color.foreground`: FillDialog's swatch is the fill's content color (dialog-local `fillColor`, default #202020; the dialog's own R15 comment — foreground/background are headless engine constants, "the shell's brush swatch does not feed them", no app foreground-color state exists at CP-0.5), so naming it "foreground" would be false. Accessible NAME for the wave-B focusable color button; wording is O-I18N's |
| `ui.dialog.fill.content` | — | — |
| `ui.dialog.fill.contiguous` | — | — |
| `ui.dialog.fill.opacity` | — | — |
| `ui.dialog.fill.preserve_transparency` | — | — |
| `ui.dialog.fill.title` | — | — |
| `ui.dialog.fill.tolerance` | — | — |
| `ui.dialog.adjust.channel` | — | v1.0.1 item 4 class (b): levels channel/tuple label. The five tuple labels are shared by all four channels because the ENGINE declares the tuple once as $defs/tuple; four copies would be four places to drift from one schema |
| `ui.dialog.adjust.in_black` | — | v1.0.1 item 4 class (b): levels channel/tuple label. The five tuple labels are shared by all four channels because the ENGINE declares the tuple once as $defs/tuple; four copies would be four places to drift from one schema |
| `ui.dialog.adjust.in_white` | — | v1.0.1 item 4 class (b): levels channel/tuple label. The five tuple labels are shared by all four channels because the ENGINE declares the tuple once as $defs/tuple; four copies would be four places to drift from one schema |
| `ui.dialog.adjust.out_black` | — | v1.0.1 item 4 class (b): levels channel/tuple label. The five tuple labels are shared by all four channels because the ENGINE declares the tuple once as $defs/tuple; four copies would be four places to drift from one schema |
| `ui.dialog.adjust.out_white` | — | v1.0.1 item 4 class (b): levels channel/tuple label. The five tuple labels are shared by all four channels because the ENGINE declares the tuple once as $defs/tuple; four copies would be four places to drift from one schema |
| `ui.dialog.adjust.hue` | — | v1.0.1 item 4 class (c): hue_saturation band/colorize label. The three band labels are shared by all SEVEN colour bands because the engine declares one $defs/band; the four colorize rows are flat top-level params rendered UNGATED, since the schema states no dependency on the colorize boolean and disabling a param the engine accepts would be an invented constraint |
| `ui.dialog.adjust.saturation` | — | v1.0.1 item 4 class (c): hue_saturation band/colorize label. The three band labels are shared by all SEVEN colour bands because the engine declares one $defs/band; the four colorize rows are flat top-level params rendered UNGATED, since the schema states no dependency on the colorize boolean and disabling a param the engine accepts would be an invented constraint |
| `ui.dialog.adjust.lightness` | — | v1.0.1 item 4 class (c): hue_saturation band/colorize label. The three band labels are shared by all SEVEN colour bands because the engine declares one $defs/band; the four colorize rows are flat top-level params rendered UNGATED, since the schema states no dependency on the colorize boolean and disabling a param the engine accepts would be an invented constraint |
| `ui.dialog.adjust.colorize` | — | v1.0.1 item 4 class (c): hue_saturation band/colorize label. The three band labels are shared by all SEVEN colour bands because the engine declares one $defs/band; the four colorize rows are flat top-level params rendered UNGATED, since the schema states no dependency on the colorize boolean and disabling a param the engine accepts would be an invented constraint |
| `ui.dialog.adjust.colorize_hue` | — | v1.0.1 item 4 class (c): hue_saturation band/colorize label. The three band labels are shared by all SEVEN colour bands because the engine declares one $defs/band; the four colorize rows are flat top-level params rendered UNGATED, since the schema states no dependency on the colorize boolean and disabling a param the engine accepts would be an invented constraint |
| `ui.dialog.adjust.colorize_sat` | — | v1.0.1 item 4 class (c): hue_saturation band/colorize label. The three band labels are shared by all SEVEN colour bands because the engine declares one $defs/band; the four colorize rows are flat top-level params rendered UNGATED, since the schema states no dependency on the colorize boolean and disabling a param the engine accepts would be an invented constraint |
| `ui.dialog.adjust.colorize_lightness` | — | v1.0.1 item 4 class (c): hue_saturation band/colorize label. The three band labels are shared by all SEVEN colour bands because the engine declares one $defs/band; the four colorize rows are flat top-level params rendered UNGATED, since the schema states no dependency on the colorize boolean and disabling a param the engine accepts would be an invented constraint |
| `ui.dialog.adjust.curve_editor` | — | v1.0.1 item 4 class (c): the curve editor's ACCESSIBLE NAME. The widget is a bare Item wrapping a Canvas, so the §31.5 lint's control list does not reach it — it is named because a screen reader would meet it unnamed, not because a guard demanded it |
| `ui.dialog.adjust.brightness` | — | v1.0.1 item 4 class (a): parameter label for the adjustment dialog. A `ui.dialog.adjust.*` family rather than reusing `ui.dialog.filter.*` — `offset` exists in both and means different things (a filter kernel offset vs an exposure offset), so sharing the key would make one of them wrong the first time a locale translated for context |
| `ui.dialog.adjust.contrast` | — | v1.0.1 item 4 class (a): parameter label for the adjustment dialog. A `ui.dialog.adjust.*` family rather than reusing `ui.dialog.filter.*` — `offset` exists in both and means different things (a filter kernel offset vs an exposure offset), so sharing the key would make one of them wrong the first time a locale translated for context |
| `ui.dialog.adjust.legacy` | — | v1.0.1 item 4 class (a): parameter label for the adjustment dialog. A `ui.dialog.adjust.*` family rather than reusing `ui.dialog.filter.*` — `offset` exists in both and means different things (a filter kernel offset vs an exposure offset), so sharing the key would make one of them wrong the first time a locale translated for context |
| `ui.dialog.adjust.exposure` | — | v1.0.1 item 4 class (a): parameter label for the adjustment dialog. A `ui.dialog.adjust.*` family rather than reusing `ui.dialog.filter.*` — `offset` exists in both and means different things (a filter kernel offset vs an exposure offset), so sharing the key would make one of them wrong the first time a locale translated for context |
| `ui.dialog.adjust.offset` | — | v1.0.1 item 4 class (a): parameter label for the adjustment dialog. A `ui.dialog.adjust.*` family rather than reusing `ui.dialog.filter.*` — `offset` exists in both and means different things (a filter kernel offset vs an exposure offset), so sharing the key would make one of them wrong the first time a locale translated for context |
| `ui.dialog.adjust.gamma` | — | v1.0.1 item 4 class (a): parameter label for the adjustment dialog. A `ui.dialog.adjust.*` family rather than reusing `ui.dialog.filter.*` — `offset` exists in both and means different things (a filter kernel offset vs an exposure offset), so sharing the key would make one of them wrong the first time a locale translated for context |
| `ui.dialog.adjust.level` | — | v1.0.1 item 4 class (a): parameter label for the adjustment dialog. A `ui.dialog.adjust.*` family rather than reusing `ui.dialog.filter.*` — `offset` exists in both and means different things (a filter kernel offset vs an exposure offset), so sharing the key would make one of them wrong the first time a locale translated for context |
| `ui.dialog.filter.amount` | — | shared param label — engine param `amount` (noise/unsharp) |
| `ui.dialog.filter.distribution` | — | add-noise distribution chooser label |
| `ui.dialog.filter.divisor` | — | custom-convolve divisor label |
| `ui.dialog.filter.kernel` | — | custom-convolve kernel label |
| `ui.dialog.filter.monochromatic` | — | add-noise toggle |
| `ui.dialog.filter.offset` | — | custom-convolve offset label |
| `ui.dialog.filter.radius` | — | shared param label — engine param `radius` (gaussian/box/median/high_pass/unsharp) |
| `ui.dialog.filter.seed` | — | add-noise seed label (§16.5 stochastic) |
| `ui.dialog.filter.threshold` | — | shared param label — engine param `threshold` (unsharp) |
| `ui.dialog.flatten.discard_hidden` | — | — |
| `ui.dialog.flatten.title` | — | — |
| `ui.dialog.licenses.body` | {name} | B-09 licences viewer: the body pane's Accessible.name, naming WHICH of the eleven texts is shown |
| `ui.dialog.licenses.body_none` | — | B-09 licences viewer: the body pane's Accessible.name with no selection |
| `ui.dialog.licenses.title` | — | B-09 licences viewer title |
| `ui.dialog.licenses.truncated` | — | B-09 licences viewer: the cap notice; 102 of the notices aggregate's 165 texts fall beyond the 256 KB render cap, so this points at Help ▸ Open Licenses Folder |
| `ui.dialog.licenses.unavailable` | — | B-09 licences viewer: no staged Contents/Resources/licenses resolved — the state every dev build is in |
| `ui.dialog.mode_convert.flatten` | — | flatten checkbox label |
| `ui.dialog.mode_convert.title` | — | doc.mode_convert dialog |
| `ui.dialog.modify.amount` | — | — |
| `ui.dialog.modify.apply` | — | — |
| `ui.dialog.modify.border` | — | — |
| `ui.dialog.modify.contract` | — | — |
| `ui.dialog.modify.expand` | — | — |
| `ui.dialog.modify.feather` | — | — |
| `ui.dialog.modify.op` | — | — |
| `ui.dialog.modify.smooth` | — | — |
| `ui.dialog.modify.title` | — | — |
| `ui.dialog.new.background` | — | — |
| `ui.dialog.new.background.black` | — | — |
| `ui.dialog.new.background.transparent` | — | — |
| `ui.dialog.new.background.white` | — | — |
| `ui.dialog.new.computed_px` | {width} {height} | v1.0.1 New Document/Resize computed-size readout; template `{width} × {height} px` (× = U+00D7), shape parallel to `ui.status.doc_size`/`cli.info.doc` — locale-invariant, shell interpolates width/height |
| `ui.dialog.new.create` | — | — |
| `ui.dialog.new.dpi` | — | v1.0.1 New Document DPI field label (O-UI feedback item 2/3) |
| `ui.dialog.new.height` | — | — |
| `ui.dialog.new.preset` | — | v1.0.1 New Document preset-picker label (O-UI feedback item 2/3) |
| `ui.dialog.new.title` | — | — |
| `ui.dialog.new.width` | — | — |
| `ui.dialog.resize.constrain` | — | — |
| `ui.dialog.resize.driving` | — | — |
| `ui.dialog.resize.height` | — | — |
| `ui.dialog.resize.longest_edge` | — | — |
| `ui.dialog.resize.ppi` | — | — |
| `ui.dialog.resize.resample` | — | — |
| `ui.dialog.resize.scale_effects` | — | — |
| `ui.dialog.resize.title` | — | registry alias candidate: `dialog.image_size.title` (registered, cataloged) names this meaning — swap is O-UI's call |
| `ui.dialog.resize.width` | — | — |
| `ui.dialog.rotate.angle` | — | — |
| `ui.dialog.rotate.expand` | — | — |
| `ui.dialog.rotate.flip_h` | — | — |
| `ui.dialog.rotate.flip_v` | — | — |
| `ui.dialog.rotate.title` | — | — |
| `ui.dialog.trim.basis` | — | — |
| `ui.dialog.trim.title` | — | — |
| `ui.error.action_unavailable` | — | P1-4(a): the GENERIC fallback reason for a chord whose precondition is unmet and whose failing conjunct has no cataloged id of its own. Used ONLY when the availability row's reasonId is absent — the reasonId policy forbids naming the WRONG conjunct, not saying something true-but-unspecific, and silence is what it replaces |
| `ui.error.bad_argument` | — | row 6 mask panel: client-side refusal for an unknown mask property name (`setMaskProp`). NOT a caption. DORMANT (cataloged, no emitter yet: B-12 MaskPanel). A-05, operator ruling 2026-09-05 |
| `ui.error.no_document` | — | client-side wire code (no-document precondition, document_controller.cpp:168), NOT in the seed — sweep find; description cataloged like the `error.*` family |
| `ui.error.nothing_to_reselect` | — | 14b: the reselect row's disabled-tooltip reasonId (`commandAvailability` left it absent — `ui.error.no_document` would misname the memory conjunct); also names the unreachable belt arm's reason (14b reviewer minor-2); `ui.error.*` disabled-reason namespace (`ui.error.no_document` precedent), "Nothing to …" register (`error.history.nothing_to_undo`) |
| `ui.fill.content.color` | — | — |
| `ui.intent.absolute` | — | family-constructed emitter (quoted prefix + value) |
| `ui.intent.perceptual` | — | family-constructed emitter (quoted prefix + value) |
| `ui.intent.relative` | — | family-constructed emitter (quoted prefix + value) |
| `ui.intent.saturation` | — | family-constructed emitter (quoted prefix + value) |
| `ui.menu.edit` | — | — |
| `ui.menu.edit.copy` | — | — |
| `ui.menu.edit.cut` | — | — |
| `ui.menu.edit.delete` | — | Edit-menu "Delete" (i18n-menu round; the availability row exists — `shell/tests/tst_command_availability.cpp` — but no menu id yet). NOT the `history.*`/`op.delete` kinds — this is the menu item name |
| `ui.menu.edit.fill` | — | — |
| `ui.menu.edit.paste` | — | — |
| `ui.menu.edit.paste_into` | — | Edit-menu "Paste Into" (i18n-menu round; engine door exists — `contracts/commands/edit.paste_into.yaml` — and `history.edit.paste_into` is a REGISTERED history label, but no MENU id; this names the menu item, meaning distinct from the history label per §27.1) |
| `ui.menu.edit.stroke_path` | — | disabled-NI carrier (the `stroke_path` kind is CP-1.0 scope; the item ships disabled until the emitting slice lands the command path) |
| `ui.menu.file` | — | — |
| `ui.menu.file.export` | — | registry alias candidate: `menu.file.export_as` (registered, cataloged) — meaning check at swap time (Export vs Export As) |
| `ui.menu.file.import` | — | — |
| `ui.menu.file.open` | — | registry alias candidate: `app.open` (registered, cataloged) |
| `ui.menu.file.save` | — | — |
| `ui.menu.file.save_as` | — | — |
| `ui.menu.filter` | — | Filter menu title (matches proto.menu.filter register) |
| `ui.menu.help` | — | Help menu title |
| `ui.menu.help.about` | — | Help ▸ About item |
| `ui.menu.help.open_licenses` | — | Help ▸ Open Licenses Folder — WORD 3's fallback, and the PRIMARY route to 62% of the licence texts (C-749) |
| `ui.menu.image` | — | — |
| `ui.menu.image.assign_profile` | — | — |
| `ui.menu.image.auto_color` | — | no ellipsis: these apply immediately (no dialog), matching the family's convention where `…` marks a dialog. Meaning differs from the REGISTERED `history.image.auto_*` (history-entry label, not menu item) — rule-1 checked. |
| `ui.menu.image.auto_contrast` | — | no ellipsis: these apply immediately (no dialog), matching the family's convention where `…` marks a dialog. Meaning differs from the REGISTERED `history.image.auto_*` (history-entry label, not menu item) — rule-1 checked. |
| `ui.menu.image.auto_tone` | — | no ellipsis: these apply immediately (no dialog), matching the family's convention where `…` marks a dialog. Meaning differs from the REGISTERED `history.image.auto_*` (history-entry label, not menu item) — rule-1 checked. |
| `ui.menu.image.canvas_size` | — | — |
| `ui.menu.image.convert_profile` | — | — |
| `ui.menu.image.crop` | — | — |
| `ui.menu.image.depth_convert` | — | — |
| `ui.menu.image.flatten` | — | — |
| `ui.menu.image.mode` | — | Image menu submenu title |
| `ui.menu.image.resize` | — | — |
| `ui.menu.image.rotate` | — | — |
| `ui.menu.image.trim` | — | — |
| `ui.menu.layer` | — | Q-D72 / G-17 keyboard door: Layer menu title — label per OPERATOR WORD 11 (3), minted through O-INT in the A-05 shape per WORD 14 (1); bound in lane/ui `shell/qml/Main.qml`, staged ahead of that lane's merge |
| `ui.menu.layer.delete` | — | Q-D72 / G-17 keyboard door: Layer ▸ Delete item — label per OPERATOR WORD 11 (3), minted through O-INT in the A-05 shape per WORD 14 (1); bound in lane/ui `shell/qml/Main.qml`, staged ahead of that lane's merge |
| `ui.menu.layer.group` | — | Q-D72 / G-17 keyboard door: Layer ▸ Group item (Ctrl+G) — label per OPERATOR WORD 11 (3), minted through O-INT in the A-05 shape per WORD 14 (1); bound in lane/ui `shell/qml/Main.qml`, staged ahead of that lane's merge |
| `ui.menu.layer.merge_down` | — | Q-D72 / G-17 keyboard door: Layer ▸ Merge Down item (Ctrl+E) — label per OPERATOR WORD 11 (3), minted through O-INT in the A-05 shape per WORD 14 (1); bound in lane/ui `shell/qml/Main.qml`, staged ahead of that lane's merge |
| `ui.menu.layer.move_down` | — | Q-D72 / G-17 keyboard door: Layer ▸ Move Down item (Ctrl+[) — label per OPERATOR WORD 11 (3), minted through O-INT in the A-05 shape per WORD 14 (1); bound in lane/ui `shell/qml/Main.qml`, staged ahead of that lane's merge |
| `ui.menu.layer.move_up` | — | Q-D72 / G-17 keyboard door: Layer ▸ Move Up item (Ctrl+]) — label per OPERATOR WORD 11 (3), minted through O-INT in the A-05 shape per WORD 14 (1); bound in lane/ui `shell/qml/Main.qml`, staged ahead of that lane's merge |
| `ui.menu.layer.ungroup` | — | Q-D72 / G-17 keyboard door: Layer ▸ Ungroup item (Ctrl+Shift+G) — label per OPERATOR WORD 11 (3), minted through O-INT in the A-05 shape per WORD 14 (1); bound in lane/ui `shell/qml/Main.qml`, staged ahead of that lane's merge |
| `ui.menu.select` | — | — |
| `ui.menu.select.all` | — | — |
| `ui.menu.select.color_range` | — | O-INT ruling: color-range in scope. Surface is the Select MENU (Photoshop Select > Color Range…), not the tool palette — keyed under `ui.menu.select.*` (dialog "…" like `ui.menu.select.modify`), NOT `ui.tool.color_range` |
| `ui.menu.select.deselect` | — | — |
| `ui.menu.select.invert` | — | — |
| `ui.menu.select.load_layer_alpha` | — | doors-flip queue; layer-alpha source arm |
| `ui.menu.select.load_saved` | — | doors-flip queue; tracked-id load (request-9 interim) |
| `ui.menu.select.modify` | — | — |
| `ui.menu.select.modify_submenu` | — | Q-D32: the canvas context menu's SUBMENU title. `ui.menu.select.modify` carries an ellipsis that is CORRECT on the menu-bar item (Main.qml opens modifyDialog directly) and FALSE on a submenu title, which opens a submenu of three ops. Same bytes, right in one seat and wrong in the other. |
| `ui.menu.select.save_selection` | — | doors-flip queue; Select menu, selection.save create-arm |
| `ui.mode.cmyk_a` | — | wire-enum family |
| `ui.mode.gray_a` | — | wire-enum family |
| `ui.mode.lab_a` | — | wire-enum family |
| `ui.mode.rgba` | — | wire-enum family ui.mode.<to> (contract doc.mode_convert to-enum) |
| `ui.noise.gaussian` | — | distribution value family |
| `ui.noise.uniform` | — | distribution value family ui.noise.<v> |
| `ui.options.antialias` | — | 11d Phase-2: selection/fill anti-alias toggle; reuses the `ui.dialog.fill.antialias` register (Anti-alias/Suavizado/アンチエイリアス) |
| `ui.options.blend` | — | 11d Phase-2: brush/eraser blend-mode combo LABEL only — values REUSE the 27 `ui.blend.*`, none re-minted; "Blend"/"Mezcla"/"ブレンド" per `ui.panel.layers.blend`/`ui.dialog.fill.blend` (distinct meaning from `ui.options.selection_op` "Mode") |
| `ui.options.brush_color` | — | toolbar brush-swatch color label; NOT `a11y.color.foreground` (O-INT false-name ruling — no app foreground state at CP-0.5) |
| `ui.options.brush_hardness` | — | item-4: options-bar label for the SERVED `brush.stroke.hardness` param (FI-9 arc closed — engine ff96a54; the hardness DO-NOT-MINT reservation lifted); from `proto.options.hardness` (colon dropped — Hardness/Dureza/硬さ), `brush_` prefix per `ui.options.brush_size`; a11y sibling surveyed NOT minted (round paragraph — §31.1 labeled-control coverage, no spec-named id, no chord surface yet) |
| `ui.options.brush_size` | — | — |
| `ui.options.commit_as` | — | options-bar label for the pen commit choice; values are `ui.options.commit_shape` / `ui.options.commit_path` |
| `ui.options.commit_path` | — | — |
| `ui.options.commit_shape` | — | — |
| `ui.options.feather` | — | 11d Phase-2: selection feather radius label; from `proto.options.feather` (colon dropped — Feather/Difuminado/ぼかし) |
| `ui.options.flow` | — | 11d Phase-2: brush/eraser flow label; clean mint, the Photoshop sibling of `ui.options.opacity` (Flow/Flujo/流量) |
| `ui.options.font_family` | — | family picker label (§44.1 resolution is the engine's) |
| `ui.options.font_size` | — | size 0.1–1296 pt per text.add contract |
| `ui.options.font_style` | — | styledRunStyle font.style |
| `ui.options.opacity` | — | 11d Phase-2: brush/eraser opacity label; reuses the cataloged opacity register (`ui.dialog.fill.opacity`/`ui.panel.layers.opacity` — Opacity/Opacidad/不透明度) |
| `ui.options.puppet_density` | — | puppet options bar (§45.5 knob) |
| `ui.options.puppet_density.fewer` | — | puppet-density value (§45.5 mesh-density scale, low end): fewer mesh points / lower rigidity; short-word register of its siblings (Fewer/Menos/少ない) |
| `ui.options.puppet_density.more` | — | puppet-density value (§45.5 mesh-density scale, high end): more mesh points / higher rigidity (More/Más/多い) |
| `ui.options.puppet_density.normal` | — | puppet-density value (§45.5 mesh-density scale, midpoint): standard mesh density; scale-midpoint register (Normal/Normal/標準 — 標準 not the blend 通常) |
| `ui.options.puppet_expansion` | — | puppet options bar (§45.5 knob) |
| `ui.options.puppet_rigidity` | — | puppet options bar (§45.5 knob) |
| `ui.options.puppet_show_mesh` | — | puppet options bar toggle |
| `ui.options.resample` | — | 11d Phase-2: transform resample combo LABEL only — values REUSE `ui.resample.*`, none re-minted; reuses the `ui.dialog.resize.resample` register (Resample/Remuestreo/再サンプル) |
| `ui.options.sample_all_layers` | — | 11d Phase-2: wand "sample all layers" toggle; clean mint (Sample All Layers/Muestrear todas las capas/全レイヤーを対象, `ui.options.puppet_show_mesh` title-case register) |
| `ui.options.scale_effects` | — | 11d Phase-2: transform scale-effects toggle; reuses the `ui.dialog.resize.scale_effects` register verbatim (Scale effects/Escalar efectos/効果を拡大縮小) |
| `ui.options.selection_op` | — | 11d Phase-2: selection-op segmented control LABEL (KS-1's §32.3 expectation); from `proto.options.selection_mode` (colon dropped — Mode/Modo/モード); values are the `.replace`/`.add`/`.subtract`/`.intersect` rows below |
| `ui.options.selection_op.add` | — | 11d Phase-2: selection-op value; `proto.options.mode.add` verbatim (Add/Añadir/追加) |
| `ui.options.selection_op.intersect` | — | 11d Phase-2: selection-op value — the ONE genuinely new value mint (proto covers only replace/add/subtract); Intersect/Intersecar/交差, the short-single-word register of its siblings |
| `ui.options.selection_op.replace` | — | 11d Phase-2: selection-op value; `proto.options.mode.replace` verbatim (Replace/Reemplazar/置換) |
| `ui.options.selection_op.subtract` | — | 11d Phase-2: selection-op value; `proto.options.mode.subtract` verbatim (Subtract/Restar/減算) |
| `ui.options.stroke_width` | — | shape stroke-width label (vector outline, "Trazo"/"線" register — distinct from brush-stroke wording) |
| `ui.options.wand_contiguous` | — | — |
| `ui.options.wand_tolerance` | — | — |
| `ui.options.warp_bend` | — | bendPct label (−100..100 per contract) |
| `ui.notice.heal_no_effect_empty_target` | — | Q-D7 rider 1 (operator, 2026-08-29): heal.stroke onto a fully transparent layer is a DEFINED no-op (heal.stroke.yaml:210 — heal never modifies alpha), and it was a SILENT one. Raised through the `ui.*` toast door (actionUnavailable), not commandRefused, because the command was ACCEPTED and inventing an `error.*` code for a successful command is a different lie. |
| `ui.panel.history.empty` | — | — |
| `ui.panel.history.title` | — | — |
| `ui.panel.layers.add` | — | — |
| `ui.panel.layers.add_group` | — | — |
| `ui.panel.layers.add_adjustment` | — | v1.0.1 item 4 class (a): the Layers-panel button that opens the adjustment menu. It previously carried the caption `adjustment.invert` because invert was the ONLY kind it could submit; now that it opens a family it needs a family name, terse like its neighbours (Add / Grp+ / Dup / Del / Mask / Fill) |
| `ui.panel.layers.add_fill` | — | v1.0.1 fill route: the Layers-panel button opening the layer.add{kind:'fill'} gradient menu. The engine has served fill layers since the demo document was authored (demo/build-demo.jsonl lines 4-5 ship two) and no menu ever reached them |
| `ui.panel.layers.add_mask` | — | v1.0.1 mask route: the Layers-panel button that opens the `mask.add` menu. DEMO-SCRIPT.md:78 recorded "there is no `mask.add` route in the shell QML at all" — the shell could composite a masked layer and could not create the mask. Terse like its neighbours (Add / Grp+ / Dup / Del) because it shares a 3-column action strip |
| `ui.panel.layers.blend` | — | — |
| `ui.panel.layers.down` | — | glyph label (↓), locale-invariant |
| `ui.panel.layers.duplicate` | — | — |
| `ui.panel.layers.fill_gradient_linear` | — | v1.0.1 fill route: linear black-to-white in linearRgb — the demo document's own first fill layer, envelope copied from demo/build-demo.jsonl line 4 rather than invented |
| `ui.panel.layers.fill_gradient_perceptual` | — | v1.0.1 fill route: the same geometry in perceptual space, which is the `space` parameter made visible — the demo's line 5 pairs them precisely to show the midtones differ |
| `ui.panel.layers.fill_gradient_radial` | — | v1.0.1 fill route: the `kind` parameter made visible (the gradient enum registers linear/radial/angular/reflected/diamond/noise; radial is the one whose difference is unmistakable at a glance) |
| `ui.panel.layers.group` | — | — |
| `ui.panel.layers.group_tag` | — | — |
| `ui.panel.layers.mask_badge` | {count} | mask-count badge template (O-UI review D6: the "M" prefix was an uncataloged literal; the shell shows a bare numeral until it adopts this id — locales control any affix) |
| `ui.panel.layers.mask_from_selection` | — | v1.0.1 mask route: `mask.add{from:'selection'}`. Carries the `layer.addMaskFromSelection` availability row, NOT `layer.addMask`: the engine requires an active selection for this mode (error.command.selection_required) and serves the other three without one, so a single shared row would disable Reveal All whenever nothing was selected |
| `ui.panel.layers.mask_from_selection_inverted` | — | v1.0.1 mask route: `mask.add{from:'selection_inverted'}`. Same availability row and same reason as `.mask_from_selection` |
| `ui.panel.layers.mask_from_transparency` | — | v1.0.1 mask route: `mask.add{from:'transparency'}`. Needs no selection, so it rides the plain `layer.addMask` row |
| `ui.panel.layers.mask_hide_all` | — | v1.0.1 mask route: `mask.add{from:'hide_all'}` |
| `ui.panel.layers.mask_reveal_all` | — | v1.0.1 mask route: `mask.add{from:'reveal_all'}` — the contract's own default (`defaults: from: reveal_all`) |
| `ui.panel.layers.mask_tag` | {count} | A11Y-3 wave-B mint: mask-glyph accessible NAME carrying {count} (single-masked rows regain a textual signal for AT; the visual badge Label goes Accessible.ignored to avoid double-speak with this name) |
| `ui.panel.layers.merge_down` | — | — |
| `ui.panel.layers.milestone_tooltip` | {kind} {milestone} | — |
| `ui.panel.layers.opacity` | — | — |
| `ui.panel.layers.remove` | — | — |
| `ui.panel.layers.rename_field` | — | A11Y-3 wave-B mint: inline rename TextField accessible NAME (unlabeled interactive = §31.5 lint roster row 3) |
| `ui.panel.layers.select_toggle` | — | A11Y-3 wave-B mint: row multi-select CheckBox accessible NAME (unlabeled interactive = §31.5 lint item 1) |
| `ui.panel.layers.ungroup` | — | — |
| `ui.panel.layers.up` | — | glyph label (↑), locale-invariant |
| `ui.panel.layers.visibility_toggle` | — | A11Y-3 wave-B mint: layer eye accessible NAME (state rides Accessible.checked = visible; the long-queued P2-D1/D2 row) |
| `ui.panel.mask.density` | — | row 6 mask panel control label. DORMANT (cataloged, no emitter yet: B-12 MaskPanel). A-05, operator ruling 2026-09-05 |
| `ui.panel.mask.enabled` | — | row 6 mask panel control label. DORMANT (cataloged, no emitter yet: B-12 MaskPanel). A-05, operator ruling 2026-09-05 |
| `ui.panel.mask.feather` | — | row 6 mask panel control label; es/ja pair is identical to `ui.options.feather` by ruling. DORMANT (cataloged, no emitter yet: B-12 MaskPanel). A-05, operator ruling 2026-09-05 |
| `ui.panel.mask.inverted` | — | row 6 mask panel control label. DORMANT (cataloged, no emitter yet: B-12 MaskPanel). A-05, operator ruling 2026-09-05 |
| `ui.panel.mask.linked` | — | row 6 mask panel control label. DORMANT (cataloged, no emitter yet: B-12 MaskPanel). A-05, operator ruling 2026-09-05 |
| `ui.panel.mask.title` | — | row 6 mask panel heading. DORMANT (cataloged, no emitter yet: B-12 MaskPanel). A-05, operator ruling 2026-09-05 |
| `ui.pending.history_goto` | — | item-25 pending-reason batch: HistoryPanel dead-row reason (rows are ListItems with no click handler today; `history.goto` contract exists but is unwired). BARE — usable as a `reasonId` (the milestone_tooltip gap: a parameterized id cannot be a bare reasonId). Register: `error.history.nothing_to_*` "not wired yet" family. Consumer: HistoryPanel row disabled-reason at Wave F |
| `ui.pending.needs_engine_arm` | — | item-25 pending-reason batch: bare sibling of the milestone tooltip reason (`ui.panel.layers.milestone_tooltip`), usable as a `reasonId` where that parameterized id cannot be. Serves class-G rows (multi-layer wand/quick sampling, composite heal/clone, smudge sampleAllLayers) + class-N info-v2 disabled controls. Consumers land at O-UI's engine-arm wiring waves |
| `ui.pending.option_required` | {option} {tool} | item-25 pending-reason batch: class-D controls whose option isn't served yet (sides, corner_radius today; grows with 11d Phase 2). Parameterized like `milestone_tooltip` (direct-QML tooltip surface, not the bare reasonId path). `{tool}`/`{option}` identical in all four locales |
| `ui.pending.session` | {tool} | item-25 pending-reason batch: class-E session-shaped wording for warp + selection.transform (distinct from the stroke-shaped preview note). One `{tool}` template serves both; identical placeholder in all four locales |
| `ui.pending.transform_masked_layer` | {code} | v1.0.1 item 19+21: the free-transform frame on a MASKED layer. The 1.4 per-layer readback refuses masked layers typed (F3b class), so there are no layer bounds and no preview pixels — the frame opens CANVAS-shaped with no live preview. NOT a wall: the matrix applies as-is in doc space, so the fallback frame is loose, not wrong, and walling would claim "you cannot do this" about something the user can still do. `layerContentSnapshot` already emits `ui.status.rejected` naming the failed read; this REPLACES it with the consequence (true-and-materially-incomplete is the honest-state failure), carrying the same wire code in `{code}`. Self-disarming when the mask-readback ABI arm or the info-v4 per-layer `contentBounds` ask lands. Design entry's placeholder name was `ui.status.transform_frame_canvas_fallback` — moved into the `ui.pending.*` RESIDUAL family because that is the family `statusPending` serves; O-I18N may rename |
| `ui.pending.transform_preview_too_large` | — | v1.0.1 item 21: the layer exceeds the 2^26-px readback cap, so no single read delivers its pixels and no QImage holds them as one plane. Frame and handles stay honest; the preview degrades to frame-only rather than showing one capped strip — the same choice the ants overlay already makes at gigapixel sizes. General fix is the viewport-windowed snapshot (`m_antsWindow`'s shape), recorded not built |
| `ui.preset.a4` | — | v1.0.1 New Document/Resize size-preset name; paper size, LOCALE-INVARIANT (A4 in every locale) |
| `ui.preset.a5` | — | v1.0.1 preset name; paper size, LOCALE-INVARIANT |
| `ui.preset.custom` | — | v1.0.1 preset name; the free-entry option (translated) |
| `ui.preset.hd` | — | v1.0.1 preset name; HD 1080p (1920×1080), LOCALE-INVARIANT technical token |
| `ui.preset.letter` | — | v1.0.1 preset name; US Letter paper (translated: Carta/レター) |
| `ui.preset.square_social` | — | v1.0.1 preset name; square social-media canvas (translated) |
| `ui.preset.uhd4k` | — | v1.0.1 preset name; 4K UHD (3840×2160), LOCALE-INVARIANT technical token |
| `ui.profile.display_p3` | — | space-name-label question (OQ-I18N-3, ruling pending) |
| `ui.profile.gray_gamma_22` | — | — |
| `ui.profile.prophoto_rgb` | — | space-name-label question (OQ-I18N-3, ruling pending) |
| `ui.profile.sgray` | — | — |
| `ui.profile.srgb` | — | space-name-label question (OQ-I18N-3, ruling pending) |
| `ui.profile.untagged` | — | — |
| `ui.resample.bicubic` | — | family-constructed emitter (quoted prefix + value) |
| `ui.resample.bicubic_sharper` | — | family-constructed emitter (quoted prefix + value) |
| `ui.resample.bicubic_smoother` | — | family-constructed emitter (quoted prefix + value) |
| `ui.resample.bilinear` | — | family-constructed emitter (quoted prefix + value) |
| `ui.resample.lanczos` | — | family-constructed emitter (quoted prefix + value) |
| `ui.resample.nearest` | — | family-constructed emitter (quoted prefix + value) |
| `ui.status.applied` | {label} | — |
| `ui.status.applied_not_durable` | {label} | review-D2 durability warning: ERR_IO after a live-binding apply means applied-but-not-journaled |
| `ui.status.clone_needs_source` | `{tool}` | O-INT #8 Q-D14 VOICE (operator word, 2026-08-29): the clone chord `S` stays BOUND and SPEAKS its own missing precondition instead of arming silently — "a bound key that does nothing quietly is not an option". Surfaced through the existing `reasonId` door on the keymap chord funnel (`Main.qml` `dispatchToolChord`), so the toast names WHY clone paints nothing rather than leaving the operator to discover it at the demo. Distinct from `ui.status.tool_pending` in WORDING, not in wiring state. **CORRECTED 2026-09-01: this cell used to read “clone IS wired and is missing an INPUT” and that is FALSE of the shell, in the very frame `tool_pending` names.** `canvas_view.cpp` `resolveStrokeDispatch` finds clone in NEITHER `kModeTools` NOR the no-mode branch, so `dispatches=false` and the press falls through to `armedToolPressed` — refuted independently by `tst_palette_pins.cpp` (“clone.stroke is DELIBERATELY ABSENT — it does not dispatch”), by `tst_gesture_net.cpp` (“CLONE IS THE ONE STILL DEAD”, pinned by historyEntries and submissionAttempts) and by `Main.qml`’s options-row comment. Clone is implemented in the ENGINE (`registry_gen.rs`, `retouch.rs`) and UNWIRED in the SHELL; the original sentence switched frames mid-clause while the string it contrasted itself with names the canvas explicitly. What survives is the wording claim, and only that: this id states the missing INPUT. ⛔ **SUPERSEDED 2026-09-01 BY Q-D21 RULED (a): it NOW CARRIES a `{tool}` placeholder and is the sentence BOTH paths speak** — the keymap chord and the canvas press — so the two-sentence disagreement recorded at INTEGRATION-log 450 is closed by construction. The cell previously argued a template reuse would be wrong; the ruling elects this sentence as the template instead, because it says WHY and stays true once priority 5 wires clone. The source-point affordance itself rides the filed next-week UI row with Q-D1's Alt-click. ICU placeholders: `{tool}` (added 2026-09-01 per Q-D21; en/es/ja hand-authored, pseudo regenerated by the tool) |
| `ui.status.copied` | — | — |
| `ui.status.doc_size` | {height} {width} | — |
| `ui.status.exported` | {path} | — |
| `ui.status.opened` | {path} | — |
| `ui.status.pending_previews` | {count} (plural) | — |
| `ui.status.ready` | — | — |
| `ui.status.redone` | {label} | — |
| `ui.status.rejected` | {code} {message} | — |
| `ui.status.saved` | {path} | — |
| `ui.status.selection_active` | — | — |
| `ui.status.stroke_preview_pending` | {code} {kind} | — |
| `ui.status.tool_pending` | {tool} | item-25 pending-reason batch, THE HIGH-LEVERAGE ONE: one `{tool}` template converts all 18 ARMED-SILENT tool rows to ARMED-HONEST (a control that isn't wired says so) with zero engine work. Surfaced on the existing `ui.status.*` path on canvas press while an armed tool is active — sibling of `ui.status.stroke_preview_pending`. `{tool}` PRESERVED identical in all four locales (i18n-audit RED-4 placeholder parity) |
| `ui.status.transform_layer_empty` | — | v1.0.1 item 19: Cmd+T on a layer with NO opaque pixels. There is nothing to frame — a zero-rect box and a doc-rect box are each a lie — so no session opens, mirroring G-3's perspective ruling including its CODE-LESS wall (no wire door refused; the empty bounds ARE the door's answer). G-3 shipped that arm MUTE because no cataloged text existed and minting was outside its fence, and recorded the notice as a queued fold-in; this is that fold-in. A control that acts on every other layer and silently does nothing here is inert-without-explanation. `ui.status.*` (a factual state, not a pending-wiring statement) per the design entry's own candidate name |
| `ui.status.transform_wall` | — | item-13 status-line text when a transform hits a §11.1 lock — a lock-class WALL (not an NI); worded as a lock message |
| `ui.status.undone` | {label} | — |
| `ui.tool.blur` | — | tier-2 palette (O-UI 11c queue): blur/focus brush |
| `ui.tool.burn` | — | tier-2 palette (O-UI 11c queue): dodge/burn/sponge toning trio |
| `ui.tool.clone` | — | tier-2 palette (O-UI 11c queue): clone-stamp source→paint |
| `ui.tool.content_aware_fill` | — | tier-2 palette (O-UI 11c queue): content-aware fill affordance |
| `ui.tool.crop` | — | — |
| `ui.tool.dodge` | — | tier-2 palette (O-UI 11c queue): dodge/burn/sponge toning trio |
| `ui.tool.eraser` | — | — |
| `ui.tool.hand` | — | tier-2 palette (O-UI 11c queue): canvas pan |
| `ui.tool.heal` | — | tier-2 palette (O-UI 11c queue): healing brush |
| `ui.tool.lasso` | — | — |
| `ui.tool.move` | — | — |
| `ui.tool.paint_bucket` | — | tier-2 palette (O-UI 11c queue): flood fill |
| `ui.tool.pen` | — | — |
| `ui.tool.perspective` | — | toolbar affordance for the live canvas mode |
| `ui.tool.polygon` | — | — |
| `ui.tool.quick_select` | — | tier-2 palette (O-UI 11c queue): quick-selection brush |
| `ui.tool.select_ellipse` | — | — |
| `ui.tool.select_rect` | — | — |
| `ui.tool.shape_ellipse` | — | en/ja carry a Shape marker (disambiguates `ui.tool.select_ellipse`); es adjective pattern matches "Selección elíptica" |
| `ui.tool.shape_line` | — | tier-2 palette: line shape (bare "Line", unambiguous — `ui.tool.shape_rect` precedent) |
| `ui.tool.shape_polygon` | — | tier-2 palette: polygon shape; en/ja carry a Shape marker (disambiguates `ui.tool.polygon`, the polygonal-lasso selection — `ui.tool.shape_ellipse` precedent) |
| `ui.tool.shape_rect` | — | bare "Rectangle" is unambiguous (the select tool is "Rect Select") — asymmetry follows the request's en verbatim |
| `ui.tool.shape_rounded_rect` | — | tier-2 palette: rounded-rectangle shape (bare, unambiguous) |
| `ui.tool.sharpen` | — | tier-2 palette (O-UI 11c queue): sharpen brush (paired with `ui.tool.blur`) |
| `ui.tool.slice` | — | tier-2 palette: slice family mode 1 (multi-mode → row-per-mode, `ui.tool.select_rect`/`select_ellipse` precedent) |
| `ui.tool.slice_select` | — | tier-2 palette: slice family mode 2 (select/adjust existing slices) |
| `ui.tool.smudge` | — | tier-2 palette (O-UI 11c queue): smudge brush |
| `ui.tool.sponge` | — | tier-2 palette (O-UI 11c queue): dodge/burn/sponge toning trio (saturation) |
| `ui.tool.swatch_background` | — | tier-2 palette: fg/bg swatch-pair control NAME (matches `a11y.color.foreground` register — es "Color de fondo", ja "背景色") |
| `ui.tool.swatch_foreground` | — | tier-2 palette: fg/bg swatch-pair control NAME, parallels `a11y.color.foreground` ("Foreground color"/"Color frontal"/"前景色"); names the swatch CONTROL — distinct from the O-INT false-name ruling barring "foreground" for the CP-0.5 brush swatch |
| `ui.tool.swatch_swap` | — | tier-2 palette: the fg/bg swatch-SWAP control NAME (i18n-menu round; `ToolPalette.qml` "No swap button: its name is not cataloged" header record — the swap button holds on this id). Matches the `ui.tool.swatch_foreground`/`.swatch_background` family; "Swap Foreground and Background Colors"/"Intercambiar colores frontal y de fondo"/"前景色と背景色を入れ替え" |
| `ui.tool.text` | — | slice-T toolbar button (the filed design's named id) |
| `ui.tool.transform` | — | — |
| `ui.tool.transform_selection` | — | tier-2 palette: transform the selection marquee (distinct from `ui.tool.transform` = pixel transform) |
| `ui.tool.wand` | — | — |
| `ui.tool.warp` | — | toolbar affordance; matches history.transform.warp register |
| `ui.tool.zoom` | — | tier-2 palette: zoom tool (distinct from the `ui.action.zoom_*` command set) |
| `ui.trim.basis.bottom_right` | — | — |
| `ui.trim.basis.top_left` | — | — |
| `ui.trim.basis.transparent` | — | — |
| `ui.trim.bottom` | — | — |
| `ui.trim.left` | — | — |
| `ui.trim.right` | — | — |
| `ui.trim.top` | — | — |
| `ui.unit.cm` | — | v1.0.1 New Document/Resize unit name; registry alias candidate `unit.cm.abbrev` (registered) — swap is O-UI's call at adoption; LOCALE-INVARIANT symbol |
| `ui.unit.in` | — | v1.0.1 unit name (inches); no registry member names it (registry unit.* set lacks an inch abbrev) — mint; LOCALE-INVARIANT symbol |
| `ui.unit.mm` | — | v1.0.1 unit name; registry alias candidate `unit.millimeter.name` (registered) — swap is O-UI's call; LOCALE-INVARIANT symbol |
| `ui.unit.px` | — | v1.0.1 unit name; registry alias candidates `unit.px.abbrev`/`unit.pixel.short` (registered) — swap is O-UI's call; LOCALE-INVARIANT symbol |
| `ui.value.percent` | {value} | consolidation DONE (O-UI ABI-1.2 wave): sole percent template; `ui.status.zoom_percent` mint dropped 2026-08-13 (ledger) |
| `ui.warp.arc` | — | preset family ui.warp.<kind>, wire snake verbatim |
| `ui.warp.arc_lower` | — | preset family |
| `ui.warp.arc_upper` | — | preset family |
| `ui.warp.bulge` | — | preset family |
| `ui.warp.fish` | — | preset family |
| `ui.warp.flag` | — | preset family |
| `ui.warp.rise` | — | preset family |
| `ui.warp.shell` | — | preset family |
| `ui.warp.squeeze` | — | preset family |
| `ui.warp.twist` | — | preset family |
| `ui.warp.wave` | — | preset family |
| `ui.window.title` | {doc} | — |
| `ui.window.title_dirty` | {title} | P4 dirty-marker template — en "● {title}" (U+25CF); locales control glyph AND placement (the P4 ruling's hardcoded LTR prefix retires when the shell adopts this id); {title} is the already-localized `ui.window.title` output |

## Minted a11y ids (`a11y.` area)

The registry's `a11y.*` area is populated (28 members) and every a11y id
the build has surfaced so far was REGISTERED-and-cataloged — no mint. This
is the FIRST minted a11y id: the §11.1 transform-lock WALL has no
registered announcement member. Minted following the a11y announcement
convention (a full spoken sentence, cf. `a11y.canvas.selection_cleared` /
`a11y.file.saved`); it is the assistive-tech partner of the visual
`ui.status.transform_wall` status line (item 13). Verified absent from
`contracts/message_ids.yaml` first. Emitter lands at O-UI's item-13
transform-wall shell touch (`error.api.internal` staged→emitted
precedent).

CP-1.0 i18n-menu round (2026-08-20): SECOND minted a11y id,
`a11y.palette.scrollbar`. The tier-2 palette merge (main `c93ac4f`)
landed the scrolling viewport whose `ScrollBar.vertical`
(`shell/qml/ToolPalette.qml`, the "No string surface (ScrollBar
exposes…)" note) carries no accessible name yet — an a11y-list-growth
candidate (a scrollbar reachable by AT with no name). Minted following
the a11y accessible-NAME convention as a short control label (the
ScrollBar is a control, not an announcement — so no spoken sentence,
cf. the `ui.panel.layers.*` accessible names). Verified absent from
`contracts/message_ids.yaml` first (no `a11y.palette.*` member). Emitter
lands at O-UI's item-7 palette shell touch (`error.api.internal`
staged→emitted precedent).

CP-1.0 v1.0.1 item-14b round (2026-08-20): THIRD minted a11y id,
`a11y.layers.nudged`. Nudge (arrow-key integer-translate of the active
layer, `shell/src/canvas_view.cpp` — "no cataloged nudge id exists,
honest silence") is silent today; nothing cataloged fits. Minted into the
`a11y.layers.*` family (the nudge acts on the active layer — siblings
`a11y.layers.hidden`/`.moved`/`.renamed`), following the a11y announcement
convention (a spoken sentence, "Layer '{name}' moved …", the `.moved`
register + the `a11y.tool.brush_size` "… pixels" unit); it coalesces per
source like the brush-size precedent. Placeholders `{name}` (layer name),
`{dx}`/`{dy}` (the coalesced displacement in pixels) — identical in all
four locales. O-UI's log names no exact id (descriptive only); minted to
the `a11y.*` family pattern and recorded. Verified absent from
`contracts/message_ids.yaml` first (no `a11y.layers.nudged` member).
Emitter lands at O-UI's nudge-announcement shell touch
(`error.api.internal` staged→emitted precedent).

| Id | Args (`?` = optional) | Note |
|---|---|---|
| `a11y.layers.nudged` | `{name}`, `{dx}`, `{dy}` | 14b: active-layer arrow-key nudge announcement (silent today, `canvas_view.cpp`); `a11y.layers.*` family (`.moved` register), coalesces per source (brush-size precedent); `{dx}`/`{dy}` = coalesced pixel displacement |
| `a11y.palette.scrollbar` | — | tier-2 palette ScrollBar accessible NAME (i18n-menu round; the `ScrollBar.vertical` at `shell/qml/ToolPalette.qml` had no string surface) — "Tool palette scroll" |
| `a11y.transform.wall` | — | §11.1 transform-lock wall announcement; spoken partner of `ui.status.transform_wall` (item 13) |
| `ui.dialog.licenses.text_missing` | {name} | B-09 licences viewer: a staged text that reads ZERO BYTES. The dialog would otherwise render a blank pane, which its own comment calls "actively misleading" because it is near-identical to the `unavailable` state. A PERMANENT POSTURE, never a shipped state — the operator ruled it in WORD 8 of 2026-09-14 18:48 ET, answer (2), together with a FAILING assertion in `tst_license_texts` so the case cannot ship |

## Inherited proto seed family (not emitted by the build)

The 129 `proto.`-prefixed catalog keys are the round-1 mockup's minted
GUI ids, carried verbatim as translation seeds for the CP-0.3 shell port
(same authorship; per-id upstream suggestions live in the proto's
PROTO-IDS.md). They are catalog-only: nothing in the build emits them,
and the audit's emitted-id sweep would FAIL if one ever were emitted —
the family rule below applies to catalog keys, not emissions. As O-UI
surfaces land, each seed is replaced by its registered id (or minted here
properly) and the `proto.` copy is dropped; the family is expected to be
empty before CP-1.0's catalog completion sweep.

Audit family rule: catalog keys starting `proto.` are bucketed
"proto seed", reported by count, never red. The `proto.lang.*` endonyms
additionally pass through `i18n/tools/pseudo.py` unaccented (the Language
menu must stay navigable in every locale).

## Ledger notes

- 2026-08-12 (activation slice): file created; 6 wire-code + 16 CLI
  mints recorded from the mechanical census (`i18n/tools/audit.py`);
  the two named by the activation brief (`error.command.not_implemented`,
  `error.api.internal`) plus four more the census surfaced
  (`error.api.invalid_arg`, `error.schema.wrong_schema`,
  `error.history.nothing_to_undo`, `error.history.nothing_to_redo`).
- 2026-08-12 (review minor 2): `error.api.internal` STAGED -> EMITTED —
  `lane/script` merged to main while this lane was in flight; no new
  mint, status refresh only (post-merge audit: 29 emitted, staged 1).
- 2026-08-12 (stroke-wave tripwire): the audit fired on merged main
  (RED-1/RED-2) for three new emissions. `error.command.locked` and
  `error.unsupported.erase_to_history` are REGISTRY ids (rows 775/1005)
  — cataloged, no mint. `cli.asset.add.ok` minted above (build mint,
  `asset add` verb). Catalogs 193 -> 196 ids per locale.
- 2026-08-13 (chrome-wave fold-in): 180 `ui.*` shell ids minted (table
  above) from the chrome wave's seed register + the wave-2/3 O-UI log
  register; 13 seed canonical-register ids (12 `history.<kind>` +
  `panel.layers.empty`) cataloged — registry rows, no mints; 20 further
  registered `history.<kind>` labels for shipped (≤0.5) kinds cataloged
  (audit's new shipped-kind history check surfaced them — same echo
  class the chrome wave hit); 2 dead seed keys not folded;
  `ui.error.no_document` sweep-found and minted. Catalogs 226 -> 439
  ids per locale.
- 2026-08-13 (CP-0.7 W05/About/blend/export coordination round): 47
  mints STAGED pre-coverage from the O-UI ABI-1.2 READY handoff queue
  (`error.api.internal` staged->emitted precedent) — 4
  `dialog.recovery.*` family members (new section above) + 43 `ui.*`
  (26 blend names, 7 About, 8 file filters, 2 statuses); 12 REGISTERED
  ids cataloged, no mint (the 11 `dialog.recovery.*` registered members
  unblocking the W05 prompt + `dialog.export.est_size`). Catalogs
  439 -> 498 ids per locale; registry coverage 105 -> 117/419. Mint
  total 205 -> 252.
- 2026-08-13 (post-ui-P-round consolidated-queue close): 2 mints STAGED
  (`ui.window.title_dirty` — the P4 dirty-marker ruling's queued
  template; `ui.panel.layers.mask_badge` — the review-D6 badge
  template); 1 REGISTERED id cataloged, no mint
  (`tool.crop.option.delete_cropped` — unblocks O-UI's deferred alias
  swap off `ui.dialog.crop.delete_pixels`); 1 mint DROPPED
  (`ui.status.zoom_percent` — never emitted; O-UI consolidated to
  `ui.value.percent` in the ABI-1.2 wave and invited the drop); 2
  cross-fence registered-id catalog rows RATIFIED as-committed
  (`error.batch.step_failed`, O-SCRIPT at bb45492;
  `error.schema.unknown_op_kind`, O-CORE at 7c027d4 — es matches the
  `tipo de comando` pattern, ja `操作の種類` matches the
  `target_kind_mismatch` precedent; no rewording). Catalogs 500 -> 502
  audited ids per locale; registry coverage 119 -> 120/419. Mint total
  252 -> 253.
- 2026-08-16 (CP-1.0 slice-V vector-tools mint round): 9 `ui.*` mints
  STAGED pre-coverage from the O-UI slice-V mint request
  (`reports/O-UI-log.md` @ fb0195f, read first-hand; same-wave audit
  rule — this companion merges before `lane/ui` so the shell's new
  emitters land on covered catalogs). 3 tool names + 4 options labels
  + 1 action + 1 disabled-NI menu carrier (rows above). 0 registered
  ids cataloged (`tool.path_select.hint` surveyed, left uncataloged —
  no slice-V path-select surface). Catalogs 502 -> 511 audited ids per
  locale; registry coverage unchanged 120/419. Mint total 253 -> 262;
  staged 53 -> 62.
- 2026-08-17 (CP-1.0 backlog round, Part 2a — color-pipeline family):
  7 `color.*` mints (new section above) from O-COLOR's cross-fence
  flag (their C5 print/proof report, "7 spec-named ids absent from the
  420-id v5.5 catalog") — spec-named in ES §6.2/§5.5 normative text,
  verified absent from the registry mechanically. Emitters were
  ALREADY LIVE on main (`appname-color/src/mode_convert.rs` warn::*),
  invisible to the sweep only because `color` was not a sweep area —
  the audit gained the `color` area + the `NON_MESSAGE_PARAM_PATHS`
  exclusion (`color.comps`/`color.model`, `EngineError::missing`
  params paths) in the same commit, so all seven flipped straight to
  emitted/covered (no dishonest staging). en for
  `color.convert.cmyk_fallback_warning` VERBATIM from the §5.5
  [NORMATIVE] sentence pair. Catalogs 521 -> 528 ids per locale;
  registry coverage unchanged 130/419. Mint total 262 -> 269; staged
  unchanged 60.
- 2026-08-17 (CP-1.0 backlog round, Part 2c — About-credits pair): 2
  `ui.*` mints STAGED (`ui.dialog.about.credit.lucide_icons`,
  `ui.dialog.about.credit.lensfun_database`) — the only genuine gaps
  left in O-UI's consolidated queue (slice-V 9, dirty-title, export
  statuses, file filters, quality-dial surface, 27-blend family, mask
  badge, `dialog.recovery.*` dormant arms, About family all verified
  already cataloged this round, id-by-id vs `contracts/message_ids.yaml`
  + the catalogs — nothing re-minted). Both credit rows are mandated by
  their deps records' noticeSurfaces (deps/lucide.yaml: "ISC AND MIT,
  both copyright lines"; deps/lensfun-db.yaml: credits-half of the
  CC-BY-SA-3.0 attribution obligation, §4(c) elements carried: title,
  contributors line, license notice, URI). Values LOCALE-INVARIANT
  (`ui.profile.*` precedent) — attribution content, not prose; the
  acknowledgments-row note now records why these two are id-backed
  while the FTL/rgb2spec/libmypaint lines stay non-catalog verbatim
  content. Emitter = O-UI's About dialog (held for this batch per
  their log's routing ack item 4); staged->emitted flips at adoption.
  Catalogs 528 -> 530 ids per locale; registry coverage unchanged
  130/419. Mint total 269 -> 271; staged 60 -> 62.
- 2026-08-18 (CP-1.0 chrome-vocabulary mint rotation): 49 `ui.*` mints
  STAGED (round paragraph above the shell-UI table; per-id notes in the
  table rows) + 28 REGISTERED rows cataloged x4 with no mints
  (`menu.edit.puppet_warp`, `menu.filter.last`, `a11y.layers.hidden`/
  `.moved`/`.renamed` with the §49.1/§31.3 golden wordings, and 23
  `history.<kind>` labels for the CP-1.0 kinds whose doors the O-UI
  census probed live — channel x6, slice x3, text x5, smartobject x4,
  `doc.mode_convert`, `meta.set`, `selection.save`/`.load`,
  `transform.puppet` — closing the frame-17 key-echo class O-UI
  reported). Every queue item verified against the registry FIRST: the
  8 filter display names were already registry-cataloged, intent names
  reuse `ui.intent.*`, fade modes reuse `ui.blend.*` — nothing
  re-minted. Cross-fence ratification recorded in the lane log:
  O-TOOLS' in-gate `filter.liquify` catalog rows (f3e6bb8, flagged)
  ratified as-committed. Catalogs 531 -> 608 keys per locale; registry
  coverage 131 -> 159/419. Mint total 271 -> 320; staged 62 -> 110 (all
  49 new mints staged; `ui.panel.layers.mask_badge` flipped
  staged->emitted at O-UI's Wave A adoption merge, audit-verified).
- 2026-08-19 (CP-1.0 A11Y-3 a11y catalog/mint round — RE-DISPATCH, the
  a11y catalog/mint of O-UI's 17-row A11Y-3 queue, started fresh after
  the sleep-killed run left nothing committed): 6 `ui.*` mints STAGED
  (round paragraph above the shell-UI table; per-row notes in the table)
  — `ui.dialog.canvas_size.anchor_cell`, `ui.dialog.fill.color_swatch`
  (the ROW-17 fill-swatch DISTINCT-id decision), `ui.panel.layers.
  mask_tag`/`.rename_field`/`.select_toggle`/`.visibility_toggle`. 11
  REGISTERED `a11y.*` rows cataloged x4 with NO mints (verified against
  `contracts/message_ids.yaml` first; golden wordings verified first-hand
  against ES §31.3/§49.1/§49.2/§49.6 — see the round paragraph). 0
  registered ids left uncataloged from the queue; 2 registered a11y ids
  deliberately NOT cataloged for lack of any emitting surface
  (`a11y.text.editing_started`, `a11y.adjustments.curve_point` — O-UI's
  recorded "NO surface" gaps). Audit-measured after the round: catalogs
  611 -> 628 keys per locale; registry coverage 162 -> 173/419 (+11
  a11y); mint total 320 -> 326; minted-not-yet-emitted (staged/dormant)
  97, including all 6 new mints (emitters land at A11Y-3 wave B).
- 2026-08-19 (CP-1.0 v1.0.1 New-Document presets/units mint round): 14
  `ui.*` mints STAGED (round paragraph above the shell-UI table; per-row
  notes in the table) from O-UI's New Document dialog feedback items 2+3
  — `ui.dialog.new.preset`/`.dpi`/`.computed_px`, the 7-member
  `ui.preset.*` family, the 4-member `ui.unit.*` family. Resize reuses
  the same rows (no Resize-specific duplicates). `computed_px` template
  `{width} × {height} px` follows the cataloged `ui.status.doc_size`
  convention (ICU `{name}`, × = U+00D7). Every id checked against
  `contracts/message_ids.yaml` first: the partial registry `unit.*` set
  (`unit.px.abbrev`, `unit.cm.abbrev`, `unit.millimeter.name`,
  `unit.pixel.short`) named as alias candidates on the px/cm/mm rows for
  O-UI's swap; no registry inch abbrev, so `ui.unit.in` is a clean mint;
  `preset`/`dpi` labels have no registry member. 0 registered ids
  cataloged, 0 re-minted. Audit-measured after the round: catalogs
  632 -> 646 keys per locale; registry coverage unchanged 176/419; mint
  total 327 -> 341; minted-not-yet-emitted (staged/dormant) 104,
  including all 14 new mints (emitters land at O-UI's New Document +
  Resize dialog binding).
- 2026-08-19 (CP-1.0 tier-2 tool-palette + item-13 transform-wall mint
  round): 24 mints STAGED unblocking O-UI's 11c tool-palette tier-2 queue
  + item-13 transform-wall work (round paragraph above the shell-UI table
  + new `a11y.` section; per-row notes in the tables) — 23 `ui.*` (21
  `ui.tool.*` tier-2 palette names, `ui.menu.select.color_range`,
  `ui.status.transform_wall`) + 1 `a11y.*` (`a11y.transform.wall`, the
  first minted a11y id). Structure calls: color-range keyed under the
  Select MENU per O-INT's ruling nuance (not `ui.tool.color_range`); the
  slice family takes two rows (`slice` + `slice_select`, row-per-mode
  precedent); the fg/bg swatch pair is
  `ui.tool.swatch_foreground`/`.swatch_background` matching the
  `a11y.color.foreground` register; `ui.tool.shape_polygon` carries a
  Shape marker (disambiguates `ui.tool.polygon`). Tool names are real
  es/ja translations; `ui.status.transform_wall`/`a11y.transform.wall`
  are worded as §11.1 lock messages (lock-class wall, NOT an NI). Every
  id verified absent from `contracts/message_ids.yaml` first; 0
  registered ids cataloged, 0 re-minted. Audit-measured after the round:
  catalogs 646 -> 670 keys per locale; registry coverage unchanged
  176/419; mint total 341 -> 365; minted-not-yet-emitted (staged/dormant)
  104 -> 128, including all 24 new mints (emitters land at O-UI's tier-2
  + item-13 shell touch).
- 2026-08-20 (CP-1.0 i18n-menu round): 4 mints STAGED unblocking O-UI's
  item-7 menu + tier-2 palette work (the zero-invented-ids gate excludes
  rows until ids exist) — 3 `ui.*` (`ui.menu.edit.paste_into`,
  `ui.menu.edit.delete`, `ui.tool.swatch_swap`) + 1 `a11y.*`
  (`a11y.palette.scrollbar`, the SECOND minted a11y id). Structure calls:
  `paste_into`/`delete` are MENU-item names under the existing
  `ui.menu.edit.*` family (distinct from the registered
  `history.edit.paste_into` label + the `op.delete`/`history.*` kinds —
  §27.1 one-id-per-meaning); the swap control joins the tier-2
  `ui.tool.swatch_foreground`/`.swatch_background` family; the ScrollBar
  name is an `a11y.palette.*` accessible NAME (first member). Per O-INT's
  naming ruling, NO Grow/Shrink minted — selection-modify stays the
  cataloged Expand/Contract (Select > Grow is a DIFFERENT command). Menu
  items carry real Photoshop es/ja (Paste Into = Pegar dentro / 選択範囲内へ
  ペースト; Delete = Eliminar / 削除); swap/scrollbar es/ja follow the
  swatch + a11y register. Every id verified absent from
  `contracts/message_ids.yaml` first; 0 registered ids cataloged, 0
  re-minted. Audit-measured after the round: catalogs 670 -> 674 keys per
  locale; registry coverage unchanged 176/419; mint total 365 -> 369;
  minted-not-yet-emitted (staged/dormant) 98, incl. all 4 new mints
  (emitters land as O-UI's item-7 thin follow-up).
- 2026-08-20 (CP-1.0 v1.0.1 item-11d Phase-2 options-bar mint round): 13
  `ui.*` mints STAGED unblocking O-UI's 11d options-bar Phase-2 slice (the
  mint-gated batch of the desk-reviewed 11d design entry, `reports/O-UI-log.md`
  @ 739d846 §4; the zero-invented-ids gate holds the rows out until ids
  exist) — the selection-op segmented control `ui.options.selection_op`
  (label) + its 4 value ids `.replace`/`.add`/`.subtract`/`.intersect`,
  plus `ui.options.antialias`/`.feather`/`.sample_all_layers`/`.opacity`/
  `.flow`/`.blend`/`.resample`/`.scale_effects` (round paragraph above the
  shell-UI table; per-row notes in the table). Wordings from the
  `proto.options.*` seed family (replace-and-drop) + the project's Fill/
  Resize/layers register: `.replace`/`.add`/`.subtract` reuse
  `proto.options.mode.*` verbatim; `.intersect` is the ONE genuinely new
  value mint (交差); `antialias`/`opacity`/`blend`/`resample`/`scale_effects`
  reuse cataloged dialog rows verbatim; `feather` from `proto.options.feather`
  (colon dropped); `flow`/`sample_all_layers` are clean mints. The blend
  and resample COMBOS reuse the existing `ui.blend.*` (27) and `ui.resample.*`
  values — only their two LABELS minted, ZERO value re-mints. Every id
  verified absent from `contracts/message_ids.yaml` (no `ui.` area in the
  frozen v5.5 registry) AND all four catalogs first; 0 registered ids
  cataloged, 0 re-minted; no ICU placeholders in the batch. Audit-measured
  after the round: catalogs 674 -> 687 keys per locale; registry coverage
  unchanged 176/419; mint total 369 -> 382; minted-not-yet-emitted
  (staged/dormant) 98 -> 111, incl. all 13 new mints (emitters land at
  O-UI's 11d Phase-2 shell touch).
- 2026-08-20 (CP-1.0 v1.0.1 item-14b mint-queue additions): 3 ids minted
  STAGED unblocking O-UI's 14b consuming slices (`reports/O-UI-log.md` @
  6270a45, the "MINT-QUEUE/CARRIES from the build" line) — 2 `ui.*`
  (`ui.dialog.delete_confirm.message` the §32.5 named-object destructive-
  confirm TEMPLATE; `ui.error.nothing_to_reselect` the reselect disabled-
  tooltip reasonId, which also renames the unreachable belt arm's reason,
  14b reviewer minor-2) + 1 `a11y.*` (`a11y.layers.nudged`, the THIRD
  minted a11y id — the silent-today nudge announcement). Structure calls:
  the delete-confirm template carries the `{name}` ICU placeholder,
  PRESERVED identical in all four locales (RED-4 parity), register from
  `dialog.recovery.discard_confirm` + Delete verb `ui.menu.edit.delete`;
  `nothing_to_reselect` minted in the shell disabled-reason namespace
  `ui.error.*` (precedent `ui.error.no_document`; reasonId contract takes a
  bare cataloged id) with the `error.history.nothing_to_undo` "Nothing to …"
  register; `a11y.layers.nudged` joins the `a11y.layers.*` family (`.moved`
  register, coalesces per source like brush-size) with `{name}`/`{dx}`/`{dy}`
  identical across locales. O-UI named an exact id only for #1; #2 and #3
  minted to the established pattern and recorded. Every id verified absent
  from `contracts/message_ids.yaml` (no `ui.` area; no `a11y.layers.nudged`)
  AND all four catalogs AND this file first; 0 registered ids cataloged, 0
  re-minted; `bogos/` and `contracts/message_ids.yaml` UNTOUCHED.
  Audit-measured after the round: catalogs 687 -> 690 keys per locale;
  registry coverage unchanged 176/419; mint total 382 -> 385;
  minted-not-yet-emitted (staged/dormant) 110 -> 113, incl. all 3 new mints
  (emitters land at O-UI's 14b consuming slices).
- 2026-08-20 (CP-1.0 v1.0.1 puppet-density value mint round): 3 `ui.*`
  mints STAGED unblocking the §45.5 puppet density combo, RESERVED until
  now because its enum VALUES were never minted (only the label
  `ui.options.puppet_density` was cataloged, 2026-08-18) — the
  fewer/normal/more mesh-density scale as siblings under the label id
  (`ui.options.puppet_density.fewer`/`.normal`/`.more`), matching the 11d
  `ui.options.selection_op.*` value-id convention (round paragraph above
  the shell-UI table; per-row notes in the table). Wordings in the
  short-single-word value register (Fewer/Normal/More · Menos/Normal/Más ·
  少ない/標準/多い — Normal = 標準 the scale midpoint, NOT the blend 通常;
  §27.1). No ICU placeholders. Every id verified absent from
  `contracts/message_ids.yaml` (no `ui.` area in the frozen v5.5 registry)
  AND all four catalogs AND this file first; 0 registered ids cataloged, 0
  re-minted; `bogos/` and `contracts/message_ids.yaml` UNTOUCHED.
  Audit-measured after the round: catalogs 690 -> 693 keys per locale;
  registry coverage unchanged 176/419; mint total 385 -> 388;
  minted-not-yet-emitted (staged/dormant) 113 -> 116, incl. all 3 new mints
  (emitters land at O-UI's puppet options-bar shell touch).
- 2026-08-20 (CP-1.0 v1.0.1 item-25 census pending-reason mint round): 5
  `ui.*` mints STAGED unblocking O-UI's item-25 census "do-nothing controls"
  honesty conversion — the 18 ARMED-SILENT tool rows become ARMED-HONEST
  (round paragraph above the shell-UI table; per-row notes in the table) —
  1 `ui.status.*` (`ui.status.tool_pending`, THE HIGH-LEVERAGE ONE: one
  `{tool}` template converts all 18 rows, sibling of
  `ui.status.stroke_preview_pending`) + 4 `ui.pending.*` (a new not-yet-wired
  reason sub-context: `.needs_engine_arm` BARE reasonId sibling of the
  `ui.panel.layers.milestone_tooltip` gap, `.option_required` {tool}/{option}
  class-D control, `.session` {tool} warp/selection.transform, `.history_goto`
  BARE HistoryPanel dead-row reason). Structure calls: the bare-vs-parameterized
  split honors the `document_controller.cpp` reasonId contract (bare ids for
  the reasonId path, parameterized for the status/direct-tooltip surfaces);
  `{tool}` on #1 PRESERVED identical in all four locales (RED-4 parity), as is
  `{tool}`/`{option}` on #3 and `{tool}` on #4; reason wordings follow the
  `milestone_tooltip` + `error.unsupported.*` "not wired / not yet supported"
  register with real es/ja translations (canvas = lienzo/キャンバス, history =
  historial/履歴, options bar = barra de opciones/オプションバー). O-UI's census
  names no exact ids (descriptive working labels only) — all 5 minted to the
  established `ui.` conventions and recorded. Every id verified absent from
  `contracts/message_ids.yaml` (no `ui.` area in the frozen v5.5 registry) AND
  all four catalogs AND this file first; 0 registered ids cataloged, 0
  re-minted; `bogos/` and `contracts/message_ids.yaml` UNTOUCHED.
  Audit-measured after the round: catalogs 693 -> 698 keys per locale;
  registry coverage unchanged 176/419; mint total 388 -> 393;
  minted-not-yet-emitted (staged/dormant) 116 -> 121, incl. all 5 new mints
  (emitters land at O-UI's item-25 wiring waves).
- 2026-08-20 (CP-1.0 v1.0.1 item-4 brush-hardness mint round): 1 `ui.*`
  mint STAGED unblocking O-UI's item-4 hardness slider (the shell half of
  FI-9; O-INT dispatch, carriage-confirmed in `reports/O-UI-log.md`) —
  `ui.options.brush_hardness`, the options-bar label for the now-SERVED
  `brush.stroke.hardness` param (FI-9 closed end-to-end: bogos b0f309e →
  mirror 811965d → registry eb015e8 → engine ff96a54; the standing
  hardness DO-NOT-MINT reservation lifted). Wording from the
  `proto.options.hardness` seed (replace-and-drop), colon dropped per the
  `ui.options.*` convention: Hardness / Dureza / 硬さ; id takes the
  `brush_` prefix per its bar sibling `ui.options.brush_size`. A11Y
  SIBLING surveyed, NOT minted (`a11y.tool.brush_hardness`) — decision
  recorded in the round paragraph: the spec's Brush A11y row names size
  chord announcements only ("(new id)" markers name none for `{`/`}`),
  the labeled bar control's own accessible value is the §31.1 coverage
  (opacity/flow/rigidity precedent), and no chord surface exists in the
  shell yet (11e queued) — a one-id mint-queue follow-up if 11e proves
  the chord path silent. No ICU placeholders. The id verified absent
  from `contracts/message_ids.yaml` (no `ui.` area; no hardness member)
  AND all four catalogs AND this file first; 0 registered ids cataloged,
  0 re-minted; `bogos/` and `contracts/message_ids.yaml` UNTOUCHED.
  Audit-measured after the round: catalogs 698 -> 699 keys per locale;
  registry coverage unchanged 176/419; mint total 393 -> 394;
  minted-not-yet-emitted (staged/dormant) 114 -> 115 incl. the new mint
  (the counts moved off the item-25 note's 121: the intervening 11d
  Phase-1 adoption merge, main 810c143, flipped the Phase-1/Phase-2
  options-bar and tier-2 emitter batches staged -> emitted —
  audit-verified, not this round's doing). Emitter lands at O-UI's
  item-4 hardness-row shell touch, has()-gated.

- 2026-08-30 (O-INT #8 Q-D14 VOICE mint round): 1 `ui.*` mint, EMITTED IN
  THE SAME COMMIT rather than staged — `ui.status.clone_needs_source`, the
  clone chord's spoken precondition. Operator word 2026-08-29 on Q-D14:
  "S stays bound and SPEAKS: the toast names why clone paints nothing
  ('clone needs a source point — not yet available') through the reasonId
  door. Silent tools are the disease this week cured; a bound key that does
  nothing quietly is not an option." Emitter is the keymap chord funnel
  `dispatchToolChord` in `shell/qml/Main.qml`, pinned by
  `cloneChordSpeaksItsMissingSource()` in `shell/tests/tst_chord_table.cpp`
  with a vacuity arm (B must arm brush and stay SILENT). That test's
  discriminator was rewritten before landing: the first version asserted a
  STATE on the status property, which is a last-event log, so it read the
  previous event and would have passed against a mute chord — G-537, the
  claim is about an EVENT, so it is settled by `QSignalSpy` on
  `statusTextChanged`. NOT a `ui.status.tool_pending` reuse, and the reuse
  was the plausible wrong answer: `tool_pending` says a tool is NOT WIRED,
  while clone IS wired and is missing an INPUT. The source-point affordance
  rides the filed next-week UI row with Q-D1's Alt-click. No ICU
  placeholders. The id verified absent from `contracts/message_ids.yaml`
  AND this file before minting; `bogos/` and `contracts/message_ids.yaml`
  UNTOUCHED. Audit-measured after the round: mint total 433 -> 434;
  catalogs 757 keys per locale (unmoved by this round — the four catalog
  values landed with the emitter); registry coverage unchanged 193/419;
  minted-not-yet-emitted (staged/dormant) 73 -> 73, UNCHANGED, and that
  is the discriminator proving this mint is emitted rather than staged.
  ROUND CAUSE, RECORDED BECAUSE IT IS THE FINDING AND NOT THE FIX: the
  catalog values were added and the MINTED row was not, so the gate went
  red on RED-1 and RED-5 while my written prediction said "i18n-audit
  PASS — 4 locales each gained the same one key, 756 -> 757". That
  prediction checked SYMMETRY (four catalogs agree with each other) where
  the audit checks CONFORMANCE (each key is logged, and pseudo matches its
  generator). Four agreeing copies are not a registration. RED-6 was the
  same shape one layer down: I hand-wrote the pseudo value, and it was
  the PLAUSIBLE wrong answer, not an absurd one — accented and bracketed
  and eye-correct, with `l`/`p`/`b` left bare where the table maps them to
  the accented forms and a 5-character expansion run where the generator
  emits 11. `pseudo.py --check` names it in one line; reading it does not.
