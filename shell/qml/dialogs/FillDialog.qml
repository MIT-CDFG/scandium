// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: contracts/commands/fill.yaml — content enum (foreground|background|
// color live at CP-0.5; gradient/pattern/content_aware are runtime
// not_implemented walls — shown DISABLED with the milestone tooltip, never
// faked); color = §22.1 wire {model:"rgb", comps:[r,g,b,0]} camelCase
// models; blend restricted to "normal" (non-normal fills are runtime
// not_implemented); opacity 0..=1 default 1; preserveTransparency default
// false; tolerance 0..=255 default 32; contiguous default true; antialias
// default true. Target = the active layer. sampleAllLayers omitted
// (runtime-refused).

import QtQuick
import AppnameShell
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Dialogs

Dialog {
    id: root
    modal: true
    title: AppI18n.tr("ui.dialog.fill.title")
    anchors.centerIn: parent
    padding: 12
    property string errorText: ""
    property color fillColor: "#202020"
    // tier-2 swatch-pair feed (ToolPalette header record): Main.qml
    // binds this to the foreground swatch state (canvas.brushColor);
    // each open seeds the color param from it, and the in-dialog
    // swatch still overrides for that open. The default keeps the
    // dialog's pre-pair standalone value.
    property color foregroundSeed: "#202020"
    onOpened: {
        errorText = "";
        fillColor = foregroundSeed;
    }

    // foreground/background CONTENT arms are engine-fixed headless
    // constants at CP-0.5 (no kind sets them) — R15: the shell's swatch
    // pair does NOT feed them, so offering these arms would fake app
    // state the engine cannot see; unifying needs an engine surface.
    // CORRECTION LOGGED (tier-2 rows slice): the earlier wording "app
    // color state that does not exist" is stale — the fg/bg swatch pair
    // now EXISTS as shell state and seeds this dialog's COLOR param
    // (foregroundSeed above); the R15 ruling on the content ARMS is
    // unchanged.
    readonly property var liveContents: ["color"]

    ColumnLayout {
        spacing: 8
        RowLayout {
            Label { text: AppI18n.tr("ui.dialog.fill.content") }
            ComboBox {
                id: content
                Layout.preferredWidth: 180
                currentIndex: 0
                model: [AppI18n.tr("ui.fill.content.color")]
                // A11Y-3: §31.1 label associations (this dialog's swatch
                // Rectangle stays a recorded wave-B gap with the toolbar
                // color well — same no-cataloged-name class).
                Accessible.name: AppI18n.tr("ui.dialog.fill.content")
            }
            // A11Y-3 wave B: a real focusable button (was Rectangle+
            // MouseArea — unreachable and role-less), named by the
            // DISTINCT cataloged id (O-I18N ruling: this is the fill's
            // content color, not app-foreground state). Boundary =
            // borderInput (A11Y-4 D6: a dark fill otherwise leaves a
            // 2.17:1 border as the sole component indicator).
            ToolButton {
                implicitWidth: 26
                implicitHeight: 26
                Accessible.name: AppI18n.tr("ui.dialog.fill.color_swatch")
                onClicked: fillColorDialog.open()
                contentItem: Rectangle {
                    radius: 3
                    color: root.fillColor
                    border.color: Theme.borderInput
                }
            }
        }
        // gradient / pattern / content_aware: engine answers
        // not_implemented at CP-0.5 — presented disabled, milestone 0.7.
        Label {
            text: AppI18n.tr("ui.panel.layers.milestone_tooltip",
                             {"kind": "gradient/pattern/content-aware"})
            font.pixelSize: 11
            color: Theme.textSecondary
        }
        RowLayout {
            Label { text: AppI18n.tr("ui.dialog.fill.blend") }
            ComboBox {
                enabled: false // non-normal blends are runtime walls at 0.5
                model: [AppI18n.tr("ui.blend.normal")]
                Layout.preferredWidth: 140
                Accessible.name: AppI18n.tr("ui.dialog.fill.blend")
            }
        }
        RowLayout {
            Label { text: AppI18n.tr("ui.dialog.fill.opacity") }
            Slider { id: opacity; from: 0; to: 1; value: 1; Layout.preferredWidth: 140; Accessible.name: AppI18n.tr("ui.dialog.fill.opacity") }
            Label { text: AppI18n.tr("ui.value.percent",
                                     {"value": Math.round(opacity.value * 100)}) }
        }
        Switch { id: preserve; text: AppI18n.tr("ui.dialog.fill.preserve_transparency") }
        RowLayout {
            Label { text: AppI18n.tr("ui.dialog.fill.tolerance") }
            SpinBox { id: tolerance; from: 0; to: 255; value: 32; editable: true; Accessible.name: AppI18n.tr("ui.dialog.fill.tolerance") }
            Switch { id: contiguous; text: AppI18n.tr("ui.dialog.fill.contiguous"); checked: true }
        }
        Switch { id: antialias; text: AppI18n.tr("ui.dialog.fill.antialias"); checked: true }
        Label {
            visible: root.errorText !== ""; text: root.errorText
            color: Theme.warnText; wrapMode: Text.WordWrap; Layout.maximumWidth: 300
        }
        RowLayout {
            Layout.alignment: Qt.AlignRight
            Button { text: AppI18n.tr("ui.dialog.cancel"); onClicked: root.close() }
            Button {
                text: AppI18n.tr("ui.dialog.apply")
                enabled: AppDoc.hasDocument
                onClicked: {
                    const rows = AppDoc.layers;
                    const idx = AppDoc.activeLayerIndex;
                    if (idx < 0 || idx >= rows.length)
                        return;
                    const params = {
                        content: root.liveContents[content.currentIndex],
                        blend: "normal",
                        opacity: opacity.value,
                        preserveTransparency: preserve.checked,
                        tolerance: tolerance.value,
                        contiguous: contiguous.checked,
                        antialias: antialias.checked
                    };
                    if (params.content === "color")
                        params.color = { model: "rgb",
                                         comps: [root.fillColor.r, root.fillColor.g,
                                                 root.fillColor.b, 0] };
                    const r = AppDoc.applyCommand("fill",
                        {"layerId": rows[idx].layerId}, params, "");
                    if (r.ok) root.close(); else root.errorText = r.code + ": " + r.message;
                }
            }
        }
    }

    ColorDialog {
        id: fillColorDialog
        selectedColor: root.fillColor
        onAccepted: root.fillColor = selectedColor
    }
}
