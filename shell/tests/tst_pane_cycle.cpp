// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// v1.0.1 item 11d Phase 1 — THE PANE-CYCLE MATRIX (T1-T5; T6 lives in
// tst_options_bar). UNLOCKED by O-QA's recorded concurrence WITH the
// mandatory order correction (desk record a4510f0): the cycle is
// [OPTIONS, PALETTE, CANVAS, PANELS] — options FIRST, per the KS-3
// step-2 trace (keyboard-scripts.yaml:86) and global rule 5's
// options-before-palette (:25). Boots the REAL Main.qml offscreen (the
// item-7/14b harness) against a REAL engine document and drives REAL
// F6/Shift+F6/Tab/Esc key events.
//
//   T1  KS-3 step-2 EXACT landing (canvas → Shift+F6 → palette →
//       Shift+F6 → options bar, first control) + the forward full loop
//       [options → palette → canvas → panels → options].
//   T2  empty-options C2 skip BOTH directions — focus never lands on
//       (or in) the invisible container; the empty state is the COMMON
//       one (~17 tools, most optionless).
//   T3  Esc from the options bar returns focus to the canvas AND is
//       CONSUMED by the pane hop — a live canvas session must remain
//       Adjusting (the KS-3 step-5 shape). This shell's crop options
//       are dialog-housed (recorded in the 11d design entry), so the
//       live-session assertion runs on a PEN click-session, whose
//       options row exists; the canvas-Esc sensitivity control proves
//       the same session WAS cancellable by an Esc that reaches it.
//   T4  200-Tab trap-free (KS global rule 3), both with a populated
//       options row and with the empty-options state.
//   T5  D4-R: with focus in a modal dialog the chord is a NO-OP —
//       never a default-to-canvas steal.

#include "announcer.h"
#include "canvas_view.h"
#include "document_controller.h"
#include "i18n_catalog.h"
#include "units.h"
#include "icon_provider.h"
#include "layer_thumb_provider.h"

#include <QGuiApplication>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQmlExpression>
#include <QQuickItem>
#include <QQuickStyle>
#include <QQuickWindow>
#include <QSet>
#include <QtTest>
#include <QStyleHints>

using namespace appname::shell;

// ⛔ ONE GUARD, IN ONE PLACE, BECAUSE THREE CASES LEARNED THIS SEPARATELY AT 300 SECONDS EACH.
//
// Outside the offscreen platform a QQC2 Menu is a NATIVE, MODAL macOS menu, and this harness
// cannot drive one. MEASURED (O-UI C-1003/C-1004, cocoa):
//
//   evalJs("mainMenuBar.menuAt(0).open()")      -> enters QCocoaMenu::showPopup ->
//                                                  NSContextMenuImpl presentPopup and DOES NOT
//                                                  RETURN (200,674 ms, killed, stack sampled)
//   QMetaObject::invokeMethod(menu, "open")     -> never reports visible; the QTRY after it
//                                                  spins to QtTest's 300 s watchdog
//
// The watchdog kills the process with SIGABRT, so an unguarded case exits rc 134 and reads in
// any log as A CRASH ON THE SHIPPING PLATFORM. It is not a crash and it is not a product defect;
// it is a harness limit with a platform cause.
//
// ★ WHAT THIS GUARD IS NOT: a clearance. VOID IS NOT GREEN. Menu-driving behaviour on macOS has
//   NO automated coverage anywhere in this tree, by construction, and it is a HAND STEP in G-06's
//   20-step sitting. Skipping it here is how that fact stays visible instead of becoming a
//   300-second mystery for the next desk.
#define SKIP_IF_MENUS_ARE_NATIVE()                                                              \
    do {                                                                                        \
        if (QGuiApplication::platformName() != QLatin1String("offscreen"))                      \
            QSKIP("this case DRIVES a menu, and outside offscreen macOS menus are native and "  \
                  "modal: open() never reports visible and menuAt(0).open() does not return "   \
                  "(QCocoaMenu::showPopup -> NSContextMenuImpl presentPopup, 200,674 ms, stack " \
                  "sampled). Measured at O-UI C-1004. It is a hand step in the sitting.");      \
    } while (false)

class TstPaneCycle : public QObject {
    Q_OBJECT
private slots:
    void initTestCase();
    void cleanup();
    void t1Ks3StepTwoExactLandingAndForwardLoop();
    void t2EmptyOptionsSkippedBothDirections();
    void t3EscFromOptionsConsumedSessionSurvives();
    void t4TwoHundredTabsTrapFree();
    void t5ChordIsNoOpUnderModal();
    void panelsPaneIsOPERABLEByKeyboardOnceTheCycleLandsThere();
    void theLAYERSPanelOPERATIONSAreREACHABLEByTabOrG17IsRight();
    void G06_everyVISIBLEEnabledControlInTheWHOLEWindowIsReachable();
    void G06_theF10MenuDOORIsTheONLYRouteToTheMenusAndTHISIsWhetherItOpens();
    void G06_theSHIPPEDDialogsControlsAreREACHABLEByTab();
    void theLAYERMenuANDThePanelAreTheSAMEImplementation();
    void G06_everyENABLEDLayerMenuItemIsREACHABLEByArrowKeys();
    void theHelpFallbackAPPEARSOnlyWhenTheFolderRESOLVES();
    void aboutCarriesTheVersionTheHostSets();
    void f9MoveUpDownAreBoundedOnEveryDoor();
    void f10TheMovedLayerStaysActive();
    void r54MergeDownKeepsTheMergedLayerActive();

private:
    CatalogQml *m_i18n = nullptr;
    DocumentController *m_controller = nullptr;
    Announcer *m_announcer = nullptr;
    QQmlApplicationEngine *m_engine = nullptr;
    QQuickWindow *m_window = nullptr;
    QQuickItem *m_canvas = nullptr;
    QQuickItem *m_toolbar = nullptr;
    QQuickItem *m_palette = nullptr;
    QQuickItem *m_panels = nullptr;

    QVariant evalJs(const QString &code) const
    {
        QQmlExpression expr(qmlContext(m_window), m_window, code);
        bool undefined = false;
        const QVariant v = expr.evaluate(&undefined);
        if (expr.hasError())
            qWarning() << "evalJs failed:" << expr.error().toString();
        return undefined ? QVariant() : v;
    }
    QQuickItem *paneById(const char *id) const
    {
        return qobject_cast<QQuickItem *>(
            evalJs(QLatin1String(id)).value<QObject *>());
    }
    bool focusInside(QQuickItem *pane) const
    {
        for (QQuickItem *n = m_window->activeFocusItem(); n;
             n = n->parentItem())
            if (n == pane)
                return true;
        return false;
    }
    void chord(int step)
    {
        QTest::keyClick(m_window, Qt::Key_F6,
                        step > 0 ? Qt::NoModifier : Qt::ShiftModifier);
        QCoreApplication::processEvents();
    }
    void setTool(const char *wireId)
    {
        m_canvas->setProperty("tool", QLatin1String(wireId));
        QCoreApplication::processEvents();
    }
    void freshDoc()
    {
        QVERIFY(m_controller->newDocument(320, 200, QStringLiteral("white")));
        focusCanvas();
    }
    void focusCanvas()
    {
        m_canvas->forceActiveFocus();
        QTRY_VERIFY(m_canvas->hasActiveFocus());
    }
    QPoint docToView(double x, double y) const
    {
        const double zoom = m_canvas->property("zoom").toDouble();
        const double dw = m_controller->docWidth();
        const double dh = m_controller->docHeight();
        const double offX = (m_canvas->width() - dw * zoom) / 2.0;
        const double offY = (m_canvas->height() - dh * zoom) / 2.0;
        return m_canvas->mapToScene(QPointF(offX + x * zoom, offY + y * zoom))
            .toPoint();
    }
};

void TstPaneCycle::initTestCase()
{
    // Mirror shell/src/main.cpp's wiring (the tst_palette_pins harness).
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

    m_canvas = m_window->findChild<QQuickItem *>(QStringLiteral("canvasView"));
    QVERIFY2(m_canvas, "canvasView not found");
    m_toolbar = paneById("toolbarPane");
    m_palette = paneById("palettePane");
    m_panels = paneById("panelsPane");
    QVERIFY(m_toolbar);
    QVERIFY(m_palette);
    QVERIFY(m_panels);
}

void TstPaneCycle::cleanup()
{
    if (m_window)
        evalJs(QStringLiteral("newDialog.close()"));
    if (m_canvas) {
        // Drop any leftover session and restore the launch tool.
        QTest::keyClick(m_window, Qt::Key_Escape);
        m_canvas->setProperty("tool", QStringLiteral("brush.stroke"));
    }
}

void TstPaneCycle::t1Ks3StepTwoExactLandingAndForwardLoop()
{
    // Brush: an options row EXISTS (the bar's populated state — the
    // KS-3 trace needs a tool whose options are live; crop's numeric
    // row is dialog-housed this slice, recorded).
    freshDoc();
    setTool("brush.stroke");

    // KS-3 step 2, EXACT: canvas → Shift+F6 → PALETTE.
    chord(-1);
    QVERIFY2(focusInside(m_palette),
             "first Shift+F6 from canvas must land in the palette");
    QVERIFY(!focusInside(m_toolbar));

    // → Shift+F6 → OPTIONS BAR, first control (the corrected order's
    // whole point — both prior records landed palette here).
    chord(-1);
    QVERIFY2(focusInside(m_toolbar),
             "second Shift+F6 must land in the OPTIONS bar (KS-3 step 2)");
    QQuickItem *focused = m_window->activeFocusItem();
    QVERIFY2(focused && focused != m_toolbar,
             "focus parked on the options CONTAINER, not a control");
    QVERIFY2(focused->inherits("QQuickSlider"),
             qPrintable(QStringLiteral("first options control should be the "
                                       "brush-size slider, got ")
                        + QLatin1String(focused->metaObject()->className())));

    // Forward full loop from options: options → palette → canvas →
    // panels → options.
    chord(1);
    QVERIFY2(focusInside(m_palette), "options → F6 → palette");
    chord(1);
    QVERIFY2(m_canvas->hasActiveFocus(), "palette → F6 → canvas");
    chord(1);
    QVERIFY2(focusInside(m_panels), "canvas → F6 → panels");
    chord(1);
    QVERIFY2(focusInside(m_toolbar), "panels → F6 → options (loop closes)");
}

