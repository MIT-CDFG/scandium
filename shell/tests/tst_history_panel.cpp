// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Wave F — THE HISTORY-PANEL JUMP NET (census item 25, DEAD row 1: the
// panel's rows carried ListItem semantics with no handler of any kind).
// Boots the REAL Main.qml offscreen (the tst_palette_pins harness: this
// binary compiles its own copy of the AppnameShell QML module and wires
// the real controller/announcer like shell/src/main.cpp) against a REAL
// engine document, and proves with real synthesized events:
//
//   1. CLICK JUMPS BACK (engine truth): a left-click on a mid-history
//      row runs history.goto{position} through the apply door — the
//      composite readback shows the JUMPED state's pixels, canRedo
//      flips true, and the history door's active flags move (§40.8
//      position form: the † run moves the cursor, appends nothing).
//   2. CLICK JUMPS FORWARD: a later click on the tail row redoes back
//      to the newest state — pixels + flags again.
//   3. KEYBOARD ACTIVATION: Return (and Space) on the focused row land
//      the same engine end-states (rows join the tab chain — the
//      panels pane is an F6 cycle stop).
//   4. MARKER ROWS (†): a history.snapshot row occupies a journal slot
//      but NO undo-stack slot — the row-position mapping must repeat
//      the preceding step's position (the state the snapshot captured)
//      and keep later rows' positions unshifted, or the tail-row jump
//      answers the engine's typed out_of_range and the readback reds.
//   5. RECEIPT + REFUSAL SHAPES: the controller arm returns
//      applyCommand's map — ok:true on a legal jump; the out-of-range
//      probe answers error.schema.out_of_range TYPED (range truth is
//      the engine's, never pre-computed shell-side).
//   6. ANNOUNCER SILENCE: no cataloged id fits a multi-step jump
//      (a11y.history.undone/redone name ONE step's label — goldens
//      pinned in tst_announcer), so the jump must announce NOTHING and
//      refuse NOTHING (silence by design, not by refused-id accident).

#include "announcer.h"
#include "click_focus_parity.h"
#include "canvas_view.h"
#include "document_controller.h"
#include "envelope_builder.h"
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
#include <QtTest>

using namespace appname::shell;

class TstHistoryPanel : public QObject {
    Q_OBJECT
private slots:
    void initTestCase();
    void init(); // fresh doc + red fill + blue fill per case

    void clickMidRowJumpsBackThenTailRowForward();
    void keyboardActivationJumps();
    void markerRowsRepeatPositionAndKeepTailReachable();
    void receiptShapesAndAnnouncerSilence();
    void cursorStaysVisibleAndIsSuppressedWhileFocused();
    void aFilterRowNamesTheFilterNotTheGenericKind();
    void unchangedAutoRefitIsSilentWhenGated();

private:
    CatalogQml *m_i18n = nullptr;
    DocumentController *m_controller = nullptr;
    Announcer *m_announcer = nullptr;
    QQmlApplicationEngine *m_engine = nullptr;
    QQuickWindow *m_window = nullptr;
    QQuickItem *m_historyList = nullptr;

    int m_redRow = -1;  // model index of the first fill (red) row
    int m_blueRow = -1; // model index of the second fill (blue) row

