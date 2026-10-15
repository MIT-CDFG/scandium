// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// v1.0.1 item 11b §5 DELTA RE-REVIEW pins (reviewer-added): the fix
// round (8695f8a) landed behavioral changes with zero test cases — the
// house standing rule gives mouse/keyboard interaction behavior real
// synthesized-event regression cases. This suite boots the REAL
// Main.qml (the test target compiles the same AppnameShell QML module
// from the same checked-in sources) offscreen on the Basic style with
// the real controller/announcer wired exactly like shell/src/main.cpp,
// then drives synthesized key/mouse events:
//   D1: Esc from a focused palette slot returns focus to the canvas;
//       Esc with an OPEN flyout closes the flyout only (popup
//       semantics — focus does not jump).
//   D2: grouped slots expose the member names (cataloged ids) through
//       Accessible.description; no key path opens a flyout; slot FACES
//       stay Space-reachable; the eight single-key tool letters reach
//       their tools.
//   D4: mousePressAndHoldInterval honored by the palette flyout hold
//       (runtime), plus a source pin that the app's own main.cpp sets
//       600 BEFORE the QML load (the test binary cannot execute the
//       app's main(); parse-the-source-of-truth is the house
//       tst_filter_table pattern).
//   D5: palette implicitWidth == max face over ALL group members
//       (re-derived mechanically from the hidden measure row) and is
//       NOT a function of interaction history — flipping the face
//       through every member of every group never moves it; the
//       measure row is invisible and absent from the tab chain and
//       the accessibility tree.
//   D6: pointer clicks on slots do NOT move focus into the pane and
//       their tool changes ANNOUNCE through the live region; a
//       keyboard-focused Space activation is SUPPRESSED; flyout-driven
//       changes announce (overlay-hosted).
//   D7 (record): the Edit > Puppet Warp menu door still enters the
//       session (wiring probe via the item's triggered signal).
// Plus the 11b framework regression net: pane cycle order, slots
// select tools, right-click opens flyouts, last-used faces update.
//
// v1.0.1 item 11c extensions (builder-added, the same offscreen
// harness): the transform group gains WARP — the slice's one
// id-covered tier-2 row (ToolPalette.qml header record). Pins:
//   C1: flyout member select commits the tool change (canvas tool
//       truth), the slot face flips to the glyphless TEXT face, the
//       member list rides Accessible.description, and the change
//       ANNOUNCES with the cataloged name (the Main.qml toolNameKeys
//       row — without it announceToolChange skips warp silently).
//   C2: ARMED-NOT-LIVE record (census-sanctioned): with warp armed the
//       canvas is INERT — a press/drag/release grows no history
//       (engine truth, not echo) and starts no gesture; and the D5
//       width contract re-derives over the 17 members with the warp
//       TEXT face measured in.
//
// TIER-2 PALETTE ROWS extensions (the post-mint follow-up slice;
// ToolPalette.qml header record — 17 new members across 5 new slots +
// 4 grown groups, ALL GROUPED-BUT-ARMED, plus the fg/bg swatch pair):
//   R1: EVERY new member — flyout select (slot click for the single-
//       member paint-bucket slot) commits the tool truth, the face
//       flips to the glyphless TEXT face, and the change ANNOUNCES the
//       MINTED cataloged name (each member's Main.qml toolNameKeys row
//       is the discriminator — a dropped row silent-skips exactly like
//       pre-11c warp).
//   R2: ALL 17 new members are ARMED-NOT-LIVE — canvas press/drag/
//       release grows no history (engine truth) and the tool stays
//       armed; ONE sensitivity guard (C2's pattern) proves the probe
//       path can see a submit.
//   R3: the swatch pair — wells exist, named by the MINTED ids,
//       DISPLAY the state (fg = canvas.brushColor, bg = palette
//       state); the fg state FEEDS the params: the Fill dialog's color
//       param seeds from it on open and the applied fill's PIXELS are
//       the seeded color (composite readback — engine truth), and a
//       brush stroke with a changed fg paints that color (readback
//       scan). The counts (16 slots / 34 members) and the D2/D5 sweeps
//       below re-derive over the grown model; the swatch wells are
//       visible ToolButtons WITHOUT a group property and are excluded
//       from the slot list mechanically on that truth.
//
// §5 TIER-2 FIX ROUND upgrades (review findings, this fix round):
//   R2 UPGRADE (review minor-2): history length proves engine OUTCOME,
//       not shell wiring — the review live-wired dodge into the brush
//       branch and r2 stayed GREEN (the engine rejects the unknown
//       kind; history never grows; the tool stays armed). r2 now also
//       asserts SUBMISSION-ATTEMPT truth per member —
//       DocumentController::submissionAttempts(), bumped at ENTRY of
//       the two submission doors (applyCommand + strokeBegin), before
//       the engine answers — plus zero strokeCommitted emissions and
//       an unchanged status line across the armed sweep (the review's
//       recommended side-effect asserts). The reviewer's exact
//       demonstration REDs this pin now (re-run first-hand this fix
//       round: live-wire -> red -> targeted restore -> green).
//   R3 UPGRADE (review MAJOR-1 acceptance): the wells' on-screen
//       EXISTENCE is now pinned on viewport truth (scene rects fully
//       inside the window AND the palette bounds), grabWindow PIXEL
//       truth (the wells' actual colors at their own coordinates —
//       the review's window-grab proof, made a standing pin), and
//       REAL synthesized pointer clicks driving each well's wired
//       behavior (probed first-hand: onClicked opens that well's
//       ColorDialog edit door — asserted via the dialogs' visible
//       flip, then closed).
//   R4 (new, review MAJOR-1 acceptance — the overflow mechanism): the
//       slots region is a clipped scrollable viewport with the swatch
//       pair as a persistent footer OUTSIDE it. Pins: content height
//       re-derived mechanically (slot heights + spacing, RULE 8);
//       overflow ENGAGES at the default 1280x800 (the size Main.qml
//       boots and the tour pins); the engage boundary is derived and
//       verified by a resize probe (fits above it, overflows below);
//       a full keyboard Tab walk reaches ALL 16 slots with the
//       focus-follow keeping every focused slot fully inside the
//       viewport (keyboard focus never rests on a clipped control —
//       the review's a11y point); a REAL wheel event scrolls the
//       viewport; and the footer wells hold their exact scene position
//       at BOTH scroll extremes (they can never scroll away).
//
// v1.0.1 item 17 (OPERATOR RULING, new feedback round — a ruling-driven
// pin update, cause recorded here, never silent): ALL selection tools
// consolidate into ONE flyout slot — the marquee/lasso/wand rows merge
// into a single six-member group in census order (rect, ellipse, lasso,
// polygon, wand, quick). Slot count 16 -> 14; the MEMBER set is
// unchanged (34), so the D5 max-over-members width is mechanically
// unchanged (the mechanical derivations below re-derive either way).
// The ruling's other half ("remove the legacy top-bar duplication") is
// ALREADY-RESOLVED on this ground: probed first-hand, the header
// ToolBar is the contextual options strip only — the operator's report
// came from a pre-11b build. Updated pins: the ground slot count, the
// D2 description rows (three selection rows collapse to one six-name
// row), r4's slot arithmetic + the overflow record: RE-DERIVED at the
// new 14-slot geometry, the slots now FIT the 1280x800 band (the R4
// block above records the pre-17 truth; post-17 the boundary is pinned
// both ways and the scroll/focus-follow/footer mechanism is probed at
// a derived overflow-ENGAGED window height — shipped code, still
// exercised, never assumed away). New pin:
//   I17: exactly one slot carries the selection members; its group and
//        its open flyout hold the six members in census order; a
//        flyout select of a member from a FORMER OTHER SLOT (wand)
//        commits + face-flips + announces; the tool letters M/L/W are
//        UNCHANGED and each flips the merged slot's face (last-used
//        face across the whole merged set, keyboard path); the full
//        six-member face sweep; the D5 width identity re-asserted with
//        the max face name + width printed (the item-17 derivation
//        record). The armed member's truths stay covered member-keyed:
//        R1 (select + announce) and R2 (attempt-truth inert) find the
//        merged slot through slotForMember — face-agnostic.
//
// MENU-MINT FOLLOW-UP, palette half (this slice): the swatch-SWAP
// control + the viewport ScrollBar's accessible name — both ids MINTED
// in the lane/i18n-menu batch (sealed 3dc5c37), each verified
// first-hand in en.json (and guarded against key-echo in the ground
// pin):
//   MM1: the swap control sits adjacent to the pair (viewport + pixel
//        truth — the r3 idiom), is named by the MINTED
//        ui.tool.swatch_swap, and a REAL synthesized click swaps the
//        SHELL state BOTH directions (fg = canvas.brushColor <-> bg =
//        the palette state) with BOTH consumers re-rendering (the
//        wells' contentItem color bindings AND grabWindow pixel truth
//        at the wells' own coordinates); double-swap = identity (the
//        same three truths again); and the swap is ENGINE-SILENT —
//        zero submission attempts, zero history growth (the R15
//        record: the engine's fill content foreground|background
//        constant arms are headless constants the pair does not feed).
//   MM2: the slots viewport's overflow ScrollBar carries the MINTED
//        a11y.palette.scrollbar name — QML-context truth plus an
//        a11y-TREE assert (interface name + ScrollBar role) at an
//        overflow-ENGAGED height (the bar is policy-hidden at the
//        default 800 px post-17). LINT INTERACTION, recorded honestly:
//        tst_a11y_tree_lint's controlRe does not list ScrollBar
//        (verified first-hand this slice), so the source lint is green
//        with or without the name — THIS pin is what guards the label;
//        the name is the honest fix the closed §31.5 roster demands,
//        not a lint appeasement.

#include "announcer.h"
#include "click_focus_parity.h"
#include "canvas_view.h"
#include "document_controller.h"
#include "i18n_catalog.h"
#include "units.h"
#include "icon_provider.h"
#include "layer_thumb_provider.h"
#include "tool_key_dispatcher.h"

#include <QAccessible>
#include <QClipboard>
#include <QElapsedTimer>
#include <QFile>
#include <QGuiApplication>
#include <QImage>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQmlExpression>
#include <QQuickItem>
#include <QQuickStyle>
#include <QQuickWindow>
#include <QSet>
#include <QSignalSpy>
#include <QStyleHints>
#include <QWheelEvent>
#include <QtMath>
#include <QtTest>

using namespace appname::shell;

namespace {

// Captured in main() BEFORE mirroring the app's setter — records what
// the platform default was, so the report can say whether main.cpp:82
// changes anything on this platform (RULE 9: claims match what ran).
int g_platformDefaultHoldMs = -1;

bool isToolButton(const QQuickItem *it)
{
    return it && it->inherits("QQuickToolButton");
}

// The VISUAL item tree (QQuickItem::childItems) — QObject-tree
// findChildren does not reach QML-created visual items reliably.
void collectItems(QQuickItem *root, QList<QQuickItem *> *out)
{
    if (!root)
        return;
    const auto kids = root->childItems();
    for (QQuickItem *k : kids) {
        out->append(k);
        collectItems(k, out);
    }
}

} // namespace

class TstPalettePins : public QObject {
    Q_OBJECT
private slots:
    void initTestCase();
    void catalogAndSceneGround();
    void d1EscFromFocusedSlotFocusesCanvas();
    void d1EscWithOpenFlyoutClosesFlyoutOnly();
    void d2DescriptionsListCatalogedMemberNames();
    // v1.0.1 item 11e: the ruled letter table runs BEFORE any case that
    // drives letters, because its expectations are the rings'
    // COLD-START members (:18516 "each group remembers its last-used
    // member for the session" — a session that has cycled is no longer
    // cold). Declaration order IS execution order; the no-key-open pin
    // below now drives every ruled letter, so it moved after this one.
    void d2SingleKeyLettersReachTheRuledTable();
    void i11eRingTableMatchesThePaletteModel();
    void i11eBareRepeatAndShiftCycleIdentically();
    void i11eRingsAreLetterLocalAndRememberTheirMember();
    void i11eSharedCRingCrossesTwoSlots();
    void i11eShapeAndToningRingOrders();
    void i11eKeyboardUnreachableSetIsSix();
    void d2NoKeyPathOpensFlyoutFacesStayReachable();
    void d4HoldIntervalHonoredAndSetBeforeQmlLoad();
    void d5WidthIsMaxOverAllMembersAndNeverRatchets();
    void d5MeasureRowInvisibleUnfocusableNotInA11yTree();
    void d6ClickAnnouncesKeyboardActivationSuppressed();
    void d6FlyoutDrivenChangeAnnouncesAndUpdatesFace();
    void regressionPaneCycleOrderPreserved();
    void d7PuppetWarpMenuDoorStillWorks();
    void c1WarpFlyoutSelectsAnnouncesAndFaceFlips();
    void c2WarpArmedNotLiveCanvasInertWidthHolds();
    void r1Tier2MembersSelectAnnounceFaceFlip();
    void r2Tier2MembersArmedNotLiveCanvasInert();
    void r3SwatchPairStateFeedsBrushAndFillParams();
    void r4SlotsOverflowFocusFollowFooterPersists();
    void i17MergedSelectionSlotCensusOrderFacesLetters();
    void mm1SwatchSwapClickSwapsStateBothConsumers();
    void mm2ViewportScrollBarCarriesMintedName();
    void s25bWarpPressAnnouncesSessionPending();
    void s25bPendingSetIsDerivedFromPressChainNotAList();
    void s25bWandRefusalAnnouncesEngineArmPending();

private:
    CatalogQml *m_i18n = nullptr;
    DocumentController *m_controller = nullptr;
    Announcer *m_announcer = nullptr;
    QQmlApplicationEngine *m_engine = nullptr;
    QQuickWindow *m_window = nullptr;
    QQuickItem *m_canvas = nullptr;
    QQuickItem *m_palette = nullptr;
    QQuickItem *m_viewport = nullptr; // the slots' clipped scroll surface
    ToolKeyDispatcher *m_toolKeys = nullptr; // item 11e: the real keymap
    QList<QQuickItem *> m_slots;   // visible palette slots, top-to-bottom
    QList<QQuickItem *> m_measure; // hidden measure-row buttons

    QString catTr(const char *key) const
    {
        return m_i18n->tr(QLatin1String(key));
    }
    QString canvasTool() const
    {
        return m_canvas->property("tool").toString();
    }
    void setCanvasTool(const char *tool)
    {
        m_canvas->setProperty("tool", QLatin1String(tool));
    }
    bool insidePalette(QQuickItem *item) const
    {
        for (QQuickItem *n = item; n; n = n->parentItem())
            if (n == m_palette)
                return true;
        return false;
    }
    QPoint center(QQuickItem *item) const
    {
        return item->mapToScene(QPointF(item->width() / 2.0,
                                        item->height() / 2.0))
            .toPoint();
    }
    // §5 fix round (MAJOR-1): the slots live in a clipped scroll
    // viewport — before a pointer interaction targets a slot's scene
    // coordinates, scroll it into view through the SAME function the
    // keyboard focus-follow uses (a user scrolls first too; a click at
    // a clipped slot's mapped coordinates would land on whatever is
    // actually painted there).
    void revealSlot(QQuickItem *slot)
    {
        QMetaObject::invokeMethod(
            m_viewport, "ensureVisible",
            Q_ARG(QVariant, QVariant::fromValue<QObject *>(slot)));
        QCoreApplication::processEvents();
    }
    // Fully-inside-the-viewport truth (scene coordinates, half-pixel
    // tolerance): the geometry claim behind "visible and clickable".
    bool slotFullyInViewport(QQuickItem *slot) const
    {
        const QPointF topLeft = slot->mapToScene(QPointF(0, 0));
        const QPointF vpTopLeft = m_viewport->mapToScene(QPointF(0, 0));
        return topLeft.y() >= vpTopLeft.y() - 0.5
               && topLeft.y() + slot->height()
                      <= vpTopLeft.y() + m_viewport->height() + 0.5;
    }
    // The slot whose GROUP contains this wire id (face-agnostic).
    QQuickItem *slotForMember(const QString &wire) const
    {
        for (QQuickItem *s : m_slots) {
            const QVariantList g = s->property("group").toList();
            for (const QVariant &m : g)
                if (m.toMap().value(QStringLiteral("tool")).toString()
                    == wire)
                    return s;
        }
        return nullptr;
    }
    // Keyboard route into the palette pane: the cycle is
    // [palette, canvas, panels], so from the canvas the palette is ONE
    // step BACKWARD (Shift+F6).
    void keyboardIntoPalette()
    {
        focusCanvas();
        QTest::keyClick(m_window, Qt::Key_F6, Qt::ShiftModifier);
        QQuickItem *fi = m_window->activeFocusItem();
        QVERIFY2(fi && insidePalette(fi),
                 "Shift+F6 from canvas did not focus into the palette");
    }
    // The slot whose CURRENT face carries this wire id.
    QQuickItem *slotFacing(const QString &wire) const
    {
        for (QQuickItem *s : m_slots)
            if (s->property("faceTool").toMap().value(QStringLiteral("tool"))
                    .toString() == wire)
                return s;
        return nullptr;
    }
    QObject *flyoutOf(QQuickItem *slot) const
    {
        const auto all = slot->findChildren<QObject *>();
        for (QObject *o : all)
            if (o->inherits("QQuickMenu"))
                return o;
        return nullptr;
    }
    bool anyFlyoutOpen() const
    {
        for (QQuickItem *s : m_slots)
            if (QObject *f = flyoutOf(s))
                if (f->property("visible").toBool())
                    return true;
        return false;
    }
    void focusCanvas()
    {
        m_canvas->forceActiveFocus();
        QTRY_VERIFY(m_canvas->hasActiveFocus());
    }
    QString accessibleDescription(QQuickItem *slot)
    {
        QQmlExpression expr(qmlContext(slot), slot,
                            QStringLiteral("Accessible.description"));
        bool undefined = false;
        const QVariant v = expr.evaluate(&undefined);
        if (expr.hasError())
            qWarning() << "Accessible.description read failed:"
                       << expr.error().toString();
        return undefined ? QString() : v.toString();
    }
    QString accessibleName(QQuickItem *item)
    {
        QQmlExpression expr(qmlContext(item), item,
                            QStringLiteral("Accessible.name"));
        bool undefined = false;
        const QVariant v = expr.evaluate(&undefined);
        if (expr.hasError())
            qWarning() << "Accessible.name read failed:"
                       << expr.error().toString();
        return undefined ? QString() : v.toString();
    }
    // The 17 tier-2 members (wire id -> minted catalog id), the
    // ToolPalette model order — the R1/R2 sweeps and the mutation
    // matrix key off this one table.
    struct Tier2Member {
        const char *wire;
        const char *key;
    };
    static const Tier2Member *tier2Members(int *count)
    {
        // 1.0 final (corrigendum (l)): every STILL-dead member was
        // REMOVED from the palette (quick select, content-aware fill,
        // paint bucket, line, polygon, rounded rectangle, transform
        // selection, slice, slice select); their absence is pinned in
        // r2. Clone stays: it is live with a source and inert without.
        static const Tier2Member rows[] = {
            // heal.stroke LEFT THIS TABLE 2026-08-29 (operator ruling
            // Q-D1(a)): it joined the stroke dispatch via the
            // palette-spelling -> (kind, mode) table in canvas_view with
            // mode "spot", so "armed is not live" stopped being true for
            // it. RELOCATED to the positive arm, never deleted -- same
            // treatment smudge and the dodge/blur four got, and for the
            // same reason: the table's claim is "armed does not mean
            // live", and the members STILL dead need their witness.
            //
            // ★ CLONE STAYS, and the pair is now the discriminator: heal
            // and clone were filed together as "both need a source
            // point". That was true of clone and only ASSUMED of heal --
            // heal had a spot mode that forbids a source, clone has no
            // such mode. One row of this table crossed and its neighbour
            // could not, which is exactly the distinction a shared
            // sentence had been hiding.
            {"clone.stroke", "ui.tool.clone"},
            // dodge_burn.dodge/.burn and blur_sharpen.blur/.sharpen LEFT
            // THIS TABLE 2026-08-28 (O-UI): they joined the stroke dispatch
            // via the palette-spelling -> (kind, mode) table in
            // canvas_view, so "armed is not live" stopped being true for
            // them and this pin correctly went red. Relocated to the
            // positive arm below, never deleted — the table's claim was
            // always "armed does not mean live", and the members STILL
            // dead need that witness.
            //
            // ★★ SPONGE LEFT THIS TABLE 2026-08-29 (operator ruling
            // Q-D3(b)). Its row was load-bearing right up to the moment it
            // was not: dodge_burn.stroke's mode enum is
            // [dodge, burn, saturate, desaturate] with NO `sponge`, so the
            // palette spelled a tool the wire cannot name, and it was the
            // one member of its family that had not crossed. The operator
            // ruled desaturate WITH A LABEL, so it crossed. RELOCATED to
            // the positive arm, never deleted -- the fourth member to make
            // this trip, and the table has kept a witness every time.
            // smudge.stroke LEFT THIS TABLE 2026-08-28 (192b34b7, O-UI):
            // it is DISPATCHED now -- canvas_view's if-chain has a branch
            // for it, so it is no longer "armed but not live" and asserting
            // inertness here reds for the right reason. The member is not
            // deleted, it is RELOCATED: the positive arm at the end of
            // r2Tier2MembersArmedNotLiveCanvasInert now pins that smudge
            // MOVES the attempt counter, so this pin still has a witness
            // for the claim "armed does not mean live" AND a witness for
            // the one member where armed now DOES mean live.
        };
        *count = int(sizeof(rows) / sizeof(rows[0]));
        return rows;
    }
    // Drain the announcer's 500 ms rate-limit window so the next
    // announcement in a probe is not coalesced with setup noise.
    void quiesceAnnouncer() { QTest::qWait(650); }
};

