<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (c) 2026 the Appname contributors -->

# Scandium 1.0 — Known Issues

**Opened 2026-09-06 on the operator's word.** Every entry below is a defect or limitation that
is **known, reproduced and deliberately shipped** rather than discovered late. Each carries what
you will see, why it happens, and the workaround where one exists.

This file ships in both release artifacts. It is **not complete until freeze** — task **H-03**
(2026-09-23) adds every remaining CP-1.0 deferral, every MINOR, every Windows skip, and every
applied cut line. Entries are recorded on the operator's rulings; no entry is added by a desk on
its own authority.

---

## 1 — Documents in Grayscale or Lab mode cannot be exported

**What you will see.** With a document whose colour mode is **`gray_a`** or **`lab_a`**, every
export fails with:

    error.command.not_implemented
    composite readback of non-rgba document modes needs the §5/§6 conversion
    pipeline arm (mode-convert door C5 covers documents converted back to rgba)

**Why.** Export encodes from a single composited plane, and the composite path is RGBA-only: the
refusal happens **before** a file format is chosen, so it is not specific to any one format —
`png`, `jpeg`, `tiff`, `webp`, `gif`, `bmp` and `ora` all reach it identically. (`psd` is
import-only and answers `error.codec.write_unsupported` for a separate reason.)

**In 1.0 the app has no menu path into these modes.** Image ▸ Mode… is removed (corrigendum (l)):
File ▸ New makes RGB documents and opened files are read as RGB, so this issue is reached only
through the scripting door (`doc.mode_convert`).

**Workaround — this works today.** Convert the document back to RGB before exporting:
`doc.mode_convert` to `rgba` (the **C5** door). Gray → RGB is lossless. Lab → RGB clips to gamut
per the rendering intent and reports that as a warning before it runs.

**Status.** Ruled a typed refusal for 1.0 (roadmap A-12, §7 cut line 6). It is a refusal, not a
silent failure: nothing is written and nothing is lost.

---

## 2 — `vector.shape_add`'s documented dirty rectangle understates what it repaints

**What you will see.** Nothing, in normal use. This is a **specification wording defect**, not an
engine defect — it matters if you are reading the contracts or writing against them.

**Why.** `contracts/commands/vector.shape_add.yaml:163` reads `dirtyRect: "D = shape bounds"`. The
engine of record **inflates** that rectangle by the stroke width, which is correct: a stroke's ink
lies outside the geometric bounds (a mitred join can reach up to 2w beyond a vertex), and §40.7
already prescribes stroke-width inflation for `vector.shape_set_style`. The behaviour is right and
the sentence beside it is stale.

**Status.** Filed as **SD-v5.7-04** in the v5.7 fold-in register and as roadmap **C-04**. The 1.0
constitution is frozen, so the wording is corrected in v5.7 rather than now; it is documented here
instead of being carried silently.

---

## 3 — One UI test is intermittent (`resizePreviewMatchesEngine`)

**What you will see.** Nothing in the application. This affects the test suite only.

**Why.** `tst_review_dialog_probes::resizePreviewMatchesEngine` fails intermittently — roughly one
run in three — under full-suite load, and passes when run alone. It is tracked as a quarantine row
(`reports/QUARANTINE.md`, QROW, filed 2026-08-27) and owned by the shell desk.

**Status.** Quarantined and tracked, not ignored. An unclassified quarantine row blocks the next
release tag, so this is resolved or reclassified before any tag is cut.

---

## 4 — No instrument enumerates individual test slots

**What you will see.** Nothing in the application. This affects the test suite's own coverage
accounting only.

**Why.** The gate counts test *binaries* and *suites* — `ctest` appears 36 times in
`scripts/gate.sh` — but nothing reads the `private slots:` declarations inside them. `slot`,
`private slots` and `Q_SLOT` each occur **0** times in `scripts/gate.sh`, and **no** script in
`scripts/` reads them, while **45** files under `shell/tests/` declare them. A suite that builds,
registers and runs green therefore proves that its binary ran; it does not prove *which* of its
slots ran. A slot that is removed, renamed, or never added is invisible to every gate step.

**Status.** Filed for **1.1** on the operator's word of 2026-09-09; not a 1.0 fix. The gate's
existing arms (`registration`, `suites-fresh`, `suites-ran`) close the binary-level form of this
gap — no suite can be built-but-unregistered, stale, or enumerated-but-not-run. The slot-level
form needs a new instrument and is out of scope for the release.

