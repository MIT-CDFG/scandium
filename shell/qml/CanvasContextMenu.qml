// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// v1.0.1 item 7 — THE CANVAS CONTEXT MENU (the items-7+14 design entry,
// reports/O-UI-log.md). One QQC2 Menu, opened by (a) right-click on the
// canvas and (b) Shift+F10 / the Menu key (§31.1) at the keyboard anchor
// (CanvasView.contextMenuAnchor: selection bounds center when visible,
// else the visible document center). Two mutually exclusive sections
// switched on AppDoc.selectionActive.
//
// THE CONTRACT (design of record): every item's enabled state reads THE
// single command-availability surface (AppDoc.commandAvailability — the
// 14a truth; zero duplicated predicates) and fires the SAME action path
// as its top-menu twin: direct controller doors use the identical
// expressions; dialog-opening items signal Main.qml, which opens the
// SAME dialog instances the top menus open (Paste's inline handler was
// extracted to root.pasteAndFit consumed by both).
//
// THE ID RULE: items reuse the SAME cataloged menu ids as the top menus
// — zero new visible strings. The two rows the item-7 slice omitted for
// missing catalog ids resolved differently once the menu-mint (3dc5c37)
// sealed both ids:
//   * Paste Into — SHIPPED at item 15 (the menu-mint follow-up
//     remainder). The id landed at the mint; item 15 landed the two
//     halves the row was waiting on: the pasteIntoFromClipboard
//     controller arm on the edit.paste_into door, and the
//     edit.pasteInto availability row (doc && abiBlock12 &&
//     selectionActive — the engine's OWN §40.5 precondition, since
//     edit.paste_into rasterizes the active selection into the pasted
//     layer's mask and refuses error.command.selection_required
//     without one). The tst_canvas_context_menu pin's anti-fake side
//     converted to the row-presence assert per its recorded
//     instruction, and the containment the row promises is proven on
//     ENGINE PIXELS there (inside the selection painted, outside
//     untouched).
//   * Delete — SHIPPED at 14b (id landed + the handler is 14b's ruled
//     disposition: edit.cut WITHOUT the app-side copy; this
//     selection-active section always takes the immediate arm; the
//     §32.5 confirm belongs to the no-selection arm, reachable from
//     the top menu row / the canvas chord). The item-7 pin's Delete
//     side flipped to a row-presence assert per its recorded
//     instruction.
//
// Modify ▸ carries the design's three ops (grow/shrink/feather) under
// their CATALOGED op ids — ui.dialog.modify.expand / .contract /
// .feather ("Expand"/"Contract"/"Feather"; the design's "Grow"/"Shrink"
// display names have no catalog rows — recorded, not minted: the
// cataloged names are the same ops by wire param). Each opens
// SelectionModifyDialog PRESET to the op (openFor); the apply path is
// the dialog's own — one implementation, behavior-identical to
// Select > Modify….
//
// PER-TOOL QUICK ACTIONS (no-selection section; derived from what each
// tool can actually do TODAY — existing action paths + existing
// availability rows only; empties are honest):
//   select.rect/ellipse/lasso/polygon/wand ->
//       Load Last Saved   (select.loadSaved row, controller door)
//       Load Layer Alpha  (select.loadLayerAlpha row, controller door)
//   crop            -> Crop… (image.crop row, cropDialog)
//                      Trim… (image.trim row, trimDialog)
//   brush.stroke / eraser.stroke -> Fill… (edit.fill row, fillDialog)
//   move / transform.free / transform.perspective / transform.warp /
//   transform.puppet / vector.pen / shape.rect / shape.ellipse / text
//                   -> none. Their vocabulary rows (nudge,
//                      transform.enter, layer.viaCopy) are 14b
//                      consumers with no handlers yet; vector.pen's
//                      make-selection has a controller door + staged id
//                      (ui.action.path_make_selection) but NO
//                      availability row — a recorded candidate, never
//                      a duplicated predicate.
//
// A11y rides the MenuItems themselves (text = accessible name); the
// Menu popup carries none (Accessible is inert on Popup-derived types).

import QtQuick
import QtQuick.Controls
import AppnameShell

