// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: plan §2.3 row — layers panel v2. LIVE visibility + opacity via
// layer.set_props (class M: mergeKey "{kind}:{targetId}:{sessionUuid}
// #{propPath}", built format-exact for the §10.5 merge machinery — which
// the ENGINE DOES NOT CONSUME YET, so the slider commits ONCE on release
// to keep a drag one history step today). Add/delete/duplicate/group/
// ungroup/merge-down/rename per their contracts. Reorder requires an
// explicit parent uuid — known only for shell-created documents
// (AppDoc.rootLayerId from the doc.new receipt; EMPTY for opened docs
// until info v2, ABI-growth request 3 — controls disable rather than
// guess). The flat info surface cannot express nesting or masks, and
// blend/fillOpacity/locks are write-only (no readback): those controls
// stay DISABLED with the milestone tooltip — never faked (request 3
// heals all of it).

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
// The dialogs/ directory import — Main.qml has carried it since it was
// the only file instantiating a dialog. This panel now owns one too.
import "dialogs"

Rectangle {
    id: panel
    color: Theme.bgSurface
    radius: 4

    readonly property string infoV2Tip: AppI18n.tr(
        "ui.panel.layers.milestone_tooltip",
        {"kind": "layer readback"})
    property var checkedIds: [] // multi-select set for Group

    function activeRow() {
        const rows = AppDoc.layers;
        const idx = AppDoc.activeLayerIndex;
        return (idx >= 0 && idx < rows.length) ? rows[idx] : null;
    }
    // v1.0.1 mask route: mask.add against the ACTIVE LAYER. The engine
    // owns every precondition this panel cannot honestly pre-compute —
    // the occupied raster slot above all (the info mask object carries no
    // `kind`, so "the raster slot is taken" is not knowable here) — and
    // its typed refusal reaches the status bar through applyCommand.
    // §9.6: newMaskId is CLIENT-generated and needs no readback.
    function addMask(from) {
        const r = panel.activeRow();
        if (r === null)
            return;
        AppDoc.applyCommand("mask.add", {"layerId": r.layerId},
                            {"kind": "raster", "from": from,
                             "newMaskId": AppDoc.newUuid()}, "");
    }
    // v1.0.1 fill route: layer.add{kind:'fill'} with a §7.3 FillContent
    // payload in kindParams. The gradient envelopes below are the DEMO
    // DOCUMENT'S OWN (demo/build-demo.jsonl lines 4-5) rather than
    // invented here — that file is a probe of this door that has already
    // been run, so the wire shape is measured, not guessed.
    //
    // §22.1 colour wire shape: model + EXACTLY FOUR comps, the trailing
    // comp zero for rgb. That is the same convention envelope_builder.cpp
    // :173-182 already writes for the fill command; a fifth or a
    // three-component array is a schema refusal, not a rounding issue.
    function addGradientFill(gradientKind, space) {
        const black = {"model": "rgb", "comps": [0, 0, 0, 0]};
        const white = {"model": "rgb", "comps": [1, 1, 1, 0]};
        const gradient = {"kind": gradientKind,
                          "stops": [{"pos": 0, "color": black},
                                    {"pos": 1, "color": white}]};
        if (space !== "")
            gradient.space = space;
        const newId = AppDoc.newUuid();
        const r = AppDoc.applyCommand("layer.add", {},
                            {"kind": "fill",
                             "newLayerId": newId,
                             "kindParams": {"type": "gradient",
                                            "gradient": gradient}}, "");
        // Select what we created — same reason addAdjustmentLayer does:
        // the info refresh only clamps an out-of-range index, so a layer
        // inserted above leaves the previous row selected.
        if (r.ok !== true)
            return;
        AppDoc.selectLayerById(newId);
        // COMPOSITE, DO NOT COVER. A gradient fill layer is opaque and
        // lands top-of-stack, so at blend `normal` it hides the image
        // completely — the demo document ships its two fills HIDDEN for
        // exactly this reason, and a menu that produces a blank frame
        // reads as broken rather than as correct-but-opaque.
        //
        // `overlay` is the graduated-filter idiom: the black end darkens,
        // the white end lightens, and the photograph stays visible
        // through it. layer.add carries no blend or opacity of its own
        // (its schema has kind/parent/index/name/kindParams/newLayerId
        // only), so this is a second command — and it is deliberately
        // NOT silent about that: it is one user action producing two
        // journal entries, which the History panel will show.
        //
        // OPACITY is changeable from this panel afterwards (:379 is a real
        // Slider). SO IS BLEND, since f7f48bfd (Q-D2(a)): the row below is a
        // real ComboBox over the contract's 27 modes, submitting
        // layer.set_props exactly as the opacity row does.
        //
        // THIS COMMENT SAID THE OPPOSITE UNTIL 2026-08-29 01:2x, and the
        // sentence it replaces was TRUE WHEN WRITTEN. f7f48bfd corrected the
        // twin comment AT the blend row and this one, three hundred lines
        // away in the same file, went on asserting the old world — which is
        // the identical failure the paragraph below already records against
        // an earlier version of itself. A comment that has now been wrong
        // TWICE, in opposite directions, about the SAME control:
        // do not describe a sibling control from memory. Read it, or say
        // nothing about it.
        //
        // An earlier version of this comment claimed "the blend dropdown
        // and the opacity slider already exist". It was half true, written
        // by this desk, and it CONTRADICTED :451 in the same file — and it
        // is the sentence a demo step was then written against.
        AppDoc.applyCommand("layer.set_props", {"layerId": newId},
                            {"blend": "overlay", "opacity": 1.0},
                            "layer.set_props:" + newId + ":"
                                + AppDoc.newUuid() + "#fillcomposite");
    }
    function reorderParentOf(row) {
        return row.parentId && row.parentId !== "" ? row.parentId
                                                   : AppDoc.rootLayerId;
    }
    function siblingsOf(row) {
        const rows = AppDoc.layers;
        const parent = row.parentId || "";
        const out = [];
        for (let i = 0; i < rows.length; ++i)
            if ((rows[i].parentId || "") === parent)
                out.push(rows[i].layerId);
        return out;
    }
    function siblingIndexOf(row) {
        return siblingsOf(row).indexOf(row.layerId);
    }
    function siblingCountOf(row) {
        return siblingsOf(row).length;
    }

    // ★ ONE IMPLEMENTATION, TWO DOORS (G-17 / Q-D72; operator WORD 8 of
    //   2026-09-14 18:48 ET, answer (3)). The Layer menu in Main.qml is a
    //   KEYBOARD door onto these same six operations.
    //
    //   WHY IT IS SHAPED THIS WAY AND NOT AS SIX MENU HANDLERS: every guard and
    //   every argument below is a `panel.*` expression -- activeRow(),
    //   checkedIds, reorderParentOf(), siblingIndexOf(), layerList.count -- none
    //   of which a Menu in Main.qml can reach. Copying them there would ship TWO
    //   implementations of six guards, and they would drift on the first change.
    //   The panel's own buttons call these functions too, so if one of these is
    //   wrong BOTH doors are wrong together -- which is the property that lets a
    //   test of one stand for the other, and is exactly what G-17 asks for.
    readonly property bool canDelete: activeRow() !== null && layerList.count > 1
    readonly property bool canGroup: checkedIds.length >= 1
    readonly property bool canUngroup: activeRow() !== null
    readonly property bool canMergeDown: activeRow() !== null && AppDoc.activeLayerIndex > 0
    // 1.0 final F-9 (O-WATCH, native): the sibling bounds live HERE, not only
    // on the panel buttons. Layer > Move Up/Down and Ctrl+]/Ctrl+[ read these
    // same properties; with the bounds only on the buttons, one layer left the
    // menu items enabled -- Move Up recorded a no-op reorder and Move Down
    // surfaced the raw "index must be >= 0" refusal.
    readonly property bool canMoveUp: activeRow() !== null
        && reorderParentOf(activeRow()) !== ""
        && siblingIndexOf(activeRow()) < siblingCountOf(activeRow()) - 1
    readonly property bool canMoveDown: activeRow() !== null
        && reorderParentOf(activeRow()) !== ""
        && siblingIndexOf(activeRow()) > 0

    // ⛔ EACH RE-CHECKS ITS OWN GUARD. A disabled menu item cannot be
    //   triggered by a mouse, but a Shortcut fires regardless of any item's
    //   enabled state -- so the guard has to live where the work is, not only
    //   where the label is.
    function doDelete() {
        if (!canDelete)
            return;
        AppDoc.applyCommand("layer.delete", {"layerId": activeRow().layerId}, {}, "");
    }
    function doGroup() {
        if (!canGroup)
            return;
        AppDoc.applyCommand("layer.group", {},
                            {"layerIds": checkedIds, "newGroupId": AppDoc.newUuid()}, "");
    }
    function doUngroup() {
        if (!canUngroup)
            return;
        AppDoc.applyCommand("layer.ungroup", {"layerId": activeRow().layerId}, {}, "");
    }
    function doMergeDown() {
        if (!canMergeDown)
            return;
        AppDoc.applyCommand("layer.merge_down", {"layerId": activeRow().layerId}, {}, "");
    }
    function doMoveUp() {
        if (!canMoveUp)
            return;
        const r = activeRow();
        AppDoc.applyCommand("layer.reorder", {"layerId": r.layerId},
                            {"parent": reorderParentOf(r),
                             "index": siblingIndexOf(r) + 1}, "");
    }
    function doMoveDown() {
        if (!canMoveDown)
            return;
        const r = activeRow();
        AppDoc.applyCommand("layer.reorder", {"layerId": r.layerId},
                            {"parent": reorderParentOf(r),
                             "index": siblingIndexOf(r) - 1}, "");
    }
    function toggleChecked(layerId) {
        const i = checkedIds.indexOf(layerId);
        let next = checkedIds.slice();
        if (i >= 0)
            next.splice(i, 1);
        else
            next.push(layerId);
        checkedIds = next;
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 6
        spacing: 4

        Label {
            text: AppI18n.tr("panel.layers.title")
            font.bold: true
            color: Theme.textPrimary
        }

        Label {
            visible: layerList.count === 0
            text: AppI18n.tr("panel.layers.empty")
            color: Theme.textSecondary
        }

        ListView {
            id: layerList
            objectName: "layerList"
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: AppDoc.layers
            // Bottom-to-top: index 0 (background) sits at the panel bottom,
            // matching document z-order.
            verticalLayoutDirection: ListView.BottomToTop
            spacing: 2
            // A11Y-3: rows are treeitems (the layer stack nests), so the
            // container reads as a tree.
            Accessible.role: Accessible.Tree

            // KEYBOARD (C-752). This list was MOUSE-ONLY: the row's
            // MouseArea was the only route to selection, so a keyboard user
            // -- and every screen-reader user, who navigates by keyboard --
            // could not change layers at all, while this very object
            // ADVERTISES Accessible.role Tree with selectable rows.
            //
            // ONE TAB STOP, ARROWS INSIDE: that is the tree/listbox
            // convention and it is DELIBERATELY NOT HistoryPanel's shape
            // (activeFocusOnTab + Return/Space on each ROW). A history list
            // is a flat run of steps; tabbing through every layer would put
            // the whole stack in the tab order between unrelated controls.
            // The divergence is stated because two panels in one pane with
            // different keyboard models is a thing a reader should see
            // argued rather than discover.
            activeFocusOnTab: true
            // THE ENGINE OWNS SELECTION, so currentIndex is a ONE-WAY mirror
            // of AppDoc.activeLayerIndex and ListView's own key navigation is
            // OFF: it assigns currentIndex directly, which would BREAK that
            // binding, and the panel would then drift from document truth the
            // next time selection changed from anywhere else.
            keyNavigationEnabled: false
            currentIndex: AppDoc.activeLayerIndex
            onActiveFocusChanged: {
                if (!activeFocus && currentItem)
                    currentItem.focus = true;
            }
            // BottomToTop (above): index 0 is the BACKGROUND and sits at the
            // panel's BOTTOM, so Up is +1. Reading the direction off the
            // layout rather than assuming screen order is the whole reason
            // this pair is not inverted.
            Keys.onUpPressed: layerList.stepActive(1)
            Keys.onDownPressed: layerList.stepActive(-1)
            function stepActive(d) {
                const n = AppDoc.layers.length;
                if (n === 0)
                    return;
                const next = Math.max(0, Math.min(n - 1, AppDoc.activeLayerIndex + d));
                if (next !== AppDoc.activeLayerIndex)
                    AppDoc.activeLayerIndex = next;
            }

            delegate: Rectangle {
                id: row
                required property int index
                required property var modelData
                width: layerList.width
                height: 34
                radius: 3
                color: AppDoc.activeLayerIndex === index ? Theme.bgActive : Theme.bgRaised
                // A11Y-3: the §31.1 layer-row checklist — role treeitem,
                // name = layer name (document content, i18n-exempt),
                // selected = active row, checked = visible. `level` has
                // no QML attached-property surface (recorded gap in the
                // lane log; a C++ bridge interface is the later-wave
                // path) — depth stays visual indent until then.
                Accessible.role: Accessible.TreeItem
                Accessible.name: row.modelData.name
                Accessible.selectable: true
                Accessible.selected: AppDoc.activeLayerIndex === row.index
                Accessible.checkable: true
                Accessible.checked: row.modelData.visible === true

                MouseArea {
                    anchors.fill: parent
                    onClicked: AppDoc.activeLayerIndex = row.index
                    onDoubleClicked: {
                        nameEdit.text = row.modelData.name;
                        nameEdit.visible = true;
                        nameEdit.forceActiveFocus();
                    }
                }

                RowLayout {
                    anchors.fill: parent
                    // Tree truth: indent by v2 depth (0 = child of root).
                    anchors.leftMargin: 6 + 12 * (row.modelData.depth || 0)
                    anchors.rightMargin: 6
                    spacing: 6

                    // Multi-select for Group (subtle).
                    CheckBox {
                        id: groupSelect
                        padding: 0
                        scale: 0.75
                        // A11Y-3 wave B adoption (un-rosters this row).
                        Accessible.name: AppI18n.tr("ui.panel.layers.select_toggle")
                        checked: panel.checkedIds.indexOf(row.modelData.layerId) >= 0
                        onClicked: panel.toggleChecked(row.modelData.layerId)
                    }

                    // P3 (ABI 1.4): per-layer content thumbnail via
                    // appname_doc_read_layer. CONTENT, not contribution —
                    // a thumbnail does not fade with layer opacity
                    // (header contract). Typed refusals from the 1.4
                    // service matrix (adjustment layers, kinds without a
                    // composite path) yield NO pixmap: the row keeps its
                    // kind glyphs, never a faked preview. The
                    // infoRevision suffix busts Qt's per-URL cache
                    // exactly when info truth moves.
                    Rectangle {
                        visible: AppDoc.abiBlock14
                        implicitWidth: 30
                        implicitHeight: 24
                        radius: 2
                        color: Theme.bgWindow
                        Image {
                            anchors.centerIn: parent
                            source: "image://layerthumb/" + row.modelData.layerId
                                    + "@" + AppDoc.infoRevision
                            sourceSize: Qt.size(28, 22)
                        }
                    }

                    // LIVE visibility: a real eye (P2 icons). Same
                    // set_props session + truth re-bind discipline (R10);
                    // the icon renders the MODEL state, so no optimistic
                    // flip is possible.
                    ToolButton {
                        // Roster anchor (review D4); name queued
                        // (ui.panel.layers.visibility_toggle).
                        id: visibilityToggle
                        padding: 2
                        implicitWidth: 26
                        implicitHeight: 26
                        display: AbstractButton.IconOnly
                        icon.source: "image://lucide/"
                            + (row.modelData.visible ? "eye" : "eye-off")
                        icon.width: 15
                        icon.height: 15
                        // Hidden state dims to textSecondary, NOT
                        // textDisabled: this is an ENABLED toggle whose
                        // tint signals layer state, so it owes the 3:1
                        // UI-glyph bar on the ACTIVE row ground —
                        // textDisabled measured 2.18:1 there (review
                        // P2-D1); textSecondary clears every ground
                        // (3.36:1 raw on bgActive; pinned in
                        // tst_theme_contrast).
                        icon.color: row.modelData.visible ? Theme.textPrimary
                                                          : Theme.textSecondary
                        // A11Y-3 wave B: the long-queued adoption — the
                        // cataloged name landed (74fd5e3). Checkbox
                        // semantics: checked mirrors the MODEL state
                        // (§31.1 toggle row).
                        Accessible.name: AppI18n.tr("ui.panel.layers.visibility_toggle")
                        Accessible.role: Accessible.CheckBox
                        Accessible.checkable: true
                        Accessible.checked: row.modelData.visible === true
                        onClicked: AppDoc.applyCommand("layer.set_props",
                            {"layerId": row.modelData.layerId},
                            {"visible": !row.modelData.visible},
                            "layer.set_props:" + row.modelData.layerId + ":"
                                + AppDoc.newUuid() + "#visible")
                    }

                    // Group + mask glyphs (P2 icons): folder for groups,
                    // contrast (circle-half) + count for masks. The group
                    // tag keeps its cataloged accessible name AND an
                    // explicit StaticText role — a bare Image defaults to
                    // NoRole, which platform bridges don't expose, so the
                    // name alone would be silent (review P2-D2). The mask
                    // glyph has NO cataloged id yet (ui.panel.layers.
                    // mask_badge is minted-dormant): its accessible name
                    // is queued in the lane log with the eye's a11y rows
                    // rather than key-echoed to screen readers.
                    Image {
                        visible: row.modelData.kind === "group"
                        source: "image://lucide/folder"
                        sourceSize: Qt.size(14, 14)
                        Accessible.role: Accessible.StaticText
                        Accessible.name: AppI18n.tr("ui.panel.layers.group_tag")
                    }
                    Image {
                        visible: (row.modelData.maskIds || []).length > 0
                        source: "image://lucide/contrast"
                        sourceSize: Qt.size(13, 13)
                        // A11Y-3 wave B: the queued mask textual signal —
                        // explicit StaticText role (bare Image = NoRole,
                        // silent to bridges — the P2-D2 lesson) with the
                        // count-carrying cataloged name.
                        Accessible.role: Accessible.StaticText
                        Accessible.name: AppI18n.tr("ui.panel.layers.mask_tag",
                            {"count": (row.modelData.maskIds || []).length})
                    }
                    // Mask badge: the cataloged D6 template landed at
                    // O-I18N (locales control any affix), so the badge
                    // now shows for EVERY masked row — restoring the
                    // textual mask signal single-masked rows lost when
                    // the glyph replaced the text tag (review P2-D2).
                    Label {
                        visible: (row.modelData.maskIds || []).length > 0
                        text: AppI18n.tr("ui.panel.layers.mask_badge",
                                         {"count": (row.modelData.maskIds || []).length})
                        font.pixelSize: 10
                        color: Theme.badge
                        // A11Y-3 wave B: the glyph's mask_tag name carries
                        // the count — this visual badge repeating it to AT
                        // would double-speak (§31.3).
                        Accessible.ignored: true
                    }

                    Label {
                        visible: !nameEdit.visible
                        text: row.modelData.name
                        elide: Text.ElideRight
                        color: Theme.textPrimary
                        Layout.fillWidth: true
                    }
                    TextField {
                        id: nameEdit
                        visible: false
                        Layout.fillWidth: true
                        // A11Y-3 wave B adoption (un-rosters this row).
                        Accessible.name: AppI18n.tr("ui.panel.layers.rename_field")
                        // A11Y-4: explicit boundary (the Basic default is
                        // sub-bar borderLight, and the Basic FOCUSED arm
                        // is palette.highlight = bgActive — invisible on
                        // this row's own bgActive ground). This field is
                        // always focused while visible (forceActiveFocus
                        // on show, hides on focus loss), so the rendered
                        // outline is the focus.ring arm: 4.03:1 on
                        // bgActive (pinned).
                        background: Rectangle {
                            implicitHeight: 24
                            color: Theme.bgSurface
                            border.width: nameEdit.activeFocus ? 2 : 1
                            border.color: nameEdit.activeFocus
                                          ? Theme.focusRing
                                          : Theme.borderInput
                        }
                        onAccepted: {
                            const t = text.trim();
                            // Unicode scalar count, not UTF-16 units (R14).
                            const n = Array.from(t).length;
                            if (n >= 1 && n <= 255)
                                AppDoc.applyCommand("layer.rename",
                                    {"layerId": row.modelData.layerId}, {"name": t}, "");
                            visible = false;
                        }
                        onActiveFocusChanged: if (!activeFocus) visible = false
                    }
                }
            }
        }

        // Active-layer opacity: one class-M session per drag (sessionUuid
        // minted at press, engine coalesces the drag to one history step).
        RowLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: panel.activeRow() !== null
            Label {
                text: AppI18n.tr("ui.panel.layers.opacity")
                color: Theme.textSecondary
                font.pixelSize: 11
            }
            Slider {
                id: opacitySlider
                Layout.fillWidth: true
                from: 0
                to: 1
                value: 1
                // A11Y-3: associate the visible caption (§31.1 slider row;
                // the platform reads value/min/max from the control).
                Accessible.name: AppI18n.tr("ui.panel.layers.opacity")
                property bool dragDirty: false
                function submit(v) {
                    const row = panel.activeRow();
                    if (!row)
                        return;
                    AppDoc.applyCommand("layer.set_props",
                        {"layerId": row.layerId}, {"opacity": v},
                        "layer.set_props:" + row.layerId + ":"
                            + AppDoc.newUuid() + "#opacity");
                }
                // Commit ONCE per drag on release (one history step today —
                // §10.5 merge machinery pending engine-side); keyboard/wheel
                // steps commit per change, each with a fresh session (R4/R8).
                onPressedChanged: {
                    if (!pressed && dragDirty) {
                        submit(value);
                        dragDirty = false;
                    }
                }
                onMoved: {
                    if (pressed)
                        dragDirty = true;
                    else
                        submit(value);
                }
                // Explicit truth sync (a user drag breaks a value: binding
                // permanently — R10): follow the active row when idle.
                Connections {
                    target: AppDoc
                    function onInfoChanged() {
                        const row = panel.activeRow();
                        if (row && !opacitySlider.pressed)
                            opacitySlider.value = row.opacity;
                    }
                    function onActiveLayerIndexChanged() {
                        const row = panel.activeRow();
                        if (row && !opacitySlider.pressed)
                            opacitySlider.value = row.opacity;
                    }
                }
            }
            Label {
                text: AppI18n.tr("ui.value.percent",
                                 {"value": Math.round(opacitySlider.value * 100)})
                color: Theme.textSecondary
                font.pixelSize: 11
            }
        }

        // Blend mode: EDITABLE as of 2026-08-29 (operator ruling Q-D2(a)).
        //
        // ★ THE BLOCKER THIS COMMENT USED TO STATE WAS STALE, AND IT WAS
        // STALE IN A DIRECTION THAT KEPT A CONTROL OFF THE PANEL. The
        // previous text said the editor stays absent because "the 27-name
        // catalog family is the next O-I18N coordination item". MEASURED
        // 2026-08-29 before writing a line of this: all 27 ui.blend.* keys
        // exist in ALL FOUR locales (en/es/ja/pseudo), 27/27 each. The
        // catalog landed and nothing here noticed, so a note about future
        // work went on justifying a gap after the work was done.
        //
        // NO ENGINE WORK: layer.set_props already carries the full blend
        // enum, and the sibling opacity row below already submits that
        // very command. This row is the same shape deliberately.
        RowLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: panel.activeRow() !== null
            Label {
                text: AppI18n.tr("ui.panel.layers.blend")
                color: Theme.textSecondary
                font.pixelSize: 11
            }
            ComboBox {
                id: blendCombo
                objectName: "layerBlendCombo"
                Layout.fillWidth: true
                Accessible.name: AppI18n.tr("ui.panel.layers.blend")
                font.pixelSize: 11
                // WIRE ORDER, NOT ALPHABETICAL. This is the contract's own
                // enum order (layer.set_props.yaml), which groups the
                // families a user expects to find together — darken group,
                // lighten group, contrast group. Sorting by the TRANSLATED
                // name would give a different order per locale and scatter
                // those groups, so the model is the wire and only the
                // LABEL is translated.
                readonly property var tags: [
                    "normal", "dissolve", "darken", "multiply", "colorBurn",
                    "linearBurn", "darkerColor", "lighten", "screen",
                    "colorDodge", "linearDodge", "lighterColor", "overlay",
                    "softLight", "hardLight", "vividLight", "linearLight",
                    "pinLight", "hardMix", "difference", "exclusion",
                    "subtract", "divide", "hue", "saturation", "color",
                    "luminosity"
                ]
                // camelCase wire tag -> snake_case catalog key. The two
                // spellings are a real seam: the enum is camelCase and the
                // i18n keys are snake_case, so "colorBurn" must reach
                // "ui.blend.color_burn". Derived, never a second table --
                // a hand-written 27-row map is the two-copy problem, and
                // it would drift the first time the enum grows.
                function keyFor(tag) {
                    return "ui.blend."
                        + tag.replace(/[A-Z]/g, function (c) {
                            return "_" + c.toLowerCase();
                        });
                }
                model: tags.map(function (t) { return AppI18n.tr(keyFor(t)); })
                function syncFromTruth() {
                    const row = panel.activeRow();
                    if (!row || popup.visible)
                        return;
                    const i = tags.indexOf(row.blend);
                    // An UNKNOWN tag is left alone rather than coerced to
                    // 0: index 0 is "normal", so a silent fallback would
                    // DISPLAY normal for a document that is not -- the
                    // exact lie the read-only Label was rewritten to stop
                    // telling. §2.8/§4.5 admit unknown blend strings, so
                    // this is a reachable state, not a defensive nicety.
                    if (i >= 0)
                        currentIndex = i;
                }
                onActivated: {
                    const row = panel.activeRow();
                    if (!row)
                        return;
                    AppDoc.applyCommand("layer.set_props",
                        {"layerId": row.layerId}, {"blend": tags[index]},
                        "layer.set_props:" + row.layerId + ":"
                            + AppDoc.newUuid() + "#blend");
                }
                // Explicit truth sync, same reason as the opacity slider's
                // (R10): a user selection breaks a currentIndex: binding
                // permanently, so follow the active row from signals.
                Connections {
                    target: AppDoc
                    function onInfoChanged() { blendCombo.syncFromTruth(); }
                    function onActiveLayerIndexChanged() { blendCombo.syncFromTruth(); }
                }
                Component.onCompleted: syncFromTruth()
            }
        }
        // Static caption (disabled QQC2 controls receive no hover events,
        // so tooltips on them never fire — R7): one line covers the
        // blend combo, group-duplicate, and opened-document reorder.
        Label {
            Layout.fillWidth: true
            visible: !AppDoc.abiBlock12 // info v2 landed: caption is a pre-1.2 note (review D7)
            text: panel.infoV2Tip
            wrapMode: Text.WordWrap
            font.pixelSize: 10
            color: Theme.textSecondary
        }

        // Action strip. Every submit's ok/rejection reaches the status bar
        // through applyCommand; typed engine refusals (e.g. duplicate of a
        // masked/group subtree without the full idMap) surface honestly.
        GridLayout {
            Layout.fillWidth: true
            uniformCellWidths: true
            columns: 3
            columnSpacing: 4
            rowSpacing: 4

            Button {
                Layout.fillWidth: true
                text: AppI18n.tr("ui.panel.layers.add")
                font.pixelSize: 11
                enabled: AppDoc.hasDocument
                onClicked: AppDoc.applyCommand("layer.add", {},
                    {"kind": "raster", "newLayerId": AppDoc.newUuid()}, "")
            }
            // 1.0 final (corrigendum (l), O-WATCH F-8): Grp+ is removed. With
            // no 1.0 control that puts a layer into a group, it only made
            // empty groups.
            // Pro-Wave row 1: an adjustment layer, masked by the selection
            // when there is one (the engine's OP-mask route — adjustment.add
            // then op.set_mask{from:'selection'}; see
            // DocumentController::addAdjustmentLayer for the route and its
            // honest limit). Enabled truth is the 14a
            // layer.addAdjustment row — document-gated, NOT selection-gated:
            // with nothing selected this correctly creates a plain unmasked
            // adjustment rather than refusing.
            //
            // CAPTION: "adjustment.invert" — an EXISTING id, in both the
            // frozen v5.5 registry and all four catalogs. Invert is the one
            // registered adjustment that takes no params, so it is the whole
            // adjustment family this button can honestly offer today; a kind
            // PICKER needs captions that do not exist yet (queued to
            // O-I18N with the two history labels — lane log). No id is
            // invented here and nothing ships disabled-and-unexplained.
            Button {
                Layout.fillWidth: true
                text: AppI18n.tr("ui.panel.layers.add_adjustment")
                font.pixelSize: 11
                id: adjustButton
                objectName: "adjustButton"
                enabled: AppDoc.commandAvailability["layer.addAdjustment"].enabled === true
                onClicked: adjustMenu.open()
                // v1.0.1 item 4 class (a). Until now this button submitted
                // adjustment.invert on click and nothing else — the shell
                // wired ONE adjustment kind while the engine registered
                // eight (C-432; DEMO-SCRIPT.md:57 claimed two and was
                // wrong — `posterize` reached the UI only from a test's
                // envelope builder).
                //
                // invert stays a PLAIN ACTION because it has zero
                // properties: opening a dialog with no rows to fill would
                // be chrome pretending there is a decision. The three
                // served kinds open the param dialog; curves, levels and
                // hue_saturation are absent because their payloads are
                // structured (control-point arrays, per-channel tuples,
                // colour bands) and the generic door cannot express them.
                Menu {
                    id: adjustMenu
                    y: adjustButton.height
                    MenuItem {
                        objectName: "adjustMenuInvert"
                        text: AppI18n.tr("adjustment.invert")
                        onTriggered: AppDoc.addAdjustmentLayer("adjustment.invert")
                    }
                    MenuItem {
                        objectName: "adjustMenuBrightnessContrast"
                        text: AppI18n.tr("adjustment.brightness_contrast")
                        onTriggered: adjustDialog.openFor("adjustment.brightness_contrast")
                    }
                    MenuItem {
                        objectName: "adjustMenuExposure"
                        text: AppI18n.tr("adjustment.exposure")
                        onTriggered: adjustDialog.openFor("adjustment.exposure")
                    }
                    MenuItem {
                        objectName: "adjustMenuLevels"
                        text: AppI18n.tr("adjustment.levels")
                        onTriggered: adjustDialog.openFor("adjustment.levels")
                    }
                    MenuItem {
                        objectName: "adjustMenuCurves"
                        text: AppI18n.tr("adjustment.curves")
                        onTriggered: adjustDialog.openFor("adjustment.curves")
                    }
                    MenuItem {
                        objectName: "adjustMenuHueSaturation"
                        text: AppI18n.tr("adjustment.hue_saturation")
                        onTriggered: adjustDialog.openFor("adjustment.hue_saturation")
                    }
                    MenuItem {
                        objectName: "adjustMenuThreshold"
                        text: AppI18n.tr("adjustment.threshold")
                        onTriggered: adjustDialog.openFor("adjustment.threshold")
                    }
                }
            }
            // v1.0.1: THE LAYER-MASK ROUTE. DEMO-SCRIPT.md:78 recorded
            // "there is no mask.add route in the shell QML at all — the
            // shell can display and composite a masked adjustment but
            // cannot yet CREATE the mask". This button is that route.
            //
            // A MENU rather than five buttons: the `from` modes are one
            // decision with five answers, and the action strip is a
            // 3-column grid. The two selection-derived modes carry their
            // OWN availability row because mask.add REQUIRES an active
            // selection for them (error.command.selection_required) while
            // the other three do not — one row for both would disable
            // Reveal All whenever nothing was selected, which the engine
            // serves perfectly well.
            //
            // `path` (the vector-mask mode) is NOT offered: it needs
            // kind:'vector' plus a path uuid that must resolve, i.e. a
            // path picker. Offering it without one would ship a control
            // that can only refuse.
            // v1.0.1: NEW FILL LAYER. The engine registers the whole
            // gradient family and the demo document ships two fill
            // layers; until now nothing in the UI could make one.
            //
            // BLACK-TO-WHITE, NOT A COLOUR PICKER, and the limit is
            // stated rather than hidden: the fg/bg swatch pair lives on
            // ToolPalette (paletteRoot.swatchBackgroundColor and the
            // canvas item's brushColor) and this panel is instantiated
            // by Main.qml with no properties bound, so reaching those
            // colours needs a Main.qml wire. Shipping a ramp the user
            // cannot recolour is a smaller lie than shipping a colour
            // control that silently ignores their swatches — and the
            // three entries below each make a DIFFERENT engine
            // parameter visible (kind, then space), which is what the
            // menu is for.
            //
            // `solid`, `pattern` and `none` FillContent arms are not
            // offered: solid needs the colour this panel cannot see,
            // pattern needs a patternRef with no picker, and `none`
            // would create an invisible layer.
            Button {
                id: fillButton
                objectName: "fillButton"
                Layout.fillWidth: true
                text: AppI18n.tr("ui.panel.layers.add_fill")
                font.pixelSize: 11
                enabled: AppDoc.commandAvailability["layer.addFill"].enabled === true
                onClicked: fillMenu.open()
                Menu {
                    id: fillMenu
                    y: fillButton.height
                    MenuItem {
                        objectName: "fillMenuLinear"
                        text: AppI18n.tr("ui.panel.layers.fill_gradient_linear")
                        onTriggered: panel.addGradientFill("linear", "")
                    }
                    MenuItem {
                        objectName: "fillMenuPerceptual"
                        text: AppI18n.tr("ui.panel.layers.fill_gradient_perceptual")
                        onTriggered: panel.addGradientFill("linear", "perceptual")
                    }
                    MenuItem {
                        objectName: "fillMenuRadial"
                        text: AppI18n.tr("ui.panel.layers.fill_gradient_radial")
                        onTriggered: panel.addGradientFill("radial", "")
                    }
                }
            }
            Button {
                id: maskButton
                objectName: "maskButton"
                Layout.fillWidth: true
                text: AppI18n.tr("ui.panel.layers.add_mask")
                font.pixelSize: 11
                enabled: AppDoc.commandAvailability["layer.addMask"].enabled === true
                onClicked: maskMenu.open()
                Menu {
                    id: maskMenu
                    y: maskButton.height
                    MenuItem {
                        objectName: "maskMenuRevealAll"
                        text: AppI18n.tr("ui.panel.layers.mask_reveal_all")
                        onTriggered: panel.addMask("reveal_all")
                    }
                    MenuItem {
                        objectName: "maskMenuHideAll"
                        text: AppI18n.tr("ui.panel.layers.mask_hide_all")
                        onTriggered: panel.addMask("hide_all")
                    }
                    MenuItem {
                        objectName: "maskMenuFromSelection"
                        text: AppI18n.tr("ui.panel.layers.mask_from_selection")
                        enabled: AppDoc.commandAvailability["layer.addMaskFromSelection"].enabled === true
                        onTriggered: panel.addMask("selection")
                    }
                    MenuItem {
                        objectName: "maskMenuFromSelectionInverted"
                        text: AppI18n.tr("ui.panel.layers.mask_from_selection_inverted")
                        enabled: AppDoc.commandAvailability["layer.addMaskFromSelection"].enabled === true
                        onTriggered: panel.addMask("selection_inverted")
                    }
                    MenuItem {
                        objectName: "maskMenuFromTransparency"
                        text: AppI18n.tr("ui.panel.layers.mask_from_transparency")
                        onTriggered: panel.addMask("transparency")
                    }
                }
            }
            Button {
                Layout.fillWidth: true
                text: AppI18n.tr("ui.panel.layers.remove")
                font.pixelSize: 11
                enabled: panel.canDelete
                onClicked: panel.doDelete()
            }
            Button {
                Layout.fillWidth: true
                text: AppI18n.tr("ui.panel.layers.duplicate")
                font.pixelSize: 11
                // info v2 makes the idMap enumerable over layers + masks
                // (op-stack entry ids are NOT in info v2 — upstream gap,
                // lane log; layers carrying op stacks reject typed). Group
                // rows need v2 parentId chains: pre-1.2 they stay disabled
                // (review D5).
                enabled: panel.activeRow() !== null
                         && (panel.activeRow().kind !== "group" || AppDoc.abiBlock12)
                onClicked: {
                    const rows = AppDoc.layers;
                    const target = panel.activeRow();
                    const inSubtree = {};
                    inSubtree[target.layerId] = true;
                    let grew = true;
                    while (grew) {
                        grew = false;
                        for (let i = 0; i < rows.length; ++i) {
                            const r = rows[i];
                            if (!inSubtree[r.layerId] && inSubtree[r.parentId]) {
                                inSubtree[r.layerId] = true;
                                grew = true;
                            }
                        }
                    }
                    const idMap = {};
                    for (let i = 0; i < rows.length; ++i) {
                        const r = rows[i];
                        if (!inSubtree[r.layerId])
                            continue;
                        idMap[r.layerId] = AppDoc.newUuid();
                        const masks = r.maskIds || [];
                        for (let m = 0; m < masks.length; ++m)
                            idMap[masks[m]] = AppDoc.newUuid();
                    }
                    AppDoc.applyCommand("layer.duplicate",
                        {"layerId": target.layerId},
                        {"idMap": idMap, "insertAbove": true}, "");
                }
            }
            // 1.0 final (corrigendum (l)): the Grp+ and Ungroup buttons are
            // removed (layer groups return with the 1.1 Layers panel).
            Button {
                Layout.fillWidth: true
                text: AppI18n.tr("ui.panel.layers.merge_down")
                font.pixelSize: 11
                enabled: panel.canMergeDown
                onClicked: panel.doMergeDown()
            }
            // Reorder needs the parent uuid — root known only for
            // shell-created docs (doc.new receipt); opened docs disable
            // until info v2 (request 3).
            RowLayout {
                spacing: 2
                // Reorder is SIBLING-relative per contract: parent = the
                // row's own parent (v2 parentId; rootLayerId fallback for
                // shell-created flat docs pre-1.2), index counts within
                // that parent's children — NOT the flat info index, which
                // silently re-parents nested rows to root (review D4).
                Button {
                    Layout.fillWidth: true
                    text: AppI18n.tr("ui.panel.layers.up")
                    font.pixelSize: 11
                    enabled: panel.canMoveUp
                    onClicked: panel.doMoveUp()
                }
                Button {
                    Layout.fillWidth: true
                    text: AppI18n.tr("ui.panel.layers.down")
                    font.pixelSize: 11
                    enabled: panel.canMoveDown
                    onClicked: panel.doMoveDown()
                }
            }
        }
    }

    // v1.0.1 item 4 class (a): the param dialog lives with the control
    // that opens it. LayersPanel is instantiated by Main.qml with no
    // properties bound, so a dialog parented here needs no wire through
    // a file this desk does not own this week.
    AdjustmentParamDialog {
        id: adjustDialog
        parent: Overlay.overlay
    }

}
