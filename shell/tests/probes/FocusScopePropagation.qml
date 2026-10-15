// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Probe P5/Q7 (A11Y-2 review, made durable at A11Y-5): a QQC2 Control
// (ToolBar here) IS a focus scope — a focused CHILD propagates
// activeFocus to the container, which is what lets the pane cycle
// detect the current pane via activeFocus alone.
// Result contract: `done` true; `barHasActiveFocus` true while the
// child button holds focus.

import QtQuick
import QtQuick.Controls

ApplicationWindow {
    id: win
    width: 400
    height: 300
    visible: true

    property bool done: false
    property bool barHasActiveFocus: false

    ToolBar {
        id: bar
        Button {
            id: child
            text: "probe"
        }
    }

    Timer {
        id: probeTimer
        interval: 50
        onTriggered: {
            win.barHasActiveFocus = bar.activeFocus === true;
            win.done = true;
        }
    }

    Component.onCompleted: {
        child.forceActiveFocus();
        probeTimer.start();
    }
}