---

## 5 — Layer styles are not in 1.0

**What you will see.** No layer-style controls anywhere in the application: no drop shadow,
stroke, inner glow, colour overlay or any other per-layer effect, and no menu or panel entry
that offers them. A document created elsewhere that carries layer styles opens without them;
its other content is unaffected.

**Why.** The feature was scoped for 1.0 as Tier-1 row 2 and **cut to 1.1** by the operator's
amendment of 2026-09-06 (ROADMAP-1.0 Amendment 5, applying §7 cut line 2). The engine and UI
halves exist on their lanes and are not merged; nothing partial ships.

**Workaround.** None in 1.0. Flatten or rasterise the effect in the originating application
before opening the file here, and it will render as ordinary pixels.

**Status.** Planned for **1.1**. Cut deliberately with the release date held, not dropped.

---

## 6 — The gradient tool is not in 1.0

**What you will see.** No gradient tool in the tool palette and no gradient entry in the fill
controls. Solid fills, patterns and the fill-layer path are unaffected and work normally.

**Why.** Scoped as Tier-1 row 3 and **cut to 1.1** by the same amendment (Amendment 5, §7 cut
line 1). The row needed a contract door that depended on a ceremony which was not held; the
fill-layer half it shares had already shipped, so the visible result is a fill path without a
gradient mode rather than a missing fill path.

**Workaround.** Build a gradient outside the application and place it as a raster layer, or
use a fill layer with a prepared image.

**Status.** Planned for **1.1**. Cut deliberately with the release date held, not dropped.

## 7 — The internal project name remains `appname`

The product is named **Scandium**. The *internal* name does not change for 1.0: crate and
library names (`appname-core`, `libappname_cabi`), contract ids, i18n key names, file paths,
the repository name, file-format identifiers and extensions, and the copyright phrase in
every source header all continue to read `appname`.

This is deliberate. A rename of the internal name before freeze would touch every file
header and every hash in the tree, and the identifiers are not user-facing: nothing in the
application's interface, documents, or exported files shows them.

One place shows it: with no document open the window title reads "Appname". With a document open,
the title names the document.

**Status.** Housekeeping for **1.1**. In 1.0 the only user-visible trace is that window title.

## 8 — Smart objects are not in 1.0

**What you will see.** No command anywhere converts a layer to a smart object: not in the Layer,
Image, Filter or Edit menus, not in the Layers panel, and not in Help search.

**Why.** Earlier text here said unmasked layers convert and only masked layers are refused.
Sitting #1 on RC2 found no conversion path at all in the 1.0 shell (`reports/SITTING-1.0.md`,
step S-18), and the operator ruled it a scope item (WORD 48 (3)).

**Workaround.** None in 1.0. Keep a copy of the source layer if you need to re-edit it
non-destructively.

**Status.** Planned after 1.0; no date set.

## 9 — The scripting example for adding a Curves adjustment does not run

The command reference publishes this example for `adjustment.add` with `opKind:
"adjustment.curves"`:

    "params": { "channels": { "master": { "points": [[0, 0], [1, 1]] } } }

Running it is refused with `unknown key "channels"`. The refusal is correct: an
adjustment's own parameters are validated against that adjustment's registered schema,
and the Curves schema takes one entry per channel, each a list of points. The example
adds two wrappers the schema does not have.

**The form that runs** — a straight line through master, which is what the example was
trying to show:

    "params": { "master": [[0, 0], [1, 1]] }

`"r"`, `"g"` and `"b"` take the same shape, each optional, and omitting `params`
entirely (`"params": {}`) is also accepted. Every other published command example in
1.0 was checked against its own contract and runs.

**Status.** The example text is corrected in the specification at its next revision. The
engine's behaviour is not affected, and no document is at risk.

---

## 10 — Bundled Qt frameworks carry the build machine's paths as their own names

**What you will see.** Nothing, in use. Inspecting the bundled libraries shows that 34 Qt
frameworks and `libbrotlicommon.1.dylib` record a `/opt/homebrew/opt/...` path as their own
install name:

```
otool -D Scandium.app/Contents/Frameworks/QtWidgets.framework/Versions/A/QtWidgets
```

**Why it happens.** `macdeployqt` copies the Homebrew-built frameworks into the bundle and
rewrites what the app *loads*, but not the name each library records for itself. Nothing resolves
through that recorded name: every dependency inside the bundle is `@executable_path`-relative, and
no binary in the image refers to an absolute non-system path. The app therefore runs correctly on
a machine that has never had Homebrew installed.