void TstPalettePins::initTestCase()
{
    // Mirror shell/src/main.cpp's wiring exactly (context properties,
    // image providers, module load).
    m_i18n = new CatalogQml(Catalog::load(QStringLiteral("en")));
    m_controller = new DocumentController(&m_i18n->catalog());
    m_announcer = new Announcer(&m_i18n->catalog());
    m_controller->setAnnouncer(m_announcer);

    m_engine = new QQmlApplicationEngine;
    m_engine->addImageProvider(QStringLiteral("lucide"),
                               new LucideIconProvider);
    m_engine->addImageProvider(QStringLiteral("layerthumb"),
                               new LayerThumbProvider(m_controller));
    m_engine->rootContext()->setContextProperty(QStringLiteral("AppI18n"),
                                                m_i18n);
    m_engine->rootContext()->setContextProperty(QStringLiteral("AppDoc"),
                                                m_controller);
    m_engine->rootContext()->setContextProperty(QStringLiteral("AppA11y"),
                                                m_announcer);
    // F10/Ctrl+F2 is not driven here; a plain QObject satisfies the
    // context-property lookup.
    // AppUnits: main.cpp:132 registers it and this harness did not, so
    // NewDocumentDialog threw two ReferenceErrors and a TypeError on every
    // boot — this suite was exercising a tree the application never has.
    m_engine->rootContext()->setContextProperty(QStringLiteral("AppUnits"),
                                                new UnitsQml(m_engine));
    m_engine->rootContext()->setContextProperty(QStringLiteral("AppKeys"),
                                                new QObject(m_engine));
    m_engine->loadFromModule("AppnameShell", "Main");
    QVERIFY2(!m_engine->rootObjects().isEmpty(), "Main.qml failed to load");
    m_window = qobject_cast<QQuickWindow *>(m_engine->rootObjects().first());
    QVERIFY(m_window);
    m_announcer->setSubject(m_window);
    QVERIFY(QTest::qWaitForWindowExposed(m_window));
    m_window->requestActivate();
    QTRY_VERIFY(m_window->isActive());

    // A real document so every slot is enabled (the gestures-suite doc
    // shape).
    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));

    m_canvas = m_window->findChild<QQuickItem *>(
        QStringLiteral("canvasView"));
    QVERIFY2(m_canvas, "canvasView not found");

    // v1.0.1 item 11e: mirror main.cpp's dispatcher install — the letter
    // chords live there now, not in Main.qml's (deleted) Shortcuts.
    m_toolKeys = new ToolKeyDispatcher(this);
    m_toolKeys->attach(m_window);

    QList<QQuickItem *> items;
    collectItems(m_window->contentItem(), &items);
    for (QQuickItem *it : items) {
        if (QLatin1String(it->metaObject()->className())
                .startsWith(QLatin1String("ToolPalette"))) {
            m_palette = it;
            break;
        }
    }
    QVERIFY2(m_palette, "ToolPalette instance not found");
    m_viewport = m_palette->findChild<QQuickItem *>(
        QStringLiteral("slotsViewport"));
    QVERIFY2(m_viewport, "slots viewport (Flickable) not found — the "
                         "§5 fix-round overflow surface is missing");

    QList<QQuickItem *> paletteItems;
    collectItems(m_palette, &paletteItems);
    for (QQuickItem *it : paletteItems) {
        if (!isToolButton(it))
            continue;
        // Slots are the visible ToolButtons WITH a group property; the
        // swatch-pair wells (tier-2) are visible ToolButtons WITHOUT
        // one (found by objectName in R3); the measure row stays the
        // invisible set.
        if (it->isVisible() && it->property("group").isValid())
            m_slots.append(it);
        else if (!it->isVisible())
            m_measure.append(it);
    }
    std::sort(m_slots.begin(), m_slots.end(),
              [](QQuickItem *a, QQuickItem *b) {
                  return a->mapToScene(QPointF(0, 0)).y()
                         < b->mapToScene(QPointF(0, 0)).y();
              });
}

void TstPalettePins::catalogAndSceneGround()
{
    // Vacuity guards (RULE 9): a key-echoing catalog would make every
    // description comparison echo-vs-echo; missing slots would make
    // the sweeps skip silently.
    QCOMPARE(catTr("ui.tool.move"), QStringLiteral("Move"));
    // Q-D7 rider 1: the no-op notice must RESOLVE, or the toast shows a
    // raw catalog id to the operator. Emitted-ness is pinned in
    // tst_gesture_net, whose fixture has an empty catalog on purpose;
    // this is the half that suite structurally cannot check.
    QCOMPARE(catTr("ui.notice.heal_no_effect_empty_target"),
             QStringLiteral("Heal had no effect: the target layer is empty."));

    // ★★★★★ EVERY DISPATCHED STROKE KIND MUST HAVE A HISTORY LABEL.
    //
    // FOUND IN A TOUR FRAME, NOT BY A TEST: 24-heal-spot.png shows the
    // status bar reading "Applied: history.heal.stroke" and the History
    // panel row reading "history.heal.stroke" -- the RAW CATALOG ID,
    // because no such key existed. document_controller looks up
    // "history." + m_strokeKind at four sites; the catalog only ever had
    // brush and eraser, the two kinds that were ALWAYS dispatched.
    //
    // ★★★ SO EVERY TOOL THAT CROSSED FROM DEAD TO LIVE THIS WEEKEND
    // SHIPPED WITHOUT ITS NAME: smudge, dodge/burn and blur/sharpen on
    // 08-28, heal on 08-29. Four tools, four surfaces each, and no test
    // could see it because the key-echo is the project's HONEST fallback
    // -- it renders, it just renders the id. A missing label is invisible
    // to everything except a person looking at the screen.
    //
    // ★★ THE KINDS ARE TRANSCRIBED, NOT READ FROM canvas_view's table.
    // Reading them from the code under test would agree by construction.
    // This list is the dispatch set: the three kinds whose palette
    // spelling IS their wire kind, plus the two family kinds the mode
    // table maps onto. clone.stroke is DELIBERATELY ABSENT -- it does not
    // dispatch, so it cannot reach these surfaces, and adding a label for
    // it would assert a coverage this suite has not got.
    static const char *const kDispatchedKinds[] = {
        "brush.stroke", "eraser.stroke", "smudge.stroke",
        "heal.stroke", "dodge_burn.stroke", "blur_sharpen.stroke",
    };
    const int kindCount = int(sizeof(kDispatchedKinds) / sizeof(kDispatchedKinds[0]));
    QCOMPARE(kindCount, 6); // population guard: a shrunken list passes vacuously
    for (const char *k : kDispatchedKinds) {
        const QString id = QStringLiteral("history.") + QLatin1String(k);
        QVERIFY2(m_i18n->tr(id) != id,
                 qPrintable(QStringLiteral("%1 key-echoes — the status bar and the "
                                           "History panel will show the raw id to the "
                                           "operator").arg(id)));
    }
    QCOMPARE(catTr("ui.tool.select_rect"), QStringLiteral("Rect Select"));
    QCOMPARE(catTr("ui.tool.warp"), QStringLiteral("Warp")); // 11c row
    // tier-2 mint vacuity guards (a key-echoing catalog would make the
    // R1 name lookups echo-vs-echo).
    QCOMPARE(catTr("ui.tool.quick_select"), QStringLiteral("Quick Selection"));
    QCOMPARE(catTr("ui.tool.content_aware_fill"),
             QStringLiteral("Content-Aware Fill"));
    QCOMPARE(catTr("ui.tool.swatch_foreground"),
             QStringLiteral("Foreground color"));
    QCOMPARE(catTr("ui.tool.swatch_background"),
             QStringLiteral("Background color"));
    // menu-mint follow-up vacuity guards (the MM1/MM2 name lookups —
    // both ids from the sealed 3dc5c37 batch, en.json first-hand).
    QCOMPARE(catTr("ui.tool.swatch_swap"),
             QStringLiteral("Swap Foreground and Background Colors"));
    QCOMPARE(catTr("a11y.palette.scrollbar"),
             QStringLiteral("Tool palette scroll"));
    // 14 group rows: the tier-2 16 minus 2 — item 17 (operator ruling)
    // merges the three selection rows into ONE six-member slot. The
    // MEMBER count is untouched by the merge: still 34 (11c's 17 + the
    // 17 tier-2 members).
    // 1.0 final (corrigendum (l)): the removal set takes out the paint
    // bucket, Text and Slice slots (14 -> 11) and 12 members (34 -> 22):
    // quick select, content-aware fill, paint bucket, text, line,
    // polygon, rounded rectangle, puppet warp, warp, transform selection,
    // slice, slice select. Heal stays (WORD 53, the operator's overrule).
    QCOMPARE(m_slots.size(), 11);
    QCOMPARE(m_measure.size(), 22);
    // Default tool = brush.stroke (canvas_view.h) — the brush slot is
    // checked at boot.
    QCOMPARE(canvasTool(), QStringLiteral("brush.stroke"));
    qInfo() << "platform default mousePressAndHoldInterval was"
            << g_platformDefaultHoldMs << "ms; app sets 600";
}

void TstPalettePins::d1EscFromFocusedSlotFocusesCanvas()
{
    keyboardIntoPalette();
    QQuickItem *fi = m_window->activeFocusItem();
    QVERIFY2(isToolButton(fi) && fi->isVisible(),
             "F6 focus did not land on a visible slot (an invisible "
             "measure button in the tab chain would land here)");
    QTest::keyClick(m_window, Qt::Key_Escape);
    QTRY_VERIFY2(m_canvas->hasActiveFocus(),
                 "Esc from a focused palette slot did not return focus "
                 "to the canvas (§31.2, fix D1)");
}

void TstPalettePins::d1EscWithOpenFlyoutClosesFlyoutOnly()
{
    // Focus the merged selection slot (grouped; item 17 — second in
    // the tab order, where the marquee slot sat) by keyboard, then
    // open its flyout by pointer.
    keyboardIntoPalette();
    QTest::keyClick(m_window, Qt::Key_Tab); // move -> selection
    QQuickItem *slot = m_window->activeFocusItem();
    QVERIFY(slot && insidePalette(slot) && isToolButton(slot));
    QObject *fly = flyoutOf(slot);
    QVERIFY2(fly, "focused slot has no flyout Menu — expected the "
                  "merged selection slot second in the tab order");
    QTest::mouseClick(m_window, Qt::RightButton, Qt::NoModifier,
                      center(slot));
    QTRY_VERIFY2(fly->property("opened").toBool(),
                 "right-click did not open the flyout (framework "
                 "regression)");
    QTest::keyClick(m_window, Qt::Key_Escape);
    QTRY_VERIFY2(!fly->property("visible").toBool(),
                 "Esc did not close the open flyout");
    QVERIFY2(!m_canvas->hasActiveFocus(),
             "Esc on an OPEN flyout jumped focus to the canvas — popup "
             "semantics require the flyout to consume it (fix D1)");
    QQuickItem *fi = m_window->activeFocusItem();
    QVERIFY2(fi && insidePalette(fi),
             "focus left the palette after the flyout closed");
}

void TstPalettePins::d2DescriptionsListCatalogedMemberNames()
{
    // Grouped slots: description == the members' cataloged names,
    // ", "-joined, in model order. Hardcoded against the checked-in
    // en.json values (not just re-derived through the same tr call).
    struct Row {
        const char *faceWire;
        QString expected;
    };
    const Row rows[] = {
        // item 17: the merged selection slot — ONE row now, all six
        // members in census order (was three rows; ruling-driven pin
        // update, suite header).
        // 1.0 final (corrigendum (l)): Quick Selection removed.
        {"select.rect",
         QStringLiteral("Rect Select, Ellipse Select, Lasso, Polygon, "
                        "Wand")},
        // Content-aware fill removed; heal and clone remain.
        {"heal.stroke", QStringLiteral("Heal, Clone")},
        // ★★ "Sponge (desaturate)" since 2026-08-29 (Q-D3(b)). THIS PIN
        // GOING RED IS THE FEATURE WORKING: the ruling asked for a label
        // "so the interim mapping is never silent", and putting it in the
        // CATALOG rather than in one QML binding is what makes it reach
        // every surface -- face, flyout, tooltip, announcer, and this,
        // the group's ACCESSIBLE DESCRIPTION. A screen-reader user hears
        // the qualifier because the string carries it; had it been
        // appended in a visual binding, this pin would have stayed green
        // and the mapping would have been silent for exactly the people
        // least able to discover it another way.
        {"dodge_burn.dodge", QStringLiteral("Dodge, Burn, Sponge (desaturate)")},
        {"blur_sharpen.blur", QStringLiteral("Blur, Sharpen, Smudge")},
        // 1.0 final (corrigendum (l)): line, polygon, rounded rectangle,
        // puppet warp, warp, transform selection and both slice tools
        // removed.
        {"shape.rect", QStringLiteral("Rectangle, Ellipse Shape")},
        {"transform.free", QStringLiteral("Transform, Perspective")},
    };
    for (const Row &r : rows) {
        QQuickItem *slot = slotFacing(QLatin1String(r.faceWire));
        QVERIFY2(slot, r.faceWire);
        QCOMPARE(accessibleDescription(slot), r.expected);
    }
    // Single-tool slots describe nothing — including the tier-2
    // paint-bucket slot (single member: gradient stays tier 3).
    QQuickItem *move = slotFacing(QStringLiteral("move"));
    QVERIFY(move);
    QCOMPARE(accessibleDescription(move), QString());
    // 1.0 final (corrigendum (l)): the paint-bucket slot is gone.
    QVERIFY2(!slotFacing(QStringLiteral("fill")), "the removed paint-bucket slot is back");
}

