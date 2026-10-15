// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: plan §2.3 "UI: full editing chrome" row; §31.2 pane order
// (palette before canvas); v1.0.1 item 11b — the grouped tool palette
// FRAMEWORK over the item-11a census map (reports/O-UI-log.md). Tier-1
// groups only this slice: Move, Marquee, Lasso, Wand, Crop, Brush,
// Eraser, Pen, Text, Shapes, Transform — transform.puppet promoted from
// menu-only (the Edit-menu entry stays, §45.5). Tier-2/3 groups land at
// 11c; no placeholder stubs (the map's own rule).
// D-43: organization-INSPIRED grouping only — our lucide glyphs and our
// cataloged names throughout. Flyout groups mark the slot corner with
// pure geometry (no glyph, no string surface) and open on click-hold OR
// right-click; a single click selects the slot's current tool; the
// last-used group member is the slot's face. Per-tool state (icon,
// catalog key, checked/enabled bindings, tooltip-with-name) carries
// over from the pre-11b toolbar rows verbatim; glyphless tools stay
// TEXT-LABELED with the same cataloged ids (the lucide vendor round for
// the missing tier-1 glyphs is queued at 11a — no invented ids here).
//
// NAMED DEVIATION (11b review D2 — §31.2 [NORMATIVE] "Every action is
// reachable without a mouse"; §31.5 gate item 2 "new controls
// tab-reachable"): grouped NON-FACE tools are POINTER-ONLY this slice.
// Flyouts open on click-hold (§49.5 600 ms, set app-side) or
// right-click only — no key opens them — so select.ellipse,
// select.polygon, shape.ellipse and transform.perspective have NO
// keyboard path until 11e lands §32.3 group-shortcut cycling (the
// group letter selects the face; pressing it again cycles members).
// Interim keyboard reach, stated honestly: the single-key letters
// (V M L W C T B E) reach eight tools directly; every slot FACE is
// F6/Tab + Space reachable (and faces are last-used-mutable, so a
// pointer face-flip changes which member that path reaches);
// transform.puppet keeps Edit > Puppet Warp. Flyout MenuItems are
// keyboard-operable once open — the gap is OPENING without a pointer.
// Shipped mitigation: flyout slots expose the group's member names
// through Accessible.description (cataloged ids only), so the group
// is non-visually discoverable. Desk-flagged via the lane log.
//
// NARROWED, NOT CLOSED (v1.0.1 item 11e, and CORRECTED in the 11e fix
// round — §5 MAJOR-1). The deviation text above stays as HISTORY; this
// block is its disposition. An earlier revision of this block declared
// the §31.2 gap CLOSED because "every grouped member has a keyboard
// path", and picked PERSPECTIVE as its example. That was FALSE, and
// this slice's own test (tst_palette_pins, the unlettered[] list) said
// so: perspective is one of the members that still has none. The
// correction is logged here rather than overwritten, because a named
// deviation is an audit surface and retiring one on a false premise
// converts a tracked [NORMATIVE] gap into "closed" for every future
// reader.
//  - CLOSED, for the members that now sit in a ruled cycle ring:
//    §32.3 cycling shipped, so a letter activates its ring's remembered
//    member and the same letter again (or Shift+letter) steps the ring
//    forward with wrap. That reaches, without a pointer: all six
//    selection members (M rect/ellipse, L lasso/polygon, W wand/quick),
//    the five shape kinds (U), the three toning members (O), crop and
//    both slice tools (C), heal (J), clone (S), and the single-member
//    slots move/brush/eraser/bucket/pen/text (V B E G P T).
//  - STILL OPEN for SIX members, verified first-hand at this ground by
//    walking every tool-write site in QML: the ONLY keyboard doors that
//    exist are the letter dispatcher, Ctrl+T (transform.free), Edit >
//    Puppet Warp (transform.puppet), and Space on a FOCUSED SLOT —
//    which activates that slot's current FACE and nothing else.
//    THE FACE DOOR IS REAL, AND COUNTING IT COSTS ONE MEMBER (11e
//    record fix; an earlier revision of this block said SEVEN and
//    counted blur among them while conceding "cold start: blur" in the
//    same breath — two sentences that contradicted each other, and the
//    reviewer was right to call it). faceIndex defaults to 0 (below),
//    so every slot's cold-start face is its FIRST member, and syncFace
//    only ever moves the face to a member the canvas tool ALREADY IS.
//    A keyboard-only session therefore cannot move any face it does not
//    already own — which cuts both ways: blur_sharpen.blur, the blur
//    slot's group[0], IS selectable by F6/Tab + Space and always will
//    be in a keyboard-only session, while sharpen and smudge never are.
//    The six with no keyboard-only way to be SELECTED:
//      * sharpen, smudge — their slot has no letter, and neither is its
//        group[0], so the only member of that slot the keyboard can
//        reach is blur. Which member is the face IS pointer-determined,
//        but that is a mixed-session nuance, not unreachability: it can
//        only ever ADD a member to the keyboard's reach (by making it
//        the face) at the cost of blur.
//      * perspective, warp, selection.transform — the transform slot
//        has keyboard doors to two of its members (Ctrl+T for
//        transform.free, which is also its group[0], and the Edit menu
//        item for puppet) and none to these three.
//      * content_aware_fill — the retouch slot's group[0] is heal, and
//        its letters (J heal, S clone) move the face to those two only,
//        never to CAF.
//    Reasons, all routed as SPEC-SIDE fold-in candidates and none a
//    shell election: OQ-3, a letter for blur/sharpen/smudge (Appendix C
//    is silent and ruling (b) forbids inventing one); the transform
//    slot's letterlessness (Appendix C has no transform letter at all);
//    and OQ-1, whether content-aware fill joins the J ring (see the J
//    row's deviation-by-silence record in tool_key_dispatcher.cpp).
//    ES:10345 [NORMATIVE] — "tool select" must have a keyboard path —
//    is therefore still owed for those six, and this record keeps
//    owing it out loud.
//    THE SIX ARE MACHINE-DERIVED, not hand-counted: tst_palette_pins
//    (i11eKeyboardUnreachableSetIsSix) computes them from the ring
//    table, the group[0] faces and the named menu door, and compares
//    against this list — because this count has now been got wrong by
//    hand three times, in three different ways.
//    The interim sentence above ("the single-key letters (V M L W C T B
//    E) reach eight tools directly") is HISTORICAL: the ruled table is
//    now 15 chords — 14 ring letters plus the X swap action.
//  - RULED OUT, permanently: NO KEY MAY OPEN A FLYOUT. §32.3 specifies
//    pointer gestures only for opening (long-press or right-click); the
//    popup is AT-INVISIBLE (QML's Accessible attached type exposes no
//    popup role or state), so a key-open would surface something AT
//    cannot describe — an a11y regression dressed as a feature. Flyout
//    items stay keyboard-operable once pointer-opened.
//    NOTE HOW THIS BINDS THE OPEN SET ABOVE: the flyout is those six
//    members' only remaining door, and it is ruled shut to the keyboard
//    for good. So their gap cannot be closed shell-side by widening the
//    keymap into the popup — it closes only when the SPEC gives those
//    members letters (or the palette gives them their own controls).
//    That is why the deviation is narrowed and left open here instead
//    of being traded for a key-open.
//    This is a PIN, not a gap: tst_palette_pins drives every ruled
//    letter and Shift+letter through the real dispatcher in both focus
//    states and asserts no flyout ever opens, so a future keymap path
//    that opens the popup REDS.
// The 11e DESIGN-INPUT CHANGE note further down (the letter-local vs
// whole-group question item 17 raised) is answered by the same ruling:
// LETTER-LOCAL. M cycles rect/ellipse, L lasso/polygon, W wand/quick —
// three separate rings inside this ONE merged six-member slot. The slot
// FACE is slot-level display and follows whichever member is active;
// ring memory is letter-level state. They coexist by construction.
//
// RE-TIER RECORD (11b review D3): 11a tier-1 slots 17+18 (Hand + Zoom
// — view-mode slots over the existing kind-less pan/zoom surfaces)
// are NOT here and move to 11c: they need a persistent canvas
// view-tool MODE the shell does not have yet, and that wiring collides
// with the main lane's in-flight canvas work. The 11a build-order text
// did not enumerate them under 11c — this line closes that tracking
// hole; 11c picks them up ahead of the swatch pair.
//
// v1.0.1 item 11c — TIER-2 rows over this framework, in the census
// build order (the 11a map, reports/O-UI-log.md). ID-COVERAGE GATE
// (house rule: zero invented ids — a row whose catalog id is not in
// i18n/en.json is EXCLUDED this slice and mint-queued, never
// key-echoed, never an English literal):
// LANDED: WARP joins the transform group below (ui.tool.warp,
// cataloged x4 — the census's "fully plumbed dangling surface" gets
// its palette entry point). GROUPED-BUT-ARMED, census-sanctioned: the
// kind SERVES (contract + envelope_builder warp builders + the tour's
// live warp step), and selecting it announces with the cataloged name
// (Main.qml toolNameKeys row), but CanvasView has no warp SESSION yet
// — canvas input stays inert while warp is armed (the 11a drift list's
// "transform.warp (dangling plumbing)" row; the session UX is its own
// later work).
// HISTORICAL RECORD — TRUE AT 11c BUILD TIME, since RESOLVED by the
// tier-2 rows slice below (corrections are logged, never overwritten):
// at 11c every OTHER tier-2 row was EXCLUDED id-uncovered (every
// en.json check first-hand) and mint-queued, final ids O-I18N's call:
// quick-select; toning dodge|burn|sponge (dodge_burn.stroke
// saturate/desaturate — sponge is NOT a separate kind);
// blur/sharpen/smudge (the cataloged filter.* names being FILTER rows,
// the false-name class); retouch heal|clone|CAF-fill; paint-bucket
// (gradient stays tier 3 per the map); shapes line|polygon|rounded-
// rect (ui.tool.polygon is the LASSO member's id: reusing it is the
// recorded false-name class, see i18n/MINTED.md's ui.options.
// brush_color ruling); selection.transform (history.selection.
// transform is a history LABEL); slice; the fg/bg swatch pair
// (a11y.color.foreground being a REGISTERED announcement TEMPLATE,
// not a control name — the O-INT false-name ruling in i18n/MINTED.md).
// THE MINT HAS SINCE LANDED (lane/i18n-tier2, merged @ d03dd7e): 21
// ui.tool.* ids named by O-I18N along the queued candidates — the 19
// this slice's rows consume, plus hand + zoom (§5 tier-2 review nit-1
// correction: an earlier revision of this line said 19; the mint
// commit itself says "21 ui.tool.* tier-2 names").
//
// v1.0.1 TIER-2 PALETTE ROWS (this slice — the post-mint follow-up to
// 11c, the census build order resumed). Every row below keys a
// NOW-CATALOGED id, each verified first-hand in i18n/en.json; zero
// invented ids. ALL 17 new members ship GROUPED-BUT-ARMED exactly like
// warp (first-hand: canvas_view.cpp's input if-chain has no branch for
// any of them — selecting arms the tool and announces the cataloged
// name via Main.qml toolNameKeys; canvas input stays inert; live
// canvas wiring is later scheduled work; canvas_view.{h,cpp} untouched
// again). Tool wire ids are mechanical derivations (RULE 8), recorded
// per row below:
//  - Wand group += quick-select (select.quick — selection.set mode
//    "quick", seed REQUIRED per contract; the select.wand precedent).
//  - Retouch (NEW slot, census 6): heal.stroke (healing|spot modes —
//    an 11d options toggle), clone.stroke (required source+offset —
//    the source-set gesture is part of the live wiring, not this
//    slice), content_aware_fill.
//  - Fill (NEW slot, census 9): paint-bucket only — fill.yaml:112
//    "the paint-bucket tool emits this kind with a seed"; gradient
//    stays tier 3 (the drag-geometry contract gap, filed).
//  - Toning (NEW slot, census 10): dodge | burn | sponge — ONE kind,
//    dodge_burn.stroke, mode enum [dodge, burn, saturate, desaturate];
//    sponge = the saturate/desaturate pair (that toggle is 11d options
//    work), NOT a separate kind (census + contract agree first-hand).
//  - Blur/Sharpen/Smudge (NEW slot, census 11): blur_sharpen.stroke
//    mode blur|sharpen + smudge.stroke.
//  - Shapes group += line | polygon | rounded-rect (vector.shape_add
//    primitive kinds line/polygon/rounded_rect — "shape."+primitive,
//    the shape.rect precedent; ui.tool.polygon stays the LASSO
//    member's id, the minted shape_polygon disambiguates exactly as
//    shape_ellipse does select_ellipse).
//  - Transform group += selection.transform (after warp — census
//    row-15 order; engine-side class R, requires an active selection).
//  - Slice (NEW slot, census 16): slice (tool id slice.add — the tool
//    draws slices, each commit emits slice.add) + slice-select (tool
//    id slice.select, shell vocabulary like "move"/"crop"; operates
//    slice.edit/slice.delete when live).
// REMAINING EXCLUSIONS (kept current):
//  - color-range: ui.menu.select.color_range EXISTS post-mint but is a
//    SELECT-MENU id ("Color Range…") — a menu surface, not a palette
//    row; also absent from the recorded 11c build order. Out of the
//    palette on both counts (recorded).
//  - HAND/ZOOM: STILL DEFERRED — and the record MOVES: ui.tool.hand/
//    .zoom are NOW MINTED, so the deferral rests SOLELY on the
//    canvas-train condition: the persistent view-tool MODE needs
//    canvas_view work that does not exist yet. (§5 tier-2 review
//    minor-1 correction: an earlier revision of this clause also
//    called item 13's train "not merged at this ground" — stale on
//    the rebased tree: 64e9abf, the item-13 merge, IS an ancestor of
//    this base 8141dd0. The gate is the canvas view-tool MODE work
//    itself, nothing in-flight.) They land with that work.
//  - SWATCH SWAP control: NO cataloged name (grep -i swap|exchange
//    over en.json: zero hits, first-hand) — the same id-coverage gate
//    that ruled 11c. The PAIR lands below; the swap button is
//    mint-queued (naming O-I18N's call). SINCE RESOLVED (menu-mint
//    follow-up, palette half — corrections logged, never overwritten):
//    ui.tool.swatch_swap is MINTED (lane/i18n-menu batch, sealed
//    3dc5c37; verified first-hand in en.json this slice) — the swap
//    control now ships at the footer below.
// fg/bg SWATCH PAIR (census slot 19, LANDED at the palette foot):
// SHELL state feeding the params the shell already controls —
// FOREGROUND IS canvas.brushColor, the existing state strokeBegin
// (canvas_view.cpp strokeBegin call) and vector.shape_add fill comps
// already consume; the well displays/edits that state, no duplicate
// copy. BACKGROUND is new shell state here (consumers pending: the
// swap is mint-queued above; fill-with-background rides item 14b's
// chords via the ENGINE's constant arms). MENU-MINT FOLLOW-UP
// correction (this slice): "the swap is mint-queued" no longer holds
// — the id landed (3dc5c37) and the SWAP CONTROL below is the bg
// state's first shell consumer (fg <-> bg, SHELL state only; both
// wells re-render through their existing bindings). The pair also seeds the
// Fill dialog's color param each open (FillDialog.foregroundSeed —
// the "fill color" half of the census sentence). R15 NAMED
// DISTINCTION (kept accurate): the ENGINE's fill content
// foreground|background arms are headless CONSTANTS this pair does
// NOT feed; unifying those needs an engine surface (recorded, not
// filed). Color-change ANNOUNCEMENTS are not wired this slice:
// a11y.color.foreground is a "{name}, {hex}" template and the shell
// has no color-NAME source — recorded, never faked with a non-name
// argument.
// §5 TIER-2 FIX ROUND (review MAJOR-1 — corrections logged, never
// overwritten): as first landed, "at the palette foot" did not HOLD at
// the 1280x800 default window — the 16 slot rows exhaust the palette
// band, the fillHeight spacer collapses, and the appended swatch row
// landed below the fold with no scroll surface: zero visible pixels
// (the review's three-proof demonstration), while staying
// keyboard-focusable (the offscreen-focus class). Fixed by the
// classic split: the SLOTS region is a clipped scrollable viewport
// (overflow-only scrollbar + keyboard-focus follow) and the pair is
// the palette's persistent FOOTER outside the scroll surface — it can
// never scroll away. Geometry derived mechanically in
// tst_palette_pins r4; the overflow boundary is recorded there.
//
// v1.0.1 item 17 (OPERATOR RULING, new feedback round): ALL selection
// tools consolidate into ONE palette flyout slot — the marquee (rect +
// ellipse), lasso (lasso + polygon) and wand (wand + quick_select)
// rows merge into a single six-member group, member order = the census
// order across the former rows 2-4. Slot count 16 -> 14; the member
// set is UNCHANGED (34), so the D5 max-over-members width is
// mechanically unchanged. PROBE RECORD (the item's other half): the
// ruling also asked for removal of "legacy top-bar duplication" — that
// duplication DOES NOT EXIST on this ground (probed first-hand: the
// header ToolBar in Main.qml is the contextual OPTIONS strip only; the
// tool buttons moved here at 11b). The operator's report came from a
// build predating the grouped palette — already-resolved, recorded,
// nothing invented to remove.
// 11e DESIGN-INPUT CHANGE (flagged for the lane log, NOT built here):
// the D2 deviation record below says 11e's §32.3 group-shortcut
// cycling is "the group letter selects the face; pressing it again
// cycles members" — that design assumed ONE letter per group. Post-17,
// the single-key TOOL letters M / L / W (unchanged this slice) all
// land in this ONE merged group: 11e must decide whether a letter
// cycles the whole six-member group or a letter-local subset (M:
// rect/ellipse, L: lasso/polygon, W: wand/quick). No group-level
// display name exists in the catalog (checked first-hand — the
// framework surfaces the face name + the member-list description, so
// none is needed; if a group label is ever wanted it is mint-queue,
// never invented).