    QVariantMap applyOk(const char *kind, const QVariantMap &target,
                        const QVariantMap &params)
    {
        return m_controller->applyCommand(QLatin1String(kind), target,
                                          params, QString());
    }
    // Whole-canvas constant fill on the first layer — one history step,
    // pixel-discriminating between steps (the selection-gestures
    // macro's proven envelope shape).
    void fillCanvas(double r, double g, double b)
    {
        QVariantMap color;
        color.insert(QStringLiteral("model"), QStringLiteral("rgb"));
        color.insert(QStringLiteral("comps"), QVariantList{r, g, b, 0.0});
        QVariantMap params;
        params.insert(QStringLiteral("content"), QStringLiteral("color"));
        params.insert(QStringLiteral("color"), color);
        QVariantMap target;
        target.insert(QStringLiteral("layerId"),
                      m_controller->layers().first().toMap().value(
                          QStringLiteral("layerId")));
        QVERIFY(applyOk("fill", target, params)
                    .value(QStringLiteral("ok"))
                    .toBool());
    }
    QColor pixel00()
    {
        QImage px;
        if (!m_controller->readRegionImage(QRect(0, 0, 1, 1), &px))
            return QColor();
        return px.pixelColor(0, 0);
    }
    // ENGINE truth via the readback door — named states, discriminating.
    bool pixelIsRed() { const QColor c = pixel00();
        return c.red() > 200 && c.blue() < 60 && c.green() < 60; }
    bool pixelIsBlue() { const QColor c = pixel00();
        return c.blue() > 200 && c.red() < 60 && c.green() < 60; }
    QVariantList rows() const { return m_controller->historyEntries(); }
    QVariantMap rowAt(int i) const { return rows().value(i).toMap(); }
    QQuickItem *rowItem(int i)
    {
        QQuickItem *item = nullptr;
        QMetaObject::invokeMethod(m_historyList, "itemAtIndex",
                                  Q_RETURN_ARG(QQuickItem *, item),
                                  Q_ARG(int, i));
        return item;
    }
    void clickRow(int i)
    {
        QQuickItem *item = rowItem(i);
        QVERIFY2(item, "history row delegate not instantiated");
        QTRY_VERIFY(item->width() > 0 && item->height() > 0);
        const QPointF scene = item->mapToScene(
            QPointF(item->width() / 2.0, item->height() / 2.0));
        QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                          scene.toPoint());
    }
    // Q-D22 VIEWPORT READS. Every other case in this suite reads
    // m_controller->historyEntries() — the MODEL — which is exactly why a
    // view that never scrolled was invisible here for the whole of Wave F.
    // These read the Flickable's own geometry instead.
    double contentY() const
    {
        return m_historyList->property("contentY").toDouble();
    }
    void parkAtTop()
    {
        QMetaObject::invokeMethod(m_historyList, "positionViewAtBeginning");
    }
    // ENGINE truth for the cursor: the last active journal row. Computed
    // from the history door, NOT read back off the QML binding, so the
    // assertion cannot echo the thing it is testing.
    int engineCursorRow() const
    {
        const QVariantList r = rows();
        int last = -1;
        for (int i = 0; i < r.size(); ++i)
            if (r.at(i).toMap().value(QStringLiteral("active")).toBool())
                last = i;
        return last;
    }
    // Is the cursor row's delegate inside the visible band? Delegates are
    // children of contentItem, so y is content-space and comparable to
    // contentY directly.
    bool cursorRowIsInViewport()
    {
        QQuickItem *cur = rowItem(engineCursorRow());
        if (!cur)
            return false; // not even realized — far outside the band
        const double top = contentY();
        const double bottom = top + m_historyList->height();
        return cur->y() >= top - 1.0 && cur->y() + cur->height() <= bottom + 1.0;
    }
    // Fill enough times to overflow the viewport (rows are 26 + 1 spacing).
    int fillPastViewport()
    {
        const int rowsNeeded =
            int(m_historyList->height() / 27.0) + 8;
        for (int i = 0; i < rowsNeeded; ++i)
            fillCanvas((i % 3) / 2.0, (i % 5) / 4.0, (i % 7) / 6.0);
        return rowsNeeded;
    }
    // Locate the two fill rows by ENGINE-reported kind (echo-proof: the
    // kinds come back through the history door, not from shell state).
    void locateFillRows()
    {
        m_redRow = m_blueRow = -1;
        const QVariantList r = rows();
        for (int i = 0; i < r.size(); ++i) {
            if (r.at(i).toMap().value(QStringLiteral("kind")).toString()
                == QLatin1String("fill")) {
                if (m_redRow < 0)
                    m_redRow = i;
                else
                    m_blueRow = i;
            }
        }
        QVERIFY2(m_redRow >= 0 && m_blueRow > m_redRow,
                 "did not find the two fill rows in the history door");
    }
};

void TstHistoryPanel::initTestCase()
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

    m_historyList = m_window->findChild<QQuickItem *>(
        QStringLiteral("historyList"));
    QVERIFY2(m_historyList, "historyList not found");
}

void TstHistoryPanel::init()
{
    QVERIFY(m_controller->newDocument(320, 200, QStringLiteral("white")));
    fillCanvas(1.0, 0.0, 0.0); // step: red
    fillCanvas(0.0, 0.0, 1.0); // step: blue
    QVERIFY2(pixelIsBlue(), "setup readback is not the blue tail state");
    QVERIFY(m_controller->canUndo());
    QVERIFY(!m_controller->canRedo());
    locateFillRows();
    // All rows active at the tail (cursor past every entry).
    for (int i = 0; i < rows().size(); ++i)
        QVERIFY2(rowAt(i).value(QStringLiteral("active")).toBool(),
                 "setup row not active at the tail");
    QTRY_VERIFY(rowItem(m_blueRow) != nullptr); // delegates settled
}

