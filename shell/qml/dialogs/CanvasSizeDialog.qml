// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: contracts/commands/doc.canvas_resize.yaml — width,height integer
// required (negatives legal only in relative mode; result >=1x1 is
// live-state-checked engine-side); anchor 9-cell enum default center;
// relative default false; background enum default background_color.

import QtQuick
import AppnameShell
import QtQuick.Controls
import QtQuick.Layouts

Dialog {
    id: root
    modal: true
    title: AppI18n.tr("ui.dialog.canvas_size.title")
    anchors.centerIn: parent
    padding: 12
    property string errorText: ""
    onOpened: {
        errorText = "";
        if (!relative.checked) {
            w.value = AppDoc.docWidth;
            h.value = AppDoc.docHeight;
        }
    }

    readonly property var anchors9: ["top_left", "top", "top_right", "left",
        "center", "right", "bottom_left", "bottom", "bottom_right"]
    property int anchorIndex: 4
    readonly property var backgrounds: ["transparent", "background_color", "white", "black"]

    ColumnLayout {
        spacing: 8
        RowLayout {
            Label { text: AppI18n.tr("ui.dialog.canvas_size.width") }
            SpinBox {
                id: w; editable: true
                from: relative.checked ? -99999 : 1; to: 99999
                value: AppDoc.docWidth
                // A11Y-3: §31.1 label association.
                Accessible.name: AppI18n.tr("ui.dialog.canvas_size.width")
            }
            Label { text: AppI18n.tr("ui.dialog.canvas_size.height") }
            SpinBox {
                id: h; editable: true
                from: relative.checked ? -99999 : 1; to: 99999
                value: AppDoc.docHeight
                Accessible.name: AppI18n.tr("ui.dialog.canvas_size.height")
            }
        }
        Switch {
            id: relative
            text: AppI18n.tr("ui.dialog.canvas_size.relative")
            checked: false
            onToggled: { w.value = relative.checked ? 0 : AppDoc.docWidth;
                         h.value = relative.checked ? 0 : AppDoc.docHeight; }
        }
        Label { text: AppI18n.tr("ui.dialog.canvas_size.anchor") }
        GridLayout {
            columns: 3
            Repeater {
                model: 9
                Button {
                    required property int index
                    Layout.preferredWidth: 34
                    Layout.preferredHeight: 26
                    checkable: true
                    checked: root.anchorIndex === index
                    text: root.anchorIndex === index ? "●" : "·"
                    onClicked: root.anchorIndex = index
                    // A11Y-3 wave B: the glyph text is meaningless to AT
                    // — the cataloged parameterized name says which cell
                    // (1-based row/column).
                    Accessible.name: AppI18n.tr("ui.dialog.canvas_size.anchor_cell",
                        {"row": Math.floor(index / 3) + 1,
                         "column": (index % 3) + 1})
                }
            }
        }
        RowLayout {
            Label { text: AppI18n.tr("ui.dialog.canvas_size.background") }
            ComboBox {
                id: bg
                // A11Y-3: §31.1 label association.
                Accessible.name: AppI18n.tr("ui.dialog.canvas_size.background")
                Layout.preferredWidth: 180
                currentIndex: 1 // background_color (contract default)
                model: [AppI18n.tr("ui.bg.transparent"),
                        AppI18n.tr("ui.bg.background_color"),
                        AppI18n.tr("ui.bg.white"), AppI18n.tr("ui.bg.black")]
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
                    const r = AppDoc.applyCommand("doc.canvas_resize", {}, {
                        width: w.value, height: h.value,
                        anchor: root.anchors9[root.anchorIndex],
                        relative: relative.checked,
                        background: root.backgrounds[bg.currentIndex]
                    }, "");
                    if (r.ok) root.close(); else root.errorText = r.code + ": " + r.message;
                }
            }
        }
    }
}
