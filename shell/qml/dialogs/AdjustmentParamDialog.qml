// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// v1.0.1 item 4 class (a) — THE ADJUSTMENT PARAM DIALOG.
//
// A TWIN OF FilterParamDialog, not an extension of it. That dialog's own
// pin asserts "expected 6 filters" and "8 param rows" against
// crates/appname-filters/src/corpus/*.rs; adding adjustment rows to its
// table would make one pin check two populations against two different
// Rust source sets, which is how a control stops being able to say what
// it covers. One table, one source set, one pin each.
//
// VALUES TRANSCRIBED FROM crates/appname-adjust/src/ops.rs's REGISTERED
// param_schema() STRINGS, and MECHANICALLY PINNED against them by
// tst_adjustment_table — the same arrangement that caught three drifted
// filters in review MAJOR-1. This is why the missing `param_schema` ABI
// arm is a CONVENIENCE here and not a correctness gap: the copy is kept
// honest by a test that parses both sides, a mechanism this repo already
// proved on the filter table.
//
// THE SENTINEL RULE, inherited deliberately: a param equal to the engine
// default is OMITTED from the submission. The engine restates its own
// defaults; the shell never does, so a drifted default cannot become a
// silent lie in the payload.
//
// BOOLEAN is a NEW control type here. brightness_contrast.legacy is the
// only boolean in the served set, and the filter table has no such row —
// which is exactly why class (a) was scoped as "the existing door plus
// ONE new control type" rather than as three more registrations.

import QtQuick
import AppnameShell
import QtQuick.Controls
import QtQuick.Layouts