void TstPalettePins::i11eKeyboardUnreachableSetIsSix()
{
    // THE NARROWED D2 SET, MACHINE-DERIVED (11e record fix). The
    // ToolPalette supersession block names six members as having no
    // keyboard-only way to be SELECTED. That count has been got wrong
    // by hand three times in three different ways — the reviewer's
    // first six (which included blur and, before CAF moved, was really
    // five), my seven (which counted blur while conceding in a
    // parenthesis that blur is the cold-start face), and the corrected
    // six. A prose list a reader trusts is exactly the thing that
    // should be computed, so this case computes it.
    //
    // The derivation, from the three doors that actually exist:
    //  (1) a member in a ruled cycle ring is reachable by its letter;
    //  (2) a member that is its slot's group[0] is reachable by
    //      F6/Tab + Space — faceIndex defaults to 0 (ToolPalette.qml)
    //      and syncFace only ever moves the face to a member the canvas
    //      tool ALREADY IS, so a keyboard-only session can never move a
    //      face away from group[0];
    //  (3) a member with a dedicated non-letter keyboard door, named
    //      explicitly below with the door.
    // Everything else is unreachable without a pointer.
    QSet<QString> reachable;
    // (1) the rings.
    for (const ToolRing &ring : ToolKeyDispatcher::ringTable())
        for (const QString &wire : ring.members)
            reachable.insert(wire);
    // (2) the cold-start faces, read from the live model.
    const QVariantList groups = m_palette->property("groups").toList();
    QVERIFY(!groups.isEmpty());
    QStringList allMembers;
    for (const QVariant &g : groups) {
        const QVariantList members = g.toList();
        QVERIFY(!members.isEmpty());
        for (const QVariant &m : members)
            allMembers << m.toMap().value(QStringLiteral("tool")).toString();
        reachable.insert(
            members.first().toMap().value(QStringLiteral("tool")).toString());
    }
    // 1.0 final (corrigendum (l)): 12 members removed, 34 -> 22.
    QCOMPARE(allMembers.size(), 22);
    // Vacuity guard for leg (2): the blur slot's group[0] really is
    // blur, i.e. the face door is doing work here rather than being a
    // no-op that happens to agree with the expected list.
    QVERIFY2(reachable.contains(QStringLiteral("blur_sharpen.blur")),
             "blur is not its slot's group[0] — the cold-start face "
             "derivation changed; re-read the record before touching it");
    QVERIFY2(!ToolKeyDispatcher::ringTable().isEmpty()
                 && m_toolKeys->ringMembers(QStringLiteral("J"))
                        .contains(QStringLiteral("heal.stroke")),
             "ring derivation vacuous");
    // (3) the named non-letter keyboard doors. transform.free also has
    // Ctrl+T, but it is its slot's group[0] so leg (2) already covers
    // it; puppet is NOT group[0] and needs its door named.
    struct NamedDoor {
        const char *wire;
        const char *door;
    };
    // 1.0 final (corrigendum (l)): Puppet Warp and its Edit menu door are
    // removed, so no member needs a named non-letter door any more.
    const QList<NamedDoor> doors;
    for (const NamedDoor &d : doors)
        reachable.insert(QLatin1String(d.wire));

    QStringList unreachable;
    for (const QString &wire : allMembers)
        if (!reachable.contains(wire))
            unreachable << wire;
    unreachable.sort();

    // 1.0 final (corrigendum (l)): content-aware fill, transform
    // selection and warp are removed; three remain.
    QStringList expected{QStringLiteral("blur_sharpen.sharpen"),
                         QStringLiteral("smudge.stroke"),
                         QStringLiteral("transform.perspective")};
    expected.sort();
    QVERIFY2(unreachable == expected,
             qPrintable(QStringLiteral("the narrowed D2 open set moved: "
                                       "computed [%1], recorded [%2] — "
                                       "update the ToolPalette record "
                                       "and this pin together")
                            .arg(unreachable.join(QLatin1String(", ")),
                                 expected.join(QLatin1String(", ")))));
    QCOMPARE(unreachable.size(), 3);
}

void TstPalettePins::d2NoKeyPathOpensFlyoutFacesStayReachable()
{
    // Focus the marquee slot; no key may open its (or any) flyout.
    // Space must still activate the FACE (interim reach statement).
    setCanvasTool("brush.stroke");
    keyboardIntoPalette();
    QTest::keyClick(m_window, Qt::Key_Tab);
    QQuickItem *slot = m_window->activeFocusItem();
    QVERIFY(slot && insidePalette(slot));
    QObject *fly = flyoutOf(slot);
    QVERIFY(fly);

    const Qt::Key probes[] = {Qt::Key_Return, Qt::Key_Enter, Qt::Key_Down,
                              Qt::Key_Up, Qt::Key_Left, Qt::Key_Right,
                              Qt::Key_Menu};
    for (Qt::Key k : probes) {
        QTest::keyClick(m_window, k);
        QTest::qWait(20);
        QVERIFY2(!anyFlyoutOpen(),
                 qPrintable(QStringLiteral("key %1 opened a flyout — "
                                           "grouped non-face tools are "
                                           "pointer-only this slice "
                                           "(named deviation D2)")
                                .arg(int(k))));
    }
    // Space = face activation (whatever member currently faces the
    // slot — faces are last-used-mutable), still no flyout.
    QVERIFY(slot->hasActiveFocus());
    const QString face = slot->property("faceTool").toMap()
                             .value(QStringLiteral("tool")).toString();
    QVERIFY(!face.isEmpty());
    QTest::keyClick(m_window, Qt::Key_Space);
    QTRY_COMPARE(canvasTool(), face);
    QVERIFY2(!anyFlyoutOpen(), "Space opened a flyout");

    // ---- v1.0.1 item 11e: THE DESIGN PIN (desk ruling (d)) ----
    // The D2 deviation this case was written for is now a DECISION:
    // there must never be a key-open path to a flyout. §32.3 specifies
    // pointer gestures only for opening, §31.2 parity is TASK-level and
    // cycling now satisfies it, and the popup is AT-INVISIBLE — a
    // key-open would surface something AT cannot describe. So the pin
    // grows teeth: EVERY ruled sequence (bare AND Shift, all fifteen
    // chords) driven through the REAL dispatcher, in BOTH focus states,
    // must leave every flyout shut. Discriminator: a future keymap path
    // that opens the popup.
    const Qt::Key ruled[] = {Qt::Key_V, Qt::Key_M, Qt::Key_L, Qt::Key_W,
                             Qt::Key_C, Qt::Key_B, Qt::Key_E, Qt::Key_J,
                             Qt::Key_S, Qt::Key_G, Qt::Key_O, Qt::Key_P,
                             Qt::Key_T, Qt::Key_U, Qt::Key_X};
    const Qt::KeyboardModifiers mods[] = {Qt::NoModifier, Qt::ShiftModifier};
    // (i) canvas focused — the keymap's home ground.
    focusCanvas();
    for (Qt::Key k : ruled) {
        for (Qt::KeyboardModifiers m : mods) {
            QTest::keyClick(m_window, k, m);
            QTest::qWait(5);
            QVERIFY2(!anyFlyoutOpen(),
                     qPrintable(QStringLiteral("ruled chord %1 (mods %2) "
                                               "opened a flyout from the "
                                               "canvas — ruling (d) says "
                                               "no key ever opens one")
                                    .arg(int(k))
                                    .arg(int(m))));
        }
    }
    // (ii) a FLYOUT-BEARING slot focused — the state where a stray
    // key-open would be most tempting to add.
    keyboardIntoPalette();
    QTest::keyClick(m_window, Qt::Key_Tab); // move -> the merged slot
    QQuickItem *grouped = m_window->activeFocusItem();
    QVERIFY(grouped && insidePalette(grouped));
    QVERIFY2(flyoutOf(grouped), "expected a flyout-bearing slot focused");
    for (Qt::Key k : ruled) {
        for (Qt::KeyboardModifiers m : mods) {
            QTest::keyClick(m_window, k, m);
            QTest::qWait(5);
            QVERIFY2(!anyFlyoutOpen(),
                     qPrintable(QStringLiteral("ruled chord %1 (mods %2) "
                                               "opened a flyout from a "
                                               "focused slot")
                                    .arg(int(k))
                                    .arg(int(m))));
        }
    }
    focusCanvas();
}

void TstPalettePins::d2SingleKeyLettersReachTheRuledTable()
{
    // RENAMED at v1.0.1 item 11e (was d2SingleKeyLettersReachEightTools
    // — the name became FALSE at fifteen chords; renamed rather than
    // left lying, the way the repaint lane renamed a case whose own fix
    // falsified its name).
    // THE LETTER TABLE SWEEP OF RECORD: every ruled letter, driven with
    // a REAL key event through the REAL dispatcher, landing its ring's
    // COLD-START member. Letters are mechanical adoptions of Appendix
    // C's keymap rows (desk ruling (b): invent nothing) — the citation
    // for each row lives beside it in the dispatcher's table.
    const QPair<Qt::Key, const char *> letters[] = {
        {Qt::Key_V, "move"},          {Qt::Key_M, "select.rect"},
        {Qt::Key_L, "select.lasso"},  {Qt::Key_W, "select.wand"},
        // 1.0 final (corrigendum (l)): T (text) and G (paint bucket)
        // lost their only members and are unbound; tst_chord_table pins
        // their absence.
        {Qt::Key_C, "crop"},
        {Qt::Key_B, "brush.stroke"},  {Qt::Key_E, "eraser.stroke"},
        // 11e rows: the letters Appendix C assigns to groups the key
        // layer left unlettered.
        {Qt::Key_J, "heal.stroke"},   {Qt::Key_S, "clone.stroke"},
        {Qt::Key_O, "dodge_burn.dodge"},
        {Qt::Key_P, "vector.pen"},    {Qt::Key_U, "shape.rect"},
    };
    focusCanvas();
    for (const auto &p : letters) {
        QTest::keyClick(m_window, p.first);
        QTRY_COMPARE(canvasTool(), QLatin1String(p.second));
    }
    // Every ruled letter's slot FACE follows the keyboard door too (the
    // face is the slot's last-used member, whatever door set it).
    for (const auto &p : letters) {
        QQuickItem *slot = slotForMember(QLatin1String(p.second));
        QVERIFY2(slot, p.second);
        if (QLatin1String(p.second) == canvasTool())
            QCOMPARE(slot->property("faceTool").toMap()
                         .value(QStringLiteral("tool")).toString(),
                     QLatin1String(p.second));
    }
}

void TstPalettePins::i11eRingTableMatchesThePaletteModel()
{
    // RULE 8 made mechanical: the rings are not a retyped list, they
    // are a READING of the palette group model. Member wire ids must
    // exist in the model and appear in the model's DECLARED order (the
    // census order) — including the C ring, which deliberately crosses
    // two slots. Discriminator: any retyped/reordered member.
    // (The false-name trap this defends: ui.tool.polygon is the LASSO
    // member while ui.tool.shape_polygon is the shape — ids are never
    // the source, the model is.)
    const QVariantList groups = m_palette->property("groups").toList();
    QVERIFY(!groups.isEmpty());
    QHash<QString, QPair<int, int>> modelPos;
    for (int g = 0; g < groups.size(); ++g) {
        const QVariantList members = groups.at(g).toList();
        for (int m = 0; m < members.size(); ++m)
            modelPos.insert(
                members.at(m).toMap().value(QStringLiteral("tool")).toString(),
                {g, m});
    }
    const QVector<ToolRing> &table = ToolKeyDispatcher::ringTable();
    // 1.0 final (corrigendum (l)): G and T unbound, 14 -> 12.
    QCOMPARE(table.size(), 12); // 12 ring letters; X is an action, D deferred
    QSet<QString> ringed;
    for (const ToolRing &ring : table) {
        QVERIFY2(!ring.members.isEmpty(), ring.letter);
        QPair<int, int> prev(-1, -1);
        for (const QString &wire : ring.members) {
            QVERIFY2(modelPos.contains(wire),
                     qPrintable(QStringLiteral("ring %1 member %2 is not "
                                               "in the palette model")
                                    .arg(QLatin1String(ring.letter), wire)));
            const QPair<int, int> at = modelPos.value(wire);
            QVERIFY2(at > prev,
                     qPrintable(QStringLiteral("ring %1 lists %2 out of "
                                               "the model's declared "
                                               "order")
                                    .arg(QLatin1String(ring.letter), wire)));
            prev = at;
            QVERIFY2(!ringed.contains(wire),
                     qPrintable(QStringLiteral("%1 appears in two rings")
                                    .arg(wire)));
            ringed.insert(wire);
        }
    }
    // The DELIBERATE omissions, pinned so they cannot be quietly
    // filled: blur/sharpen/smudge take NO letter (Appendix C is silent;
    // assigning a free key would be a spec-side ADD, a fold-in
    // candidate, never a shell election), and the transform slot has no
    // letter anywhere in Appendix C — its keyboard reach is Ctrl+T.
    const char *unlettered[] = {
        "blur_sharpen.blur", "blur_sharpen.sharpen", "smudge.stroke",
        "transform.free",    "transform.perspective", "transform.puppet",
        "transform.warp",    "selection.transform",
        // 11e fix round (§5 minor-2): content-aware fill left the J ring.
        // Appendix C says nothing at all about it, and the desk ruled
        // that even a NAMED example cannot enumerate a ring (S). Total
        // silence cannot do more than a named example, so CAF is
        // unlettered on the same footing.
        "content_aware_fill",
    };
    for (const char *wire : unlettered)
        QVERIFY2(!ringed.contains(QLatin1String(wire)),
                 qPrintable(QStringLiteral("%1 acquired a letter — that "
                                           "is a spec-side addition, not "
                                           "a shell election")
                                .arg(QLatin1String(wire))));
    // S is a RING OF ONE by desk ruling (Appendix C's "Shift+S the
    // clone group" is an example citation, not a membership
    // enumeration): recorded as a deviation-by-silence, so heal and CAF
    // must NOT be in it.
    QCOMPARE(m_toolKeys->ringMembers(QStringLiteral("S")),
             QStringList{QStringLiteral("clone.stroke")});
    // J is a RING OF ONE: heal (content-aware fill, its old second
    // reading, is removed for 1.0).
    QCOMPARE(m_toolKeys->ringMembers(QStringLiteral("J")),
             QStringList{QStringLiteral("heal.stroke")});
}

void TstPalettePins::i11eBareRepeatAndShiftCycleIdentically()
{
    // T2 — desk ruling (c): §32.3's bare-letter repeat and Appendix C
    // :18516's Shift+letter are the SAME motion, forward, wrapping.
    // Discriminators: a reverse Shift arm; Shift diverging from bare;
    // no wrap.
    // VACUITY FIX (11e fix round, §5 minor-4): this case used to drive
    // ONLY the two-member marquee ring, where forward and reverse are
    // the same sequence — so it advertised a reverse-arm discriminator
    // it could not actually detect (the reviewer's backward-Shift
    // mutation left it green). It now drives a 3- and a 5-member ring
    // as well, where direction is visible. Same aliasing class as the
    // auto-repeat count fixed earlier in this slice.
    struct RingProbe {
        Qt::Key key;
        const char *letter;
        int presses; // > ring size, so the WRAP is inside the sequence
    };
    const RingProbe probes[] = {
        {Qt::Key_M, "M", 4}, // 2 members
        {Qt::Key_O, "O", 4}, // 3 members — reverse is visible here
        {Qt::Key_U, "U", 6}, // 5 members — and here
    };
    for (const RingProbe &p : probes) {
        const QStringList members =
            m_toolKeys->ringMembers(QLatin1String(p.letter));
        QVERIFY2(members.size() >= 2 || QLatin1String(p.letter) == "M",
                 p.letter);
        const auto drive = [&](Qt::KeyboardModifiers mods) {
            setCanvasTool(qPrintable(members.first())); // memory := first
            setCanvasTool("brush.stroke");              // ...leave the ring
            focusCanvas();
            QStringList seen;
            for (int i = 0; i < p.presses; ++i) {
                QTest::keyClick(m_window, p.key, mods);
                QTest::qWait(10);
                seen << canvasTool();
            }
            return seen;
        };
        // Expected motion derived MECHANICALLY from the ring itself
        // (whose membership and order are pinned against the palette
        // model by i11eRingTableMatchesThePaletteModel): activate the
        // first member, then step forward, wrapping.
        QStringList expected;
        for (int i = 0; i < p.presses; ++i)
            expected << members.at(i % members.size());
        const QStringList bare = drive(Qt::NoModifier);
        const QStringList shifted = drive(Qt::ShiftModifier);
        QVERIFY2(bare == expected,
                 qPrintable(QStringLiteral("ring %1 bare motion %2 != "
                                           "forward-with-wrap %3")
                                .arg(QLatin1String(p.letter),
                                     bare.join(QLatin1Char(',')),
                                     expected.join(QLatin1Char(',')))));
        QVERIFY2(shifted == bare,
                 qPrintable(QStringLiteral("ring %1 Shift motion %2 "
                                           "diverges from bare %3")
                                .arg(QLatin1String(p.letter),
                                     shifted.join(QLatin1Char(',')),
                                     bare.join(QLatin1Char(',')))));
    }
}

void TstPalettePins::i11eRingsAreLetterLocalAndRememberTheirMember()
{
    // T3 + T4 — desk ruling (a): rings are LETTER-LOCAL subsets of the
    // merged six-member selection slot, not one six-member group ring.
    // And :18516's "each group remembers its last-used member for the
    // session" is what makes a letter's first press an ACTIVATION.
    // Discriminators: a whole-group six-member ring (L would continue
    // M's walk); ring memory reset on ring exit; the slot FACE used as
    // the ring pointer.
    // Known start, through NON-keyboard doors: each ring's memory is
    // put on its first member (a previous case may legitimately have
    // left it elsewhere — that IS the memory this case then measures).
    setCanvasTool("select.rect");
    setCanvasTool("select.lasso");
    setCanvasTool("select.wand");
    setCanvasTool("brush.stroke");
    focusCanvas();
    QTest::keyClick(m_window, Qt::Key_M);
    QTRY_COMPARE(canvasTool(), QStringLiteral("select.rect"));
    QTest::keyClick(m_window, Qt::Key_M);
    QTRY_COMPARE(canvasTool(), QStringLiteral("select.ellipse"));
    // L starts its OWN ring at ITS first member — it does not continue
    // the marquee walk.
    QTest::keyClick(m_window, Qt::Key_L);
    QTRY_COMPARE(canvasTool(), QStringLiteral("select.lasso"));
    QTest::keyClick(m_window, Qt::Key_L);
    QTRY_COMPARE(canvasTool(), QStringLiteral("select.polygon"));
    QTest::keyClick(m_window, Qt::Key_W);
    QTRY_COMPARE(canvasTool(), QStringLiteral("select.wand"));
    // 1.0 final (corrigendum (l)): Quick Selection is removed, so W is a
    // ring of one and a second press stays on the wand.
    QTest::keyClick(m_window, Qt::Key_W);
    QTest::qWait(30);
    QCOMPARE(canvasTool(), QStringLiteral("select.wand"));
    // Back to M: the marquee ring REMEMBERED ellipse across two other
    // rings' activity (spec row tool_group_cycling_memory, executable
    // here for the first time).
    QTest::keyClick(m_window, Qt::Key_M);
    QTRY_COMPARE(canvasTool(), QStringLiteral("select.ellipse"));
    QCOMPARE(m_toolKeys->ringCurrent(QStringLiteral("M")),
             QStringLiteral("select.ellipse"));
    QCOMPARE(m_toolKeys->ringCurrent(QStringLiteral("L")),
             QStringLiteral("select.polygon"));
    // Leaving the ring entirely and coming back keeps the memory.
    QTest::keyClick(m_window, Qt::Key_V);
    QTRY_COMPARE(canvasTool(), QStringLiteral("move"));
    QTest::keyClick(m_window, Qt::Key_M);
    QTRY_COMPARE(canvasTool(), QStringLiteral("select.ellipse"));
    // The merged slot's FACE is slot-level display and follows the
    // active member; the three rings keep their own pointers. Both
    // truths at once — this is the coexistence the item-17 note asked
    // 11e to settle.
    QQuickItem *sel = slotForMember(QStringLiteral("select.rect"));
    QVERIFY(sel);
    QCOMPARE(sel->property("faceTool").toMap()
                 .value(QStringLiteral("tool")).toString(),
             QStringLiteral("select.ellipse"));
    QCOMPARE(m_toolKeys->ringCurrent(QStringLiteral("W")),
             QStringLiteral("select.wand"));
}