void TstPaneCycle::t2EmptyOptionsSkippedBothDirections()
{
    // Move: NO options row — the COMMON state. The C2 rule must skip
    // the empty pane in BOTH directions and never park focus on (or in)
    // the invisible container.
    freshDoc();
    setTool("move");

    // Backward from palette: palette → (skip empty options) → panels.
    m_palette->forceActiveFocus(Qt::TabFocusReason);
    QTRY_VERIFY(focusInside(m_palette));
    chord(-1);
    QVERIFY2(!focusInside(m_toolbar),
             "backward skip failed: focus landed in the empty options bar");
    QVERIFY2(focusInside(m_panels),
             "backward from palette must skip empty options to panels");

    // Forward from panels: panels → (skip empty options) → palette.
    chord(1);
    QVERIFY2(!focusInside(m_toolbar),
             "forward skip failed: focus landed in the empty options bar");
    QVERIFY2(focusInside(m_palette),
             "forward from panels must skip empty options to palette");

    // And the full forward cycle from canvas visits exactly
    // palette/canvas/panels while options stays skipped.
    focusCanvas();
    chord(1); // canvas → panels
    QVERIFY(focusInside(m_panels));
    chord(1); // panels → (skip options) → palette
    QVERIFY(focusInside(m_palette));
    QVERIFY(!focusInside(m_toolbar));
    chord(1); // palette → canvas
    QVERIFY(m_canvas->hasActiveFocus());
}

void TstPaneCycle::t3EscFromOptionsConsumedSessionSurvives()
{
    // The KS-3 step-5 shape: Esc from the options bar returns focus to
    // the canvas, the Esc is CONSUMED by the pane hop, and the live
    // session stays Adjusting. Session: a pen click-session (2 anchors
    // live, penActive) — pen's options row exists, and a canvas-scope
    // Esc would CANCEL this session (the sensitivity control below), so
    // survival discriminates consumption.
    freshDoc();
    setTool("vector.pen");
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(40, 40));
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(120, 80));
    // The session is live (gesture state is not a public property; the
    // commit-on-Enter door proves liveness at the end — here the
    // overlay state is asserted via the same Esc discriminator).

    // Hop into the options bar (backward: canvas → palette → options).
    chord(-1);
    chord(-1);
    QVERIFY2(focusInside(m_toolbar), "options landing failed (pen row)");

    // Esc: focus returns to canvas; the session SURVIVES.
    QTest::keyClick(m_window, Qt::Key_Escape);
    QTRY_VERIFY2(m_canvas->hasActiveFocus(),
                 "Esc from the options bar must return focus to the canvas");
    // Survival proof: Enter still commits the live pen path — the
    // journal gains vector.path_new/shape_add. A cancelled session
    // commits NOTHING (anchors dropped).
    const int before =
        m_controller->property("historyEntries").toList().size();
    QTest::keyClick(m_window, Qt::Key_Return);
    QTRY_VERIFY2(m_controller->property("historyEntries").toList().size()
                     == before + 1,
                 "the pane-hop Esc CANCELLED the live session (KS-3 step 5 "
                 "regression: Esc must be consumed by the hop)");

    // Sensitivity control: the SAME session shape, Esc delivered AT the
    // canvas — cancelled, Enter commits nothing.
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(40, 40));
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(120, 80));
    QTest::keyClick(m_window, Qt::Key_Escape); // canvas scope: cancels
    const int after =
        m_controller->property("historyEntries").toList().size();
    QTest::keyClick(m_window, Qt::Key_Return);
    QTest::qWait(100);
    QCOMPARE(m_controller->property("historyEntries").toList().size(), after);
}

void TstPaneCycle::t4TwoHundredTabsTrapFree()
{
    // KS global rule 3: 200 Tabs, the focus cycle must stay closed —
    // focus never null, never stuck, and the walk revisits (an open
    // cycle fails). Run with the EMPTY options bar (move) and with a
    // populated one (brush).
    const char *tools[] = {"move", "brush.stroke"};
    for (const char *tool : tools) {
        freshDoc();
        setTool(tool);
        focusCanvas();
        QSet<QQuickItem *> visited;
        QQuickItem *last = nullptr;
        int stuck = 0;
        for (int i = 0; i < 200; ++i) {
            QTest::keyClick(m_window, Qt::Key_Tab);
            QQuickItem *f = m_window->activeFocusItem();
            QVERIFY2(f, qPrintable(QStringLiteral(
                                       "Tab %1 (%2): focus went NULL")
                                       .arg(i)
                                       .arg(QLatin1String(tool))));
            if (f == last)
                ++stuck;
            else
                stuck = 0;
            // §5 completion minor: the bound must be REACHABLE inside
            // this 200-iteration loop (stuck maxes at 199, so a
            // "< 200" assert was vacuous — a fully frozen focus
            // passed). A genuinely cycling chain never repeats one
            // item 8 consecutive Tabs; a frozen one reds here fast.
            QVERIFY2(stuck < 8, "focus stuck on one item");
            // The empty options bar must contribute NOTHING to the tab
            // chain — its container must never hold focus.
            if (QLatin1String(tool) == QLatin1String("move"))
                QVERIFY2(f != m_toolbar,
                         "empty options container entered the tab chain");
            visited.insert(f);
            last = f;
        }
        QVERIFY2(visited.size() < 200,
                 "no focus cycle in 200 Tabs — open chain (trap)");
    }
}

void TstPaneCycle::t5ChordIsNoOpUnderModal()
{
    // D4-R: focus inside a modal dialog — F6/Shift+F6 are consumed
    // no-ops; no pane focus steal (never default-to-canvas).
    freshDoc();
    evalJs(QStringLiteral("newDialog.open()"));
    QTRY_VERIFY(evalJs(QStringLiteral("newDialog.visible")).toBool());
    QTRY_VERIFY(m_window->activeFocusItem() != nullptr);
    QQuickItem *inDialog = m_window->activeFocusItem();
    QVERIFY(!m_canvas->hasActiveFocus());

    chord(1);
    QCOMPARE(m_window->activeFocusItem(), inDialog);
    QVERIFY(!m_canvas->hasActiveFocus());
    QVERIFY(!focusInside(m_toolbar));
    QVERIFY(!focusInside(m_palette));
    QVERIFY(!focusInside(m_panels));

    chord(-1);
    QCOMPARE(m_window->activeFocusItem(), inDialog);
    QVERIFY(!m_canvas->hasActiveFocus());

    evalJs(QStringLiteral("newDialog.close()"));
    QTRY_VERIFY(!evalJs(QStringLiteral("newDialog.visible")).toBool());
}

// ADDED C-752 — THE CYCLE DELIVERS YOU TO A PANE; THIS ASKS WHETHER THERE IS
// ANYTHING TO DO WHEN YOU ARRIVE. T1-T5 prove focus REACHES the panels pane
// and never traps there. None of them asks whether the pane can be OPERATED,
// and the layers list is that panel's primary control.
//
// Found by generalising C-751: the licences list answered only onClicked, so
// arrows moved a highlight while the pane behind it never followed. A sweep
// of shell/qml for views with a clickable delegate and NO keyboard route
// (currentIndex / keyNavigation / Keys / isCurrentItem) returned exactly one
// besides the one already fixed -- this one -- with the pre-C-751
// LicensesDialog as the sweep's positive control and the post-C-751 version
// as its negative.
//
// IT MATTERS MORE HERE THAN IT DID THERE: LayersPanel declares
// Accessible.role Tree with selectable/selected rows, so it ADVERTISES a
// selectable tree to assistive technology. A screen-reader user navigates by
// keyboard, and there was no keyboard.
void TstPaneCycle::panelsPaneIsOPERABLEByKeyboardOnceTheCycleLandsThere()
{
    freshDoc();
    QVERIFY2(m_controller
                 ->applyCommand(QStringLiteral("layer.add"), {},
                                {{QStringLiteral("kind"), QStringLiteral("raster")},
                                 {QStringLiteral("newLayerId"), m_controller->newUuid()}},
                                QString())
                 .value(QStringLiteral("ok"))
                 .toBool(),
             "layer.add refused -- cannot test selection with one layer");
    QTRY_VERIFY2(m_controller->property("layers").toList().size() >= 2,
                 "multi-layer state not reached");

    QQuickItem *list = m_window->findChild<QQuickItem *>(QStringLiteral("layerList"));
    QVERIFY2(list, "layerList not found in Main.qml");

    list->forceActiveFocus();
    // A29: hasActiveFocus() is TRUE for ancestors, so it cannot say WHERE a
    // key lands. The window's own activeFocusItem names the one item that
    // receives it; walk up from there to prove it is inside the list.
    QQuickItem *focused = m_window->activeFocusItem();
    QVERIFY2(focused, "nothing holds keyboard focus");
    bool inList = false;
    for (QQuickItem *p = focused; p; p = p->parentItem())
        if (p == list) { inList = true; break; }
    QVERIFY2(inList, "the layer list cannot take keyboard focus at all");

    // PRECONDITION PINNED RATHER THAN ASSUMED. Until RC3 D-7 (operator WORD
    // 35) `layer.add` did NOT select the layer it creates (C-443), so the
    // Background at index 0 stayed active -- and this pin was written so that
    // a change to layer.add's selection would surface HERE, as itself. D-7 is
    // that change: the new layer (index 1, the TOP row under BottomToTop) is
    // now active, so the arrows are driven from the top.
    QCOMPARE(m_controller->property("activeLayerIndex").toInt(), 1);

    // DOWN is toward the LOWER index: down the stack, down the panel.
    QTest::keyClick(m_window, Qt::Key_Down);
    QTRY_COMPARE(m_controller->property("activeLayerIndex").toInt(), 0);

    // UP returns -- both directions, so this cannot pass on a list that only
    // ever runs one way.
    QTest::keyClick(m_window, Qt::Key_Up);
    QTRY_COMPARE(m_controller->property("activeLayerIndex").toInt(), 1);

    // AND IT CLAMPS RATHER THAN WRAPPING. A wrap at the floor would jump the
    // selection to the top of the stack, which on a layer panel is a
    // destructive surprise: the next paint stroke lands somewhere else.
    QTest::keyClick(m_window, Qt::Key_Down);
    QTRY_COMPARE(m_controller->property("activeLayerIndex").toInt(), 0);
    QTest::keyClick(m_window, Qt::Key_Down);
    QCoreApplication::processEvents();
    QCOMPARE(m_controller->property("activeLayerIndex").toInt(), 0);
}

