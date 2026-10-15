// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: contracts/commands/image.flatten.yaml — discardHidden bool default
// true; class C (whole-document snapshot; inverse snap) — the
// expensive-undo note is shown per the lane's undo-granularity survey.

import QtQuick
import AppnameShell
import QtQuick.Controls
import QtQuick.Layouts

Dialog {
    id: root
    modal: true
    title: AppI18n.tr("ui.dialog.flatten.title")
    anchors.centerIn: parent
    padding: 12
    property string errorText: ""
    onOpened: errorText = ""

    ColumnLayout {
        spacing: 8
        Switch {
            id: discardHidden
            text: AppI18n.tr("ui.dialog.flatten.discard_hidden")
            checked: true
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
                enabled: AppDoc.hasDocument
                onClicked: {
                    const r = AppDoc.applyCommand("image.flatten", {},
                        { discardHidden: discardHidden.checked }, "");
                    if (r.ok) root.close(); else root.errorText = r.code + ": " + r.message;
                }
            }
        }
    }
}