void TstPalettePins::i11eSharedCRingCrossesTwoSlots()
{
    // T5, 1.0 final (corrigendum (l)): the slice tools are removed, so the
    // C ring that crossed the Crop and Slice slots is crop alone, and the
    // Slice slot is gone. (Name kept for the record; the cross-slot claim
    // no longer holds.)
    setCanvasTool("brush.stroke");
    focusCanvas();
    for (int i = 0; i < 2; ++i) {
        QTest::keyClick(m_window, Qt::Key_C);
        QTRY_COMPARE(canvasTool(), QStringLiteral("crop"));
    }
    QCOMPARE(m_toolKeys->ringMembers(QStringLiteral("C")),
             QStringList{QStringLiteral("crop")});
    QVERIFY2(!slotForMember(QStringLiteral("slice.add"))
                 && !slotForMember(QStringLiteral("slice.select")),
             "a removed slice tool is back in the palette");
}

void TstPalettePins::i11eShapeAndToningRingOrders()
{
    // T6 — the five-member U ring and the three-member O ring, in the
    // palette model's order exactly (O is an exact match to :18516's
    // dodge/burn/sponge example). Discriminator: any other order.
    setCanvasTool("brush.stroke");
    focusCanvas();
    // 1.0 final (corrigendum (l)): line, polygon and rounded rectangle
    // are removed; U is the two live shapes. Known start through a
    // NON-keyboard door (the ring remembers its last member, and an
    // earlier case may have left it on ellipse).
    setCanvasTool("shape.rect");
    setCanvasTool("brush.stroke");
    focusCanvas();
    const char *shapes[] = {"shape.rect", "shape.ellipse", "shape.rect"};
    for (const char *wire : shapes) {
        QTest::keyClick(m_window, Qt::Key_U);
        QTRY_COMPARE(canvasTool(), QLatin1String(wire));
    }
    const char *toning[] = {"dodge_burn.dodge", "dodge_burn.burn",
                            "dodge_burn.sponge", "dodge_burn.dodge"};
    for (const char *wire : toning) {
        QTest::keyClick(m_window, Qt::Key_O);
        QTRY_COMPARE(canvasTool(), QLatin1String(wire));
    }
    // Shift walks the same rings the same way (ruling (c)) — checked on
    // a ring longer than two, where a reverse arm would be obvious.
    QTest::keyClick(m_window, Qt::Key_O, Qt::ShiftModifier);
    QTRY_COMPARE(canvasTool(), QStringLiteral("dodge_burn.burn"));
}

void TstPalettePins::d4HoldIntervalHonoredAndSetBeforeQmlLoad()
{
    // Source pin (house parse-the-source-of-truth pattern): the app's
    // main.cpp must call setMousePressAndHoldInterval(600) BEFORE the
    // engine loads any QML — the palette's pressAndHold timer reads
    // this hint at press time from controls created after load.
    QFile f(QStringLiteral(APPNAME_SHELL_MAIN_CPP));
    QVERIFY(f.open(QIODevice::ReadOnly));
    const QString src = QString::fromUtf8(f.readAll());
    const int setPos = src.indexOf(QStringLiteral(
        "styleHints()->setMousePressAndHoldInterval(600)"));
    const int loadPos =
        src.indexOf(QStringLiteral("loadFromModule(\"AppnameShell\""));
    QVERIFY2(setPos >= 0,
             "main.cpp no longer sets mousePressAndHoldInterval(600) "
             "(fix D4 reverted?)");
    QVERIFY2(loadPos >= 0, "main.cpp QML load call not found — update "
                           "this pin's anchor");
    QVERIFY2(setPos < loadPos,
             "the 600 ms hold interval is set AFTER the QML load — "
             "controls may cache platform defaults (fix D4 placement)");

    // Runtime: with the hint at 600 (set in this binary's main() the
    // same way), a press-and-hold on a grouped slot opens the flyout
    // at ~600 ms — not earlier, and before the ~800 ms platform-default
    // ballpark; a plain click never rides the hold path.
    QCOMPARE(QGuiApplication::styleHints()->mousePressAndHoldInterval(),
             600);
    setCanvasTool("brush.stroke");
    focusCanvas();
    QQuickItem *slot = slotForMember(QStringLiteral("transform.free"));
    QVERIFY(slot);
    revealSlot(slot); // §5 fix round: slots live in a clipped viewport
    QObject *fly = flyoutOf(slot);
    QVERIFY(fly);
    const QString toolBefore = canvasTool();

    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      center(slot));
    QElapsedTimer held;
    held.start();
    QTest::qWait(250);
    QVERIFY2(!fly->property("visible").toBool(),
             "flyout opened well before the 600 ms hold interval");
    QTRY_VERIFY_WITH_TIMEOUT(fly->property("opened").toBool(), 2000);
    const qint64 openedAt = held.elapsed();
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        center(slot));
    qInfo() << "press-and-hold flyout opened at" << openedAt << "ms";
    QVERIFY2(openedAt >= 450 && openedAt <= 790,
             qPrintable(QStringLiteral("hold fired at %1 ms — outside "
                                       "the 600 ms window (a platform-"
                                       "default ~800 ms read would land "
                                       "above 790)")
                            .arg(openedAt)));
    // wasHeld semantics: the hold-open release must NOT select.
    QCOMPARE(canvasTool(), toolBefore);
    QTest::keyClick(m_window, Qt::Key_Escape); // close the flyout
    QTRY_VERIFY(!fly->property("visible").toBool());
}

void TstPalettePins::d5WidthIsMaxOverAllMembersAndNeverRatchets()
{
    // Mechanical re-derivation (RULE 8): implicitWidth must equal the
    // max implicitWidth over the hidden measure buttons (34 members
    // post-tier-2 — the ground pin counts them) plus the palette's own
    // side padding.
    qreal maxFace = 0;
    for (QQuickItem *b : m_measure)
        maxFace = qMax(maxFace, b->implicitWidth());
    QVERIFY(maxFace > 0);
    const qreal expected = maxFace
                           + m_palette->property("leftPadding").toReal()
                           + m_palette->property("rightPadding").toReal();
    const qreal w0 = m_palette->implicitWidth();
    QVERIFY2(qAbs(w0 - expected) < 0.5,
             qPrintable(QStringLiteral("implicitWidth %1 != max member "
                                       "face %2 + padding")
                            .arg(w0)
                            .arg(expected)));

    // History-independence: walk the face through EVERY member of
    // every group (the sticky last-used flip) — the width must never
    // move (a breathing width RATCHETS).
    // 1.0 final (corrigendum (l)): the 22 members left after the removal set.
    const char *allWires[] = {
        "move", "select.rect", "select.ellipse", "select.lasso",
        "select.polygon", "select.wand", "crop",
        "heal.stroke", "clone.stroke",
        "brush.stroke", "eraser.stroke", "dodge_burn.dodge",
        "dodge_burn.burn", "dodge_burn.sponge", "blur_sharpen.blur",
        "blur_sharpen.sharpen", "smudge.stroke", "vector.pen",
        "shape.rect", "shape.ellipse", "transform.free", "transform.perspective"};
    for (const char *wire : allWires) {
        setCanvasTool(wire);
        QCoreApplication::processEvents();
        // The face flip is REAL (syncFace follows the tool)...
        QQuickItem *slot = slotFacing(QLatin1String(wire));
        QVERIFY2(slot, wire);
        // ...and the geometry does not follow it.
        QVERIFY2(qAbs(m_palette->implicitWidth() - w0) < 0.001,
                 qPrintable(QStringLiteral("palette width moved to %1 "
                                           "(from %2) after facing %3 — "
                                           "the ratchet is back (fix D5)")
                                .arg(m_palette->implicitWidth())
                                .arg(w0)
                                .arg(QLatin1String(wire))));
    }
    // Sanity that the sweep exercised a WIDE text face: perspective is
    // glyphless — its face renders the cataloged text label.
    setCanvasTool("transform.perspective");
    QCoreApplication::processEvents();
    QQuickItem *tslot = slotFacing(QStringLiteral("transform.perspective"));
    QVERIFY(tslot);
    QCOMPARE(tslot->property("text").toString(), catTr("ui.tool.perspective"));
    setCanvasTool("brush.stroke");
}

void TstPalettePins::d5MeasureRowInvisibleUnfocusableNotInA11yTree()
{
    for (QQuickItem *b : m_measure)
        QVERIFY2(!b->isVisible(), "a measure-row button is visible");

    // Unfocusable: walk the palette's whole tab chain — the measure
    // buttons must never appear.
    keyboardIntoPalette();
    QQuickItem *first = m_window->activeFocusItem();
    QVERIFY(first && insidePalette(first));
    QQuickItem *n = first;
    for (int i = 0; i < 64 && n; ++i) {
        n = n->nextItemInFocusChain(true);
        QVERIFY2(!m_measure.contains(n),
                 "a hidden measure button is in the focus chain");
        if (n == first)
            break;
    }

    // A11y tree: a group member that is NOT any slot's current face
    // has its name ONLY on a measure button (all flyouts closed; the
    // puppet id is excluded — the Edit menu shares that cataloged
    // name), so such a name appearing as an accessible node means the
    // measure row leaked into the tree.
    QVERIFY(!anyFlyoutOpen());
    QSet<QString> facedKeys;
    for (QQuickItem *s : m_slots)
        facedKeys << s->property("faceTool").toMap()
                         .value(QStringLiteral("key")).toString();
    QAccessibleInterface *root =
        QAccessible::queryAccessibleInterface(m_window);
    if (!root) {
        qWarning() << "offscreen platform returned no accessible "
                      "interface for the window — a11y-tree absence "
                      "not verifiable here (recorded in review)";
        return;
    }
    QStringList names;
    QSet<QObject *> nodes;
    std::function<void(QAccessibleInterface *)> walk =
        [&](QAccessibleInterface *i) {
            if (!i)
                return;
            names << i->text(QAccessible::Name);
            if (i->object())
                nodes << i->object();
            for (int c = 0; c < i->childCount(); ++c)
                walk(i->child(c));
        };
    walk(root);
    Q_UNUSED(facedKeys);
    // VERIFIED TRUTH on Qt 6.11 (11b delta review DR-1): the measure
    // buttons ARE nodes of the QAccessible tree — proved by OBJECT
    // identity, not names (closed flyout MenuItems share the cataloged
    // names) — so the fix-round comment's "skipped by the a11y tree"
    // is not literally true at tree level. What holds, and what this
    // pin therefore guards: every measure-row node carries the
    // INVISIBLE state (the bit AT bridges suppress on), so no phantom
    // button surfaces to AT users. Accessible.ignored: true (QtQuick
    // 6.8+) is the recorded one-line tightening; when it lands, flip
    // this pin to assert absence (!nodes.contains(b)).
    int measureNodes = 0;
    for (QQuickItem *b : m_measure) {
        if (!nodes.contains(b))
            continue;
        ++measureNodes;
        QAccessibleInterface *bi =
            QAccessible::queryAccessibleInterface(b);
        QVERIFY(bi);
        QVERIFY2(bi->state().invisible,
                 "a measure-row a11y node LOST the invisible state — "
                 "phantom palette buttons would surface to AT users");
    }
    qInfo() << measureNodes << "of" << m_measure.size()
            << "measure buttons present as invisible-state a11y nodes "
               "(DR-1: comment claims the tree skips them)";
    // ...while the real slots ARE exposed (tree walk not vacuous).
    int slotsExposed = 0;
    for (QQuickItem *s : m_slots)
        if (nodes.contains(s))
            ++slotsExposed;
    QCOMPARE(slotsExposed, m_slots.size());
}

void TstPalettePins::d6ClickAnnouncesKeyboardActivationSuppressed()
{
    QSignalSpy spy(m_announcer, &Announcer::announced);
    QVERIFY(spy.isValid());
    auto toolAnnouncements = [&spy]() {
        int c = 0;
        for (const auto &row : spy)
            if (row.at(1).toString() == QLatin1String("a11y.tool.selected"))
                ++c;
        return c;
    };

    // Pointer path: click the eraser slot from canvas focus.
    setCanvasTool("brush.stroke");
    focusCanvas();
    quiesceAnnouncer();
    spy.clear();
    QQuickItem *eraser = slotFacing(QStringLiteral("eraser.stroke"));
    QVERIFY(eraser);
    revealSlot(eraser); // §5 fix round: clipped viewport
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      center(eraser));
    QTRY_COMPARE(canvasTool(), QStringLiteral("eraser.stroke"));
    // The D6-corrected record's load-bearing claim: the click did NOT
    // move focus into the pane.
    QQuickItem *fi = m_window->activeFocusItem();
    QVERIFY2(!fi || !insidePalette(fi),
             "pointer click moved focus INTO the palette pane — the "
             "corrected D6 record is wrong on this platform");
    QTRY_VERIFY2_WITH_TIMEOUT(toolAnnouncements() == 1,
                              "slot-click tool change did not announce "
                              "through the live region (D6 record)",
                              1500);

    // Keyboard path: Shift+F6 into the pane, Space-activate the move
    // slot — suppressed (the platform speaks the focused control
    // itself). The click above must have LEFT canvas focus intact
    // (that is the D6 claim), so the pane chord still routes.
    quiesceAnnouncer();
    spy.clear();
    QVERIFY2(m_canvas->hasActiveFocus(),
             "pointer click stole canvas focus — D6 record wrong");
    QTest::keyClick(m_window, Qt::Key_F6, Qt::ShiftModifier);
    QQuickItem *slot = m_window->activeFocusItem();
    QVERIFY(slot && insidePalette(slot));
    // The scope restores its last-focused slot; make sure activating
    // it CHANGES the tool (a no-op Space would make the suppression
    // check vacuous).
    auto faceOf = [](QQuickItem *s) {
        return s->property("faceTool").toMap()
            .value(QStringLiteral("tool")).toString();
    };
    if (faceOf(slot) == canvasTool()) {
        QTest::keyClick(m_window, Qt::Key_Tab);
        slot = m_window->activeFocusItem();
        QVERIFY(slot && insidePalette(slot));
    }
    const QString face = faceOf(slot);
    QVERIFY(!face.isEmpty() && face != canvasTool());
    QTest::keyClick(m_window, Qt::Key_Space);
    QTRY_COMPARE(canvasTool(), face);
    QTest::qWait(900); // one full rate-limit window + margin
    QVERIFY2(toolAnnouncements() == 0,
             "keyboard-focused activation ANNOUNCED — the no-double-"
             "speak guard stopped covering the D3 case");
    QTest::keyClick(m_window, Qt::Key_Escape); // restore canvas focus
}

void TstPalettePins::d6FlyoutDrivenChangeAnnouncesAndUpdatesFace()
{
    QSignalSpy spy(m_announcer, &Announcer::announced);
    QVERIFY(spy.isValid());
    setCanvasTool("brush.stroke");
    focusCanvas();
    quiesceAnnouncer();
    spy.clear();

    // item 17: select.rect now lives in the MERGED selection slot —
    // slotForMember is member-keyed, so the probe carries over.
    QQuickItem *marquee = slotForMember(QStringLiteral("select.rect"));
    QVERIFY(marquee);
    revealSlot(marquee); // §5 fix round: clipped viewport
    QObject *fly = flyoutOf(marquee);
    QVERIFY(fly);
    QTest::mouseClick(m_window, Qt::RightButton, Qt::NoModifier,
                      center(marquee));
    QTRY_VERIFY(fly->property("opened").toBool());
    // Click the ellipse member item (via the Menu's own item table —
    // popup content is not QObject-tree reachable).
    QQuickItem *ellipseItem = nullptr;
    const int itemCount = fly->property("count").toInt();
    for (int i = 0; i < itemCount; ++i) {
        QQuickItem *it = nullptr;
        QMetaObject::invokeMethod(fly, "itemAt",
                                  Q_RETURN_ARG(QQuickItem *, it),
                                  Q_ARG(int, i));
        if (it && it->property("text").toString()
                      == catTr("ui.tool.select_ellipse")) {
            ellipseItem = it;
            break;
        }
    }
    QVERIFY2(ellipseItem, "ellipse member item not found in the flyout");
    QTRY_VERIFY(ellipseItem->isVisible());
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      center(ellipseItem));
    QTRY_COMPARE(canvasTool(), QStringLiteral("select.ellipse"));
    // Last-used face update (framework regression).
    QTRY_COMPARE(marquee->property("faceTool").toMap()
                     .value(QStringLiteral("tool")).toString(),
                 QStringLiteral("select.ellipse"));
    // Overlay-hosted => announces (D6 record).
    bool announced = false;
    QTRY_VERIFY_WITH_TIMEOUT(
        [&]() {
            for (const auto &row : spy)
                if (row.at(1).toString()
                    == QLatin1String("a11y.tool.selected"))
                    announced = true;
            return announced;
        }(),
        1500);
    QVERIFY2(announced, "flyout-driven tool change did not announce");
    setCanvasTool("brush.stroke");
}

