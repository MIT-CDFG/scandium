// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: contracts/commands/doc.resize.yaml — exactly ONE of width|height|
// longestEdge (int 1..=1_000_000); resolutionPpi >0 optional; resample
// 6-kernel enum default lanczos; constrainAspect default true (and
// REQUIRED true with longestEdge — cross-field rule); scaleEffects
// default true. Plan §2.3 UI row (exact contract params).
//
// v1.0.1 item 3: the driving value is unit-aware (px/in/cm/mm) — PURE
// dialog math (units.h) through the existing ppi field; the door still
// receives the ROUNDED INT PX shown in the computed row. Disposition
// (noted for §5; hardened in the §5 fix round): physical units are
// UNSELECTABLE while ppi is empty/invalid, enforced at the SELECTION
// level — disabled delegates stop the mouse AND the activation handler
// rejects + snaps back to the px row for every keyboard path (closed-
// combo arrows, type-ahead, popup arrows+Return) — so the broken state
// is unreachable rather than explained, since no cataloged row exists
// for such a caption; if ppi is invalidated WHILE a physical unit is
// live, the combo reverts to px ON THAT EDIT, carrying the value
// across at the last-good ppi (ppiEdited validates the FRESH text
// itself — the ppiOk binding re-evaluates only afterwards), so the
// shown quantity never changes meaning and Apply never dies silently.
// The computed row previews the FULL result; the non-driven dimension
// mirrors the engine's round().max(1) over the aspect ratio and the
// engine's own rounding remains authoritative. Resample/driving
// semantics UNCHANGED.

import QtQuick
import AppnameShell
import QtQuick.Controls
import QtQuick.Layouts