// ADDED C-768 — CLOSING C-747's STATED LIMIT. That entry pinned the Help >
// Open Licenses Folder item's DECLARATION (catalogued caption, typeof guard,
// visible gate, handler target) and said plainly: "this pins the DECLARATION,
// not the render, and it does not DRIVE the item." So the item WORD 3 called
// "the fallback either way" -- and which C-749 measured as the PRIMARY route
// to 62% of the licence texts -- had never been observed to exist at runtime.
//
// NOT DRIVEN HERE EITHER, and for the same reason: onTriggered calls
// Qt.openUrlExternally, which would open a real Finder window on whoever runs
// the suite. What IS newly measured is the state a user would see: present,
// hidden when nothing resolves, shown and correctly captioned when something
// does, with the folder it points at actually existing.
void TstPaneCycle::theHelpFallbackAPPEARSOnlyWhenTheFolderRESOLVES()
{
    // PRE-EXISTING ON main AND UNGUARDED THERE: this case drives a Menu too, so main's own
    // tst_pane_cycle cannot finish on cocoa today (measured 300,010 ms then SIGABRT). That is
    // also the likeliest reason nobody noticed main() was missing the focus-policy line: the
    // suite had never completed a cocoa run for anyone to read.
    SKIP_IF_MENUS_ARE_NATIVE();
    QObject *item = m_window->findChild<QObject *>(QStringLiteral("helpOpenLicensesItem"));
    QVERIFY2(item, "helpOpenLicensesItem is not in the booted Main.qml at all");

    // ★ THE MENU MUST BE OPEN TO READ `visible` AT ALL, and finding that out
    // is what saved this test from being decoration. A MenuItem's `visible`
    // as seen from C++ is its EFFECTIVE visibility: Qt propagates a closed
    // popup's state to its content items, so a closed menu reports false for
    // every item regardless of its declared gate. Measured: dir was the
    // staged path and text was the catalogued caption while visible stayed
    // false, with parentVisible false beside it.
    //
    // MY FIRST VERSION READ `visible` WITH THE MENU CLOSED AND "PASSED"
    // DIRECTION ONE — vacuously. It would have passed with the gate
    // hardcoded `visible: true`. Only direction two failing exposed it,
    // which is the A29 shape again: a property that means something other
    // than what the assertion assumes.
    auto *menu = qobject_cast<QObject *>(item->parent());
    QVERIFY2(menu, "the fallback item has no parent Menu");
    auto openMenu = [&]() {
        QMetaObject::invokeMethod(menu, "open");
        for (int i = 0; i < 8; ++i)
            QCoreApplication::processEvents(QEventLoop::AllEvents, 20);
    };
    auto closeMenu = [&]() {
        QMetaObject::invokeMethod(menu, "close");
        for (int i = 0; i < 8; ++i)
            QCoreApplication::processEvents(QEventLoop::AllEvents, 20);
    };

    // DIRECTION ONE: this harness never sets AppLicensesDir, so the typeof
    // guard resolves to "" and the item must be HIDDEN. A fallback that
    // advertises a folder it cannot open is worse than an absent one.
    openMenu();
    QTRY_VERIFY2(menu->property("visible").toBool(), "the Help menu did not open");
    QCOMPARE(item->property("dir").toString(), QString());
    QVERIFY2(!item->property("visible").toBool(),
             "the fallback is shown while no licences folder resolves");
    closeMenu();

    // DIRECTION TWO: give it a real folder and it must appear. Without this
    // arm the test passes on an item that is hidden unconditionally -- the
    // one-directional guard O-WATCH's §5c names, and which this desk shipped
    // once already (C-732).
    QTemporaryDir staged;
    QVERIFY(staged.isValid());
    QFile probe(QDir(staged.path()).filePath(QStringLiteral("MIT.txt")));
    QVERIFY(probe.open(QIODevice::WriteOnly));
    probe.write("MIT\n");
    probe.close();

    m_engine->rootContext()->setContextProperty(QStringLiteral("AppLicensesDir"),
                                                staged.path());
    QTRY_COMPARE(item->property("dir").toString(), staged.path());
    openMenu();
    QTRY_VERIFY2(menu->property("visible").toBool(), "the Help menu did not re-open");
    QTRY_VERIFY2(item->property("visible").toBool(),
                 "the folder resolves and the fallback still does not appear");

    // AND THE CAPTION IS THE CATALOGUE'S, not a literal. Echo-proofed: a
    // key-echo is non-empty and truthy (C10), so "it has text" proves
    // nothing.
    const QString want = m_i18n->catalog().tr(QStringLiteral("ui.menu.help.open_licenses"));
    QVERIFY2(want != QStringLiteral("ui.menu.help.open_licenses"),
             "the catalog key-echoed the fallback's caption");
    QCOMPARE(item->property("text").toString(), want);

    // THE FOLDER IT POINTS AT EXISTS AND HAS FILES -- the property the
    // source-text pin at C-747 could not reach.
    const QDir opened(item->property("dir").toString());
    QVERIFY2(opened.exists(), "the fallback points at a folder that is not there");
    QVERIFY2(!opened.entryList(QDir::Files).isEmpty(),
             "the fallback points at an empty folder");

    closeMenu();

    // Leave the world as found (C-730): a context property set here would
    // otherwise follow every later slot in this suite, and a menu left open
    // is the same hazard one level up.
    m_engine->rootContext()->setContextProperty(QStringLiteral("AppLicensesDir"),
                                                QVariant());
}


// RC3 (operator WORD 36 (a), sitting S-03): About shows the build's version
// beside the build commit. main.cpp sets the AppVersion context property from
// APPNAME_VERSION (Cargo.toml, via CMake); Main.qml binds it into the dialog
// with the AppBuildCommit guard. Both directions, as the Help fallback above:
// unset renders nothing (the dialog's honest-empty default), set flows in.
void TstPaneCycle::aboutCarriesTheVersionTheHostSets()
{
    QObject *about = nullptr;
    for (QObject *o : m_window->findChildren<QObject *>())
        if (QLatin1String(o->metaObject()->className()).startsWith(QLatin1String("AboutDialog")))
            about = o;
    QVERIFY2(about, "no AboutDialog in Main.qml");
    QCOMPARE(about->property("appVersion").toString(), QString());

    m_engine->rootContext()->setContextProperty(QStringLiteral("AppVersion"),
                                                QStringLiteral("9.8.7"));
    QTRY_COMPARE(about->property("appVersion").toString(), QStringLiteral("9.8.7"));

    // Leave the world as found (C-730).
    m_engine->rootContext()->setContextProperty(QStringLiteral("AppVersion"), QVariant());
    QTRY_COMPARE(about->property("appVersion").toString(), QString());
}
// Offscreen + the app's own Basic style, forced before the app exists.
// ─────────────────────────────────────────────────────────────────────────
// G-17 / Q-D72, MEASURED ON MY OWN SURFACE (O-UI, C-856)
//
// Q-D72 (MAJOR, 09-13) reports q-gesture's G-17 of 09-07: "a keyboard-only
// user cannot manage layers", on three doors — the Tab ring never visits
// them, five layer ops give 0 Shortcut hits, and there is no Layer menu —
// with "LayersPanel.qml carries zero Shortcut, Keys.on*, activeFocusOnTab or
// focusPolicy".
//
// ⛔ THAT LAST CLAUSE IS FALSE ON THIS TREE AND I CHECKED BEFORE ACTING:
//    LayersPanel.qml:197 carries `activeFocusOnTab: true` and :209-210 carry
//    Keys.onUpPressed / Keys.onDownPressed. `Shortcut` and `focusPolicy` are
//    genuinely zero. panelsPaneIsOPERABLEByKeyboardOnceTheCycleLandsThere
//    already pins the list: it takes focus and both arrows move selection.
//
// So the OPEN question is narrow and is the one that decides whether Q-D72
// is a MAJOR before Friday's RC1: can the Tab ring reach the OPERATION
// controls — add, add group, add adjustment, fill, and the row buttons —
// or only the list? A user who can select a layer and cannot act on it is
// exactly as stuck as one who can do neither.
//
// ⛔⛔⛔ READ THIS BEFORE TREATING A GREEN HERE AS A CLEARANCE (C-856).
// THIS TEST RUNS OFFSCREEN, WHERE tabFocusBehavior IS 0xff
// (TabFocusAllControls). THE SHIPPED APP RUNS ON COCOA, WHERE IT IS 0x3
// (TabFocusTextControls|TabFocusListControls) — BUTTONS ARE NOT ON THE TAB
// RING AT ALL — because shell/src/main.cpp NEVER SETS THE HINT (it sets
// setMousePressAndHoldInterval at :89 and stops there), so the app inherits
// the platform default, and macOS's default is Full Keyboard Access OFF.
//
// MEASURED, same binary, same tree, same two-layer document:
//     offscreen  tabFocusBehavior=0xff  focus chain from the canvas: 41 items
//     cocoa      tabFocusBehavior=0x3   focus chain from the canvas:  0 items
//
// So a green below means "reachable IF the app opts in", NOT "reachable by a
// user on Friday's RC1". G-17 is right on the shipping platform. Filed as
// Q-D72's addendum with the one-line candidate fix; the decision is not mine
// to take unilaterally, and reddening a shared gate on the eve of RC1 less so.
//
// The instrument is Qt's own traversal, and reachability is the control OR
// ANY DESCENDANT of it: a QQC2 control may delegate focus to its contentItem
// (C-855 — reading control identity alone produced a FALSE accusation
// against the options bar one tick ago, and that is not repeated here).
void TstPaneCycle::theLAYERSPanelOPERATIONSAreREACHABLEByTabOrG17IsRight()
{
    freshDoc();
    QVERIFY2(m_panels, "panelsPane not resolved");

    // TWO LAYERS, NOT ONE: delete / merge-down / duplicate are disabled on a
    // single-layer document, and this sweep only examines ENABLED controls —
    // so a one-layer fixture would clear the very operations G-17 is about by
    // never looking at them. Measured: 9 controls with one layer, and the pin
    // below is set from the two-layer count.
    QVERIFY2(m_controller
                 ->applyCommand(QStringLiteral("layer.add"), {},
                                {{QStringLiteral("kind"), QStringLiteral("raster")},
                                 {QStringLiteral("newLayerId"), m_controller->newUuid()}},
                                QString())
                 .value(QStringLiteral("ok"))
                 .toBool(),
             "layer.add refused -- the sweep would run on a one-layer document");
    QTRY_VERIFY(m_controller->property("layers").toList().size() >= 2);

    QSet<QQuickItem *> chain;
    {
        QQuickItem *start = m_canvas;
        QQuickItem *cur = start;
        for (int i = 0; i < 4000; ++i) {
            cur = cur->nextItemInFocusChain(true);
            if (!cur || cur == start || chain.contains(cur))
                break;
            chain << cur;
        }
    }
    QVERIFY2(chain.size() > 5,
             qPrintable(QStringLiteral("the focus chain walked %1 items — the walk is broken and "
                                       "nothing below means anything — platform=%2 tabFocusBehavior=0x%3 "
                                       "activeFocusItem=%4")
                            .arg(chain.size())
                            .arg(QGuiApplication::platformName())
                            .arg(int(QGuiApplication::styleHints()->tabFocusBehavior()), 0, 16)
                            .arg(m_window->activeFocusItem()
                                     ? QLatin1String(m_window->activeFocusItem()->metaObject()->className())
                                     : QLatin1String("<none>"))));

    std::function<bool(QQuickItem *)> canReach = [&](QQuickItem *n) -> bool {
        if (chain.contains(n))
            return true;
        for (QQuickItem *k : n->childItems())
            if (canReach(k))
                return true;
        return false;
    };

    QStringList unreachable;
    int swept = 0;
    std::function<void(QQuickItem *)> walk = [&](QQuickItem *node) {
        for (QQuickItem *c : node->childItems()) {
            const bool interactive = c->inherits("QQuickAbstractButton")
                                  || c->inherits("QQuickComboBox");
            if (interactive && c->isVisible() && c->isEnabled()) {
                ++swept;
                if (!canReach(c)) {
                    const QString name = c->objectName().isEmpty()
                        ? QStringLiteral("<%1 \"%2\">").arg(
                              QLatin1String(c->metaObject()->className()),
                              c->property("text").toString())
                        : c->objectName();
                    unreachable << name;
                }
            }
            walk(c);
        }
    };
    walk(m_panels);

    // NOT VACUOUS: the panels pane carries layer add / add group / add
    // adjustment / fill and the per-row buttons, so a sweep that found
    // almost nothing examined almost nothing.
    QVERIFY2(swept >= 13,
             qPrintable(QStringLiteral("swept only %1 visible enabled controls in the panels pane — "
                                       "the sweep is not a sweep").arg(swept)));

    QVERIFY2(unreachable.isEmpty(),
             qPrintable(QStringLiteral("G-17 IS RIGHT ON THIS TREE: %1 of %2 visible, enabled panel "
                                       "controls are unreachable by Tab — %3")
                            .arg(unreachable.size()).arg(swept)
                            .arg(unreachable.join(QStringLiteral(", ")))));
}

