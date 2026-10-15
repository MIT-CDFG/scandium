// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: contracts/commands/filter.apply.yaml + the ENGINE's registered
// per-filter schemas (corpus/*.rs — the registry is the validator;
// param names/ranges below were read from the registered schema_shape
// strings, the sigmaPx dead-name lesson applied). ONE table-driven
// dialog serves the six numeric-param filters; params equal to the
// engine default are OMITTED (the sentinel rule — engine defaults are
// never re-stated); a typed refusal surfaces in-dialog. A11Y-1
// substrate slice; liquify (workspace tier) and custom (kernel editor)
// have their own recorded later-wave surfaces.

import QtQuick
import AppnameShell
import QtQuick.Controls
import QtQuick.Layouts

Dialog {
    id: root
    modal: true
    title: filterId === "" ? "" : AppI18n.tr(filterId)
    anchors.centerIn: parent
    padding: 12

    property string filterId: ""
    property string errorText: ""

    // Engine-truth param tables (registered schemas): label keys are the
    // cataloged ui.dialog.filter.* family.
    // Values TRANSCRIBED from the registered schema_shape strings and
    // MECHANICALLY PINNED against them (tst_filter_table parses this
    // block vs the corpus sources — review MAJOR-1: three filters had
    // drifted, and the default-omission rule turns a drifted default
    // into a silent lie; the pin makes the class red). integer: true
    // rows step whole and serialize bare (serde parses them as ints).
    readonly property var tables: ({
        "filter.gaussian_blur": [
            { name: "radius", labelKey: "ui.dialog.filter.radius",
              min: 0.1, max: 1000, def: 8.0, step: 0.5, integer: false }
        ],
        "filter.box_blur": [
            { name: "radius", labelKey: "ui.dialog.filter.radius",
              min: 1, max: 1000, def: 4, step: 1, integer: true }
        ],
        "filter.median": [
            { name: "radius", labelKey: "ui.dialog.filter.radius",
              min: 1, max: 100, def: 2, step: 1, integer: true }
        ],
        "filter.high_pass": [
            { name: "radius", labelKey: "ui.dialog.filter.radius",
              min: 0.1, max: 1000, def: 10, step: 0.5, integer: false }
        ],
        "filter.add_noise": [
            { name: "amount", labelKey: "ui.dialog.filter.amount",
              min: 0.01, max: 4.0, def: 0.1, step: 0.01, integer: false }
        ],
        "filter.unsharp_mask": [
            { name: "amount", labelKey: "ui.dialog.filter.amount",
              min: 0.01, max: 5, def: 0.8, step: 0.05, integer: false },
            { name: "radius", labelKey: "ui.dialog.filter.radius",
              min: 0.1, max: 1000, def: 2, step: 0.5, integer: false },
            { name: "threshold", labelKey: "ui.dialog.filter.threshold",
              min: 0, max: 1, def: 0, step: 0.01, integer: false }
        ]
    })
    readonly property var rows: filterId in tables ? tables[filterId] : []

    function openFor(id) {
        filterId = id;
        errorText = "";
        open();
    }

    onOpened: {
        for (let i = 0; i < paramRepeater.count; ++i) {
            const item = paramRepeater.itemAt(i);
            if (item)
                item.resetToDefault();
        }
    }

    ColumnLayout {
        spacing: 10

        Repeater {
            id: paramRepeater
            model: root.rows
            RowLayout {
                id: paramRow
                required property var modelData
                spacing: 8
                // Alias, not positional child access (review MINOR-2).
                property alias realValue: box.realValue
                function resetToDefault() { box.value = Math.round(modelData.def * box.scale); }
                Label {
                    text: AppI18n.tr(paramRow.modelData.labelKey)
                    Layout.preferredWidth: 90
                }
                SpinBox {
                    id: box
                    // A11Y-3: §31.1 label association — the row's own
                    // schema-driven label key (same source as the visible
                    // Label above, so the two can never diverge).
                    Accessible.name: AppI18n.tr(paramRow.modelData.labelKey)
                    // Integer params step whole at scale 1 (their engine
                    // schemas are integer-typed — fractions refuse);
                    // fractional params scale x100 (pure presentation).
                    readonly property int scale: paramRow.modelData.integer ? 1 : 100
                    from: Math.round(paramRow.modelData.min * scale)
                    to: Math.round(paramRow.modelData.max * scale)
                    stepSize: Math.max(1, Math.round(paramRow.modelData.step * scale))
                    value: Math.round(paramRow.modelData.def * scale)
                    editable: true
                    property real realValue: value / scale
                    // Explicit validator (review MINOR-1): the default
                    // IntValidator rejects the '.' keystroke, making
                    // fractional values keyboard-unreachable — the house
                    // DoubleValidator pattern (ResizeDialog) applies.
                    validator: DoubleValidator {
                        bottom: paramRow.modelData.min
                        top: paramRow.modelData.max
                        decimals: paramRow.modelData.integer ? 0 : 2
                    }
                    textFromValue: function(v) {
                        return paramRow.modelData.integer
                            ? String(v) : (v / 100.0).toFixed(2);
                    }
                    valueFromText: function(t) {
                        return Math.round(parseFloat(t) * scale);
                    }
                }
            }
        }
        Label {
            visible: root.errorText !== ""
            text: root.errorText
            color: Theme.warnText
            wrapMode: Text.WordWrap
            Layout.maximumWidth: 280
        }

        RowLayout {
            Layout.alignment: Qt.AlignRight
            spacing: 8
            Button {
                text: AppI18n.tr("ui.dialog.cancel")
                onClicked: root.close()
            }
            Button {
                text: AppI18n.tr("ui.dialog.apply")
                enabled: root.rows.length > 0
                onClicked: {
                    const filterParams = {};
                    for (let i = 0; i < paramRepeater.count; ++i) {
                        const item = paramRepeater.itemAt(i);
                        const row = root.rows[i];
                        const v = item.realValue;
                        if (Math.abs(v - row.def) > 1e-9)
                            filterParams[row.name] = v; // default -> omit
                    }
                    const target = {};
                    if (root.activeLayerIdValue() !== "")
                        target["layerId"] = root.activeLayerIdValue();
                    const r = AppDoc.applyCommand("filter.apply", target, {
                        "filterId": root.filterId,
                        "params": filterParams
                    }, "");
                    if (r.ok)
                        root.close();
                    else
                        root.errorText = r.code + (r.message ? ": " + r.message : "");
                }
            }
        }
    }

    function activeLayerIdValue() {
        const rows = AppDoc.layers;
        const idx = AppDoc.activeLayerIndex;
        return (idx >= 0 && idx < rows.length)
            ? rows[idx].layerId : "";
    }
}