Menu {
    id: menuRoot
    objectName: "canvasContextMenu"

    // §5 delta F4 (MAJOR): the DISMISSAL press must be CONSUMED, never
    // leaked to the canvas tool — an unmodal popup's outside-click
    // passed through and PAINTED A JOURNALED DAB under brush, made a
    // wand selection, and added a stray vertex to a live polygon
    // session (reviewer-probed, engine truth). modal consumes the
    // outside press (the reviewer-verified fix shape). With dismissal
    // consumed, the click-session exclusion in the open guard
    // (Main.qml — polygon/pen sessions hold no button, serial opens
    // allowed) is honest: an open cannot damage the session and a
    // dismissal cannot feed it input.
    modal: true

    // Bound by the instantiator to Main.qml's root.activeLayerId and
    // canvas.tool — one derivation, no duplicate.
    property string activeLayerId: ""
    property string canvasTool: ""
    // The six live filter kinds — bound to the SAME array the top
    // Filter menu's Repeater consumes (root.liveFilterKinds).
    property var filterKinds: []

    // Dialog-opening items relay to Main.qml, which opens the SAME
    // dialog instances the top menus open.
    signal pasteRequested()
    // item 15: Paste Into relays like Paste does — Main.qml owns the ONE
    // handler both menus fire (root.pasteIntoSelection).
    signal pasteIntoRequested()
    signal fillRequested()
    signal modifyRequested(string opParam)
    signal filterRequested(string filterKind)
    signal cropRequested()
    signal trimRequested()
    // 14b: the Delete row relays to Main.qml's ONE disposition function
    // (root.deleteLayerContentWithConfirm) — in this selection-active
    // section that is always the immediate arm.
    signal deleteRequested()

    readonly property bool selectionMode: AppDoc.selectionActive === true
    readonly property bool selectionTool:
        ["select.rect", "select.ellipse", "select.lasso",
         "select.polygon", "select.wand"].indexOf(canvasTool) >= 0
    readonly property bool cropTool: canvasTool === "crop"
    readonly property bool paintTool:
        canvasTool === "brush.stroke" || canvasTool === "eraser.stroke"

    // ---- selection-active section ----
    MenuItem {
        visible: menuRoot.selectionMode
        text: AppI18n.tr("ui.menu.edit.cut")
        enabled: AppDoc.commandAvailability["edit.cut"].enabled === true
        onTriggered: AppDoc.cutLayer(menuRoot.activeLayerId)
    }
    MenuItem {
        visible: menuRoot.selectionMode
        text: AppI18n.tr("ui.menu.edit.copy")
        enabled: AppDoc.commandAvailability["edit.copy"].enabled === true
        onTriggered: AppDoc.copyCompositeToClipboard()
    }
    // item 15: Paste Into SHIPPED — the row the item-7 slice omitted and
    // the menu-mint sealed the id for. It belongs to THIS section by
    // construction: its availability row (edit.pasteInto) carries the
    // engine's §40.5 ACTIVE-SELECTION precondition, so outside the
    // selection-active section it could only ever render disabled.
    // Relays to Main.qml's ONE handler (root.pasteIntoSelection), the
    // same one the top Edit menu row fires.
    MenuItem {
        visible: menuRoot.selectionMode
        text: AppI18n.tr("ui.menu.edit.paste_into")
        enabled: AppDoc.commandAvailability["edit.pasteInto"].enabled === true
        onTriggered: menuRoot.pasteIntoRequested()
    }
    MenuItem {
        visible: menuRoot.selectionMode
        text: AppI18n.tr("ui.menu.edit.fill")
        enabled: AppDoc.commandAvailability["edit.fill"].enabled === true
        onTriggered: menuRoot.fillRequested()
    }
    // 14b: the Delete row SHIPPED — its mint (ui.menu.edit.delete,
    // sealed 3dc5c37) landed and its handler is 14b's ruled disposition
    // (the design order: Cut/Copy/Paste Into/Fill…/Delete/Deselect —
    // complete as of item 15, which shipped the Paste Into row above).
    // Chord display: Backspace (canvas-scope chord; ChordMenuItem
    // header records the measured Basic-style display gap).
    ChordMenuItem {
        visible: menuRoot.selectionMode
        text: AppI18n.tr("ui.menu.edit.delete")
        chord: "Backspace"
        enabled: AppDoc.commandAvailability["edit.delete"].enabled === true
        onTriggered: menuRoot.deleteRequested()
    }
    MenuSeparator { visible: menuRoot.selectionMode }
    MenuItem {
        visible: menuRoot.selectionMode
        text: AppI18n.tr("ui.menu.select.deselect")
        enabled: AppDoc.commandAvailability["select.deselect"].enabled === true
        onTriggered: AppDoc.applyCommand("selection.none", {}, {}, "")
    }
    MenuItem {
        visible: menuRoot.selectionMode
        text: AppI18n.tr("ui.menu.select.invert")
        enabled: AppDoc.commandAvailability["select.invert"].enabled === true
        onTriggered: AppDoc.applyCommand("selection.invert", {}, {}, "")
    }
    Menu {
        id: modifySubmenu
        // Q-D32: a SUBMENU title, so it takes the ellipsis-free id. The menu-bar
        // row keeps `ui.menu.select.modify` because there the ellipsis is true —
        // it opens modifyDialog directly. An arrow promises a submenu and an
        // ellipsis promises a dialog; this row showed both and only one was true.
        title: AppI18n.tr("ui.menu.select.modify_submenu")
        MenuItem {
            text: AppI18n.tr("ui.dialog.modify.expand")
            enabled: AppDoc.commandAvailability["select.modify"].enabled === true
            onTriggered: menuRoot.modifyRequested("expand")
        }
        MenuItem {
            text: AppI18n.tr("ui.dialog.modify.contract")
            enabled: AppDoc.commandAvailability["select.modify"].enabled === true
            onTriggered: menuRoot.modifyRequested("contract")
        }
        MenuItem {
            text: AppI18n.tr("ui.dialog.modify.feather")
            enabled: AppDoc.commandAvailability["select.modify"].enabled === true
            onTriggered: menuRoot.modifyRequested("feather")
        }
    }
    MenuItem {
        visible: menuRoot.selectionMode
        text: AppI18n.tr("ui.menu.select.save_selection")
        enabled: AppDoc.commandAvailability["select.saveSelection"].enabled === true
        onTriggered: AppDoc.saveSelectionAsChannel()
    }
    Menu {
        id: filtersSubmenu
        title: AppI18n.tr("ui.menu.filter")
        Repeater {
            model: menuRoot.filterKinds
            MenuItem {
                required property string modelData
                text: AppI18n.tr(modelData)
                // The six live kinds share ONE availability row (14a).
                enabled: AppDoc.commandAvailability["filter.apply"].enabled === true
                onTriggered: menuRoot.filterRequested(modelData)
            }
        }
    }

    // ---- no-selection section ----
    MenuItem {
        visible: !menuRoot.selectionMode
        text: AppI18n.tr("ui.menu.edit.paste")
        enabled: AppDoc.commandAvailability["edit.paste"].enabled === true
        onTriggered: menuRoot.pasteRequested()
    }
    MenuItem {
        visible: !menuRoot.selectionMode
        text: AppI18n.tr("ui.menu.select.all")
        enabled: AppDoc.commandAvailability["select.all"].enabled === true
        onTriggered: AppDoc.applyCommand("selection.all", {}, {}, "")
    }
    MenuSeparator {
        visible: !menuRoot.selectionMode
                 && (menuRoot.selectionTool || menuRoot.cropTool
                     || menuRoot.paintTool)
    }
    // Tool-keyed quick actions (the header table).
    MenuItem {
        visible: !menuRoot.selectionMode && menuRoot.selectionTool
        text: AppI18n.tr("ui.menu.select.load_saved")
        enabled: AppDoc.commandAvailability["select.loadSaved"].enabled === true
        onTriggered: AppDoc.loadSavedChannelSelection()
    }
    MenuItem {
        visible: !menuRoot.selectionMode && menuRoot.selectionTool
        text: AppI18n.tr("ui.menu.select.load_layer_alpha")
        enabled: AppDoc.commandAvailability["select.loadLayerAlpha"].enabled === true
        onTriggered: AppDoc.loadLayerAlphaSelection(menuRoot.activeLayerId)
    }
    MenuItem {
        visible: !menuRoot.selectionMode && menuRoot.cropTool
        text: AppI18n.tr("ui.menu.image.crop")
        enabled: AppDoc.commandAvailability["image.crop"].enabled === true
        onTriggered: menuRoot.cropRequested()
    }
    MenuItem {
        visible: !menuRoot.selectionMode && menuRoot.cropTool
        text: AppI18n.tr("ui.menu.image.trim")
        enabled: AppDoc.commandAvailability["image.trim"].enabled === true
        onTriggered: menuRoot.trimRequested()
    }
    MenuItem {
        visible: !menuRoot.selectionMode && menuRoot.paintTool
        text: AppI18n.tr("ui.menu.edit.fill")
        enabled: AppDoc.commandAvailability["edit.fill"].enabled === true
        onTriggered: menuRoot.fillRequested()
    }

    // Nested Menus surface in the parent as auto-created wrapper
    // MenuItems (item.subMenu points back at the Menu) — the wrappers
    // are not declarable, so their section visibility binds here (both
    // submenus belong to the selection-active section). AND: on this
    // Qt, an invisible menu item still occupies its layout slot (the
    // popup opened at full both-sections height and got clamped at the
    // window edge — measured first-hand), so every content item also
    // collapses to zero height while hidden; the popup then sizes to
    // the ACTIVE section alone.
    Component.onCompleted: {
        for (let i = 0; i < count; ++i) {
            const it = itemAt(i);
            if (!it)
                continue;
            if (it.subMenu)
                it.visible = Qt.binding(function() {
                    return menuRoot.selectionMode;
                });
            it.height = Qt.binding(function() {
                return it.visible ? it.implicitHeight : 0;
            });
        }
    }
}