void TstHistoryPanel::clickMidRowJumpsBackThenTailRowForward()
{
    // BACK: click the red-fill row (mid-history — the blue fill is
    // ahead of it).
    clickRow(m_redRow);
    QTRY_VERIFY2(pixelIsRed(),
                 "mid-row click did not land the red engine state");
    QVERIFY(m_controller->canRedo()); // forward steps exist — info truth
    QVERIFY2(!rowAt(m_blueRow).value(QStringLiteral("active")).toBool(),
             "blue row still active after jumping behind it");
    QVERIFY2(rowAt(m_redRow).value(QStringLiteral("active")).toBool(),
             "red row lost active after jumping TO it");

    // FORWARD: click the tail (blue) row — the redo run.
    clickRow(m_blueRow);
    QTRY_VERIFY2(pixelIsBlue(),
                 "tail-row click did not redo to the blue engine state");
    QVERIFY(!m_controller->canRedo());
    QVERIFY2(rowAt(m_blueRow).value(QStringLiteral("active")).toBool(),
             "blue row not active after the forward jump");
}

void TstHistoryPanel::keyboardActivationJumps()
{
    // BACK via Return on the focused red row.
    QQuickItem *red = rowItem(m_redRow);
    QVERIFY(red);
    red->forceActiveFocus();
    QTRY_VERIFY(red->hasActiveFocus());
    QTest::keyClick(m_window, Qt::Key_Return);
    QTRY_VERIFY2(pixelIsRed(),
                 "Return on the red row did not land the red state");
    QVERIFY(m_controller->canRedo());

    // FORWARD via Space on the focused blue row (both key arms real).
    QQuickItem *blue = rowItem(m_blueRow);
    QVERIFY(blue);
    blue->forceActiveFocus();
    QTRY_VERIFY(blue->hasActiveFocus());
    QTest::keyClick(m_window, Qt::Key_Space);
    QTRY_VERIFY2(pixelIsBlue(),
                 "Space on the blue row did not redo to the blue state");
    QVERIFY(!m_controller->canRedo());
}

void TstHistoryPanel::markerRowsRepeatPositionAndKeepTailReachable()
{
    // Journal now: [..., fill(red), fill(blue)]. Add a † marker then a
    // third step so a marker sits MID-journal:
    // [..., fill(red), fill(blue), history.snapshot, fill(red2)].
    QVERIFY(applyOk("history.snapshot", {}, {})
                .value(QStringLiteral("ok"))
                .toBool());
    fillCanvas(1.0, 0.0, 0.0);
    const QVariantList r = rows();
    const int snapRow = r.size() - 2;
    const int tailRow = r.size() - 1;
    QCOMPARE(rowAt(snapRow).value(QStringLiteral("kind")).toString(),
             QStringLiteral("history.snapshot"));
    // † mapping pin: the marker row repeats the blue row's position
    // (markers occupy NO undo-stack slot — §40.8), and the tail row is
    // exactly one past it. A marker-counting regression here shifts the
    // tail position out of the stack range and the jump below refuses.
    QCOMPARE(rowAt(snapRow).value(QStringLiteral("position")).toInt(),
             rowAt(m_blueRow).value(QStringLiteral("position")).toInt());
    QCOMPARE(rowAt(tailRow).value(QStringLiteral("position")).toInt(),
             rowAt(snapRow).value(QStringLiteral("position")).toInt() + 1);

    // Jump back across the marker to the BLUE state, then forward to
    // the tail — engine readback discriminates each landing.
    clickRow(m_blueRow);
    QTRY_VERIFY2(pixelIsBlue(),
                 "jump across the marker did not land the blue state");
    QVERIFY(m_controller->canRedo());
    clickRow(tailRow);
    QTRY_VERIFY2(pixelIsRed(),
                 "tail jump past the marker did not land the red2 state");
    QVERIFY(!m_controller->canRedo());
}