// ─────────────────────────────────────────────────────────────────────────
// G-06's THIRD CLAUSE, WHOLE-WINDOW (O-UI, C-859)
//
// ROADMAP row G-06 (M5, due 09-21): "Accessibility sweep: VoiceOver pass
// through the 20-step sitting on macOS; Narrator smoke on Windows; EVERY
// CONTROL REACHABLE BY KEYBOARD". The VoiceOver half needs a person and the
// Narrator half is re-based to the Windows follow-on (WORD 3 (2)). The third
// clause is machine work and it is mine, and until now I have measured it on
// TWO PANES -- the options bar (C-855) and the panels pane (C-856). Two panes
// is not "every control".
//
// ⛔ SAME HINT CAVEAT AS EVERY OTHER REACHABILITY ASSERTION HERE: this runs
//    offscreen at tabFocusBehavior 0xff, the shipped app defaults to 0x3, and
//    the assertion at the top of this test says so rather than leaving it to a
//    comment. A green means "reachable IF the app opts in" (Q-D72).
void TstPaneCycle::G06_everyVISIBLEEnabledControlInTheWHOLEWindowIsReachable()
{
    QVERIFY2(int(QGuiApplication::styleHints()->tabFocusBehavior())
                 == int(Qt::TabFocusAllControls),
             qPrintable(QStringLiteral("tabFocusBehavior is 0x%1, not 0xff — this sweep's reading "
                                       "of \"reachable\" is not the one it was written for (Q-D72)")
                            .arg(int(QGuiApplication::styleHints()->tabFocusBehavior()), 0, 16)));
    freshDoc();
    QVERIFY2(m_controller
                 ->applyCommand(QStringLiteral("layer.add"), {},
                                {{QStringLiteral("kind"), QStringLiteral("raster")},
                                 {QStringLiteral("newLayerId"), m_controller->newUuid()}},
                                QString())
                 .value(QStringLiteral("ok")).toBool(),
             "layer.add refused -- the sweep would run on a one-layer document");
    QTRY_VERIFY(m_controller->property("layers").toList().size() >= 2);

    QQuickItem *root = m_window->contentItem();
    QVERIFY(root);

    // ⛔ THE CHAIN IS RECOMPUTED PER STATE, AND THE FIRST VERSION DID NOT.
    // It walked once and then changed the tool five times. The options bar
    // REBUILDS its delegates on every tool change, so every control created
    // after the walk was missing from a set taken before it existed, and the
    // sweep reported EIGHT unreachable controls on a byte-identical tree --
    // the wand SpinBox, both Sample All Layers checkboxes, the sliders, the
    // puppet Switch. tst_options_bar says those ARE reachable, and a loop that
    // disagrees with its own control is the loop being wrong (G-10). Caught
    // only because the other instrument existed to disagree.
    QSet<QQuickItem *> chain;
    const auto walkChain = [&]() {
        chain.clear();
        QQuickItem *start = m_canvas;
        QQuickItem *cur = start;
        for (int i = 0; i < 8000; ++i) {
            cur = cur->nextItemInFocusChain(true);
            if (!cur || cur == start || chain.contains(cur))
                break;
            chain << cur;
        }
    };
    walkChain();
    QVERIFY2(chain.size() > 10,
             qPrintable(QStringLiteral("the focus chain walked %1 items — the walk is broken and "
                                       "nothing below means anything").arg(chain.size())));

    std::function<bool(QQuickItem *)> canReach = [&](QQuickItem *n) -> bool {
        if (chain.contains(n))
            return true;
        for (QQuickItem *k : n->childItems())
            if (canReach(k))
                return true;
        return false;
    };

    // Every interactive CONTROL class the shell builds. A list delegate is not
    // in here on purpose: LayersPanel is ONE tab stop with arrows inside, which
    // is the tree/listbox convention its own comment argues for, so demanding a
    // stop per row would assert the opposite of the design.
    const char *KINDS[] = { "QQuickAbstractButton", "QQuickComboBox", "QQuickSlider",
                            "QQuickSpinBox", "QQuickTextField", "QQuickTextArea" };
    // ⛔ MenuBarItem IS EXCLUDED, AND THE EXCLUSION IS PAID FOR BELOW.
    // The first run of this sweep reported exactly six unreachable controls and
    // all six were MenuBarItems (File, Edit, Select, Image, Filter, Help). That
    // is NOT a defect: a menu bar is not a tab stop by convention on either
    // platform, and Main.qml:639-690 carries a deliberate, review-probed door --
    // F10 (and Ctrl+F2, §49.1's macOS MENU chord) opens menu 0 and ARMS the bar,
    // then Left/Right between menus, Down/Up within, Return to activate. The
    // comment there records that "focus arms native navigation" was DISPROVED at
    // runtime and worked around, so the door is measured, not assumed.
    //
    // ★ BUT AN EXCLUSION WITHOUT A REPLACEMENT CHECK IS A HOLE (C-858's rule, my
    //   own, applied to myself). Grepping the suites: the only Key_F10 in the
    //   tree is SHIFT+F10 in tst_canvas_context_menu -- the CONTEXT menu, a
    //   different door -- so the sole keyboard route to the entire menu system
    //   had NO automated coverage at all. The arm at the end of this test is
    //   that coverage, and it is why these six may be skipped here.
    const char *EXCLUDED = "QQuickMenuBarItem";
    QStringList bad;
    int swept = 0;
    std::function<void(QQuickItem *)> walk = [&](QQuickItem *node) {
        for (QQuickItem *c : node->childItems()) {
            bool interactive = false;
            for (const char *k : KINDS)
                if (c->inherits(k)) { interactive = true; break; }
            if (c->inherits(EXCLUDED))
                interactive = false;
            if (interactive && c->isVisible() && c->isEnabled()) {
                ++swept;
                if (!canReach(c)) {
                    QStringList path;
                    for (QQuickItem *n = c; n && path.size() < 4; n = n->parentItem())
                        path.prepend(n->objectName().isEmpty()
                                         ? QLatin1String(n->metaObject()->className())
                                         : n->objectName());
                    bad << QStringLiteral("%1 [%2]")
                               .arg(path.join(QLatin1Char('/')), c->property("text").toString());
                }
            }
            walk(c);
        }
    };
    // ⛔ ONE STATE IS NOT "EVERY CONTROL", AND MY FIRST VERSION SWEPT ONE.
    // The options bar rebuilds per tool, so a checkbox that exists only under
    // select.wand is INVISIBLE to a sweep taken under whatever tool the fixture
    // happened to leave selected -- and the must-fire proved it: with the
    // options-bar CheckBox delegate forced activeFocusOnTab:false, the sweep
    // still PASSED, because it had never seen one. Third population hole of my
    // own in one day, and the only reason it was caught is that a must-fire is
    // not optional. Sweeping the tool states that actually build the varied
    // control kinds is what makes the name on this test true.
    for (const char *tool : { "brush.stroke", "select.wand", "heal.stroke",
                              "transform.puppet", "vector.shape_add" }) {
        setTool(tool);
        QCoreApplication::processEvents();
        walkChain();                 // the bar was just rebuilt; so must the chain be
        QVERIFY2(chain.size() > 10,
                 qPrintable(QStringLiteral("%1: the focus chain walked %2 items")
                                .arg(QLatin1String(tool)).arg(chain.size())));
        walk(root);
    }

    // NOT VACUOUS: the shell shows a tool palette, an options bar and three
    // panels at once, so a whole-window sweep that found a handful examined
    // almost nothing.
    // ⛔ RAISED 100 -> 186 ON 2026-09-14 (C-893). The comment here used to say
    //   the floor was "the MEASURED panels-pane count (13) plus a margin ...
    //   not a round number". That describes how 100 was CONSTRUCTED, not what
    //   the sweep MEASURES -- and 100 is the roundest number in this file. The
    //   sweep actually finds 186, so EIGHTY-SIX controls (46%) could leave the
    //   window with this guard green. Derived, not remembered: the test printed
    //   "swept only 186 visible enabled controls" with the floor temporarily at
    //   999999. Shape per Q-D61 (a): hand-set to the derived count, raised in
    //   its own commit, never lowered without a word.
    // 1.0 final (corrigendum (l), WORD 41 (4) is the word): the removal set
    // took 11 controls out of the five tool states (palette slots, the
    // Layers panel's Grp+/Ungroup, the wand's Sample All box). The sweep
    // measured 175 on the removal tree; the floor follows the measurement.
    // 175 -> 170 (O-WATCH F-8): the Layers panel's Grp+ button is removed,
    // one control in each of the five tool states; measured 170.
    // 170 -> 169 (S-08): heal's Sample All Layers box is removed; heal is
    // one of the five tool states; measured 169.
    QVERIFY2(swept >= 169,
             qPrintable(QStringLiteral("swept only %1 visible enabled controls across five tool "
                                       "states — the sweep is not a sweep (one state alone "
                                       "measured 38)").arg(swept)));

    QVERIFY2(bad.isEmpty(),
             qPrintable(QStringLiteral("G-06 clause 3: %1 of %2 visible, enabled controls are "
                                       "unreachable by Tab — %3")
                            .arg(bad.size()).arg(swept).arg(bad.join(QStringLiteral(" · ")))));

    // ── THE PRICE OF THE MenuBarItem EXCLUSION, AND EXACTLY WHAT IT BUYS ──
    // A menu bar is not a tab stop, so the right question is whether its OWN
    // door works. MEASURED THIS TICK, three arms, offscreen:
    //
    //   F10 (the product's chord)      -> no menu opens
    //   root.toggleMenuAccess()        -> no menu opens
    //   mainMenuBar.menuAt(0).open()   -> OPENS, and anyMenuOpen() sees it
    //
    // The third arm is what makes this readable: menus DO open here and ARE
    // observable, so the first two do not fail for want of observation. They
    // fail in the ARMING path -- Main.qml:679-687 records that a programmatic
    // open does not arm menu-to-menu navigation and that AppKeys.armMenuBar
    // injects a GENUINE Space to arm it, and genuine key injection is what an
    // offscreen, never-exposed window does not deliver.
    //
    // ⛔ SO THIS IS A PLATFORM LIMIT, NOT A PRODUCT VERDICT — and it is NOT a
    //    clearance either. VOID IS NOT GREEN: the F10 door's real sequence has
    //    no automated coverage anywhere in this tree (the only Key_F10 in the
    //    suites is SHIFT+F10, the context menu, a different door), and the sole
    //    evidence it works is a reviewer's hand probe recorded in a comment.
    //    Filed under G-06. What IS assertable offscreen is asserted here, so the
    //    exclusion above is not free.
    //
    // ★★ THE COCOA RUN THIS COMMENT ASKED FOR HAS NOW HAPPENED (O-UI C-1003/C-1004),
    //    AND IT CLOSED THE QUESTION THE OTHER WAY: on cocoa `menuAt(0).open()`
    //    enters QCocoaMenu::showPopup -> NSContextMenuImpl presentPopup and DOES
    //    NOT RETURN (200,674 ms, killed, stack sampled). macOS menus are native and
    //    modal, so no harness drives them. **The door is a HAND STEP in the 20-step
    //    sitting; it is not awaiting a cleverer machine.**
    //
    // ⛔ AND THAT HANG IS WHY THIS BLOCK IS FENCED. Unfenced, it took THIS WHOLE CASE
    //    past QtTest's 300 s watchdog on cocoa (measured: 300,003 ms, SIGABRT) — which
    //    reads as a CRASH ON THE SHIPPING PLATFORM in any log anyone skims. Everything
    //    above this point runs happily on cocoa and is the reachability evidence G-06
    //    actually wants; only the door probe cannot. Fence the part that cannot, keep
    //    the part that can.
    if (QGuiApplication::platformName() != QLatin1String("offscreen")) {
        qInfo("menu-door probe skipped on %s: macOS menus are native and modal "
              "(C-1004). The sweep above ran; the door is a hand step.",
              qPrintable(QGuiApplication::platformName()));
        return;
    }
    const int menuCount = evalJs(QStringLiteral("mainMenuBar.count")).toInt();
    QVERIFY2(menuCount > 0,
             qPrintable(QStringLiteral("mainMenuBar.count is %1 — the exclusion above now hides "
                                       "six controls behind a bar this test cannot even locate")
                            .arg(menuCount)));
    QVERIFY2(!evalJs(QStringLiteral("root.anyMenuOpen()")).toBool(),
             "a menu was already open before the door was touched — precondition not met");
    evalJs(QStringLiteral("mainMenuBar.menuAt(0).open()"));
    QTRY_VERIFY2(evalJs(QStringLiteral("root.anyMenuOpen()")).toBool(),
                 "menuAt(0).open() did not open a menu — the menus themselves are broken, which "
                 "is a product defect and not a platform limit");
    evalJs(QStringLiteral("root.closeAllMenus()"));
    QTRY_VERIFY2(!evalJs(QStringLiteral("root.anyMenuOpen()")).toBool(),
                 "closeAllMenus() left a menu open — Review D2's no-zombie-open property has "
                 "regressed, and that IS the product");
}

