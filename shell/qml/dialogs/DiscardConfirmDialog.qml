// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Standing priority (4)-3: CONFIRM-ON-REPLACE. The shell is
// single-document (one m_hasDocument, no Close), so "replace" is the only
// door an unsaved document can leave by -- plus the window's own close,
// which is the one with no next state in which a user could notice the
// loss.
//
// THE DOOR LIST IS DERIVED BY EFFECT, NOT BY MENU SURFACE (measured
// 2026-09-01): File > New and File > Open REPLACE the document;
// File > Import does NOT -- importImage() requires an open document and
// issues edit.paste, so it adds an UNDOABLE LAYER, and a confirm there
// would ask the user to approve losing work that one Cmd+Z restores.
// The recovery prompt is a BRANCH inside the Open door (reachable only
// from openFileDialog.onAccepted), so one guard covers both branches.
//
// STRING DISCIPLINE (checked first-hand against i18n/en.json and the id
// registry, per DeleteConfirmDialog's precedent): the only cataloged
// "discard" sentences are dialog.recovery.* and they say something else
// -- discard_confirm_never_saved is about deleting a RECOVERY LOG ("it
// exists nowhere else and will be deleted entirely"), not about
// replacing an open document. Composing from them would have put a
// recovery sentence on a replace dialog. Three ids minted instead
// (i18n/MINTED.md), because here the MESSAGE IS THE FEATURE: a confirm
// whose proposition is borrowed from another flow is not a confirm.
// Cancel reuses the existing ui.dialog.cancel.
//
// modal: true -- the item-7 F4 lesson, as DeleteConfirmDialog records it:
// a dialog's dismissal press must be CONSUMED, never leaked to the canvas
// tool underneath.

import QtQuick
import AppnameShell
import QtQuick.Controls
import QtQuick.Layouts

Dialog {
    id: root
    modal: true
    title: AppI18n.tr("ui.dialog.discard_confirm.title")
    anchors.centerIn: parent
    padding: 12

    // The action to run if the user discards. Set by Main.qml's
    // confirmDiscard() helper, which is the ONLY caller -- one reader
    // function, three doors, so the reader set is enumerable rather than
    // scattered across the menu.
    property var discardAction: null
    // RC3 D-3 (operator WORD 35, sitting S-20): the prompt offers SAVE as
    // well as Don't Save and Cancel. Set alongside discardAction by
    // confirmDiscard().
    property var saveAction: null

    ColumnLayout {
        spacing: 8
        Label {
            text: AppI18n.tr("ui.dialog.discard_confirm.message")
            wrapMode: Text.WordWrap
            Layout.maximumWidth: 320
        }
        RowLayout {
            Layout.alignment: Qt.AlignRight
            spacing: 8
            Button {
                // Don't Save. The existing "Discard Changes" string serves;
                // the exact "Don't Save" wording is queued to O-I18N.
                text: AppI18n.tr("ui.dialog.discard_confirm.discard")
                objectName: "discardConfirmDontSave"
                onClicked: {
                    // Read and CLEAR before invoking: the action closes or
                    // replaces the document, and a stale handler surviving
                    // into the next document is the cross-document handle
                    // hazard document_controller already clears by name.
                    const act = root.discardAction;
                    root.discardAction = null;
                    root.saveAction = null;
                    root.close();
                    if (act) act();
                }
            }
            Button {
                text: AppI18n.tr("ui.dialog.cancel")
                objectName: "discardConfirmCancel"
                onClicked: { root.discardAction = null; root.saveAction = null; root.close(); }
            }
            Button {
                // RC3 D-3: SAVE, the default. Main.qml's saveAction saves
                // (Cmd+S's path; Save As when untitled) and continues only if
                // the save succeeded. Read-and-clear, as above.
                text: AppI18n.tr("ui.menu.file.save")
                objectName: "discardConfirmSave"
                highlighted: true
                onClicked: {
                    const save = root.saveAction;
                    root.discardAction = null;
                    root.saveAction = null;
                    root.close();
                    if (save) save();
                }
            }
        }
    }
}
