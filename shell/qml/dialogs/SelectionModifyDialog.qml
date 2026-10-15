// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: contracts/commands/selection.modify.yaml — params carry EXACTLY ONE
// of feather (number 0.1..=1000) | expand | contract | smooth | border
// (each integer 1..=500); requires an active selection
// (error.command.selection_required is the engine's typed answer — the
// caller additionally gates on AppDoc.selectionActive). Plan §2.3 UI row.

import QtQuick
import AppnameShell
import QtQuick.Controls
import QtQuick.Layouts

Dialog {
    id: root
    modal: true
    title: AppI18n.tr("ui.dialog.modify.title")
    anchors.centerIn: parent
    padding: 12

    property string errorText: ""

    // Wire param name per op; feather is the only non-integer arm.
    readonly property var ops: [
        { key: "ui.dialog.modify.feather",  param: "feather",  integer: false },
        { key: "ui.dialog.modify.expand",   param: "expand",   integer: true },
        { key: "ui.dialog.modify.contract", param: "contract", integer: true },
        { key: "ui.dialog.modify.smooth",   param: "smooth",   integer: true },
        { key: "ui.dialog.modify.border",   param: "border",   integer: true }
    ]

    onOpened: errorText = ""

    // v1.0.1 item 7: the context menu's Modify ▸ submenu opens THIS
    // dialog preset to the chosen op (wire param name). The apply path
    // stays the dialog's own Apply — one implementation,
    // behavior-identical to Select > Modify….
    function openFor(param) {
        for (let i = 0; i < ops.length; ++i) {
            if (ops[i].param === param) {
                opBox.currentIndex = i;
                break;
            }
        }
        open();
    }

    // Explicit per-arm validators for the shared amount SpinBox (never
    // QQuickSpinBox's DEFAULT IntValidator, which silently eats the '.'
    // keystroke on the feather arm — "7.3" typed as "73", committed as
    // feather 73.0: the ResizeDialog D3 10x trap). Ranges are the
    // CONTRACT's (selection.modify.yaml): feather number 0.1..=1000 at
    // one decimal (the 10x fixed-point display step); the integer ops
    // 1..=500. Dot-decimal only ("C" locale) — valueFromText parses
    // with Number(), exactly the ResizeDialog pattern.
    IntValidator { id: intArmValidator; bottom: 1; top: 500 }
    DoubleValidator { id: featherValidator; bottom: 0.1; top: 1000
        decimals: 1; locale: "C" }

    ColumnLayout {
        spacing: 10

        RowLayout {
            spacing: 8
            Label { text: AppI18n.tr("ui.dialog.modify.op") }
            ComboBox {
                id: opBox
                Layout.preferredWidth: 160
                textRole: "label"
                model: root.ops.map(o => ({ label: AppI18n.tr(o.key), param: o.param, integer: o.integer }))
                // A11Y-3: §31.1 label association.
                Accessible.name: AppI18n.tr("ui.dialog.modify.op")
            }
        }

        RowLayout {
            spacing: 8
            Label { text: AppI18n.tr("ui.dialog.modify.amount") }
            // Integer arms: 1..=500. Feather: 0.1..=1000 — SpinBox is
            // integer-valued; feather rides a 10x fixed-point mapping so
            // sub-pixel feather stays reachable (0.1 steps).
            SpinBox {
                id: amount
                readonly property bool integerArm: opBox.currentIndex >= 0
                    && root.ops[opBox.currentIndex].integer
                from: integerArm ? 1 : 1        // 1 -> 0.1 px feather
                to: integerArm ? 500 : 10000    // 10000 -> 1000.0 px
                value: integerArm ? 1 : 10
                stepSize: integerArm ? 1 : 5
                editable: true
                validator: integerArm ? intArmValidator : featherValidator
                textFromValue: function(value) {
                    return integerArm ? String(value) : (value / 10.0).toFixed(1);
                }
                valueFromText: function(text) {
                    const n = Number(text);
                    if (!isFinite(n))
                        return value;
                    return integerArm ? Math.round(n) : Math.round(n * 10);
                }
                Accessible.name: AppI18n.tr("ui.dialog.modify.amount")
            }
        }

        Label {
            visible: root.errorText !== ""
            text: root.errorText
            color: Theme.warnText
            wrapMode: Text.WordWrap
            Layout.maximumWidth: 260
        }

        RowLayout {
            Layout.alignment: Qt.AlignRight
            spacing: 8
            Button {
                text: AppI18n.tr("ui.dialog.cancel")
                onClicked: root.close()
            }
            Button {
                text: AppI18n.tr("ui.dialog.modify.apply")
                enabled: AppDoc.selectionActive && opBox.currentIndex >= 0
                onClicked: {
                    const op = root.ops[opBox.currentIndex];
                    const params = {};
                    params[op.param] = op.integer ? amount.value : amount.value / 10.0;
                    const r = AppDoc.applyCommand("selection.modify", {}, params, "");
                    if (r.ok)
                        root.close();
                    else
                        root.errorText = r.code + ": " + r.message;
                }
            }
        }
    }
}