// ─────────────────────────────────────────────────────────────────────────
// G-06's OWED HALF: THE F10 DOOR (O-UI, C-860)
//
// C-859 found it and could not answer it. The menu bar is deliberately not a
// tab stop, so the whole-window Tab sweep excludes MenuBarItems -- and the
// door that replaces Tab for the menus, F10 / Ctrl+F2 (Main.qml:692-700,
// §49.1's macOS MENU chord), has NO automated coverage anywhere in this tree.
// The only Key_F10 in the suites is SHIFT+F10, the context menu, a different
// door. Its sole evidence is a reviewer's hand probe recorded in a comment.
//
// ★ WHY THIS IS ITS OWN TEST AND NOT A CLAUSE OF THE SWEEP: the sweep asserts
//   tabFocusBehavior == 0xff and would abort under cocoa before ever reaching
//   the menus. The menu door is F10, not Tab, so the hint is irrelevant to it
//   -- and cocoa is the platform where this question actually needs answering.
//
// ⛔ AND IT SKIPS RATHER THAN REDS WHERE THE ENVIRONMENT CANNOT ANSWER. Under
//   offscreen the arming path is unavailable: measured C-859, F10 and
//   toggleMenuAccess() both open nothing while a programmatic menuAt(0).open()
//   DOES open and IS observable, because armMenuBar injects a GENUINE Space and
//   a never-exposed window does not deliver one. VOID IS NOT RED -- a verdict
//   that describes the harness is not a verdict about the code. The skip is
//   loud and says what to run instead.
void TstPaneCycle::G06_theF10MenuDOORIsTheONLYRouteToTheMenusAndTHISIsWhetherItOpens()
{
    freshDoc();
    const QString platform = QGuiApplication::platformName();

    // ⛔⛔ NOTHING BELOW MAY RUN OUTSIDE offscreen, AND THIS GUARD IS THE WHOLE
    // ANSWER TO THE QUESTION C-859 LEFT OPEN. C-859 said "run this under cocoa,
    // which is where the question is real". I did. It HUNG FOR 200,674 ms and
    // died on SIGTERM, and a `sample` of the stuck process named the frame:
    //
    //   evalJs("mainMenuBar.menuAt(0).open()")
    //     -> QQmlExpressionPrivate::value -> QQuickPopup::qt_metacall
    //        -> QCocoaMenu::showPopup -> +[NSContextMenuImpl presentPopup:...]
    //
    // On macOS these menus are NATIVE and the popup is presented MODALLY: the
    // call does not return until a human dismisses it. That is true of the
    // programmatic probe AND of the F10 chord, which opens the same menu.
    //
    // ★ SO THE ANSWER IS NOT "it works" OR "it is broken" — IT IS THAT THIS DOOR
    //   IS NOT MACHINE-TESTABLE ON THE PLATFORM THAT SHIPS, and it belongs in
    //   G-06's 20-step sitting as a hand step. Which is also why the only
    //   evidence that ever existed for it was a reviewer's hand probe.
    //
    // And a test that can hang a gate for 200 s is worse than one that skips: a
    // GUI app left holding a native menu takes the KEYBOARD of whoever's machine
    // it is running on.
    if (platform != QLatin1String("offscreen")) {
        QSKIP("macOS menus are NATIVE and MODAL — measured: menuAt(0).open() under cocoa enters "
              "QCocoaMenu::showPopup -> NSContextMenuImpl presentPopup and does not return "
              "(200,674 ms, killed, stack sampled). Neither the probe nor the F10 chord can be "
              "driven headlessly on the shipping platform. THE DOOR IS A HAND STEP IN G-06's "
              "20-step sitting, not a machine check.");
    }

    // CAPABILITY PROBE FIRST, so a skip below is grounded in a measurement and
    // not in an assumption about the platform. If menus cannot even be opened
    // programmatically and observed, nothing here can be concluded at all.
    QVERIFY2(!evalJs(QStringLiteral("root.anyMenuOpen()")).toBool(),
             "a menu was already open before the probe — precondition not met");
    evalJs(QStringLiteral("mainMenuBar.menuAt(0).open()"));
    bool observable = false;
    for (int i = 0; i < 50 && !observable; ++i) {
        observable = evalJs(QStringLiteral("root.anyMenuOpen()")).toBool();
        if (!observable) QTest::qWait(20);
    }
    evalJs(QStringLiteral("root.closeAllMenus()"));
    QTRY_VERIFY2(!evalJs(QStringLiteral("root.anyMenuOpen()")).toBool(),
                 "closeAllMenus() left a menu open — Review D2's no-zombie-open property has "
                 "regressed, and that IS the product on any platform");
    QVERIFY2(observable,
             "a programmatic menuAt(0).open() does not open an observable menu — the menus "
             "themselves are broken, which is the product and not a harness limit");

    // THE DOOR ITSELF.
    QTest::keyClick(m_window, Qt::Key_F10);
    bool opened = false;
    for (int i = 0; i < 100 && !opened; ++i) {
        opened = evalJs(QStringLiteral("root.anyMenuOpen()")).toBool();
        if (!opened) QTest::qWait(20);
    }

    if (!opened && platform == QLatin1String("offscreen")) {
        evalJs(QStringLiteral("root.closeAllMenus()"));
        QSKIP("F10 does not arm under the offscreen platform: armMenuBar injects a GENUINE Space "
              "and a never-exposed window does not deliver one (menus ARE observable here, so this "
              "is the arming path and not observation). THE DOOR REMAINS UNVERIFIED — run this "
              "binary with QT_QPA_PLATFORM=cocoa, which is where the question is real.");
    }

    QVERIFY2(opened,
             qPrintable(QStringLiteral("F10 did NOT open a menu on platform '%1', where menus are "
                                       "observable and the arming path should work — the ONLY "
                                       "keyboard route to the entire menu system is dead (G-06)")
                            .arg(platform)));
    // Review D2: the same chord closes it rather than leaving a zombie-open state.
    QTest::keyClick(m_window, Qt::Key_F10);
    QTRY_VERIFY2(!evalJs(QStringLiteral("root.anyMenuOpen()")).toBool(),
                 "F10 opened a menu and would not close it — Review D2's no-zombie-open property "
                 "has regressed");
}

