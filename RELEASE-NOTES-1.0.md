<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (c) 2026 the Appname contributors -->

> **Draft, written 2026-09-26 before the release candidate is cut.** Three things are completed by the
> release ceremony and are marked where they appear: the list of fixes since the first candidate, the
> checksums, and the tag date if it moves. Everything else is final.

# Scandium 1.0 — Release Notes

Scandium 1.0 is a free and open-source image editor for macOS and Windows. It is released under the
GNU General Public License, version 3 only; the full text is in `LICENSE`, and the plug-in SDK is
separately licensed under Apache-2.0 in `LICENSE-SDK`.

Both platforms ship from a single tag, `v1.0.2`, so the macOS disk image and the Windows zip are built
from exactly the same source. Version strings inside the application read `1.0.2`.

## What is in 1.0

Scandium edits layered documents with a non-destructive pipeline: painting and retouching tools,
selections and masks, adjustment layers, vector shapes, and an import/export path covering the
common still formats. A scripting door is included for driving the editor from the command line.
Text, Quick Selection, Warp, live preview in adjustment dialogs and the other items listed under
Known issues below are not in 1.0.

**Crop trims the document by default.** In the Crop dialog, *Delete cropped pixels* starts on, so
Apply makes the document the cropped size. Turn it off for a non-destructive crop: the document
keeps its size, only the frame that is shown and exported changes, and Undo brings the full image
back. Changing the canvas size afterwards drops that frame. The default is on because, in the first
test sitting, a non-destructive crop that left the size unchanged read as "Crop did nothing".

**Scandium 1.0 does not load plug-ins.** The application binary contains no plug-in host, so no
third-party code can be loaded or executed. The plug-in interface is declared for a future release
and `LICENSE-SDK` is published against it, but nothing in 1.0 can run a plug-in.

Scandium makes no network calls of its own. There is no telemetry, no update check, and no
account, and the application contains no code that contacts a server.

One thing you will notice if you inspect the application, and we would rather tell you than have
you find it: Scandium bundles the Qt libraries, and the Qt set it needs brings Qt's networking
library, two TLS backends, and OpenSSL along with it. They are present in the application bundle
because other Qt components depend on them, and Scandium never calls them. If you run `otool -L`
on the executable you will see `QtNetwork` in the list; that is the dependency, not a feature.

## What is not in 1.0

Three features were cut to a later release. Each was cut deliberately, on a dated decision, and each
is listed in `KNOWN-ISSUES.md` as well:

- **Layer styles.** Cut to 1.1 by Amendment 5 of 2026-09-06, when the ceremony that would have opened
  the specification for them was not held.
- **The gradient tool.** Cut to 1.1 by the same amendment. Gradient *fill layers* are unaffected and
  do ship.
- **A fix for non-RGBA export.** Cut by Amendment 7 of 2026-09-10 in favour of a typed refusal and this
  note: a document converted to Grayscale or Lab mode cannot currently be exported in any format. Convert
  back to RGB before exporting. This is issue 1 below and it is the one cut we most regret.

Two further items are deferred rather than cut, meaning the work exists but does not ship in 1.0:

- **macOS notarization** is deferred by Amendment 9 of 2026-09-12. The disk image is not signed, and
  the application inside carries only an ad-hoc signature with no Developer ID, so macOS cannot
  verify who built it; see "Opening Scandium the first time" below.
- **Windows code signing** is deferred. The Windows build is unsigned; see the same section.

## Windows

**Rendering.** On Windows, Scandium renders on the GPU backend that wgpu selects for the machine.
Direct3D 12 and Vulkan were both measured against the CPU reference and agree with it within 1/255
per channel.

**Exported pixels.** A Windows export is byte-identical to the macOS export of the same document;
every file in our golden test corpus was checked. Two math kernels in the engine (ACES tone mapping
and deconvolution) call the platform's math library, which rounds differently on Windows and macOS
in the last bits; no 1.0 export uses them, and on their own test inputs they agree within 1/255 per
channel (measured: under 0.0001 of one 8-bit step). A single math library on both platforms is
planned for 1.0.1.

## Known issues

Twenty-nine issues are known, reproduced, and shipped deliberately. `KNOWN-ISSUES.md` carries the
full description, what you will see, and the workaround where one exists.

1. Documents in Grayscale or Lab mode cannot be exported in any format. In 1.0 only the scripting
   door can make one (Image ▸ Mode… is removed); convert to RGB first.
2. The documented dirty rectangle for `vector.shape_add` understates the area actually repainted. The
   result is correct; only the documentation is narrow.
3. One user-interface test is intermittent. This affects our test runs, not the application.
4. No instrument enumerates individual test slots, so our own coverage reports are coarser than we
   would like.
5. Layer styles are not in 1.0.
6. The gradient tool is not in 1.0.
7. The internal project name is still `appname` in file paths and the executable name. This is
   cosmetic and will be reconciled in 1.1.
