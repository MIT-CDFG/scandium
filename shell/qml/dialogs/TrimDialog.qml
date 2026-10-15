// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: contracts/commands/doc.trim.yaml — basis enum default transparent;
// top/bottom/left/right booleans default true; all-four-false is a
// contract cross-field error (confirm disabled here).

import QtQuick
import AppnameShell
import QtQuick.Controls
import QtQuick.Layouts

Dialog {
    id: root
    modal: true
    title: AppI18n.tr("ui.dialog.trim.title")
    anchors.centerIn: parent
    padding: 12
    property string errorText: ""
    onOpened: errorText = ""

    readonly property var bases: ["transparent", "top_left_color", "bottom_right_color"]

    ColumnLayout {
        spacing: 8
        RowLayout {
            Label { text: AppI18n.tr("ui.dialog.trim.basis") }
            ComboBox {
                id: basis
                Layout.preferredWidth: 190
                model: [AppI18n.tr("ui.trim.basis.transparent"),
                        AppI18n.tr("ui.trim.basis.top_left"),
                        AppI18n.tr("ui.trim.basis.bottom_right")]
                // A11Y-3: §31.1 label association.
                Accessible.name: AppI18n.tr("ui.dialog.trim.basis")
            }
        }
        GridLayout {
            columns: 2
            Switch { id: top; text: AppI18n.tr("ui.trim.top"); checked: true }
            Switch { id: bottom; text: AppI18n.tr("ui.trim.bottom"); checked: true }
            Switch { id: left; text: AppI18n.tr("ui.trim.left"); checked: true }
            Switch { id: right; text: AppI18n.tr("ui.trim.right"); checked: true }
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
                         && (top.checked || bottom.checked || left.checked || right.checked)
                onClicked: {
                    const r = AppDoc.applyCommand("doc.trim", {}, {
                        basis: root.bases[basis.currentIndex],
                        top: top.checked, bottom: bottom.checked,
                        left: left.checked, right: right.checked
                    }, "");
                    if (r.ok) root.close(); else root.errorText = r.code + ": " + r.message;
                }
            }
        }
    }
}