void TstHistoryPanel::receiptShapesAndAnnouncerSilence()
{
    // RECEIPT: the controller arm returns applyCommand's map.
    const int backPos =
        rowAt(m_redRow).value(QStringLiteral("position")).toInt();
    const QVariantMap ok = m_controller->historyGoto(backPos);
    QVERIFY2(ok.value(QStringLiteral("ok")).toBool(),
             "legal jump-back receipt not ok");
    QTRY_VERIFY(pixelIsRed());

    // TYPED REFUSAL: range truth is the ENGINE's — far out of range
    // answers error.schema.out_of_range, document unchanged.
    const QVariantMap refused = m_controller->historyGoto(9999);
    QVERIFY(!refused.value(QStringLiteral("ok")).toBool());
    QCOMPARE(refused.value(QStringLiteral("code")).toString(),
             QStringLiteral("error.schema.out_of_range"));
    QVERIFY2(pixelIsRed(), "refused jump moved the document");

    // ANNOUNCER SILENCE by design: a jump must neither announce (no
    // cataloged id fits a multi-step run) nor refuse an id (nothing may
    // even be ATTEMPTED — refusal-count delta would mean an uncataloged
    // id was submitted). Wait out a full rate-limit window so a queued
    // announcement cannot hide from the log (vacuous-negative hunt).
    // W-18: drain the announcer FIRST. init()'s newDocument fit announces the
    // zoom and the refused jump above announces its rejection; the rate limit
    // flushes one entry per window, so either can land after logBefore and be
    // charged to the click (measured on Windows: the fit and the refusal ran
    // before logBefore, the click ran no fit, the entry landed 1.2-1.7 s in).
    // Settled = the log did not grow across a full window.
    for (int settle = 0; settle < 10; ++settle) {
        const int n = m_announcer->announcementLog().size();
        QTest::qWait(650);
        if (m_announcer->announcementLog().size() == n)
            break;
    }
    const int logBefore = m_announcer->announcementLog().size();
    const int refusedBefore = m_announcer->refusedCount();
    clickRow(m_blueRow);
    QTRY_VERIFY(pixelIsBlue());
    QTest::qWait(650); // > the 500 ms announcer window
    // W-18 (Windows: Actual 3, Expected 2): the cause is not settled from the
    // Mac — this suite never clicks a button, so it is not the click-focus
    // class. Name every unexpected entry on failure so the next Windows run
    // identifies the source instead of only counting it.
    {
        const QVariantList log = m_announcer->announcementLog();
        QString extra;
        for (int i = logBefore; i < log.size(); ++i) {
            const QVariantMap e = log.at(i).toMap();
            extra += QStringLiteral(" [%1 %2 \"%3\" @%4ms]")
                         .arg(e.value(QStringLiteral("source")).toString(),
                              e.value(QStringLiteral("id")).toString(),
                              e.value(QStringLiteral("text")).toString(),
                              e.value(QStringLiteral("ms")).toString());
        }
        QVERIFY2(log.size() == logBefore,
                 qPrintable(QStringLiteral("a history-row click announced %1 "
                                           "entr(y/ies):%2")
                                .arg(log.size() - logBefore)
                                .arg(extra)));
    }
    QCOMPARE(m_announcer->refusedCount(), refusedBefore);

    // Instrument check (RULE 9): the same log DOES grow when a real
    // cataloged announcement fires — the silence assert above is armed.
    m_announcer->announce(QStringLiteral("test"),
                          QStringLiteral("a11y.file.saved"), {});
    QTRY_COMPARE(m_announcer->announcementLog().size(), logBefore + 1);
}