8. Smart objects are not in 1.0.
9. The scripting example for adding a Curves adjustment does not run as printed.
10. The bundled Qt frameworks carry the build machine's paths in their internal names. This is
    harmless and does not affect relinking.
11. `additionalProperties: false` is enforced only at the top level of a command's `params`.
12. Five wasmtime advisories are ignored for 1.0; wasmtime is in no shipped binary.
13. Quick Selection is not in 1.0.
14. Warp and Puppet Warp are not in 1.0.
15. Rounded Rectangle and path boolean operations are not in 1.0.
16. Text cannot be typed or rendered in 1.0.
17. Adjustment dialogs have no live preview.
18. OpenRaster (`.ora`) files cannot be opened or imported.
19. Layers cannot be put into a group.
20. The Magic Wand always samples all layers; hide other layers to select by one.
21. Value fields do not accept typed numbers.
22. Image ▸ Rotate does not rotate shape layers.
23. Heal: 22–24 s per stroke on a 3000×1875 image on an M-series Mac; faster on smaller images;
    optimized in 1.0.1.
24. Paint Bucket, Content-Aware Fill, Slice and Transform Selection are not in 1.0.
25. Filter ▸ Liquify and Filter ▸ Custom are not in 1.0.
26. Free Transform: about 4 s to apply on a 3000×1875 image on an M-series Mac; faster on smaller
    images.
27. Heal samples the active layer only; merge first, or use Clone with Sample All Layers.
28. After Layer ▸ Merge Down, the lower layer's mask is applied into its pixels and the mask is
    removed; the composite is unchanged.
29. About: closing it through the accessibility Press action while Licenses is open leaves a dim
    overlay; keyboard and mouse close are unaffected.

## Fixed since the first release candidate

*(This list is completed by the release ceremony when the candidate is cut. The fixes below are the
set queued for it.)*

- `q-tutorial/Q-5` — the pending grace window was applied per session rather than per asset add.
- `q-tutorial/Q-7` — both information doors now publish each layer's operations, so `op.*` is
  addressable on a document you did not author.
- `q-tutorial/Q-12` — corrected, with a discriminator added so the failure cannot recur silently.
- `q-scenario/Q-17` — both shape doors now decode their fill, and a fill with no type is refused by
  name instead of rendering black.
- `G-06` — a four-line correction to the layers panel.
- `E-11` — the engine-version assertion now derives from the crate's own version rather than a
  hand-kept constant, so a version bump cannot leave it stale.
- `D-8` — the Magic Wand works on every document: it always samples all layers, and its Sample All
  Layers checkbox is gone.
- `D-9` — deleting, merging, reordering or duplicating a layer updates the canvas at once.
- `S-10` — undo and redo of a mask change, or of a layer delete, merge, move or duplicate, update
  the canvas at once.
- Every control that did not do its job has been removed rather than shipped (the tools listed as
  known issues 13–16, 24 and 25, Layer ▸ Group / Ungroup, and Image ▸ Mode…, whose Grayscale and
  Lab documents could not be exported).

## Rights, credits and notices

Scandium contains no third-party imagery, glyphs, or trade dress. Every image committed to this
project is generated by our own synthesis script. Scandium is not affiliated with, endorsed by, or
connected to Adobe Inc. or any other image-editing vendor, and no part of this product is derived
from their software or assets.

`THIRD-PARTY-NOTICES.md` ships in both artifacts and lists every dependency with its licence and its
obligations, and it is the authority; this section only points at it. Among the components that carry
an attribution requirement are **rgb2spec**, **libmypaint** (ISC) and **spectral.js** (MIT). In
addition:

- **Qt** is used under the LGPL. Scandium ships Qt as replaceable dynamic libraries rather than
  statically absorbing them, so the relinking right that the LGPL grants you is preserved: you may
  substitute your own build of the Qt libraries and the application will use them.
- **wasmtime** appears in `THIRD-PARTY-NOTICES.md` because it is in this project's dependency
  graph. It is **not** compiled into the shipped application, since 1.0 ships no plug-in host.

## Opening Scandium the first time

**macOS.** The disk image is neither signed nor notarized, and the application carries only an ad-hoc
signature (see "What is not in 1.0"). Gatekeeper will refuse to open the application on a double-click. Open it once by right-clicking the application and choosing **Open**,
then confirming; macOS remembers the choice and subsequent launches are normal.

**Windows.** The executable is unsigned. SmartScreen will show a "Windows protected your PC" notice on
first run. Choose **More info**, then **Run anyway**. As with macOS this is required once.

We would rather ship signed and notarized builds, and intend to for 1.1.

## Checksums

*(Completed by the release ceremony.)*

    SHA256SUMS — to be filled at the tag

Verify a download by comparing its SHA-256 against the published `SHA256SUMS` file in the release set.

## Building from source

`BUILDING.md` covers building on macOS and Windows from a clean checkout. The source tarball published
with the release is the tagged tree.

## Reporting problems

Please check `KNOWN-ISSUES.md` first — if what you have hit is listed there, it is known and a
workaround is given where one exists.