// ─────────────────────────────────────────────────────────────────────────
// G-06 CLAUSE 3, THE DIALOGS (O-UI, C-861)
//
// "Every control reachable by keyboard" (ROADMAP-1.0.md:255). C-859 swept the
// main window across five tool states. TWENTY DIALOGS SHIP under
// shell/qml/dialogs/, and MEASURED THIS TICK: not one of them has any
// keyboard-reachability coverage. The only Tab assertion in any dialog suite
// is tst_qd30_dialog_render's, and its subject is a SYNTHETIC `Item` inside a
// quoted QML fixture string, not a shipped dialog. A dialog is where a
// keyboard user is most trapped -- it is modal, so if its buttons are off the
// ring there is nowhere else to go.
//
// ⛔ FileDialog AND ColorDialog ARE EXCLUDED BY NAME, and that is C-860's
//    lesson paid forward rather than relearned: those are NATIVE on macOS, and
//    a native popup presented modally does not return until a human dismisses
//    it -- C-860 hung 200,674 s^H^Hms on exactly that shape and had to be killed
//    off the operator's machine. Our own dialogs under shell/qml/dialogs/ are
//    in-window QML Popups and carry no such hazard.
void TstPaneCycle::G06_theSHIPPEDDialogsControlsAreREACHABLEByTab()
{
    QVERIFY2(int(QGuiApplication::styleHints()->tabFocusBehavior())
                 == int(Qt::TabFocusAllControls),
             qPrintable(QStringLiteral("tabFocusBehavior is 0x%1, not 0xff — this sweep's reading "
                                       "of \"reachable\" is not the one it was written for (Q-D72)")
                            .arg(int(QGuiApplication::styleHints()->tabFocusBehavior()), 0, 16)));
    freshDoc();

    // Our own QML dialogs, by the ids Main.qml gives them. NOT importFileDialog,
    // saveFileDialog, openFileDialog or colorDialog: native, see above.
    const char *DIALOGS[] = {
        "aboutDialog", "discardConfirmDialog", "recoveryDialog", "modifyDialog",
        "modeConvertDialog", "filterParamDialog", "resizeDialog", "canvasSizeDialog",
        "rotateDialog", "trimDialog", "cropDialog", "fillDialog", "assignProfileDialog",
        "convertProfileDialog", "depthConvertDialog", "flattenDialog", "deleteConfirmDialog",
        "newDialog",
    };

    // ⛔ THE STARTING POINT IS THE WHOLE MEASUREMENT WHEN A MODAL IS UP, AND MY
    // FIRST VERSION GOT IT WRONG. Walking from m_canvas with a dialog open
    // reported 100 OF 100 controls unreachable -- a total that is the signature
    // of a broken instrument, not of a product in which every dialog button is
    // dead. Qt CONFINES the focus chain to a modal popup's own subtree, so a
    // walk that starts OUTSIDE the modal can never enter it. Both figures are
    // measured per dialog and reported in the failure, so the explanation is a
    // measurement and not an assertion.
    QSet<QQuickItem *> chain;
    int lastFromCanvas = -1, lastFromDialog = -1;
    const auto walkFrom = [&](QQuickItem *start) {
        chain.clear();
        QQuickItem *cur = start;
        for (int i = 0; i < 8000; ++i) {
            cur = cur->nextItemInFocusChain(true);
            if (!cur || cur == start || chain.contains(cur))
                break;
            chain << cur;
        }
        return chain.size();
    };
    std::function<bool(QQuickItem *)> canReach = [&](QQuickItem *n) -> bool {
        if (chain.contains(n))
            return true;
        for (QQuickItem *k : n->childItems())
            if (canReach(k))
                return true;
        return false;
    };

    QStringList bad, didNotOpen, empty;
    int opened = 0, swept = 0;
    for (const char *id : DIALOGS) {
        const QString sid = QLatin1String(id);
        evalJs(sid + QStringLiteral(".open()"));
        bool up = false;
        for (int i = 0; i < 60 && !up; ++i) {
            up = evalJs(sid + QStringLiteral(".opened")).toBool();
            if (!up) QTest::qWait(20);
        }
        if (!up) {                       // refused to open: recorded, not fatal
            didNotOpen << sid;
            evalJs(sid + QStringLiteral(".close()"));
            continue;
        }
        ++opened;
        QQuickItem *content = qobject_cast<QQuickItem *>(
            evalJs(sid + QStringLiteral(".contentItem")).value<QObject *>());
        lastFromCanvas = walkFrom(m_canvas);                 // evidence, kept on purpose
        lastFromDialog = content ? walkFrom(content) : -1;   // the real measurement
        int here = 0;
        if (content) {
            std::function<void(QQuickItem *)> walk = [&](QQuickItem *node) {
                for (QQuickItem *c : node->childItems()) {
                    const bool interactive = c->inherits("QQuickAbstractButton")
                                          || c->inherits("QQuickComboBox")
                                          || c->inherits("QQuickSlider")
                                          || c->inherits("QQuickSpinBox")
                                          || c->inherits("QQuickTextField")
                                          || c->inherits("QQuickTextArea");
                    if (interactive && c->isVisible() && c->isEnabled()) {
                        ++here; ++swept;
                        if (!canReach(c))
                            bad << QStringLiteral("%1 / %2 [%3]")
                                       .arg(sid, QLatin1String(c->metaObject()->className()),
                                            c->property("text").toString());
                    }
                    walk(c);
                }
            };
            walk(content);
        }
        // A dialog that opened and offered NOTHING is not a pass, it is a hole in
        // this sweep: either the content lookup failed or the dialog is empty,
        // and both mean this iteration asserted nothing at all.
        if (here == 0)
            empty << sid;
        evalJs(sid + QStringLiteral(".close()"));
        QTRY_VERIFY2(!evalJs(sid + QStringLiteral(".opened")).toBool(),
                     qPrintable(sid + QStringLiteral(" would not close — a modal left open would "
                                                     "poison every dialog measured after it")));
    }

    // ⛔ RAISED 10 -> 18 ON 2026-09-14 (C-893). All 18 shipped dialogs open;
    //   the floor of 10 let EIGHT of them stop opening in silence, and it
    //   carried no comment explaining the slack because there was none.
    QVERIFY2(opened >= 18,
             qPrintable(QStringLiteral("only %1 of %2 dialogs opened (refused: %3) — the sweep is "
                                       "not a sweep")
                            .arg(opened).arg(int(std::size(DIALOGS)))
                            .arg(didNotOpen.join(QStringLiteral(", ")))));
    // ⛔ RAISED 25 -> 100 ON 2026-09-14 (C-893), measured the same way: the
    //   sweep finds 100 controls across the 18 dialogs, so SEVENTY-FIVE (75%)
    //   could vanish under a floor of 25. It had no comment at all, which is
    //   how it survived three floor reviews.
    QVERIFY2(swept >= 100,
             qPrintable(QStringLiteral("swept only %1 controls across %2 dialogs — the sweep is "
                                       "not a sweep").arg(swept).arg(opened)));
    QVERIFY2(empty.isEmpty(),
             qPrintable(QStringLiteral("these dialogs opened and yielded NO controls, so nothing "
                                       "was asserted about them: %1")
                            .arg(empty.join(QStringLiteral(", ")))));
    QVERIFY2(bad.isEmpty(),
             qPrintable(QStringLiteral("G-06 clause 3, dialogs: %1 of %2 visible, enabled controls "
                                       "unreachable by Tab [last dialog: chain from the canvas "
                                       "%3, from inside the dialog %4] — %5")
                            .arg(bad.size()).arg(swept).arg(lastFromCanvas).arg(lastFromDialog)
                            .arg(bad.join(QStringLiteral(" · ")))));
}

