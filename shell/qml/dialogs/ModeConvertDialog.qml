// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: contracts/commands/doc.mode_convert.yaml — to (4 modes), intent
// (4 rendering intents, default relative), flatten (default false);
// class C (whole-document step; the cataloged class-C note shows).
// cmyk_a SUBMITS and surfaces the engine's typed refusal honestly (the
// recorded wide-door substrate gap — never disabled-faked here, the
// engine owns the wall). A11Y-1 substrate slice.

import QtQuick
import AppnameShell
import QtQuick.Controls
import QtQuick.Layouts

Dialog {
    id: root
    modal: true
    title: AppI18n.tr("ui.dialog.mode_convert.title")
    anchors.centerIn: parent
    padding: 12

    property string errorText: ""

    readonly property var modes: [
        { key: "ui.mode.rgba", wire: "rgba" },
        { key: "ui.mode.gray_a", wire: "gray_a" },
        { key: "ui.mode.cmyk_a", wire: "cmyk_a" },
        { key: "ui.mode.lab_a", wire: "lab_a" }
    ]
    readonly property var intents: [
        { key: "ui.intent.relative", wire: "relative" },
        { key: "ui.intent.perceptual", wire: "perceptual" },
        { key: "ui.intent.saturation", wire: "saturation" },
        { key: "ui.intent.absolute", wire: "absolute" }
    ]

    onOpened: errorText = ""

    ColumnLayout {
        spacing: 10

        RowLayout {
            spacing: 8
            Label { text: AppI18n.tr("ui.menu.image.mode") }
            ComboBox {
                id: modeBox
                Layout.preferredWidth: 180
                model: root.modes.map(m => AppI18n.tr(m.key))
                // A11Y-3: §31.1 label association.
                Accessible.name: AppI18n.tr("ui.menu.image.mode")
            }
        }
        RowLayout {
            spacing: 8
            Label { text: AppI18n.tr("ui.dialog.convert_profile.intent") }
            ComboBox {
                id: intentBox
                Layout.preferredWidth: 180
                model: root.intents.map(i => AppI18n.tr(i.key))
                Accessible.name: AppI18n.tr("ui.dialog.convert_profile.intent")
            }
        }
        CheckBox {
            id: flattenBox
            text: AppI18n.tr("ui.dialog.mode_convert.flatten")
            checked: false
        }
        // Class C: undo restores a snapshot (cataloged note, shared id).
        Label {
            text: AppI18n.tr("ui.dialog.class_c_note")
            font.pixelSize: 10
            color: Theme.textSecondary
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
                text: AppI18n.tr("ui.dialog.apply")
                onClicked: {
                    const params = {
                        "to": root.modes[modeBox.currentIndex].wire,
                        "flatten": flattenBox.checked
                    };
                    // Intent omitted at the default keeps the engine's
                    // default authoritative (the sentinel rule).
                    const intent = root.intents[intentBox.currentIndex].wire;
                    if (intent !== "relative")
                        params["intent"] = intent;
                    const r = AppDoc.applyCommand("doc.mode_convert", {}, params, "");
                    if (r.ok)
                        root.close();
                    else
                        root.errorText = r.code + (r.message ? ": " + r.message : "");
                }
            }
        }
    }
}