void TstPalettePins::regressionPaneCycleOrderPreserved()
{
    // UPDATED at v1.0.1 item 11d Phase 1 (the D5/A11Y-5 closure, desk
    // record a4510f0): the options pane JOINED the cycle on O-QA's
    // CORRECTED order [OPTIONS, PALETTE, CANVAS, PANELS] — this pin now
    // guards the 11b invariants that SURVIVE (palette membership,
    // §31.2 palette-before-canvas, Esc-to-canvas) inside the corrected
    // order. The full 11d matrix (skip rule, exact landings, modal
    // no-op) lives in tst_pane_cycle. The brush tool is active here, so
    // the options row is POPULATED (no C2 skip in this walk).
    setCanvasTool("brush.stroke");
    // The options pane handle, by qml id (the palette-resolution idiom).
    QQmlExpression toolbarExpr(qmlContext(m_window), m_window,
                               QStringLiteral("toolbarPane"));
    auto *toolbar = qobject_cast<QQuickItem *>(
        toolbarExpr.evaluate().value<QObject *>());
    QVERIFY2(toolbar, "toolbarPane not resolvable");
    const auto insideToolbar = [&](QQuickItem *item) {
        for (QQuickItem *n = item; n; n = n->parentItem())
            if (n == toolbar)
                return true;
        return false;
    };
    focusCanvas();
    QTest::keyClick(m_window, Qt::Key_F6);
    QQuickItem *fi = m_window->activeFocusItem();
    QVERIFY2(fi && !insidePalette(fi) && !insideToolbar(fi)
                 && !m_canvas->hasActiveFocus(),
             "F6 #1 from canvas: focus not in the panels pane");
    QTest::keyClick(m_window, Qt::Key_F6);
    QVERIFY2(insideToolbar(m_window->activeFocusItem()),
             "F6 #2: focus did not wrap to the OPTIONS pane (11d order)");
    QTest::keyClick(m_window, Qt::Key_F6);
    QVERIFY2(insidePalette(m_window->activeFocusItem()),
             "F6 #3: focus did not advance options -> palette");
    QTest::keyClick(m_window, Qt::Key_F6);
    QTRY_VERIFY2(m_canvas->hasActiveFocus(), "F6 #4: focus not on canvas");
    // §31.2 palette-before-canvas holds: backward from the canvas still
    // lands in the palette (unchanged by the 11d insertion).
    QTest::keyClick(m_window, Qt::Key_F6, Qt::ShiftModifier);
    QVERIFY2(insidePalette(m_window->activeFocusItem()),
             "Shift+F6 from canvas: focus not in the palette");
    QTest::keyClick(m_window, Qt::Key_Escape);
    QTRY_VERIFY(m_canvas->hasActiveFocus());
}

void TstPalettePins::d7PuppetWarpMenuDoorStillWorks()
{
    // 1.0 final (corrigendum (l)): Puppet Warp is REMOVED. Neither the
    // Edit-menu door nor the palette row exists; the name is kept for the
    // record and now pins the absence.
    for (QObject *o : m_window->findChildren<QObject *>())
        if (o->inherits("QQuickMenuItem"))
            QVERIFY2(o->property("text").toString() != catTr("menu.edit.puppet_warp"),
                     "a Puppet Warp menu item is back; it was removed for 1.0");
    QVERIFY2(!slotForMember(QStringLiteral("transform.puppet")),
             "Puppet Warp is back in the palette");
}

void TstPalettePins::c1WarpFlyoutSelectsAnnouncesAndFaceFlips()
{
    // 1.0 final (corrigendum (l)): Warp is REMOVED. The transform slot is
    // Transform + Perspective; its flyout offers no Warp member.
    QVERIFY2(!slotForMember(QStringLiteral("transform.warp")),
             "Warp is back in the palette; it was removed for 1.0");
    QQuickItem *slot = slotForMember(QStringLiteral("transform.free"));
    QVERIFY(slot);
    QCOMPARE(slot->property("group").toList().size(), 2);
}

void TstPalettePins::c2WarpArmedNotLiveCanvasInertWidthHolds()
{
    // 1.0 final (corrigendum (l)): Warp is REMOVED, so there is no
    // measure-row button carrying its face either.
    for (QQuickItem *b : m_measure)
        QVERIFY2(b->property("text").toString() != catTr("ui.tool.warp"),
                 "a measure-row button still carries the removed Warp face");
}

void TstPalettePins::r1Tier2MembersSelectAnnounceFaceFlip()
{
    // TIER-2 R1 (the C1 shape swept over EVERY new member): selecting
    // a member commits the canvas tool truth, flips the slot face to
    // the glyphless TEXT face (all 16 are glyphless — the vendor round
    // is O-BUILD's), and ANNOUNCES the MINTED cataloged name. Each
    // member's toolNameKeys row is the per-member discriminator.
    QSignalSpy spy(m_announcer, &Announcer::announced);
    QVERIFY(spy.isValid());
    int count = 0;
    const Tier2Member *members = tier2Members(&count);
    // 17 -> 16 on 2026-08-28: smudge.stroke left the tier-2 table when it
    // joined the dispatch chain (192b34b7). THIS PIN CAUGHT THAT EDIT --
    // it is a population-size guard and it fired on a change to the shared
    // table made for the OTHER consumer at :1799. Updated deliberately
    // with the reason, not bumped: a size pin that gets silently
    // incremented whenever it complains is not a pin.
    // 12 -> 11 -> 10 on 2026-08-29: heal.stroke left on Q-D1(a) and
    // dodge_burn.sponge on Q-D3(b). Updated DELIBERATELY WITH THE REASON
    // each time and never bumped -- a size pin that gets silently
    // incremented whenever it complains is not a pin. (17 -> 16 was
    // smudge on 08-28; the four dodge/blur members went the same way.)
    // ★ The table is now doing its real job: it holds the members that
    // are STILL dead, and every departure has been a product ruling
    // rather than a fix.
    // 10 -> 1 on 2026-10-11, corrigendum (l): the nine dead members were
    // removed from the palette, not wired; clone is what is left.
    QCOMPARE(count, 1);
    for (int mi = 0; mi < count; ++mi) {
        const Tier2Member &m = members[mi];
        setCanvasTool("brush.stroke");
        focusCanvas();
        quiesceAnnouncer();
        spy.clear();

        QQuickItem *slot = slotForMember(QLatin1String(m.wire));
        QVERIFY2(slot,
                 qPrintable(QStringLiteral("no palette slot's group "
                                           "carries %1 (tier-2 row "
                                           "missing)")
                                .arg(QLatin1String(m.wire))));
        const QVariantList group = slot->property("group").toList();
        revealSlot(slot); // §5 fix round: clipped viewport (the slice
                          // slot sits below the fold at 1280x800)
        if (group.size() == 1) {
            // Single-member slot (paint-bucket): a plain click selects
            // — no flyout exists (D2 pins the empty description).
            QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                              center(slot));
        } else {
            QObject *fly = flyoutOf(slot);
            QVERIFY2(fly, m.wire);
            QTest::mouseClick(m_window, Qt::RightButton, Qt::NoModifier,
                              center(slot));
            QTRY_VERIFY(fly->property("opened").toBool());
            QQuickItem *memberItem = nullptr;
            const int itemCount = fly->property("count").toInt();
            for (int i = 0; i < itemCount; ++i) {
                QQuickItem *it = nullptr;
                QMetaObject::invokeMethod(fly, "itemAt",
                                          Q_RETURN_ARG(QQuickItem *, it),
                                          Q_ARG(int, i));
                if (it && it->property("text").toString() == catTr(m.key)) {
                    memberItem = it;
                    break;
                }
            }
            QVERIFY2(memberItem,
                     qPrintable(QStringLiteral("member item %1 not found "
                                               "in its flyout (cataloged-"
                                               "name lookup)")
                                    .arg(catTr(m.key))));
            QTRY_VERIFY(memberItem->isVisible());
            QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                              center(memberItem));
        }
        QTRY_COMPARE(canvasTool(), QLatin1String(m.wire));
        // Last-used face flip to the TEXT-LABELED face.
        QTRY_COMPARE(slot->property("faceTool").toMap()
                         .value(QStringLiteral("tool")).toString(),
                     QLatin1String(m.wire));
        QCOMPARE(slot->property("text").toString(), catTr(m.key));
        QVERIFY2(slot->property("checked").toBool(), m.wire);
        // Announce with the MINTED cataloged name (pointer paths
        // announce — the D6 record; the toolNameKeys row is what a
        // mutation drop reds here).
        bool announced = false;
        QTRY_VERIFY2_WITH_TIMEOUT(
            [&]() {
                for (const auto &row : spy)
                    if (row.at(1).toString()
                            == QLatin1String("a11y.tool.selected")
                        && row.at(2).toString() == catTr(m.key))
                        announced = true;
                return announced;
            }(),
            qPrintable(QStringLiteral("%1 selection did not announce "
                                      "the cataloged name (toolNameKeys "
                                      "row missing? — the pre-11c warp "
                                      "silent-skip class)")
                           .arg(QLatin1String(m.wire))),
            1500);
    }
    setCanvasTool("brush.stroke");
}

void TstPalettePins::r2Tier2MembersArmedNotLiveCanvasInert()
{
    // TIER-2 R2 (the C2 armed-record shape over the 16 members that are STILL not live —
    // first-hand: canvas_view.cpp's input if-chain has no branch for
    // any of them): with each armed, a canvas press/drag/release is
    // INERT. §5 FIX-ROUND UPGRADE (review minor-2): "inert" is now
    // pinned on FOUR truths per member, because history length alone
    // proves engine OUTCOME, not shell wiring — the review live-wired
    // dodge into the brush branch and this pin stayed green (the
    // engine rejects the unknown kind; history never grows):
    //   1. ZERO submission ATTEMPTS (submissionAttempts() — bumped at
    //      entry of applyCommand + strokeBegin, before the engine
    //      answers: the wiring discriminator, REDs the reviewer's
    //      demonstration);
    //   2. zero strokeCommitted emissions (no stroke lifecycle ran);
    //   3. status line unchanged (no rejection surfaced);
    //   4. no history growth + the tool stays armed (the original
    //      outcome truths).
    // One sensitivity guard (RULE 9) proves the probe path can see a
    // submit on BOTH instruments (attempts AND history).
    //
    // ITEM 25b (HONESTY SWEEP) — THIS PIN IS DELIBERATELY SPLIT.
    // Instrument 3 INVERTS and the other three STAY. "Inert" used to
    // mean "the shell says nothing", which is the dishonesty rule D4
    // forbids: the user clicked, the slot checked, the name announced,
    // and the canvas did nothing forever. The engine-silence half is
    // still the whole point — armed tools must reach NO engine door —
    // so 1, 2 and 4 are unchanged and are what prove the announcement
    // is shell-local. The status half now asserts the OPPOSITE of what
    // it did: the press must MOVE the status line to this member's
    // resolved pending wording, and must put exactly ONE announcement
    // on the existing announcer path with the right reason id.
    focusCanvas();
    const QPoint mid = m_canvas
                           ->mapToScene(QPointF(m_canvas->width() / 2.0,
                                                m_canvas->height() / 2.0))
                           .toPoint();
    int historyBefore = m_controller->historyEntries().size();
    QSignalSpy commitSpy(m_controller, &DocumentController::strokeCommitted);
    QVERIFY(commitSpy.isValid());
    QSignalSpy announceSpy(m_announcer, &Announcer::announced);
    QVERIFY(announceSpy.isValid());
    int count = 0;
    const Tier2Member *members = tier2Members(&count);
    for (int mi = 0; mi < count; ++mi) {
        const Tier2Member &m = members[mi];
        setCanvasTool(m.wire);
        QCoreApplication::processEvents();
        // The pending family shares ONE announcer source, so successive
        // members coalesce latest-wins inside the 500 ms window. Drain
        // it, then clear, so each member's row is provably its own.
        quiesceAnnouncer();
        announceSpy.clear();
        // Expected wording, resolved from the REAL catalog: the
        // session-shaped members take the class-E id, clone takes the
        // source-point sentence, everyone else the one {tool} template.
        // Recomputed here, never copied (RULE 8) -- this rule is stated
        // test-side ON PURPOSE. Reading Main.qml's pendingReasonIds map
        // instead would make this a tautology that agrees with whatever
        // the shell currently says.
        //
        // Q-D21 RULED (a), 2026-09-01: clone.stroke leaves the
        // tool_pending class. "Clone needs a source point" says WHY,
        // which is strictly more than "not wired", and it stays true
        // after priority 5 wires clone. The three instruments that
        // prove INERTNESS (attempts, strokeCommitted, history) are
        // untouched by that ruling and still apply to clone unchanged;
        // only the wording half moves.
        const QString expectId =
            (qstrcmp(m.wire, "selection.transform") == 0)
                ? QStringLiteral("ui.pending.session")
                : (qstrcmp(m.wire, "clone.stroke") == 0)
                      ? QStringLiteral("ui.status.clone_needs_source")
                      : QStringLiteral("ui.status.tool_pending");
        const QString expectText = m_i18n->tr(
            expectId,
            QVariantMap{{QStringLiteral("tool"), catTr(m.key)}});
        QVERIFY2(!expectText.isEmpty() && !expectText.contains(expectId),
                 qPrintable(QStringLiteral("catalog key-echoed %1 — the "
                                           "pending wording asserts below "
                                           "would be vacuous")
                                .arg(expectId)));
        if (qstrcmp(m.wire, "clone.stroke") == 0)
            QVERIFY2(expectText
                         != m_i18n->tr(
                             QStringLiteral("ui.status.tool_pending"),
                             QVariantMap{{QStringLiteral("tool"),
                                          catTr(m.key)}}),
                     "clone's sentence has been reworded back into the "
                     "not-wired sentence -- Q-D21's distinction (say WHY, "
                     "not just 'not wired') has dissolved in the catalog "
                     "while every assert below still passes");
        const int attemptsBefore = m_controller->submissionAttempts();
        QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier, mid);
        QTest::mouseMove(m_window, mid + QPoint(40, 25));
        QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                            mid + QPoint(40, 25));
        QTest::qWait(80);
        QVERIFY2(m_controller->submissionAttempts() == attemptsBefore,
                 qPrintable(QStringLiteral(
                                "canvas NOT inert with %1 armed — the "
                                "drag produced a submission ATTEMPT "
                                "(live wiring present; review minor-2: "
                                "an engine-REJECTED submission never "
                                "grows history, so only attempt truth "
                                "can see it)")
                                .arg(QLatin1String(m.wire))));
        QVERIFY2(commitSpy.isEmpty(),
                 qPrintable(QStringLiteral("canvas NOT inert with %1 "
                                           "armed — a stroke lifecycle "
                                           "ran (strokeCommitted fired)")
                                .arg(QLatin1String(m.wire))));
        // 25b: the status line MUST move, to this member's wording.
        QCOMPARE(m_controller->statusText(), expectText);
        // ... and exactly one announcement, on the EXISTING announcer
        // path, carrying the member's own reason id and resolved name.
        QTRY_VERIFY2_WITH_TIMEOUT(
            [&]() {
                for (const auto &row : announceSpy)
                    if (row.at(0).toString()
                        == QLatin1String("tool_pending"))
                        return true;
                return false;
            }(),
            qPrintable(QStringLiteral("%1's press announced NOTHING — the "
                                      "armed-silent press is still silent "
                                      "(rule D4)")
                           .arg(QLatin1String(m.wire))),
            1500);
        int pendingRows = 0;
        for (const auto &row : announceSpy) {
            if (row.at(0).toString() != QLatin1String("tool_pending"))
                continue;
            ++pendingRows;
            QCOMPARE(row.at(1).toString(), expectId);
            QCOMPARE(row.at(2).toString(), expectText);
        }
        QCOMPARE(pendingRows, 1);
        QVERIFY2(m_controller->historyEntries().size() == historyBefore,
                 qPrintable(QStringLiteral("canvas NOT inert with %1 "
                                           "armed — history grew (the "
                                           "armed-only record is wrong "
                                           "for this member)")
                                .arg(QLatin1String(m.wire))));
        QCOMPARE(canvasTool(), QLatin1String(m.wire));
    }
    // Sensitivity guard (C2's engine-truth pattern, same coordinates):
    // the identical gesture with the brush moves BOTH instruments —
    // the attempt counter (strokeBegin's entry bump) and history.
    setCanvasTool("brush.stroke");
    const int attemptsBefore = m_controller->submissionAttempts();
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier, mid);
    QTest::mouseMove(m_window, mid + QPoint(40, 25));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        mid + QPoint(40, 25));
    QTRY_VERIFY2_WITH_TIMEOUT(
        m_controller->historyEntries().size() > historyBefore,
        "sensitivity guard: a brush drag on the same probe path did not "
        "grow history — the 16 inert asserts would be vacuous", 3000);
    QVERIFY2(m_controller->submissionAttempts() > attemptsBefore,
             "sensitivity guard: the brush drag did not move the attempt "
             "counter — the 16 zero-attempt asserts would be vacuous");
    QVERIFY2(!commitSpy.isEmpty(),
             "sensitivity guard: the brush drag did not emit "
             "strokeCommitted — the zero-emission asserts would be "
             "vacuous");

    // ---- THE FOUR PALETTE-MODE TOOLS THAT CROSSED WITH SMUDGE ----
    // Same shape as the smudge arm below: the identical gesture that must
    // move NOTHING for the members still in the table must MOVE the
    // attempt counter for these. Listing one here without wiring it reds
    // just as loudly as wiring one without listing it.
    for (const char *live : {"dodge_burn.dodge", "dodge_burn.burn",
                             "blur_sharpen.blur", "blur_sharpen.sharpen"}) {
        setCanvasTool(live);
        QCoreApplication::processEvents();
        const int liveBefore = m_controller->submissionAttempts();
        QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier, mid);
        QTest::mouseMove(m_window, mid + QPoint(20, 0));
        QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                            mid + QPoint(20, 0));
        QVERIFY2(m_controller->submissionAttempts() > liveBefore,
                 qPrintable(QStringLiteral("%1 is listed as WIRED and a canvas "
                                           "drag raised no submission attempt — "
                                           "the wiring was lost")
                                .arg(QLatin1String(live))));
    }

    // ---- THE RELOCATED MEMBER (192b34b7). smudge.stroke used to sit in
    // the table above and be asserted INERT. It is dispatched now, so the
    // claim about it INVERTS rather than disappearing: the same gesture
    // that must move nothing for the other sixteen must move the attempt
    // counter for this one.
    //
    // WHY THIS ARM EXISTS AT ALL RATHER THAN JUST DROPPING THE ROW: a pin
    // deleted because your own change broke it is how coverage dies
    // quietly. The table's claim was never "smudge does nothing" -- it was
    // "armed does not mean live" -- and that claim still needs a witness
    // on BOTH sides once one member crosses over.
    setCanvasTool("smudge.stroke");
    QCoreApplication::processEvents();
    const int smudgeAttemptsBefore = m_controller->submissionAttempts();
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier, mid);
    QTest::mouseMove(m_window, mid + QPoint(30, 18));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        mid + QPoint(30, 18));
    // AND THE HALF THE RELOCATION DROPPED, restored here rather than
    // reported. Removing smudge from tier2Members took it out of r2 —
    // which was the point — but the table has TWO consumers, and r1
    // (select · announce · face flip) lost its only witness for this
    // member at the same time. Nothing else covers that claim: the other
    // smudge rows in this file are the keyboard-unreachable set and the
    // slot census, which assert different things.
    //
    // A relocation that keeps one of a member's two claims and silently
    // drops the other is a smaller version of the row deletion it was
    // written to avoid.
    setCanvasTool("brush.stroke");
    focusCanvas();
    QQuickItem *smudgeSlot = slotForMember(QLatin1String("smudge.stroke"));
    QVERIFY2(smudgeSlot,
             "no palette slot's group carries smudge.stroke — the row that "
             "r1 used to check for this member is gone and nothing replaced it");
    setCanvasTool("smudge.stroke");
    QTRY_VERIFY2(smudgeSlot->property("checked").toBool(),
                 "the smudge slot did not take the checked face when armed");

    QVERIFY2(m_controller->submissionAttempts() > smudgeAttemptsBefore,
             "smudge.stroke is in the dispatch chain (192b34b7) but a drag "
             "produced NO submission attempt — either the wiring regressed "
             "or this member belongs back in the tier-2 table");

    // 1.0 final (corrigendum (l), WORD 41 (4)): THE REMOVAL SET, pinned as
    // ABSENCE. Every member that was armed-but-not-live is out of the
    // palette entirely: no slot carries it, so no click or flyout can arm
    // it. (Heal stays: WORD 53, the operator's overrule.)
    for (const char *wire : {"select.quick", "content_aware_fill", "fill",
                             "text", "shape.line", "shape.polygon", "shape.rounded_rect",
                             "transform.puppet", "transform.warp", "selection.transform",
                             "slice.add", "slice.select"})
        QVERIFY2(!slotForMember(QLatin1String(wire)),
                 qPrintable(QStringLiteral("removed tool %1 is back in the palette")
                                .arg(QLatin1String(wire))));

    // HEAL'S POSITIVE ARM — operator ruling Q-D1(a), 2026-08-29. Written
    // in the SAME SHAPE as smudge's above, deliberately: the relocation
    // that moved smudge dropped half its coverage (r1's select · announce
    // · face-flip witness) and had to restore it here. Doing heal any
    // other way would re-earn that lesson.
    setCanvasTool("brush.stroke");
    focusCanvas();
    QQuickItem *healSlot = slotForMember(QLatin1String("heal.stroke"));
    QVERIFY2(healSlot,
             "no palette slot's group carries heal.stroke — the row that "
             "r1 used to check for this member is gone and nothing replaced it");
    setCanvasTool("heal.stroke");
    QTRY_VERIFY2(healSlot->property("checked").toBool(),
                 "the heal slot did not take the checked face when armed");
    QCoreApplication::processEvents();
    const int healAttemptsBefore = m_controller->submissionAttempts();
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier, mid);
    QTest::mouseMove(m_window, mid + QPoint(30, 18));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        mid + QPoint(30, 18));
    QVERIFY2(m_controller->submissionAttempts() > healAttemptsBefore,
             "heal.stroke is in the dispatch chain (Q-D1(a)) but a drag "
             "produced NO submission attempt — either the wiring regressed "
             "or this member belongs back in the tier-2 table");

    // SPONGE'S POSITIVE ARM — operator ruling Q-D3(b), 2026-08-29. Same
    // shape as heal's and smudge's, for the same reason: a relocation
    // that keeps one of a member's two claims and drops the other is a
    // smaller version of the row deletion it was written to avoid.
    setCanvasTool("brush.stroke");
    focusCanvas();
    QQuickItem *spongeSlot = slotForMember(QLatin1String("dodge_burn.sponge"));
    QVERIFY2(spongeSlot,
             "no palette slot's group carries dodge_burn.sponge — the row that "
             "r1 used to check for this member is gone and nothing replaced it");
    setCanvasTool("dodge_burn.sponge");
    QTRY_VERIFY2(spongeSlot->property("checked").toBool(),
                 "the sponge slot did not take the checked face when armed");
    QCoreApplication::processEvents();
    const int spongeAttemptsBefore = m_controller->submissionAttempts();
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier, mid);
    QTest::mouseMove(m_window, mid + QPoint(30, 18));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        mid + QPoint(30, 18));
    QVERIFY2(m_controller->submissionAttempts() > spongeAttemptsBefore,
             "dodge_burn.sponge is in the dispatch chain (Q-D3(b)) but a drag "
             "produced NO submission attempt — either the wiring regressed "
             "or this member belongs back in the tier-2 table");

    // ★★★ THE LABEL IS PART OF THE RULING, SO IT IS PINNED TO THE
    // BEHAVIOUR AND NOT TO A STRING.
    //
    // Q-D3(b) is "desaturate WITH A LABEL, so the interim mapping is
    // never silent". The label lives in the catalog (so every render site
    // — face, flyout, tooltip, Accessible.name, announcer — gets it with
    // no QML change) and it names the WIRE MODE untranslated, like an
    // error code, because translating it would let the visible word drift
    // from the enum value it describes.
    //
    // THIS ASSERTION TIES THE TWO TOGETHER: whatever mode the drag above
    // actually sent must appear inside the label the user reads. Pinning
    // the literal "Sponge (desaturate)" instead would go green on a tool
    // relabelled correctly and REWIRED to `saturate` — the exact
    // two-opposite-things failure the operator refused to let a desk
    // guess at, arriving through the half nobody was watching.
    const QString sentMode = m_controller->lastStrokeMode();
    QCOMPARE(sentMode, QStringLiteral("desaturate")); // the ruling itself
    QVERIFY2(catTr("ui.tool.sponge").contains(sentMode),
             qPrintable(QStringLiteral("the sponge label %1 does not name the "
                                       "mode it actually sends (%2) — the "
                                       "interim mapping has gone silent")
                            .arg(catTr("ui.tool.sponge"), sentMode)));
}

