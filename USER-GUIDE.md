<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (c) 2026 the Appname contributors -->

# Scandium User Guide

Scandium is an image editor for macOS and Windows. This guide is organised by what you are trying to
do. Shortcuts are given in both forms: macOS first, Windows second, like this — **Cmd+T / Ctrl+T**.

---

## Contents

- [1. The workspace](#1-the-workspace)
- [2. The keyboard](#2-the-keyboard)
- [3. Tools and their shortcuts](#3-tools-and-their-shortcuts)
- [4. Painting and retouching](#4-painting-and-retouching)
- [5. Undo, redo and the clipboard](#5-undo-redo-and-the-clipboard)
- [6. Selections](#6-selections)
- [7. Masks](#7-masks)
- [8. Layers and adjustment layers](#8-layers-and-adjustment-layers)
- [9. Filters](#9-filters)
- [10. Transforms](#10-transforms)
- [11. Changing the image itself](#11-changing-the-image-itself)
- [12. Opening and saving](#12-opening-and-saving)
- [13. Not losing your work](#13-not-losing-your-work)
- [14. About and Licenses](#14-about-and-licenses)
- [15. If something goes wrong](#15-if-something-goes-wrong)

---

## 1. The workspace

When you open a document you get one window with four parts.

**The canvas** sits in the middle and shows the image. The scroll wheel zooms, centred on the pointer;
**Cmd+0 / Ctrl+0** fits the image to the window and **Cmd+1 / Ctrl+1** returns it to 100%.

**The tool palette** runs down the side. Click a tool to select it, or press its letter (see §3). Tools
that share a letter sit in a group; the palette shows the one you used last.

**The options bar** runs across the top and changes with the tool. Fourteen of the twenty-two tools put
something there; the other eight leave it empty, which is the ordinary case and not a fault.

- The **brush** shows **Size**, **Hardness** and **Brush color**.
- The **eraser**, **heal**, **dodge**, **burn**, **sponge**, **blur**, **sharpen** and **smudge** show
  **Size** alone.
- The **clone stamp** shows **Size** and **Sample All Layers**.
- The **magic wand** shows **Tolerance** and **Contiguous**.
- The **rectangle** and **ellipse** show **Stroke** — the width of the outline.
- The **pen** shows **Commit as**, either **Shape layer** or **Path**, which decides what finishing a path
  produces.

Everything else leaves the bar empty: move, crop, both marquees, both lassos, and the free and
perspective transforms.

**Three panels** sit together on the right:

- **Layers** — the layer stack, topmost first. Each row has a visibility toggle, the layer's name, and a
  checkbox for picking several layers at once. Below the list are opacity and blend mode for the
  selected layer, and buttons to add, duplicate, merge down, delete and reorder — plus the
  **Mask** button (§7) and the **adjustment** menu (§8).
- **History** — one row per action you have taken. Click a row to return the document to that point.
- **Mask** — the mask on the selected layer: enabled, feather, density, invert, and whether the mask is
  linked to the layer's position.

**Panels and dialogs behave differently, and it is worth knowing which you are in.** A panel control acts as
you move it — drag the mask's feather and the image changes under your hand. A dialog does not: it waits for
**Apply**, and **Cancel** leaves the document exactly as it was.

---

## 2. The keyboard

Tab reaches every control in Scandium — buttons, checkboxes and tool buttons as well as text fields and
lists — regardless of the macOS "Full Keyboard Access" setting. On Windows, Tab reaches every control as
it does in any application.

### Getting around without the mouse

**Tab** and **Shift+Tab** step through the controls. **F6** moves to the next panel and **Shift+F6** to the
previous one, so you can cross from the canvas to the tool palette to the panels without reaching for the
trackpad. **F10** — or **Cmd+F2 / Ctrl+F2** — arms the menu bar. **Shift+F10**, or the **Menu** key if your
keyboard has one, opens the canvas's own right-click menu (§5) at the point the pointer would have used.

With the canvas focused, the **arrow keys** nudge the selected layer by one pixel and **Shift+arrow** by ten.

### Every shortcut Scandium binds

Tool letters have their own table in §3. Everything else is here.

| Shortcut | What it does |
|---|---|
| **Cmd+N / Ctrl+N** | New document |
| **Cmd+O / Ctrl+O** | Open |
| **Cmd+S / Ctrl+S** | Save |
| **Shift+Cmd+S / Ctrl+Shift+S** | Save As |
| **Cmd+Z / Ctrl+Z** | Undo |
| **Shift+Cmd+Z / Ctrl+Y** | Redo |
| **Cmd+X / Ctrl+X** | Cut — removes the selected layer |
| **Cmd+C / Ctrl+C** | Copy the flattened image |
| **Cmd+V / Ctrl+V** | Paste as a new layer |
| **Delete** or **Backspace** | Delete — see §5 for what it clears |
| **Cmd+A / Ctrl+A** | Select all |
| **Cmd+D / Ctrl+D** | Deselect |
| **Shift+Cmd+I / Ctrl+Shift+I** | Invert the selection |
| **Shift+Cmd+D / Ctrl+Shift+D** | Reselect — bring back the selection you just dropped |
| **Cmd+J / Ctrl+J** | Copy the selection into a new layer |
| **Cmd+T / Ctrl+T** | Free transform |
| **Cmd+0 / Ctrl+0** | Fit the image to the window |
| **Cmd+1 / Ctrl+1** | View at 100% |
| **Cmd+plus / Ctrl+plus** | Zoom in |
| **Cmd+minus / Ctrl+minus** | Zoom out |
| **F6** / **Shift+F6** | Next / previous panel |
| **F10**, or **Cmd+F2 / Ctrl+F2** | Arm the menu bar |
| **Shift+F10** or **Menu** | Open the canvas menu |

⚠ **Two of these have no menu item and no button anywhere in the window.** **Reselect** and **Copy the
selection into a new layer** are reachable from the keyboard only in this release, so a mouse-only user will
not find them by hunting the menus — there is nothing there to find.

---

## 3. Tools and their shortcuts

Press a letter to reach a tool. Press the same letter again to move to the next tool in that group — the
group remembers which one you used last, whether you got there by key, palette click, or menu.
**Shift+letter** works the same way.

| Key (macOS / Windows) | Tools in the group |
|---|---|
| **V** | Move |
| **M** | Rectangular marquee → Elliptical marquee |
| **L** | Lasso → Polygon lasso |
| **W** | Magic wand |
| **C** | Crop |
| **B** | Brush |
| **E** | Eraser |
| **J** | Heal |
| **S** | Clone stamp |
| **O** | Dodge → Burn → Sponge |
| **P** | Pen |
| **U** | Rectangle → Ellipse |
| **X** | Swap the foreground and background colours (not a tool) |

Five tools have no letter and are selected from the palette: blur, sharpen, smudge, and the free and
perspective transforms. Free transform is also **Cmd+T / Ctrl+T**.

**Setting a clone source.** With the clone stamp active, **Option-click / Alt-click** the place you want
to copy *from*, then paint elsewhere. This is the one place a modifier click changes what a tool does.

---

## 4. Painting and retouching

Choose a layer in the Layers panel first — painting always goes to the selected layer.

The brush takes **Size**, **Hardness** and a colour from the options bar. The eraser takes **Size**
alone — hardness belongs to the brush and to nothing else. Heal, dodge, burn, sponge, blur, sharpen
and smudge all work the same way: pick the tool, set its options, and drag on the canvas. Each stroke
becomes one row in the History panel, so you can step back to any point.

The clone stamp offers **sample all layers**. With it on, it copies from the composited image; with it
off, it copies only from the selected layer. Heal always samples the selected layer (Known Issue 27).

The magic wand has its own two settings: **Tolerance**, which widens or narrows the range of colour it
accepts, and **Contiguous**, which decides whether it selects only the region touching where you clicked or
**every matching pixel on the canvas**.

---

## 5. Undo, redo and the clipboard

Every edit can be undone. **Cmd+Z / Ctrl+Z** steps back and **Shift+Cmd+Z / Ctrl+Y** steps forward. The
History panel is the same thing shown by name: click any row to jump straight to that point rather than
pressing the key repeatedly.

The clipboard commands are on the **Edit** menu, and two of them are **not** mirror images of each other —
worth knowing before you reach for them:

| Command | Shortcut | What it does |
|---|---|---|
| **Cut** | **Cmd+X / Ctrl+X** | removes the **selected layer** |
| **Copy** | **Cmd+C / Ctrl+C** | copies the **flattened image as you see it** — not the selected layer |
| **Paste** | **Cmd+V / Ctrl+V** | makes a **new layer above the selected one**. It ignores any selection |
| **Paste Into** | — | the same new layer, **masked to the current selection**. It needs one, and is greyed out without |
| **Delete** | **Delete** / **Backspace** | see below |

If a paste arrives larger than the window, the view zooms out so you can see all of it.

**Delete** behaves differently depending on whether something is selected. With a selection active it clears
the selected layer *within the selection* and does not ask. With **no** selection it would clear the whole
layer, so it stops and asks first.

**Right-clicking the canvas** opens a menu carrying the clipboard commands, the whole **Select** menu
including *Modify*, **Filter**, and **Crop** and **Trim**. Every one of them is also in the menu bar — it is a
shorter route to the same commands, never extra features.

---

## 6. Selections

Make a selection with the marquee (**M**), lasso (**L**), or the magic wand (**W**), then
paint, adjust or delete inside it. A selection shows as a moving dashed outline.

The **Select** menu holds the rest: select all, deselect, invert, save the current selection, load a
saved one, and load a layer's alpha as a selection.

**Select ▸ Modify** reshapes the selection you already have. It offers five operations — **expand**,
**contract**, **feather**, **border** and **smooth** — each taking an amount.

---

## 7. Masks

A mask hides part of a layer without deleting it. Add one from the **Mask** button in the Layers panel, which
offers five starting points:

| Start from | What you get |
|---|---|
| **Reveal All** | a mask that hides nothing yet |
| **Hide All** | a mask that hides the whole layer |
| **From Selection** | the selected area is what shows |
| **From Selection, Inverted** | the selected area is what hides |
| **From Transparency** | a mask built from the layer's own transparent areas |

The **Mask** panel then controls the selected layer's mask: turn it off and on, feather its edge, reduce its
density, invert it, and link or unlink it so it does or does not move with the layer. Every one of those acts
as you change it, and every change can be undone.

⚠ **Painting on a mask is not in this release.** A brush stroke always goes to a layer, never to a mask, so a
mask is shaped by the five starting points above and then by the panel's controls — not by hand with a brush.

---

## 8. Layers and adjustment layers

The Layers panel builds the picture from the bottom up. Add, duplicate, merge down, reorder and delete
are all on the panel. Layers cannot be grouped in 1.0 (Known Issue 19). To
rename a layer, double-click its name in the list and type.

**Adjustment layers** change the layers beneath them without altering a pixel. Add one from the panel's
adjustment menu and choose a kind:

| Adjustment | What it does |
|---|---|
| Brightness/Contrast | Overall lightness and contrast, plus a **legacy** switch |
| Curves | A tone curve you shape by hand — separately for **master, red, green and blue** |
| Exposure | Exposure, offset and gamma |
| Hue/Saturation | Hue, saturation and lightness for the **master** and six colour bands, plus a **colorize** option with its own three settings |
| Invert | Inverts the tones — no settings at all |
| Levels | Input black and white, midpoint, output black and white — for the **composite** channel and R, G and B separately |
| Threshold | Reduces everything to black and white at a level you choose |

Six of the seven open a dialog with **Apply** and **Cancel**. The image changes when you apply — not
while you are still moving the numbers — so set the values, then apply. **Invert** has nothing to set,
so choosing it adds the layer at once.
Because the adjustment lives on its own layer, you can hide it, reorder it, mask it, or delete it later.

The **Image** menu also holds three one-step commands — **auto tone**, **auto contrast** and **auto
colour** — which apply once to the image rather than adding a layer.

---

## 9. Filters

The **Filter** menu has six filters, each with its own settings dialog: **Gaussian Blur**, **Box Blur**,
**Median**, **High Pass**, **Add Noise** and **Unsharp Mask**. As with adjustments, the filter reaches the
image when you press **Apply**.

Liquify and a custom-kernel filter are not in 1.0 (Known Issue 25).

---

## 10. Transforms

Two transforms are in the palette: **free** and **perspective**. Select the layer, choose the transform,
drag the handles, and confirm with **Return**. Neither has settings in the options bar. A transform
applies to the selected layer only, and it can be undone like any other edit. Warp and Puppet Warp are not
in 1.0 (Known Issue 14).

---

## 11. Changing the image itself

Five commands on the **Image** menu change the whole document rather than one layer. Each opens a dialog
with **Apply** and **Cancel**.

- **Resize Image…** — **Set** chooses which dimension you type — **width**, **height** or **longest edge** —
  and the value goes beside it. The rest of the dialog is **Resolution (PPI)**, **Constrain aspect** to hold
  the proportions, **Resample**, and **Scale effects**.
- **Canvas Size…** — changes the working area without scaling the picture. Set **width** and **height**,
  an **Anchor** deciding where the existing image sits in the new area, **Relative** to add to the current
  size instead of replacing it, and an **Extension color** for the new space: white, black, transparent, or
  the current background colour.
- **Rotate…** — an **Angle (degrees)**, **Flip horizontal**, **Flip vertical**, **Expand canvas** so
  rotated corners are not cut off, and **Resample**.
- **Trim…** — removes uniform edges. Choose what to judge by under **Based on** — **transparent pixels**,
  the **top-left colour**, or the **bottom-right colour** — and tick which edges to trim: **top**, **left**,
  **right**, **bottom**.
- **Flatten Image…** — merges the whole stack into one layer, with **Discard hidden layers** as an option.
  Its dialog says plainly that this is a whole-document step and that undo restores a snapshot of what you
  had. **Image ▸ Convert Depth…** says the same thing for the same reason.

---

## 12. Opening and saving

**Open** takes **one** kind of file: Scandium's own package, which the dialog lists as **Scandium package**
and which uses the extension **`.appname`** — the internal project name, kept for 1.0 (Known Issue 7). It is
the only format that carries a layer stack, and opening it restores the document whole.

⚠ **So you do not start from a photograph by opening it.** Make a document first — **File ▸ New** — and then
bring the picture in with **Import…**, or by dragging the file onto the window. Both add it as a layer to the
document you already have; a drop onto a window with no document open says so rather than doing nothing.

**Import…** adds a picture to the document you already have, as a **new layer**, instead of opening it as
its own document. It takes flat rasters only — `png`, `jpg`/`jpeg`, `tif`/`tiff`, `webp`, `gif` and `bmp` —
so a Scandium package cannot be imported — that one you **open** — and an `ora` file cannot be brought in at
all in this release. Because Import only adds a layer it never asks about unsaved changes, and it can be
undone like any other edit.

**Export** writes a flattened copy in any of six formats: **png**, **jpeg**, **tiff**, **webp**, **gif** and
**bmp**. Export is for handing a picture to something else; it does not carry layers.

**Save** and **Save As** write the Scandium package (`.appname`), which keeps everything — layers, masks,
adjustment layers and history position. **That is the format to work in**, and Export is what you produce at
the end.

**Image ▸ Convert Depth…** converts the document between **8-bit integer** and **32-bit float**.
**Assign Profile…** tags the document with sRGB, Display P3, ProPhoto RGB, or no profile; **Convert to
Profile…** converts the pixels into sRGB, Display P3 or ProPhoto RGB. Documents are always RGB in 1.0.

---

## 13. Not losing your work

Scandium stops you before unsaved work is lost. **Three** things stop: closing the window with unsaved
changes, starting a new document, and opening another. The dialog is titled **Unsaved Changes** and it offers
exactly two buttons — **Cancel** and **Discard Changes**. **Cancel** always returns you to the document
exactly as it was.

⚠ **It does not offer to save for you, and *Discard Changes* means what it says.** If you want to keep the
work, choose **Cancel**, save with **Cmd+S / Ctrl+S**, and then do again whatever you were doing.

⚠ **Save before you quit.** The guard sits on those three doors, and quitting the application is not one of
them in this release — so do not rely on being asked at all.

If the application stops unexpectedly, your work is not lost. The next time you **open that document**,
Scandium notices the newer unsaved state and offers to recover it. The offer comes with the document, not
at startup, so open the file you were working on.

---

## 14. About and Licenses

**Help ▸ About Scandium** shows the version and the build it came from, the acknowledgments for the open
source work Scandium is built on, the Qt LGPL notice, a trademarks line, and this statement:

> Scandium is an independent project, not affiliated with, endorsed by, or sponsored by Adobe Inc.

From that same window, **Third-party notices…** opens the licence list. Select a component to read its text.
A long text may be shown in part, with a pointer to the licences folder; and a build that does not carry the
texts says so plainly instead of showing an empty pane.

**Help ▸ Open Licenses Folder** opens the folder holding the complete licence files in your file manager. That
is where the full text lives — the Help menu has these two entries and no other.

---

## 15. If something goes wrong

**macOS refuses to open the app ("unidentified developer").** Right-click the application and choose
**Open**, then confirm. If macOS still refuses, try to open the app once, then go to **System Settings →
Privacy & Security** and choose **Open Anyway**. This is needed once.

**Windows shows a SmartScreen warning.** Choose **More info**, then **Run anyway**.

**Export fails on a grayscale or Lab document.** The app itself only makes RGB documents, so you can meet
this only through the scripting door (`doc.mode_convert`). Exporting is not supported for grayscale or Lab
documents in this release; the export refuses and says so rather than writing a wrong file. Convert the
document back to RGB with `doc.mode_convert` to `rgba` and export again. This is Known Issue 1.

**A licence entry is blank with no explanation.** Report it. Two *explained* states are normal and are not
faults: a long text may say it is shown in part and point you at the licences folder, and a build that does
not carry the texts says so (see §14). A pane that is simply empty, saying nothing, is the one to report.
