// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// v1.0.1 item 14b — the §32.5 named-object destructive CONFIRM for the
// Delete disposition's NO-SELECTION arm (desk ruling, items-7+14 design
// entry): without a selection, Delete clears the active layer's WHOLE
// content (§40.5) — the shell confirms first. The selection-active arm
// never opens this dialog (immediate).
//
// STRING DISCIPLINE (checked first-hand against i18n/en.json + the
// message-id registry): NO §32.5 delete-confirm ids exist. This dialog
// composes ONLY existing cataloged values — title = ui.menu.edit.delete
// ("Delete", the command's own cataloged name — the item-7 reuse rule)
// and the FlattenConfirmDialog button pair (ui.dialog.cancel /
// ui.dialog.apply). The NAMED OBJECT is the layer's name from info
// truth — document content, i18n-exempt (the export-basename
// precedent). RECORDED LIMITATION + MINT-QUEUE: a proper §32.5 message
// template (ui.dialog.delete_confirm.message = e.g. "Delete the
// contents of “{name}”?") is queued to O-I18N; until it
// lands the dialog names the object bare. Never invented, never faked.
//
// modal: true — the item-7 F4 lesson: a dialog's dismissal press must
// be CONSUMED, never leaked to the canvas tool underneath.

import QtQuick
import AppnameShell
import QtQuick.Controls
import QtQuick.Layouts

Dialog {
    id: root
    objectName: "deleteConfirmDialog"
    modal: true
    title: AppI18n.tr("ui.menu.edit.delete")
    anchors.centerIn: parent
    padding: 12

    property string targetLayerId: ""
    property string errorText: ""

    function layerNameFor(layerId) {
        const rows = AppDoc.layers;
        for (let i = 0; i < rows.length; ++i)
            if (rows[i].layerId === layerId)
                return rows[i].name;
        return "";
    }

    function openFor(layerId) {
        targetLayerId = layerId;
        errorText = "";
        open();
    }

    ColumnLayout {
        spacing: 8
        // The §32.5 named object: the layer's own name (info truth —
        // document content, i18n-exempt; see header).
        Label {
            objectName: "deleteConfirmObjectName"
            text: root.layerNameFor(root.targetLayerId)
            font.bold: true
        }
        Label {
            visible: root.errorText !== ""; text: root.errorText
            color: Theme.warnText; wrapMode: Text.WordWrap
            Layout.maximumWidth: 280
        }
        RowLayout {
            Layout.alignment: Qt.AlignRight
            Button {
                text: AppI18n.tr("ui.dialog.cancel")
                onClicked: root.close()
            }
            Button {
                objectName: "deleteConfirmApply"
                text: AppI18n.tr("ui.dialog.apply")
                enabled: AppDoc.commandAvailability["edit.delete"].enabled === true
                onClicked: {
                    // The bare door: edit.cut WITHOUT the app-side copy
                    // (desk ruling). A typed refusal (lock class) shows
                    // here AND on the status line (applyCommand's
                    // existing ui.status.rejected path).
                    const r = AppDoc.deleteLayerContent(root.targetLayerId);
                    if (r.ok)
                        root.close();
                    else
                        root.errorText = r.code + ": " + r.message;
                }
            }
        }
    }
}
