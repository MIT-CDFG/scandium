// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// B-12 — ROW 6's SHELL HALF: the §11.2 mask property panel.
//
// WHY IT EXISTS NOW AND NOT BEFORE. Row 6's engine half and its
// `mask.set_props` door shipped on lane/ui-row6, but the panel was HELD
// on i18n: `Catalog::tr` key-echoes an uncataloged id verbatim, so
// shipping these controls would have printed "ui.panel.mask.feather" at
// a user. A-05 minted all seven ids on 2026-09-05 in four locales, the
// tripwire in tst_mask_feather fired exactly as designed, and Amendment
// 3 re-scoped the shell half as B-12. Nothing here invents a string.
//
// THE BINDING SOURCE IS THE PROPERTY, NOT THE ACCESSOR. `masksOf()` is
// Q_INVOKABLE and imperative; `AppDoc.layers` is a Q_PROPERTY on
// NOTIFY infoChanged and already carries each row's `masks` inline
// (document_controller's layer projection). Binding the property is
// reactive BY CONSTRUCTION — an imperative call would need a manual
// refresh on every engine receipt and would go stale the first time
// someone forgot one.
//
// SCOPE, STATED RATHER THAN IMPLIED: this panel edits the layer's FIRST
// mask. `mask.set_props` targets one maskId and the projection returns a
// LIST, so a layer with two masks shows only the first here. That is a
// real limit, not an oversight — a mask selector is its own row and is
// not in B-12's DONE-WHEN. The controls hide entirely when the active
// layer has no mask, which is the common case.
//
// RANGES ARE THE CONTRACT'S, READ FIRST-HAND (mask.set_props.yaml):
// density 0..1, feather 0..1000 px (Gaussian sigma, §11.2), and three
// booleans. No presentation ceiling is invented for feather — the
// contract pins one, so the control uses it.

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Rectangle {
    id: maskPanel
    color: Theme.bgSurface
    radius: 4

    // The active layer row, by the LayersPanel idiom (that panel is the
    // precedent for "which layer is active" and there must not be two
    // answers in one shell).
    function activeRow() {
        const rows = AppDoc.layers;
        const idx = AppDoc.activeLayerIndex;
        return (idx >= 0 && idx < rows.length) ? rows[idx] : null;
    }
    // The mask under edit, or null. See the SCOPE note: first mask only.
    function activeMask() {
        const row = activeRow();
        if (!row || !row.masks || row.masks.length === 0)
            return null;
        return row.masks[0];
    }
    readonly property var mask: activeMask()

    // ONE writer for all five controls. Every property rides the same
    // door with the same refusal handling, so a control cannot acquire
    // its own quiet failure path.
    //
    // THE REFUSAL IS SURFACED, NOT SWALLOWED. setMaskProp refuses
    // client-side with ok:false and a CATALOGED code
    // (ui.error.bad_argument, minted A-05) for an unknown property or a
    // missing id; engine refusals arrive through applyCommand's own
    // status path. An unhandled false here would be a control that does
    // nothing and says nothing — the exact defect Q-D14 was raised about.
    function write(prop, value, sessionUuid) {
        const row = activeRow();
        const m = activeMask();
        if (!row || !m)
            return;
        const r = AppDoc.setMaskProp(row.layerId, m.maskId, prop, value,
                                     sessionUuid || "");
        if (r && r.ok === false && r.code)
            AppDoc.statusPending(r.code, {});
    }

    // CONTENT-SIZED, not a share of the pane. With no mask this collapses
    // to its heading; with one it asks for exactly the five rows it draws.
    // Taking a fixed fraction instead is what broke tst_chord_table's
    // rename case: shrinking LayersPanel moved the row its double-click
    // targets, and a geometry test is right to catch that.
    implicitHeight: maskCol.implicitHeight + 12 // anchors.margins 6, both ends

    ColumnLayout {
        id: maskCol
        anchors.fill: parent
        anchors.margins: 6
        spacing: 4

        Label {
            text: AppI18n.tr("ui.panel.mask.title")
            font.bold: true
            color: Theme.textPrimary
        }

        // Everything below is gated on there BEING a mask. A layer with
        // none shows the heading alone rather than five dead controls —
        // an armed-looking control that refuses is the shape this project
        // keeps removing.
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 4
            visible: maskPanel.mask !== null

            CheckBox {
                id: enabledBox
                objectName: "maskEnabledBox"
                text: AppI18n.tr("ui.panel.mask.enabled")
                // A11Y-3: the visible caption IS the accessible name; the
                // platform reads checked state from the control itself.
                Accessible.name: text
                onToggled: maskPanel.write("enabled", checked, "")
            }
            CheckBox {
                id: invertedBox
                objectName: "maskInvertedBox"
                text: AppI18n.tr("ui.panel.mask.inverted")
                Accessible.name: text
                onToggled: maskPanel.write("inverted", checked, "")
            }
            CheckBox {
                id: linkedBox
                objectName: "maskLinkedBox"
                text: AppI18n.tr("ui.panel.mask.linked")
                Accessible.name: text
                onToggled: maskPanel.write("linked", checked, "")
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: 6
                Label {
                    text: AppI18n.tr("ui.panel.mask.density")
                    color: Theme.textSecondary
                    font.pixelSize: 11
                }
                Slider {
                    id: densitySlider
                    objectName: "maskDensitySlider"
                    Layout.fillWidth: true
                    from: 0
                    to: 1
                    Accessible.name: AppI18n.tr("ui.panel.mask.density")
                    property bool dragDirty: false
                    // ONE class-M session per drag: the uuid is minted at
                    // commit and the engine coalesces the drag to a single
                    // history step (the contract's own words: "Properties-
                    // panel drags record per session"). Without it a drag
                    // writes one journal entry per mouse move.
                    function submit(v) {
                        maskPanel.write("density", v, AppDoc.newUuid());
                    }
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
                            submit(value); // keyboard/wheel: own session
                    }
                }
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: 6
                Label {
                    text: AppI18n.tr("ui.panel.mask.feather")
                    color: Theme.textSecondary
                    font.pixelSize: 11
                }
                Slider {
                    id: featherSlider
                    objectName: "maskFeatherSlider"
                    Layout.fillWidth: true
                    from: 0
                    to: 1000 // the CONTRACT's maximum, not a presentation guess
                    Accessible.name: AppI18n.tr("ui.panel.mask.feather")
                    property bool dragDirty: false
                    function submit(v) {
                        maskPanel.write("feather", v, AppDoc.newUuid());
                    }
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
                }
            }
        }
    }

    // EXPLICIT TRUTH SYNC (R10, the LayersPanel opacity precedent): a user
    // drag breaks a `value:` binding PERMANENTLY, and a checkbox's
    // `checked:` binding breaks on the first toggle the same way. So the
    // controls follow engine truth here rather than through bindings that
    // silently stop being bindings. Skipped while a slider is held, or an
    // in-flight receipt would yank the handle out from under the user.
    function syncFromEngine() {
        const m = maskPanel.mask;
        if (!m)
            return;
        enabledBox.checked = m.enabled === true;
        invertedBox.checked = m.inverted === true;
        linkedBox.checked = m.linked === true;
        if (!densitySlider.pressed)
            densitySlider.value = m.density;
        if (!featherSlider.pressed)
            featherSlider.value = m.feather;
    }
    Component.onCompleted: syncFromEngine()
    Connections {
        target: AppDoc
        function onInfoChanged() { maskPanel.syncFromEngine(); }
        function onActiveLayerIndexChanged() { maskPanel.syncFromEngine(); }
    }
}