void TstPalettePins::r3SwatchPairStateFeedsBrushAndFillParams()
{
    // TIER-2 R3: the fg/bg swatch pair (census slot 19; ToolPalette
    // header record). The wells are named by the MINTED ids and
    // DISPLAY the state; the FEED claims are probed on engine truth:
    // the Fill dialog's color param seeds from the fg state and the
    // applied fill's PIXELS are that color (composite readback through
    // the clipboard door); a brush stroke with a changed fg paints
    // that color (readback scan). R15 stays honest: only the COLOR
    // param is fed — the engine's foreground/background content arms
    // are untouched by the pair.
    QQuickItem *fg = m_palette->findChild<QQuickItem *>(
        QStringLiteral("swatchForegroundWell"));
    QQuickItem *bg = m_palette->findChild<QQuickItem *>(
        QStringLiteral("swatchBackgroundWell"));
    QVERIFY2(fg, "foreground swatch well not found in the palette");
    QVERIFY2(bg, "background swatch well not found in the palette");
    QVERIFY(fg->isVisible() && bg->isVisible());
    QCOMPARE(accessibleName(fg), catTr("ui.tool.swatch_foreground"));
    QCOMPARE(accessibleName(bg), catTr("ui.tool.swatch_background"));

    // Display bindings: fg IS canvas.brushColor (no duplicate state);
    // bg is the palette's own state.
    QVERIFY(m_canvas->setProperty("brushColor", QColor(Qt::red)));
    QQuickItem *fgRect = fg->property("contentItem").value<QQuickItem *>();
    QVERIFY(fgRect);
    QTRY_COMPARE(fgRect->property("color").value<QColor>(),
                 QColor(Qt::red));
    QVERIFY(m_palette->setProperty("swatchBackgroundColor",
                                   QColor(0x33, 0x66, 0xcc)));
    QQuickItem *bgRect = bg->property("contentItem").value<QQuickItem *>();
    QVERIFY(bgRect);
    QTRY_COMPARE(bgRect->property("color").value<QColor>(),
                 QColor(0x33, 0x66, 0xcc));

    // §5 FIX-ROUND UPGRADE (review MAJOR-1): VIEWPORT TRUTH. The old
    // isVisible() checks the QQuickItem visible-property chain only —
    // it was TRUE while the pair rendered zero pixels below the fold.
    // The wells' scene rects must sit fully inside the window AND the
    // palette bounds at the default 1280x800.
    const QRectF winRect(0, 0, m_window->width(), m_window->height());
    const QRectF palRect(m_palette->mapToScene(QPointF(0, 0)),
                         QSizeF(m_palette->width(), m_palette->height()));
    for (QQuickItem *well : {fg, bg}) {
        const QRectF r(well->mapToScene(QPointF(0, 0)),
                       QSizeF(well->width(), well->height()));
        QVERIFY2(winRect.contains(r),
                 qPrintable(QStringLiteral(
                                "%1 scene rect %2,%3 %4x%5 is not fully "
                                "inside the %6x%7 window — the swatch "
                                "pair is below the fold again (review "
                                "MAJOR-1)")
                                .arg(well->objectName())
                                .arg(r.x()).arg(r.y())
                                .arg(r.width()).arg(r.height())
                                .arg(m_window->width())
                                .arg(m_window->height())));
        QVERIFY2(palRect.contains(r),
                 qPrintable(QStringLiteral("%1 is outside the palette's "
                                           "own bounds")
                                .arg(well->objectName())));
    }
    qInfo() << "wells scene y" << fg->mapToScene(QPointF(0, 0)).y()
            << "window" << m_window->height()
            << "palette bottom" << palRect.bottom();

    // PIXEL TRUTH (the review's window-grab proof, standing pin): the
    // wells' actual colors at their own coordinates — fg was just set
    // red, bg #3366cc; the grab must SEE them (a zero-pixel pair
    // cannot pass this regardless of any visible-property claim).
    const QImage grab = m_window->grabWindow();
    QVERIFY(!grab.isNull());
    const qreal dpr = grab.width() / qreal(m_window->width());
    const QPoint fgMid = center(fg);
    const QPoint bgMid = center(bg);
    const QColor fgPx = grab.pixelColor(int(fgMid.x() * dpr),
                                        int(fgMid.y() * dpr));
    const QColor bgPx = grab.pixelColor(int(bgMid.x() * dpr),
                                        int(bgMid.y() * dpr));
    QVERIFY2(fgPx.red() > 200 && fgPx.green() < 80 && fgPx.blue() < 80,
             qPrintable(QStringLiteral("window grab at the fg well's "
                                       "center is %1, not the red the "
                                       "well displays — zero pixels "
                                       "painted (review MAJOR-1 class)")
                            .arg(fgPx.name())));
    QVERIFY2(qAbs(bgPx.red() - 0x33) < 24 && qAbs(bgPx.green() - 0x66) < 24
                 && qAbs(bgPx.blue() - 0xcc) < 24,
             qPrintable(QStringLiteral("window grab at the bg well's "
                                       "center is %1, not the #3366cc "
                                       "the well displays")
                            .arg(bgPx.name())));

    // REAL POINTER EVENTS (review MAJOR-1: "wells are real focusable
    // buttons" must mean CLICKABLE at the shipping geometry). Wired
    // behavior probed first-hand: onClicked opens that well's
    // ColorDialog edit door. A synthesized click at each well's own
    // scene coordinates must fire it.
    QObject *fgDlg = m_palette->findChild<QObject *>(
        QStringLiteral("swatchForegroundDialog"));
    QObject *bgDlg = m_palette->findChild<QObject *>(
        QStringLiteral("swatchBackgroundDialog"));
    QVERIFY2(fgDlg && bgDlg, "swatch ColorDialog edit doors not found");
    QVERIFY(!fgDlg->property("visible").toBool());
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier, fgMid);
    QTRY_VERIFY2(fgDlg->property("visible").toBool(),
                 "a real click at the fg well's coordinates did not "
                 "open its ColorDialog — the well is not actually "
                 "clickable at 1280x800 (review MAJOR-1)");
    QMetaObject::invokeMethod(fgDlg, "close");
    QTRY_VERIFY(!fgDlg->property("visible").toBool());
    QVERIFY(!bgDlg->property("visible").toBool());
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier, bgMid);
    QTRY_VERIFY2(bgDlg->property("visible").toBool(),
                 "a real click at the bg well's coordinates did not "
                 "open its ColorDialog");
    QMetaObject::invokeMethod(bgDlg, "close");
    QTRY_VERIFY(!bgDlg->property("visible").toBool());
    // The dialogs can take window activation with them on the offscreen
    // platform — hand it back before the focus-dependent probes below.
    m_window->requestActivate();
    QTRY_VERIFY(m_window->isActive());

    // FILL param feed: the dialog seeds its color param from the fg
    // state on open (the discriminating assert — without the
    // Main.qml foregroundSeed binding or the onOpened seed line the
    // dialog stays at its standalone #202020).
    QObject *dlg = nullptr;
    const auto all = m_window->findChildren<QObject *>();
    for (QObject *o : all) {
        if (o->inherits("QQuickDialog")
            && o->property("title").toString()
                   == catTr("ui.dialog.fill.title")) {
            dlg = o;
            break;
        }
    }
    QVERIFY2(dlg, "Fill dialog instance not found by cataloged title");
    QMetaObject::invokeMethod(dlg, "open");
    QTRY_VERIFY(dlg->property("opened").toBool());
    QCOMPARE(dlg->property("fillColor").value<QColor>(), QColor(Qt::red));

    // Apply: the committed fill's pixels ARE the seeded color —
    // engine truth via composite readback (the clipboard door reads
    // the engine composite, not a shell echo).
    const int before = m_controller->historyEntries().size();
    QQuickItem *content =
        dlg->property("contentItem").value<QQuickItem *>();
    QVERIFY(content);
    QList<QQuickItem *> items;
    collectItems(content, &items);
    QQuickItem *apply = nullptr;
    for (QQuickItem *it : items) {
        if (it->inherits("QQuickButton")
            && it->property("text").toString() == catTr("ui.dialog.apply")) {
            apply = it;
            break;
        }
    }
    QVERIFY2(apply, "Apply button not found inside the Fill dialog");
    QVERIFY(QMetaObject::invokeMethod(apply, "clicked"));
    QTRY_COMPARE(m_controller->historyEntries().size(), before + 1);
    const QVariantMap lastEntry =
        m_controller->historyEntries().last().toMap();
    QCOMPARE(lastEntry.value(QStringLiteral("kind")).toString(),
             QStringLiteral("fill"));
    QTRY_VERIFY2(!dlg->property("visible").toBool(),
                 "Fill dialog did not close on the ok path");
    QVERIFY(m_controller->copyCompositeToClipboard(QRect()));
    QImage img = QGuiApplication::clipboard()->image();
    QVERIFY(!img.isNull());
    const QColor fillPx(img.pixel(img.width() / 2, img.height() / 2));
    QVERIFY2(fillPx.red() > 250 && fillPx.green() < 5 && fillPx.blue() < 5,
             qPrintable(QStringLiteral("composite center %1 is not the "
                                       "seeded red — the fill color "
                                       "param did not flow from the fg "
                                       "swatch state")
                            .arg(fillPx.name())));

    // BRUSH param feed: the fg state is the SAME state strokeBegin
    // consumes — a green-fg stroke over the red fill must leave
    // green-dominant pixels (scan — view-to-doc mapping-agnostic).
    QVERIFY(m_canvas->setProperty("brushColor", QColor(Qt::green)));
    setCanvasTool("brush.stroke");
    focusCanvas();
    const QPoint mid = m_canvas
                           ->mapToScene(QPointF(m_canvas->width() / 2.0,
                                                m_canvas->height() / 2.0))
                           .toPoint();
    const int h2 = m_controller->historyEntries().size();
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier, mid);
    QTest::mouseMove(m_window, mid + QPoint(40, 25));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        mid + QPoint(40, 25));
    QTRY_VERIFY_WITH_TIMEOUT(m_controller->historyEntries().size() > h2,
                             3000);
    QVERIFY(m_controller->copyCompositeToClipboard(QRect()));
    img = QGuiApplication::clipboard()->image();
    QVERIFY(!img.isNull());
    bool greenFound = false;
    for (int y = 0; y < img.height() && !greenFound; ++y)
        for (int x = 0; x < img.width() && !greenFound; ++x) {
            const QColor px(img.pixel(x, y));
            if (px.green() > 200 && px.red() < 100 && px.blue() < 100)
                greenFound = true;
        }
    QVERIFY2(greenFound,
             "no green-dominant pixel after a green-fg brush stroke — "
             "the fg swatch state did not feed the stroke color params");

    // Restore: undo the stroke and the fill; boot-default states back
    // (canvas_view.h's #141414 fg, white bg).
    m_controller->undo();
    m_controller->undo();
    QVERIFY(m_canvas->setProperty("brushColor", QColor(20, 20, 20)));
    QVERIFY(m_palette->setProperty("swatchBackgroundColor",
                                   QColor(Qt::white)));
    setCanvasTool("brush.stroke");
}