// ★ WORD 8 (3)'s REAL PROPERTY, AND IT IS NOT "THE MENU EXISTS" (C-915).
//
// The Layer menu is a SECOND DOOR onto six operations the layers panel already
// owned. The thing worth testing is not that six items render -- that passes
// with six hardcoded labels and six copied guards -- but that the menu and the
// panel CANNOT DISAGREE, because the menu reads the panel's own predicate and
// calls the panel's own function.
//
// (*) THE MENU MUST BE OPEN TO READ `enabled` AT ALL. The scar is in this same
//     file, on the Help fallback: a MenuItem's state as seen from C++ is its
//     EFFECTIVE state, and a closed popup propagates its own to its contents. A
//     version of this test that read the six with the menu shut would compare
//     one table of falses against another and call it agreement.
//
// (**) THE ANTI-VACUITY ARM IS THE POINT OF THE SECOND STATE. Six predicates
//      that are false in both states agree perfectly and prove nothing, so this
//      test REQUIRES at least one to have CHANGED between the one-layer and
//      two-layer documents, and fails if none did.
void TstPaneCycle::theLAYERMenuANDThePanelAreTheSAMEImplementation()
{
    SKIP_IF_MENUS_ARE_NATIVE();   // measured 300,000 ms then SIGABRT before this line existed
    QObject *panel = m_window->findChild<QObject *>(QStringLiteral("layersPanel"));
    QVERIFY2(panel, "layersPanel is not in the booted Main.qml -- nothing below is measured");
    QObject *menu = m_window->findChild<QObject *>(QStringLiteral("menuLayer"));
    QVERIFY2(menu, "the Layer menu is not in the booted Main.qml");

    struct Pair { const char *item; const char *can; };
    static const Pair PAIRS[] = {
        {"menuLayerDelete",    "canDelete"},
        {"menuLayerMergeDown", "canMergeDown"},
        {"menuLayerMoveUp",    "canMoveUp"},
        {"menuLayerMoveDown",  "canMoveDown"},
    };

    auto pump = [] {
        for (int i = 0; i < 8; ++i)
            QCoreApplication::processEvents(QEventLoop::AllEvents, 20);
    };
    auto readAll = [&](const char *state, QVector<bool> *out) {
        QMetaObject::invokeMethod(menu, "open");
        pump();
        QTRY_VERIFY2(menu->property("visible").toBool(),
                     qPrintable(QStringLiteral("the Layer menu did not open (%1)")
                                    .arg(QLatin1String(state))));
        for (const Pair &p : PAIRS) {
            QObject *it = m_window->findChild<QObject *>(QLatin1String(p.item));
            QVERIFY2(it, qPrintable(QStringLiteral("menu item %1 is missing")
                                        .arg(QLatin1String(p.item))));
            const bool can = panel->property(p.can).toBool();
            const bool en = it->property("enabled").toBool();
            QVERIFY2(can == en,
                     qPrintable(QStringLiteral("%1: panel.%2 is %3 but its menu item is %4 -- the "
                                               "door and the panel disagree, so the menu is not "
                                               "reading the panel's predicate")
                                    .arg(QLatin1String(state), QLatin1String(p.can))
                                    .arg(can).arg(en)));
            out->append(can);
        }
        QMetaObject::invokeMethod(menu, "close");
        pump();
    };

    freshDoc();
    QVector<bool> one, two;
    readAll("one layer", &one);

    // (***) THE SETUP IS PART OF THE ASSERTION. My first version called
    //       applyCommand("layer.add", {}, {}, "") with no arguments; the engine
    //       REFUSED it, the document stayed at one layer, and the anti-vacuity
    //       arm below fired on the first run -- which is the arm working, not
    //       failing. The argument shape is the one this file already uses
    //       nineteen lines up, and the layer count is now VERIFIED to move
    //       rather than assumed to.
    QVERIFY(m_controller);
    QVERIFY2(m_controller
                 ->applyCommand(QStringLiteral("layer.add"), {},
                                {{QStringLiteral("kind"), QStringLiteral("raster")},
                                 {QStringLiteral("newLayerId"), m_controller->newUuid()}},
                                QString())
                 .value(QStringLiteral("ok"))
                 .toBool(),
             "layer.add refused -- the second state would be the first state again");
    QTRY_VERIFY(m_controller->property("layers").toList().size() >= 2);
    pump();
    readAll("two layers", &two);

    QCOMPARE(one.size(), int(std::size(PAIRS)));
    QCOMPARE(two.size(), int(std::size(PAIRS)));

    int varied = 0;
    QStringList moved;
    for (int i = 0; i < one.size(); ++i) {
        if (one.at(i) != two.at(i)) {
            ++varied;
            moved << QLatin1String(PAIRS[i].can);
        }
    }
    QVERIFY2(varied > 0,
             "not one of the six predicates changed between a one-layer and a two-layer document, "
             "so this test compared two identical tables and proved nothing");
    qInfo("layer-menu delegation: %d of %d predicates moved between states (%s)",
          varied, int(std::size(PAIRS)), qPrintable(moved.join(QStringLiteral(", "))));
}

// (*) G-06's REMAINING ARM, AND IT IS SPLIT BY PLATFORM ON PURPOSE (C-927).
//
// "Can a keyboard user reach the Layer menu's items" is TWO questions with two
// different answers here:
//
//   OPENING the menu    F10 -> menu bar -> arrows. The MenuBar's keyboard-nav
//                       state does not arm offscreen, which is why
//                       G06_theF10MenuDOORIsTHEONLYRoute... SKIPs -- and skips
//                       on the platform every CI run uses.
//   NAVIGATING it once  the popup handles Up/Down itself and does so offscreen.
//   it is open          THAT is what this test measures.
//
// (**) QQC2 MENUS SKIP DISABLED ITEMS, so the reachable set is the ENABLED set
//      and nothing else. That makes the assertion state-dependent, which is a
//      feature: with one layer some items are disabled, so "visited == enabled"
//      is a real comparison rather than "six == six".
//
// (***) ANTI-VACUITY: if EVERY item were enabled the test would degenerate into
//       walking a list. It therefore requires the enabled set to be a PROPER
//       SUBSET of the six in the one-layer state -- if that ever stops being
//       true the test says so instead of passing quietly.
void TstPaneCycle::G06_everyENABLEDLayerMenuItemIsREACHABLEByArrowKeys()
{
    freshDoc();
    QObject *panel = m_window->findChild<QObject *>(QStringLiteral("layersPanel"));
    QVERIFY2(panel, "layersPanel is not in the booted Main.qml");
    // 1.0 final F-9: on ONE layer every Layer item is now correctly disabled
    // (Move Up/Down were the only ones lit, and that was the bug). The
    // fixture is therefore TWO layers with the bottom one active: Delete and
    // Move Up enabled, Move Down disabled -- still a proper subset.
    {
        QVariantMap prm;
        prm.insert(QStringLiteral("kind"), QStringLiteral("raster"));
        prm.insert(QStringLiteral("newLayerId"), m_controller->newUuid());
        QVERIFY(m_controller->applyCommand(QStringLiteral("layer.add"), {}, prm, {})
                    .value(QStringLiteral("ok")).toBool());
        QTRY_COMPARE(m_controller->layers().size(), 2);
        bool found = false;
        for (int i = 0; i < 2 && !found; ++i) {
            m_controller->setActiveLayerIndex(i);
            QCoreApplication::processEvents();
            found = panel->property("canMoveUp").toBool()
                    && !panel->property("canMoveDown").toBool();
        }
        QVERIFY2(found, "no bottom-of-two layer to make active");
    }
    QObject *menu = m_window->findChild<QObject *>(QStringLiteral("menuLayer"));
    QVERIFY2(menu, "the Layer menu is not in the booted Main.qml");

    static const char *ITEMS[] = {"menuLayerDelete",
                                  "menuLayerMergeDown", "menuLayerMoveUp", "menuLayerMoveDown"};
    // 1.0 final (corrigendum (l)): Layer > Group / Ungroup are removed.
    QVERIFY2(!m_window->findChild<QObject *>(QStringLiteral("menuLayerGroup"))
                 && !m_window->findChild<QObject *>(QStringLiteral("menuLayerUngroup")),
             "Layer > Group / Ungroup are back; they were removed for 1.0");
    auto pump = [] {
        for (int i = 0; i < 6; ++i)
            QCoreApplication::processEvents(QEventLoop::AllEvents, 20);
    };

    SKIP_IF_MENUS_ARE_NATIVE();   // measured 300,570 ms then SIGABRT before this line existed

    QMetaObject::invokeMethod(menu, "open");
    pump();
    QTRY_VERIFY2(menu->property("visible").toBool(), "the Layer menu did not open");

    QStringList enabled, disabled;
    QVector<QObject *> objs;
    for (const char *n : ITEMS) {
        QObject *it = m_window->findChild<QObject *>(QLatin1String(n));
        QVERIFY2(it, qPrintable(QStringLiteral("menu item %1 is missing").arg(QLatin1String(n))));
        objs << it;
        (it->property("enabled").toBool() ? enabled : disabled) << QLatin1String(n);
    }
    QVERIFY2(!enabled.isEmpty(), "no Layer item is enabled on a fresh document -- nothing to reach");
    QVERIFY2(!disabled.isEmpty(),
             "EVERY Layer item is enabled on the two-layer fixture, so this test would only be "
             "walking a list; the fixture no longer distinguishes reachable from present");

    // Walk the popup with Down. Twelve presses is twice the item count: enough to
    // wrap the whole menu, few enough that a runaway is still a failure.
    QSet<QString> visited;
    for (int step = 0; step < 12; ++step) {
        QTest::keyClick(m_window, Qt::Key_Down);
        pump();
        for (int i = 0; i < objs.size(); ++i) {
            const QVariant hl = objs.at(i)->property("highlighted");
            if (hl.isValid() && hl.toBool())
                visited << QLatin1String(ITEMS[i]);
        }
    }
    QMetaObject::invokeMethod(menu, "close");
    pump();

    QStringList missed;
    for (const QString &e : enabled)
        if (!visited.contains(e))
            missed << e;
    QVERIFY2(missed.isEmpty(),
             qPrintable(QStringLiteral("enabled Layer items the arrow keys never highlighted: %1 "
                                       "(enabled %2, visited %3) -- an item a keyboard user cannot "
                                       "reach is not in the menu as far as they are concerned")
                            .arg(missed.join(QStringLiteral(", ")))
                            .arg(enabled.size()).arg(visited.size())));
    qInfo("layer-menu arrow reach: %d enabled, %d disabled, %d highlighted",
          int(enabled.size()), int(disabled.size()), int(visited.size()));
}

int main(int argc, char *argv[])
{
    if (qgetenv("QT_QPA_PLATFORM").isEmpty())
        qputenv("QT_QPA_PLATFORM", "offscreen");
    QQuickStyle::setStyle(QStringLiteral("Basic"));
    QGuiApplication app(argc, argv);
    // MIRRORS shell/src/main.cpp and tst_gesture_chrome's main() — operator WORD 16 (4):
    // "a harness must model the app's focus policy or it measures a different program".
    // This binary has its own main() and never compiles src/main.cpp, so without this
    // line every reach case here reads Qt's macOS DEFAULT instead of the shipped policy.
    //
    // ★ MEASURED, which is why the line is here rather than assumed useful. Run under
    //   QT_QPA_PLATFORM=cocoa WITHOUT it and G06_everyVISIBLEEnabledControlInTheWHOLE
    //   WindowIsReachable fails on its own guard -- "tabFocusBehavior is 0x3, not 0xff".
    //   Offscreen defaults to 0xff and hid the gap for the whole life of this suite;
    //   cocoa is 0x3 and exposed it in one run. The guard was right and the harness was
    //   a line short.
    //
    // ★★ AND THIS SUITE *CAN* RUN ON COCOA, which tst_gesture_chrome cannot: this
    //   fixture waits for qWaitForWindowExposed, while that one asserts
    //   m_window->isActive(), and macOS will not make a shell-launched window key.
    //   That difference is the whole reason a cocoa reading is available here at all.
    QGuiApplication::styleHints()->setTabFocusBehavior(Qt::TabFocusAllControls);
    TstPaneCycle tc;
    QTEST_SET_MAIN_SOURCE_PATH
    return QTest::qExec(&tc, argc, argv);
}

