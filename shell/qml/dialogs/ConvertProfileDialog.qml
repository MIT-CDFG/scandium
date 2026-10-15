// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: contracts/commands/doc.convert_profile.yaml — profile uuid
// required (null NOT legal here, unlike assign); intent enum default
// relative; blackPointCompensation default TRUE; dither default true;
// flatten OMITTED (runtime not_implemented at CP-0.5). Builtin ids as in
// AssignProfileDialog (derived + engine-confirmed).

import QtQuick
import AppnameShell
import QtQuick.Controls
import QtQuick.Layouts

Dialog {
    id: root
    modal: true
    title: AppI18n.tr("ui.dialog.convert_profile.title")
    anchors.centerIn: parent
    padding: 12
    property string errorText: ""
    onOpened: errorText = ""

    readonly property var profiles: [
        { key: "ui.profile.srgb", id: "7ac4a286-7940-525f-805a-dc79e0f10fd8" },
        { key: "ui.profile.display_p3", id: "ee12374b-5f25-51fd-a1c8-f45790a61d92" },
        { key: "ui.profile.prophoto_rgb", id: "d299c05b-5f85-592c-965e-bd6f98f0a191" }
        // 1.0 final (corrigendum (l)): the two gray profiles are removed;
        // every 1.0 document is RGB, and the engine refuses a gray
        // profile on it (error.color.profile_space_mismatch, measured).
    ]
    readonly property var intents: ["perceptual", "relative", "saturation", "absolute"]

    ColumnLayout {
        spacing: 8
        RowLayout {
            Label { text: AppI18n.tr("ui.dialog.convert_profile.profile") }
            ComboBox {
                id: profile
                Layout.preferredWidth: 210
                model: root.profiles.map(p => AppI18n.tr(p.key))
                // A11Y-3: §31.1 label association.
                Accessible.name: AppI18n.tr("ui.dialog.convert_profile.profile")
            }
        }
        RowLayout {
            Label { text: AppI18n.tr("ui.dialog.convert_profile.intent") }
            ComboBox {
                id: intent
                Layout.preferredWidth: 170
                currentIndex: 1 // relative (contract default)
                model: root.intents.map(i => AppI18n.tr("ui.intent." + i))
                Accessible.name: AppI18n.tr("ui.dialog.convert_profile.intent")
            }
        }
        Switch {
            id: bpc
            text: AppI18n.tr("ui.dialog.convert_profile.bpc")
            checked: true // contract default TRUE
        }
        Switch { id: dither; text: AppI18n.tr("ui.dialog.convert_profile.dither"); checked: true }
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
                    const r = AppDoc.applyCommand("doc.convert_profile", {}, {
                        profile: root.profiles[profile.currentIndex].id,
                        intent: root.intents[intent.currentIndex],
                        blackPointCompensation: bpc.checked,
                        dither: dither.checked
                    }, "");
                    if (r.ok) root.close(); else root.errorText = r.code + ": " + r.message;
                }
            }
        }
    }
}
