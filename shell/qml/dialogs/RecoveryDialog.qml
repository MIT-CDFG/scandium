// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: W05 (crash recovery) GUI prompt — P-ROUND item 6, armed on the
// dialog.recovery.* catalog family (registered ids, cataloged x4 by
// O-I18N). Evidence comes from AppDoc.queryRecovery BEFORE any open
// mutates state; Restore = appname_doc_open_recovered (log untouched
// until a save to the same path resolves it); Discard = two-step confirm
// then appname_autosave_discard + normal open; Not now = normal open,
// which preserves the log and runs the session unprotected
// (bindAutosaveIfSafe skip arm). Every string is a registered cataloged
// id — zero mints in this dialog.

import QtQuick
import AppnameShell
import QtQuick.Controls
import QtQuick.Layouts

Dialog {
    id: root
    modal: true
    title: AppI18n.tr("dialog.recovery.title")
    anchors.centerIn: parent
    padding: 12
    closePolicy: Popup.NoAutoClose

    property string packageUrl: ""
    property string logPath: ""
    property int commandCount: 0
    property bool confirmingDiscard: false

    function openFor(url, query) {
        packageUrl = url;
        logPath = query.logPath;
        commandCount = query.commands;
        confirmingDiscard = false;
        open();
    }

    ColumnLayout {
        spacing: 10

        Label {
            text: AppI18n.tr("dialog.recovery.unsaved_commands",
                             {"count": root.commandCount})
            color: Theme.textPrimary
        }

        // Two-step discard: the confirm text carries the document name and
        // the exact count being deleted.
        Label {
            visible: root.confirmingDiscard
            text: AppI18n.tr("dialog.recovery.discard_confirm",
                             {"title": decodeURIComponent(
                                  root.packageUrl.substring(
                                      root.packageUrl.lastIndexOf("/") + 1)),
                              "commands": root.commandCount})
            color: Theme.warnText
            wrapMode: Text.WordWrap
            Layout.maximumWidth: 340
        }

        RowLayout {
            Layout.alignment: Qt.AlignRight
            spacing: 8
            Button {
                visible: !root.confirmingDiscard
                text: AppI18n.tr("dialog.recovery.not_now")
                onClicked: {
                    root.close();
                    AppDoc.openDocument(root.packageUrl);
                }
            }
            Button {
                visible: !root.confirmingDiscard
                text: AppI18n.tr("dialog.recovery.discard")
                onClicked: root.confirmingDiscard = true
            }
            Button {
                visible: root.confirmingDiscard
                text: AppI18n.tr("ui.dialog.cancel")
                onClicked: root.confirmingDiscard = false
            }
            Button {
                visible: root.confirmingDiscard
                text: AppI18n.tr("dialog.recovery.discard_permanently")
                onClicked: {
                    root.close();
                    AppDoc.discardRecoveryLog(root.logPath);
                    AppDoc.openDocument(root.packageUrl);
                }
            }
            Button {
                visible: !root.confirmingDiscard
                highlighted: true
                text: AppI18n.tr("dialog.recovery.restore")
                onClicked: {
                    root.close();
                    AppDoc.openRecoveredDocument(root.packageUrl, root.logPath);
                }
            }
        }
    }
}