import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts

ToolBar {
    id: paletteRoot

    // The CanvasView instance (Main.qml's canvas): the tool-state truth
    // every binding below derives from.
    required property var canvasItem

    // Swatch-pair BACKGROUND state (header record): new shell state,
    // no engine mirror (R15). Foreground needs no property here — it
    // IS canvasItem.brushColor. Defaults: fg #141414 (canvas_view.h's
    // boot value), bg white — the conventional pair complement.
    property color swatchBackgroundColor: "#ffffff"

    // The swap ACTION, single-sourced (v1.0.1 item 11e): the footer's
    // swap control and the X chord (Main.qml dispatchColorSwap) both
    // call this — one behaviour, one place, so the chord can never drift
    // from the control it rides. Swaps SHELL state only: fg
    // (canvasItem.brushColor — the state strokeBegin and the Fill seed
    // consume) <-> bg (the palette state above); both wells re-render
    // through their existing bindings. R15 stays accurate: the ENGINE's
    // fill content foreground|background arms are headless CONSTANTS
    // this control does not feed — no envelope, no history, a pure
    // shell-state exchange.
    // VALUE COPY, load-bearing (caught red-handed by MM1 first-hand): a
    // bare `const fg = ...brushColor` is a QML value-type REFERENCE that
    // re-reads the property lazily — after the first assignment below it
    // yields the NEW color and the swap collapses both sides onto bg.
    // Qt.rgba forces the copy before any mutation.
    function swapSwatches() {
        const fgRef = paletteRoot.canvasItem.brushColor;
        const fg = Qt.rgba(fgRef.r, fgRef.g, fgRef.b, fgRef.a);
        paletteRoot.canvasItem.brushColor = paletteRoot.swatchBackgroundColor;
        paletteRoot.swatchBackgroundColor = fg;
    }

    padding: Theme.spaceXs

    // §31.2 [NORMATIVE]: "Esc from a panel returns focus to the canvas."
    // Ruling (11b review D1): every non-canvas pane carries the contract
    // — the spec's own runnable artifact applies it to the options bar
    // (keyboard-scripts KS-3 step 5), so "panel" means pane, not just
    // the docked panels. Same handler shape as panelsPane and the menu
    // bar in Main.qml; an unhandled Esc from a focused slot propagates
    // up this focus scope and lands here. (An OPEN flyout consumes its
    // own Esc first — popup semantics, same as open menus.)
    Keys.onEscapePressed: canvasItem.forceActiveFocus(Qt.OtherFocusReason)

    // Tier-1 group rows in 11a map order. Row shape = the pre-11b
    // toolbar model row: key = catalog id, tool = wire id, icon =
    // vendored lucide basename (absent icon = the text-label pattern).
    // transform.puppet reuses its cataloged menu id — the SAME id the
    // §31.3 announcement map already resolves for it; a dedicated
    // ui.tool.* id is O-I18N's call at the vendor round (NOTED in the
    // lane log's mint register path, never invented here).
    readonly property var groups: [
        [{ key: "ui.tool.move", tool: "move", icon: "move" }],
        // item 17 (operator ruling, new feedback round): ALL selection
        // tools consolidate into ONE flyout slot — the former marquee /
        // lasso / wand rows (census rows 2-4) merge, member order = the
        // census order across them. Per-member state carries over
        // verbatim; the framework's last-used face, flyout, and
        // Accessible.description member surfacing apply unchanged.
        // quick-select stays the tier-2 GROUPED-BUT-ARMED member.
        [{ key: "ui.tool.select_rect", tool: "select.rect", icon: "square-dashed" },
         { key: "ui.tool.select_ellipse", tool: "select.ellipse", icon: "circle" },
         { key: "ui.tool.lasso", tool: "select.lasso", icon: "lasso" },
         { key: "ui.tool.polygon", tool: "select.polygon", icon: "pentagon" },
         { key: "ui.tool.wand", tool: "select.wand", icon: "wand" }],
        [{ key: "ui.tool.crop", tool: "crop", icon: "crop" }],
        // tier-2: Retouch (census row 6) — all glyphless TEXT faces
        // (the lucide vendor round is O-BUILD's); GROUPED-BUT-ARMED.
        [{ key: "ui.tool.heal", tool: "heal.stroke" },
         { key: "ui.tool.clone", tool: "clone.stroke" }],
        [{ key: "tool.brush", tool: "brush.stroke", icon: "brush" }],
        [{ key: "ui.tool.eraser", tool: "eraser.stroke", icon: "eraser" }],
        // tier-2: Toning (census row 10) — one kind, three tools; the
        // sponge member arms the saturate/desaturate mode pair (11d
        // options toggle picks which).
        [{ key: "ui.tool.dodge", tool: "dodge_burn.dodge" },
         { key: "ui.tool.burn", tool: "dodge_burn.burn" },
         { key: "ui.tool.sponge", tool: "dodge_burn.sponge" }],
        // tier-2: census row 11.
        [{ key: "ui.tool.blur", tool: "blur_sharpen.blur" },
         { key: "ui.tool.sharpen", tool: "blur_sharpen.sharpen" },
         { key: "ui.tool.smudge", tool: "smudge.stroke" }],
        [{ key: "ui.tool.pen", tool: "vector.pen" }],
        [{ key: "ui.tool.shape_rect", tool: "shape.rect" },
         { key: "ui.tool.shape_ellipse", tool: "shape.ellipse" }],
        // 1.0 final (corrigendum (l)): the puppet-warp row is removed
        // (no solve backend in 1.0); the transform slot is free + perspective.
        [{ key: "ui.tool.transform", tool: "transform.free", icon: "scaling" },
         { key: "ui.tool.perspective", tool: "transform.perspective" }]
    ]

    // 11b review D5: FIXED palette width — chrome geometry must never be
    // a function of interaction history (faces are sticky, so a breathing
    // width RATCHETS; windowed screenshots would stop being
    // session-reproducible). The width is the max face width over ALL
    // group members, measured declaratively from the model by the hidden
    // row below (never rendered, never focusable, and Accessible.ignored
    // — delta re-review DR-1: without the explicit ignore the measure
    // buttons ARE QAccessible nodes, merely invisible-state-suppressed;
    // the earlier "skipped by the a11y tree" wording was false at tree
    // level): locale-correct today and self-adapting when 11c adds
    // glyphless text faces. Slots stretch to it via Layout.fillWidth.
    readonly property var allTools: {
        const out = [];
        for (let g = 0; g < groups.length; ++g)
            for (let t = 0; t < groups[g].length; ++t)
                out.push(groups[g][t]);
        return out;
    }
    readonly property real slotFaceWidth: {
        let m = 0;
        for (let i = 0; i < faceMeasureRepeater.count; ++i) {
            const b = faceMeasureRepeater.itemAt(i);
            if (b)
                m = Math.max(m, b.implicitWidth);
        }
        return m;
    }
    implicitWidth: slotFaceWidth + leftPadding + rightPadding

    Item {
        id: faceMeasure
        visible: false
        Repeater {
            id: faceMeasureRepeater
            model: paletteRoot.allTools
            ToolButton {
                required property var modelData
                display: modelData.icon !== undefined
                         ? AbstractButton.IconOnly
                         : AbstractButton.TextBesideIcon
                icon.source: modelData.icon !== undefined
                             ? "image://lucide/" + modelData.icon : ""
                icon.width: 18
                icon.height: 18
                text: modelData.icon !== undefined ? "" : AppI18n.tr(modelData.key)
                Accessible.name: AppI18n.tr(modelData.key)
                Accessible.ignored: true // DR-1: measure-only, no AT node
            }
        }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 2

        // §5 TIER-2 FIX ROUND (review MAJOR-1): the slots region is a
        // CLIPPED scrollable viewport. At the 1280x800 default (the
        // window Main.qml boots and the tour pins) the 16 slot rows
        // exceed the band left above the swatch footer, so overflow
        // ENGAGES there: wheel/drag scrolling, an overflow-only
        // scrollbar, and keyboard-focus follow (a focused slot scrolls
        // itself into view — keyboard focus never rests on a
        // viewport-clipped control). When the window is tall enough the
        // viewport outgrows its content and behaves exactly like the
        // old spacer layout. The swatch footer lives BELOW this
        // viewport, outside the scroll surface (persistent — the
        // classic pattern).
        Flickable {
            id: slotsViewport
            objectName: "slotsViewport"
            Layout.fillWidth: true
            Layout.fillHeight: true
            contentWidth: width
            contentHeight: slotsColumn.implicitHeight
            clip: true
            // Drag-scroll only when there is something to scroll: an
            // always-interactive Flickable would contend with the slot
            // press-and-hold gesture for no reason.
            interactive: contentHeight > height
            boundsBehavior: Flickable.StopAtBounds
            // Keyboard-focus follow (bounded to the scrollable range).
            function ensureVisible(item) {
                const maxY = Math.max(0, contentHeight - height);
                const y = item.mapToItem(slotsColumn, 0, 0).y;
                if (y < contentY)
                    contentY = Math.max(0, Math.min(y, maxY));
                else if (y + item.height > contentY + height)
                    contentY = Math.max(0, Math.min(y + item.height - height,
                                                    maxY));
            }
            // Overflow affordance: shown exactly while the slots
            // overflow the band. MENU-MINT FOLLOW-UP correction (this
            // slice; the earlier record stays as history): the bar now
            // carries the MINTED a11y.palette.scrollbar name. Lint
            // interaction, kept honest: tst_a11y_tree_lint's controlRe
            // does not list ScrollBar (verified first-hand again this
            // slice), so the lint was green before and stays green
            // after — the name lands because the closed roster demands
            // labeled interactive controls, not because the lint
            // forces it; tst_palette_pins (mm2) is the pin that
            // actually guards it. (Original record: "no string surface
            // — ScrollBar exposes no uncataloged text; it is not in
            // the a11y source lint's interactive-control list".)
            ScrollBar.vertical: ScrollBar {
                Accessible.name: AppI18n.tr("a11y.palette.scrollbar")
                policy: slotsViewport.contentHeight > slotsViewport.height
                        ? ScrollBar.AlwaysOn : ScrollBar.AlwaysOff
            }

            ColumnLayout {
                id: slotsColumn
                width: slotsViewport.width
                spacing: 2

                Repeater {
                    model: paletteRoot.groups

                    ToolButton {
                        id: slot
                        required property var modelData
                        readonly property var group: modelData
                        readonly property bool hasFlyout: group.length > 1
                        // Last-used member = the slot's face. syncFace
                        // follows the canvas tool through EVERY selection
                        // path (slot click, flyout item, single-key
                        // shortcut, Edit > Puppet Warp) and is
                        // membership-gated: leaving the group keeps the
                        // face on the last-used member.
                        property int faceIndex: 0
                        readonly property var faceTool: group[faceIndex]

                        function syncFace() {
                            const t = paletteRoot.canvasItem.tool;
                            for (let i = 0; i < group.length; ++i) {
                                if (group[i].tool === t) {
                                    faceIndex = i;
                                    break;
                                }
                            }
                        }
                        function openFlyout() {
                            if (!hasFlyout)
                                return;
                            flyout.x = slot.width;
                            flyout.y = 0;
                            flyout.open();
                        }
                        Component.onCompleted: {
                            syncFace();
                            // pressAndHold connected ONLY for flyout
                            // groups: a connected hold handler suppresses
                            // the release click (QQuickAbstractButton
                            // wasHeld semantics), so single-tool slots
                            // keep the plain-button gesture contract of
                            // the pre-11b toolbar unchanged.
                            if (hasFlyout)
                                slot.pressAndHold.connect(slot.openFlyout);
                        }
                        Connections {
                            target: paletteRoot.canvasItem
                            function onToolChanged() { slot.syncFace(); }
                        }
                        // §5 fix round (MAJOR-1): keyboard-focus follow —
                        // focus must never rest on a viewport-clipped
                        // slot (the review's offscreen-focus a11y point).
                        onActiveFocusChanged: {
                            if (activeFocus)
                                slotsViewport.ensureVisible(slot);
                        }

                        Layout.fillWidth: true

                        // Pre-11b toolbar row state, verbatim (P2 icon
                        // rows; the A11Y-1 text-label pattern for
                        // glyphless tools).
                        display: faceTool.icon !== undefined
                                 ? AbstractButton.IconOnly
                                 : AbstractButton.TextBesideIcon
                        icon.source: faceTool.icon !== undefined
                                     ? "image://lucide/" + faceTool.icon : ""
                        icon.width: 18
                        icon.height: 18
                        icon.color: enabled ? (checked ? Theme.accent
                                                       : Theme.textPrimary)
                                            : Theme.textDisabled
                        text: faceTool.icon !== undefined
                              ? "" : AppI18n.tr(faceTool.key)
                        checkable: true
                        checked: paletteRoot.canvasItem.tool === faceTool.tool
                        enabled: AppDoc.hasDocument
                                 || paletteRoot.canvasItem.tool === faceTool.tool
                        onClicked: paletteRoot.canvasItem.tool = faceTool.tool
                        Accessible.name: AppI18n.tr(faceTool.key)
                        // 11b review D2 mitigation: QML's Accessible
                        // attached type exposes no popup role/state, so
                        // the group's existence gets a non-visual surface
                        // through the description — the member names,
                        // CATALOGED IDS ONLY. The ", " list join is
                        // uncataloged punctuation: the recorded
                        // milestone-arg debt class (on O-I18N's books);
                        // a group-list template id rides the vendor-round
                        // mint queue. Single-tool slots describe nothing.
                        Accessible.description: {
                            if (!hasFlyout)
                                return "";
                            const names = [];
                            for (let i = 0; i < group.length; ++i)
                                names.push(AppI18n.tr(group[i].key));
                            return names.join(", ");
                        }
                        ToolTip.visible: hovered
                        ToolTip.delay: 600
                        ToolTip.text: AppI18n.tr(faceTool.key)

                        // Right-click = the second flyout-open gesture
                        // (spec'd alongside click-hold). Left button
                        // passes through untouched; hover stays the
                        // button's (tooltip).
                        MouseArea {
                            anchors.fill: parent
                            acceptedButtons: Qt.RightButton
                            enabled: slot.hasFlyout
                            onClicked: slot.openFlyout()
                        }

                        // Flyout corner marker: pure geometry — no
                        // uncataloged glyph or string surface. Indicator
                        // bar (≥3:1 UI): textSecondary on the button
                        // grounds per Theme.qml's recorded ratios;
                        // textDisabled when the slot is.
                        Canvas {
                            id: flyoutMarker
                            visible: slot.hasFlyout
                            width: 6
                            height: 6
                            anchors.right: parent.right
                            anchors.bottom: parent.bottom
                            anchors.margins: 3
                            property color tint: slot.enabled
                                                 ? Theme.textSecondary
                                                 : Theme.textDisabled
                            onTintChanged: requestPaint()
                            onPaint: {
                                const ctx = getContext("2d");
                                ctx.clearRect(0, 0, width, height);
                                ctx.beginPath();
                                ctx.moveTo(width, 0);
                                ctx.lineTo(width, height);
                                ctx.lineTo(0, height);
                                ctx.closePath();
                                ctx.fillStyle = tint;
                                ctx.fill();
                            }
                        }

                        // The group flyout: one item per member tool, the
                        // SAME cataloged name as text (§31.5 item 1
                        // surfaces), the same per-tool enabled/checked
                        // truth as the slots. §31.3 record (corrected,
                        // 11b review D6): flyout items are
                        // overlay-hosted — outside the guard's
                        // containment walk — so changes committed here
                        // ANNOUNCE, like menu-driven changes. So do
                        // pointer CLICKS on slots (click does not move
                        // focus into the pane — probed on cocoa and
                        // offscreen); the guard suppresses
                        // keyboard-focused activation only. Full record
                        // at the guard in Main.qml. (Overlay hosting also
                        // means the viewport's clip cannot truncate an
                        // open flyout.)
                        Menu {
                            id: flyout
                            Repeater {
                                model: slot.group
                                MenuItem {
                                    required property var modelData
                                    text: AppI18n.tr(modelData.key)
                                    checkable: true
                                    checked: paletteRoot.canvasItem.tool
                                             === modelData.tool
                                    enabled: AppDoc.hasDocument
                                             || paletteRoot.canvasItem.tool
                                                === modelData.tool
                                    onTriggered: paletteRoot.canvasItem.tool
                                                 = modelData.tool
                                }
                            }
                        }
                    }
                }
            }
        }

        // fg/bg SWATCH PAIR (census slot 19; header record). Wells are
        // real focusable buttons named by the MINTED ids (the FillDialog
        // swatch conversion's reviewed shape: ToolButton + contentItem
        // Rectangle with the borderInput boundary, A11Y-4 D6 class).
        // No swap button: its name is not cataloged (header record —
        // mint-queued). SINCE RESOLVED (menu-mint follow-up, this
        // slice — the line above stays as history): ui.tool.swatch_swap
        // is MINTED (3dc5c37) and the swap control lands adjacent to
        // the pair below (the classic corner-arrow convention; the
        // X-key affordance is 11e's §32.3 wave, not built here).
        // Fixed 26 px wells sit far below the slot-face
        // max, so the D5 fixed-width contract is untouched.
        // §5 fix round (MAJOR-1): this row is the palette's PERSISTENT
        // FOOTER — a sibling of the slots viewport, outside its scroll
        // surface, so it is visible at every scroll position and every
        // window height (the classic pattern; r3 pins viewport + pixel
        // truth, r4 pins persistence at both scroll extremes).
        RowLayout {
            spacing: 4
            ToolButton {
                objectName: "swatchForegroundWell"
                implicitWidth: 26
                implicitHeight: 26
                Accessible.name: AppI18n.tr("ui.tool.swatch_foreground")
                ToolTip.visible: hovered
                ToolTip.delay: 600
                ToolTip.text: AppI18n.tr("ui.tool.swatch_foreground")
                onClicked: swatchForegroundDialog.open()
                contentItem: Rectangle {
                    radius: 3
                    color: paletteRoot.canvasItem.brushColor
                    border.color: Theme.borderInput
                }
            }
            ToolButton {
                objectName: "swatchBackgroundWell"
                implicitWidth: 26
                implicitHeight: 26
                Accessible.name: AppI18n.tr("ui.tool.swatch_background")
                ToolTip.visible: hovered
                ToolTip.delay: 600
                ToolTip.text: AppI18n.tr("ui.tool.swatch_background")
                onClicked: swatchBackgroundDialog.open()
                contentItem: Rectangle {
                    radius: 3
                    color: paletteRoot.swatchBackgroundColor
                    border.color: Theme.borderInput
                }
            }
            // SWATCH SWAP control (menu-mint follow-up, palette half):
            // the classic corner arrow adjacent to the pair — the
            // CLICKABLE swap affordance (the X-key half is 11e's).
            // Swaps the SHELL state only: fg (canvas.brushColor — the
            // state strokeBegin and the Fill seed consume) <-> bg (the
            // palette state above); both wells re-render through their
            // existing bindings. R15 stays accurate: the ENGINE's fill
            // content foreground|background arms are headless CONSTANTS
            // this control does not feed — no envelope, no history, a
            // pure shell-state exchange. Glyph is PURE GEOMETRY (the
            // flyoutMarker precedent): no swap lucide is vendored and
            // the vendor round is O-BUILD's — no invented icon, no
            // string surface beyond the MINTED name.
            ToolButton {
                id: swatchSwapControl
                objectName: "swatchSwapControl"
                implicitWidth: 18
                implicitHeight: 18
                padding: 2
                Layout.alignment: Qt.AlignVCenter
                Accessible.name: AppI18n.tr("ui.tool.swatch_swap")
                ToolTip.visible: hovered
                ToolTip.delay: 600
                ToolTip.text: AppI18n.tr("ui.tool.swatch_swap")
                // v1.0.1 item 11e: the body moved to paletteRoot's
                // swapSwatches (header) so this control and the X chord
                // are ONE behaviour. The value-copy record travels with
                // it, unchanged.
                onClicked: paletteRoot.swapSwatches()
                contentItem: Canvas {
                    property color tint: swatchSwapControl.enabled
                                         ? Theme.textSecondary
                                         : Theme.textDisabled
                    onTintChanged: requestPaint()
                    onPaint: {
                        const ctx = getContext("2d");
                        ctx.clearRect(0, 0, width, height);
                        const s = Math.min(width, height);
                        const cx = 0.24 * s;
                        const cy = 0.24 * s;
                        const r = 0.46 * s;
                        // Quarter arc bowing through the outer corner —
                        // the classic bidirectional swap arrow.
                        ctx.strokeStyle = tint;
                        ctx.fillStyle = tint;
                        ctx.lineWidth = Math.max(1.2, 0.09 * s);
                        ctx.beginPath();
                        ctx.arc(cx, cy, r, Math.PI / 2, 0, true);
                        ctx.stroke();
                        // Arrowhead pointing down (toward the wells).
                        ctx.beginPath();
                        ctx.moveTo(cx, 0.96 * s);
                        ctx.lineTo(cx - 0.17 * s, 0.62 * s);
                        ctx.lineTo(cx + 0.17 * s, 0.62 * s);
                        ctx.closePath();
                        ctx.fill();
                        // Arrowhead pointing right.
                        ctx.beginPath();
                        ctx.moveTo(0.96 * s, cy);
                        ctx.lineTo(0.62 * s, cy - 0.17 * s);
                        ctx.lineTo(0.62 * s, cy + 0.17 * s);
                        ctx.closePath();
                        ctx.fill();
                    }
                }
            }
            Item { Layout.fillWidth: true }
        }
    }

    // Edit doors for the pair (the Main.qml brush-color well pattern).
    ColorDialog {
        id: swatchForegroundDialog
        objectName: "swatchForegroundDialog"
        selectedColor: paletteRoot.canvasItem.brushColor
        onAccepted: paletteRoot.canvasItem.brushColor = selectedColor
    }
    ColorDialog {
        id: swatchBackgroundDialog
        objectName: "swatchBackgroundDialog"
        selectedColor: paletteRoot.swatchBackgroundColor
        onAccepted: paletteRoot.swatchBackgroundColor = selectedColor
    }
}
