// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: plan §2.2 row — history list panel: journal rows from
// appname_doc_history_json. Labels resolve the registry rule "a command
// kind's history label is history.<kind>" through the catalog; rows behind
// the cursor (active:false, redoable) render dimmed.
//
// Wave F: rows are ACTIVATABLE — click or keyboard (Return/Enter/Space on
// the focused row) jumps the document to that history state through the
// engine's history.goto position form (AppDoc.historyGoto — envelope via
// the apply door, receipt-driven refresh; no optimistic UI: the rows'
// active flags and the canvas re-render from the post-receipt poll).
// Enabled truth = the 14a history.goto availability row.

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Rectangle {
    id: historyPanel
    color: Theme.bgSurface
    radius: 4

    // The 14a availability surface (the CanvasContextMenu consumption
    // idiom) — one read for every row.
    readonly property bool gotoEnabled:
        AppDoc.commandAvailability["history.goto"].enabled === true
    // The cursor row (§7.8 truth relayed by the history door): the last
    // active entry — `active` is journal index < cursor.
    readonly property int cursorRow: {
        var rows = AppDoc.historyEntries;
        var last = -1;
        for (var i = 0; i < rows.length; ++i)
            if (rows[i].active)
                last = i;
        return last;
    }
    // Q-D22 RULED (a), 2026-09-01 — the history moved, so reveal the cursor.
    // `cursorRow` is a binding over the same model the view renders, so its
    // change signal IS the "history changed" event: undo, redo, a row jump
    // and a new action all move it. Nothing polls and nothing new is computed.
    onCursorRowChanged: historyList.revealCursor()

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 6
        spacing: 4

        Label {
            text: AppI18n.tr("ui.panel.history.title")
            font.bold: true
            color: Theme.textPrimary
        }

        Label {
            visible: historyList.count === 0
            text: AppI18n.tr("ui.panel.history.empty")
            color: Theme.textSecondary
        }

        ListView {
            id: historyList
            objectName: "historyList" // gesture-net anchor (tst_history_panel)
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: AppDoc.historyEntries
            spacing: 1
            // A11Y-3: plain list semantics for the journal rows.
            Accessible.role: Accessible.List

            // Q-D22 RULED (a) — KEEP THE CURSOR ROW ON SCREEN. The panel
            // rendered a cursor (the done/undone boundary: color + italic,
            // A11Y-4) that scrolled out of view after ~10 actions, so both
            // the row just acted on AND the boundary sat off-screen while
            // `Accessible.selected` announced the cursor correctly to a
            // screen reader — the accessibility layer better served than
            // the visual one (Q-D22's second instance of that pattern).
            //
            // SUPPRESSED WHILE THIS VIEW HOLDS FOCUS (the ruled clause): a
            // user who tabbed into the journal is browsing deliberately and
            // must not be scrolled out from under.
            //   ★ THE PREDICATE IS THE LIST, NOT THE PANEL, AND THAT IS
            //   MEASURED. With a delegate row focused, ListView.activeFocus
            //   reads TRUE (QQuickItemView is a focus scope) while the
            //   enclosing plain Rectangle `historyPanel` reads FALSE —
            //   probed offscreen with FocusScope and QQC2 Pane as positive
            //   controls, BOTH TRUE, so the window was active and that
            //   FALSE is a real discrimination rather than a dead window.
            //   `historyPanel.activeFocus` is the obvious reading of "while
            //   the panel has focus" and would have suppressed NOTHING
            //   while still looking correct. Main.qml:559 records the same
            //   fact from the other side ("QQC2 Control IS a focus scope").
            //
            // MODE — `Contain` scrolls ONLY when the cursor is off-screen,
            // so a cursor already in view never moves the viewport.
            //   ★ CONFIRMED BY THE OPERATOR 2026-09-04, EXPLICITLY AND NOT
            //   BY SILENCE: "KEEP ListView.Contain — scrolling only when the
            //   cursor leaves view is the least disruptive to a user reading
            //   nearby rows." Q-D22 had named the Contain/Center/End choice
            //   as its reason for being a question and the ruling took (a)
            //   as drafted without naming one, so this was my call until
            //   now; it is now his, on the record, and no longer a default
            //   riding inside a ruled item.
            function revealCursor() {
                if (historyList.activeFocus)
                    return;
                var r = historyPanel.cursorRow;
                // cursorRow and count both track AppDoc.historyEntries with
                // no guaranteed update order — position only inside the
                // range the view actually has.
                if (r < 0 || r >= historyList.count)
                    return;
                historyList.positionViewAtIndex(r, ListView.Contain);
            }
            // BOTH doors: the cursor moving (undo/redo/jump) and the model
            // growing (a new action, which can also replace the tail).
            // revealCursor is idempotent, so firing twice costs nothing.
            onCountChanged: historyList.revealCursor()

            delegate: Rectangle {
                id: histRow
                required property int index
                required property var modelData
                width: historyList.width
                height: 26
                radius: 3
                color: Theme.bgRaised
                // Wave F: keyboard activation — the row joins the tab
                // chain (the panels pane is an F6 cycle stop) and the
                // §30.4 focus.ring token marks the focused row (LayersPanel
                // rename-field idiom). Visual state language otherwise
                // UNCHANGED (A11Y-4: color + italic, below).
                activeFocusOnTab: true
                border.width: activeFocus ? 2 : 0
                border.color: Theme.focusRing
                Keys.onReturnPressed: histRow.jump()
                Keys.onEnterPressed: histRow.jump()
                Keys.onSpacePressed: histRow.jump()
                // The jump: engine truth only — history.goto through the
                // apply door; `position` rides the model row (computed
                // from the same history JSON these rows render). No
                // optimistic state: the receipt-driven poll refreshes
                // rows + canvas.
                function jump() {
                    if (historyPanel.gotoEnabled)
                        AppDoc.historyGoto(modelData.position);
                }
                // A11Y-4: redoable (undone) rows read as COLOR state, not
                // an opacity dim — the old whole-row 0.45 dim composited
                // the label to ~3.2:1 effective (below AA for
                // informational text); textSecondary/bgRaised is 5.1:1
                // (pinned, with a usage pin on this ternary).
                Accessible.role: Accessible.ListItem
                // ★ A11Y GATE — NEVER SPEAK A RAW CATALOG KEY.
                // `Catalog::tr` KEY-ECHOES on a miss (i18n_catalog.cpp, "the
                // id itself + all args, visibly untranslated"), and the echo
                // is NON-EMPTY AND TRUTHY, so `tr(k) || fallback` can never
                // fire — trap C10. The visible Label below KEEPS the echo on
                // purpose: the header calls key-echo the thing that "keeps
                // the UI honest", and a missing label should be obvious on
                // screen. SPOKEN, the same string is gibberish — an
                // assistive technology reads it out on focus, so a blind
                // user doing a clone stroke hears "history.clone.stroke".
                // That asymmetry is trap C11, and this is the belt to
                // O-INT's braces while Q-D35's ruled wordings are landed.
                //
                // MEASURED, not assumed: 8 of the 102 command kinds in
                // contracts/commands/_index.yaml have no `history.<kind>`
                // key (94 + 8 = 102, zero keys with no kind), and two of the
                // eight — clone.stroke and content_aware_fill — are
                // GUI-reachable today per Q-D35.
                //
                // WHY IDENTITY AND NOT A HELPER: `Catalog::has()` EXISTS in
                // C++ and is documented as the announcer's refusal gate
                // (§31.3), but CatalogQml exposes ONLY `tr` as Q_INVOKABLE,
                // so QML cannot ask. Identity against the key is exact here
                // because this call passes NO ARGS (with args the echo
                // appends " name=value"). Bound and stated: it would
                // mis-fire on a catalog entry whose value equals its own
                // key — measured 0 of 760 today, and 0 empty values.
                // Exposing has() to QML would remove the assumption; that is
                // a shell/src change and is filed, not taken here.
                Accessible.name: histRow.spokenLabel()
                // B-06 (Q-D24), RULED engine projection 2026-09-05: a row
                // that names a SUBJECT reads the subject's own translated
                // name. Every filter used to render "Apply Filter", so two
                // different filters were the same row forever and undoing
                // past one meant choosing between identical rows. A-18
                // carries `filterId` on the row; the shell resolves it
                // through the SAME catalog door as any other label, so
                // "Gaussian Blur" and "Liquify" are translated by the ids
                // that already ship in four locales — no id was minted for
                // this, and none was needed.
                function rowLabelKey() {
                    var fid = modelData.filterId;
                    return (fid && fid.length > 0) ? fid
                                                   : "history." + modelData.kind;
                }
                function spokenLabel() {
                    var key = histRow.rowLabelKey();
                    var s = AppI18n.tr(key);
                    // Fallback is the panel's own cataloged title — present
                    // and translated in all four locales. NO STRING IS
                    // MINTED: the history.* wordings are Q-D35's to land.
                    return s === key ? AppI18n.tr("ui.panel.history.title") : s;
                }
                // Wave F: activatable-row state per the LayersPanel row
                // convention — selected marks the CURSOR row (the state
                // the document is at). The redoable wording itself stays
                // a recorded gap (see the label comment below).
                Accessible.selectable: true
                Accessible.selected: histRow.index === historyPanel.cursorRow

                MouseArea {
                    anchors.fill: parent
                    enabled: historyPanel.gotoEnabled
                    onClicked: histRow.jump()
                }

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 6
                    anchors.rightMargin: 6
                    spacing: 6

                    Label {
                        text: modelData.index
                        color: Theme.textSecondary
                    }
                    Label {
                        text: AppI18n.tr(histRow.rowLabelKey())
                        elide: Text.ElideRight
                        // A11Y-4 review D3 (§31.4 "never by color alone",
                        // §31.5 gate item 4): the redoable state carries a
                        // SHAPE cue (italic) alongside the color pair —
                        // both usage-pinned. The ACCESSIBLE state remains
                        // a recorded gap (no cataloged wording for
                        // redoable yet; the row's name is the label; the
                        // a11y.* state wording rides a future mint — lane
                        // log, with the treeitem `level` gap).
                        color: modelData.active ? Theme.textPrimary
                                                : Theme.textSecondary
                        font.italic: !modelData.active
                        Layout.fillWidth: true
                    }
                }
            }
        }
    }
}