void TstHistoryPanel::cursorStaysVisibleAndIsSuppressedWhileFocused()
{
    // Q-D22, RULED (a) 2026-09-01: scroll to the cursor when history
    // changes, SUPPRESSED while the panel has focus.
    //
    // THE TWO ARMS ARE EACH OTHER'S CONTROL. `cursorRowIsInViewport()` is
    // asserted TRUE in arm A and FALSE in arm B against the same code and
    // the same instrument, so a predicate that could only ever answer true
    // (the classic false green on a viewport check) is ruled out here
    // rather than assumed.
    QVERIFY2(m_historyList->height() > 0, "history list has no height");
    const int added = fillPastViewport();
    QVERIFY2(added > 0, "no rows added");
    QTRY_VERIFY(rows().size() > added);

    // ---- ARM A: UNFOCUSED — the view follows the cursor. ----
    parkAtTop();
    QTRY_COMPARE(contentY(), 0.0);
    // SET THE PRECONDITION, DO NOT INHERIT IT. The window is built once in
    // initTestCase() and init() does not reset focus, so an earlier case
    // (keyboardActivationJumps) leaves a row focused and arm A would run
    // under arm B's condition — measured, as a red, on the first run of
    // this case.
    //   ★ AND `contentItem()->forceActiveFocus()` DOES NOT CLEAR IT — also
    //   measured, as a second identical red: the content item is a focus
    //   scope, so forcing focus onto it hands focus straight back down to
    //   its remembered child, which is the row we were trying to leave.
    //   Focus a REAL item outside the panel instead. This is also the
    //   honest scenario: the user is at the canvas, painting, which is
    //   precisely when history changes and the cursor must follow.
    QQuickItem *canvasView =
        m_window->findChild<QQuickItem *>(QStringLiteral("canvasView"));
    QVERIFY2(canvasView, "canvasView not found — cannot park focus off the panel");
    canvasView->forceActiveFocus();
    QTRY_VERIFY2(!m_historyList->hasActiveFocus(),
                 "arm A precondition: the list must NOT hold focus");
    // One more real history step through the engine.
    fillCanvas(0.0, 1.0, 0.0);
    QTRY_VERIFY2(contentY() > 0.0,
                 "the view never moved when history changed (Q-D22's defect)");
    QVERIFY2(cursorRowIsInViewport(),
             "history changed but the cursor row is still outside the viewport");

    // ---- ARM B: FOCUSED — the view is suppressed and must NOT move. ----
    // A user who tabbed into the journal is browsing deliberately.
    parkAtTop();
    QTRY_COMPARE(contentY(), 0.0);
    QQuickItem *row0 = rowItem(0);
    QVERIFY2(row0, "row 0 delegate not realized after parking at the top");
    row0->forceActiveFocus();
    QTRY_VERIFY2(m_historyList->hasActiveFocus(),
                 "focusing a delegate did not give the ListView active focus "
                 "(it is a focus scope; if this fails the predicate is wrong)");
    fillCanvas(1.0, 0.0, 1.0);
    QTRY_VERIFY(rows().size() > added + 1);
    QTest::qWait(120); // give any errant scroll a chance to happen
    QCOMPARE(contentY(), 0.0);
    // AND THE NEGATIVE HALF, which is what makes arm A's assert meaningful:
    // with the view held at the top, the cursor is genuinely off-screen, so
    // the very predicate that passed above now FAILS.
    QVERIFY2(!cursorRowIsInViewport(),
             "arm B: the cursor is somehow in view while parked at the top — "
             "cursorRowIsInViewport() cannot discriminate and arm A is vacuous");
}

// B-06 (Q-D24), the shell half of the RULED engine projection. Every
// filter used to write a history row reading "Apply Filter", so two
// different filters were the same row forever — undoing past one meant
// choosing between identical rows, and a screen reader heard the same
// sentence twice. A-18 puts the filter's own id on the row; this asserts
// the panel reads it.
void TstHistoryPanel::aFilterRowNamesTheFilterNotTheGenericKind()
{
    // The two catalog strings this case discriminates between MUST differ,
    // or every assertion below passes for the wrong reason.
    const QString filterName =
        m_i18n->catalog().tr(QStringLiteral("filter.gaussian_blur"));
    const QString genericName =
        m_i18n->catalog().tr(QStringLiteral("history.filter.apply"));
    QVERIFY2(!filterName.isEmpty() && !filterName.contains(QLatin1Char('.')),
             "filter.gaussian_blur key-echoed — the catalog lacks it and this "
             "case would compare an id against an id");
    QVERIFY2(filterName != genericName,
             "the filter's name and the generic label are the same string — "
             "this case cannot tell them apart and proves nothing");

    QJsonObject blur;
    blur.insert(QLatin1String("radius"), 4.0);
    QVariantMap target;
    target.insert(QStringLiteral("layerId"),
                  m_controller->layers().first().toMap().value(
                      QStringLiteral("layerId")));
    const QVariantMap r = m_controller->applyCommand(
        QStringLiteral("filter.apply"), target,
        buildFilterApplyParams(QStringLiteral("filter.gaussian_blur"), blur)
            .toVariantMap(),
        QString());
    QVERIFY2(r.value(QStringLiteral("ok")).toBool(),
             qPrintable(QStringLiteral("filter.apply was refused: %1")
                            .arg(r.value(QStringLiteral("code")).toString())));

    // (1) THE PROJECTION: A-18's field reaches the shell's row model.
    QTRY_VERIFY(rows().size() > 0);
    const QVariantMap last = rows().last().toMap();
    QCOMPARE(last.value(QStringLiteral("kind")).toString(),
             QStringLiteral("filter.apply"));
    QCOMPARE(last.value(QStringLiteral("filterId")).toString(),
             QStringLiteral("filter.gaussian_blur"));

    // (2) THE PANEL CONSUMES IT: the row an assistive technology reads
    // speaks the FILTER's translated name, not the generic kind label.
    QQuickItem *row = nullptr;
    QTRY_VERIFY((row = rowItem(rows().size() - 1)) != nullptr);
    QQmlExpression expr(qmlContext(row), row, QStringLiteral("Accessible.name"));
    bool undef = false;
    const QString spoken = expr.evaluate(&undef).toString();
    QVERIFY2(!expr.hasError(), "Accessible.name read failed");
    QCOMPARE(spoken, filterName);
    QVERIFY2(spoken != genericName,
             "the filter row still speaks the generic \"Apply Filter\" — "
             "A-18's subject reached the model but not the label");

    // (3) A NON-FILTER row is UNTOUCHED: the fallback still names the kind,
    // so this change cannot have replaced one collapse with another.
    const QVariantMap first = rows().first().toMap();
    QVERIFY2(first.value(QStringLiteral("filterId")).toString().isEmpty(),
             "a non-filter row carries a filterId — the engine skips the "
             "field on 101 of 102 kinds and the shell should see none");
}

