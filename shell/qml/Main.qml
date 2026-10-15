// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: plan §2.3 "UI: full editing chrome" row (menus, toolbox, options
// bar, dialogs for the 0.5 kinds, OS clipboard copy-out). Trade dress
// D-43: generic layout (menu bar, LEFT tool palette since v1.0.1 item
// 11b, top contextual-options strip, canvas center, side panels, status
// bar) and generic names — our lucide glyphs (P2), cataloged text labels
// where the vendored set has no fitting glyph yet.
// Every visible string resolves through AppI18n. Import/Export/Paste ship
// DISABLED pending the raster-ingress + export ABI additions
// (reports/O-UI-log.md, ABI-growth request 1-2); guides/grid UI is
// deferred whole to the readback wave (request 3) — a client-side mirror
// would desync on undo.

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Dialogs
import AppnameShell
import "dialogs"

ApplicationWindow {
    id: root
    width: 1280
    height: 800
    visible: true
    title: AppDoc.windowTitle
    color: Theme.bgWindow

    // ---- standing priority (4)-3: THE DIRTY FLAG'S READERS ----
    // The flag itself has been correct since 2026-08-13; until now it had
    // exactly one consumer, the title's dot, because the computation was
    // a local inside windowTitle() and nothing could read it. These are
    // the readers the operator's ruling requires ship WITH it.
    //
    // ONE helper, THREE doors, derived BY EFFECT: File > New (via
    // newDialog.confirmGate), File > Open (openFileDialog.onAccepted,
    // ahead of the recovery branch so both arms are covered), and the
    // window's own close. File > Import is deliberately NOT here --
    // importImage() adds an undoable layer and discards nothing.
    property bool forceClose: false

    function confirmDiscard(action) {
        if (!AppDoc.documentHasUnsavedWork) { action(); return; }
        discardConfirmDialog.discardAction = action;
        discardConfirmDialog.saveAction = function() { root.saveThenContinue(action); };
        discardConfirmDialog.open();
    }
    // RC3 D-3 (operator WORD 35, sitting S-20): the prompt's Save. A titled
    // document saves through Cmd+S's path; an untitled one opens Save As and
    // the pending action runs only after a SUCCESSFUL save (a cancelled or
    // failed Save As leaves the user where they were -- nothing is lost).
    function saveThenContinue(action) {
        if (AppDoc.documentPath !== "") {
            if (AppDoc.saveDocument(""))
                action();
            return;
        }
        saveFileDialog.afterSave = action;
        saveFileDialog.open();
    }
    // RC3 D-3: File > Close (Cmd+W) -- through the same unsaved-changes gate.
    function closeDocumentGuarded() {
        if (!AppDoc.hasDocument)
            return;
        root.confirmDiscard(function() { AppDoc.closeDocument(); });
    }

    // DOOR 3, and it is the one with no next state: a replaced document
    // leaves the user somewhere they could notice the loss; a closed
    // window leaves nothing at all. Qt delivers the close as a vetoable
    // event, so the guard REFUSES it and re-issues the close only once
    // the user has said so -- forceClose is what keeps that second close
    // from re-entering this handler forever.
    onClosing: function(close) {
        if (root.forceClose || !AppDoc.documentHasUnsavedWork)
            return; // nothing unsaved, or already confirmed: let it go
        close.accepted = false;
        root.confirmDiscard(function() { root.forceClose = true; root.close(); });
    }

    // App-wide dark chrome (P1b): the Basic style derives every control's
    // colors from the window palette — one token-driven block themes
    // toolbars, menus, dialogs, and inputs consistently (AA pairs per
    // Theme.qml).
    // Explicit color groups, NO parent-group shorthand: a parent role
    // assigned via a BINDING writes across all groups after the disabled
    // sub-block applies, clobbering it (review P1-D1; minimal repros in
    // the review record). active/inactive mirrored; disabled carries the
    // dimmed text roles on the same grounds.
    palette {
        active {
            window: Theme.bgWindow
            windowText: Theme.textPrimary
            button: Theme.bgRaised
            buttonText: Theme.textPrimary
            base: Theme.bgSurface
            text: Theme.textPrimary
            highlight: Theme.bgActive
            highlightedText: Theme.textPrimary
            mid: Theme.borderLight
            dark: Theme.border
            light: Theme.borderLight
            placeholderText: Theme.textSecondary
            toolTipBase: Theme.bgRaised
            toolTipText: Theme.textPrimary
        }
        inactive {
            window: Theme.bgWindow
            windowText: Theme.textPrimary
            button: Theme.bgRaised
            buttonText: Theme.textPrimary
            base: Theme.bgSurface
            text: Theme.textPrimary
            highlight: Theme.bgActive
            highlightedText: Theme.textPrimary
            mid: Theme.borderLight
            dark: Theme.border
            light: Theme.borderLight
            placeholderText: Theme.textSecondary
            toolTipBase: Theme.bgRaised
            toolTipText: Theme.textPrimary
        }
        disabled {
            window: Theme.bgWindow
            windowText: Theme.textDisabled
            button: Theme.bgRaised
            buttonText: Theme.textDisabled
            base: Theme.bgSurface
            text: Theme.textDisabled
            placeholderText: Theme.textDisabled
        }
    }

    readonly property string activeLayerId: {
        const rows = AppDoc.layers;
        const idx = AppDoc.activeLayerIndex;
        return (idx >= 0 && idx < rows.length) ? rows[idx].layerId : "";
    }

    // ---- A11Y-3 wave B: UI-event announcements (§31.3) ----
    // Tool wire-id -> display-name catalog key (the same keys the
    // palette slots and flyout items show; puppet's palette row reuses
    // its menu item's cataloged name — one id, every surface).
    // P1-1 (operator ruling, 2026-08-28): FILE GLOBS ARE CODE CONSTANTS, never
    // carried inside a translatable string. They were embedded in the catalog
    // values (`"Appname package (*.appname)"`), which put a load-bearing match
    // pattern behind a lookup that KEY-ECHOES ON A MISS -- Catalog::tr returns
    // the key itself, a string with no glob, so a catalog that fails to load
    // yields a filter matching NOTHING and every file greys out. The label is
    // translated; the pattern is not, because a translated glob is a defect in
    // every language it is translated into.
    readonly property var fileGlobs: ({
        "appname": "*.appname",
        "images":  "*.png *.jpg *.jpeg *.tif *.tiff *.webp *.gif *.bmp",
        "png":     "*.png",
        "jpeg":    "*.jpg *.jpeg",
        "tiff":    "*.tif *.tiff",
        "webp":    "*.webp",
        "gif":     "*.gif",
        "bmp":     "*.bmp",
        "ora":     "*.ora"
    })
    // Catalog keys as WHOLE LITERALS, never composed. The first version built the
    // key as tr("ui.dialog.file.filter_" + kind) and the i18n audit RED-1/RED-2'd
    // it: audit.py's QUOTED_DOTTED_RE extracts quoted dotted strings from QML, so
    // it read the concatenation PREFIX "ui.dialog.file.filter_" as a complete id
    // that exists in no catalog -- while the nine real ids became invisible to it.
    // A KEY BUILT AT RUNTIME CANNOT BE SEEN BY A STATIC AUDIT. Fixing one
    // invisibility (a glob hidden behind a lookup that key-echoes) had created
    // another. Verified against the audit's own regex before landing: nine ids
    // extracted, zero prefix-shaped.
    readonly property var fileFilterKeys: ({
        "appname": "ui.dialog.file.filter_appname",
        "images":  "ui.dialog.file.filter_images",
        "png":     "ui.dialog.file.filter_png",
        "jpeg":    "ui.dialog.file.filter_jpeg",
        "tiff":    "ui.dialog.file.filter_tiff",
        "webp":    "ui.dialog.file.filter_webp",
        "gif":     "ui.dialog.file.filter_gif",
        "bmp":     "ui.dialog.file.filter_bmp",
        "ora":     "ui.dialog.file.filter_ora"
    })
    // Compose a Qt name filter from a translated LABEL and a code-constant glob.
    function fileFilter(kind) {
        return AppI18n.tr(root.fileFilterKeys[kind]) + " (" + root.fileGlobs[kind] + ")";
    }
    readonly property var toolNameKeys: ({
        "move": "ui.tool.move",
        "select.rect": "ui.tool.select_rect",
        "select.ellipse": "ui.tool.select_ellipse",
        "select.lasso": "ui.tool.lasso",
        "select.polygon": "ui.tool.polygon",
        "select.wand": "ui.tool.wand",
        "crop": "ui.tool.crop",
        "brush.stroke": "tool.brush",
        "eraser.stroke": "ui.tool.eraser",
        "transform.free": "ui.tool.transform",
        "vector.pen": "ui.tool.pen",
        "shape.rect": "ui.tool.shape_rect",
        "shape.ellipse": "ui.tool.shape_ellipse",
        "text": "ui.tool.text",
        "transform.perspective": "ui.tool.perspective",
        "transform.puppet": "menu.edit.puppet_warp",
        // 11c: the warp palette row (ToolPalette.qml transform group)
        // announces with its cataloged name; without this row
        // announceToolChange skips warp silently.
        "transform.warp": "ui.tool.warp",
        // tier-2 rows (post-mint slice; ToolPalette.qml header record):
        // every armed member announces its MINTED cataloged name —
        // a missing row here is the same silent-skip class as warp's.
        "select.quick": "ui.tool.quick_select",
        "heal.stroke": "ui.tool.heal",
        "clone.stroke": "ui.tool.clone",
        "content_aware_fill": "ui.tool.content_aware_fill",
        "fill": "ui.tool.paint_bucket",
        "dodge_burn.dodge": "ui.tool.dodge",
        "dodge_burn.burn": "ui.tool.burn",
        "dodge_burn.sponge": "ui.tool.sponge",
        "blur_sharpen.blur": "ui.tool.blur",
        "blur_sharpen.sharpen": "ui.tool.sharpen",
        "smudge.stroke": "ui.tool.smudge",
        "shape.line": "ui.tool.shape_line",
        "shape.polygon": "ui.tool.shape_polygon",
        "shape.rounded_rect": "ui.tool.shape_rounded_rect",
        "selection.transform": "ui.tool.transform_selection",
        "slice.add": "ui.tool.slice",
        "slice.select": "ui.tool.slice_select"
    })
    // Pre-completion property bindings fire the change signals once at
    // startup — announcing those would narrate the app constructing
    // itself.
    property bool uiReady: false
    Component.onCompleted: uiReady = true
    // §31.3 no-double-speak: when active focus sits INSIDE a tool
    // surface, the platform itself speaks the focused control's own
    // changes (a button's checked flip, a slider's value change — QQC2
    // raises the value-change accessibility events). The live region
    // stays quiet for changes driven from there; key/menu-driven changes
    // announce. (Review D3: the brush-size announcement needed this same
    // guard — Tab-to-slider + arrows had the platform AND the region
    // speaking.) Since 11b the tool buttons live in palettePane and the
    // contextual options stay in toolbarPane — the guard walks BOTH (the
    // two sites updated coherently with the pane cycle below).
    // WHAT THE GUARD ACTUALLY COVERS (11b review D6 correction,
    // probe-verified offscreen AND cocoa): KEYBOARD-FOCUSED activation
    // only. A pointer click on a palette slot does NOT move focus into
    // the pane (macOS button click-focus convention; offscreen agrees),
    // so slot-CLICK tool changes announce through the live region — as
    // do flyout items (overlay-hosted, outside the walk) and
    // shortcut/menu-driven changes. That is pre-11b parity (the old
    // toolbar had the same structure), and the platform speaks nothing
    // on an unfocused click, so no double-speak arises; the review-D3
    // case (Tab to a control, activate from focus) stays suppressed.
    // Operator's windowed VoiceOver pass: confirm the click path for
    // SR-with-mouse-tracking users.
    function focusInsideToolbar() {
        for (let n = root.activeFocusItem; n; n = n.parent)
            if (n === toolbarPane || n === palettePane)
                return true;
        return false;
    }
    // v1.0.1 item 11e, desk ruling (e) — THE EXEMPTION KEYS ON INPUT
    // SOURCE, not on focus. The D6 guard exists to suppress
    // focus-traversal double-speak; a letter chord fired while the
    // palette pane holds focus moves NO item focus, so suppressing there
    // was pure SILENCE — the recorded hole (F6 into the palette, press a
    // letter, the tool changes with nothing spoken). Keymap dispatch is
    // not focus traversal, so a chord-driven change announces in every
    // pane; click and Space-on-a-focused-slot are unchanged.
    function announceToolChange() {
        if (!uiReady || (focusInsideToolbar() && !root.chordDriven))
            return;
        const key = toolNameKeys[canvas.tool];
        if (key)
            AppA11y.announce("tool", "a11y.tool.selected",
                             {"name": AppI18n.tr(key)});
    }
    // ---- item 25b HONESTY SWEEP (honest-state rule D4) ----
    // Pending-reason OVERRIDES only. The default reason for an armed
    // tool whose press falls through the canvas if-chain is
    // ui.status.tool_pending ("{tool} is not wired to the canvas yet");
    // rows appear here only when the honest wording differs.
    //
    // This is NOT a roster of armed tools — the armed set is derived in
    // C++ from the press chain's fall-through and never listed. Adding a
    // row here does not arm a tool and removing one does not wire it;
    // the map only chooses WORDING for whatever the derivation produced.
    //
    // warp + selection.transform take the class-E session wording
    // (i18n/MINTED.md: "distinct from the stroke-shaped preview note"):
    // both open an interactive canvas session rather than acting on a
    // single press, so "not wired to the canvas yet" would understate
    // what is missing.
    //
    // Deliberately NOT here: shape.polygon / shape.rounded_rect. Their
    // honest reason is ui.pending.option_required, whose {option} arg
    // would have to name the `sides` / `corner_radius` controls — and
    // those labels are mint-RESERVED under D4 (no ui.options.sides,
    // no ui.options.corner_radius). Passing a raw English word is the
    // P2-D2 key-echo class, so both rows take the plain template until
    // 11d lands their tooltip surface and their minted labels together.
    readonly property var pendingReasonIds: ({
        "transform.warp": "ui.pending.session",
        "selection.transform": "ui.pending.session",
        // ★★★★★ Q-D21 RULED (a), 2026-09-01. Before this row, SELECTING clone
        // said "Clone needs a source point" and PRESSING it said "Clone is not
        // wired to the canvas yet" — two sentences, seconds apart, naming
        // different missing things (INTEGRATION-log 450). Both paths now speak
        // this one, and they agree BY CONSTRUCTION rather than by care.
        // ★★ Why THIS sentence and not tool_pending: it says WHY, which is
        // strictly more than "not wired", and it stays true after priority 5
        // wires clone — at which point the missing source point is the whole
        // of what remains. The operator's ruling: (b) is forbidden by Q-D14's
        // own word that a bound key must not be silent, and (c) would pin
        // today's incoherence with a test priority 5 immediately invalidates.
        "clone.stroke": "ui.status.clone_needs_source"
    })
    // One announcer SOURCE for the whole pending family: the announcer
    // coalesces latest-wins per source inside its 500 ms window, so
    // press-spamming an armed tool speaks once with the latest state
    // instead of queueing a stutter.
    function announcePending(reason, args) {
        AppA11y.announce("tool_pending", reason, args);
        AppDoc.statusPending(reason, args);
    }
    Connections {
        target: canvas
        function onToolChanged() { root.announceToolChange(); }
        // The armed-silent press, made honest. `tool` is the PALETTE
        // wire id: the display name must come from the id the user
        // clicked, not from the engine kind — the two are not 1:1 (the
        // three Toning tools share one kind, and announcing all three as
        // the same name is exactly the lie this sweep removes).
        function onArmedToolPressed(tool) {
            const nameKey = root.toolNameKeys[tool];
            if (!nameKey)
                return; // no cataloged name: silence beats key-echoing a wire id
            root.announcePending(root.pendingReasonIds[tool]
                                     || "ui.status.tool_pending",
                                 {"tool": AppI18n.tr(nameKey)});
        }
        // A WIRED tool whose engine arm answered typed not_implemented
        // (today: the wand on a multi-layer document). The reason id is
        // BARE — it describes the milestone, not the tool — so it takes
        // no args, and passing any would key-echo into the wording.
        function onArmedToolRefusedPending(tool) {
            root.announcePending("ui.pending.needs_engine_arm", {});
        }
        function onBrushSizeChanged() {
            if (root.uiReady && !root.focusInsideToolbar())
                AppA11y.announce("brush", "a11y.tool.brush_size",
                                 {"size": Math.round(canvas.brushSize)});
        }
        function onZoomChanged() {
            // Footer zoom buttons carry no value info in their own
            // names — the announcement is wanted there (review-verified;
            // no toolbar guard on zoom).
            if (root.uiReady)
                AppA11y.announce("zoom", "a11y.zoom.level",
                                 {"value": canvas.zoomPercent});
        }
    }

    // ---- shortcuts (standard, generic) ----
    // v1.0.1 item 14b: every chord with an availability row CONSUMES the
    // 14a surface (the same row its menu item binds — one truth, zero
    // duplicated predicates); a chord whose row is disabled is a no-op,
    // exactly like its disabled menu item. New/Open have no rows (never
    // gated). The Delete/Backspace + Alt/Cmd+Delete + arrow chords are
    // NOT here: they are CANVAS-FOCUS scope (canvas_view keyPressEvent —
    // the §30.2 yield-to-text-fields pattern; Alt/Cmd+Delete are macOS
    // text-editing chords a window-context Shortcut would steal).
    function rowEnabled(action) {
        return AppDoc.commandAvailability[action].enabled === true;
    }
    // ★ THE FIRING HALF OF THE LAYER MENU (WORD 8 (3)). ChordMenuItem only
    //   RENDERS a chord; these are the objects that fire. Each calls the same
    //   panel function the menu item does, and each of those re-checks its own
    //   guard -- a Shortcut fires regardless of any menu item being enabled.
    // 1.0 final (corrigendum (l)): no Ctrl+G / Ctrl+Shift+G (Group removed).
    Shortcut { sequence: "Ctrl+E";       onActivated: layersPanel.doMergeDown() }
    Shortcut { sequence: "Ctrl+]";       onActivated: layersPanel.doMoveUp() }
    Shortcut { sequence: "Ctrl+[";       onActivated: layersPanel.doMoveDown() }
    Shortcut { sequences: [StandardKey.New]; onActivated: newDialog.open() }
    Shortcut { sequences: [StandardKey.Close]; onActivated: root.closeDocumentGuarded() }
    Shortcut { sequences: [StandardKey.Open]; onActivated: openFileDialog.open() }
    Shortcut { sequences: [StandardKey.Save]; onActivated: if (root.rowEnabled("file.save")) root.saveOrSaveAs(); else AppDoc.reportUnavailable("file.save") }
    Shortcut { sequences: [StandardKey.SaveAs]; onActivated: if (root.rowEnabled("file.saveAs")) saveFileDialog.open(); else AppDoc.reportUnavailable("file.saveAs") }
    // Undo binds the chord table's OWN row (portable Ctrl+Z = Cmd+Z),
    // not the StandardKey binding SET: some platform themes alias
    // Alt+Backspace onto Undo (measured on the offscreen theme), which
    // COLLIDES with the ruled Alt+Delete/Backspace = fill-foreground
    // chord. The table wins — one binding per chord, every theme.
    Shortcut { sequence: "Ctrl+Z"; onActivated: if (root.rowEnabled("edit.undo")) AppDoc.undo(); else AppDoc.reportUnavailable("edit.undo") }
    Shortcut { sequences: [StandardKey.Redo]; onActivated: if (root.rowEnabled("edit.redo")) AppDoc.redo(); else AppDoc.reportUnavailable("edit.redo") }
    // 14b: the Cut chord (the design table's "menu ✓; chord added") —
    // the menu item's exact door (§26.1 app-side copy + edit.cut).
    Shortcut { sequences: [StandardKey.Cut]; onActivated: if (root.rowEnabled("edit.cut")) AppDoc.cutLayer(root.activeLayerId); else AppDoc.reportUnavailable("edit.cut") }
    Shortcut { sequences: [StandardKey.Copy]; onActivated: if (root.rowEnabled("edit.copy")) AppDoc.copyCompositeToClipboard(); else AppDoc.reportUnavailable("edit.copy") }
    Shortcut {
        sequences: [StandardKey.Paste]
        // Item 12 fit-and-center rides root.pasteAndFit (item 7's
        // extraction); the inline hasDocument/abiBlock12 conjuncts
        // collapsed into the edit.paste row (14b — same truth).
        onActivated: if (root.rowEnabled("edit.paste")) root.pasteAndFit(); else AppDoc.reportUnavailable("edit.paste")
    }
    Shortcut { sequences: [StandardKey.SelectAll]; onActivated: if (root.rowEnabled("select.all")) AppDoc.applyCommand("selection.all", {}, {}, ""); else AppDoc.reportUnavailable("select.all") }
    Shortcut { sequence: "Ctrl+D"; onActivated: if (root.rowEnabled("select.deselect")) AppDoc.applyCommand("selection.none", {}, {}, ""); else AppDoc.reportUnavailable("select.deselect") }
    Shortcut { sequence: "Ctrl+Shift+I"; onActivated: if (root.rowEnabled("select.invert")) AppDoc.applyCommand("selection.invert", {}, {}, ""); else AppDoc.reportUnavailable("select.invert") }
    // 14b: reselect — the shell's last-selection memory replayed through
    // the journaled selection.set door (row disabled until memory arms).
    Shortcut { sequence: "Ctrl+Shift+D"; onActivated: if (root.rowEnabled("select.reselect")) AppDoc.reselect(); else AppDoc.reportUnavailable("select.reselect") }
    // 14b: Cmd+T enters transform.free on the active layer (the item-13
    // ground-truth correction of record: transform.free SERVES the full
    // affine — the frame WORKS; only the §11.1 lock-class wall fires,
    // and item-13's wall surface handles it). setTool both switches and
    // RE-ARMS an inactive frame on same-tool re-entry (A11Y-1 MAJOR-2
    // semantics); an already-active frame is a no-op. No focus steal —
    // the tool-letter chords don't steal either (recorded).
    // v1.0.1 item 11e FIX ROUND — ROUTED THROUGH THE FUNNEL. Ctrl+T is a
    // KEYMAP chord: it wrote canvas.tool directly, so it never set the
    // input-source mark and announceToolChange suppressed it whenever
    // focus sat inside a tool pane — measured: F6 into the palette,
    // Ctrl+T, tool changes, nothing spoken. That is the exact silence
    // hole ruling (e) exists to close, and closing it only for the
    // LETTER chords left it open on a shipping chord. The funnel also
    // carries the same-tool re-arm semantics the comment above
    // describes (it is a plain write; setTool decides), so the routing
    // changes the announcement and nothing else.
    Shortcut { sequence: "Ctrl+T"; onActivated: if (root.rowEnabled("transform.enter")) root.dispatchToolChord("transform.free"); else AppDoc.reportUnavailable("transform.enter") }
    // 14b: Cmd+J layer-via-copy (desk-ruled composition; both arms in
    // AppDoc.layerViaCopy — selection: app-side copy -> edit.paste new
    // layer; none: layer.duplicate).
    Shortcut { sequence: "Ctrl+J"; onActivated: if (root.rowEnabled("layer.viaCopy")) AppDoc.layerViaCopy(root.activeLayerId); else AppDoc.reportUnavailable("layer.viaCopy") }
    Shortcut { sequences: [StandardKey.ZoomIn]; onActivated: canvas.zoomIn() }
    Shortcut { sequences: [StandardKey.ZoomOut]; onActivated: canvas.zoomOut() }
    Shortcut { sequence: "Ctrl+0"; onActivated: canvas.zoomFit() }
    Shortcut { sequence: "Ctrl+1"; onActivated: canvas.zoomOneToOne() }
    // ---- ES:16452 single-key suppression + the tool-letter dispatch ----
    // ES:16452 [NORMATIVE]: "While a text/numeric field has focus or a
    // Type-tool editing session is active on canvas, single-key and
    // Shift+letter shortcuts (tool letters, brackets, digits-opacity, F
    // screen mode, Q quick mask, X/D) are suppressed — they insert
    // text. Modifier chords still dispatch (§30.2)."
    // Guard point = THIS dispatch layer, keyed on focus-item TYPE:
    // TextInput/TextEdit are the text-editing surfaces (TextField,
    // TextArea, SpinBox and editable ComboBox editors all carry one of
    // the two underneath) and both expose inputMethodComposing — no
    // other Quick item does. A guarded Shortcut goes DISABLED, leaving
    // the shortcut map entirely, so the letter DELIVERS to the focused
    // editor and inserts text (the spec's "they insert text" — an
    // activation no-op would consume the key and type nothing). The
    // modifier chords above carry no guard: they still dispatch (§30.2).
    // The canvas Type-tool editing-session arm has no live surface yet
    // (submitText commits empty text; the typing surface arrives with
    // the chrome mint batch) — when it lands as a focused text editor
    // it enters this same guard by type.
    // OBLIGATION, carried explicitly (11e fix round, §5 minor-3 — the
    // disclosure above is accurate, re-verified first-hand at this
    // ground: submitText still commits empty text and no typing surface
    // exists). ES:16452's second arm is "or a Type-tool editing session
    // is active on canvas". This guard is keyed on the ACTIVE FOCUS
    // ITEM, so an on-canvas Type session that keeps canvas focus would
    // NOT engage it and letters would switch tools mid-typing. Whoever
    // lands that session owns this: either give it a focused editor
    // (then it enters this guard by type, free) or extend this
    // predicate to ask the canvas whether a Type session is live. The
    // permanent no-yield-surface case in tst_chord_table already
    // exercises that CLASS, so the net is ready for it either way.
    readonly property bool textEditingFocus:
        !!root.activeFocusItem
        && ("inputMethodComposing" in root.activeFocusItem)
    // The keymap-dispatch entry point: every letter-chord tool change
    // routes through here and only here. 11e hand-off HONORED: the (e)
    // ruling's input-source distinction (chord-driven tool changes
    // ALWAYS announce, exempt from the focusInsideToolbar guard) keys on
    // THIS call site — never inline it back into a shortcut body.
    // chordDriven is a TRANSIENT mark, safe because QML signal delivery
    // is synchronous: toolChanged is emitted inside setTool
    // (canvas_view.cpp) and the Connections handler below runs in-line,
    // so the flag is still true when announceToolChange reads it and
    // false again before anything else can observe it.
    property bool chordDriven: false
    function dispatchToolChord(toolId) {
        root.chordDriven = true;
        canvas.tool = toolId;
        root.chordDriven = false;
        // ★★★★★ Q-D14 = VOICE (operator, 2026-08-30). `S` stays bound and
        // SPEAKS: "Silent tools are the disease this week cured; a bound key
        // that does nothing quietly is not an option."
        //
        // ★★★★ WHY HERE AND NOT AT THE KEY. writeTool() is the SOLE keymap
        // tool writer and funnels through this function -- so one call covers
        // every chord path including Ctrl+T, and it cannot drift out of sync
        // with the binding table the way a per-key hook would.
        //
        // ★★★ WHY statusPending AND NOT setStatus. statusPending is the
        // shell's own statement that a control is ARMED but its wiring has
        // not arrived, and it REFUSES an uncataloged id rather than echoing
        // it at a user. `ui.status.clone_needs_source` is cataloged in all
        // four locales; if it is ever removed this goes MUTE and testable
        // instead of printing a raw MessageId.
        //
        // ★★ SCOPE, STATED: this is the KEYMAP path, which is what Q-D14
        // asked about ("S selects a tool that paints nothing and says
        // nothing"). The palette slot and flyout select clone by a different
        // route and are NOT covered here -- recorded rather than silently
        // implied, because a partial fix described as complete is the shape
        // this project keeps catching.
        // ★★★★ Q-D21 RULED (a), 2026-09-01: UNIFY ON THIS SENTENCE. The id now
        // carries a {tool} placeholder, so this call MUST pass the arg — a
        // templated string invoked bare renders `{tool}` at the user, which is
        // the key-echo class the reasonId door exists to prevent. The display
        // name comes from the id the user pressed (toolNameKeys), never from
        // the engine kind, matching onArmedToolPressed below.
        // ★★★★ Q-D21 RULED (a), 2026-09-01: UNIFY ON THIS SENTENCE. The id
        // carries a {tool} placeholder, so this call MUST pass the arg — a
        // templated string invoked bare renders `{tool}` at the user, which
        // is the key-echo class the reasonId door exists to prevent. The
        // display name comes from the id the user pressed (toolNameKeys),
        // never from the engine kind.
        //
        // ★★★★★ RESTORED 2026-09-06, AND THE REMOVAL WAS MY ERROR. B-02
        // moved this call to the canvas's unarmed-paint signal, reasoning
        // that once clone WORKS the sentence is false at selection time.
        // That is wrong, and the ruling had already said so: Q-D21 (a) reads
        // "'Clone needs a source point' says WHY, which is strictly more
        // than 'not wired', AND IT STAYS TRUE AFTER PRIORITY 5 WIRES CLONE
        // … only the wording half moves." Clone genuinely does need a
        // source before it can paint, wired or not. What B-02 made
        // questionable is the catalog TAIL ("— not yet available"), which
        // is the i18n lane's wording and not this call's placement.
        // Two pins caught the removal — tst_palette_pins rule D4 and
        // tst_chord_table's chord sentence — and they were right.
        //
        // The unarmed-paint voice B-02 added is KEPT, as an ADDITION: it
        // fires on a different event (an attempted stroke with no anchor),
        // not on selection, so the two do not double-speak.
        if (toolId === "clone.stroke")
            AppDoc.statusPending("ui.status.clone_needs_source",
                                 {"tool": AppI18n.tr(root.toolNameKeys[toolId])});
    }
    // v1.0.1 item 11e: the X action chord (Appendix C tool.swap_colors;
    // §32.3's palette-foot swap). The swap ITSELF stays single-sourced
    // in the palette's own swapSwatches function — the chord and the
    // visible swap control run the same code, so the chord rides a
    // named, AT-discoverable control (house pattern). Announcement per
    // ruling (e): the ACTION is announced through existing ids, never a
    // per-color name (the shell has no color-name source — recorded
    // wall). Keymap-driven, so it announces unconditionally, exactly
    // like a chord-driven tool change; a distinct announcer source keeps
    // a swap from coalescing away a tool announcement.
    function dispatchColorSwap() {
        palettePane.swapSwatches();
        if (root.uiReady)
            AppA11y.announce("color", "a11y.tool.selected",
                             {"name": AppI18n.tr("ui.tool.swatch_swap")});
    }
    // ---- v1.0.1 item 11e: THE LETTER SHORTCUTS ARE GONE ON PURPOSE ----
    // The key layer landed eight press-only Shortcut objects here
    // (V M L W C T B E). 11e MIGRATED them into a real QKeyEvent
    // dispatcher (shell/src/tool_key_dispatcher.{h,cpp}, installed on
    // this window by main.cpp and by every Main.qml test harness),
    // because a Shortcut cannot see RELEASE (ES:16446 spring-load needs
    // hold-vs-tap), cannot see auto-repeat identity (ES:16453), and
    // cannot consult the ES:16446 capture gate at event time. The
    // letters had to LEAVE the shortcut map entirely for the dispatcher
    // to see their press/release at all — that is why these lines are
    // deleted rather than merely bypassed. textEditingFocus above is
    // still the ONE guard definition; the dispatcher consults it first
    // and declines WITHOUT consuming, so a suppressed letter still
    // inserts into the focused field (ES:16452's "they insert text").
    // The ruled table, its Appendix C citations and the S ring-of-one
    // deviation-by-silence record all live in the dispatcher's .cpp.

    // ---- A11Y-2 + v1.0.1 item 11d: the §31.2 pane cycle (keyboard-
    // scripts global rule 5). F6 forward / Shift+F6 backward over
    // [OPTIONS, PALETTE, CANVAS, PANELS] — a fixed navigation chord
    // dispatched as an application shortcut (before any tool keymap).
    // 11d Phase 1 (the D5/A11Y-5 closure, desk record a4510f0): the
    // options pane JOINS the cycle on O-QA's CORRECTED order — options
    // FIRST (KS-3 step 2 trace: canvas → Shift+F6 → palette → Shift+F6
    // → OPTIONS, keyboard-scripts.yaml:86; global rule 5's
    // options-before-palette at :25). Both prior records ([palette,
    // options, …]) were defective — correction owned in the lane log.
    // C4 RECORD (the accepted interpretation, so "conforms as written"
    // is not overstated): the spec's five-pane cycle BEGINS at the menu
    // bar; menu-bar-as-F6-pane-0 is realized OUT-OF-CYCLE via the
    // F10/Ctrl+F2 access chord (A11Y-5 hook B below) — the four
    // focusable panes here are the in-cycle mapping.
    readonly property var paneCycle: [toolbarPane, palettePane, canvas, panelsPane]
    // C2 operability probe: the pane's first focus-chain item that is a
    // GENUINE DESCENDANT — the same containment walk the review-D3
    // landing advance uses (one truth: a parallel visibility scan could
    // diverge from Qt's own chain). Probe QD: with no focusable child,
    // nextItemInFocusChain walks OUT to a neighbor pane — containment
    // failing IS the empty-pane signal. Returns null when the pane has
    // no operable visible focusable child.
    function paneFirstOperable(pane) {
        const first = pane.nextItemInFocusChain(true);
        if (!first || first === pane)
            return null;
        for (let n = first; n; n = n.parent)
            if (n === pane)
                return first;
        return null;
    }
    function cyclePane(step) {
        // activeFocus alone covers all four panes: QQC2 Control IS a
        // focus scope, so a focused child propagates (reviewer probe P5).
        let current = -1;
        for (let i = 0; i < paneCycle.length; ++i)
            if (paneCycle[i].activeFocus) {
                current = i;
                break;
            }
        // Review D4-R: focus OUTSIDE every pane (modal dialogs and other
        // overlay-hosted popups) means NO pane boundary is crossable —
        // the chord is a NO-OP, never a default-to-canvas steal (KS
        // rules 3+5). Detection via overlay-item properties categorically
        // cannot work (probe Q5: Overlay.overlay children are popup
        // ITEMS — QQuickPopupItem/dim rects — which expose no `modal`);
        // the no-pane-focus test needs no modality detection at all and
        // also covers future non-pane focus hosts.
        if (current < 0)
            return;
        // C2 SKIP RULE (O-QA concur condition): a cycle target ≠ canvas
        // with no operable visible focusable child is SKIPPED — keep
        // advancing in the SAME direction; the walk is bounded by
        // paneCycle.length and the canvas is always operable, so it
        // terminates trap-free. Focus is never parked on an empty
        // container. The empty-OPTIONS state is COMMON (~17 tools, most
        // optionless) — this rule is load-bearing, not an edge case.
        for (let k = 1; k <= paneCycle.length; ++k) {
            const next = ((current + step * k) % paneCycle.length
                          + paneCycle.length) % paneCycle.length;
            const pane = paneCycle[next];
            if (pane === canvas) {
                pane.forceActiveFocus(Qt.TabFocusReason);
                return;
            }
            const target = root.paneFirstOperable(pane);
            if (!target)
                continue; // C2: empty pane — same direction, next pane
            pane.forceActiveFocus(Qt.TabFocusReason);
            // Review D3: a first visit can land focus on the CONTAINER
            // (non-operable, invisible in the Basic style). Advance to
            // the first operable child — containment already proven by
            // paneFirstOperable. A pane that propagated focus to its
            // remembered child (focus-scope memory) is left alone.
            if (root.activeFocusItem === pane)
                target.forceActiveFocus(Qt.TabFocusReason);
            return;
        }
    }
    Shortcut {
        sequence: "F6"
        context: Qt.ApplicationShortcut
        onActivated: root.cyclePane(1)
    }
    Shortcut {
        sequence: "Shift+F6"
        context: Qt.ApplicationShortcut
        onActivated: root.cyclePane(-1)
    }
    // A11Y-5 hook B: in-window menu access — the KS harness config has
    // no native menu access, and §31.2's pane order BEGINS at the menu
    // bar (the A11Y-2 three-pane F6 cycle stands as recorded; the
    // access chord is not a cycle change). Review D1 disproved the
    // "focus arms native navigation" assumption at runtime: QQC2's
    // MenuBar keyboard-nav state does NOT arm on forceActiveFocus (the
    // first arrow is silently swallowed; bar-level Left/Right move an
    // internal index, not focus; Return is inert). What IS
    // deterministic (probe-verified): with a menu OPEN, in-menu
    // arrows/Return and between-menu Left/Right all work. So the chord
    // OPENS the first menu outright — the drivable sequence is:
    // F10 (File menu opens) → Left/Right to the target menu →
    // Down/Up to the item → Return. Review D2: with any menu already
    // open, the chord CLOSES it back to the canvas (no zombie-open
    // states). Review D3: Ctrl+F2 bound alongside — §49.1's macOS
    // MENU() chord (F10 stays for the other legs; both unbound in
    // Appendix C).
    function anyMenuOpen() {
        for (let i = 0; i < mainMenuBar.count; ++i) {
            const m = mainMenuBar.menuAt(i);
            if (m && m.opened)
                return true;
        }
        return false;
    }
    function closeAllMenus() {
        for (let i = 0; i < mainMenuBar.count; ++i) {
            const m = mainMenuBar.menuAt(i);
            if (m && m.opened)
                m.close();
        }
    }
    function toggleMenuAccess() {
        if (anyMenuOpen()) {
            closeAllMenus();
            canvas.forceActiveFocus(Qt.OtherFocusReason);
        } else if (mainMenuBar.activeFocus) {
            canvas.forceActiveFocus(Qt.OtherFocusReason);
        } else {
            // Review delta D1: menu.open() does NOT arm QQuickMenuBar's
            // menu-to-menu switching (Left/Right stayed dead after a
            // programmatic open). A REAL Space activation is the only
            // arming path (reviewer probes H1-H3): focus item 0, then
            // inject genuine Space — menu 0 opens AND the bar arms, so
            // the recorded sequence F10 → Left/Right between menus →
            // Down/Up in-menu → Return is true end to end.
            const first = mainMenuBar.itemAt(0);
            if (first) {
                first.forceActiveFocus(Qt.TabFocusReason);
                AppKeys.armMenuBar(root);
            }
        }
    }
    Shortcut {
        sequence: "F10"
        context: Qt.ApplicationShortcut
        onActivated: root.toggleMenuAccess()
    }
    Shortcut {
        sequence: "Ctrl+F2"
        context: Qt.ApplicationShortcut
        onActivated: root.toggleMenuAccess()
    }

    function saveOrSaveAs() {
        if (AppDoc.documentPath === "")
            saveFileDialog.open();
        else
            AppDoc.saveDocument("");
    }

    // v1.0.1 item 7: the Paste handler EXTRACTED (design rule: one
    // implementation) — the top Edit menu and the canvas context menu
    // both fire THIS (item-12 fit-and-center behavior unchanged).
    function pasteAndFit() {
        if (AppDoc.pasteFromClipboard())
            canvas.fitIfOverflow();
    }

    // v1.0.1 item 15 (the menu-mint follow-up remainder): THE Paste Into
    // handler — ONE implementation, fired by the top Edit menu row and
    // the canvas context-menu row alike (pasteAndFit's rule).
    // The controller arm submits edit.paste_into, whose engine
    // precondition is an ACTIVE SELECTION (§40.5 — the selection becomes
    // the pasted layer's mask). That precondition is the
    // "edit.pasteInto" availability row, so both consumers are already
    // disabled without one; a submission that reaches the engine anyway
    // gets its TYPED error.command.selection_required on the existing
    // ui.status.rejected line — never a fake success.
    // Viewport behavior is the twin's verbatim (item 12 fit-and-center);
    // no second implementation.
    function pasteIntoSelection() {
        if (AppDoc.pasteIntoFromClipboard())
            canvas.fitIfOverflow();
    }

    // v1.0.1 item 14b: THE Delete disposition — ONE implementation for
    // every consumer (the top-menu Delete row, the context-menu Delete
    // row, the canvas Delete/Backspace chord). Desk ruling: the edit.cut
    // ENGINE kind WITHOUT the app-side copy; selection active ->
    // IMMEDIATE (§13.1 coverage-scaled clear of the selected region);
    // no selection -> the §32.5 named-object destructive CONFIRM, then
    // the §40.5 whole-content clear; a transparency-lock refusal
    // surfaces typed on the existing ui.status.rejected path (the
    // item-13 wall class — locks are write-only pre-info-v2).
    function deleteLayerContentWithConfirm() {
        if (AppDoc.commandAvailability["edit.delete"].enabled !== true)
            return;
        if (AppDoc.selectionActive)
            AppDoc.deleteLayerContent(root.activeLayerId);
        else
            deleteConfirmDialog.openFor(root.activeLayerId);
    }

    // v1.0.1 item 7: the six live filter kinds — ONE array consumed by
    // the top Filter menu's Repeater AND the context menu's Filters ▸
    // submenu (same wire ids as catalog keys, same shared
    // filter.apply availability row).
    readonly property var liveFilterKinds: [
        "filter.gaussian_blur", "filter.box_blur", "filter.median",
        "filter.high_pass", "filter.add_noise", "filter.unsharp_mask"]

    // v1.0.1 item 7: open the canvas context menu at (x, y) in
    // canvas/container coordinates — the right-click surface passes the
    // press position; the §31.1 keyboard paths pass
    // canvas.contextMenuAnchor() (selection bounds center when ants are
    // visible, else the visible document center).
    function openCanvasContextMenuAt(x, y) {
        canvasContextMenu.x = x;
        canvasContextMenu.y = y;
        canvasContextMenu.open();
    }

    // ---- menus ----
    // v1.0.1 item 14a: every menu item's enabled truth binds the SINGLE
    // command-availability surface (AppDoc.commandAvailability — the
    // items-7+14 design entry) keyed by shell-internal action ids. The
    // per-item duplicated predicates collapsed BEHAVIOR-IDENTICALLY
    // (frozen verbatim + matrix-asserted in tst_command_availability);
    // the item-7 context menu and 14b shortcuts consume the same rows.
    // Disabled rows may carry reasonId (existing cataloged ids only) —
    // no menu tooltip consumer this slice (behavior-identical contract).
    menuBar: MenuBar {
        id: mainMenuBar
        // §31.2 symmetry: Esc from the focused bar (no menu open — an
        // open menu consumes Esc itself) returns to the canvas, the
        // same contract panels honor.
        Keys.onEscapePressed: canvas.forceActiveFocus(Qt.OtherFocusReason)
        Menu {
            title: AppI18n.tr("ui.menu.file")
            MenuItem { text: AppI18n.tr("menu.file.new"); onTriggered: newDialog.open() }
            MenuItem { text: AppI18n.tr("app.open"); onTriggered: openFileDialog.open() }
            MenuItem {
                // RC3 D-3: "Close" reuses the existing ui.dialog.close string
                // until a ui.menu.file.close key is minted (queued to O-I18N).
                objectName: "fileMenuClose"
                text: AppI18n.tr("ui.dialog.close")
                enabled: AppDoc.hasDocument
                onTriggered: root.closeDocumentGuarded()
            }
            MenuItem {
                text: AppI18n.tr("ui.menu.file.save")
                enabled: AppDoc.commandAvailability["file.save"].enabled === true
                onTriggered: root.saveOrSaveAs()
            }
            MenuItem {
                text: AppI18n.tr("ui.menu.file.save_as")
                enabled: AppDoc.commandAvailability["file.saveAs"].enabled === true
                onTriggered: saveFileDialog.open()
            }
            MenuSeparator {}
            // ABI 1.2 doors (version-gated per N.5 rule 2 — the abi
            // conjunct lives in the availability rows now).
            MenuItem {
                text: AppI18n.tr("ui.menu.file.import")
                enabled: AppDoc.commandAvailability["file.import"].enabled === true
                onTriggered: importFileDialog.open()
            }
            MenuItem {
                text: AppI18n.tr("menu.file.export_as")
                enabled: AppDoc.commandAvailability["file.export"].enabled === true
                onTriggered: exportFileDialog.open()
            }
        }
        Menu {
            title: AppI18n.tr("ui.menu.edit")
            // 14b: chord-paired items render their chords (ChordMenuItem
            // — measured: the Basic style renders no shortcut text of
            // its own). The FIRING chords stay the Shortcut objects
            // above (window scope) / the canvas key handler (Delete —
            // canvas scope); `chord` here is display only.
            ChordMenuItem { text: AppI18n.tr("ui.action.undo"); chord: "Ctrl+Z"; enabled: AppDoc.commandAvailability["edit.undo"].enabled === true; onTriggered: AppDoc.undo() }
            ChordMenuItem { text: AppI18n.tr("ui.action.redo"); chord: "Ctrl+Shift+Z"; enabled: AppDoc.commandAvailability["edit.redo"].enabled === true; onTriggered: AppDoc.redo() }
            MenuSeparator {}
            // §26.1 Cut, FLIPPED at ABI 1.4 (growth request 7 granted):
            // app-side Copy — per-layer readback, alpha scaled by the
            // selection's fractional coverage — then the edit.cut
            // mutation clears. The clipboard updates only when the
            // mutation applies, so a typed refusal (locked) cannot
            // clobber the previous clipboard. Pre-1.4 the item stays
            // DISABLED: absence beats a mutation-only Cut (review R3).
            ChordMenuItem {
                text: AppI18n.tr("ui.menu.edit.cut")
                chord: "Ctrl+X"
                enabled: AppDoc.commandAvailability["edit.cut"].enabled === true
                onTriggered: AppDoc.cutLayer(root.activeLayerId)
            }
            ChordMenuItem {
                text: AppI18n.tr("ui.menu.edit.copy")
                chord: "Ctrl+C"
                enabled: AppDoc.commandAvailability["edit.copy"].enabled === true
                onTriggered: AppDoc.copyCompositeToClipboard()
            }
            ChordMenuItem {
                text: AppI18n.tr("ui.menu.edit.paste")
                chord: "Ctrl+V"
                enabled: AppDoc.commandAvailability["edit.paste"].enabled === true
                // item 7: extracted handler (root.pasteAndFit) shared
                // with the context menu — behavior unchanged.
                onTriggered: root.pasteAndFit()
            }
            // item 15: Paste Into — the clipboard-into-selection door.
            // The id landed with the sealed menu-mint (3dc5c37); THIS
            // slice ships its handler (AppDoc.pasteIntoFromClipboard on
            // the edit.paste_into engine kind) and its availability row
            // (edit.pasteInto = doc && abiBlock12 && selectionActive —
            // the engine's own §40.5 selection precondition). No chord:
            // none is registered for it in the chord table, and a
            // display-only chord would name a binding that does not
            // exist.
            MenuItem {
                text: AppI18n.tr("ui.menu.edit.paste_into")
                enabled: AppDoc.commandAvailability["edit.pasteInto"].enabled === true
                onTriggered: root.pasteIntoSelection()
            }
            // 14b: the Delete row (ui.menu.edit.delete landed with the
            // sealed menu-mint 3dc5c37; the HANDLER is 14b's ruled
            // disposition — root.deleteLayerContentWithConfirm, the one
            // implementation the context menu and the canvas chord
            // share). Chord display: Backspace (the primary macOS key;
            // the canvas handler accepts Delete AND Backspace).
            ChordMenuItem {
                text: AppI18n.tr("ui.menu.edit.delete")
                chord: "Backspace"
                enabled: AppDoc.commandAvailability["edit.delete"].enabled === true
                onTriggered: root.deleteLayerContentWithConfirm()
            }
            MenuSeparator {}
            MenuItem {
                text: AppI18n.tr("ui.menu.edit.fill")
                enabled: AppDoc.commandAvailability["edit.fill"].enabled === true
                onTriggered: fillDialog.open()
            }
            // 1.0 final (corrigendum (l)): Edit > Puppet Warp is removed;
            // its solve has no backend in 1.0.
        }
        Menu {
            title: AppI18n.tr("ui.menu.select")
            ChordMenuItem {
                text: AppI18n.tr("ui.menu.select.all")
                chord: "Ctrl+A"
                enabled: AppDoc.commandAvailability["select.all"].enabled === true
                onTriggered: AppDoc.applyCommand("selection.all", {}, {}, "")
            }
            ChordMenuItem {
                text: AppI18n.tr("ui.menu.select.deselect")
                chord: "Ctrl+D"
                enabled: AppDoc.commandAvailability["select.deselect"].enabled === true
                onTriggered: AppDoc.applyCommand("selection.none", {}, {}, "")
            }
            ChordMenuItem {
                text: AppI18n.tr("ui.menu.select.invert")
                chord: "Ctrl+Shift+I"
                enabled: AppDoc.commandAvailability["select.invert"].enabled === true
                onTriggered: AppDoc.applyCommand("selection.invert", {}, {}, "")
            }
            MenuItem {
                text: AppI18n.tr("ui.menu.select.modify")
                enabled: AppDoc.commandAvailability["select.modify"].enabled === true
                onTriggered: modifyDialog.open()
            }
            MenuSeparator {}
            // Doors-flip wave mechanics get their chrome (A11Y-1): the
            // request-9 interim enablement truth rides the controller
            // (cursor-truth tracked channel; layer-alpha needs no
            // enumeration).
            MenuItem {
                text: AppI18n.tr("ui.menu.select.save_selection")
                enabled: AppDoc.commandAvailability["select.saveSelection"].enabled === true
                onTriggered: AppDoc.saveSelectionAsChannel()
            }
            MenuItem {
                text: AppI18n.tr("ui.menu.select.load_saved")
                enabled: AppDoc.commandAvailability["select.loadSaved"].enabled === true
                onTriggered: AppDoc.loadSavedChannelSelection()
            }
            MenuItem {
                text: AppI18n.tr("ui.menu.select.load_layer_alpha")
                enabled: AppDoc.commandAvailability["select.loadLayerAlpha"].enabled === true
                onTriggered: AppDoc.loadLayerAlphaSelection(root.activeLayerId)
            }
        }
        Menu {
            title: AppI18n.tr("ui.menu.image")
            MenuItem { text: AppI18n.tr("ui.menu.image.resize"); enabled: AppDoc.commandAvailability["image.resize"].enabled === true; onTriggered: resizeDialog.open() }
            MenuItem { text: AppI18n.tr("ui.menu.image.canvas_size"); enabled: AppDoc.commandAvailability["image.canvasSize"].enabled === true; onTriggered: canvasSizeDialog.open() }
            MenuItem { text: AppI18n.tr("ui.menu.image.rotate"); enabled: AppDoc.commandAvailability["image.rotate"].enabled === true; onTriggered: rotateDialog.open() }
            MenuItem { text: AppI18n.tr("ui.menu.image.trim"); enabled: AppDoc.commandAvailability["image.trim"].enabled === true; onTriggered: trimDialog.open() }
            MenuItem { text: AppI18n.tr("ui.menu.image.crop"); enabled: AppDoc.commandAvailability["image.crop"].enabled === true; onTriggered: cropDialog.open() }
            MenuSeparator {}
            // §17.2 Auto Tone / Auto Contrast / Auto Color. Their OWN
            // group: the rows above are DOCUMENT geometry, these three
            // bake per-channel levels into the ACTIVE LAYER's pixels
            // (and share one availability row for that reason).
            //
            // NO ELLIPSIS, and no dialog — each contract says
            // `actionRecording: recordable; no dialog (menu item)`, so
            // the row applies immediately through the one apply door.
            // The caption comes from the catalog whole: nine of the
            // thirteen cataloged ui.menu.image.* captions carry `…` and
            // every one of those opens a dialog; these three do not, and
            // concatenating one here would contradict the catalog.
            //
            // Empty target block by design: §40.2's family rule ignores
            // it (the edited layer is params.targetLayer, default =
            // active layer). Empty params = the contract defaults
            // (clipLow/clipHigh 0.001); no mergeKey (mergeKey: null).
            MenuItem {
                text: AppI18n.tr("ui.menu.image.auto_tone")
                enabled: AppDoc.commandAvailability["image.autoAdjust"].enabled === true
                onTriggered: AppDoc.applyCommand("image.auto_tone", {}, {}, "")
            }
            MenuItem {
                text: AppI18n.tr("ui.menu.image.auto_contrast")
                enabled: AppDoc.commandAvailability["image.autoAdjust"].enabled === true
                onTriggered: AppDoc.applyCommand("image.auto_contrast", {}, {}, "")
            }
            MenuItem {
                text: AppI18n.tr("ui.menu.image.auto_color")
                enabled: AppDoc.commandAvailability["image.autoAdjust"].enabled === true
                onTriggered: AppDoc.applyCommand("image.auto_color", {}, {}, "")
            }
            MenuSeparator {}
            MenuItem { text: AppI18n.tr("ui.menu.image.flatten"); enabled: AppDoc.commandAvailability["image.flatten"].enabled === true; onTriggered: flattenDialog.open() }
            MenuSeparator {}
            // 1.0 final (corrigendum (l), WORD 41 (2)): Image > Mode… is removed;
            // a Grayscale or Lab document cannot be exported (KNOWN-ISSUES 1).
            MenuItem { text: AppI18n.tr("ui.menu.image.assign_profile"); enabled: AppDoc.commandAvailability["image.assignProfile"].enabled === true; onTriggered: assignProfileDialog.open() }
            MenuItem { text: AppI18n.tr("ui.menu.image.convert_profile"); enabled: AppDoc.commandAvailability["image.convertProfile"].enabled === true; onTriggered: convertProfileDialog.open() }
            MenuItem { text: AppI18n.tr("ui.menu.image.depth_convert"); enabled: AppDoc.commandAvailability["image.depthConvert"].enabled === true; onTriggered: depthConvertDialog.open() }
        }
        // ★ THE G-17 KEYBOARD DOOR (Q-D72; operator WORD 8 of 2026-09-14
        //   18:48 ET, answer (3)). Six operations that until now existed ONLY as
        //   mouse targets in the layers panel.
        //
        //   ⛔ EVERY ITEM DELEGATES. Not one guard or argument is restated here:
        //   `enabled` reads the panel's own predicate and `onTriggered` calls the
        //   panel's own function, so the menu and the panel cannot disagree about
        //   what is possible or about what it does. Restating them would be two
        //   implementations of six guards with nothing keeping them equal.
        //
        //   The chords RENDER here and FIRE from the Shortcut objects above:
        //   ChordMenuItem's own converter Shortcut is enabled:false by design, so
        //   this introduces no ambiguous duplicate sequence.
        Menu {
            title: AppI18n.tr("ui.menu.layer")
            objectName: "menuLayer"
            MenuItem {
                objectName: "menuLayerDelete"
                text: AppI18n.tr("ui.menu.layer.delete")
                enabled: layersPanel.canDelete
                onTriggered: layersPanel.doDelete()
            }
            MenuSeparator {}
            // 1.0 final (corrigendum (l)): Group / Ungroup are removed with
            // their Ctrl+G / Ctrl+Shift+G chords.
            ChordMenuItem {
                objectName: "menuLayerMergeDown"
                text: AppI18n.tr("ui.menu.layer.merge_down"); chord: "Ctrl+E"
                enabled: layersPanel.canMergeDown
                onTriggered: layersPanel.doMergeDown()
            }
            MenuSeparator {}
            ChordMenuItem {
                objectName: "menuLayerMoveUp"
                text: AppI18n.tr("ui.menu.layer.move_up"); chord: "Ctrl+]"
                enabled: layersPanel.canMoveUp
                onTriggered: layersPanel.doMoveUp()
            }
            ChordMenuItem {
                objectName: "menuLayerMoveDown"
                text: AppI18n.tr("ui.menu.layer.move_down"); chord: "Ctrl+["
                enabled: layersPanel.canMoveDown
                onTriggered: layersPanel.doMoveDown()
            }
        }
        Menu {
            title: AppI18n.tr("ui.menu.filter")
            // Six numeric-param filters ride the shared table-driven
            // dialog; the filter NAMES are their wire ids as catalog
            // keys (the registry family). 1.0 final (corrigendum (l)):
            // the always-disabled Liquify and Custom items are removed.
            Repeater {
                // item 7: the kind list single-sourced (root
                // .liveFilterKinds) with the context menu's Filters ▸.
                model: root.liveFilterKinds
                MenuItem {
                    required property string modelData
                    text: AppI18n.tr(modelData)
                    // The six live kinds share ONE availability row.
                    enabled: AppDoc.commandAvailability["filter.apply"].enabled === true
                    onTriggered: filterParamDialog.openFor(modelData)
                }
            }
        }
        // B-09. The About dialog existed and was reachable from NOWHERE —
        // compiled into every binary, instantiated zero times (measured
        // C-713 with a DiscardConfirmDialog control at 1). Its acceptance
        // clause is "About shows all", and a dialog with no door shows
        // nothing. This is the door.
        //
        // A Help menu rather than the macOS application menu: this shell
        // uses a QQC2 MenuBar, which does not perform the platform's
        // About-role relocation, so an item placed for that convention
        // would simply not appear. Help is also what B-10 needs on BOTH
        // platforms.
        //
        // NO `enabled:` binding. Every other MenuItem here gates on
        // AppDoc.commandAvailability — About is not a command, takes no
        // document, and cannot be unavailable; a false availability row
        // would be the D4 lie in reverse (a control that is disabled for
        // no reason).
        Menu {
            title: AppI18n.tr("ui.menu.help")
            MenuItem {
                objectName: "helpAboutItem"
                text: AppI18n.tr("ui.menu.help.about")
                onTriggered: aboutDialog.open()
            }
            // WORD 3's fallback, built FIRST at the operator's instruction:
            // "it is an hour and it is the fallback either way". If B-09's
            // display half is not certified by 09-16 18:00 ET, THIS is what
            // ships in its place; if it IS certified, this still stands as
            // the door to the full texts the dialog only summarises.
            //
            // VISIBLE ONLY WHEN THE FOLDER EXISTS. AppLicensesDir is empty
            // when no candidate resolved, and the `typeof` guard is not
            // decoration: the Main.qml harness suites boot this file with no
            // host, so an unguarded reference throws ReferenceError in every
            // one of them — measured, three suites at once, when I wired
            // AppBuildCommit without it.
            MenuItem {
                objectName: "helpOpenLicensesItem"
                readonly property string dir:
                    (typeof AppLicensesDir === "string") ? AppLicensesDir : ""
                visible: dir !== ""
                text: AppI18n.tr("ui.menu.help.open_licenses")
                onTriggered: Qt.openUrlExternally("file://" + dir)
            }
        }
    }

    // ---- contextual options strip: the OPTIONS BAR ----
    // v1.0.1 item 11d Phase 1: the flat per-tool control list became a
    // DECLARATIVE PER-TOOL OPTIONS MODEL — rows keyed by tool wire id,
    // each an ordered list of control descriptors
    // {kind: slider|spinbox|checkbox|segment|swatch|toggle, captionId,
    //  binding, range/options} — rendered by ONE Repeater below (the
    // FilterParamDialog table idiom). Tools absent from the model show
    // an EMPTY bar (the honest common case; the pane cycle's C2 rule
    // skips it). The toolbarPane id + header housing stay load-bearing:
    // the §31.3 no-double-speak guard walks it (focusInsideToolbar
    // above), and Repeater-created controls are ordinary visual children
    // of the RowLayout — IN the parent-chain walk; NO Loader/popup
    // indirection (the recorded D4 overlay class).
    //
    // HONEST-STATE dispositions (D4 rule: a control that does nothing is
    // a lie; ships-only-with-live-wiring), each RESERVED id stays
    // cataloged-unemitted with its trigger recorded in the lane log:
    //  - stroke_width: SHIPS WIRED as of Pro-Wave row 4. Its recorded
    //    trigger has FIRED: the compositor's shape leg
    //    (resolve_parametric_page) no longer refuses widthPx > 0, and
    //    the flip was verified FIRST-HAND at the door that used to fail
    //    — composite readback AND the exported PNG, on pixels
    //    (tst_options_bar strokeSurvivesCompositeAndExportDoors). Only
    //    the WIDTH ships: every other §20.2 vectorStroke knob is HELD
    //    (see below).
    //
    // HELD by row 4, in THREE buckets — the bucket is the disposition,
    // so it is drawn from what the RENDERER does, never from what the
    // parser accepts (a parser accepting a param is not the engine
    // serving it; the fix round re-derived every field below by driving
    // it and reading pixels back):
    //  - HELD ON LABELS ONLY — stroke PAINT (`color`/`fill`), `join`,
    //    `miterLimit`, `align`: the engine SERVES these at the renderer,
    //    measured on this row's own geometry. align center/inside/
    //    outside give three DISTINCT bands; join miter/bevel/round give
    //    three distinct corners; miterLimit 4 vs 1 differ (1 falls back
    //    to bevel); stroke paint honours `color`, honours `fill`, and
    //    `fill` wins when both are present. What is missing is only the
    //    LABEL: no cataloged id exists for those captions or for their
    //    enum VALUE labels (ui.options.stroke_{color,join,miter,align},
    //    ui.join.*, ui.align.* are all absent from en/es/ja/pseudo).
    //    Catalog::tr KEY-ECHOES an uncataloged id, so shipping one would
    //    print a raw id at the user. These are genuine shipping
    //    candidates the day their ids land; a HOLD test reds then.
    //  - HELD BECAUSE IT IS INERT HERE — `cap`: NOT a labelling hold.
    //    stroke.rs polyline_pieces emits cap geometry only inside
    //    `if !poly.closed`, and every `primitive` kind vector.shape_add
    //    can build (rect, rounded_rect, ellipse, polygon, line — `custom`
    //    refuses typed) comes back CLOSED from primitives.rs, which wraps
    //    each constructor in `close()`. Measured: butt / round / square
    //    at width 24 give BYTE-IDENTICAL 120x120 rasters over the whole
    //    top-left corner of a shape.rect. A cap control on shape.rect or
    //    shape.ellipse could not move one pixel — the D4 lie exactly.
    //    Minting ui.cap.* must NOT be read as unblocking this row. `cap`
    //    IS live on an OPEN subpath (driven through this same command's
    //    `path` form with closed:false: butt/round/square give three
    //    distinct rasters), so it belongs to whatever row ships an open-
    //    path tool, not to this one.
    //  - HELD BECAUSE THE ENGINE REFUSES — `dashes`, `dashOffset`,
    //    `profile`: refused typed (error.command.not_implemented, §20.2
    //    rules 4 and 6) — a control would be a wall with a knob on it.
    //  - vector.pen carries NO stroke row: its commit mode toggles
    //    between vector.shape_add (stroked) and vector.path_new (no
    //    stroke field at all), and the declarative model has no
    //    conditional row — a width control that does nothing in path
    //    mode is exactly the D4 lie.
    //  - font_family / font_style: RESERVED — the text door VALIDATES
    //    style.font then DISCARDS it ("font does not resolve in v1";
    //    CharStyle carries no font field; name matching is the recorded
    //    C-6 arrival) — an emitted family changes nothing.
    //  - puppet_density: RESERVED — the value ids (fewer/normal/more)
    //    are unminted; wire-enum values need minted ids (the ui.mode.*/
    //    ui.warp.* discipline), queued for O-I18N.
    //  - warp_bend: RESERVED — warp has no live options-bar session.
    readonly property var optRowBrushSize: ({
        kind: "slider", captionId: "ui.options.brush_size",
        binding: "brushSize", min: 1, max: 200, step: 1, valueLabel: "round"
    })
    // Tier-1 row 5 (operator, 2026-09-04): ONE shared sample-scope row
    // for the three tools the order names — wand, heal, clone (the
    // optRowBrushSize sharing precedent; the id was minted for this at
    // 11d Phase-2 and has waited cataloged-unemitted since). The bound
    // property is INTENT; each emission site wires it per its own
    // contract (C-666 findings: wand and spot heal emit the boolean,
    // clone emits source:"composite" + the boolean — the §45.2 "all").
    readonly property var optRowSampleAll: ({
        kind: "checkbox", captionId: "ui.options.sample_all_layers",
        binding: "sampleAllLayers"
    })
    // Pro-Wave row 4 — §20.2 `stroke.widthPx` on vector.shape_add.
    // FRACTIONAL spinbox (the font_size shape): the wire type is
    // `number`, and sub-pixel widths are meaningful geometry, so an
    // integer control would silently coarsen the contract. min 0 is the
    // schema's own `minimum: 0` AND the engine's no-visible-stroke arm;
    // max 200 is a PRESENTATION election — the contract pins no maximum
    // (vector.shape_add.yaml:139 "no range pinned for VectorStroke"),
    // and 200 mirrors optRowBrushSize's own presentation ceiling on the
    // same kind of canvas-pixel quantity. A width past what the engine's
    // band-plane ceiling can serve over the drawn bounds refuses TYPED
    // and lands on ui.status.rejected.
    readonly property var optRowShapeStrokeWidth: ({
        kind: "spinbox", captionId: "ui.options.stroke_width",
        binding: "shapeStrokeWidth", min: 0, max: 200, step: 1,
        integer: false
    })
    readonly property var toolOptionsModel: ({
        "brush.stroke": [
            root.optRowBrushSize,
            // Item 4 (the FI-9 payoff): the hardness override rides
            // brush.stroke only (contract: hardness 0..=1 optional,
            // "absent = preset hardness", symmetric with sizePx;
            // eraser.stroke's contract is correctly un-extended, so the
            // row must NOT join eraser's list). O-29: an UNTOUCHED row
            // emits no `hardness` at all, so the engine's preset
            // hardness governs; the displayed 0.0 equals the built-in
            // soft-round-pressure preset's own hardness (preset.rs
            // §15.7), which is why omitting changes no rendered pixel
            // today. Announcement: the labeled
            // control's own accessible value is the §31.1 coverage
            // (a11y sibling surveyed NOT minted — MINTED.md; one-id
            // follow-up only if 11e's {/} chords prove silent).
            { kind: "slider", captionId: "ui.options.brush_hardness",
              binding: "brushHardness", min: 0, max: 1, step: 0.01,
              valueLabel: "fixed2" },
            // A11Y-5 close carried over: the swatch is a real focusable
            // button (FillDialog conversion shape, borderInput boundary).
            { kind: "swatch", captionId: "ui.options.brush_color",
              binding: "brushColor" }
        ],
        "eraser.stroke": [root.optRowBrushSize],

        // ★★★★★ THE STROKE TOOLS THAT COULD NOT BE SIZED (operator-hit Sunday;
        // the B -> size -> J workaround was the symptom). The options bar is
        // `toolOptionsModel[canvas.tool]` (:1048) and falls back to `[]`, so a
        // tool with no key here renders an EMPTY bar rather than a wrong one --
        // which is why this was invisible: nothing was broken on screen, there
        // was simply nothing on screen.
        //
        // DERIVED, NOT LISTED. The set is every PALETTE id that (a) reaches the
        // engine -- kModeTools or the no-mode list in canvas_view.cpp -- and
        // (b) whose CONTRACT carries sizePx, AND (c) had no options row yet.
        // ★ (c) ADDED C-644, RE-DERIVING MY OWN CLAIM: (a)+(b) alone is NINE --
        // kModeTools holds six (dodge/burn/SPONGE, blur/sharpen, heal) and the
        // no-mode list three (brush, eraser, smudge), and every one of those
        // wire kinds carries sizePx. brush.stroke and eraser.stroke already had
        // rows and were never the defect, so the fix set is 9-2 = 7. The number
        // was right and the two-condition derivation printed above it was not:
        // read literally it yields 9. Same class the peer sweeps have been
        // finding in shell/tests -- a CLAIM rotting while its citations hold.
        // That is 7 tools; the dispatch named
        // 4 of them, and blur_sharpen.blur/.sharpen/smudge.stroke were the same
        // defect unnamed. select.*/shape.*/transform.* are correctly absent:
        // they take no sizePx and are not stroke tools.
        //
        // ★★★★ SIZE ONLY, AND THAT IS THE CONTRACT'S RULING NOT A PREFERENCE:
        //   hardness  brush.stroke ALONE (the :977 note already says eraser's
        //             contract is "correctly un-extended"; none of these extend
        //             it either, so the row must not travel).
        //   color     brush.stroke ALONE. heal.stroke and smudge.stroke DO
        //             match /color/ -- in their TITLE and a DESCRIPTION, not in
        //             a param. brush.stroke's is `required: [brushPreset,
        //             color]` with a real $defs/color. A looser read adds two
        //             controls that no schema accepts.
        // Control for both claims: this probe reproduces the SHIPPED rows above
        // exactly -- brush yes/yes/yes, eraser yes/no/no.
        // 1.0 final S-08 (O-SIT RC5 sitting, ★): heal's Sample All Layers had
        // no effect (0 px changed with it on, on an empty top layer) -- the
        // engine's heal samples the active layer. Removed; clone keeps it.
        "heal.stroke":          [root.optRowBrushSize],
        "dodge_burn.dodge":     [root.optRowBrushSize],
        "dodge_burn.burn":      [root.optRowBrushSize],
        "dodge_burn.sponge":    [root.optRowBrushSize],
        "blur_sharpen.blur":    [root.optRowBrushSize],
        "blur_sharpen.sharpen": [root.optRowBrushSize],
        "smudge.stroke":        [root.optRowBrushSize],
        // ★★ clone.stroke IS NAMED IN THE DISPATCH AND IS THE ONE ROW I WOULD
        // HAVE QUESTIONED: its contract carries sizePx, but it is in NEITHER
        // kModeTools NOR the no-mode list, so it does not dispatch and paints
        // nothing (Q-D14). A size slider on an inert tool is a working-looking
        // control. Added as directed, and it is not silent about itself: the
        // item 25b sweep still says "Clone is not wired to the canvas yet" on
        // press, so the affordance is bounded by an existing statement.
        // Row 5 rides the same bounded affordance: the state binds and
        // the builder arm reads it the day clone dispatches (Q-D14).
        "clone.stroke":         [root.optRowBrushSize, root.optRowSampleAll],
        "select.wand": [
            { kind: "spinbox", captionId: "ui.options.wand_tolerance",
              binding: "wandTolerance", min: 0, max: 255, integer: true },
            { kind: "checkbox", captionId: "ui.options.wand_contiguous",
              binding: "wandContiguous" }
            // 1.0 final D-8: no Sample All row; the wand always samples
            // the composite (canvas_view.cpp).
        ],
        // Pen: the EXISTING (previously unexposed) commit-mode state —
        // vector.shape_add (inline path) vs vector.path_new.
        "vector.pen": [
            { kind: "segment", captionId: "ui.options.commit_as",
              binding: "penCommitsShape", options: [
                  { captionId: "ui.options.commit_shape", value: true },
                  { captionId: "ui.options.commit_path", value: false }
              ] }
        ],
        // Text: style.size (0.1..=1296 pt, text.add.yaml) — consumed by
        // the door; default 12 = the engine's own default run size.
        "text": [
            { kind: "spinbox", captionId: "ui.options.font_size",
              binding: "textFontSize", min: 0.1, max: 1296, step: 1,
              integer: false }
        ],
        // Puppet: the §45.5 knobs (contract ranges/defaults) + the
        // D2-ruled presentation-only mesh overlay toggle.
        "transform.puppet": [
            { kind: "spinbox", captionId: "ui.options.puppet_expansion",
              binding: "puppetExpansion", min: 0, max: 32, integer: true },
            { kind: "slider", captionId: "ui.options.puppet_rigidity",
              binding: "puppetRigidity", min: 0, max: 1, step: 0.01,
              valueLabel: "fixed2" },
            { kind: "toggle", captionId: "ui.options.puppet_show_mesh",
              binding: "puppetShowMesh" }
        ],
        // Pro-Wave row 4: the two tools that ALWAYS emit
        // vector.shape_add (submitShapePrimitive). vector.pen is
        // deliberately absent — see the HELD list above.
        "shape.rect": [root.optRowShapeStrokeWidth],
        "shape.ellipse": [root.optRowShapeStrokeWidth]
    })
    header: ToolBar {
        id: toolbarPane
        // C3 (11d Phase 1): the §31.2 Esc-to-canvas contract the other
        // cycle panes already honor — same shape as panelsPane's (KS-3
        // step 5 regresses without it).
        Keys.onEscapePressed: canvas.forceActiveFocus(Qt.OtherFocusReason)
        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: 8
            anchors.rightMargin: 8
            spacing: 4

            Repeater {
                id: optionsRepeater
                model: root.toolOptionsModel[canvas.tool] !== undefined
                    ? root.toolOptionsModel[canvas.tool] : []
                delegate: RowLayout {
                    id: optRow
                    required property var modelData
                    spacing: 4

                    // Caption for captioned kinds.
                    // ⛔ CORRECTED 2026-09-01: this comment used to say
                    // "checkbox/toggle/swatch carry their own labels." TRUE of
                    // checkbox and toggle -- both set `text:` below. FALSE of
                    // the SWATCH, which sets only `Accessible.name` and draws a
                    // bare Rectangle as its contentItem, so its caption reaches
                    // a screen reader and never reaches a pixel.
                    //
                    // ★★ THE MISSING CAPTION IS NOT ASSUMED TO BE A DEFECT. A
                    // colour swatch is conventionally unlabeled in an image
                    // editor and the affordance is the colour itself; that is a
                    // design call and it is left alone. What was wrong was the
                    // COMMENT, which told a reader auditing "does every
                    // captioned control show its caption?" to answer yes.
                    //
                    // ★ AND THE CONSEQUENCE WORTH KNOWING: `ui.options.brush_color`
                    // is minted, cataloged in four locales, counted EMITTED by
                    // i18n/tools/audit.py, and asserted by tst_options_bar --
                    // four green signals for a string that never renders. The
                    // audit's "emitted" means the id appears in a source file;
                    // it cannot distinguish a visible label from a
                    // screen-reader-only one. Verified by opening 01-launch.png.
                    Label {
                        visible: optRow.modelData.kind === "slider"
                              || optRow.modelData.kind === "spinbox"
                              || optRow.modelData.kind === "segment"
                        text: AppI18n.tr(optRow.modelData.captionId)
                    }
                    Slider {
                        id: optSlider
                        visible: optRow.modelData.kind === "slider"
                        from: visible ? optRow.modelData.min : 0
                        to: visible ? optRow.modelData.max : 1
                        stepSize: visible ? optRow.modelData.step : 0
                        value: visible ? canvas[optRow.modelData.binding] : 0
                        Layout.preferredWidth: 110
                        onMoved: canvas[optRow.modelData.binding] = value
                        // A11Y-3: associate the visible caption (§31.1
                        // slider row).
                        Accessible.name: AppI18n.tr(optRow.modelData.captionId)
                    }
                    Label {
                        visible: optSlider.visible
                        text: optRow.modelData.valueLabel === "fixed2"
                            ? optSlider.value.toFixed(2)
                            : Math.round(optSlider.value)
                    }
                    // Integer spinbox: the pre-11d wand-tolerance shape
                    // VERBATIM (T6 rehouse parity — default validator and
                    // text conversion untouched).
                    SpinBox {
                        visible: optRow.modelData.kind === "spinbox"
                              && optRow.modelData.integer === true
                        from: visible ? optRow.modelData.min : 0
                        to: visible ? optRow.modelData.max : 0
                        value: visible ? canvas[optRow.modelData.binding] : 0
                        editable: true
                        onValueModified: canvas[optRow.modelData.binding] = value
                        // A11Y-3: associate the visible caption (§31.1
                        // spinbox row).
                        Accessible.name: AppI18n.tr(optRow.modelData.captionId)
                    }
                    // Fractional spinbox: the FilterParamDialog x100
                    // presentation-scale shape (contract exactness at the
                    // control — DoubleValidator keeps '.' typeable).
                    SpinBox {
                        id: optSpinReal
                        visible: optRow.modelData.kind === "spinbox"
                              && optRow.modelData.integer === false
                        from: visible ? Math.round(optRow.modelData.min * 100) : 0
                        to: visible ? Math.round(optRow.modelData.max * 100) : 0
                        stepSize: visible
                            ? Math.max(1, Math.round(optRow.modelData.step * 100)) : 1
                        value: visible
                            ? Math.round(canvas[optRow.modelData.binding] * 100) : 0
                        editable: true
                        onValueModified: canvas[optRow.modelData.binding] = value / 100.0
                        validator: DoubleValidator {
                            bottom: optSpinReal.visible ? optRow.modelData.min : 0
                            top: optSpinReal.visible ? optRow.modelData.max : 1
                            decimals: 2
                        }
                        textFromValue: function(v) { return (v / 100.0).toFixed(2); }
                        valueFromText: function(t) { return Math.round(parseFloat(t) * 100); }
                        Accessible.name: AppI18n.tr(optRow.modelData.captionId)
                    }
                    CheckBox {
                        visible: optRow.modelData.kind === "checkbox"
                        text: AppI18n.tr(optRow.modelData.captionId)
                        checked: visible && canvas[optRow.modelData.binding]
                        onToggled: canvas[optRow.modelData.binding] = checked
                    }
                    Switch {
                        visible: optRow.modelData.kind === "toggle"
                        text: AppI18n.tr(optRow.modelData.captionId)
                        checked: visible && canvas[optRow.modelData.binding]
                        onToggled: canvas[optRow.modelData.binding] = checked
                    }
                    // Segment: bound-state buttons. NON-checkable by
                    // design — the checked VISUAL is pure bound state, so
                    // interaction can never break the binding (the QQC2
                    // checkable-interaction trap); a click writes canvas
                    // state and both buttons re-evaluate.
                    RowLayout {
                        visible: optRow.modelData.kind === "segment"
                        spacing: 4
                        Repeater {
                            model: optRow.modelData.kind === "segment"
                                ? optRow.modelData.options : []
                            delegate: ToolButton {
                                required property var modelData
                                text: AppI18n.tr(modelData.captionId)
                                checked: canvas[optRow.modelData.binding]
                                    === modelData.value
                                onClicked: canvas[optRow.modelData.binding]
                                    = modelData.value
                            }
                        }
                    }
                    ToolButton {
                        implicitWidth: 26
                        implicitHeight: 26
                        visible: optRow.modelData.kind === "swatch"
                        Accessible.name: AppI18n.tr(optRow.modelData.captionId)
                        onClicked: colorDialog.open()
                        contentItem: Rectangle {
                            radius: 3
                            color: optRow.modelData.kind === "swatch"
                                ? canvas[optRow.modelData.binding] : "transparent"
                            border.color: Theme.borderInput
                        }
                    }
                }
            }

            Item { Layout.fillWidth: true }
        }
    }

    RowLayout {
        anchors.fill: parent
        spacing: 0

        // v1.0.1 item 11b: the grouped tool palette, docked left of the
        // canvas (§31.2: palette before canvas — mirrored by its slot in
        // paneCycle above).
        ToolPalette {
            id: palettePane
            canvasItem: canvas
            Layout.fillHeight: true
        }

        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true

            CanvasView {
                id: canvas
                objectName: "canvasView"
                anchors.fill: parent
                controller: AppDoc
                focus: true
                // A11Y-3: §31.1 canvas row — named by the document title,
                // described by the status bar. (The window title already
                // carries doc name + placeholder branding; zoom is in the
                // status text.)
                Accessible.role: Accessible.Canvas
                Accessible.name: AppDoc.windowTitle
                Accessible.description: AppDoc.statusText
                // v1.0.1 item 7 (§31.1 a11y bar): Shift+F10 AND the
                // Menu key open the canvas context menu at the keyboard
                // anchor — canvas-focus scope (the menu belongs to the
                // focused canvas). Keys attached runs BeforeItem, so
                // the C++ tool keys (Space/Esc/Enter/…) are untouched:
                // only these two chords are accepted here. Shift+F10 is
                // a distinct sequence from the F10 menu-access chord.
                // §5 fix round: the chords are IGNORED while a gesture
                // is in flight — same contract as the pointer guard.
                // PROBED FIRST-HAND (unguarded tree, real events): a
                // keyboard open mid-drag does NOT steal the mouse grab
                // — the marquee survived byte-identically (mask ==
                // control) — so this arm is NOT gesture rescue. It is
                // guarded anyway because the open itself is the hazard:
                // the popup takes keyboard focus mid-drag, so Enter can
                // fire a row DURING the gesture (e.g. Paste swaps the
                // active layer mid-MoveDrag and the release's move then
                // targets the pasted layer) or open a MODAL dialog
                // mid-stroke (the grab steal one level removed), and
                // the selection-switched sections flip live under the
                // open popup the moment the release lands. Pre-item-7
                // these chords did nothing; item 7's design added the
                // SERIAL keyboard open only.
                Keys.onPressed: function(event) {
                    if (event.key === Qt.Key_Menu
                        || (event.key === Qt.Key_F10
                            && (event.modifiers & Qt.ShiftModifier))) {
                        if (canvas.gestureInFlight()) {
                            event.accepted = true;
                            return;
                        }
                        const p = canvas.contextMenuAnchor();
                        root.openCanvasContextMenuAt(p.x, p.y);
                        event.accepted = true;
                    }
                }
                // 14b: the plain Delete/Backspace chord (canvas scope;
                // emitted by keyPressEvent with the edit.delete row
                // already consumed) runs THE disposition — the same
                // function the two Delete menu rows fire.
                onDeleteChordPressed: root.deleteLayerContentWithConfirm()
            }

            // Drag-drop import (ABI 1.2 raster ingress; journaled paste).
            DropArea {
                anchors.fill: parent
                // ALWAYS ENABLED, deliberately (honest-state rule D4).
                // Previously `enabled: AppDoc.hasDocument && AppDoc.abiBlock12`,
                // which made a drop onto a documentless window do NOTHING AT ALL
                // -- no ink, no message, no feedback. A silent surface IS the
                // defect the rule names: the user cannot tell "refused" from
                // "broken" from "I missed the target". The precondition is
                // unchanged; it is now CHECKED INSIDE and ANSWERED, instead of
                // being spelled as an absence.
                enabled: true
                onDropped: function(drop) {
                    if (!drop.hasUrls)
                        return;
                    if (!AppDoc.hasDocument || !AppDoc.abiBlock12) {
                        // ui.error.no_document is ALREADY MINTED ("No document is
                        // open") and is the same id the availability rows use for
                        // this exact conjunct — document_controller.cpp's
                        // `reasonId policy` block and the `noDocId` it derives
                        // (:707-720 today). RE-ANCHORED C-643 BY CONTENT, NOT
                        // RENUMBERED: the citation read :686-688, which is now the
                        // tail of a save function (`return true; }`), and the rows
                        // using `doc && abiBlock12()` sit at :738-739.
                        // so the drop surface and the menus give ONE answer rather
                        // than two wordings for one condition. No new mint was
                        // needed -- checked before asking for one.
                        //
                        // Channel: statusPending is the QML door onto ui.status.*
                        // opened by the item-25b honesty sweep for exactly this
                        // shape, and it is CATALOG-GATED, so a typo fails silent
                        // and testable instead of key-echoing a raw id at a user.
                        // NOT ui.status.rejected with code+message: that is the
                        // ENGINE's rejection shape, and nothing reached the engine
                        // here -- claiming it refused would be a lie in the polite
                        // direction.
                        AppDoc.statusPending("ui.error.no_document");
                        drop.accept();
                        return;
                    }
                    for (let i = 0; i < drop.urls.length; ++i)
                        if (AppDoc.importImage(drop.urls[i].toString()))
                            canvas.fitIfOverflow(); // item 12
                    drop.accept();
                }
            }

            // v1.0.1 item 7: the right-click surface. The canvas item
            // accepts Left|Middle buttons only (canvas_view.cpp), so
            // this overlay receives exactly the right-button presses;
            // SERIAL tool gestures pass through untouched. The menu
            // opens ON PRESS (the macOS convention; the grab keeps the
            // paired release out of the overlay). Availability rows
            // gate every item, so a no-document open is honest (all
            // rows disabled), same as the top menus.
            //
            // §5 fix round (review F1, MAJOR): CONCURRENT input is the
            // guarded case — Qt 6 delivers a right press through the
            // item stack even mid-left-drag (macOS: a two-finger tap
            // while dragging); accepting it let the popup steal the
            // exclusive grab, the canvas never saw the left release,
            // and the marquee died silently. The guard REFUSES while
            // any other button is down (the press event's own buttons
            // mask — a mid-drag right press carries the held button)
            // or a button-held canvas gesture is in flight (canvas
            // truth, polled live — covers pan/stroke/drag uniformly).
            // Refusal = mouse.accepted = false: the MouseArea takes no
            // part (it must not become the grabber), the press falls
            // through to the canvas, which ignores non-left/middle —
            // the pre-item-7 contract: a mid-drag right press is
            // IGNORED ENTIRELY, no deferred open.
            MouseArea {
                anchors.fill: parent
                acceptedButtons: Qt.RightButton
                onPressed: function(mouse) {
                    if ((mouse.buttons & ~Qt.RightButton)
                        || canvas.gestureInFlight()) {
                        mouse.accepted = false;
                        return;
                    }
                    root.openCanvasContextMenuAt(mouse.x, mouse.y);
                }
            }

            // item 7: the context menu itself — declared inside the
            // canvas container so its popup coordinates share the
            // canvas coordinate space both open paths use.
            CanvasContextMenu {
                id: canvasContextMenu
                activeLayerId: root.activeLayerId
                canvasTool: canvas.tool
                filterKinds: root.liveFilterKinds
                onPasteRequested: root.pasteAndFit()
                // item 15: the same ONE handler the top Edit menu fires.
                onPasteIntoRequested: root.pasteIntoSelection()
                onFillRequested: fillDialog.open()
                // 14b: the Delete row relays to the ONE disposition
                // (selection section => the immediate arm by
                // construction — same function regardless).
                onDeleteRequested: root.deleteLayerContentWithConfirm()
                onModifyRequested: function(opParam) {
                    modifyDialog.openFor(opParam);
                }
                onFilterRequested: function(filterKind) {
                    filterParamDialog.openFor(filterKind);
                }
                onCropRequested: cropDialog.open()
                onTrimRequested: trimDialog.open()
            }

            Label {
                anchors.centerIn: parent
                visible: !AppDoc.hasDocument
                text: AppI18n.tr("ui.canvas.no_document")
                // A11Y-4 review D1: the ground here is bgCanvas — the
                // canvas paints it UNCONDITIONALLY before its no-document
                // early-return — and textSecondary reads 4.38:1 there,
                // under AA for this running text on the LAUNCH surface.
                // textPrimary: 7.91:1 (usage-pinned).
                color: Theme.textPrimary
            }
        }

        FocusScope {
            id: panelsPane
            Layout.preferredWidth: 280
            Layout.fillHeight: true
            // Esc from a panel returns focus to the canvas (§31.2 rule 5).
            Keys.onEscapePressed: canvas.forceActiveFocus(Qt.OtherFocusReason)

            Rectangle {
            anchors.fill: parent
            color: Theme.bgRaised

            ColumnLayout {
                anchors.fill: parent
                anchors.margins: 6
                spacing: 6

                LayersPanel {
                                    id: layersPanel
                    objectName: "layersPanel"
                    Layout.fillWidth: true
                    // 0.55 RESTORED. B-12 first took this to 0.35 to make
                    // room for the mask panel, and tst_chord_table's rename
                    // case went red — it double-clicks a layer ROW's centre,
                    // and a shorter panel moves that row. The mask panel
                    // sizes to its content instead, so no existing pane
                    // gives up a share for controls that are usually absent.
                    Layout.preferredHeight: parent.height * 0.55
                }
                // B-12: row 6's mask properties, between the layer it
                // belongs to and the history it writes into. It sizes to
                // its CONTENT rather than a share of the pane: with no
                // mask on the active layer it collapses to its heading,
                // so the panes above and below do not lose room to five
                // controls that are not there.
                MaskPanel {
                    Layout.fillWidth: true
                    Layout.preferredHeight: implicitHeight
                }
                HistoryPanel {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                }
            }
            }
        }
    }

    // ---- P1-2: THE REFUSAL TOAST ----------------------------------------
    // Bound to AppDoc.commandRefused (the EVENT), never to statusText. setStatus
    // returns early when the text is unchanged, so a surface bound to the
    // property shows one toast for two identical refusals and the second press
    // reads as success. The event fires every time.
    //
    // TRUNCATION: reports/ROW1-SHELL-BRIEF.md:291 states as a UI REQUIREMENT
    // that this string must never be shortened -- it is LABEL first, MECHANISM
    // after the em-dash, so a cut prefix displays a sentence that is FALSE for
    // some rows. That brief names "toast" explicitly as a shortening surface.
    // So: the MECHANISM is rendered FIRST and on its own, wrapped, with NO
    // elide, and the taxonomy id sits under it as a separate line. No prefix of
    // the composite string is ever displayed.
    Connections {
        target: AppDoc
        function onCommandRefused(code, message) {
            refusalToast.code = code;
            refusalToast.message = message;
            refusalToast.visible = true;
            refusalToastTimer.restart();   // re-fires on a repeated refusal
        }
        // RECONCILIATION (operator's weekend order, item 4). The toast bound
        // ONE of the two refusal channels, so a REFUSED command surfaced here
        // and an UNAVAILABLE chord did not — press Cmd+T with no document and
        // the status line spoke while the toast stayed dark. Same user
        // experience, two different amounts of feedback.
        //
        // THE CODE SLOT STAYS EMPTY FOR THIS ARM, and that is the whole care
        // in it: `code` renders a WIRE VALUE the user may quote to support,
        // and a reasonId is a cataloged MessageId to be TRANSLATED. Putting
        // `ui.error.no_document` in the monospace line would be a raw id on
        // screen — the P2-D2 key-echo class, which is the exact mistake
        // reportUnavailable was built to avoid on the status line.
        function onActionUnavailable(action, reasonId) {
            refusalToast.code = "";
            refusalToast.message = AppI18n.tr(reasonId);
            refusalToast.visible = true;
            refusalToastTimer.restart();
        }
    }
    Timer {
        id: refusalToastTimer
        interval: 6000
        onTriggered: refusalToast.visible = false
    }
    Rectangle {
        id: refusalToast
        property string code: ""
        property string message: ""
        objectName: "refusalToast"
        visible: false
        z: 1000
        color: Theme.bgRaised
        // Theme.warnText, not an invented name behind an undefined-guard: I first
        // wrote `Theme.danger !== undefined ? Theme.danger : Theme.fgMuted` and
        // NEITHER property exists, so the guard would have silently taken a
        // fallback that is also undefined -- a control that cannot fire, in the
        // slice whose subject is controls that cannot fire. warnText is real and
        // carries a documented 7.3:1 contrast against bgSurface (Theme.qml:35).
        border.color: Theme.warnText
        border.width: 1
        radius: 4
        width: Math.min(parent ? parent.width - 48 : 480, 560)
        height: toastColumn.implicitHeight + 20
        anchors.horizontalCenter: parent ? parent.horizontalCenter : undefined
        anchors.bottom: parent ? parent.bottom : undefined
        anchors.bottomMargin: 24
        Accessible.role: Accessible.AlertMessage
        Accessible.name: refusalToast.message
        ColumnLayout {
            id: toastColumn
            anchors.fill: parent
            anchors.margins: 10
            spacing: 4
            Label {
                // MECHANISM FIRST, WRAPPED, NEVER ELIDED.
                text: refusalToast.message
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }
            Label {
                text: refusalToast.code
                // An empty code must not leave a blank row reserving space
                // under the message — the unavailable arm has no wire code
                // to show.
                visible: refusalToast.code !== ""
                font.family: "monospace"
                color: Theme.textSecondary
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }
        }
        MouseArea { anchors.fill: parent; onClicked: refusalToast.visible = false }
    }

    footer: ToolBar {
        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: 8
            anchors.rightMargin: 8
            spacing: 12

            Label {
                text: AppDoc.statusText
                elide: Text.ElideRight
                Layout.fillWidth: true
            }
            // Honest selection indicator: this Label shows active-or-not.
            // CORRECTED C-638 — the previous text said "the ABI exposes
            // active-or-not only (geometry readback = ABI-growth request 4)"
            // and that is FALSE. Named by O-INT's Q-D23 re-derivation and
            // verified HERE against the sources rather than taken on faith:
            // crates/appname-cabi/src/doc.rs:490 documents `selection.bounds`
            // [x,y,w,h] and :671-672 emits it, and shell/src/
            // document_controller.cpp:2254 already deserializes this very
            // payload — its own comment names selection.bounds. Two files in
            // shell/ disagreed and this was the wrong one. Showing geometry is
            // one JSON key plus placeholders, NOT an ABI-growth request.
            // Q-D23 is OPEN: this chooses none of its options and only removes
            // a false premise a future reader would otherwise inherit.
            Label {
                visible: AppDoc.selectionActive
                text: AppI18n.tr("ui.status.selection_active")
                color: Theme.accent
            }
            Label {
                visible: canvas.pendingPreviewCount > 0
                text: AppI18n.tr("ui.status.pending_previews",
                                 {"count": canvas.pendingPreviewCount})
            }
            Label {
                visible: AppDoc.hasDocument
                text: AppI18n.tr("ui.status.doc_size",
                                 {"width": AppDoc.docWidth, "height": AppDoc.docHeight})
            }
            ToolButton {
                text: AppI18n.tr("ui.action.zoom_out")
                enabled: AppDoc.hasDocument
                onClicked: canvas.zoomOut()
            }
            Label {
                text: AppI18n.tr("ui.value.percent", {"value": canvas.zoomPercent})
                visible: AppDoc.hasDocument
            }
            ToolButton {
                text: AppI18n.tr("ui.action.zoom_in")
                enabled: AppDoc.hasDocument
                onClicked: canvas.zoomIn()
            }
            ToolButton {
                text: AppI18n.tr("ui.action.zoom_fit")
                enabled: AppDoc.hasDocument
                onClicked: canvas.zoomFit()
            }
            ToolButton {
                text: AppI18n.tr("ui.action.zoom_one_to_one")
                enabled: AppDoc.hasDocument
                onClicked: canvas.zoomOneToOne()
            }
        }
    }

    // ---- crop tool -> pre-filled dialog ----
    Connections {
        target: canvas
        function onCropRectDragged(x, y, w, h) {
            // A11Y-3 wave B: crop bounds announce ON SETTLE (drag end —
            // §31.3 "never per mouse-move"); this signal IS the settle.
            AppA11y.announce("crop", "a11y.canvas.crop_bounds",
                             {"width": Math.round(w),
                              "height": Math.round(h)});
            cropDialog.openWithRect(x, y, w, h);
        }
    }

    // ---- dialogs ----
    NewDocumentDialog { id: newDialog; confirmGate: root.confirmDiscard }
    // The guard is SEMANTICS, not a workaround. `AppBuildCommit` is a
    // context property set by main.cpp; the Main.qml HARNESS SUITES boot
    // this file with no host, so the property genuinely does not exist
    // there — and the truthful answer to "which commit built this?" when
    // nobody said is EMPTY, which is exactly the posture appVersion and
    // AboutDialog already take (an empty row does not render).
    //
    // Caught by tst_auto_adjust_menu's g_qmlScriptErrors assertion —
    // "a green run is not a clean tree" — on its first real opportunity,
    // in three suites at once. An unguarded reference threw ReferenceError
    // in every harness that boots Main.qml without main.cpp.
    AboutDialog {
        id: aboutDialog
        buildCommit: (typeof AppBuildCommit === "string") ? AppBuildCommit : ""
        // RC3 (operator WORD 36 (a)): the version main.cpp sets from the build
        // (APPNAME_VERSION); same guard and same honest-empty posture as the
        // commit above when no host sets it.
        appVersion: (typeof AppVersion === "string") ? AppVersion : ""
    }
    DiscardConfirmDialog { id: discardConfirmDialog }
    RecoveryDialog { id: recoveryDialog }
    SelectionModifyDialog { id: modifyDialog }
    ModeConvertDialog { id: modeConvertDialog }
    FilterParamDialog { id: filterParamDialog }
    ResizeDialog { id: resizeDialog }
    CanvasSizeDialog { id: canvasSizeDialog }
    RotateDialog { id: rotateDialog }
    TrimDialog { id: trimDialog }
    CropDialog { id: cropDialog }
    // tier-2 swatch-pair feed: each open seeds the dialog's color param
    // from the foreground swatch state (= canvas.brushColor; ToolPalette
    // header record). In-dialog override still applies per open.
    FillDialog { id: fillDialog; foregroundSeed: canvas.brushColor }
    AssignProfileDialog { id: assignProfileDialog }
    ConvertProfileDialog { id: convertProfileDialog }
    DepthConvertDialog { id: depthConvertDialog }
    FlattenConfirmDialog { id: flattenDialog }
    // 14b: the §32.5 named-object confirm for Delete's no-selection
    // whole-content arm (existing cataloged strings only; see the
    // dialog header for the mint-queue record).
    DeleteConfirmDialog { id: deleteConfirmDialog }

    ColorDialog {
        id: colorDialog
        selectedColor: canvas.brushColor
        onAccepted: canvas.brushColor = selectedColor
    }

    FileDialog {
        id: importFileDialog
        fileMode: FileDialog.OpenFile
        // The filter MIRRORS THE INGRESS'S ACCEPTED SET — it is engine
        // capability shown to the user, not a shell preference. Import
        // runs a TWO-door path (DocumentController::importImage):
        // appname_asset_create, then edit.paste. The asset door sniffs
        // appname.samples.v1 FIRST and otherwise takes the raster arm
        // (PNG at CP-0.5, decode-validated); edit.paste then requires an
        // ingested RASTER asset and refuses a samples asset by name
        // (kinds.rs resolve_raster_asset, §10.3/§9.6 rule 2). The set
        // reachable through THIS dialog is therefore PNG ALONE, so PNG
        // alone is offered: a filter row for anything the ingress
        // refuses would invite the refusal it promises to prevent.
        // WIDEN when the tiered import door lands (operator item 22 /
        // engine P2, JPEG/TIFF/WebP/BMP/GIF) — ui.dialog.file.filter_images
        // is minted and waiting for exactly that day; it is NOT honest
        // today. MINT QUEUE (no id exists, so nothing is invented here):
        // an "All files (*)" escape row for a correctly-formatted blob
        // under an odd extension; a samples-blob row only if a door ever
        // makes samples importable here. Supersedes the old "filter mint
        // on the O-I18N handoff" note — that mint landed (filter_png, all
        // four locales).
        // ⛔⛔ EVERYTHING FROM HERE TO `nameFilters` IS A DATED RECORD OF A WORLD
        // THAT ENDED 2026-08-30, AND IT IS KEPT ONLY AS ONE. Operator ruling D5
        // wired the ingress; the filter row below is already the WIDENED one.
        // Read the paragraphs beneath this marker as history, never as
        // instruction. Marked 2026-09-03, every claim re-measured that day:
        //
        //   "the CP-0.5 ingress still accepts PNG ALONE"  -> FALSE.
        //       appname-io/src/ingress.rs:60
        //       INGRESS_ALLOWED = [png, jpeg, tiff, webp, gif, bmp].
        //   "a REAL JPEG ... indistinguishable from garbage"  -> FALSE, and the
        //       door does not gate on the extension at all: the ingress sniffs
        //       CONTENT (ingress.rs:58 says why -- `evil.png` holding PSD bytes).
        //   "tst_import_filter::everyAdvertisedExtensionIsIngestible ...
        //    WILL GO RED here"  -> IT IS GREEN. Run direct 2026-09-03:
        //       9 passed, 0 failed; that case PASS.
        //   "DO NOT 'fix' it by widening its accepted set"  -> THE WIDENING
        //       ALREADY HAPPENED AND WAS CORRECT (ruling D5). This sentence is
        //       the dangerous one: obeyed today it argues for REVERTING the fix.
        //   "`Codec::decode` ... has ZERO callers in appname-cabi, appname-cli
        //    or shell"  -> THREE callers of the ingress door `decode_for_ingress`:
        //       appname-cabi/src/doc.rs:438, appname-cli/src/main.rs:993, :1166.
        //   "import has no registry wiring at all"  -> wired by D5.
        //
        // ★★★ WHY THIS SAT HERE FOR FOUR DAYS. The 08-30 work (INTEGRATION-log
        // 399) measured every predicate above, retired the gate's EXPECTED-FAIL
        // register, and repaired the test -- and never came back to the comment.
        // The correction reached the copies that are READ and not the copy that
        // RUNS, which is this file. That failure has its own line in the
        // payload's through-line, and this is it happening.
        // ★★ KEPT, NOT DELETED: the 08-25 measurement was real and the ruling
        // that overturned it is legible only if what it overturned is still here.
        // ★★ OPERATOR-DIRECTED WIDENING, 2026-08-25, THIRD REQUEST — AND IT IS A
        // PROMISE AHEAD OF DELIVERY. The paragraph above is NOT stale: the CP-0.5
        // ingress still accepts PNG ALONE (Engine::add_raster_asset ->
        // decode_png_to_buffer; appname-core carries `png = "0.18"` and no other
        // decoder). Measured 2026-08-25 through two doors with controls: the CLI
        // and the exact cabi entry point agree on the same assetId for a PNG, and
        // a REAL JPEG returns the BYTE-IDENTICAL error to 64 random bytes
        // (error.command.bad_target, "Invalid PNG signature"). To this door a JPEG
        // is indistinguishable from garbage.
        //
        // SO A USER WHO PICKS A .jpg HERE NOW GETS A STATUS-LINE REFUSAL RATHER
        // THAN AN ABSENT OPTION. That is the outcome the paragraph above exists to
        // prevent, and it is being accepted deliberately, not overlooked.
        //
        // CONSEQUENCE, STATED SO NOBODY REDISCOVERS IT AS A MYSTERY:
        // tst_import_filter::everyAdvertisedExtensionIsIngestible ASSERTS
        // `advertised == accepted` off the LIVE dialog and WILL GO RED here,
        // naming jpg/tiff/webp/gif/bmp. It is not flaky and it is not wrong — it
        // is the guard doing precisely its job. DO NOT "fix" it by widening its
        // accepted set; that would convert a true red into a false green.
        // The honest close is operator item 22 / engine P2: a CABI-SIDE DECODE
        // ENTRY POINT. `Codec::decode` in appname-io has ZERO callers in
        // appname-cabi, appname-cli or shell today — import has no registry
        // wiring at all, which is why this is a filter change and not a fix.
        nameFilters: [root.fileFilter("images")]
        onAccepted: if (AppDoc.importImage(selectedFile.toString())) canvas.fitIfOverflow() // item 12
    }
    FileDialog {
        id: exportFileDialog
        fileMode: FileDialog.SaveFile
        defaultSuffix: "png"
        // The rows MIRROR WHAT THE ENCODER WRITES. Export is a SAVE
        // dialog, so the honest set is the write set, not the read set
        // importFileDialog above mirrors:
        // DocumentController::exportDocument -> AbiDoc::exportComposite
        // -> appname_doc_export -> export_encoded_bytes resolves the id
        // in CodecRegistry::builtin() (§8.1 order png, jpeg, tiff, webp,
        // gif, bmp, psd, ora, and the feature-gated raw slot) and refuses
        // any codec whose capabilities().write is false with
        // error.codec.write_unsupported. PSD is §8.5 import-only and the
        // RAW slot answers the same way, so neither is offered; the other
        // seven are, in that registry order. Not taken on this comment's
        // word: tst_export_filter case E0 drives the REAL door once per
        // registry id and every row here is checked against what came
        // back.
        //
        // NO "All files" ESCAPE ROW. An unbound FileDialog does not show
        // "no filter" — Qt substitutes its own untranslated English
        // "All Files (*)", which is exactly what this dialog shipped, in
        // every locale, from a string the i18n audit cannot see because
        // Qt and not the shell emits it. An escape row would restore the
        // defect. It would also buy a SAVE dialog little: the rows govern
        // the LISTING, while the format comes from the name the user
        // types, and an unknown suffix still answers typed below. No
        // all-files id is cataloged anyway — mint-queue note, not an
        // invention.
        //
        // THE TYPED SUFFIX STAYS AUTHORITATIVE, not the selected row.
        // Deriving the format from a TRANSLATED filter string is how a
        // locale change becomes a format change; the typed name is the
        // user's own statement of intent. defaultSuffix stays "png": it
        // is the first offered row, it is served, and it is the one
        // format with a byte-exact core reference path (the
        // non-parametric single-raster arm of export_encoded_bytes).
        //
        // Supersedes the old "filter strings ride the next i18n
        // coordination wave" note — those mints had landed (all seven ids
        // in en/es/ja/pseudo). The QUALITY-DIAL half of that note still
        // holds: §8.2 quality is jpeg/webp-only, no UI drives it, and the
        // handler passes -1 for the §8.2 defaults.
        nameFilters: [
            root.fileFilter("png"),
            root.fileFilter("jpeg"),
            root.fileFilter("tiff"),
            root.fileFilter("webp"),
            root.fileFilter("gif"),
            root.fileFilter("bmp"),
            root.fileFilter("ora")
        ]
        // Common-alias mapping to registry ids (review D8). An unknown
        // suffix passes through UNCHANGED so the engine answers typed
        // (error.codec.unrecognized) instead of the shell silently
        // substituting a format. Named rather than inlined so the filter
        // net drives THIS mapping instead of a second copy of it.
        function formatForSuffix(suffix) {
            const aliases = { "jpg": "jpeg", "jpe": "jpeg", "tif": "tiff" };
            const s = (suffix || "").toLowerCase();
            return aliases[s] || s;
        }
        onAccepted: {
            const path = selectedFile.toString();
            // Basename-only suffix parse (a dotted DIRECTORY must not leak
            // into the format).
            const base = path.substring(path.lastIndexOf("/") + 1);
            const dot = base.lastIndexOf(".");
            const suffix = dot >= 0 ? base.substring(dot + 1) : "png";
            AppDoc.exportDocument(path, formatForSuffix(suffix), -1);
        }
    }
    FileDialog {
        id: openFileDialog
        fileMode: FileDialog.OpenFile
        // RC3 D-4 (operator WORD 35, sitting S-04): Open accepts images too.
        // The FIRST filter covers both, because the macOS panel greys out
        // every file the active filter does not match -- an images-second
        // list would leave the sitting's failure in place by default. Its
        // label joins two existing whole-literal catalog strings (no key is
        // composed; a single combined label is queued to O-I18N).
        nameFilters: [
            AppI18n.tr("ui.dialog.file.filter_appname") + ", "
                + AppI18n.tr("ui.dialog.file.filter_images")
                + " (" + root.fileGlobs["appname"] + " " + root.fileGlobs["images"] + ")",
            root.fileFilter("appname"),
            root.fileFilter("images")
        ]
        onAccepted: {
            // W05: sidecar evidence BEFORE opening — a recoverable crash
            // log routes through the prompt; otherwise open directly.
            const url = selectedFile.toString();
            // An IMAGE opens as a new document of its size with the image
            // imported into it (D-4); the package path below is unchanged.
            if (!/\.appname$/i.test(url)) {
                root.confirmDiscard(function() {
                    if (AppDoc.openImageAsDocument(url))
                        canvas.zoomFit();
                });
                return;
            }
            // (4)-3 DOOR 2. The guard sits AHEAD of queryRecovery because
            // both arms below replace the open document — the recovery
            // prompt is a branch inside this door, not a fourth door, so
            // one confirm covers a path that has two exits.
            root.confirmDiscard(function() {
                const q = AppDoc.queryRecovery(url);
                if (q.available)
                    recoveryDialog.openFor(url, q);
                else
                    AppDoc.openDocument(url);
            });
        }
    }
    FileDialog {
        id: saveFileDialog
        fileMode: FileDialog.SaveFile
        defaultSuffix: "appname"
        nameFilters: [root.fileFilter("appname")]
        // RC3 D-3: set by saveThenContinue -- what to do after this Save As
        // succeeds (null for a plain Save As).
        property var afterSave: null
        onAccepted: {
            const ok = AppDoc.saveDocument(selectedFile.toString());
            const next = afterSave;
            afterSave = null;
            if (ok && next)
                next();
        }
        onRejected: afterSave = null
    }
}
