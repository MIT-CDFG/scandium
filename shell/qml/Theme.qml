// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: plan P-ROUND item 1 (operator directive) — the shell's design
// tokens: ONE place for palette, spacing, radii, and type scale. D-43
// trade dress: generic professional dark, no brand resemblance. Every
// visible-text pairing below holds WCAG-AA (>= 4.5:1 normal text,
// >= 3:1 large/bold + UI glyphs) against its documented background —
// ratios computed and recorded in the lane log per slice; changing a
// token means re-running that check (pinned by tst_theme_contrast).
// Indicator glyphs (group tag, mask badge — 10px UI indicators, not
// running text) are held to the 3:1 UI bar; measured 4.0:1 on bgActive
// (review P1-D5 classification).
//
// Canvas-side C++ rendering colors (workspace ground, overlay pens) stay
// C++ constants this slice — a context-property bridge is a noted
// follow-up, not silently divergent: the values here and there match.

pragma Singleton
import QtQuick

QtObject {
    // ---- palette: grounds ----
    readonly property color bgWindow: "#232428"   // window chrome ground
    readonly property color bgSurface: "#2c2d30"  // panels
    readonly property color bgRaised: "#37383b"   // rows, cards
    readonly property color bgActive: "#4a5364"   // active/selected row
    readonly property color bgCanvas: "#404245"   // workspace behind the document

    // ---- palette: content ----
    readonly property color textPrimary: "#e2e4e7"   // vs bgSurface 10.8:1
    readonly property color textSecondary: "#a8abb0" // vs bgSurface 6.0:1; UI-glyph bar on bgActive 3.4:1 (eye-off tint — review P2-D1)
    readonly property color textDisabled: "#85888e"  // >=3:1 UI bar on bgRaised 3.3, bgSurface 3.9, bgWindow 4.4 (P1-D3); NOT bgActive-safe (2.2:1) — never tint state indicators with it (P2-D1)
    readonly property color accent: "#8ec0ee"        // vs bgSurface 7.2:1
    readonly property color warnText: "#e6b0b0"      // vs bgSurface 7.3:1
    readonly property color badge: "#d4b96a"         // vs bgRaised 6.1:1
    readonly property color border: "#1d1e21"
    // borderLight: hairlines/separators AND — via palette.mid — the
    // Basic ComboBox's PRESSED face and the SpinBox up/down indicator
    // fills (Button's pressed fill is a 50% button/mid BLEND, review
    // A11Y-4 D4); textPrimary holds 5.6:1 on the pure token (pinned —
    // the ComboBox-face case exactly). NOT a compliant input boundary
    // (1.9:1 vs bgSurface): input outlines use borderInput instead.
    readonly property color borderLight: "#55585e"
    // A11Y-4: the ≥3:1 input-boundary token (WCAG 1.4.11) — 3.9:1 on
    // bgSurface (the input fill), 4.4 bgWindow, 3.3 bgRaised. Explicit
    // TextField backgrounds use it for the unfocused outline; the
    // SpinBox/ComboBox outline class is structurally palette.button in
    // the Basic style (couples to button faces) and migrates at the
    // Increment-2 style pass, recorded in the lane log.
    readonly property color borderInput: "#85888e"
    // §30.4 focus.ring token (A11Y-2): keyboard-focus indicator — the
    // accent hue at ring weight; >=3:1 on every ground it rings
    // (bgWindow 8.07, bgSurface 7.16, bgRaised 6.10, bgActive 4.03 —
    // reviewer-recomputed M2 values, pinned in tst_theme_contrast).
    readonly property color focusRing: "#8ec0ee"

    // ---- spacing / radii ----
    readonly property int spaceXs: 4
    readonly property int spaceSm: 6
    readonly property int spaceMd: 8
    readonly property int spaceLg: 12
    readonly property int radiusSm: 3
    readonly property int radiusMd: 4

    // ---- type scale ----
    readonly property int fontSm: 10
    readonly property int fontMd: 11
    readonly property int fontBase: 12
    readonly property int fontLg: 13
}