void TstHistoryPanel::unchangedAutoRefitIsSilentWhenGated()
{
    // W-18 (Windows port): on Windows a history-row click is followed by a
    // layout reflow that re-fits the canvas to the SAME zoom, and the shell
    // announced "Zoom 230 percent" for a level that had not moved. The gate
    // (refitSilentWhenUnchanged) is on by default on Windows only; switch it
    // on here so the behaviour is proven on every platform.
    QQuickItem *canvas =
        m_window->findChild<QQuickItem *>(QStringLiteral("canvasView"));
    QVERIFY2(canvas, "canvasView not found");
    const QVariant gateBefore = canvas->property("refitSilentWhenUnchanged");
    QVERIFY2(gateBefore.isValid(), "refitSilentWhenUnchanged property missing");
    canvas->setProperty("refitSilentWhenUnchanged", true);
    const QSize sizeBefore = m_window->size();
    auto zoomEntriesSince = [this](int from) {
        int n = 0;
        const QVariantList log = m_announcer->announcementLog();
        for (int i = from; i < log.size(); ++i)
            if (log.at(i).toMap().value(QStringLiteral("source")).toString()
                == QLatin1String("zoom"))
                ++n;
        return n;
    };
    QTest::qWait(650); // drain the 500 ms window from init()

    // (1) HEIGHT loss: the fit is width-limited, so the zoom does not move
    // and the automatic re-fit must stay silent.
    const int zoomBefore = canvas->property("zoomPercent").toInt();
    int from = m_announcer->announcementLog().size();
    m_window->setHeight(m_window->height() - 24);
    QTest::qWait(650);
    QCOMPARE(canvas->property("zoomPercent").toInt(), zoomBefore);
    QCOMPARE(zoomEntriesSince(from), 0);

    // (2) WIDTH loss changes the fit, so the new level MUST still be spoken
    // -- the gate silences repeats only, and this arms the assert above.
    from = m_announcer->announcementLog().size();
    m_window->setWidth(m_window->width() - 200);
    QTRY_VERIFY(canvas->property("zoomPercent").toInt() != zoomBefore);
    QTRY_COMPARE(zoomEntriesSince(from), 1);

    m_window->resize(sizeBefore);
    canvas->setProperty("refitSilentWhenUnchanged", gateBefore);
}

int main(int argc, char *argv[])
{
    if (qgetenv("QT_QPA_PLATFORM").isEmpty())
        qputenv("QT_QPA_PLATFORM", "offscreen");
    QQuickStyle::setStyle(QStringLiteral("Basic"));
    QGuiApplication app(argc, argv);
    installClickFocusParity(app); // mirrors shell/src/main.cpp (Windows only)
    TstHistoryPanel tc;
    QTEST_SET_MAIN_SOURCE_PATH
    return QTest::qExec(&tc, argc, argv);
}
#include "tst_history_panel.moc"