Dialog {
    id: root
    modal: true
    title: opKind === "" ? "" : AppI18n.tr(opKind)
    anchors.centerIn: parent
    padding: 12

    property string opKind: ""
    property string errorText: ""

    // Engine-truth param tables. `bool: true` rows render a CheckBox and
    // serialize as a JSON boolean; every other row is a numeric spin.
    readonly property var tables: ({
        "adjustment.brightness_contrast": [
            { name: "brightness", labelKey: "ui.dialog.adjust.brightness",
              min: -1, max: 1, def: 0, step: 0.01, integer: false, bool: false },
            { name: "contrast", labelKey: "ui.dialog.adjust.contrast",
              min: -1, max: 1, def: 0, step: 0.01, integer: false, bool: false },
            { name: "legacy", labelKey: "ui.dialog.adjust.legacy",
              min: 0, max: 1, def: 0, step: 1, integer: true, bool: true }
        ],
        "adjustment.exposure": [
            { name: "exposure", labelKey: "ui.dialog.adjust.exposure",
              min: -20, max: 20, def: 0, step: 0.1, integer: false, bool: false },
            { name: "offset", labelKey: "ui.dialog.adjust.offset",
              min: -0.5, max: 0.5, def: 0, step: 0.01, integer: false, bool: false },
            { name: "gamma", labelKey: "ui.dialog.adjust.gamma",
              min: 0.01, max: 9.99, def: 1, step: 0.01, integer: false, bool: false }
        ],
        "adjustment.threshold": [
            { name: "level", labelKey: "ui.dialog.adjust.level",
              min: 0, max: 1, def: 0.5, step: 0.01, integer: false, bool: false }
        ]
    })

    // ---- CLASS (b): GROUPED SCALARS ----
    // levels is FOUR channels x the SAME five bounded scalars = 20 pure
    // numbers, every one of them what the Repeater above already renders.
    // C-421 filed it with curves and hue_saturation as needing a
    // purpose-built editor; the C-430 measurement moved it out of that
    // bucket, and this is what "a grouping concept, not a bespoke editor"
    // costs: a channel selector and a nested payload.
    //
    // The tuple is declared ONCE and shared by all four channels because
    // the engine declares it once, as `$defs/tuple` referenced by
    // composite/r/g/b. Writing it out four times would be four places to
    // drift from one schema.
    // GENERALISED AT CLASS (c). levels was the first grouped op and its
    // tuple lived in a property named after it; hue_saturation is the
    // SAME SHAPE with a different tuple and seven channels instead of
    // four, so the tuple moved INTO the table. C-421 filed three ops as
    // needing purpose-built editors, C-430's measurement moved levels
    // out, and this measurement moves hue_saturation out too: seven
    // bands x three bounded numbers is grouping, not a new concept.
    // CURVES IS THE ONLY ONE LEFT — its payload is arrays of control
    // points, which no arrangement of spin rows can express.
    readonly property var levelsTuple: [
        { name: "inBlack",  labelKey: "ui.dialog.adjust.in_black",
          min: 0, max: 1, def: 0, step: 0.01, integer: false, bool: false },
        { name: "inWhite",  labelKey: "ui.dialog.adjust.in_white",
          min: 0, max: 1, def: 1, step: 0.01, integer: false, bool: false },
        { name: "gamma",    labelKey: "ui.dialog.adjust.gamma",
          min: 0.1, max: 10, def: 1, step: 0.01, integer: false, bool: false },
        { name: "outBlack", labelKey: "ui.dialog.adjust.out_black",
          min: 0, max: 1, def: 0, step: 0.01, integer: false, bool: false },
        { name: "outWhite", labelKey: "ui.dialog.adjust.out_white",
          min: 0, max: 1, def: 1, step: 0.01, integer: false, bool: false }
    ]
    readonly property var hueSatBand: [
        { name: "hue", labelKey: "ui.dialog.adjust.hue",
          min: -180, max: 180, def: 0, step: 1, integer: false, bool: false },
        { name: "sat", labelKey: "ui.dialog.adjust.saturation",
          min: -1, max: 1, def: 0, step: 0.01, integer: false, bool: false },
        { name: "lightness", labelKey: "ui.dialog.adjust.lightness",
          min: -1, max: 1, def: 0, step: 0.01, integer: false, bool: false }
    ]
    readonly property var groupedTables: ({
        "adjustment.levels": {
            channels: ["composite", "r", "g", "b"], tuple: levelsTuple },
        "adjustment.hue_saturation": {
            channels: ["master", "r", "y", "g", "c", "b", "m"], tuple: hueSatBand }
    })
    // A grouped op may ALSO carry flat top-level params. hue_saturation
    // does: colorize (boolean) plus three colorize* numbers. They are
    // rendered UNGATED and that is deliberate — the schema states no
    // dependency between them, and a shell that disables a param the
    // engine will happily accept is inventing a constraint.
    // CURVES: channels like the grouped ops, but the per-channel payload
    // is a POINT LIST, not a tuple of scalars — so it gets the editor
    // rather than a repeater of spin rows.
    readonly property var curveTables: ({
        "adjustment.curves": { channels: ["master", "r", "g", "b"] }
    })
    readonly property bool isCurve: curveTables[opKind] !== undefined
    // Per-channel point lists, rebuilt on open.
    property var curveValues: ({})
    readonly property var groupedFlat: ({
        "adjustment.hue_saturation": [
            { name: "colorize", labelKey: "ui.dialog.adjust.colorize",
              min: 0, max: 1, def: 0, step: 1, integer: true, bool: true },
            { name: "colorizeHue", labelKey: "ui.dialog.adjust.colorize_hue",
              min: 0, max: 360, def: 0, step: 1, integer: false, bool: false },
            { name: "colorizeSat", labelKey: "ui.dialog.adjust.colorize_sat",
              min: 0, max: 1, def: 0.25, step: 0.01, integer: false, bool: false },
            { name: "colorizeLightness", labelKey: "ui.dialog.adjust.colorize_lightness",
              min: -1, max: 1, def: 0, step: 0.01, integer: false, bool: false }
        ]
    })
    readonly property bool isGrouped: groupedTables[opKind] !== undefined
    readonly property var channels: isGrouped
                                    ? groupedTables[opKind].channels
                                    : (isCurve ? curveTables[opKind].channels : [])
    readonly property var activeTuple: isGrouped ? groupedTables[opKind].tuple : []
    readonly property var flatRows: isGrouped && groupedFlat[opKind] !== undefined
                                    ? groupedFlat[opKind] : []
    property int activeChannel: 0
    // Per-channel edited values, rebuilt on open: channelValues[i][name].
    property var channelValues: ({})

    readonly property var rows: isGrouped
                                ? activeTuple
                                : (opKind !== "" && tables[opKind] !== undefined
                                   ? tables[opKind] : [])

    function openFor(kind) {
        root.opKind = kind;
        root.errorText = "";
        root.activeChannel = 0;
        const cv = {};
        const chans = root.groupedTables[kind] !== undefined
                      ? root.groupedTables[kind].channels : [];
        for (let c = 0; c < chans.length; ++c) {
            const one = {};
            const tup = root.groupedTables[kind].tuple;
            for (let j = 0; j < tup.length; ++j)
                one[tup[j].name] = tup[j].def;
            cv[c] = one;
        }
        root.channelValues = cv;
        if (root.curveTables[kind] !== undefined) {
            const curv = {};
            const cch = root.curveTables[kind].channels;
            for (let q = 0; q < cch.length; ++q)
                curv[q] = [[0, 0], [1, 1]];
            root.curveValues = curv;
            root.activeChannel = 0;
            curveEditor.points = [[0, 0], [1, 1]];
        }
        for (let i = 0; i < repeater.count; ++i) {
            const it = repeater.itemAt(i);
            if (it)
                it.resetToDefault();
        }
        root.open();
    }

    // The submission payload, sentinel rule applied: only params that
    // DIFFER from the engine default ride the wire.
    function collectParams() {
        const out = {};
        if (isCurve) {
            // SENTINEL RULE, curve form: omit a channel still at the
            // identity [[0,0],[1,1]]. Sending it explicitly would assert
            // the user shaped that channel and deliberately left it flat
            // — a claim the shell has no basis for about a channel they
            // never opened.
            root.curveValues[root.activeChannel] = curveEditor.points;
            for (let c = 0; c < root.channels.length; ++c) {
                const pts = root.curveValues[c];
                if (!pts || pts.length !== 2)
                    { if (pts) out[root.channels[c]] = pts; continue; }
                const identity = pts[0][0] === 0 && pts[0][1] === 0
                                 && pts[1][0] === 1 && pts[1][1] === 1;
                if (!identity)
                    out[root.channels[c]] = pts;
            }
            return out;
        }
        if (isGrouped) {
            // NESTED payload, sentinel rule applied TWICE: a param equal
            // to the engine default is omitted, and a channel whose every
            // param is default is omitted ENTIRELY rather than sent as an
            // empty object. `{}` is not the same message as absent — it
            // asserts the channel was considered and left at unity, which
            // is a claim the shell has no reason to make.
            root.syncActiveChannel();
            for (let c = 0; c < root.channels.length; ++c) {
                const vals = root.channelValues[c] || {};
                const one = {};
                for (let j = 0; j < root.activeTuple.length; ++j) {
                    const row = root.activeTuple[j];
                    if (vals[row.name] !== undefined && vals[row.name] !== row.def)
                        one[row.name] = vals[row.name];
                }
                if (Object.keys(one).length > 0)
                    out[root.channels[c]] = one;
            }
            // Flat top-level params of a grouped op ride BESIDE the
            // channels, sentinel rule applied the same way.
            for (let k = 0; k < flatRepeater.count; ++k) {
                const it = flatRepeater.itemAt(k);
                if (!it)
                    continue;
                const row = root.flatRows[k];
                if (row.bool === true) {
                    if (it.boolValue !== (row.def !== 0))
                        out[row.name] = it.boolValue;
                } else if (it.numValue !== row.def) {
                    out[row.name] = it.numValue;
                }
            }
            return out;
        }
        for (let i = 0; i < repeater.count; ++i) {
            const it = repeater.itemAt(i);
            if (!it)
                continue;
            const row = rows[i];
            if (row.bool === true) {
                if (it.boolValue !== (row.def !== 0))
                    out[row.name] = it.boolValue;
            } else if (it.numValue !== row.def) {
                out[row.name] = row.integer ? Math.round(it.numValue)
                                            : it.numValue;
            }
        }
        return out;
    }

    // The visible spin rows edit the ACTIVE channel; this writes them
    // back before the channel changes or the dialog submits, so switching
    // channels does not silently discard what was typed.
    function syncActiveChannel() {
        if (!isGrouped)
            return;
        const cv = root.channelValues;
        const one = cv[root.activeChannel] || {};
        for (let i = 0; i < repeater.count; ++i) {
            const it = repeater.itemAt(i);
            if (it)
                one[root.activeTuple[i].name] = it.numValue;
        }
        cv[root.activeChannel] = one;
        root.channelValues = cv;
        // RC3 D-2 (operator WORD 35, sitting S-12): a stray copy of
        // openFor()'s curve-reset block lived here, naming openFor's `kind`
        // parameter, which does not exist in this function. Every call
        // threw "ReferenceError: kind is not defined" -- and this runs for
        // the GROUPED ops only (levels, hue_saturation), on Apply and on a
        // channel switch, so their Apply died before reaching the engine:
        // no layer, no dialog text, no status line. The reset belongs (and
        // already is) in openFor().
    }

    contentItem: ColumnLayout {
        spacing: 8
        RowLayout {
            visible: root.isGrouped || root.isCurve
            Layout.fillWidth: true
            Label { text: AppI18n.tr("ui.dialog.adjust.channel") }
            ComboBox {
                objectName: "adjustChannelCombo"
                Accessible.name: AppI18n.tr("ui.dialog.adjust.channel")
                Layout.fillWidth: true
                model: root.channels
                currentIndex: root.activeChannel
                onActivated: {
                    if (root.isCurve) {
                        root.curveValues[root.activeChannel] = curveEditor.points;
                        root.activeChannel = index;
                        const nx = root.curveValues[index];
                        curveEditor.points = nx !== undefined ? nx : [[0, 0], [1, 1]];
                        return;
                    }
                    root.syncActiveChannel();
                    root.activeChannel = index;
                    for (let i = 0; i < repeater.count; ++i) {
                        const it = repeater.itemAt(i);
                        const v = (root.channelValues[index] || {})[root.activeTuple[i].name];
                        if (it)
                            it.numValue = v !== undefined ? v : root.activeTuple[i].def;
                    }
                }
            }
        }
        // ONE delegate, TWO repeaters. The tuple rows and the flat
        // top-level rows render identically — extracting the delegate
        // rather than copying it is the same reasoning the shared tuple
        // uses: a second copy is a second place to drift.
        Component {
            id: paramRowDelegate
            RowLayout {
                required property var modelData
                property real numValue: modelData.def
                property bool boolValue: modelData.def !== 0
                function resetToDefault() {
                    numValue = modelData.def;
                    boolValue = modelData.def !== 0;
                }
                Layout.fillWidth: true
                spacing: 8
                Label {
                    text: AppI18n.tr(modelData.labelKey)
                    Layout.preferredWidth: 150
                }
                CheckBox {
                    objectName: "adjustBool_" + modelData.name
                    // The visible label is the sibling Label, so this
                    // control carries NO text of its own — which reads as
                    // an unnamed checkbox to a screen reader. Accessible
                    // .name is the right channel here rather than `text`:
                    // duplicating the label visually would show it twice.
                    Accessible.name: AppI18n.tr(modelData.labelKey)
                    visible: modelData.bool === true
                    checked: parent.boolValue
                    onToggled: parent.boolValue = checked
                }
                SpinBox {
                    objectName: "adjustSpin_" + modelData.name
                    Accessible.name: AppI18n.tr(modelData.labelKey)
                    visible: modelData.bool !== true
                    Layout.fillWidth: true
                    // SpinBox is integer-valued; scale by 100 for the
                    // fractional rows so the engine's real bounds are
                    // representable without a second control type.
                    readonly property int scale: modelData.integer ? 1 : 100
                    from: Math.round(modelData.min * scale)
                    to: Math.round(modelData.max * scale)
                    stepSize: Math.max(1, Math.round(modelData.step * scale))
                    value: Math.round(parent.numValue * scale)
                    onValueModified: parent.numValue = value / scale
                    textFromValue: function (v) { return (v / scale).toFixed(modelData.integer ? 0 : 2); }
                    valueFromText: function (t) { return Math.round(parseFloat(t) * scale); }
                }
            }
        }
        Repeater {
            id: repeater
            model: root.rows
            delegate: paramRowDelegate
        }
        Repeater {
            id: flatRepeater
            model: root.flatRows
            delegate: paramRowDelegate
        }
        CurveEditor {
            id: curveEditor
            objectName: "curveEditor"
            visible: root.isCurve
            Layout.fillWidth: true
            Layout.preferredHeight: visible ? 240 : 0
            onPointsEdited: root.curveValues[root.activeChannel] = points
        }
        Label {
            objectName: "adjustErrorLabel"
            visible: root.errorText !== ""
            text: root.errorText
            // Theme.warnText, NOT an invented name: my first draft wrote
            // Theme.danger, which does not exist — QML would have resolved
            // it to an invalid colour and degraded the error text's
            // contrast with no test to notice. warnText is the vocabulary
            // and carries a documented 7.3:1 against bgSurface.
            color: Theme.warnText
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
        }

        // Q-D30: catalog-driven buttons, matching the other seventeen.
        //
        // This was the ONE dialog of eighteen using Qt's `standardButtons`, so
        // its "OK" and "Cancel" came from Qt's own translation machinery — and
        // NO QTranslator is installed (0 hits in shell/src/), so both rendered
        // hard English in es, ja and pseudo alike. The reason it survived a
        // census is that there was no emitted catalog id to check: audit.py
        // could not have gone red on it, because the instrument built to find
        // untranslated strings can only see strings that ASK the catalog.
        RowLayout {
            Layout.alignment: Qt.AlignRight
            Button { text: AppI18n.tr("ui.dialog.cancel"); onClicked: root.close() }
            Button {
                text: AppI18n.tr("ui.dialog.apply")
                onClicked: {
                    // The engine's typed refusal stays IN-DIALOG rather than
                    // closing over a rejection the user cannot see. Under
                    // `standardButtons` that required accept-then-reopen; the
                    // house pattern simply never closes on failure, so the
                    // reopen goes with the standardButtons it existed for.
                    const r = AppDoc.addAdjustmentLayer(root.opKind, root.collectParams());
                    if (r.ok === true) {
                        root.close();
                    } else {
                        root.errorText = r.message !== undefined && r.message !== ""
                                         ? r.message : r.code;
                    }
                }
            }
        }
    }

}