**Effect.** Provenance only: the paths disclose the layout of the machine that built the release,
and they make the bytes vary with the builder's prefix. There is no runtime consequence, so this
is recorded rather than changed for 1.0 (finding OB-92, operator ruling WORD 14).

---

## 11 — `additionalProperties: false` is enforced only at the top level of a command's `params`

**What you will see.** A command envelope that carries a misspelled or unknown key *inside* a nested
object is **accepted** instead of refused. The unknown key is discarded, the rest of the object
applies normally, and the result is `ok`. Nothing is written incorrectly and nothing is lost — but a
typo you expected to be caught is not reported.

**What is not affected.** The top level of `params` **is** enforced: an unknown key there is refused
with `error.schema.out_of_range` and the message quotes the strictness ruling. Nested `required` is
also enforced where a kind decodes the object itself — for example `vector.shape_add`'s `fill` now
refuses a payload that matches no declared branch instead of rendering it black.

**Why.** Stage-3 validation (`crates/appname-core/src/engine/registry.rs`) checks a declared
property's type, enum membership, bounds, and — on object-valued properties — a `oneOf` of
required-alternatives. For an object that satisfies its type it then stops: it does not descend into
that object's own `properties`, `required` or `additionalProperties`. So the 2026-08-17 strictness
ruling, which made `additionalProperties: false` the contract surface, holds at the top level of
`params` and not below it. The behaviour is **loose acceptance, not data loss** — the engine never
stores or acts on a key it does not recognise.

