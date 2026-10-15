// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: contracts/commands/doc.new.yaml (width/height 1..=1_000_000,
// background enum; mode/depth ride their contract defaults at CP-0.3).
// Wire enum values never translate; their labels do.
//
// v1.0.1 items 2+3: presets + px/in/cm/mm units are PURE UI over the
// UNCHANGED doc.new door — whatever unit is displayed, Create submits
// the ROUNDED INT PX the computed row shows (units.h math; dpi feeds
// the physical conversions only). Custom is the default; picking a
// preset sets w/h/unit/dpi; EDITING a dimension flips the picker back
// to Custom one-way (never fight the user). Unit switches convert the
// displayed dims through the current DPI, preserving the px meaning.

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Dialog {
    id: dialog
    modal: true
    anchors.centerIn: parent
    title: AppI18n.tr("ui.dialog.new.title")
    padding: 16

    readonly property var backgroundValues: ["white", "transparent", "black"]
    readonly property var unitKeys: ["px", "in", "cm", "mm"]
    readonly property var presetList: AppUnits.presets
    property string unitKey: "px"

    // The rounded ints the computed row shows AND Create submits — one
    // truth, no re-derivation at accept time. Unparseable text reads as
    // 0 (out of the door's 1..=1_000_000 range, so Create disables).
    readonly property int widthPx: pxOf(widthField.text)
    readonly property int heightPx: pxOf(heightField.text)
    readonly property bool dimsOk: widthField.acceptableInput
        && heightField.acceptableInput
        && widthPx >= 1 && widthPx <= 1000000
        && heightPx >= 1 && heightPx <= 1000000

    function pxOf(text) {
        const px = AppUnits.pxFromUnit(Number(text), unitKey, dpiBox.value);
        // Cap the DOUBLE before the int-property coercion: a conversion
        // past 2^32 would otherwise ToInt32-wrap (14316558.09 in @300 =
        // 2^32+131 -> widthPx 131) and OPEN the Create gate on an absurd
        // request. 2e6 sits safely past the door's 1e6 ceiling, so
        // dimsOk stays false for everything out of range.
        return isFinite(px) ? Math.round(Math.min(px, 2000000)) : 0;
    }

    // px shows whole pixels; physical units show up to 3 decimals
    // (trailing zeros stripped), matching the validator's precision.
    function fmt(v) {
        return unitKey === "px" ? String(Math.round(v))
                                : String(Number(v.toFixed(3)));
    }

    IntValidator { id: pxValidator; bottom: 1; top: 1000000 }
    DoubleValidator { id: physValidator; bottom: 0.001; decimals: 3
        locale: "C" } // Number() parses dot-decimal only (R13)

    GridLayout {
        columns: 3
        columnSpacing: 10
        rowSpacing: 8

        Label { text: AppI18n.tr("ui.dialog.new.preset") }
        ComboBox {
            id: presetBox
            Layout.columnSpan: 2
            Layout.fillWidth: true
            // Custom first + default (index 0); the six data presets
            // follow in units.cpp table order.
            model: [AppI18n.tr("ui.preset.custom")].concat(
                dialog.presetList.map(p => AppI18n.tr(p.nameKey)))
            // A11Y-3: §31.1 label association.
            Accessible.name: AppI18n.tr("ui.dialog.new.preset")
            onActivated: function(index) {
                if (index === 0)
                    return; // Custom = free entry; impose nothing
                const p = dialog.presetList[index - 1];
                dialog.unitKey = p.unit;
                unitBox.currentIndex = dialog.unitKeys.indexOf(p.unit);
                if (p.dpi > 0)
                    dpiBox.value = p.dpi; // px presets leave DPI alone
                widthField.text = dialog.fmt(p.width);
                heightField.text = dialog.fmt(p.height);
            }
        }

        Label { text: AppI18n.tr("ui.dialog.new.width") }
        TextField {
            id: widthField
            text: "1024"
            Layout.preferredWidth: 96
            validator: dialog.unitKey === "px" ? pxValidator : physValidator
            // One-way: a USER edit of a dimension is a departure from
            // the preset (textEdited never fires on programmatic sets,
            // so preset/unit writes don't loop back here).
            onTextEdited: presetBox.currentIndex = 0
            Accessible.name: AppI18n.tr("ui.dialog.new.width")
            // A11Y-4: explicit boundary — the Basic unfocused outline is
            // below the 1.4.11 bar (the ResizeDialog ppi precedent).
            background: Rectangle {
                implicitWidth: 96
                implicitHeight: 28
                color: Theme.bgSurface
                border.width: widthField.activeFocus ? 2 : 1
                border.color: widthField.activeFocus ? Theme.focusRing
                                                     : Theme.borderInput
            }
        }
        ComboBox {
            id: unitBox
            Layout.rowSpan: 2
            Layout.preferredWidth: 76
            model: dialog.unitKeys.map(u => AppI18n.tr("ui.unit." + u))
            // A11Y-3: the ResizeDialog driving-SpinBox pattern — the
            // visible label a sighted user reads for this control IS
            // its current unit symbol (the dialog carries no units
            // caption and the minted batch names none); the accessible
            // name mirrors exactly that, no new id.
            Accessible.name: currentText
            onActivated: function(index) {
                const newKey = dialog.unitKeys[index];
                if (newKey === dialog.unitKey)
                    return;
                // Convert the displayed dims through the CURRENT dpi so
                // the computed px is preserved; an unparseable field is
                // left as typed (nothing to convert honestly).
                const dpi = dpiBox.value;
                const wOk = widthField.acceptableInput;
                const hOk = heightField.acceptableInput;
                const wPx = AppUnits.pxFromUnit(Number(widthField.text),
                                                dialog.unitKey, dpi);
                const hPx = AppUnits.pxFromUnit(Number(heightField.text),
                                                dialog.unitKey, dpi);
                dialog.unitKey = newKey;
                if (wOk)
                    widthField.text = dialog.fmt(
                        AppUnits.unitFromPx(wPx, newKey, dpi));
                if (hOk)
                    heightField.text = dialog.fmt(
                        AppUnits.unitFromPx(hPx, newKey, dpi));
            }
        }

        Label { text: AppI18n.tr("ui.dialog.new.height") }
        TextField {
            id: heightField
            text: "768"
            Layout.preferredWidth: 96
            validator: dialog.unitKey === "px" ? pxValidator : physValidator
            onTextEdited: presetBox.currentIndex = 0
            Accessible.name: AppI18n.tr("ui.dialog.new.height")
            background: Rectangle {
                implicitWidth: 96
                implicitHeight: 28
                color: Theme.bgSurface
                border.width: heightField.activeFocus ? 2 : 1
                border.color: heightField.activeFocus ? Theme.focusRing
                                                      : Theme.borderInput
            }
        }

        Label { text: AppI18n.tr("ui.dialog.new.dpi") }
        SpinBox {
            id: dpiBox
            Layout.columnSpan: 2
            from: 1
            to: 9600
            value: 300
            editable: true
            Accessible.name: AppI18n.tr("ui.dialog.new.dpi")
        }

        Label { text: AppI18n.tr("ui.dialog.new.background") }
        ComboBox {
            id: backgroundBox
            Layout.columnSpan: 2
            Layout.fillWidth: true
            Accessible.name: AppI18n.tr("ui.dialog.new.background")
            model: [
                AppI18n.tr("ui.dialog.new.background.white"),
                AppI18n.tr("ui.dialog.new.background.transparent"),
                AppI18n.tr("ui.dialog.new.background.black")
            ]
        }

        // ALWAYS visible: the px truth the door will receive, whatever
        // unit is displayed above.
        Label {
            Layout.columnSpan: 3
            text: AppI18n.tr("ui.dialog.new.computed_px",
                             {"width": dialog.widthPx,
                              "height": dialog.heightPx})
        }
    }

    footer: DialogButtonBox {
        Button {
            text: AppI18n.tr("ui.dialog.cancel")
            DialogButtonBox.buttonRole: DialogButtonBox.RejectRole
        }
        Button {
            text: AppI18n.tr("ui.dialog.new.create")
            enabled: dialog.dimsOk
            DialogButtonBox.buttonRole: DialogButtonBox.AcceptRole
        }
    }

    // Standing priority (4)-3: THE REPLACEMENT IS THE GUARDED ACT, so
    // onAccepted ROUTES rather than acts. Gating the MENU ITEM instead
    // would prompt, then let the user cancel this dialog, having asked
    // them to approve a discard that never happened.
    //
    // The gate is injected rather than reached for: Main.qml sets
    // confirmGate, and the null fallback keeps this component correct
    // standalone -- several test harnesses instantiate it with no window
    // around it, and a component that only works inside one parent is a
    // second dispatch path waiting to disagree (G-525).
    property var confirmGate: null

    onAccepted: {
        const create = function() {
            AppDoc.newDocument(widthPx, heightPx,
                               backgroundValues[backgroundBox.currentIndex]);
        };
        if (dialog.confirmGate) dialog.confirmGate(create);
        else create();
    }
}