void TstPalettePins::r4SlotsOverflowFocusFollowFooterPersists()
{
    // §5 FIX ROUND (review MAJOR-1 acceptance): the overflow mechanism
    // itself. See the suite header for the pinned claims.
    m_window->requestActivate(); // r3's dialogs may have taken activation
    QTRY_VERIFY(m_window->isActive());

    // 1. RULE 8 re-derivation: the viewport's contentHeight equals the
    // sum of the slot heights + column spacings (11 slots after the 1.0
    // removal set; item 17's merge had made it 14) — the mechanical
    // ground for every number below.
    qreal slotSum = 0;
    for (QQuickItem *s : m_slots)
        slotSum += s->height();
    const qreal spacing = 2.0 * (m_slots.size() - 1);
    const qreal contentH = m_viewport->property("contentHeight").toReal();
    QVERIFY2(qAbs(contentH - (slotSum + spacing)) < 0.5,
             qPrintable(QStringLiteral("viewport contentHeight %1 != "
                                       "sum of slot heights %2 + "
                                       "spacing %3")
                            .arg(contentH).arg(slotSum).arg(spacing)));

    // 2. The overflow RECORD at the default 1280x800 (Main.qml's boot
    // size and the tour's pinned size) — RE-DERIVED for item 17 (the
    // pre-17 pin's own message said "re-derive if the slot set
    // shrank", and it did: 16 -> 14 slots): the 14 slots now FIT the
    // band — no overflow, no scroll, no scrollbar at the shipping
    // default. The mechanism below is still shipped code and still
    // engages at shorter windows, so it is probed at a derived
    // overflow-engaged height, never assumed away.
    QCOMPARE(m_window->height(), 800);
    const qreal vpH = m_viewport->height();
    QVERIFY2(contentH <= vpH + 0.5,
             qPrintable(QStringLiteral("content %1 > viewport %2 at the "
                                       "default 800 px — the post-17 "
                                       "fits record is wrong; re-derive "
                                       "if the slot set grew")
                            .arg(contentH).arg(vpH)));
    QVERIFY(!m_viewport->property("interactive").toBool());
    // The engage boundary: every other row of the window is height-
    // fixed, so the viewport grows 1:1 with the window — the smallest
    // window height at which the content still fits is derived, then
    // probed BOTH ways.
    const int fitsAt = m_window->height() - qFloor(vpH - contentH);
    qInfo() << "slots content" << contentH << "viewport band" << vpH
            << "headroom px" << (vpH - contentH)
            << "-> scroll engages below window height" << fitsAt
            << "(no app minimum window size is set — recorded)";
    m_window->setHeight(fitsAt - 1);
    QTRY_VERIFY2(m_viewport->property("contentHeight").toReal()
                     > m_viewport->height(),
                 "below the derived boundary the overflow must engage");
    QVERIFY(m_viewport->property("interactive").toBool());
    m_window->setHeight(fitsAt);
    QTRY_VERIFY2(m_viewport->property("contentHeight").toReal()
                     <= m_viewport->height() + 0.5,
                 "at the derived boundary the slots must fit without "
                 "scrolling");
    QVERIFY(!m_viewport->property("interactive").toBool());
    // Probe height for the mechanism: decisively overflowing (the
    // 1 px sliver at the boundary would make the scroll/extreme
    // probes vacuously weak). 1.0 final (corrigendum (l)): DERIVED, not
    // the old literal 600 -- after the removal set the 11 slots fit at
    // 600 px natively, so the probe sits 150 px below the boundary.
    const int probeH = fitsAt - 150;
    m_window->setHeight(probeH);
    QTRY_VERIFY2(m_viewport->property("contentHeight").toReal()
                     > m_viewport->height(),
                 qPrintable(QStringLiteral("at %1 px (150 below the derived "
                                           "boundary) the overflow must engage "
                                           "for the mechanism probes below")
                                .arg(probeH)));
    QVERIFY(m_viewport->property("interactive").toBool());

    // 3. A REAL wheel event scrolls the viewport (the scroll surface is
    // a user-drivable mechanism, not a property).
    m_viewport->setProperty("contentY", 0);
    QCoreApplication::processEvents();
    const QPointF vpMid = m_viewport->mapToScene(
        QPointF(m_viewport->width() / 2.0, m_viewport->height() / 2.0));
    QWheelEvent wheel(vpMid, m_window->mapToGlobal(vpMid.toPoint()),
                      QPoint(), QPoint(0, -240), Qt::NoButton,
                      Qt::NoModifier, Qt::NoScrollPhase, false);
    // A hand-built event carries timestamp 0; after a long full-suite
    // run that sits BEHIND the Flickable's internal clock and the wheel
    // is treated as stale (reproduced first-hand: full run red, any
    // subset green). Stamp it with real monotonic time like the
    // platform would.
    wheel.setTimestamp(quint64(QDateTime::currentMSecsSinceEpoch()));
    QVERIFY(QGuiApplication::sendEvent(m_window, &wheel));
    QTRY_VERIFY2(m_viewport->property("contentY").toReal() > 0,
                 "a wheel event over the slots did not scroll the "
                 "viewport");

    // 4. Keyboard focus-follow: walk the whole slot chain by Tab from
    // the first slot — EVERY focused slot must sit fully inside the
    // viewport (keyboard focus never rests on a clipped control — the
    // review's a11y point), and the walk must reach ALL 14 slots and
    // then the two wells.
    m_viewport->setProperty(
        "contentY", m_viewport->property("contentHeight").toReal()
                        - m_viewport->height());
    QCoreApplication::processEvents(); // start scrolled AWAY from slot 1
    m_slots.first()->forceActiveFocus(Qt::TabFocusReason);
    QTRY_VERIFY(m_slots.first()->hasActiveFocus());
    QSet<QQuickItem *> slotsVisited;
    bool fgReached = false;
    bool bgReached = false;
    QQuickItem *fgWell = m_palette->findChild<QQuickItem *>(
        QStringLiteral("swatchForegroundWell"));
    QQuickItem *bgWell = m_palette->findChild<QQuickItem *>(
        QStringLiteral("swatchBackgroundWell"));
    QVERIFY(fgWell && bgWell);
    for (int i = 0; i < 40; ++i) {
        QQuickItem *fi = m_window->activeFocusItem();
        if (!fi || !insidePalette(fi))
            break;
        if (m_slots.contains(fi)) {
            slotsVisited << fi;
            QVERIFY2(fi->isVisible(), "a focused slot is not visible");
            QVERIFY2(slotFullyInViewport(fi),
                     qPrintable(QStringLiteral(
                                    "focused slot '%1' is not fully "
                                    "inside the viewport — the focus-"
                                    "follow broke (keyboard focus on a "
                                    "clipped control, the review's "
                                    "offscreen-focus class)")
                                    .arg(accessibleName(fi))));
        } else if (fi == fgWell) {
            fgReached = true;
        } else if (fi == bgWell) {
            bgReached = true;
        }
        if (slotsVisited.size() == m_slots.size() && fgReached && bgReached)
            break;
        QTest::keyClick(m_window, Qt::Key_Tab);
    }
    QCOMPARE(slotsVisited.size(), m_slots.size());
    QVERIFY2(fgReached && bgReached,
             "the Tab walk did not reach both swatch wells after the "
             "slots — the footer left the keyboard chain");

    // 5. Footer persistence: the wells hold their EXACT scene position
    // (inside the window) at both scroll extremes — the pair can never
    // scroll away.
    m_viewport->setProperty("contentY", 0);
    QCoreApplication::processEvents();
    const QPointF fgAtTop = fgWell->mapToScene(QPointF(0, 0));
    m_viewport->setProperty(
        "contentY", m_viewport->property("contentHeight").toReal()
                        - m_viewport->height());
    QCoreApplication::processEvents();
    const QPointF fgAtBottom = fgWell->mapToScene(QPointF(0, 0));
    QCOMPARE(fgAtBottom, fgAtTop);
    QVERIFY2(fgAtTop.y() + fgWell->height() <= m_window->height() + 0.5,
             "the fg well sits below the window at a scroll extreme");
    QVERIFY2(bgWell->mapToScene(QPointF(0, 0)) .y() + bgWell->height()
                 <= m_window->height() + 0.5,
             "the bg well sits below the window at a scroll extreme");

    // Restore: default window height, viewport to the top, focus to
    // the canvas.
    m_window->setHeight(800);
    QTRY_VERIFY(m_viewport->property("contentHeight").toReal()
                <= m_viewport->height() + 0.5);
    m_viewport->setProperty("contentY", 0);
    QTest::keyClick(m_window, Qt::Key_Escape);
    QTRY_VERIFY(m_canvas->hasActiveFocus());
}

void TstPalettePins::i17MergedSelectionSlotCensusOrderFacesLetters()
{
    // item 17 (operator ruling; suite header): ALL selection tools in
    // ONE flyout slot. Census order across the former marquee/lasso/
    // wand rows; wire ids + cataloged keys pinned pairwise.
    struct Member {
        const char *wire;
        const char *key;
    };
    const Member census[] = {
        {"select.rect", "ui.tool.select_rect"},
        {"select.ellipse", "ui.tool.select_ellipse"},
        {"select.lasso", "ui.tool.lasso"},
        {"select.polygon", "ui.tool.polygon"},
        {"select.wand", "ui.tool.wand"},
        // 1.0 final (corrigendum (l)): Quick Selection removed (6 -> 5).
    };
    const int memberCount = int(sizeof(census) / sizeof(census[0]));

    // 1. Exactly ONE slot carries select.* members ("selection." — the
    // transform group's selection.transform — does not match the
    // "select." prefix: probed on the string, not assumed), and its
    // group is the six members in census order.
    QQuickItem *sel = nullptr;
    int slotsWithSelectMembers = 0;
    for (QQuickItem *s : m_slots) {
        const QVariantList g = s->property("group").toList();
        for (const QVariant &m : g) {
            if (m.toMap().value(QStringLiteral("tool")).toString()
                    .startsWith(QLatin1String("select."))) {
                sel = s;
                ++slotsWithSelectMembers;
                break;
            }
        }
    }
    QCOMPARE(slotsWithSelectMembers, 1);
    QVERIFY(sel);
    const QVariantList group = sel->property("group").toList();
    QCOMPARE(group.size(), memberCount);
    for (int i = 0; i < memberCount; ++i) {
        const QVariantMap row = group.at(i).toMap();
        QCOMPARE(row.value(QStringLiteral("tool")).toString(),
                 QLatin1String(census[i].wire));
        QCOMPARE(row.value(QStringLiteral("key")).toString(),
                 QLatin1String(census[i].key));
    }

    // 2. The OPEN flyout carries the six members as MenuItems in the
    // same order, cataloged names (the D2 surface is separately pinned;
    // this is the flyout itself).
    setCanvasTool("brush.stroke");
    focusCanvas();
    revealSlot(sel);
    QObject *fly = flyoutOf(sel);
    QVERIFY2(fly, "merged selection slot has no flyout Menu");
    QTest::mouseClick(m_window, Qt::RightButton, Qt::NoModifier,
                      center(sel));
    QTRY_VERIFY(fly->property("opened").toBool());
    QCOMPARE(fly->property("count").toInt(), memberCount);
    for (int i = 0; i < memberCount; ++i) {
        QQuickItem *it = nullptr;
        QMetaObject::invokeMethod(fly, "itemAt",
                                  Q_RETURN_ARG(QQuickItem *, it),
                                  Q_ARG(int, i));
        QVERIFY2(it, census[i].wire);
        QCOMPARE(it->property("text").toString(), catTr(census[i].key));
        QVERIFY2(it->property("enabled").toBool(), census[i].wire);
    }

    // 3. Flyout select of a member from a FORMER OTHER SLOT (wand sat
    // two slots away pre-merge — the merge-critical path): commits the
    // canvas tool truth, flips the merged face, announces the
    // cataloged name (live member — pointer paths announce, D6).
    QSignalSpy spy(m_announcer, &Announcer::announced);
    QVERIFY(spy.isValid());
    quiesceAnnouncer();
    spy.clear();
    QQuickItem *wandItem = nullptr;
    QMetaObject::invokeMethod(fly, "itemAt",
                              Q_RETURN_ARG(QQuickItem *, wandItem),
                              Q_ARG(int, 4)); // census: wand
    QVERIFY(wandItem);
    QTRY_VERIFY(wandItem->isVisible());
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      center(wandItem));
    QTRY_COMPARE(canvasTool(), QStringLiteral("select.wand"));
    QTRY_COMPARE(sel->property("faceTool").toMap()
                     .value(QStringLiteral("tool")).toString(),
                 QStringLiteral("select.wand"));
    QVERIFY(sel->property("checked").toBool());
    bool announced = false;
    QTRY_VERIFY_WITH_TIMEOUT(
        [&]() {
            for (const auto &row : spy)
                if (row.at(1).toString()
                        == QLatin1String("a11y.tool.selected")
                    && row.at(2).toString() == catTr("ui.tool.wand"))
                    announced = true;
            return announced;
        }(),
        1500);
    QVERIFY2(announced,
             "wand flyout select in the merged slot did not announce");

    // 4. The single-key TOOL letters are UNCHANGED by the merge (they
    // are tool shortcuts, not group shortcuts) — and each now flips
    // the SAME slot's face (the last-used face crossing the former
    // slot boundaries through the keyboard path).
    // v1.0.1 item 11e — WHY THIS LEG GAINED A SETUP LINE (recorded, not
    // quietly re-pinned): the letters now carry RING MEMORY (Appendix C
    // :18516 [NORMATIVE]: "each group remembers its last-used member
    // for the session"), so "M lands select.rect" is a COLD-START fact,
    // not an always-fact — mid-suite the marquee ring may legitimately
    // remember select.ellipse and M activates THAT. The pin's own claim
    // (the merge did not turn a letter into a six-member GROUP
    // shortcut, and each letter flips this one slot's face) is
    // unchanged and is what is asserted; the ring memory is simply put
    // in a known state first, through a NON-keyboard door, so the leg
    // measures the merge and not an ordering artifact. Cold-start
    // landing itself is pinned by d2SingleKeyLettersReachTheRuledTable,
    // which runs before any letter has cycled anything.
    const QPair<Qt::Key, const char *> letters[] = {
        {Qt::Key_M, "select.rect"},
        {Qt::Key_L, "select.lasso"},
        {Qt::Key_W, "select.wand"},
    };
    for (const auto &p : letters) {
        setCanvasTool(p.second);       // ring memory := this member
        setCanvasTool("brush.stroke"); // ...and leave the ring
        focusCanvas();
        QTest::keyClick(m_window, p.first);
        QTRY_COMPARE(canvasTool(), QLatin1String(p.second));
        QTRY_COMPARE(sel->property("faceTool").toMap()
                         .value(QStringLiteral("tool")).toString(),
                     QLatin1String(p.second));
        QVERIFY2(sel->property("checked").toBool(), p.second);
    }

    // 5. Full six-member face sweep (property path — syncFace follows
    // every selection door): the face crosses ALL former boundaries.
    for (int i = 0; i < memberCount; ++i) {
        setCanvasTool(census[i].wire);
        QCoreApplication::processEvents();
        QTRY_COMPARE(sel->property("faceTool").toMap()
                         .value(QStringLiteral("tool")).toString(),
                     QLatin1String(census[i].wire));
    }

    // 6. D5 width identity under the merged model, with the derivation
    // PRINTED (the item-17 record): the max face over the 34 members —
    // the member set is unchanged by the merge, so the width is too;
    // asserted mechanically, not assumed.
    qreal maxFace = 0;
    QString maxFaceName;
    for (QQuickItem *b : m_measure) {
        if (b->implicitWidth() > maxFace) {
            maxFace = b->implicitWidth();
            maxFaceName = accessibleName(b);
        }
    }
    QVERIFY(maxFace > 0);
    const qreal expected = maxFace
                           + m_palette->property("leftPadding").toReal()
                           + m_palette->property("rightPadding").toReal();
    QVERIFY2(qAbs(m_palette->implicitWidth() - expected) < 0.5,
             qPrintable(QStringLiteral("palette width %1 != max member "
                                       "face %2 + padding post-merge")
                            .arg(m_palette->implicitWidth())
                            .arg(expected)));
    qInfo() << "item-17 width derivation: max face" << maxFaceName
            << maxFace << "px over" << m_measure.size()
            << "members (member set unchanged by the merge); palette"
            << m_palette->implicitWidth() << "px";

    setCanvasTool("brush.stroke");
    focusCanvas();
}

void TstPalettePins::mm1SwatchSwapClickSwapsStateBothConsumers()
{
    // MENU-MINT FOLLOW-UP MM1 (suite header). The control, the minted
    // name, the shell-state exchange, both consumers, identity on
    // double-swap, and engine silence.
    m_window->requestActivate();
    QTRY_VERIFY(m_window->isActive());
    QQuickItem *swap = m_palette->findChild<QQuickItem *>(
        QStringLiteral("swatchSwapControl"));
    QVERIFY2(swap, "swatch swap control not found in the palette");
    QVERIFY(isToolButton(swap));
    QVERIFY(swap->isVisible());
    QCOMPARE(accessibleName(swap), catTr("ui.tool.swatch_swap"));
    QQuickItem *fg = m_palette->findChild<QQuickItem *>(
        QStringLiteral("swatchForegroundWell"));
    QQuickItem *bg = m_palette->findChild<QQuickItem *>(
        QStringLiteral("swatchBackgroundWell"));
    QVERIFY(fg && bg);

    // Adjacent to the pair, on viewport truth (the r3 idiom): the
    // control's scene rect sits fully inside the window AND the
    // palette bounds, on the wells' own footer row.
    const QRectF winRect(0, 0, m_window->width(), m_window->height());
    const QRectF palRect(m_palette->mapToScene(QPointF(0, 0)),
                         QSizeF(m_palette->width(), m_palette->height()));
    const QRectF swapRect(swap->mapToScene(QPointF(0, 0)),
                          QSizeF(swap->width(), swap->height()));
    QVERIFY2(winRect.contains(swapRect),
             "the swap control is not fully inside the window");
    QVERIFY2(palRect.contains(swapRect),
             "the swap control is outside the palette bounds");
    const QRectF bgRect(bg->mapToScene(QPointF(0, 0)),
                        QSizeF(bg->width(), bg->height()));
    QVERIFY2(swapRect.left() >= bgRect.right() - 0.5,
             "the swap control is not adjacent to (right of) the pair");
    QVERIFY2(qAbs(swapRect.center().y() - bgRect.center().y())
                 <= bgRect.height() / 2.0 + 0.5,
             "the swap control left the footer row");

    // Distinct, known state on both sides (r3's colors and tolerances).
    QVERIFY(m_canvas->setProperty("brushColor", QColor(Qt::red)));
    QVERIFY(m_palette->setProperty("swatchBackgroundColor",
                                   QColor(0x33, 0x66, 0xcc)));
    QCoreApplication::processEvents();
    const auto wellPixel = [&](QQuickItem *well) {
        const QImage grab = m_window->grabWindow();
        const qreal dpr = grab.width() / qreal(m_window->width());
        const QPoint mid = center(well);
        return grab.pixelColor(int(mid.x() * dpr), int(mid.y() * dpr));
    };
    const auto isRed = [](const QColor &c) {
        return c.red() > 200 && c.green() < 80 && c.blue() < 80;
    };
    const auto isBlue = [](const QColor &c) {
        return qAbs(c.red() - 0x33) < 24 && qAbs(c.green() - 0x66) < 24
               && qAbs(c.blue() - 0xcc) < 24;
    };
    QVERIFY2(isRed(wellPixel(fg)) && isBlue(wellPixel(bg)),
             "pre-swap pixel ground is wrong — the wells do not show "
             "the colors just set (r3 idiom broken; this pin would be "
             "vacuous)");

    // Engine-silence instruments (the R15 record): a shell-state swap
    // submits nothing and grows nothing.
    const int attemptsBefore = m_controller->submissionAttempts();
    const int historyBefore = m_controller->historyEntries().size();

    // ONE real click: state truth BOTH directions...
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      center(swap));
    QTRY_COMPARE(m_canvas->property("brushColor").value<QColor>(),
                 QColor(0x33, 0x66, 0xcc));
    QCOMPARE(m_palette->property("swatchBackgroundColor").value<QColor>(),
             QColor(Qt::red));
    // ...both consumers re-render: binding truth...
    QQuickItem *fgRect = fg->property("contentItem").value<QQuickItem *>();
    QQuickItem *bgRect2 = bg->property("contentItem").value<QQuickItem *>();
    QVERIFY(fgRect && bgRect2);
    QTRY_COMPARE(fgRect->property("color").value<QColor>(),
                 QColor(0x33, 0x66, 0xcc));
    QTRY_COMPARE(bgRect2->property("color").value<QColor>(),
                 QColor(Qt::red));
    // ...and PIXEL truth at the wells' own coordinates (grabWindow —
    // the rendered wells actually flipped).
    QVERIFY2(isBlue(wellPixel(fg)),
             "fg well pixels did not flip to the old bg color");
    QVERIFY2(isRed(wellPixel(bg)),
             "bg well pixels did not flip to the old fg color");
    // Engine silence (R15): no submission attempt, no history growth.
    QCOMPARE(m_controller->submissionAttempts(), attemptsBefore);
    QCOMPARE(m_controller->historyEntries().size(), historyBefore);

    // Double-swap = identity (the same three truths).
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      center(swap));
    QTRY_COMPARE(m_canvas->property("brushColor").value<QColor>(),
                 QColor(Qt::red));
    QCOMPARE(m_palette->property("swatchBackgroundColor").value<QColor>(),
             QColor(0x33, 0x66, 0xcc));
    QTRY_COMPARE(fgRect->property("color").value<QColor>(),
                 QColor(Qt::red));
    QTRY_COMPARE(bgRect2->property("color").value<QColor>(),
                 QColor(0x33, 0x66, 0xcc));
    QVERIFY2(isRed(wellPixel(fg)) && isBlue(wellPixel(bg)),
             "double-swap is not the identity at pixel truth");
    QCOMPARE(m_controller->submissionAttempts(), attemptsBefore);
    QCOMPARE(m_controller->historyEntries().size(), historyBefore);

    // Restore boot-default states (canvas_view.h's #141414 fg, white
    // bg — the r3 restore shape).
    QVERIFY(m_canvas->setProperty("brushColor", QColor(20, 20, 20)));
    QVERIFY(m_palette->setProperty("swatchBackgroundColor",
                                   QColor(Qt::white)));
}