**Status.** Filed as `q-scenario/Q-6`, documented here for 1.0 by operator ruling (WORD 26 (1)).
A fix was written and measured rather than estimated, and it was declined on its measurements:

    recursive enforcement, as drafted ..... 40 added lines (the ruling's bar was under twenty)
    cargo test --workspace --no-fail-fast . 139 suites, 2560 passed, **5 failed** in 3 suites
                                            (the ruling's bar was gate-green)

    All five failures share one cause, and it is not the validator: three test suites send
    `"fill": {"type": "solid"}` with no `color`, which the contract's own `solid` branch declares
    required. Enforcing the nested schema therefore refuses fixtures that are themselves
    schema-invalid. Repairing them is a test change, and the 1.0 tree is frozen.

Enforcement below the top level is deferred past 1.0 with the fixtures it would expose.

## 12 — Five wasmtime advisories (RUSTSEC-2026-0315, RUSTSEC-2026-0316, RUSTSEC-2026-0325, RUSTSEC-2026-0326, RUSTSEC-2026-0327) are ignored for 1.0

wasmtime is in the source dependency graph but compiled into no shipped binary, so these advisories (fuel accounting; GC heap corruption; an unvalidated component callback result count) do not reach the application; the bump to wasmtime ≥ 48.0.4, which fixes all five, lands in 1.0.1.

---

## 13 — Quick Selection is not in 1.0

**What you will see.** There is no Quick Selection tool. It is not in the tool palette, and W
chooses the Magic Wand only.

**Why.** The engine has the tool; the shell does not yet route canvas input to it. Recorded in
sitting #1 on RC2 (`reports/SITTING-1.0.md`, step S-09) and ruled a scope item, not a defect, by
the operator (WORD 48 (3)).

**Workaround.** Use the Rectangle, Lasso or Magic Wand selection tools, which work, and refine
with Select ▸ Modify… or Shift-click (add) and Option-click (subtract) with the wand.

**Status.** Planned after 1.0; no date set.

---

## 14 — Warp and Puppet Warp are not in 1.0

**What you will see.** There is no Warp or Puppet Warp. Neither is in the tool palette, and Edit
▸ Puppet Warp is gone. Free Transform and Perspective work.

**Why.** The interactive warp session is not wired to the canvas in the 1.0 shell. Recorded in
sitting #1 on RC2 (`reports/SITTING-1.0.md`, step S-16) and ruled a scope item, not a defect, by
the operator (WORD 48 (3)).

**Workaround.** Use Free Transform or Perspective for the shapes they can make.

**Status.** Planned after 1.0; no date set.

---

## 15 — Rounded Rectangle and path boolean operations are not in 1.0

**What you will see.** The shape tools are Rectangle and Ellipse; there is no Rounded Rectangle,
Line or Polygon shape tool, and U cycles between the two that work. No menu or options-row
control combines paths or shapes (unite, subtract, intersect). The Pen tool works.

**Why.** Neither feature is wired in the 1.0 shell. Recorded in sitting #1 on RC2
(`reports/SITTING-1.0.md`, step S-14) and ruled a scope item, not a defect, by the operator
(WORD 48 (3)).

**Workaround.** Draw the outline with the Pen tool. Shapes cannot be combined in 1.0.

**Status.** Planned after 1.0; no date set.

---

## 16 — Text cannot be typed or rendered in 1.0

**What you will see.** There is no Text tool. It is not in the tool palette, and T does nothing.

**Why.** Text layout and glyph rendering are not wired in the 1.0 shell. Recorded in sitting #1 on RC2
(`reports/SITTING-1.0.md`, step S-15) and ruled a scope item, not a defect, by the operator
(WORD 48 (3)).

**Workaround.** Set the text in another application and bring it in as a raster layer with File
▸ Import….

**Status.** Planned after 1.0; no date set.

---

## 17 — Adjustment dialogs have no live preview

**What you will see.** The adjustment dialogs (Brightness/Contrast, Exposure, Levels, Curves,
Hue/Saturation, Threshold) are modal and the canvas behind them does not change until you press
Apply. Cancel restores the document exactly.

**Why.** Live preview is not wired in the 1.0 dialogs. Recorded in sitting #1 on RC2
(`reports/SITTING-1.0.md`, step S-12 (a)) and ruled a scope item, not a defect, by the operator
(WORD 48 (3)).

**Workaround.** Apply, judge the result, and undo if needed. Applied adjustments are
non-destructive layers, so they can also be adjusted or deleted later.

**Status.** Planned after 1.0; no date set.

---

## 18 — OpenRaster (`.ora`) files cannot be opened or imported

**What you will see.** No menu path opens or imports an OpenRaster file. `.ora` is greyed out in
both File ▸ Open and File ▸ Import….

**Why.** OpenRaster decoding has no entry point in the 1.0 shell. Recorded in sitting #1 on RC2
(`reports/SITTING-1.0.md`, step S-04) and ruled a scope item, not a defect, by the operator
(WORD 48 (3)).

**Workaround.** Export the document from its originating application as PNG or TIFF and open or
import that.

**Status.** Planned after 1.0; no date set.

---

## 19 — Layers cannot be put into a group

**What you will see.** There is no way to group layers. The Layers panel has no group or
ungroup button, and Layer ▸ Group, Layer ▸ Ungroup, Cmd+G and Cmd+Shift+G are gone.

**Why.** Multi-selection and group membership are not wired in the 1.0 Layers panel. Recorded in
sitting #1 on RC2 (`reports/SITTING-1.0.md`, step S-11) and ruled a scope item, not a defect, by
the operator (WORD 48 (3)).

**Workaround.** None in 1.0. Keep related layers adjacent and reorder them with Move Up and Move
Down.

**Status.** Planned after 1.0; no date set.

---

## 20 — The Magic Wand always samples all layers

**What you will see.** The wand has no Sample All Layers option. It always selects by the colours
of the whole visible image, on any document.

**Why.** Sampling only the active layer of a multi-layer document needs engine support that is
not in 1.0. Recorded in sitting #1 on RC2 (`reports/SITTING-1.0.md`, step S-08) and ruled a
scope item, not a defect, by the operator (WORD 48 (3)).

**Workaround.** To select by one layer's colours, hide the other layers first.

**Status.** Planned after 1.0; no date set.

---

## 21 — Value fields do not accept typed numbers

**What you will see.** In the adjustment dialogs and in the options row (for example brush size),
typing a number into a value field and pressing Tab leaves the old value in place. In the dialogs
only the − and + buttons change the value, in steps of 0.01.

**Why.** Numeric entry is not wired in the 1.0 value fields. Recorded in sitting #1 on RC2
(`reports/SITTING-1.0.md`, step S-12) and ruled a scope item, not a defect, by the operator
(WORD 48 (3)).

**Workaround.** In the dialogs, use the − and + buttons.

**Status.** Planned after 1.0; no date set.

---

## 22 — Image ▸ Rotate does not rotate shape layers

**What you will see.** Rotating the image rotates every pixel layer but leaves shape layers
(Rectangle, Ellipse, Pen) where they were.

**Why.** Whole-image rotation does not yet transform vector shape layers in 1.0. Recorded in
sitting #1 on RC2 (`reports/SITTING-1.0.md`, step S-17) and ruled a scope item, not a defect, by
the operator (WORD 48 (3)).

**Workaround.** Keep the shapes on their own layers and rotate the image before drawing them, or
redraw them after rotating.

**Status.** Planned after 1.0; no date set.

## 23 — The Healing Brush is slow on large images

**What you will see.** Heal: 22–24 s per stroke on a 3000×1875 image on an M-series Mac; faster on
smaller images; optimized in 1.0.1. The stroke completes with the correct result, but the app
does not respond until it does.

**Why.** Measured natively on the 1.0 build (one heal stroke on a 5.6-megapixel photograph). The
operator kept the tool for 1.0 with this note rather than remove it (2026-10-11).

**Workaround.** Wait for the stroke to finish; the result is correct. On smaller images it is
faster.

**Status.** Optimized in 1.0.1.

---

## 24 — Paint Bucket, Content-Aware Fill, Slice and Transform Selection are not in 1.0

**What you will see.** None of these tools is in the tool palette. G does nothing, and C chooses
Crop only.

**Why.** None of them was wired to the canvas in the 1.0 shell; each only said so when used. The
operator's rule for 1.0 removes every control that does not do its job (corrigendum (l)).

**Workaround.** For a fill, use Edit ▸ Fill… on a selection, or the Fill button in the Layers panel. To move
or scale selected pixels, copy them to a new layer and use Free Transform.

**Status.** Planned after 1.0; no date set.

---

## 25 — Filter ▸ Liquify and Filter ▸ Custom are not in 1.0

**What you will see.** The Filter menu lists only the filters that work. Liquify and Custom are
gone.

**Why.** Both items were always greyed out. The operator's rule for 1.0 removes every control that
does not do its job (corrigendum (l)).

**Workaround.** None in 1.0.

**Status.** Planned after 1.0; no date set.

---

## 26 — Free Transform is slow to apply on large images

**What you will see.** Free Transform: about 4 s to apply on a 3000×1875 image on an M-series Mac;
faster on smaller images. The first frame of the result appears within half a second, and the
transform finishes with the correct result.

**Why.** Measured natively on the 1.0 release image (Return after a free transform on a
5.6-megapixel photograph, settled time). Under the operator's 1.0 rule a correct control that takes
3–10 seconds stays, documented as slow on large images (corrigendum (o), ruling of 2026-10-11).

**Workaround.** Wait for the transform to finish; the result is correct.

**Status.** Planned for 1.0.1.

---

## 27 — Heal samples the active layer only

**What you will see.** The Healing Brush has no Sample All Layers option. On an empty layer above the
picture, a heal stroke changes nothing, although History records it.

**Why.** The 1.0 healing engine reads only the active layer. Its Sample All Layers checkbox had no
effect, so it was removed under the operator's 1.0 rule (corrigendum (l)); found in the RC5 sitting
(S-08).

**Workaround.** To heal from the visible image, merge or flatten the layers first, or use the
Clone Stamp with Sample All Layers on.

**Status.** Planned after 1.0; no date set.

---

## 28 — Merge Down applies the lower layer's mask

**What you will see.** After Layer ▸ Merge Down, the lower layer's mask is applied into its pixels and
the mask is removed; the composite is unchanged.

**Why.** Found in the RC7 checks (O-WATCH #1216); recorded on the operator's ruling (t), 2026-10-13.

**Workaround.** None needed for the picture, which is unchanged.

**Status.** Planned after 1.0; no date set.

---

## 29 — About can leave a dim overlay when closed through accessibility while Licenses is open

**What you will see.** About: closing it through the accessibility Press action while Licenses is open
leaves a dim overlay; keyboard and mouse close are unaffected.

**Why.** Found in the RC7 sitting (R7-1, O-WATCH #1204); recorded on the operator's ruling (t),
2026-10-13.

**Workaround.** Close Licenses first, or close About with the keyboard or the mouse.

**Status.** Planned after 1.0; no date set.

---

## 30 — Windows: not yet verified on a clean machine

**What you will see.** Nothing different is expected. The Windows 1.0 zip has been built and tested on
the build machine, and every DLL it imports has been checked to be in the zip or provided by Windows
or the Visual C++ Redistributable. It has not yet been installed and run on a clean Windows install.

**Why.** No clean-machine VM was available before release (D-13, operator's word (o), 2026-10-11).

**Workaround.** Install the Microsoft Visual C++ Redistributable (x64) before first launch (see README).

**Status.** The clean-machine run follows the release; no date set. Platform: Windows.

---