Dialog {
    id: root
    modal: true
    title: AppI18n.tr("dialog.image_size.title")
    anchors.centerIn: parent
    padding: 12
    property string errorText: ""
    onOpened: {
        errorText = "";
        unitKey = "px";
        unitBox.currentIndex = 0;
        pixels.value = AppDoc.docWidth > 0 ? AppDoc.docWidth : 1024;
        if (ppiOk)
            lastGoodPpi = Number(ppi.text);
    }

    readonly property var drivers: ["width", "height", "longestEdge"]
    readonly property var kernels: ["nearest", "bilinear", "bicubic",
        "bicubic_smoother", "bicubic_sharper", "lanczos"]
    readonly property var unitKeys: ["px", "in", "cm", "mm"]
    property string unitKey: "px"
    // Physical units are only ever entered while ppi is valid, so this
    // is live whenever unitKey != px.
    property double lastGoodPpi: 0
    readonly property bool ppiOk: ppi.text !== "" && ppi.acceptableInput

    // The px truth of the driving value (physical values ride the
    // SpinBox as thousandths fixed point behind an EXPLICIT per-arm
    // validator — DoubleValidator decimals:3 physical, IntValidator px).
    readonly property int drivenPx: {
        if (unitKey === "px")
            return pixels.value;
        const dpi = ppiOk ? Number(ppi.text) : lastGoodPpi;
        const px = AppUnits.pxFromUnit(pixels.value / 1000.0, unitKey, dpi);
        // Cap the DOUBLE before the int-property coercion: a huge ppi
        // would otherwise ToInt32-wrap at 2^32. 2e6 is safely past the
        // door's 1e6 ceiling, so the engine still rejects loudly —
        // never a silently wrapped small number.
        return isFinite(px) ? Math.round(Math.min(px, 2000000)) : 0;
    }
    // Full-result preview (the non-driven side derives from the doc
    // aspect; the engine's own rounding is authoritative — this row is
    // the unit-conversion truth). The derived side mirrors the engine's
    // round().max(1.0) EXACTLY (geometry.rs resize_dims), so degenerate
    // aspect ratios preview the 1 px floor the engine actually produces
    // instead of an impossible 0.
    readonly property int outW: {
        if (AppDoc.docWidth <= 0 || AppDoc.docHeight <= 0)
            return 0;
        if (driver.currentIndex === 0)
            return drivenPx;
        if (driver.currentIndex === 1)
            return constrain.checked
                ? Math.max(1, Math.round(drivenPx * AppDoc.docWidth
                                         / AppDoc.docHeight))
                : AppDoc.docWidth;
        return Math.max(1, Math.round(AppDoc.docWidth * drivenPx
                          / Math.max(AppDoc.docWidth, AppDoc.docHeight)));
    }
    readonly property int outH: {
        if (AppDoc.docWidth <= 0 || AppDoc.docHeight <= 0)
            return 0;
        if (driver.currentIndex === 1)
            return drivenPx;
        if (driver.currentIndex === 0)
            return constrain.checked
                ? Math.max(1, Math.round(drivenPx * AppDoc.docHeight
                                         / AppDoc.docWidth))
                : AppDoc.docHeight;
        return Math.max(1, Math.round(AppDoc.docHeight * drivenPx
                          / Math.max(AppDoc.docWidth, AppDoc.docHeight)));
    }

    function unitPicked(newKey) {
        if (newKey === unitKey)
            return;
        // Selection-level guard (second belt behind onActivated): a
        // physical arm can never be COMMITTED while ppi is invalid,
        // whatever input path got here.
        if (newKey !== "px" && !ppiOk) {
            unitBox.currentIndex = unitKeys.indexOf(unitKey);
            return;
        }
        // Physical arms only proceed with a valid ppi (guard above).
        const dpi = Number(ppi.text);
        const curPx = unitKey === "px"
            ? pixels.value
            : AppUnits.pxFromUnit(pixels.value / 1000.0, unitKey, dpi);
        unitKey = newKey; // first: re-arms the SpinBox range/format
        if (newKey === "px")
            pixels.value = Math.min(1000000, Math.max(1, Math.round(curPx)));
        else
            pixels.value = Math.max(1, Math.round(
                AppUnits.unitFromPx(curPx, newKey, dpi) * 1000));
    }

    function ppiEdited() {
        // Validate the FRESH text directly: this handler runs off
        // ppi.onTextChanged BEFORE the root ppiOk binding re-evaluates,
        // so reading ppiOk here would see PRE-edit validity — the exact
        // mechanism that poisoned lastGoodPpi with Number("")==0 and
        // deferred the revert onto the user's correcting keystroke.
        const ok = ppi.text !== "" && ppi.acceptableInput;
        if (ok) {
            // lastGoodPpi is only ever assigned from VALID text.
            lastGoodPpi = Number(ppi.text);
            return;
        }
        if (unitKey !== "px") {
            // The disposition's second half, ON the invalidating edit:
            // carry the value to px at the last-good ppi instead of
            // displaying a dead physical number the field can no
            // longer convert — the quantity never changes meaning.
            const curPx = AppUnits.pxFromUnit(pixels.value / 1000.0,
                                              unitKey, lastGoodPpi);
            unitKey = "px";
            unitBox.currentIndex = 0;
            pixels.value = Math.min(1000000,
                                    Math.max(1, Math.round(curPx)));
        }
    }

    // Explicit per-arm validators for the driving SpinBox (never the
    // QQuickSpinBox default IntValidator, which silently eats the '.'
    // keystroke in physical mode — "7.3" typed as "73", a 10x trap).
    // The physical arm matches textFromValue/valueFromText exactly:
    // dot-decimal ("C" locale, Number()/toFixed(3)) at decimals:3.
    IntValidator { id: pxSpinValidator; bottom: 1; top: 1000000 }
    DoubleValidator { id: physSpinValidator; bottom: 0.001; decimals: 3
        locale: "C" } // Number() parses dot-decimal only (R13)

    ColumnLayout {
        spacing: 8
        RowLayout {
            Label { text: AppI18n.tr("ui.dialog.resize.driving") }
            ComboBox {
                id: driver
                Layout.preferredWidth: 150
                model: [AppI18n.tr("ui.dialog.resize.width"),
                        AppI18n.tr("ui.dialog.resize.height"),
                        AppI18n.tr("ui.dialog.resize.longest_edge")]
                // A11Y-3: §31.1 label association.
                Accessible.name: AppI18n.tr("ui.dialog.resize.driving")
            }
            SpinBox {
                id: pixels
                from: 1
                to: root.unitKey === "px" ? 1000000 : 1000000000
                stepSize: root.unitKey === "px" ? 1 : 1000
                value: 1024
                editable: true
                validator: root.unitKey === "px" ? pxSpinValidator
                                                 : physSpinValidator
                // Physical units ride thousandths (0.001-unit steps —
                // the DoubleValidator decimals:3 precision); px is the
                // plain integer it always was.
                textFromValue: function(value, locale) {
                    return root.unitKey === "px"
                        ? String(value)
                        : (value / 1000.0).toFixed(3);
                }
                valueFromText: function(text, locale) {
                    const n = Number(text);
                    if (!isFinite(n))
                        return value;
                    return root.unitKey === "px" ? Math.round(n)
                                                 : Math.round(n * 1000);
                }
                // A11Y-3: this field's visible label IS the driver combo's
                // current selection (Width/Height/Longest edge) — the
                // accessible name mirrors exactly what a sighted user
                // reads, no new id.
                Accessible.name: driver.currentText
            }
            ComboBox {
                id: unitBox
                Layout.preferredWidth: 76
                model: root.unitKeys.map(u => AppI18n.tr("ui.unit." + u))
                // A11Y-3: the driving-SpinBox pattern — the visible
                // label a sighted user reads for this control is its
                // current unit symbol; the accessible name mirrors it,
                // no new id.
                Accessible.name: currentText
                delegate: ItemDelegate {
                    required property int index
                    required property string modelData
                    width: unitBox.width
                    text: modelData
                    // Items 2+3 disposition: physical units unselectable
                    // while ppi is empty/invalid (px is always live).
                    enabled: index === 0 || root.ppiOk
                    highlighted: unitBox.highlightedIndex === index
                }
                onActivated: function(index) {
                    // SELECTION-level enforcement of the disposition:
                    // the disabled delegates stop only the mouse —
                    // closed-combo arrows, type-ahead and popup
                    // arrows+Return all land HERE with currentIndex
                    // already moved. While ppi is invalid, no input
                    // path may commit a physical unit: snap back to
                    // the px row (no caption — unreachable stays the
                    // design, no new id).
                    if (index !== 0 && !root.ppiOk) {
                        currentIndex = 0;
                        return;
                    }
                    root.unitPicked(root.unitKeys[index]);
                }
            }
        }
        RowLayout {
            Label { text: AppI18n.tr("ui.dialog.resize.ppi") }
            TextField {
                id: ppi; placeholderText: ""
                validator: DoubleValidator { bottom: 0.001; decimals: 3
                    locale: "C" } // Number() parses dot-decimal only (R13)
                Layout.preferredWidth: 90
                Accessible.name: AppI18n.tr("ui.dialog.resize.ppi")
                onTextChanged: root.ppiEdited()
                // A11Y-4: the Basic unfocused outline (palette.mid =
                // borderLight) is 1.9:1 vs the fill — below the 1.4.11
                // boundary bar. Explicit boundary: borderInput unfocused,
                // the §30.4 focus.ring focused.
                background: Rectangle {
                    implicitWidth: 90
                    implicitHeight: 28
                    color: Theme.bgSurface
                    border.width: ppi.activeFocus ? 2 : 1
                    border.color: ppi.activeFocus ? Theme.focusRing
                                                  : Theme.borderInput
                }
            }
        }
        RowLayout {
            Label { text: AppI18n.tr("ui.dialog.resize.resample") }
            ComboBox {
                id: resample
                Layout.preferredWidth: 170
                currentIndex: 5 // lanczos (contract default)
                model: root.kernels.map(k => AppI18n.tr("ui.resample." + k))
                Accessible.name: AppI18n.tr("ui.dialog.resize.resample")
            }
        }
        // longestEdge with constrainAspect:false is a contract cross-field
        // error — the switch locks on while longestEdge drives.
        Switch {
            id: constrain
            text: AppI18n.tr("ui.dialog.resize.constrain")
            checked: true
            enabled: driver.currentIndex !== 2
            onEnabledChanged: if (!enabled) checked = true
        }
        Switch {
            id: scaleEffects
            text: AppI18n.tr("ui.dialog.resize.scale_effects")
            checked: true
        }
        // ALWAYS visible: the px the door will receive (driven side is
        // exact; see the outW/outH preview note above).
        Label {
            text: AppI18n.tr("ui.dialog.new.computed_px",
                             {"width": root.outW, "height": root.outH})
        }
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
                    && (root.unitKey === "px" || root.ppiOk)
                onClicked: {
                    const params = {};
                    params[root.drivers[driver.currentIndex]] = root.drivenPx;
                    if (ppi.text !== "" && ppi.acceptableInput)
                        params.resolutionPpi = Number(ppi.text);
                    params.resample = root.kernels[resample.currentIndex];
                    params.constrainAspect = constrain.checked;
                    params.scaleEffects = scaleEffects.checked;
                    const r = AppDoc.applyCommand("doc.resize", {}, params, "");
                    if (r.ok) root.close(); else root.errorText = r.code + ": " + r.message;
                }
            }
        }
    }
}
