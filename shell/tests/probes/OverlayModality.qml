// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Probe Q5 (A11Y-2 review, made durable at A11Y-5): Overlay.overlay
// children are popup ITEMS (QQuickPopupItem / dim rects), which expose
// NO `modal` property — modality detection via overlay children is
// categorically dead; the no-pane-focus test is the correct guard.
// Result contract: `done` flips true; `itemsSeen` > 0 while a modal
// dialog is open; `modalPropertySeen` stays false.

import QtQuick
import QtQuick.Controls

ApplicationWindow {
    id: win
    width: 400
    height: 300
    visible: true

    property bool done: false
    property int itemsSeen: 0
    property bool modalPropertySeen: false

    Dialog {
        id: dlg
        modal: true
        title: "probe"
        Label { text: "probe body" }
    }

    Timer {
        id: probeTimer
        interval: 50
        onTriggered: {
            const overlay = win.Overlay.overlay;
            for (let i = 0; i < overlay.children.length; ++i) {
                win.itemsSeen += 1;
                if (overlay.children[i].modal !== undefined)
                    win.modalPropertySeen = true;
            }
            win.done = true;
        }
    }

    Component.onCompleted: {
        dlg.open();
        probeTimer.start();
    }
}
