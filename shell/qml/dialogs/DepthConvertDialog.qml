// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: contracts/commands/doc.depth_convert.yaml — to enum u8|u16|f16|f32;
// dither default true (down-conversions). Runtime: u8/f32 storage-backed;
// u16 feature-gated, f16 paused — both answer not_implemented, so they
// render DISABLED with the milestone tooltip (never faked). Class C
// (whole-document snapshot) — expensive-undo note shown.

import QtQuick
import AppnameShell
import QtQuick.Controls
import QtQuick.Layouts

Dialog {
    id: root
    modal: true
    title: AppI18n.tr("ui.dialog.depth.title")
    anchors.centerIn: parent
    padding: 12
    property string errorText: ""
    onOpened: errorText = ""

    readonly property var depths: [
        // 1.0 final (corrigendum (l)): 16-bit integer and 16-bit float are
        // removed; the engine refuses both in 1.0 (not_implemented,
        // measured), so they were greyed-always choices.
        { key: "ui.depth.u8", to: "u8", live: true },
        { key: "ui.depth.f32", to: "f32", live: true }
    ]

    ColumnLayout {
        spacing: 8
        RowLayout {
            Label { text: AppI18n.tr("ui.dialog.depth.to") }
            ComboBox {
                id: depth
                Layout.preferredWidth: 150
                textRole: "label"
                // A11Y-3: §31.1 label association.
                Accessible.name: AppI18n.tr("ui.dialog.depth.to")
                model: root.depths.map(d => ({ label: AppI18n.tr(d.key), live: d.live }))
                delegate: ItemDelegate {
                    required property var model
                    required property int index
                    width: depth.width
                    text: model.label
                    enabled: root.depths[index].live
                }
            }
        }
        Switch { id: dither; text: AppI18n.tr("ui.dialog.depth.dither"); checked: true }
        // Static caption: disabled entries get no hover events (R7).
        Label {
            text: AppI18n.tr("ui.panel.layers.milestone_tooltip",
                             {"kind": "u16/f16"})
            font.pixelSize: 11
            color: Theme.textSecondary
        }
        Label {
            text: AppI18n.tr("ui.dialog.class_c_note")
            font.pixelSize: 11
            color: Theme.textSecondary
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
                enabled: AppDoc.hasDocument && root.depths[depth.currentIndex].live
                onClicked: {
                    const r = AppDoc.applyCommand("doc.depth_convert", {}, {
                        to: root.depths[depth.currentIndex].to,
                        dither: dither.checked
                    }, "");
                    if (r.ok) root.close(); else root.errorText = r.code + ": " + r.message;
                }
            }
        }
    }
}
