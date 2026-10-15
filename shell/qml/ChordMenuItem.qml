// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// v1.0.1 item 14b — a MenuItem that RENDERS its keyboard chord (the
// items-7+14 design entry: "every chord renders in its menu item").
// MEASURED at this build (offscreen probe, Qt 6.11.1 Basic style): the
// Basic MenuItem delegate is a single IconLabel/MnemonicLabel — it
// renders NO shortcut text even when an Action carries one. So the
// display is built here: a custom contentItem with the label left and
// the chord right.
//
// `chord` is the PORTABLE sequence (string like "Ctrl+D"/"Backspace" or
// a StandardKey value). Display text comes from Shortcut.nativeText —
// Qt's own OS-conventional rendering (⌘D on macOS), platform-provided
// like key names themselves: NOT a catalog string (the i18n-exempt
// class; zero new visible catalog strings here). The converter Shortcut
// is enabled: false — it never fires; the FIRING chord stays where it
// lives (a Main.qml Shortcut object, or the canvas key handler for the
// canvas-scope chords like Delete) so this component never introduces an
// ambiguous duplicate sequence.
//
// Colors ride the state-resolved control palette (QQuickControl
// resolves the disabled group automatically), so enabled/disabled
// dimming matches the Basic delegate this replaces. The chord label
// reuses the same resolved role at reduced opacity (presentation only).

import QtQuick
import QtQuick.Controls

MenuItem {
    id: item

    // §31.5 item 1 (the a11y tree lint models labels at DECLARATION
    // depth): this is a reusable COMPONENT — its `text` arrives at each
    // instantiation (always a cataloged menu id there). The binding
    // below restates MenuItem's own default accessible naming (name =
    // text) at this depth so the label truth is explicit where the
    // lint reads; behavior-identical (MenuItem is AbstractButton-
    // derived — Accessible is live here, not the Popup-inert class).
    Accessible.name: item.text

    // Portable sequence for display (string or StandardKey). Empty =
    // behave exactly like a plain MenuItem.
    property var chord: ""

    Shortcut {
        id: chordSeq
        enabled: false // converter only — never a second firing binding
        sequence: item.chord
    }

    contentItem: Item {
        implicitWidth: labelText.implicitWidth
                       + (chordText.visible ? chordText.implicitWidth + 24 : 0)
        implicitHeight: Math.max(labelText.implicitHeight,
                                 chordText.implicitHeight)
        Text {
            id: labelText
            anchors.left: parent.left
            anchors.verticalCenter: parent.verticalCenter
            anchors.right: chordText.visible ? chordText.left : parent.right
            anchors.rightMargin: chordText.visible ? 12 : 0
            text: item.text
            font: item.font
            color: item.palette.text
            elide: Text.ElideRight
            verticalAlignment: Text.AlignVCenter
        }
        Text {
            id: chordText
            objectName: "chordLabel"
            visible: chordSeq.nativeText !== ""
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            text: chordSeq.nativeText
            font: item.font
            color: item.palette.text
            opacity: 0.62
            verticalAlignment: Text.AlignVCenter
        }
    }
}