void TstPalettePins::mm2ViewportScrollBarCarriesMintedName()
{
    // MENU-MINT FOLLOW-UP MM2 (suite header): the overflow ScrollBar's
    // minted accessible name. LINT INTERACTION record: the a11y source
    // lint (tst_a11y_tree_lint) sweeps a fixed interactive-control
    // regex that does NOT include ScrollBar — the bar was lint-green
    // unlabeled and stays lint-green labeled; THIS pin is the guard.
    m_window->requestActivate();
    QTRY_VERIFY(m_window->isActive());
    QQuickItem *bar = nullptr;
    int barsFound = 0;
    const auto kids = m_viewport->findChildren<QQuickItem *>();
    for (QQuickItem *it : kids) {
        if (it->inherits("QQuickScrollBar")) {
            bar = it;
            ++barsFound;
        }
    }
    QCOMPARE(barsFound, 1);
    QVERIFY2(bar, "the slots viewport has no ScrollBar child");
    // QML-context truth at any height (the name binding is
    // policy-independent).
    QCOMPARE(accessibleName(bar), catTr("a11y.palette.scrollbar"));

    // A11y-TREE truth at an overflow-ENGAGED height (post-17 the slots
    // fit at 800 px and the bar is policy-hidden; r4's 600 px probe
    // height engages it).
    // 1.0 final (corrigendum (l)): r4's probe height, derived the same
    // way (150 px below the fit boundary), not the old literal 600.
    const int fitsAt = m_window->height()
        - qFloor(m_viewport->height() - m_viewport->property("contentHeight").toReal());
    m_window->setHeight(fitsAt - 150);
    QTRY_VERIFY2(m_viewport->property("contentHeight").toReal()
                     > m_viewport->height(),
                 "150 px below the derived fit boundary the overflow must "
                 "engage (r4's probe height)");
    QTRY_VERIFY2(bar->isVisible(),
                 "overflow engaged but the ScrollBar is not visible "
                 "(AlwaysOn policy regression)");
    QAccessibleInterface *iface = QAccessible::queryAccessibleInterface(bar);
    if (!iface) {
        // The d5 a11y-walk precedent: the offscreen platform's tree
        // access is best-effort — recorded, never silently skipped.
        qWarning() << "offscreen platform returned no accessible "
                      "interface for the ScrollBar — tree-level name "
                      "not verifiable here (QML-context truth asserted "
                      "above; recorded)";
    } else {
        QCOMPARE(iface->text(QAccessible::Name),
                 catTr("a11y.palette.scrollbar"));
        QCOMPARE(iface->role(), QAccessible::ScrollBar);
    }

    // Restore: default height, viewport to the top.
    m_window->setHeight(800);
    QTRY_VERIFY(m_viewport->property("contentHeight").toReal()
                <= m_viewport->height() + 0.5);
    m_viewport->setProperty("contentY", 0);
}

void TstPalettePins::s25bWarpPressAnnouncesSessionPending()
{
    // ITEM 25b, the class-E half. transform.warp is the 18th armed tool
    // and is NOT in tier2Members (it arrived with 11c), so R2's sweep
    // does not cover it — and it is one of the two members whose honest
    // wording differs: warp opens an interactive canvas SESSION, so
    // "not wired to the canvas yet" would understate what is missing.
    // Pinned against the REAL catalog so a rewording reds here.
    focusCanvas();
    setCanvasTool("transform.warp");
    QCoreApplication::processEvents();
    quiesceAnnouncer();
    QSignalSpy announceSpy(m_announcer, &Announcer::announced);
    QVERIFY(announceSpy.isValid());
    const QString expectText = m_i18n->tr(
        QStringLiteral("ui.pending.session"),
        QVariantMap{{QStringLiteral("tool"), catTr("ui.tool.warp")}});
    QVERIFY2(!expectText.contains(QLatin1String("ui.pending.session")),
             "catalog key-echoed ui.pending.session — the assert below "
             "would be vacuous");
    const int historyBefore = m_controller->historyEntries().size();
    const int attemptsBefore = m_controller->submissionAttempts();
    const QPoint mid = m_canvas
                           ->mapToScene(QPointF(m_canvas->width() / 2.0,
                                                m_canvas->height() / 2.0))
                           .toPoint();
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier, mid);
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier, mid);
    QCOMPARE(m_controller->statusText(), expectText);
    QTRY_VERIFY2_WITH_TIMEOUT(
        [&]() {
            for (const auto &row : announceSpy)
                if (row.at(1).toString()
                    == QLatin1String("ui.pending.session"))
                    return true;
            return false;
        }(),
        "warp's press did not announce the class-E session wording", 1500);
    for (const auto &row : announceSpy)
        if (row.at(0).toString() == QLatin1String("tool_pending"))
            QCOMPARE(row.at(2).toString(), expectText);
    // Still ENGINE-SILENT: the announcement is shell-local.
    QCOMPARE(m_controller->submissionAttempts(), attemptsBefore);
    QCOMPARE(m_controller->historyEntries().size(), historyBefore);
    setCanvasTool("brush.stroke");
}

void TstPalettePins::s25bPendingSetIsDerivedFromPressChainNotAList()
{
    // ITEM 25b — THE DESIGN PIN. This is the test that protects the
    // derivation itself, and it is the reason the sweep does not carry
    // a roster of armed tool ids anywhere in the shell.
    //
    // The pending announcement fires from the FALL-THROUGH of
    // mousePressEvent's if-chain and from nowhere else, so membership
    // in that chain IS the wiring predicate. A tool that later gets
    // wired stops announcing automatically, with no second edit — which
    // is what lets this sweep and the retouch-wiring waves land in
    // EITHER order without double-announcing or lying.
    //
    // Replace the derivation with a hand list and this test is what
    // reds: it presses with tools that ARE wired and demands silence.
    // The three session tools are the sharp end of it. Their branches
    // are gated on a session flag, so with no session open their press
    // does nothing at all — a bare trailing `else`, or any list that
    // forgets them, would announce "not wired to the canvas yet" about
    // three tools that are wired and whose entry is simply menu-driven.
    // That is why the chain claims them by TOOL ID and gates only the
    // body.
    focusCanvas();
    const QPoint mid = m_canvas
                           ->mapToScene(QPointF(m_canvas->width() / 2.0,
                                                m_canvas->height() / 2.0))
                           .toPoint();
    // The three session tools need their session CLOSED for this pin to
    // mean anything, and closing it is not the default state: probed
    // first-hand, CanvasView::setTool BEGINS the session as a side
    // effect of arming any of them, so an armed session tool normally
    // has its session open and takes its branch's body. The state this
    // pin exists for is the one the setTool re-arm comment names —
    // "commit/cancel ends the session but leaves the tool selected" —
    // and Esc is the door to it (§39.2: Esc resolves the innermost
    // in-progress context and nothing else). Without the Esc the whole
    // session half of this test would be VACUOUS.
    struct WiredTool {
        const char *wire;
        bool sessionTool; // arm, then Esc the session closed
    };
    static const WiredTool wiredTools[] = {
        {"brush.stroke", false},   // the canonical wired tool
        {"select.rect", false},    // marquee gesture
        {"crop", false},           // drag-geometry gesture
        {"move", false},           // drag gesture
        {"select.wand", false},    // single-press submit (this doc has ONE
                                   // raster layer, so the engine ACCEPTS)
        {"transform.free", true},  // session-gated, session CLOSED
        {"transform.puppet", true},
        {"transform.perspective", true},
    };
    for (const auto &t : wiredTools) {
        const char *wire = t.wire;
        setCanvasTool(wire);
        QCoreApplication::processEvents();
        if (t.sessionTool) {
            focusCanvas();
            QTest::keyClick(m_window, Qt::Key_Escape);
            QCoreApplication::processEvents();
        }
        // Verified, never assumed — a still-open session would take the
        // branch body and hide the defect this pin hunts. (Puppet has no
        // queryable flag; cancelGesture closes all three together and Esc
        // is pinned by the escape contract, so the two that ARE queryable
        // stand for the state of all three.)
        QVERIFY2(!m_canvas->property("transformActive").toBool(),
                 "a transform session is still open — the "
                 "inactive-session case this pin exists for is not being "
                 "exercised");
        QVERIFY2(!m_canvas->property("perspectiveActive").toBool(),
                 "a perspective session is still open — same vacuity "
                 "hazard");
        quiesceAnnouncer();
        QSignalSpy announceSpy(m_announcer, &Announcer::announced);
        QVERIFY(announceSpy.isValid());
        QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier, mid);
        QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier, mid);
        QTest::qWait(120);
        for (const auto &row : announceSpy)
            QVERIFY2(row.at(0).toString() != QLatin1String("tool_pending"),
                     qPrintable(QStringLiteral(
                                    "%1 is WIRED and its press announced "
                                    "pending anyway — the pending set is "
                                    "no longer derived from the press "
                                    "chain's fall-through (a hand list "
                                    "has gone stale, or the session-gated "
                                    "branches lost their id claim)")
                                    .arg(QLatin1String(wire))));
        // The status line must not carry a pending note either — the
        // announcement and the status note are one decision, so a lie
        // on either surface is the same defect. The needle is the
        // template's ARG-FREE remainder, taken from the real catalog,
        // so a rewording moves the needle with it (no visible string
        // is written into this test).
        const QString pendingInvariant =
            m_i18n->tr(QStringLiteral("ui.status.tool_pending"),
                       QVariantMap{{QStringLiteral("tool"), QString()}})
                .trimmed();
        QVERIFY2(!pendingInvariant.isEmpty(),
                 "the pending template resolved to its arg alone — the "
                 "status assert below cannot discriminate");
        QVERIFY2(!m_controller->statusText().contains(pendingInvariant),
                 qPrintable(QStringLiteral("%1 is WIRED but the status "
                                           "line carries a pending note")
                                .arg(QLatin1String(wire))));
    }
    // SENSITIVITY GUARD (RULE 9): the same probe path, same coordinates,
    // with an ARMED-SILENT tool DOES announce — otherwise every assert
    // above would pass on a shell that announces nothing at all.
    setCanvasTool("fill");
    QCoreApplication::processEvents();
    quiesceAnnouncer();
    QSignalSpy armedSpy(m_announcer, &Announcer::announced);
    QVERIFY(armedSpy.isValid());
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier, mid);
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier, mid);
    QTRY_VERIFY2_WITH_TIMEOUT(
        [&]() {
            for (const auto &row : armedSpy)
                if (row.at(0).toString() == QLatin1String("tool_pending"))
                    return true;
            return false;
        }(),
        "sensitivity guard: an ARMED-SILENT press on the same probe path "
        "announced nothing — the wired-tool silence asserts are vacuous",
        1500);
    setCanvasTool("brush.stroke");
}

void TstPalettePins::s25bWandRefusalAnnouncesEngineArmPending()
{
    // ITEM 25b, the WAND MECHANICAL FIX — both halves, on ENGINE truth.
    //
    // select.wand is WIRED, but on a document with more than one raster
    // layer the engine's sampling_source answers a typed
    // error.command.not_implemented (headless carries no active-layer
    // state). Before this slice submitWand DISCARDED the result map and
    // emitted selectionSubmitted() unconditionally — the signal claimed
    // a submitted selection on a command the engine had refused — and
    // the only surface was the raw ui.status.rejected spec jargon.
    //
    // The refusal is a MILESTONE gap, so it announces pending with the
    // BARE reason id (it describes the milestone, not the tool, and
    // takes no args). Kept distinct from the not-wired template: this
    // tool's gesture is honest, its engine arm is what is missing.
    focusCanvas();
    setCanvasTool("select.wand");
    QCoreApplication::processEvents();
    const QPoint mid = m_canvas
                           ->mapToScene(QPointF(m_canvas->width() / 2.0,
                                                m_canvas->height() / 2.0))
                           .toPoint();
    QSignalSpy submitSpy(m_canvas, SIGNAL(selectionSubmitted()));
    QVERIFY(submitSpy.isValid());

    // (1) SINGLE-raster-layer document: the engine ACCEPTS. The live arm
    // must keep emitting, and must NOT announce pending — the
    // regression guard on the conditional emit.
    QCOMPARE(m_controller->layers().size(), 1);
    quiesceAnnouncer();
    QSignalSpy okSpy(m_announcer, &Announcer::announced);
    QVERIFY(okSpy.isValid());
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier, mid);
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier, mid);
    QTest::qWait(120);
    QCOMPARE(submitSpy.size(), 1);
    for (const auto &row : okSpy)
        QVERIFY2(row.at(0).toString() != QLatin1String("tool_pending"),
                 "an ACCEPTED wand click announced pending — the refusal "
                 "arm is firing on success");

    // (2) SECOND raster layer. 1.0 final D-8 (corrigendum (l)): the wand
    // always samples the composite, so the multi-layer click that used to
    // take the typed refusal arm now SERVES: one submission, no pending
    // announcement, one history row. (The conditional emit in submitWand
    // stays; the user can no longer reach its refusal arm.)
    QVERIFY(m_controller
                ->applyCommand(QStringLiteral("layer.add"), {},
                               QVariantMap{{QStringLiteral("kind"),
                                            QStringLiteral("raster")},
                                           {QStringLiteral("newLayerId"),
                                            m_controller->newUuid()}},
                               {})
                .value(QStringLiteral("ok"))
                .toBool());
    QCOMPARE(m_controller->layers().size(), 2);
    const int historyBefore = m_controller->historyEntries().size();
    submitSpy.clear();
    quiesceAnnouncer();
    QSignalSpy serveSpy(m_announcer, &Announcer::announced);
    QVERIFY(serveSpy.isValid());
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier, mid);
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier, mid);
    QTest::qWait(120);
    QVERIFY2(submitSpy.size() == 1,
             qPrintable(QStringLiteral("the multi-layer wand click did not serve (D-8); status: %1")
                            .arg(m_controller->statusText())));
    for (const auto &row : serveSpy)
        QVERIFY2(row.at(0).toString() != QLatin1String("tool_pending"),
                 "the multi-layer wand click announced pending (D-8 regressed)");
    QCOMPARE(m_controller->historyEntries().size(), historyBefore + 1);

    // Restore the fixture: no selection, one raster layer, brush armed.
    m_controller->applyCommand(QStringLiteral("selection.none"), {}, {}, {});
    const QVariantList rows = m_controller->layers();
    QVariantMap target;
    target.insert(QStringLiteral("layerId"),
                  rows.last().toMap()
                      .value(QStringLiteral("layerId")).toString());
    m_controller->applyCommand(QStringLiteral("layer.delete"), target, {}, {});
    QTRY_COMPARE(m_controller->layers().size(), 1);
    setCanvasTool("brush.stroke");
}

// Offscreen + the app's own Basic style; the hold interval mirrored
// from shell/src/main.cpp:82 AFTER capturing the platform default.
int main(int argc, char *argv[])
{
    if (qgetenv("QT_QPA_PLATFORM").isEmpty())
        qputenv("QT_QPA_PLATFORM", "offscreen");
    QQuickStyle::setStyle(QStringLiteral("Basic"));
    QGuiApplication app(argc, argv);
    installClickFocusParity(app); // mirrors shell/src/main.cpp (Windows only)
    g_platformDefaultHoldMs =
        QGuiApplication::styleHints()->mousePressAndHoldInterval();
    QGuiApplication::styleHints()->setMousePressAndHoldInterval(600);
    TstPalettePins tc;
    QTEST_SET_MAIN_SOURCE_PATH
    return QTest::qExec(&tc, argc, argv);
}
#include "tst_palette_pins.moc"
