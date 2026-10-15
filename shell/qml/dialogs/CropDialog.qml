// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: contracts/commands/doc.crop.yaml — rect{x,y int; w,h int >=1}
// required; deletePixels default false; straightenAngleDeg -45..=45
// default 0; resample default bicubic; allowGrowth default false
// (rect-intersects-canvas is live-state-checked unless allowGrowth).
// openWithRect(x,y,w,h) pre-fills from the canvas crop tool drag.

import QtQuick
import AppnameShell
import QtQuick.Controls
import QtQuick.Layouts

Dialog {
    id: root
    modal: true
    title: AppI18n.tr("ui.dialog.crop.title")
    anchors.centerIn: parent
    padding: 12
    property string errorText: ""
    onOpened: errorText = ""

    function openWithRect(x, y, w, h) {
        rx.value = x; ry.value = y; rw.value = Math.max(1, w); rh.value = Math.max(1, h);
        open();
    }

    readonly property var kernels: ["nearest", "bilinear", "bicubic",
        "bicubic_smoother", "bicubic_sharper", "lanczos"]

    ColumnLayout {
        spacing: 8
        GridLayout {
            columns: 4
            // A11Y-3: §31.1 label associations (each adjacent Label's id).
            Label { text: AppI18n.tr("ui.dialog.crop.x") }
            SpinBox { id: rx; from: -99999; to: 99999; value: 0; editable: true; Accessible.name: AppI18n.tr("ui.dialog.crop.x") }
            Label { text: AppI18n.tr("ui.dialog.crop.y") }
            SpinBox { id: ry; from: -99999; to: 99999; value: 0; editable: true; Accessible.name: AppI18n.tr("ui.dialog.crop.y") }
            Label { text: AppI18n.tr("ui.dialog.crop.width") }
            SpinBox { id: rw; from: 1; to: 1000000; value: AppDoc.docWidth; editable: true; Accessible.name: AppI18n.tr("ui.dialog.crop.width") }
            Label { text: AppI18n.tr("ui.dialog.crop.height") }
            SpinBox { id: rh; from: 1; to: 1000000; value: AppDoc.docHeight; editable: true; Accessible.name: AppI18n.tr("ui.dialog.crop.height") }
        }
        // v1.0.1 item 5: while the non-destructive arm is typed-NI
        // (§19.1 crop_rect pending), Apply must not discover it —
        // default ON, toggle disabled with the inline milestone note;
        // everything re-enables automatically when the controller
        // truth-point flips (the crop_rect capability signal).
        Switch {
            id: deletePixels
            text: AppI18n.tr("ui.dialog.crop.delete_pixels")
            // RC3 D-5 (operator WORD 35, sitting S-17): ON BY DEFAULT.
            // Measured natively: with the capability live, the old binding
            // (!cropNonDestructiveLive) opened OFF, so Apply set only the
            // non-destructive output frame -- the document stayed 640x480
            // (status bar, Canvas Size) and read as "Crop did nothing".
            // ON makes Apply produce a document of the cropped size; the
            // user can still untick for the non-destructive crop, and the
            // WIRE default is unchanged (the dialog sends the key either way).
            // While the capability is dead, ON is also the only servable arm.
            checked: true
            enabled: AppDoc.cropNonDestructiveLive
        }
        Label {
            visible: !AppDoc.cropNonDestructiveLive
            text: AppI18n.tr("ui.panel.layers.milestone_tooltip",
                             {"kind": "non-destructive crop"})
            wrapMode: Text.WordWrap
            font.pixelSize: 11
            color: Theme.textSecondary
            Layout.maximumWidth: 280
        }
        RowLayout {
            Label { text: AppI18n.tr("ui.dialog.crop.straighten") }
            SpinBox {
                id: straighten; from: -450; to: 450; value: 0; editable: true
                textFromValue: v => (v / 10.0).toFixed(1)
                valueFromText: t => { const n = Number(t); return isFinite(n) ? Math.round(n * 10) : value; }
                Accessible.name: AppI18n.tr("ui.dialog.crop.straighten")
            }
        }
        RowLayout {
            Label { text: AppI18n.tr("ui.dialog.resize.resample") }
            ComboBox {
                id: resample
                Layout.preferredWidth: 170
                currentIndex: 2 // bicubic
                model: root.kernels.map(k => AppI18n.tr("ui.resample." + k))
                Accessible.name: AppI18n.tr("ui.dialog.resize.resample")
            }
        }
        Switch { id: allowGrowth; text: AppI18n.tr("ui.dialog.crop.allow_growth") }
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
                    const r = AppDoc.applyCommand("doc.crop", {}, {
                        rect: { x: rx.value, y: ry.value, w: rw.value, h: rh.value },
                        deletePixels: deletePixels.checked,
                        straightenAngleDeg: straighten.value / 10.0,
                        resample: root.kernels[resample.currentIndex],
                        allowGrowth: allowGrowth.checked
                    }, "");
                    if (r.ok) root.close(); else root.errorText = r.code + ": " + r.message;
                }
            }
        }
    }
}