// 1.0 final F-9 (O-WATCH #1115, native): Layer > Move Up / Move Down were
// enabled on a one-layer document; Up recorded a no-op "Reorder Layer" and
// Down surfaced "index must be >= 0". The bounds now live in canMoveUp /
// canMoveDown, which the menu items, the Ctrl+] / Ctrl+[ chords and the
// panel buttons all read. Pinned: one layer, top of two, bottom of two --
// panel property, menu item, and the chord (no history change when barred).
void TstPaneCycle::f9MoveUpDownAreBoundedOnEveryDoor()
{
    freshDoc(); // one layer
    QObject *panel = m_window->findChild<QObject *>(QStringLiteral("layersPanel"));
    QVERIFY(panel);
    QObject *up = m_window->findChild<QObject *>(QStringLiteral("menuLayerMoveUp"));
    QObject *down = m_window->findChild<QObject *>(QStringLiteral("menuLayerMoveDown"));
    QVERIFY(up && down);
    auto state = [&](bool wantUp, bool wantDown, const char *where) {
        QCoreApplication::processEvents();
        QVERIFY2(panel->property("canMoveUp").toBool() == wantUp
                     && up->property("enabled").toBool() == wantUp,
                 qPrintable(QStringLiteral("%1: Move Up should be %2")
                                .arg(QLatin1String(where), wantUp ? "enabled" : "disabled")));
        QVERIFY2(panel->property("canMoveDown").toBool() == wantDown
                     && down->property("enabled").toBool() == wantDown,
                 qPrintable(QStringLiteral("%1: Move Down should be %2")
                                .arg(QLatin1String(where), wantDown ? "enabled" : "disabled")));
    };
    // Window-scoped Shortcuts fire only in the ACTIVE window. Natively the
    // test window is not active by default (measured: isActive()==false and
    // the allowed chord did nothing), which would make the barred-chord
    // checks vacuous. Activate first, as mm2 does.
    m_window->requestActivate();
    QTRY_VERIFY(m_window->isActive());
    auto chordIsInert = [&](Qt::Key key, const char *where) {
        focusCanvas();
        const int h = m_controller->historyEntries().size();
        QTest::keyClick(m_window, key, Qt::ControlModifier);
        QTest::qWait(50);
        QVERIFY2(m_controller->historyEntries().size() == h,
                 qPrintable(QStringLiteral("%1: a barred reorder chord changed history")
                                .arg(QLatin1String(where))));
        QVERIFY2(!m_controller->statusText().contains(QLatin1String("out_of_range")),
                 qPrintable(QStringLiteral("%1: the raw out_of_range refusal surfaced")
                                .arg(QLatin1String(where))));
    };

    state(false, false, "one layer");
    chordIsInert(Qt::Key_BracketRight, "one layer, Ctrl+]");
    chordIsInert(Qt::Key_BracketLeft, "one layer, Ctrl+[");

    QVariantMap prm;
    prm.insert(QStringLiteral("kind"), QStringLiteral("raster"));
    prm.insert(QStringLiteral("newLayerId"), m_controller->newUuid());
    QVERIFY(m_controller->applyCommand(QStringLiteral("layer.add"), {}, prm, {})
                .value(QStringLiteral("ok")).toBool());
    QTRY_COMPARE(m_controller->layers().size(), 2);
    // Find the top and bottom of the two root siblings by stack order.
    const QVariantList rows = m_controller->layers();
    int topIdx = -1, bottomIdx = -1;
    for (int i = 0; i < rows.size(); ++i) {
        m_controller->setActiveLayerIndex(i);
        QCoreApplication::processEvents();
        if (panel->property("canMoveDown").toBool() && !panel->property("canMoveUp").toBool())
            topIdx = i;
        if (panel->property("canMoveUp").toBool() && !panel->property("canMoveDown").toBool())
            bottomIdx = i;
    }
    QVERIFY2(topIdx >= 0 && bottomIdx >= 0 && topIdx != bottomIdx,
             "two root layers did not resolve to one top and one bottom");
    m_controller->setActiveLayerIndex(topIdx);
    state(false, true, "top of two");
    chordIsInert(Qt::Key_BracketRight, "top of two, Ctrl+]");
    m_controller->setActiveLayerIndex(bottomIdx);
    state(true, false, "bottom of two");
    chordIsInert(Qt::Key_BracketLeft, "bottom of two, Ctrl+[");
    // And the allowed move still works: bottom moves up, one history row.
    // This is also the CONTROL for the barred-chord checks above: if the
    // chord were never delivered, they would pass vacuously and this fails.
    focusCanvas();
    const int h = m_controller->historyEntries().size();
    const bool canUpBefore = panel->property("canMoveUp").toBool();
    const int activeBefore = m_controller->activeLayerIndex();
    QTest::keyClick(m_window, Qt::Key_BracketRight, Qt::ControlModifier);
    QTest::qWaitFor([&] { return m_controller->historyEntries().size() == h + 1; }, 2000);
    QVERIFY2(m_controller->historyEntries().size() == h + 1,
             qPrintable(QStringLiteral("allowed Ctrl+] from the bottom layer did not reorder: "
                                       "history %1 -> %2, canMoveUp before %3, active %4 -> %5, "
                                       "window active %6, focus %7, status '%8'")
                            .arg(h).arg(m_controller->historyEntries().size())
                            .arg(canUpBefore).arg(activeBefore)
                            .arg(m_controller->activeLayerIndex())
                            .arg(m_window->isActive())
                            .arg(m_window->activeFocusItem()
                                     ? QString::fromLatin1(m_window->activeFocusItem()->metaObject()->className())
                                     : QStringLiteral("none"))
                            .arg(m_controller->statusText())));
}


// 1.0 final F-10 (O-WATCH #1123, native): after Move Up / Move Down the
// ACTIVE layer was the neighbour (the active layer is an index and the
// reorder kept the index), so a second move or the next stroke landed on the
// other layer. Pinned through the chords: move the top layer down, it stays
// active; move it back up, it still is; the history grows by one each time.
void TstPaneCycle::f10TheMovedLayerStaysActive()
{
    freshDoc();
    m_window->requestActivate();
    QTRY_VERIFY(m_window->isActive());
    QVariantMap prm;
    prm.insert(QStringLiteral("kind"), QStringLiteral("raster"));
    const QString movedId = m_controller->newUuid();
    prm.insert(QStringLiteral("newLayerId"), movedId);
    QVERIFY(m_controller->applyCommand(QStringLiteral("layer.add"), {}, prm, {})
                .value(QStringLiteral("ok")).toBool());
    QTRY_COMPARE(m_controller->layers().size(), 2);
    auto activeId = [&]() {
        return m_controller->layers().value(m_controller->activeLayerIndex())
            .toMap().value(QStringLiteral("layerId")).toString();
    };
    QCOMPARE(activeId(), movedId); // layer.add selects the new (top) layer
    QObject *panel = m_window->findChild<QObject *>(QStringLiteral("layersPanel"));
    QVERIFY(panel);
    QVERIFY2(panel->property("canMoveDown").toBool() && !panel->property("canMoveUp").toBool(),
             "the new layer is not the top of two");

    focusCanvas();
    int h = m_controller->historyEntries().size();
    QTest::keyClick(m_window, Qt::Key_BracketLeft, Qt::ControlModifier); // Move Down
    QTRY_COMPARE(m_controller->historyEntries().size(), h + 1);
    QVERIFY2(activeId() == movedId,
             qPrintable(QStringLiteral("after Move Down the active layer is %1, not the moved %2")
                            .arg(activeId(), movedId)));
    QVERIFY2(panel->property("canMoveUp").toBool() && !panel->property("canMoveDown").toBool(),
             "the moved layer is active but the panel does not see it at the bottom");

    focusCanvas();
    h = m_controller->historyEntries().size();
    QTest::keyClick(m_window, Qt::Key_BracketRight, Qt::ControlModifier); // Move Up
    QTRY_COMPARE(m_controller->historyEntries().size(), h + 1);
    QVERIFY2(activeId() == movedId,
             qPrintable(QStringLiteral("after Move Up the active layer is %1, not the moved %2")
                            .arg(activeId(), movedId)));
}


// 1.0 final R5-4 (O-SIT RC5 sitting, O-WATCH #1186): after Layer > Merge Down
// the highlight moved to the layer ABOVE the merged result (F-10's shape:
// the active layer is an index). Pinned: three root layers, the middle one
// active, Merge Down (Ctrl+E): the receiving (bottom) layer is active, and
// the layer above is not.
void TstPaneCycle::r54MergeDownKeepsTheMergedLayerActive()
{
    freshDoc(); // one layer: the receiving one
    m_window->requestActivate();
    QTRY_VERIFY(m_window->isActive());
    auto idAt = [&](int i) {
        return m_controller->layers().value(i).toMap().value(QStringLiteral("layerId")).toString();
    };
    auto activeId = [&]() { return idAt(m_controller->activeLayerIndex()); };
    const QString bottomId = idAt(0);
    QStringList added;
    for (int n = 0; n < 2; ++n) {
        const QString id = m_controller->newUuid();
        QVERIFY(m_controller->applyCommand(QStringLiteral("layer.add"), {},
                    {{QStringLiteral("kind"), QStringLiteral("raster")},
                     {QStringLiteral("newLayerId"), id}}, {})
                    .value(QStringLiteral("ok")).toBool());
        added << id;
    }
    QTRY_COMPARE(m_controller->layers().size(), 3);
    // Activate the middle layer (the first one added, directly above bottom).
    QVERIFY(m_controller->selectLayerById(added.at(0)));
    QCOMPARE(activeId(), added.at(0));
    focusCanvas();
    const int h = m_controller->historyEntries().size();
    QTest::keyClick(m_window, Qt::Key_E, Qt::ControlModifier); // Layer > Merge Down
    QTRY_COMPARE(m_controller->historyEntries().size(), h + 1);
    QTRY_COMPARE(m_controller->layers().size(), 2);
    QVERIFY2(activeId() == bottomId,
             qPrintable(QStringLiteral("after Merge Down the active layer is %1, not the "
                                       "receiving layer %2 (the layer above is %3)")
                            .arg(activeId(), bottomId, added.at(1))));
}

#include "tst_pane_cycle.moc"
