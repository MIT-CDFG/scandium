// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// B-09's display half: the eleven texts the bundle ships, listed and read.
//
// THE MODEL IS C++ AND ON DEMAND, FOR A MEASURED REASON. THIRD-PARTY-
// NOTICES.md is 573,513 bytes; eager-loading all eleven would cost ~750 KB
// at startup for a dialog most users never open, and half a megabyte in one
// Text element stalls the UI. `AppLicenseTexts.entries` is metadata only and
// `read()` is the sole call that touches bytes.
//
// TRUNCATION IS SAID, NOT HIDDEN. A capped read shows a notice pointing at
// Help > Open Licenses Folder, which opens the SAME folder these texts come
// from. The full text always ships; the cap governs what is rendered at
// once, never what is carried.
//
// THE `typeof` GUARD IS LOAD-BEARING, NOT DEFENSIVE HABIT. The Main.qml
// harness suites boot this tree with no host, so AppLicenseTexts does not
// exist there. An unguarded reference throws ReferenceError in every one of
// them — measured, three suites at once, when AppBuildCommit shipped without
// the same guard.

import QtQuick
import AppnameShell
import QtQuick.Controls
import QtQuick.Layouts

Dialog {
    id: root
    modal: true
    // ★ KEPT ON QT'S CONTRACT, NOT ON A MEASUREMENT FROM THIS TREE. I first
    // wrote that a modal Popup without `focus` "never receives key events at
    // all" -- that is FALSE and the controls said so: Escape still closed
    // (the overlay routes it, not active focus), and the list still held
    // focus with this line, the ListView's `focus` and the onOpened
    // forceActiveFocus ALL removed. The offscreen harness grants content
    // focus by itself, so it cannot discriminate any of the three. This
    // stays because a real desktop window is not the harness, and the cost
    // of being wrong is a modal a keyboard user cannot drive.
    focus: true
    title: AppI18n.tr("ui.dialog.licenses.title")
    anchors.centerIn: parent
    padding: 12

    readonly property var model_:
        (typeof AppLicenseTexts !== "undefined") ? AppLicenseTexts : null
    readonly property bool haveTexts: model_ !== null && model_.available

    property string currentRel: ""
    // ★ WORD 8 (2), 2026-09-14 18:48 ET. TRUE when the selected entry is
    //   enumerated but reads ZERO BYTES -- a case the operator ruled a PERMANENT
    //   POSTURE and never a shipped state, with a failing assertion in
    //   tst_license_texts to keep it that way.
    property bool currentMissing: false
    property string currentName: ""
    property bool currentTruncated: false

    // SELECTION IS AN INDEX, and every route into it goes through here:
    // click, arrow key, and open-time seeding. When the list drove only
    // `onClicked`, arrows moved the highlight while the pane behind it did
    // not follow -- measured, not inferred -- which reads as a broken app
    // rather than as an unsupported gesture.
    function showIndex(i) {
        if (!root.haveTexts || i < 0)
            return;
        const es = root.model_.entries;
        if (i >= es.length)
            return;
        root.show(es[i].rel, es[i].name);
    }

    // OPENING ON A BLANK PANE LOOKS LIKE A LOAD FAILURE, and this dialog has
    // an "unavailable" state that looks almost identical -- so an empty body
    // is actively misleading here, not merely unhelpful. Re-asserted on every
    // open rather than seeded once, so it holds however the dialog was left.
    onOpened: {
        if (!root.haveTexts)
            return;
        if (licensesList.currentIndex < 0)
            licensesList.currentIndex = 0;
        root.showIndex(licensesList.currentIndex);
        licensesList.forceActiveFocus();
    }

    function show(rel, name) {
        if (!root.haveTexts)
            return;
        const r = root.model_.read(rel);
        root.currentRel = rel;
        root.currentName = name;
        // THE HIGHLIGHT FOLLOWS, WHATEVER THE ENTRY POINT. C-752's rule is
        // that the highlight and the keyboard cursor must be the SAME thing,
        // and this function is a SECOND way in: called directly it moved the
        // body while `currentIndex` stayed put, leaving the list pointing at
        // one text and the pane showing another. Seen in a rendered frame
        // (C-762) -- and the product could not reach that state, but the
        // TESTS did, so they were exercising a UI no user can produce.
        // Assigning the same index back is inert: QML fires no change signal
        // for an unchanged value, so this cannot recurse through
        // onCurrentIndexChanged.
        const es = root.model_.entries;
        for (let i = 0; i < es.length; ++i) {
            if (es[i].rel === rel) {
                licensesList.currentIndex = i;
                break;
            }
        }
        root.currentTruncated = r.truncated === true;
        // ⛔ A BLANK PANE IS THE ONE THING THIS DIALOG MUST NOT DO. Its own
        //   comment above says an empty body is "actively misleading" because it
        //   is near-identical to the `unavailable` state -- the reader cannot tell
        //   "this build ships no texts" from "this text is empty". Say which.
        root.currentMissing = (r.text === undefined || r.text === "");
        body.text = root.currentMissing
            ? AppI18n.tr("ui.dialog.licenses.text_missing", {"name": name})
            : r.text;
        body.cursorPosition = 0;
    }

    ColumnLayout {
        spacing: 8

        // EMPTY IS AN HONEST STATE, and it is the one a dev build hits: no
        // staged folder resolved, so there is nothing to list. Saying so
        // beats an empty list that looks like a load failure.
        Label {
            objectName: "licensesUnavailable"
            visible: !root.haveTexts
            text: AppI18n.tr("ui.dialog.licenses.unavailable")
            Layout.maximumWidth: 520
            wrapMode: Text.WordWrap
        }

        RowLayout {
            visible: root.haveTexts
            spacing: 8

            ListView {
                id: licensesList
                objectName: "licensesList"
                Layout.preferredWidth: 220
                Layout.preferredHeight: 320
                clip: true
                // ELEVEN TEXTS, EIGHT ROWS VISIBLE — measured by looking at
                // the rendered dialog (C-761). Without a permanent scrollbar
                // there is NO affordance that the other three exist, and the
                // count grows with every dependency drop, so sizing the list
                // to today's eleven would be brittle where this is not.
                ScrollBar.vertical: ScrollBar { policy: ScrollBar.AlwaysOn }
                focus: true
                activeFocusOnTab: true
                keyNavigationEnabled: true
                currentIndex: -1
                model: root.haveTexts ? root.model_.entries : []
                onCurrentIndexChanged: root.showIndex(currentIndex)
                delegate: ItemDelegate {
                    required property var modelData
                    required property int index
                    width: ListView.view.width
                    text: modelData.name
                    // isCurrentItem, NOT a comparison against currentRel:
                    // the highlight and the keyboard cursor must be the
                    // SAME thing, or arrowing moves one and not the other.
                    highlighted: ListView.isCurrentItem
                    onClicked: licensesList.currentIndex = index
                }
            }

            ScrollView {
                // 690px GIVES A ~600px TEXT VIEWPORT, and the number comes from
                // the corpus rather than from taste. Measured across the
                // eleven staged texts: EIGHT are hard-wrapped at 74-78
                // columns (GPL/Apache/CC-BY-SA/CC0/Unicode and the notices
                // aggregate at p90 74), and THREE are unwrapped paragraphs —
                // ISC p90 471, MIT 105, LGPL-3.0-only max 937.
                //
                // SO WRAP MUST STAY: without it those three would demand
                // 900-column horizontal scrolling. And the pane must fit the
                // wrapped majority, or every upstream line re-wraps and
                // orphans its last word — which is what 520px did, defeating
                // this file's own stated intent of keeping the breaks the
                // texts were written with.
                // 690px is ~84 MONOSPACE COLUMNS at the rendered glyph width
                // (~8.2px, measured off the captured frame), which clears the
                // corpus MAX of 78. The 520 it replaces fit only ~63, which
                // is why every 71-78 column line re-wrapped and orphaned its
                // last word.
                Layout.preferredWidth: 690
                Layout.preferredHeight: 320
                TextArea {
                    id: body
                    objectName: "licensesBody"
                    // A read-only TextArea is still an interactive control
                    // (focusable, caret-navigable, selectable), so ES 31.5
                    // item 1 applies -- and a static "License text" would
                    // leave a reader unable to tell WHICH of the eleven is
                    // in the pane, since the name lives in the sibling list.
                    Accessible.name: root.currentRel === ""
                        ? AppI18n.tr("ui.dialog.licenses.body_none")
                        : root.currentMissing
                          ? AppI18n.tr("ui.dialog.licenses.text_missing",
                                       {"name": root.currentName})
                          : AppI18n.tr("ui.dialog.licenses.body",
                                       {"name": root.currentName})
                    readOnly: true
                    wrapMode: TextArea.WordWrap
                    // A licence text is not prose to be re-flowed by a font
                    // stack: fixed width keeps the upstream line breaks the
                    // texts were written with.
                    font.family: "Menlo"
                }
            }
        }

        Label {
            objectName: "licensesTruncated"
            visible: root.currentTruncated
            text: AppI18n.tr("ui.dialog.licenses.truncated")
            Layout.maximumWidth: 740
            wrapMode: Text.WordWrap
        }

        RowLayout {
            Layout.alignment: Qt.AlignRight
            Button {
                objectName: "licensesCloseButton"
                text: AppI18n.tr("ui.dialog.close")
                onClicked: root.close()
            }
        }
    }
}
