// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Probe QD (A11Y-2 review, made durable at A11Y-5): with EVERY child of
// a pane disabled, nextItemInFocusChain from the pane walks the
// WINDOW-WIDE chain OUT to a neighbor — which is why the pane cycle's
// first-operable-child advance must accept only DESCENDANTS (the
// containment check), or the cycle order breaks at the all-disabled
// edge.
// Result contract: `done` true; `walkedOutside` true (the chain exits
// the all-disabled pane).

import QtQuick
import QtQuick.Controls

ApplicationWindow {
    id: win
    width: 400
    height: 300
    visible: true

    property bool done: false
    property bool walkedOutside: false

    Column {
        Control {
            id: pane
            contentItem: Column {
                Button { text: "a"; enabled: false }
                Button { text: "b"; enabled: false }
            }
        }
        Button {
            id: neighbor
            text: "neighbor"
        }
    }

    Timer {
        id: probeTimer
        interval: 50
        onTriggered: {
            const first = pane.nextItemInFocusChain(true);
            let inside = false;
            for (let n = first; n; n = n.parent)
                if (n === pane) {
                    inside = true;
                    break;
                }
            win.walkedOutside = (first !== null) && !inside;
            win.done = true;
        }
    }

    Component.onCompleted: probeTimer.start()
}
