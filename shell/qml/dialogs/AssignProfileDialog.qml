// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: contracts/commands/doc.assign_profile.yaml — params.profile is
// type [string,null]: a resolvable profile uuid, or JSON null = untagged.
// The five builtin working-space ids are content-independent UUIDv5
// constants under the engine namespace (mechanically derived and
// engine-confirmed; assign!=convert per DD (7)). Embedded-ICC enumeration
// waits on info v2 (ABI-growth request 3, reports/O-UI-log.md).

import QtQuick
import AppnameShell
import QtQuick.Controls
import QtQuick.Layouts

Dialog {
    id: root
    modal: true
    title: AppI18n.tr("ui.dialog.assign_profile.title")
    anchors.centerIn: parent
    padding: 12
    property string errorText: ""
    onOpened: errorText = ""

    // null = untagged (wire JSON null, not a uuid).
    readonly property var profiles: [
        { key: "ui.profile.srgb", id: "7ac4a286-7940-525f-805a-dc79e0f10fd8" },
        { key: "ui.profile.display_p3", id: "ee12374b-5f25-51fd-a1c8-f45790a61d92" },
        { key: "ui.profile.prophoto_rgb", id: "d299c05b-5f85-592c-965e-bd6f98f0a191" },
        // 1.0 final (corrigendum (l)): the two gray profiles are removed;
        // every 1.0 document is RGB, and the engine refuses a gray
        // profile on it (error.color.profile_space_mismatch, measured).
        { key: "ui.profile.untagged", id: null }
    ]

    ColumnLayout {
        spacing: 8
        RowLayout {
            Label { text: AppI18n.tr("ui.dialog.assign_profile.profile") }
            ComboBox {
                id: profile
                Layout.preferredWidth: 210
                model: root.profiles.map(p => AppI18n.tr(p.key))
                // A11Y-3: §31.1 label association.
                Accessible.name: AppI18n.tr("ui.dialog.assign_profile.profile")
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
                    const r = AppDoc.applyCommand("doc.assign_profile", {},
                        { profile: root.profiles[profile.currentIndex].id }, "");
                    if (r.ok) root.close(); else root.errorText = r.code + ": " + r.message;
                }
            }
        }
    }
}
