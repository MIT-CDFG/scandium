// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: contracts/commands/doc.rotate.yaml — angleDeg number -360..=360
// (multiples of 90 are lossless permutations); flipH/flipV default false;
// expand default true; resample 6-kernel enum default bicubic.

import QtQuick
import AppnameShell
import QtQuick.Controls
import QtQuick.Layouts

Dialog {
    id: root
    modal: true
    title: AppI18n.tr("ui.dialog.rotate.title")
    anchors.centerIn: parent
    padding: 12
    property string errorText: ""
    onOpened: errorText = ""

    readonly property var kernels: ["nearest", "bilinear", "bicubic",
        "bicubic_smoother", "bicubic_sharper", "lanczos"]

    ColumnLayout {
        spacing: 8
        RowLayout {
            Label { text: AppI18n.tr("ui.dialog.rotate.angle") }
            // Tenth-degree fixed point (SpinBox is integer-valued).
            SpinBox {
                id: angle; from: -3600; to: 3600; value: 900; stepSize: 150
                editable: true
                textFromValue: v => (v / 10.0).toFixed(1)
                valueFromText: t => { const n = Number(t); return isFinite(n) ? Math.round(n * 10) : value; }
                // A11Y-3: §31.1 label association.
                Accessible.name: AppI18n.tr("ui.dialog.rotate.angle")
            }
            Button { text: "90"; onClicked: angle.value = 900 }
            Button { text: "-90"; onClicked: angle.value = -900 }
            Button { text: "180"; onClicked: angle.value = 1800 }
        }
        RowLayout {
            Switch { id: flipH; text: AppI18n.tr("ui.dialog.rotate.flip_h") }
            Switch { id: flipV; text: AppI18n.tr("ui.dialog.rotate.flip_v") }
        }
        Switch { id: expand; text: AppI18n.tr("ui.dialog.rotate.expand"); checked: true }
        RowLayout {
            Label { text: AppI18n.tr("ui.dialog.resize.resample") }
            ComboBox {
                id: resample
                Layout.preferredWidth: 170
                currentIndex: 2 // bicubic (contract default)
                model: root.kernels.map(k => AppI18n.tr("ui.resample." + k))
                Accessible.name: AppI18n.tr("ui.dialog.resize.resample")
            }
        }
        Label {
            visible: root.errorText !== ""; text: root.errorText
            color: Theme.warnText; wrapMode: Text.WordWrap; Layout.maximumWidth: 280
        }
        RowLayout {
            Layout.alignment: Qt.AlignRight
            Button { text: AppI18n.tr("ui.dialog.cancel"); onClicked: root.close() }
            Button {
                text: AppI18n.tr("ui.dialog.apply")
                enabled: AppDoc.hasDocument
                onClicked: {
                    const r = AppDoc.applyCommand("doc.rotate", {}, {
                        angleDeg: angle.value / 10.0,
                        flipH: flipH.checked, flipV: flipV.checked,
                        expand: expand.checked,
                        resample: root.kernels[resample.currentIndex]
                    }, "");
                    if (r.ok) root.close(); else root.errorText = r.code + ": " + r.message;
                }
            }
        }
    }
}
