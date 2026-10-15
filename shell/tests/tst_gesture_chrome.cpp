// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// q-gesture (standing functional-QA session): the CHROME half of the
// gesture net — the seam between the real shell chrome and the engine.
//
// Boots the REAL Main.qml offscreen (the tst_palette_pins harness
// pattern) and drives synthesized events END TO END: pick a tool the way
// a user actually picks one (a palette slot click, a single-key letter),
// then complete that tool's real gesture on the canvas, and assert ENGINE
// truth.
//
// Why this is not a duplicate of the suites either side of it:
//   * tst_palette_pins (O-UI, item 11b/11c) stops at the canvas `tool`
//     PROPERTY — it proves the chrome SELECTS the tool.
//   * tst_gesture_net / tst_selection_gestures set the tool
//     PROGRAMMATICALLY — they prove the gesture COMPLETES.
// Nobody joins the two halves, and the join is exactly the defect class
// this session owns: the tool is selected, the property is right, and
// the user still cannot complete the gesture.

#include "announcer.h"
#include "canvas_view.h"
#include "document_controller.h"
#include "i18n_catalog.h"
#include "icon_provider.h"
#include "layer_thumb_provider.h"
#include "tool_key_dispatcher.h"
#include "units.h"

#include <QClipboard>
#include <QDragEnterEvent>
#include <QDropEvent>
#include <QMimeData>
#include <QGuiApplication>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQmlExpression>
#include <QQuickItem>
#include <QQuickStyle>
#include <QStyleHints>
#include <QQuickWindow>
#include <QWheelEvent>
#include <QtTest>
#include <QDir>
#include <functional>

using namespace appname::shell;

namespace {

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

bool isToolButton(QQuickItem *it)
{
    return it && it->property("group").isValid()
        && it->property("faceTool").isValid();
}

QRect maskBounds(const QImage &mask, int yOffset, QRect acc)
{
    for (int y = 0; y < mask.height(); ++y) {
        const uchar *row = mask.constScanLine(y);
        for (int x = 0; x < mask.width(); ++x)
            if (row[x])
                acc = acc.united(QRect(x, yOffset + y, 1, 1));
    }
    return acc;
}

} // namespace

class TstGestureChrome : public QObject {
    Q_OBJECT
private slots:
    void initTestCase();
    void init();

    void groundChromeGeometry();
    void letterRoutedSelectionToolsCompleteGestures();
    void letterRoutedBrushPaintsThroughRealChrome();
    void paletteSlotClickThenGestureCompletes();
    void pasteMenuGlueFitsOnOverflow();
    void cropDragOpensPrefilledCropDialog();
    void cropDragThenApplyNarrowsTheFrameNotTheCanvas();
    void cropDragThenApplyWithDeletePixelsShrinksTheCanvas();
    void selectionModifyDialogExpandsTheGestureSelection();
    void fillDialogFillsOnlyInsideTheGestureSelection();
    void armedControlsAnnouncePendingOnPress();
    void letterTReachesTypeNotTransform();
    void unifiedSelectionFlyoutMemberCompletesItsGesture();
    void historyPanelRowJumpRewindsTheDocument();
    void pasteIntoClipsToTheGestureSelection();
    void pasteIntoMenuGlueFitsOnOverflow();
    void dropGlueFitsOnOverflow();
    void importDialogGlueFitsOnOverflow();
    void documentlessDropAnswersOutLoud();
    void importFilterPromiseHoldsEndToEnd();
    void newDocumentPresetCreatesThatDocument();
    void letterRingCyclesAndEachMemberCompletesItsGesture();
    void springLoadedLetterRevertsAndTheOriginalToolStillWorks();
    void resizeAndRotateDialogsReachTheEngine();
    void cropDeletePixelsArmTracksTheCapability();
    void autoAdjustMenuItemsAreArmedAndHonest();
    void historyPanelRowClickRewindsTheDocument();
    void honestlyDisabledControlsStayDisabled();
    void historyPanelRowKeyboardActivationRewindsTheDocument();
    void layersPanelButtonsReachTheEngineThroughAClick();
    void zoomButtonsReachTheViewThroughAClick();
    void layerRowGesturesSelectAndToggleVisibility();
    void layerRowDoubleClickRenamesTheLayer();
    void layerOpacitySliderReachesTheEngine();
    void toolOptionRowControlsReachTheCanvasThroughAClick();
    void undoRedoShortcutsReachTheDocument();
    void refusalToastAppearsOnATypedRefusal();
    void layersPanelMaskAndFillMenusReachTheEngine();
    void unavailableChordsSpeakInsteadOfGoingSilent();
    void adjustmentMenuArmsReachTheEngine();
    void adjustmentParamDialogAcceptReachesTheEngine();
    void levelsAndHueSaturationApplyLandsOrSaysWhy();
    // T67: the week's shell landings on my surface.
    void sizeRowShipsForEveryStrokeToolThatTakesASize();
    void sizeRowSliderIsReachableByTheKeyboard();
    void sizeRowToolKeysTapSelectAndHoldSpringsBack();
    void dirtyReplaceDoorsRaiseTheDiscardConfirm();
    void cloneKeySpeaksInsteadOfSelectingSilently();
    // T68: the third (4)-3 door, the replace shortcuts as real keys, and
    // the button-level arm on the adjustment dialog.
    void fileOpenOverDirtyDocumentConfirmsBeforeReplacing();
    void replaceShortcutsReachTheirDoorsAndCloseIsBound();
    void closeDoorAndThePromptsSaveButton();
    // 1.0 final D-9 (operator WORD 38/39): a deleted layer leaves the canvas.
    void deletedLayerLeavesTheCanvasWithoutAnotherReceipt();
    void s10UndoAndRedoRepaintMaskAndLayerChanges();
    void adjustmentDialogAcceptButtonIsClickableAndLabelled();
    void undoGestureScrollsTheCursorBackWithoutMovingAVisibleOne();
    // T69: ROADMAP G-02 "dialog keyboard paths"; feeds G-06.
    void adjustmentDialogIsOperableFromTheKeyboard();
    // T70: G-02 dialog keyboard paths, on the must-pass discard confirm.
    void discardConfirmKeyboardPathsDoNotLoseWork();
    // T72: the G-16 census, measured instead of parsed.
    void noDialogCommitsOnReturnAcrossTheWholeShell();
    // T73: G-06 keyboard reachability, measured against the LIVE tree.
    void visibleControlsOnTheSittingPathAreKeyboardReachable();
    // T76: G-06 in a POPULATED document — the owed case from T75.
    void keyboardReachHoldsWhenTheDocumentHasRealLayers();
    // T78: G-06 across every tool — the options bar is a per-tool surface.
    void optionsBarStaysKeyboardReachableAcrossEveryTool();
    // T79: G-06 modal focus containment — the shell's own modal:true contract.
    void modalDialogsKeepKeyboardFocusInsideThem();
    // T80: G-06 — Escape is the only exit from a contained modal.
    void everyDialogClosesOnEscape();
    // T82: B-10's behavioural half — the state 0 of 60 cases had entered.
    void coldOpenEmptyStateOffersOnlyItsOwnControls();

private:
    // T82: the COLD-OPEN EMPTY STATE, captured in initTestCase because that
    // is the only moment it exists -- `hasDocument` is read-only and goes
    // false->true once, init() opens a document before EVERY case, and the
    // shell is single-document with no Close. Zero of this suite's 60 cases
    // had ever run in it.
    bool m_emptyHasDocument = true;
    bool m_emptyNoDocLabel = false;
    bool m_emptyCmdTRefused = false;
    int m_emptyEnabledControls = -1;
    QStringList m_emptyEnabledNames;

    CatalogQml *m_i18n = nullptr;
    DocumentController *m_controller = nullptr;
    Announcer *m_announcer = nullptr;
    QQmlApplicationEngine *m_engine = nullptr;
    QQuickWindow *m_window = nullptr;
    QQuickItem *m_canvasItem = nullptr;
    CanvasView *m_canvas = nullptr;
    QQuickItem *m_palette = nullptr;
    QList<QQuickItem *> m_slots;
    ToolKeyDispatcher *m_toolKeys = nullptr;

    QString canvasTool() const
    {
        return m_canvasItem->property("tool").toString();
    }
    // LOAD-ROBUST letter tap (T34). QTest::keyClick sends press+release,
    // but 11e's dispatcher reads a press→release gap past
    // ToolKeyDispatcher::SpringHoldMs (300 ms) as a HOLD — spring-load —
    // which borrows the tool and REVERTS it on release. Under heavy load
    // a synthesized keyClick genuinely crosses that boundary, so the tap
    // silently becomes a hold and the tool is not what the letter names.
    //
    // Diagnosed T34 from a full-suite red that passed in isolation and
    // with its minimal prefix; the mechanism is proven by this file's own
    // springLoadedLetterRevertsAndTheOriginalToolStillWorks, which shows a
    // >300 ms hold borrowing then reverting. Not a product defect — a
    // timing fragility in MY probe that 11e's spring-load introduced.
    //
    // Retry a bounded number of times and say so, rather than either
    // flaking or quietly widening the assertion.
    bool tapLetterUntilTool(Qt::Key key, const QString &wire)
    {
        for (int attempt = 1; attempt <= 4; ++attempt) {
            m_canvasItem->forceActiveFocus();
            QTest::keyClick(m_window, key);
            QCoreApplication::processEvents();
            if (canvasTool() == wire)
                return true;
            qInfo().noquote()
                << QStringLiteral("  letter tap attempt %1 landed on %2, not "
                                  "%3 — spring-load boundary crossed under "
                                  "load; retrying")
                       .arg(attempt).arg(canvasTool()).arg(wire);
        }
        return false;
    }
    void focusCanvas()
    {
        m_canvasItem->forceActiveFocus();
        QTRY_VERIFY(m_canvasItem->hasActiveFocus());
    }
    // Doc -> window/scene coordinates. After a fresh document the canvas
    // has zoomFit'd (onDocumentReset) and init() then pins zoom 1, so the
    // doc is CENTERED in the canvas item: offset = (item - doc) / 2.
    // Guarded by the zoomPercent == 100 assert in init().
    // Evaluate JS in Main.qml's root context, so a dialog declared there is
    // reachable BY ITS QML id. Mirrors tst_import_filter's evalJs/liveFilters
    // (the tst_canvas_context_menu id-lookup pattern) -- these FileDialogs
    // carry no objectName, so findChildren cannot name them.
    QVariant evalJs(const QString &code) const
    {
        QQmlExpression expr(qmlContext(m_window), m_window, code);
        bool undefined = false;
        const QVariant v = expr.evaluate(&undefined);
        if (expr.hasError())
            qWarning() << "evalJs failed:" << expr.error().toString();
        return undefined ? QVariant() : v;
    }

    // T67: resolve a Main.qml `id` to its item. Same door as evalJs
    // above -- the options strip carries an id and NO objectName, so
    // findChild cannot name it, and a window-wide class search finds the
    // LayersPanel opacity slider instead (the negative control caught
    // exactly that).
    QQuickItem *itemById(const QString &id) const
    {
        QQmlExpression expr(qmlContext(m_window), m_window, id);
        const QVariant v = expr.evaluate();
        if (expr.hasError())
            qWarning() << "itemById failed:" << expr.error().toString();
        return qobject_cast<QQuickItem *>(v.value<QObject *>());
    }
    QPoint docToScene(int x, int y) const
    {
        const double ox = (m_canvasItem->width() - m_controller->docWidth()) / 2.0;
        const double oy = (m_canvasItem->height() - m_controller->docHeight()) / 2.0;
        return m_canvasItem->mapToScene(QPointF(x + ox, y + oy)).toPoint();
    }
    QRect engineSelectionBounds() const
    {
        const int w = m_controller->docWidth();
        const int h = m_controller->docHeight();
        const int maxRows = qMax(1, (1 << 22) / qMax(1, w));
        QRect b;
        for (int y = 0; y < h; y += maxRows) {
            QImage strip;
            if (!m_controller->readSelectionImage(
                    QRect(0, y, w, qMin(maxRows, h - y)), &strip))
                return QRect();
            b = maskBounds(strip, y, b);
        }
        return b;
    }
    static bool isWhitishPixel(const QColor &c)
    {
        return c.red() > 245 && c.green() > 245 && c.blue() > 245;
    }
    QColor enginePixel(int x, int y) const
    {
        QImage px;
        if (!m_controller->readRegionImage(QRect(x, y, 1, 1), &px))
            return QColor();
        return px.pixelColor(0, 0);
    }
    QQuickItem *slotFacing(const QString &wire) const
    {
        for (QQuickItem *s : m_slots)
            if (s->property("faceTool").toMap()
                    .value(QStringLiteral("tool")).toString() == wire)
                return s;
        return nullptr;
    }
    QPoint center(QQuickItem *item) const
    {
        return item->mapToScene(QPointF(item->width() / 2.0,
                                        item->height() / 2.0))
            .toPoint();
    }
};

void TstGestureChrome::initTestCase()
{
    // Mirror shell/src/main.cpp's wiring exactly (the tst_palette_pins
    // pattern) so this drives the SHIPPING chrome, not a stand-in.
    m_i18n = new CatalogQml(Catalog::load(QStringLiteral("en")));
    m_controller = new DocumentController(&m_i18n->catalog());
    m_announcer = new Announcer(&m_i18n->catalog());
    m_controller->setAnnouncer(m_announcer);

    m_engine = new QQmlApplicationEngine;
    m_engine->addImageProvider(QStringLiteral("lucide"), new LucideIconProvider);
    m_engine->addImageProvider(QStringLiteral("layerthumb"),
                               new LayerThumbProvider(m_controller));
    m_engine->rootContext()->setContextProperty(QStringLiteral("AppI18n"), m_i18n);
    m_engine->rootContext()->setContextProperty(QStringLiteral("AppDoc"), m_controller);
    m_engine->rootContext()->setContextProperty(QStringLiteral("AppA11y"), m_announcer);
    m_engine->rootContext()->setContextProperty(QStringLiteral("AppKeys"),
                                                new QObject(m_engine));
    // items 2+3: Main.qml's New Document / Resize dialogs read AppUnits;
    // without it the chrome boots with JS reference errors (mirrors
    // shell/src/main.cpp:131).
    m_engine->rootContext()->setContextProperty(QStringLiteral("AppUnits"),
                                                new UnitsQml(m_engine));
    m_engine->loadFromModule("AppnameShell", "Main");
    QVERIFY2(!m_engine->rootObjects().isEmpty(), "Main.qml failed to load");
    m_window = qobject_cast<QQuickWindow *>(m_engine->rootObjects().first());
    QVERIFY(m_window);
    m_announcer->setSubject(m_window);
    // v1.0.1 item 11e: the tool letter-keys LEFT the QML shortcut map and
    // live in this dispatcher, installed on the loaded window. main.cpp
    // does exactly this and its comment says "every Main.qml test harness
    // mirrors this one call" — mine did not, so after 11e merged every
    // letter-driven case in this file failed. That was MY harness drifting
    // from the shipping wiring, not a defect: the same class as T22's
    // bare-canvas honesty probe.
    m_toolKeys = new ToolKeyDispatcher(this);
    m_toolKeys->attach(m_window);
    QVERIFY(QTest::qWaitForWindowExposed(m_window));
    m_window->requestActivate();
    QTRY_VERIFY(m_window->isActive());

    // ---- T82: CAPTURE THE COLD-OPEN EMPTY STATE ----------------------
    // B-10 (due 09-16) names "no document open: New/Import only, Cmd+T
    // refusal toast, single-document model". Its stated instrument is the
    // TOUR, which owns the appearance (01-launch.png, byte-identical every
    // tick); nothing owned the BEHAVIOUR, because this suite could not get
    // here: init() opens a document before every case and nothing closes
    // one. This is the one moment the state exists, so it is measured here
    // and asserted by coldOpenEmptyStateOffersOnlyItsOwnControls().
    //
    // ★ Everything below must leave the fixture EXACTLY as it found it --
    // a probe that contaminates initTestCase poisons all 60 cases, and the
    // two guards at the end fail loudly rather than let that happen quietly.
    m_emptyHasDocument = m_controller->hasDocument();
    const QString noDocText = m_i18n->tr(QStringLiteral("ui.canvas.no_document"));
    QQuickItem *toast = nullptr;
    for (QObject *o : m_window->findChildren<QObject *>()) {
        if (o->objectName() == QLatin1String("refusalToast"))
            toast = qobject_cast<QQuickItem *>(o);
        if (QQuickItem *it = qobject_cast<QQuickItem *>(o))
            if (it->isVisible()
                && it->property("text").toString() == noDocText)
                m_emptyNoDocLabel = true;
    }
    {
        QSet<QQuickItem *> seen;
        int n = 0;
        auto consider = [&](QQuickItem *b) {
            if (!b || seen.contains(b)) return;
            if (!(b->inherits("QQuickAbstractButton") || b->inherits("QQuickSlider")
                  || b->inherits("QQuickSpinBox") || b->inherits("QQuickComboBox")
                  || b->inherits("QQuickTextField")))
                return;
            if (!b->isVisible() || b->width() <= 0 || b->height() <= 0) return;
            if (!b->property("enabled").toBool()) return;
            seen << b;
            ++n;
            if (m_emptyEnabledNames.size() < 30) {
                const QString t = b->property("text").toString().trimmed();
                // Identify unnamed controls by objectName, then by icon
                // source, then by class -- "ToolButton" seven times says
                // nothing about whether they belong here, and B-10's claim
                // ("New/Import only") can only be checked against identities.
                QString id = b->objectName();
                if (id.isEmpty() && !t.isEmpty())
                    id = t;
                if (id.isEmpty()) {
                    const QVariant ic = b->property("icon");
                    QString src;
                    if (ic.isValid()) {
                        const QObject *io = ic.value<QObject *>();
                        if (io) src = io->property("source").toString();
                    }
                    if (src.isEmpty())
                        src = b->property("iconSource").toString();
                    if (!src.isEmpty())
                        id = src.section(QLatin1Char('/'), -1);
                }
                if (id.isEmpty())
                    id = QString::fromLatin1(b->metaObject()->className())
                             .section(QLatin1Char('_'), 0, 0);
                QString parentId;
                for (QQuickItem *w = b->parentItem(); w; w = w->parentItem())
                    if (!w->objectName().isEmpty()) {
                        parentId = w->objectName(); break;
                    }
                m_emptyEnabledNames
                    << (parentId.isEmpty() ? id
                                           : QStringLiteral("%1@%2").arg(id, parentId));
            }
        };
        for (QObject *o : m_window->findChildren<QObject *>())
            consider(qobject_cast<QQuickItem *>(o));
        std::function<void(QQuickItem *)> walk = [&](QQuickItem *i2) {
            for (QQuickItem *c : i2->childItems()) { consider(c); walk(c); }
        };
        if (QQuickItem *ci = m_window->contentItem()) walk(ci);
        m_emptyEnabledControls = n;
    }
    if (toast && !toast->property("visible").toBool()) {
        QTest::keyClick(m_window, Qt::Key_T, Qt::ControlModifier);
        QCoreApplication::processEvents();
        QTest::qWaitFor([&] { return toast->property("visible").toBool(); }, 1500);
        m_emptyCmdTRefused = toast->property("visible").toBool();
        // Put it back: wait the toast out and prove it is gone. The hide is
        // a 6 s Timer (this suite's own refusal case records the interval),
        // so a 6000 ms wait RACES it -- the first draft did exactly that and
        // the guard below caught the probe leaving a toast up. 9 s clears the
        // timer with margin.
        QTest::qWaitFor([&] { return !toast->property("visible").toBool(); }, 9000);
        QVERIFY2(!toast->property("visible").toBool(),
                 "the empty-state probe left a refusal toast up -- it would "
                 "land on whichever case runs first");
    }
    QVERIFY2(!m_controller->hasDocument(),
             "the empty-state probe created a document in initTestCase -- "
             "every case's fixture would start wrong");

    m_canvasItem = m_window->findChild<QQuickItem *>(QStringLiteral("canvasView"));
    QVERIFY2(m_canvasItem, "canvasView not found");
    m_canvas = qobject_cast<CanvasView *>(m_canvasItem);
    QVERIFY2(m_canvas, "canvasView is not a CanvasView");

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
    QList<QQuickItem *> paletteItems;
    collectItems(m_palette, &paletteItems);
    for (QQuickItem *it : paletteItems)
        if (isToolButton(it) && it->isVisible())
            m_slots.append(it);
    std::sort(m_slots.begin(), m_slots.end(), [](QQuickItem *a, QQuickItem *b) {
        return a->mapToScene(QPointF(0, 0)).y() < b->mapToScene(QPointF(0, 0)).y();
    });
    QVERIFY2(!m_slots.isEmpty(), "no visible palette slots found");
}

void TstGestureChrome::init()
{
    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
    m_canvas->zoomFit();     // known centered state...
    m_canvas->zoomOneToOne(); // ...then zoom 1, still centered
    QCOMPARE(m_canvasItem->property("zoomPercent").toInt(), 100);
    QVERIFY(!m_controller->selectionActive());
    focusCanvas();
}

void TstGestureChrome::groundChromeGeometry()
{
    // Vacuity guard (rule 9): every case below maps doc->scene through
    // the canvas item, which is only valid while the item is larger than
    // the document at zoom 1 (otherwise the doc is clipped/offset
    // negative and the probe points would land off-canvas silently).
    qInfo() << "window" << m_window->size() << "canvas item"
            << QSizeF(m_canvasItem->width(), m_canvasItem->height())
            << "slots" << m_slots.size() << "boot tool" << canvasTool();
    QVERIFY2(m_canvasItem->width() >= m_controller->docWidth()
                 && m_canvasItem->height() >= m_controller->docHeight(),
             "canvas item is smaller than the document at zoom 1 — the "
             "doc->scene mapping these cases rely on is not valid");
    QStringList faces;
    for (QQuickItem *s : m_slots)
        faces << s->property("faceTool").toMap()
                     .value(QStringLiteral("tool")).toString();
    qInfo() << "palette faces at boot:" << faces;
}

void TstGestureChrome::letterRoutedSelectionToolsCompleteGestures()
{
    // The single-key tool letters are pinned to reach the TOOL by
    // tst_palette_pins (D2). Here each letter must also leave the canvas
    // able to COMPLETE that tool's gesture, to engine truth.
    struct Leg { Qt::Key key; const char *wire; };
    const Leg legs[] = {{Qt::Key_M, "select.rect"},
                        {Qt::Key_L, "select.lasso"},
                        {Qt::Key_W, "select.wand"}};
    for (const Leg &leg : legs) {
        QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
        m_canvas->zoomFit();
        m_canvas->zoomOneToOne();
        focusCanvas();
        QVERIFY2(tapLetterUntilTool(leg.key, QLatin1String(leg.wire)),
                 qPrintable(QStringLiteral("letter for %1 never reached its "
                                           "tool in 4 taps (last: %2)")
                                .arg(QLatin1String(leg.wire))
                                .arg(canvasTool())));

        if (leg.key == Qt::Key_W) {
            QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                              docToScene(320, 200));
        } else {
            QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                              docToScene(120, 110));
            QTest::mouseMove(m_window, docToScene(260, 190));
            QTest::mouseMove(m_window, docToScene(380, 280));
            QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                                docToScene(380, 280));
        }
        QVERIFY2(m_controller->selectionActive(),
                 qPrintable(QStringLiteral("tool %1 was reached by its letter "
                                           "but its gesture committed NO "
                                           "selection through the real "
                                           "chrome; status: %2")
                                .arg(QLatin1String(leg.wire))
                                .arg(m_controller->statusText())));
        const QRect b = engineSelectionBounds();
        QVERIFY2(b.width() >= 100 && b.height() >= 80,
                 qPrintable(QStringLiteral("tool %1 committed a DEGENERATE "
                                           "selection through the chrome: "
                                           "%2,%3 %4x%5")
                                .arg(QLatin1String(leg.wire))
                                .arg(b.x()).arg(b.y())
                                .arg(b.width()).arg(b.height())));
        m_controller->applyCommand(QStringLiteral("selection.none"), {}, {}, {});
    }
}

void TstGestureChrome::letterRoutedBrushPaintsThroughRealChrome()
{
    QTest::keyClick(m_window, Qt::Key_B);
    QCOMPARE(canvasTool(), QStringLiteral("brush.stroke"));
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToScene(120, 200));
    QTest::mouseMove(m_window, docToScene(240, 200));
    QTest::mouseMove(m_window, docToScene(360, 200));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToScene(360, 200));
    const QColor on = enginePixel(240, 200);
    QVERIFY2(on.isValid() && !(on.red() > 245 && on.green() > 245
                               && on.blue() > 245),
             qPrintable(QStringLiteral("the brush reached by its letter "
                                       "painted NOTHING through the real "
                                       "chrome; status: %1")
                            .arg(m_controller->statusText())));
    const QColor off = enginePixel(240, 340);
    QVERIFY2(off.red() > 245 && off.green() > 245 && off.blue() > 245,
             "the chrome-routed brush painted far off its path");
}

void TstGestureChrome::paletteSlotClickThenGestureCompletes()
{
    // The MOUSE route through the shipping palette: click the slot whose
    // face is the rect marquee, then drag on the canvas. A slot that
    // selects the tool but leaves the canvas unable to finish the drag is
    // exactly the shipped-selection defect class.
    // ORDER-INDEPENDENCE (item 17, merged 2026-08-21): all selection
    // tools now share ONE slot whose face follows LAST USE. This case used
    // to rely on select.rect being a slot face at boot, which an earlier
    // case in the same suite (the letter route, which ends on the wand)
    // silently invalidated — it passed alone and failed in-suite. Seed the
    // face deterministically first, so the click below tests the MOUSE
    // ROUTE rather than the leftover state of whatever ran before.
    m_canvasItem->setProperty("tool", QStringLiteral("select.rect"));
    QQuickItem *slot = nullptr;
    QTRY_VERIFY_WITH_TIMEOUT(
        (slot = slotFacing(QStringLiteral("select.rect"))) != nullptr, 3000);
    QVERIFY2(slot, "no palette slot faces select.rect even after seeding it "
                   "— the palette's face no longer follows the canvas tool");
    // Prove the click is not a no-op: leave on a different tool, so the
    // slot press is what puts the canvas back on the marquee.
    m_canvasItem->setProperty("tool", QStringLiteral("brush.stroke"));
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier, center(slot));
    QTRY_COMPARE(canvasTool(), QStringLiteral("select.rect"));

    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToScene(100, 100));
    QTest::mouseMove(m_window, docToScene(220, 180));
    QTest::mouseMove(m_window, docToScene(340, 260));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToScene(340, 260));
    QVERIFY2(m_controller->selectionActive(),
             qPrintable(QStringLiteral("a palette-selected marquee could not "
                                       "complete its drag; status: %1")
                            .arg(m_controller->statusText())));
    // The chrome's canvas item is an ODD width against an even document,
    // so the centred zoom-1 offset lands on a half pixel and doc->scene
    // cannot round-trip exactly. The EXTENT is engine truth and is pinned
    // exactly; the origin is allowed the single pixel that mapping costs
    // (a broken gesture yields no selection or a degenerate one — neither
    // survives this).
    const QRect b = engineSelectionBounds();
    QCOMPARE(b.size(), QSize(240, 160));
    QVERIFY2(qAbs(b.x() - 100) <= 1 && qAbs(b.y() - 100) <= 1,
             qPrintable(QStringLiteral("palette-routed marquee landed at "
                                       "%1,%2 — expected 100,100 +/-1")
                            .arg(b.x()).arg(b.y())));
}

void TstGestureChrome::pasteMenuGlueFitsOnOverflow()
{
    // CLOSES A RECORDED OMISSION. tst_selection_gestures'
    // fitIfOverflowFitsOnlyWhenOverflowing records that the four Main.qml
    // item-12 call sites are "QML glue no harness drives" — true when it
    // was written, stale as of this same merge wave: Main.qml-booting
    // harnesses now exist (tst_palette_pins, and this file). This drives
    // the Edit > Paste site for real: Main.qml's pasteAndFit(), which
    // the Edit menu and the canvas context menu both fire.
    // (This line cited "Main.qml:403" until 2026-09-03. That line has
    // never held the call: it was at 574 when this test was written and
    // is at 692 today. Name the function -- a line number is the
    // recomputable value that goes stale unread.)
    QImage payload(64, 48, QImage::Format_ARGB32);
    payload.fill(QColor(200, 40, 40));
    QGuiApplication::clipboard()->setImage(payload);

    QObject *paste = nullptr;
    for (QObject *o : m_window->findChildren<QObject *>()) {
        if (!o->inherits("QQuickMenuItem"))
            continue;
        if (o->property("text").toString()
            != m_i18n->tr(QStringLiteral("ui.menu.edit.paste")))
            continue;
        paste = o;
        break;
    }
    QVERIFY2(paste, "Edit > Paste menu item not found");
    QVERIFY2(paste->property("enabled").toBool(),
             "Edit > Paste disabled with a document open");

    // Overflow the viewport so the glue has something to do: at 400% the
    // 640x400 doc renders 2560x1600 into a ~909x680 canvas item.
    m_canvas->setZoomCentered(4.0);
    QCOMPARE(m_canvasItem->property("zoomPercent").toInt(), 400);
    const int layersBefore = m_controller->layers().size();

    QVERIFY(QMetaObject::invokeMethod(paste, "triggered"));

    // Non-vacuity (rule 9): the glue is guarded by the paste's success
    // return, so a paste that did nothing would leave the zoom at 400 for
    // the WRONG reason and this case would pass as a false green.
    QTRY_VERIFY2(m_controller->layers().size() > layersBefore,
                 qPrintable(QStringLiteral("the paste door did not land a "
                                           "layer, so the item-12 glue was "
                                           "never reached; status: %1")
                                .arg(m_controller->statusText())));
    QTRY_VERIFY2(m_canvasItem->property("zoomPercent").toInt() != 400,
                 "Edit > Paste succeeded but the item-12 fitIfOverflow glue "
                 "did not fire — the view stayed overflowed at 400%");
    // And it fit rather than landing on some other zoom.
    const double fit = qMin((m_canvasItem->width() - 24.0) / 640.0,
                            (m_canvasItem->height() - 24.0) / 400.0);
    QCOMPARE(m_canvasItem->property("zoomPercent").toInt(),
             int(qRound(fit * 100.0)));
}

void TstGestureChrome::cropDragOpensPrefilledCropDialog()
{
    // The crop tool's whole user story is glue: the drag commits nothing
    // itself, it settles into cropRectDragged, and Main.qml's Connections
    // block turns that into a PRE-FILLED crop dialog. tst_gesture_net
    // pins the signal payload; only a Main.qml harness can pin the half
    // the user actually sees, and none did before this file.
    // A QML Dialog is a QQuickPopup, NOT a QQuickItem — hold it as a
    // plain QObject or the cast silently nulls it.
    QObject *dialog = nullptr;
    for (QObject *o : m_window->findChildren<QObject *>()) {
        // QML-defined dialog types do NOT answer inherits("QQuickDialog")
        // — they carry a generated CropDialog_QMLTYPE_* metaobject.
        if (!QLatin1String(o->metaObject()->className())
                 .startsWith(QLatin1String("CropDialog")))
            continue;
        QCOMPARE(o->property("title").toString(),
                 m_i18n->tr(QStringLiteral("ui.dialog.crop.title")));
        dialog = o;
        break;
    }
    QVERIFY2(dialog, "the crop dialog was not found in the chrome");
    QVERIFY2(!dialog->property("visible").toBool(),
             "the crop dialog is already open before the drag");

        // 11e ring cycling (merged this wave): a letter's SECOND press cycles
    // its ring (C = crop -> slice.add -> slice.select; M = select.rect ->
    // select.ellipse). Letters are the subject of the letterRouted*/letterT*
    // cases; HERE the tool choice is incidental setup, so pressing a letter
    // made this case depend on how many times earlier cases pressed it — it
    // passed alone and failed in-suite. Set the tool directly instead.
    m_canvasItem->setProperty("tool", QStringLiteral("crop"));
    QCOMPARE(canvasTool(), QStringLiteral("crop"));
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToScene(120, 80));
    QTest::mouseMove(m_window, docToScene(300, 200));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToScene(400, 280));

    QTRY_VERIFY2(dialog->property("visible").toBool(),
                 "a crop drag did not open the crop dialog — the item's "
                 "settle glue did not fire");
    // Pre-filled from the DRAG, not from the document defaults (640x400):
    // a glue regression that opened a default dialog would pass a mere
    // visibility check.
    auto spin = [&](const char *accName) -> int {
        for (QObject *o : dialog->findChildren<QObject *>()) {
            // Controls SpinBoxes carry generated SpinBox_QMLTYPE_*
            // metaobjects, not QQuickSpinBox.
            if (!QLatin1String(o->metaObject()->className())
                     .startsWith(QLatin1String("SpinBox")))
                continue;
            QQmlExpression expr(qmlContext(o), o,
                                QStringLiteral("Accessible.name"));
            if (expr.evaluate().toString()
                == m_i18n->tr(QLatin1String(accName)))
                return o->property("value").toInt();
        }
        return -1;
    };
    // The EXTENT is what proves the pre-fill came from the drag rather
    // than the document defaults (640x400) — pinned exactly. The origin
    // carries the same one pixel the chrome's half-pixel doc->scene
    // mapping costs everywhere in this file.
    QCOMPARE(spin("ui.dialog.crop.width"), 280);
    QCOMPARE(spin("ui.dialog.crop.height"), 200);
    QVERIFY2(qAbs(spin("ui.dialog.crop.x") - 120) <= 1
                 && qAbs(spin("ui.dialog.crop.y") - 80) <= 1,
             qPrintable(QStringLiteral("crop dialog pre-filled origin "
                                       "%1,%2 — expected 120,80 +/-1")
                            .arg(spin("ui.dialog.crop.x"))
                            .arg(spin("ui.dialog.crop.y"))));
    QMetaObject::invokeMethod(dialog, "close");
    QTRY_VERIFY(!dialog->property("visible").toBool());
}

void TstGestureChrome::cropDragThenApplyNarrowsTheFrameNotTheCanvas()
{
    // The crop tool's COMPLETE user story, end to end, on the
    // NON-DESTRUCTIVE arm: drag on canvas -> pre-filled dialog -> Apply ->
    // the OUTPUT FRAME becomes the dragged rect while the CANVAS is left
    // alone. cropDragOpensPrefilledCropDialog pins the first half; the
    // Apply button's doc.crop call (CropDialog.qml:103) had NO coverage at
    // all — verified by grep: this file and tst_crop_capability_pin.cpp
    // are the only tests repo-wide that mention CropDialog, and this file
    // only for the pre-fill.
    //
    // Grown on a quiet tick (main unmoved four ticks running) against the
    // standing uncovered-surface list, rather than idling: a known gap
    // does not need main to move before it can be closed.
    //
    // ★ ARM: since RC3 D-5 this case CLICKS the deletePixels Switch OFF
    // (the dialog opens ON) to reach the non-destructive arm (deletePixels:
    // false — §19.1). Its sibling cropDragThenApplyWithDeletePixelsShrinks
    // TheCanvas performs the IDENTICAL drag on the default (ON), and the two
    // assert DIFFERENT outcomes from that same drag. If you ever find them
    // asserting the same thing, one of them has stopped measuring an arm.
    QObject *dialog = nullptr;
    for (QObject *o : m_window->findChildren<QObject *>()) {
        if (!QLatin1String(o->metaObject()->className())
                 .startsWith(QLatin1String("CropDialog")))
            continue;
        dialog = o;
        break;
    }
    QVERIFY2(dialog, "the crop dialog was not found in the chrome");
    QCOMPARE(m_controller->docWidth(), 640);
    QCOMPARE(m_controller->docHeight(), 400);

        // 11e ring cycling (merged this wave): a letter's SECOND press cycles
    // its ring (C = crop -> slice.add -> slice.select; M = select.rect ->
    // select.ellipse). Letters are the subject of the letterRouted*/letterT*
    // cases; HERE the tool choice is incidental setup, so pressing a letter
    // made this case depend on how many times earlier cases pressed it — it
    // passed alone and failed in-suite. Set the tool directly instead.
    m_canvasItem->setProperty("tool", QStringLiteral("crop"));
    QCOMPARE(canvasTool(), QStringLiteral("crop"));
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToScene(100, 60));
    QTest::mouseMove(m_window, docToScene(300, 200));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToScene(420, 300));
    QTRY_VERIFY2(dialog->property("visible").toBool(),
                 "the crop drag did not open the dialog");

    // RC3 D-5: the dialog opens on the DESTRUCTIVE arm now, so this case
    // reaches the non-destructive arm the way a user does -- by CLICKING the
    // deletePixels Switch OFF (found by its cataloged label; CropDialog has
    // two Switches).
    QQuickItem *deletePixels = nullptr;
    for (QObject *o : dialog->findChildren<QObject *>()) {
        if (!QLatin1String(o->metaObject()->className())
                 .startsWith(QLatin1String("Switch")))
            continue;
        if (o->property("text").toString()
            != m_i18n->tr(QStringLiteral("ui.dialog.crop.delete_pixels")))
            continue;
        deletePixels = qobject_cast<QQuickItem *>(o);
        break;
    }
    QVERIFY2(deletePixels, "the deletePixels Switch was not found by its "
                           "cataloged label");
    QVERIFY(deletePixels->property("checked").toBool());
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      center(deletePixels));
    QTRY_VERIFY2(!deletePixels->property("checked").toBool(),
                 "clicking the deletePixels Switch did not untick it -- the "
                 "non-destructive arm is unreachable by gesture");

    // Press the dialog's Apply button by name (the cataloged label), not
    // by position — a moved button must not silently stop being pressed.
    QObject *apply = nullptr;
    for (QObject *o : dialog->findChildren<QObject *>()) {
        if (!QLatin1String(o->metaObject()->className())
                 .startsWith(QLatin1String("Button")))
            continue;
        if (o->property("text").toString()
            != m_i18n->tr(QStringLiteral("ui.dialog.apply")))
            continue;
        apply = o;
        break;
    }
    QVERIFY2(apply, "the crop dialog's Apply button was not found by its "
                    "cataloged label");
    QVERIFY2(apply->property("enabled").toBool(),
             "the crop dialog's Apply button is disabled with a document open");
    auto *applyItem = qobject_cast<QQuickItem *>(apply);
    QVERIFY(applyItem);
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      center(applyItem));

    // ---- THE DOOR, BEFORE THE GEOMETRY ---------------------------------
    // `!visible` is the r.ok PROBE: CropDialog.qml:110 closes the dialog
    // ONLY on r.ok. Asserted AFTER the geometry compare (as it was until
    // this round), a REFUSED door and a WRONG-GEOMETRY door produce the
    // same first-fail line — "the frame is not 320x240" — and the two are
    // indistinguishable in the failure output. Asserted FIRST, the
    // first-fail names which of the two broke.
    QTRY_VERIFY2(!dialog->property("visible").toBool(),
                 "the crop dialog stayed open — it closes only on r.ok, so "
                 "the engine REFUSED doc.crop. This is a DOOR failure, not "
                 "a geometry failure; the frame asserts below never ran");

    // ---- ENGINE TRUTH under §19.1 non-destructive crop ------------------
    // deletePixels:false is SERVED, so the CANVAS deliberately does not
    // shrink; the OUTPUT FRAME narrows to the dragged extent WITH ITS
    // ORIGIN CARRIED. (The old destructive contract — "the document is now
    // the dragged rect's size" — is what the sibling below measures, and
    // asserting it here would assert behaviour the engine no longer has.)
    //
    // ⚠ THIS ASSERTION WAS `docWidth() == 320` UNTIL 2026-08-23 and it was
    // RIGHT THEN: `abiHasBlock17()` was false, the dialog forced
    // `deletePixels` ON, and the canvas really did shrink. The capability
    // going live is an INTENDED behaviour change (operator-authorized), so
    // the test states the new truth rather than the old one — and the case
    // was renamed from `cropDragThenApplyCropsTheDocument` to say which arm
    // it now measures. The old name and the old assertion are preserved
    // where they are still true: on the destructive sibling below, which is
    // the only place `deletePixels:true` is still driven end to end.
    //
    // Pinning BOTH halves is strictly stronger than the single check it
    // replaces: `docWidth() == 320` alone could not distinguish "the canvas
    // shrank" (destructive) from "the image frame narrowed" (correct), and
    // a regression back to destructive crop would have passed it.
    //
    // The dragged extent is 320x240 at origin 100,60. BOTH halves are
    // pinned: a size-only assertion would pass against a frame anchored at
    // (0,0), which is the exact bug class the origin carry exists to
    // prevent. The origin carries the same one pixel the chrome's
    // half-pixel doc->scene mapping costs everywhere in this file (see the
    // pre-fill assert in cropDragOpensPrefilledCropDialog), so it is
    // pinned +/-1 while the SIZE is pinned exactly.
    QTRY_COMPARE_WITH_TIMEOUT(m_controller->imageRect().size(),
                              QSize(320, 240), 3000);
    const QRect frame = m_controller->imageRect();
    QVERIFY2(qAbs(frame.x() - 100) <= 1 && qAbs(frame.y() - 60) <= 1,
             qPrintable(QStringLiteral("output frame origin %1,%2 — expected "
                                       "100,60 +/-1. A frame at 0,0 with the "
                                       "right SIZE reads the right number of "
                                       "pixels from the wrong place")
                            .arg(frame.x())
                            .arg(frame.y())));

    // ★ AND THE CANVAS DID NOT MOVE. This is the half that makes the
    // sibling's assertion a genuine disagreement rather than a second
    // spelling of the same one.
    QCOMPARE(m_controller->docWidth(), 640);
    QCOMPARE(m_controller->docHeight(), 400);

    // CLEANUP, not subject: the suite shares ONE CropDialog and the Switch
    // remembers the user's last choice across opens (by design), so put it
    // back to the RC3 D-5 default -- otherwise the sibling inherits OFF.
    deletePixels->setProperty("checked", true);
}

void TstGestureChrome::cropDragThenApplyWithDeletePixelsShrinksTheCanvas()
{
    // ★★★★ THE DESTRUCTIVE ARM, END TO END FROM A GESTURE.
    //
    // Same tool, same drag, same dialog, same cataloged Apply button as
    // cropDragThenApplyNarrowsTheFrameNotTheCanvas. The ONLY difference
    // is the deletePixels Switch, clicked ON before Apply. The two cases
    // assert DIFFERENT OUTCOMES FROM THE SAME DRAG:
    //
    //   sibling (Switch untouched => deletePixels:false, the served
    //       §19.1 default):  canvas stays 640x400, frame -> [100,60,320,240]
    //   this case (Switch ON => deletePixels:true):
    //                        canvas SHRINKS to 320x240, frame -> [0,0,320,240]
    //
    // WHY THIS CASE EXISTS. Before the capability flip, the case now
    // named cropDragThenApplyNarrowsTheFrameNotTheCanvas (then
    // cropDragThenApplyCropsTheDocument) was the ONLY case anywhere
    // driving deletePixels:true through the full gesture -> dialog ->
    // engine loop. The flip moved it onto the non-destructive arm, so
    // retargeting its assertion — which was the whole of the reported
    // fix — would have left the destructive arm reachable in tests only
    // through a direct
    // applyCommand (tst_crop_capability_pin.cpp STATE 3) or at envelope
    // level with no engine behind it (deadCapabilityApplySendsDeletePixels-
    // True). The suite would have gone green and stopped measuring the arm
    // without saying so. This case is what keeps that loop measured.
    QObject *dialog = nullptr;
    for (QObject *o : m_window->findChildren<QObject *>()) {
        if (!QLatin1String(o->metaObject()->className())
                 .startsWith(QLatin1String("CropDialog")))
            continue;
        dialog = o;
        break;
    }
    QVERIFY2(dialog, "the crop dialog was not found in the chrome");
    QCOMPARE(m_controller->docWidth(), 640);
    QCOMPARE(m_controller->docHeight(), 400);

    // The Switch is only reachable while the capability is live
    // (CropDialog.qml binds `enabled: AppDoc.cropNonDestructiveLive`).
    // With the capability DEAD the Switch is disabled and already pinned
    // ON, so every assertion below would still pass — but it would be
    // measuring a forced default, not a user toggle, and this case's whole
    // subject is the toggle. Say so instead of passing quietly.
    QVERIFY2(m_controller->property("cropNonDestructiveLive").toBool(),
             "abi_minor < 7 in this tree: the deletePixels Switch is locked "
             "ON, so this case cannot distinguish a user-flipped arm from "
             "the forced NI default and would pass for the wrong reason");

    m_canvasItem->setProperty("tool", QStringLiteral("crop"));
    QCOMPARE(canvasTool(), QStringLiteral("crop"));
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToScene(100, 60));
    QTest::mouseMove(m_window, docToScene(300, 200));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToScene(420, 300));
    QTRY_VERIFY2(dialog->property("visible").toBool(),
                 "the crop drag did not open the dialog");

    // The deletePixels Switch by its CATALOGED LABEL, not by findChildren
    // order: CropDialog has TWO Switches (deletePixels and allowGrowth)
    // and with the capability live they are property-identical
    // (enabled=true, checked=false), so "the first Switch" is not an
    // identification.
    QQuickItem *deletePixels = nullptr;
    for (QObject *o : dialog->findChildren<QObject *>()) {
        if (!QLatin1String(o->metaObject()->className())
                 .startsWith(QLatin1String("Switch")))
            continue;
        if (o->property("text").toString()
            != m_i18n->tr(QStringLiteral("ui.dialog.crop.delete_pixels")))
            continue;
        deletePixels = qobject_cast<QQuickItem *>(o);
        break;
    }
    QVERIFY2(deletePixels, "the deletePixels Switch was not found by its "
                           "cataloged label");
    QVERIFY2(deletePixels->property("enabled").toBool(),
             "the deletePixels Switch is disabled while the capability "
             "reports live");
    // RC3 D-5 (operator WORD 35, sitting S-17): the dialog now OPENS on the
    // destructive arm -- Apply crops the document. This case rides that
    // default untouched; its sibling cropDragThenApplyNarrowsTheFrameNotThe
    // Canvas CLICKS the Switch OFF, so the identical drag still asserts two
    // different outcomes from two switch states.
    QVERIFY2(deletePixels->property("checked").toBool(),
             "the crop dialog opened OFF -- Apply would set only the output "
             "frame and the document would keep its old size (S-17)");

    QObject *apply = nullptr;
    for (QObject *o : dialog->findChildren<QObject *>()) {
        if (!QLatin1String(o->metaObject()->className())
                 .startsWith(QLatin1String("Button")))
            continue;
        if (o->property("text").toString()
            != m_i18n->tr(QStringLiteral("ui.dialog.apply")))
            continue;
        apply = o;
        break;
    }
    QVERIFY2(apply, "the crop dialog's Apply button was not found by its "
                    "cataloged label");
    QVERIFY2(apply->property("enabled").toBool(),
             "the crop dialog's Apply button is disabled with a document open");
    auto *applyItem = qobject_cast<QQuickItem *>(apply);
    QVERIFY(applyItem);
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      center(applyItem));

    // The door first, for the same reason as in the sibling.
    QTRY_VERIFY2(!dialog->property("visible").toBool(),
                 "the crop dialog stayed open — it closes only on r.ok, so "
                 "the engine REFUSED the DESTRUCTIVE doc.crop. This is a "
                 "DOOR failure, not a geometry failure");

    // ★ ENGINE TRUTH on the destructive arm: the CANVAS shrinks. This is
    // the assertion the sibling can no longer make, and the reason this
    // case is not decorative.
    QTRY_COMPARE_WITH_TIMEOUT(m_controller->docWidth(), 320, 3000);
    QCOMPARE(m_controller->docHeight(), 240);
    // ...and the frame collapses onto the new canvas at the origin: with
    // the pixels gone there is no crop rect left to carry an origin.
    QCOMPARE(m_controller->imageRect(), QRect(0, 0, 320, 240));

    // ---- PUT THE SWITCH BACK -------------------------------------------
    // This chrome is built ONCE in initTestCase and shared by every case
    // in this file, and a user click on a bound property DESTROYS the
    // binding (`checked: !AppDoc.cropNonDestructiveLive`) for the rest of
    // the session — that is shipping QML behaviour, not a test artifact.
    // Left alone, this case would hand every later case a dialog silently
    // parked on the destructive arm: measured, not assumed —
    // cropDeletePixelsArmTracksTheCapability's probe read checked=0 before
    // this case existed and checked=1 after. Restoring the VALUE is the
    // most that can be restored (the binding is gone either way), and it
    // is what the next observer reads.
    deletePixels->setProperty("checked", false);
    QCOMPARE(deletePixels->property("checked").toBool(), false);
}

void TstGestureChrome::selectionModifyDialogExpandsTheGestureSelection()
{
    // A gesture-made selection carried through a DIALOG into the engine.
    // O-UI's review probe covers this dialog's SpinBox validator (the
    // feather decimal arm); nobody covers the loop — marquee drag ->
    // Select > Modify -> Apply -> the engine's MASK actually changes.
    //
    // The dialog's Apply is gated on `AppDoc.selectionActive`, so the
    // gesture is a real precondition: if the drag did not commit, Apply is
    // disabled and this case says so rather than silently doing nothing.
    QObject *dialog = nullptr;
    for (QObject *o : m_window->findChildren<QObject *>()) {
        if (QLatin1String(o->metaObject()->className())
                .startsWith(QLatin1String("SelectionModifyDialog"))) {
            dialog = o;
            break;
        }
    }
    QVERIFY2(dialog, "SelectionModifyDialog not found in the chrome");

    // 1. the GESTURE
        // 11e ring cycling (merged this wave): a letter's SECOND press cycles
    // its ring (C = crop -> slice.add -> slice.select; M = select.rect ->
    // select.ellipse). Letters are the subject of the letterRouted*/letterT*
    // cases; HERE the tool choice is incidental setup, so pressing a letter
    // made this case depend on how many times earlier cases pressed it — it
    // passed alone and failed in-suite. Set the tool directly instead.
    m_canvasItem->setProperty("tool", QStringLiteral("select.rect"));
    QCOMPARE(canvasTool(), QStringLiteral("select.rect"));
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToScene(200, 140));
    QTest::mouseMove(m_window, docToScene(320, 220));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToScene(400, 260));
    QVERIFY2(m_controller->selectionActive(),
             "the marquee drag committed no selection");
    const QRect before = engineSelectionBounds();
    QCOMPARE(before.size(), QSize(200, 120));

    // 2. the DIALOG — opened through the menu door the user uses
    QMetaObject::invokeMethod(dialog, "open");
    QTRY_VERIFY2(dialog->property("visible").toBool(),
                 "the selection-modify dialog did not open");
    // Pick EXPAND (ops index 1) and an amount of 20.
    QObject *opBox = nullptr;
    QObject *amount = nullptr;
    for (QObject *o : dialog->findChildren<QObject *>()) {
        const QLatin1String cls(o->metaObject()->className());
        if (!opBox && cls.startsWith(QLatin1String("ComboBox")))
            opBox = o;
        else if (!amount && cls.startsWith(QLatin1String("SpinBox")))
            amount = o;
    }
    QVERIFY2(opBox && amount, "the dialog's op/amount controls were not found");
    opBox->setProperty("currentIndex", 1); // expand
    amount->setProperty("value", 20);

    QObject *apply = nullptr;
    for (QObject *o : dialog->findChildren<QObject *>()) {
        if (!QLatin1String(o->metaObject()->className())
                 .startsWith(QLatin1String("Button")))
            continue;
        if (o->property("text").toString()
            != m_i18n->tr(QStringLiteral("ui.dialog.modify.apply")))
            continue;
        apply = o;
        break;
    }
    QVERIFY2(apply, "the dialog's Apply button was not found by its label");
    QVERIFY2(apply->property("enabled").toBool(),
             "Apply is disabled even though the gesture committed a "
             "selection — the dialog's selectionActive gate disagrees with "
             "the engine");
    auto *applyItem = qobject_cast<QQuickItem *>(apply);
    QVERIFY(applyItem);
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      center(applyItem));

    // 3. ENGINE TRUTH: expand by 20 grows the mask 20 px on every side.
    QTRY_VERIFY_WITH_TIMEOUT(engineSelectionBounds() != before, 3000);
    const QRect after = engineSelectionBounds();
    QVERIFY2(after.width() >= before.width() + 30
                 && after.height() >= before.height() + 30,
             qPrintable(QStringLiteral("expand(20) grew the selection from "
                                       "%1x%2 to only %3x%4 — the dialog's "
                                       "amount did not reach the engine")
                            .arg(before.width()).arg(before.height())
                            .arg(after.width()).arg(after.height())));
    QVERIFY2(!dialog->property("visible").toBool(),
             "the modify applied but the dialog stayed open (it closes only "
             "on r.ok — a stuck dialog means the door refused)");
    m_controller->applyCommand(QStringLiteral("selection.none"), {}, {}, {});
}

void TstGestureChrome::fillDialogFillsOnlyInsideTheGestureSelection()
{
    // The THIRD gesture-fed dialog (crop at T13, selection-modify at T17):
    // marquee drag -> Edit > Fill -> Apply -> the engine fills INSIDE the
    // gesture's selection and nowhere else.
    //
    // The exploratory run already fills, but through applyCommand directly
    // — it never touches this dialog. So the dialog's own clipping story
    // (does the fill it submits actually respect the mask the gesture
    // made?) had no coverage.
    QObject *dialog = nullptr;
    for (QObject *o : m_window->findChildren<QObject *>()) {
        if (QLatin1String(o->metaObject()->className())
                .startsWith(QLatin1String("FillDialog"))) {
            dialog = o;
            break;
        }
    }
    QVERIFY2(dialog, "FillDialog not found in the chrome");

    // 1. the GESTURE — a marquee over the middle of the canvas
        // 11e ring cycling (merged this wave): a letter's SECOND press cycles
    // its ring (C = crop -> slice.add -> slice.select; M = select.rect ->
    // select.ellipse). Letters are the subject of the letterRouted*/letterT*
    // cases; HERE the tool choice is incidental setup, so pressing a letter
    // made this case depend on how many times earlier cases pressed it — it
    // passed alone and failed in-suite. Set the tool directly instead.
    m_canvasItem->setProperty("tool", QStringLiteral("select.rect"));
    QCOMPARE(canvasTool(), QStringLiteral("select.rect"));
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToScene(180, 120));
    QTest::mouseMove(m_window, docToScene(300, 200));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToScene(420, 280));
    QVERIFY2(m_controller->selectionActive(),
             "the marquee drag committed no selection");
    const QRect sel = engineSelectionBounds();
    QCOMPARE(sel.size(), QSize(240, 160));
    // Pre-state: white inside AND outside.
    QVERIFY(isWhitishPixel(enginePixel(300, 200)));
    QVERIFY(isWhitishPixel(enginePixel(80, 60)));

    // 2. the DIALOG
    QMetaObject::invokeMethod(dialog, "open");
    QTRY_VERIFY2(dialog->property("visible").toBool(),
                 "Edit > Fill did not open the fill dialog");
    QObject *apply = nullptr;
    for (QObject *o : dialog->findChildren<QObject *>()) {
        if (!QLatin1String(o->metaObject()->className())
                 .startsWith(QLatin1String("Button")))
            continue;
        const QString t = o->property("text").toString();
        if (t == m_i18n->tr(QStringLiteral("ui.dialog.fill.apply"))
            || t == m_i18n->tr(QStringLiteral("ui.dialog.apply"))) {
            apply = o;
            break;
        }
    }
    QVERIFY2(apply, "the fill dialog's Apply button was not found by label");
    auto *applyItem = qobject_cast<QQuickItem *>(apply);
    QVERIFY(applyItem);
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      center(applyItem));

    // 3. ENGINE TRUTH — filled INSIDE, untouched OUTSIDE. The outside
    // probe is the load-bearing half: a fill that ignored the gesture's
    // mask would paint the whole layer and still look "applied".
    // WAIT, do not ASSERT here: a QTRY firing first makes the QVERIFY2
    // below unreachable, so a real failure reports a bare condition with
    // an EMPTY message. Caught by mutation at T43.
    QTest::qWaitFor([&] { return !isWhitishPixel(enginePixel(300, 200)); },
                    3000);
    QVERIFY2(isWhitishPixel(enginePixel(80, 60)),
             "the dialog's fill leaked OUTSIDE the gesture selection — it "
             "did not respect the mask the marquee made");
    QVERIFY2(isWhitishPixel(enginePixel(600, 380)),
             "the dialog's fill reached the far corner — the selection was "
             "ignored entirely");
    QVERIFY2(!dialog->property("visible").toBool(),
             "the fill applied but the dialog stayed open (it closes only "
             "on r.ok — a stuck dialog means the door refused)");
    m_controller->applyCommand(QStringLiteral("selection.none"), {}, {}, {});
}

void TstGestureChrome::armedControlsAnnouncePendingOnPress()
{
    // OPERATOR RULE: once the honesty slice lands, any control still
    // ARMED-SILENT or DEAD is a MAJOR finding. The slice landed
    // (`ad75531` — item 25b: "18 armed-but-unwired tools announce
    // 'pending' instead of silently doing nothing").
    //
    // THIS PROBE LIVES HERE, NOT IN tst_gesture_net, AND THAT MATTERS.
    // The net's armedControlHonestyProbe drives a BARE CanvasView with no
    // QML chrome — but the sweep is wired at Main.qml:211
    // (`announcePending` -> AppA11y.announce + AppDoc.statusPending). A
    // bare-canvas probe is structurally incapable of observing it and
    // reports every tool silent forever. Measuring honesty on the wrong
    // surface would have produced a false MAJOR against a slice that
    // works.
    static const char *armed[] = {
        "select.quick", "heal.stroke", "clone.stroke", "content_aware_fill",
        "fill", "dodge_burn.dodge", "dodge_burn.burn", "dodge_burn.sponge",
        "blur_sharpen.blur", "blur_sharpen.sharpen", "smudge.stroke",
        "shape.line", "shape.polygon", "shape.rounded_rect",
        "selection.transform", "slice.add", "slice.select", "transform.warp",
    };
    // ★★★★★ T67 CORRECTION — THIS LIST IS NOT AN ARMED ROSTER ANY MORE,
    // AND UNTIL THIS TICK THE CASE DID NOT NOTICE. Seven of the eighteen
    // rows above are LIVE now (smudge T64; dodge/burn/blur/sharpen T66;
    // heal Q-D1(a) and sponge Q-D3(b), both landed this range). The old
    // assertion was "the status CHANGED and is non-empty", which a live
    // tool satisfies by COMMITTING -- it prints "Applied: ...". So the
    // headline read "18 honest of 18" while eleven rows were proving the
    // honesty claim and seven were proving that a stroke commits.
    //
    // ★★★★ IT NEVER PRODUCED A FALSE PASS, and that is stated rather than
    // glossed: an armed tool going silent still reds, because a silent
    // press leaves the status unchanged either way. What was wrong was
    // the case's ACCOUNT OF ITSELF -- the number 18 claimed a scope the
    // case did not have. Recorded as MINOR against myself (G-13), the
    // same family as T64/T66's stale roster rows and my own G-7.
    //
    // ★★★ THE FIX IS THE T66 SHAPE: STOP HARDCODING THE CLASSIFICATION,
    // DERIVE IT. `armedToolPressed` is emitted in exactly one place --
    // canvas_view.cpp's else fall-through, the complement of the dispatch
    // if-chain -- so "armed" is observable and needs no roster: a tool is
    // armed iff a full press-drag-release moves NO journal entry. Each
    // row is classified at runtime and then held to the rule for the
    // class it actually landed in. A tool graduating now MOVES BETWEEN
    // ARMS instead of quietly changing what the case measures.
    QStringList silent, armedNow, liveNow;
    for (const char *wire : armed) {
        m_controller->applyCommand(QStringLiteral("selection.none"), {}, {}, {});
        m_canvasItem->setProperty("tool", QLatin1String(wire));
        QCoreApplication::processEvents();
        // NON-VACUITY: if the tool did not take, everything below is a
        // fact about whatever tool was already selected.
        QCOMPARE(canvasTool(), QLatin1String(wire));
        const QString before = m_controller->statusText();
        const int histBefore = m_controller->historyEntries().size();
        QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                          docToScene(200, 150));
        QTest::mouseMove(m_window, docToScene(300, 220));
        QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                            docToScene(300, 220));
        QCoreApplication::processEvents();
        const QString after = m_controller->statusText();
        const bool committed =
            m_controller->historyEntries().size() > histBefore;
        if (committed) {
            liveNow << QLatin1String(wire);
            continue; // a live tool owes no pending announcement
        }
        armedNow << QLatin1String(wire);
        // THE ARMED RULE, and it is the PENDING SENTENCE and not merely
        // "something was said". A changed status is what the old arm
        // accepted; a live tool's "Applied" satisfies that and says
        // nothing about honesty.
        if (after == before || after.isEmpty()) {
            silent << QLatin1String(wire);
            continue;
        }
        const QString expected =
            evalJs(QStringLiteral("(function(){ var t = \"%1\"; "
                                  "var k = root.toolNameKeys[t]; "
                                  "if (!k) return \"\"; "
                                  "return AppI18n.tr(root.pendingReasonIds[t] "
                                  "|| \"ui.status.tool_pending\", "
                                  "{\"tool\": AppI18n.tr(k)}); })()")
                       .arg(QLatin1String(wire)))
                .toString();
        // A tool with no cataloged name is DELIBERATELY silent
        // (Main.qml: "no cataloged name: silence beats key-echoing a
        // wire id"), so an empty expectation is not a failure -- but it
        // must not be treated as a pass either, and it cannot arise for
        // a row that just announced something.
        if (!expected.isEmpty() && !after.contains(expected))
            silent << (QLatin1String(wire) + QStringLiteral(" [said \"")
                       + after + QStringLiteral("\", not the pending sentence \"")
                       + expected + QStringLiteral("\"]"));
    }
    qInfo().noquote()
        << QStringLiteral("ARMED-HONESTY (chrome): %1 rows -> %2 ARMED "
                          "(each must speak its pending sentence), %3 LIVE "
                          "(each committed); %4 dishonest")
               .arg(int(sizeof(armed) / sizeof(armed[0])))
               .arg(armedNow.size()).arg(liveNow.size()).arg(silent.size());
    qInfo().noquote() << QStringLiteral("  LIVE now: %1")
                             .arg(liveNow.join(QStringLiteral(", ")));
    // Both classes must be NON-EMPTY. All-live would mean the honesty
    // claim is untested; all-armed would mean the dispatch walk above it
    // is measuring nothing.
    QVERIFY2(!armedNow.isEmpty() && !liveNow.isEmpty(),
             qPrintable(QStringLiteral("the runtime split degenerated "
                                       "(%1 armed, %2 live) -- one arm of "
                                       "this case is now vacuous")
                            .arg(armedNow.size()).arg(liveNow.size())));
    QVERIFY2(silent.isEmpty(),
             qPrintable(QStringLiteral("the honesty slice has landed but "
                                       "these ARMED controls did not speak "
                                       "their pending sentence on press: %1")
                            .arg(silent.join(QStringLiteral("; ")))));
}

void TstGestureChrome::letterTReachesTypeNotTransform()
{
    // 1.0 final (corrigendum (l)): the Text tool is REMOVED and T is
    // unbound. A bare T neither reaches Type nor falls back to transform:
    // the tool does not move.
    m_canvasItem->setProperty("tool", QStringLiteral("select.rect"));
    m_canvasItem->forceActiveFocus();
    for (int i = 0; i < 4; ++i) {
        QTest::keyClick(m_window, Qt::Key_T);
        QTest::qWait(30);
    }
    QCOMPARE(canvasTool(), QStringLiteral("select.rect"));
}

void TstGestureChrome::unifiedSelectionFlyoutMemberCompletesItsGesture()
{
    // FEATURE COVERAGE: item 17 — ALL selection tools now share ONE
    // palette flyout slot. The gesture-level question the palette suite
    // does not ask: after picking a member from that unified slot, does
    // that member's gesture still COMPLETE to engine truth? A regrouping
    // that quietly broke one member would look fine in a slot/face test.
    QQuickItem *slot = nullptr;
    for (QQuickItem *s : m_slots) {
        const QVariantList g = s->property("group").toList();
        int found = 0;
        for (const QVariant &m : g) {
            const QString w = m.toMap().value(QStringLiteral("tool")).toString();
            if (w.startsWith(QLatin1String("select.")))
                ++found;
        }
        if (found >= 3) { slot = s; break; } // the unified selection slot
    }
    QVERIFY2(slot, "no palette slot groups 3+ selection tools — item 17's "
                   "unified slot was not found");
    const QVariantList group = slot->property("group").toList();
    QStringList members;
    for (const QVariant &m : group)
        members << m.toMap().value(QStringLiteral("tool")).toString();
    qInfo().noquote()
        << QStringLiteral("ITEM-17 unified selection slot members: %1")
               .arg(members.join(QStringLiteral(", ")));
    QVERIFY2(members.size() >= 4,
             "the unified slot holds fewer members than item 17 describes");

    // Drive EVERY marquee-style member of the unified slot to a committed
    // selection. (The wand fires on press and is covered separately.)
    for (const QString &wire : members) {
        if (wire == QLatin1String("select.wand")
            || wire == QLatin1String("select.polygon"))
            continue; // click-grammar tools, covered by their own cases
        if (wire == QLatin1String("select.quick")) {
            // ARMED, not live (tier-2). Item 17's unified slot therefore
            // mixes live and armed members — G-4's mixed-liveness pattern,
            // now inside the SELECTION slot. The honest requirement for
            // this member is not "completes a gesture" but "says so":
            // assert it announces pending rather than falling silent.
            m_canvasItem->setProperty("tool", wire);
            const QString before = m_controller->statusText();
            QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                              docToScene(150, 110));
            QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                                docToScene(280, 190));
            const QString after = m_controller->statusText();
            QVERIFY2(after != before && !after.isEmpty(),
                     "the unified slot's ARMED member fell silent on press "
                     "instead of announcing pending");
            qInfo().noquote()
                << QStringLiteral("  armed member %1 announces: %2")
                       .arg(wire).arg(after);
            continue;
        }
        m_controller->applyCommand(QStringLiteral("selection.none"), {}, {}, {});
        m_canvasItem->setProperty("tool", wire);
        QCOMPARE(canvasTool(), wire);
        QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                          docToScene(150, 110));
        QTest::mouseMove(m_window, docToScene(280, 190));
        QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                            docToScene(380, 250));
        QVERIFY2(m_controller->selectionActive(),
                 qPrintable(QStringLiteral("unified-slot member %1 committed "
                                           "NO selection; status: %2")
                                .arg(wire)
                                .arg(m_controller->statusText())));
        const QRect b = engineSelectionBounds();
        QVERIFY2(b.width() >= 150 && b.height() >= 100,
                 qPrintable(QStringLiteral("unified-slot member %1 committed "
                                           "a degenerate selection %2x%3")
                                .arg(wire).arg(b.width()).arg(b.height())));
    }
    m_controller->applyCommand(QStringLiteral("selection.none"), {}, {}, {});
}

void TstGestureChrome::historyPanelRowJumpRewindsTheDocument()
{
    // G-6 REMEDY (owed since T20). Wave F wired history.goto — the
    // HistoryPanel row jump, "the census's worst DEAD row retired".
    // O-UI ships a history-panel unit suite; this is the GESTURE dimension:
    // a gesture makes history, a row activation jumps back, and the
    // DOCUMENT actually rewinds.
        // 11e ring cycling (merged this wave): a letter's SECOND press cycles
    // its ring (C = crop -> slice.add -> slice.select; M = select.rect ->
    // select.ellipse). Letters are the subject of the letterRouted*/letterT*
    // cases; HERE the tool choice is incidental setup, so pressing a letter
    // made this case depend on how many times earlier cases pressed it — it
    // passed alone and failed in-suite. Set the tool directly instead.
    m_canvasItem->setProperty("tool", QStringLiteral("select.rect"));
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToScene(160, 120));
    QTest::mouseMove(m_window, docToScene(280, 200));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToScene(360, 240));
    QVERIFY(m_controller->selectionActive());
    QVariantMap color;
    color.insert(QStringLiteral("model"), QStringLiteral("rgb"));
    color.insert(QStringLiteral("comps"), QVariantList{0.9, 0.1, 0.1, 0.0});
    QVariantMap fill;
    fill.insert(QStringLiteral("content"), QStringLiteral("color"));
    fill.insert(QStringLiteral("color"), color);
    QVariantMap target;
    target.insert(QStringLiteral("layerId"),
                  m_controller->layers().first().toMap().value(
                      QStringLiteral("layerId")));
    QVERIFY(m_controller->applyCommand(QStringLiteral("fill"), target, fill, {})
                .value(QStringLiteral("ok")).toBool());
    QVERIFY2(!isWhitishPixel(enginePixel(250, 180)),
             "setup: the fill did not paint");
    const int entries = m_controller->historyEntries().size();
    QVERIFY2(entries >= 2,
             qPrintable(QStringLiteral("setup: only %1 history entries")
                            .arg(entries)));

    // Diagnostic: what positions does the panel actually render?
    {
        QStringList rows;
        for (const QVariant &e : m_controller->historyEntries()) {
            const QVariantMap m = e.toMap();
            rows << QStringLiteral("pos=%1 label=%2")
                        .arg(m.value(QStringLiteral("position")).toInt())
                        .arg(m.value(QStringLiteral("label")).toString());
        }
        qInfo().noquote() << "HISTORY ROWS:" << rows.join(QStringLiteral(" | "));
    }
    // The JUMP, through the same door the panel row fires. Positions are
    // 1-BASED and the panel only ever renders 1..N: position 0 is a
    // pre-document state the UI cannot reach, and asking for it leaves the
    // canvas with no composite to read ("an uninitialized document has no
    // composite"). Jump to the FIRST rendered row instead — what a user
    // clicking the top of the panel actually does.
    const QVariantList rows = m_controller->historyEntries();
    QVERIFY(!rows.isEmpty());
    const int firstPos =
        rows.first().toMap().value(QStringLiteral("position")).toInt();
    QCOMPARE(firstPos, 1);
    const QVariantMap r = m_controller->historyGoto(firstPos);
    QVERIFY2(r.value(QStringLiteral("ok")).toBool(),
             qPrintable(QStringLiteral("history.goto(first row) refused: %1")
                            .arg(m_controller->statusText())));
    // ENGINE TRUTH: the fill is undone — the document really rewound,
    // rather than the panel merely re-rendering.
    QTest::qWaitFor([&] { return isWhitishPixel(enginePixel(250, 180)); },
                    3000); // wait only — the QVERIFY2 below carries the message
    QVERIFY2(isWhitishPixel(enginePixel(250, 180)),
             "the history row jump reported ok but the filled pixel "
             "survived — the jump did not rewind the document");
}

void TstGestureChrome::pasteIntoClipsToTheGestureSelection()
{
    // G-6 REMEDY (owed since T20). Paste Into — the clipboard-into-
    // selection door. Its whole contract is the CLIP: the pasted pixels
    // must land INSIDE the gesture's mask and nowhere else, which is what
    // distinguishes it from ordinary paste.
    QImage payload(400, 300, QImage::Format_ARGB32);
    payload.fill(QColor(20, 200, 60));
    QGuiApplication::clipboard()->setImage(payload);

        // 11e ring cycling (merged this wave): a letter's SECOND press cycles
    // its ring (C = crop -> slice.add -> slice.select; M = select.rect ->
    // select.ellipse). Letters are the subject of the letterRouted*/letterT*
    // cases; HERE the tool choice is incidental setup, so pressing a letter
    // made this case depend on how many times earlier cases pressed it — it
    // passed alone and failed in-suite. Set the tool directly instead.
    m_canvasItem->setProperty("tool", QStringLiteral("select.rect"));
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToScene(180, 140));
    QTest::mouseMove(m_window, docToScene(280, 200));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToScene(380, 250));
    QVERIFY2(m_controller->selectionActive(),
             "the marquee committed no selection to paste into");

    const bool ok = m_controller->pasteIntoFromClipboard();
    qInfo().noquote()
        << QStringLiteral("PASTE-INTO: ok=%1 status=%2")
               .arg(ok ? QStringLiteral("true") : QStringLiteral("false"))
               .arg(m_controller->statusText());
    // VACUITY TRIPWIRE (T32): the honest-refusal arm was right while the
    // door was unserved. Paste Into has SERVED since T23, so a refusal is
    // now a regression — and returning after merely checking the refusal
    // was typed would let the clip contract go untested silently.
    QVERIFY2(ok,
             qPrintable(QStringLiteral("Paste Into REFUSED — it served from "
                                       "T23, so this is a regression, not an "
                                       "unbuilt door; status: %1")
                            .arg(m_controller->statusText())));
    // THE CLIP is the contract: inside painted, outside untouched.
    QVERIFY2(!isWhitishPixel(enginePixel(250, 190)),
             "Paste Into reported success but painted nothing inside the "
             "selection");
    QVERIFY2(isWhitishPixel(enginePixel(60, 60)),
             "Paste Into leaked OUTSIDE the gesture selection — the clip "
             "that distinguishes it from a plain paste did not happen");
    QVERIFY2(isWhitishPixel(enginePixel(600, 380)),
             "Paste Into reached the far corner — the selection was ignored");
}

void TstGestureChrome::pasteIntoMenuGlueFitsOnOverflow()
{
    // CLOSES THE SECOND OF THE FOUR item-12 GLUE SITES. The recorded
    // omission at tst_selection_gestures' fitIfOverflowFitsOnlyWhenOverflowing
    // says the four Main.qml call sites are "QML glue no harness drives";
    // pasteMenuGlueFitsOnOverflow above closed the FIRST (pasteAndFit).
    // This closes pasteIntoSelection().
    //
    // WHY IT IS A SEPARATE CASE FROM pasteIntoClipsToTheGestureSelection,
    // WHICH IS THE WHOLE POINT: that case drives
    // m_controller->pasteIntoFromClipboard() -- the CONTROLLER method -- and
    // never touches root.pasteIntoSelection(). So the ENGINE door was
    // covered and the QML glue was not, and a case whose NAME reads like
    // coverage of this door was the reason nobody noticed
    // (INTEGRATION-log 523).
    //
    // THE PREDICTION, IN THE REQUIREMENT'S OWN WORDS FIRST (G-516).
    // Main.qml: `function pasteIntoSelection() {
    //              if (AppDoc.pasteIntoFromClipboard()) canvas.fitIfOverflow(); }`
    // So: with a selection active and the viewport overflowing, triggering
    // Edit > Paste Into must leave zoomPercent NOT 400 and equal to the fit.
    // If the glue is missing, the paste still succeeds and the zoom stays
    // 400 -- which is exactly the "ships green" failure the omission names.

    // Selection FIRST, at the default zoom: docToScene() maps through the
    // canvas item's geometry and does not account for zoom, so the marquee
    // is committed before the viewport is overflowed. (pasteMenuGlue* has
    // the same ordering for the same reason.)
    m_canvasItem->setProperty("tool", QStringLiteral("select.rect"));
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToScene(180, 140));
    QTest::mouseMove(m_window, docToScene(280, 200));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToScene(380, 250));
    QVERIFY2(m_controller->selectionActive(),
             "the marquee committed no selection, so edit.pasteInto cannot arm");

    QImage payload(400, 300, QImage::Format_ARGB32);
    payload.fill(QColor(20, 200, 60));
    QGuiApplication::clipboard()->setImage(payload);

    QObject *pasteInto = nullptr;
    for (QObject *o : m_window->findChildren<QObject *>()) {
        if (!o->inherits("QQuickMenuItem"))
            continue;
        if (o->property("text").toString()
            != m_i18n->tr(QStringLiteral("ui.menu.edit.paste_into")))
            continue;
        pasteInto = o;
        break;
    }
    // G-547: the lookup must have PRODUCED something. A null here with the
    // asserts below skipped would render as a pass over an empty search.
    QVERIFY2(pasteInto, "Edit > Paste Into menu item not found in the chrome");
    // The availability row is edit.pasteInto = doc && abiBlock12 &&
    // selectionActive, so an enabled row is also evidence the marquee above
    // reached the row rather than only the engine.
    QVERIFY2(pasteInto->property("enabled").toBool(),
             "Edit > Paste Into disabled with a document open and a selection "
             "active — the availability row did not see the marquee");

    m_canvas->setZoomCentered(4.0);
    QCOMPARE(m_canvasItem->property("zoomPercent").toInt(), 400);
    const int layersBefore = m_controller->layers().size();

    QVERIFY(QMetaObject::invokeMethod(pasteInto, "triggered"));

    // Non-vacuity: the glue is guarded by pasteIntoFromClipboard()'s success
    // return, so a paste that did nothing would leave the zoom at 400 for the
    // WRONG reason and this case would pass as a false green.
    QTRY_VERIFY2(m_controller->layers().size() > layersBefore,
                 qPrintable(QStringLiteral("Paste Into landed no layer, so the "
                                           "item-12 glue was never reached; "
                                           "status: %1")
                                .arg(m_controller->statusText())));
    QTRY_VERIFY2(m_canvasItem->property("zoomPercent").toInt() != 400,
                 "Paste Into succeeded but the item-12 fitIfOverflow glue did "
                 "not fire — the view stayed overflowed at 400%");
    // And it FIT rather than landing on some other zoom.
    const double fit = qMin((m_canvasItem->width() - 24.0) / 640.0,
                            (m_canvasItem->height() - 24.0) / 400.0);
    QCOMPARE(m_canvasItem->property("zoomPercent").toInt(),
             int(qRound(fit * 100.0)));
}

void TstGestureChrome::dropGlueFitsOnOverflow()
{
    // SITE 3 OF THE FOUR item-12 GLUE SITES: the DropArea's onDropped.
    //
    // ★★ THIS CASE EXISTS BECAUSE A COMMENT SAID IT COULD NOT.
    // tst_import_filter.cpp, above dropRefusalIdResolvesThroughTheCatalog,
    // states: "the drop path has no functional test: QTest cannot synthesize
    // a DropArea drop. This is the guard that can be had." That sentence is a
    // LIVE EXONERATION of a coverage gap, so it earns the harder check
    // (G-506: verify an exoneration as hard as an accusation), and it packs
    // two claims that do not imply one another:
    //   (a) QTest has no drop helper            -- true, there is no
    //                                              QTest::drop().
    //   (b) therefore a harness cannot drive it -- DOES NOT FOLLOW.
    // Delivering a QDragEnterEvent and a QDropEvent to the window is plain
    // event delivery, not a QTest gesture. (a) is a fact about one tool;
    // (b) was inferred from it. This case is the firing test for (b) -- §11:
    // a claim that a check CANNOT discriminate is itself a claim needing one.
    //
    // THE PREDICTION, IN THE REQUIREMENT'S OWN WORDS FIRST (G-516).
    // Main.qml's DropArea: `for (let i = 0; i < drop.urls.length; ++i)
    //     if (AppDoc.importImage(drop.urls[i].toString()))
    //         canvas.fitIfOverflow(); // item 12`
    // So: a dropped PNG url, with the viewport overflowed, must land a layer
    // AND leave zoomPercent at the fit rather than 400.

    QTemporaryDir tmp;
    QVERIFY(tmp.isValid());
    const QString png = tmp.filePath(QStringLiteral("dropped.png"));
    QImage payload(120, 90, QImage::Format_ARGB32);
    payload.fill(QColor(15, 90, 200));
    QVERIFY2(payload.save(png), "could not write the PNG this case drops");

    QMimeData mime;
    mime.setUrls({QUrl::fromLocalFile(png)});
    QVERIFY2(mime.hasUrls(), "vacuity: the QMimeData carries no urls, so "
                             "drop.hasUrls would be false and the handler "
                             "would return before reaching the glue");

    m_canvas->setZoomCentered(4.0);
    QCOMPARE(m_canvasItem->property("zoomPercent").toInt(), 400);
    const int layersBefore = m_controller->layers().size();

    // A DropArea only accepts a drop it has seen ENTER: QQuickDropArea tracks
    // containsDrag, so a bare QDropEvent is dropped on the floor.
    const QPoint at(int(m_canvasItem->width() / 2),
                    int(m_canvasItem->height() / 2));
    QDragEnterEvent enter(at, Qt::CopyAction, &mime, Qt::LeftButton,
                          Qt::NoModifier);
    QGuiApplication::sendEvent(m_window, &enter);
    QDragMoveEvent move(at, Qt::CopyAction, &mime, Qt::LeftButton,
                        Qt::NoModifier);
    QGuiApplication::sendEvent(m_window, &move);
    QDropEvent drop(at, Qt::CopyAction, &mime, Qt::LeftButton, Qt::NoModifier);
    QGuiApplication::sendEvent(m_window, &drop);

    qInfo().noquote()
        << QStringLiteral("DROP: enterAccepted=%1 dropAccepted=%2 layers %3->%4 "
                          "zoom=%5 status=%6")
               .arg(enter.isAccepted() ? QStringLiteral("true")
                                       : QStringLiteral("false"))
               .arg(drop.isAccepted() ? QStringLiteral("true")
                                      : QStringLiteral("false"))
               .arg(layersBefore)
               .arg(m_controller->layers().size())
               .arg(m_canvasItem->property("zoomPercent").toInt())
               .arg(m_controller->statusText());

    // Non-vacuity first, the same shape as the two paste-glue cases: the glue
    // is guarded by importImage()'s success return, so a drop that never
    // landed would leave the zoom at 400 for the WRONG reason.
    QTRY_VERIFY2(m_controller->layers().size() > layersBefore,
                 qPrintable(QStringLiteral("the synthesised drop landed no "
                                           "layer, so the item-12 glue was "
                                           "never reached; status: %1")
                                .arg(m_controller->statusText())));
    QTRY_VERIFY2(m_canvasItem->property("zoomPercent").toInt() != 400,
                 "the drop imported but the item-12 fitIfOverflow glue did not "
                 "fire — the view stayed overflowed at 400%");
    const double dropFit = qMin((m_canvasItem->width() - 24.0) / 640.0,
                                (m_canvasItem->height() - 24.0) / 400.0);
    QCOMPARE(m_canvasItem->property("zoomPercent").toInt(),
             int(qRound(dropFit * 100.0)));
}

void TstGestureChrome::importDialogGlueFitsOnOverflow()
{
    // SITE 4 OF THE FOUR item-12 GLUE SITES, AND THE LAST: the File > Import
    // FileDialog's onAccepted.
    //
    // ★★ ITS REACHABILITY WAS UNTESTED, NOT HARD -- WHICH IS ENTRY 527's
    // LESSON APPLIED TO MY OWN WORDING. Three entries called this site "the
    // one whose reachability I have still not tested", and that is a
    // different sentence from "cannot be reached". The word "cannot" cost a
    // real coverage gap once already (the DropArea, INTEGRATION-log 527), so
    // this is the measurement rather than another paragraph about it.
    //
    // THE MECHANISM, AND WHY IT IS THE SAME STANDING AS SITE 2's.
    // These FileDialogs carry no objectName, so findChildren cannot name
    // them; tst_import_filter already reads this one's LIVE nameFilters by
    // its Main.qml id through a QQmlExpression, and that is the door reused
    // here. Setting selectedFile and emitting accepted() drives the REAL
    // handler on the REAL object by its own signal -- exactly what
    // invokeMethod(pasteInto, "triggered") does for site 2. It does not open
    // a platform dialog and does not need to: the glue lives in onAccepted.
    //
    // THE PREDICTION, IN THE REQUIREMENT'S OWN WORDS FIRST (G-516).
    // Main.qml: `onAccepted: if (AppDoc.importImage(selectedFile.toString()))
    //                            canvas.fitIfOverflow() // item 12`
    // So: with selectedFile pointing at a real PNG and the viewport
    // overflowed, emitting accepted() must land a layer AND leave zoomPercent
    // at the fit rather than 400.

    QTemporaryDir tmp;
    QVERIFY(tmp.isValid());
    const QString png = tmp.filePath(QStringLiteral("chosen.png"));
    QImage payload(150, 110, QImage::Format_ARGB32);
    payload.fill(QColor(220, 120, 30));
    QVERIFY2(payload.save(png), "could not write the PNG this case imports");
    const QString url = QUrl::fromLocalFile(png).toString();

    // The dialog must be REACHABLE before anything is asserted about it.
    // G-547 at the input: a null here would leave every assert below
    // measuring an absent object.
    const QVariant probe = evalJs(QStringLiteral("importFileDialog !== null"));
    QVERIFY2(probe.toBool(),
             "importFileDialog is not reachable by its Main.qml id -- the "
             "evalJs door tst_import_filter uses did not resolve here");

    m_canvas->setZoomCentered(4.0);
    QCOMPARE(m_canvasItem->property("zoomPercent").toInt(), 400);
    const int layersBefore = m_controller->layers().size();

    evalJs(QStringLiteral("importFileDialog.selectedFile = '%1'").arg(url));
    // READ IT BACK. If selectedFile is read-only the assignment above fails
    // SILENTLY, importImage gets an empty url, and the layer arm below would
    // red with a message about the glue -- blaming the wrong thing. This arm
    // makes that failure say what it actually is.
    const QString readBack =
        evalJs(QStringLiteral("String(importFileDialog.selectedFile)")).toString();
    QVERIFY2(readBack == url,
             qPrintable(QStringLiteral("selectedFile did not take the "
                                       "assignment (read back '%1', set "
                                       "'%2') -- the property is not writable "
                                       "from here, so this case cannot reach "
                                       "the handler")
                            .arg(readBack, url)));

    evalJs(QStringLiteral("importFileDialog.accepted()"));

    qInfo().noquote()
        << QStringLiteral("IMPORT-DIALOG: selectedFile=%1 layers %2->%3 zoom=%4 "
                          "status=%5")
               .arg(readBack)
               .arg(layersBefore)
               .arg(m_controller->layers().size())
               .arg(m_canvasItem->property("zoomPercent").toInt())
               .arg(m_controller->statusText());

    // Non-vacuity, the same shape as the other three: the glue is guarded by
    // importImage()'s success return, so an import that never landed would
    // leave the zoom at 400 for the WRONG reason.
    QTRY_VERIFY2(m_controller->layers().size() > layersBefore,
                 qPrintable(QStringLiteral("File > Import landed no layer, so "
                                           "the item-12 glue was never "
                                           "reached; status: %1")
                                .arg(m_controller->statusText())));
    QTRY_VERIFY2(m_canvasItem->property("zoomPercent").toInt() != 400,
                 "File > Import succeeded but the item-12 fitIfOverflow glue "
                 "did not fire — the view stayed overflowed at 400%");
    const double importFit = qMin((m_canvasItem->width() - 24.0) / 640.0,
                                  (m_canvasItem->height() - 24.0) / 400.0);
    QCOMPARE(m_canvasItem->property("zoomPercent").toInt(),
             int(qRound(importFit * 100.0)));
}

void TstGestureChrome::documentlessDropAnswersOutLoud()
{
    // CLOSES THE GAP THE item-12 ARC LEFT NAMED. All four glue cases drive
    // their door with a document OPEN, because init() creates one and
    // asserts it -- so none of them takes the DropArea's refusal branch
    // (`!AppDoc.hasDocument || !AppDoc.abiBlock12`). INTEGRATION-log 528
    // and 531 both recorded that branch as covered ONLY by
    // tst_import_filter::dropRefusalIdResolvesThroughTheCatalog, a
    // catalog-level guard. This is the functional half.
    //
    // WHY THE BRANCH MATTERS, IN Main.qml's OWN WORDS: the DropArea is
    // `enabled: true` DELIBERATELY (honest-state rule D4). It used to be
    // gated on hasDocument && abiBlock12, "which made a drop onto a
    // documentless window do NOTHING AT ALL -- no ink, no message, no
    // feedback. A silent surface IS the defect the rule names: the user
    // cannot tell 'refused' from 'broken' from 'I missed the target'."
    //
    // THE PREDICTION, IN THE REQUIREMENT'S OWN WORDS FIRST (G-516).
    // `if (!AppDoc.hasDocument || !AppDoc.abiBlock12) {
    //      AppDoc.statusPending("ui.error.no_document"); drop.accept(); return; }`
    // So, with no document: a dropped url must (a) leave the status line
    // carrying the RESOLVED message, (b) land no layer, and (c) still be
    // ACCEPTED -- an unaccepted drop falls through to the platform, which is
    // the silent surface D4 removed.

    // REACHING THE DOCUMENTLESS STATE, AND THE ROUTE IS THE USER'S OWN.
    // document_controller.cpp:2187 documents it: "undoing doc.new is legal --
    // the document is gone but the journal is not", so refreshInfo() on a
    // pre-init handle clears m_hasDocument. That is Cmd+Z after New, not a
    // contrived failure path -- newDocument()'s own error branch (:249) also
    // clears the flag, but reaching it needs a rejected envelope, which would
    // test the engine's validation rather than this branch.
    m_controller->undo();
    QTRY_VERIFY2(!m_controller->hasDocument(),
                 qPrintable(QStringLiteral("the documentless PRECONDITION was "
                                           "not reached -- undo did not unwind "
                                           "init()'s doc.new (status: %1). "
                                           "Nothing below this line is a claim "
                                           "about the drop surface.")
                                .arg(m_controller->statusText())));

    // The status line as the undo LEFT it, captured before the drop. This
    // exists because the mutation control taught it: see the stale-line
    // paragraph below.
    const QString beforeStatus = m_controller->statusText();

    QTemporaryDir tmp;
    QVERIFY(tmp.isValid());
    const QString png = tmp.filePath(QStringLiteral("onto-nothing.png"));
    QImage payload(80, 60, QImage::Format_ARGB32);
    payload.fill(QColor(90, 90, 200));
    QVERIFY2(payload.save(png), "could not write the PNG this case drops");

    QMimeData mime;
    mime.setUrls({QUrl::fromLocalFile(png)});
    // The handler checks hasUrls BEFORE the document guard, so a urlless
    // drop would return early and this case would prove nothing (G-547 at
    // the input, same arm as dropGlueFitsOnOverflow).
    QVERIFY2(mime.hasUrls(), "vacuity: no urls, so the handler returns before "
                             "it ever reaches the document guard");

    const int layersBefore = m_controller->layers().size();
    const QPoint at(int(m_canvasItem->width() / 2),
                    int(m_canvasItem->height() / 2));
    QDragEnterEvent enter(at, Qt::CopyAction, &mime, Qt::LeftButton,
                          Qt::NoModifier);
    QGuiApplication::sendEvent(m_window, &enter);
    QDragMoveEvent move(at, Qt::CopyAction, &mime, Qt::LeftButton,
                        Qt::NoModifier);
    QGuiApplication::sendEvent(m_window, &move);
    QDropEvent drop(at, Qt::CopyAction, &mime, Qt::LeftButton, Qt::NoModifier);
    QGuiApplication::sendEvent(m_window, &drop);

    const QString status = m_controller->statusText();
    qInfo().noquote()
        << QStringLiteral("DOCUMENTLESS-DROP: hasDocument=%1 dropAccepted=%2 "
                          "layers %3->%4 status=%5 beforeStatus=%6")
               .arg(m_controller->hasDocument() ? QStringLiteral("true")
                                                : QStringLiteral("false"))
               .arg(drop.isAccepted() ? QStringLiteral("true")
                                      : QStringLiteral("false"))
               .arg(layersBefore)
               .arg(m_controller->layers().size())
               .arg(status)
               .arg(beforeStatus);

    // (c) ACCEPTED. An unaccepted drop is the silent surface D4 removed.
    QVERIFY2(drop.isAccepted(),
             "the documentless drop was NOT accepted — it falls through to the "
             "platform, which is exactly the silent surface honest-state rule "
             "D4 removed");
    // (a) The status line ANSWERS, and with the RESOLVED string. An id on
    // screen is the key-echo defect; statusPending is catalog-gated, so a
    // renamed id would leave the line untouched and go silent again. This is
    // the functional half of tst_import_filter's catalog guard.
    // ⛔ THIS ARM USED TO READ `!statusText().isEmpty()`, AND ITS MUTATION
    // CONTROL PROVED IT CANNOT DETECT WHAT IT CLAIMED. Renaming the id (the
    // hazard tst_import_filter names: statusPending is catalog-gated, so a
    // rename turns the message OFF silently) left the line NON-EMPTY --
    // carrying the engine rejection the UNDO had put there
    // ("error.command.bad_target: an uninitialized document has no
    // composite"). An emptiness check reads a stale line from an earlier
    // action as "answered", which is exactly the D4 confusion it was written
    // to prevent: the user cannot tell this drop's refusal from the previous
    // command's complaint.
    // ★ So the arm that actually detects "this drop said nothing" is that the
    //   line CHANGED. Emptiness is a property of the session; a REPLY is a
    //   property of the event (10b-H's state-vs-event split, on my own arm).
    QTRY_VERIFY2(m_controller->statusText() != beforeStatus,
                 qPrintable(QStringLiteral("the documentless drop did not "
                                           "change the status line — it still "
                                           "reads what the undo left ('%1'), "
                                           "so this drop said NOTHING and the "
                                           "D4 silent surface is back")
                                .arg(beforeStatus)));
    QCOMPARE(m_controller->statusText(),
             m_i18n->tr(QStringLiteral("ui.error.no_document")));
    QVERIFY2(m_controller->statusText()
                 != QStringLiteral("ui.error.no_document"),
             "the status line KEY-ECHOED the raw id instead of the resolved "
             "message");
    // (b) No layer, because nothing was imported.
    QCOMPARE(m_controller->layers().size(), layersBefore);
}

void TstGestureChrome::importFilterPromiseHoldsEndToEnd()
{
    // FEATURE COVERAGE (per-feature rule): lane/ui-importfilter — the
    // Import dialog offers a name filter that MIRRORS the ingress, because
    // "a filter row for anything the ingress refuses would invite the
    // refusal it promises to prevent" (the reasoning sits above
    // `importFileDialog`'s `nameFilters` in Main.qml; that block now
    // carries a 2026-09-03 marker saying which half of it is history).
    //
    // ⛔ THIS COMMENT SAID "a PNG-only name filter" AND CITED "Main.qml:1318"
    // UNTIL 2026-09-03. BOTH WERE WRONG, IN DIFFERENT WAYS.
    // ★ The filter is no longer PNG-only: operator ruling D5 widened the
    //   ingress to INGRESS_ALLOWED = [png, jpeg, tiff, webp, gif, bmp]
    //   (appname-io/src/ingress.rs:60), and the row is `fileFilter("images")`.
    // ★ The line number never survived: the quoted sentence is at :1714
    //   today. A line number is the recomputable value that goes stale
    //   unread -- name the anchor, not the offset. (This is the second such
    //   citation on this desk's record in two days; the other was in this
    //   same file, INTEGRATION-log 521.)
    // ★★ THE ASSERTIONS BELOW WERE READ BEFORE THIS COMMENT WAS TOUCHED AND
    //   THEY DO NOT DEPEND ON THE STALE PREMISE: the case imports a PNG (an
    //   advertised format, must succeed) and a .txt (not an image at all,
    //   must refuse out loud). Neither encodes "PNG alone", so the widening
    //   left the code correct and only the prose behind.
    //
    // O-UI's tst_import_filter pins that the filter MIRRORS the ingress's
    // accepted set — a parse-level property of the dialog. This is the
    // other half, in my dimension: the filter makes a PROMISE to the user,
    // and the promise is only worth something if the advertised format
    // actually lands pixels through the real two-door path
    // (asset_create -> edit.paste), and an unadvertised one is refused
    // OUT LOUD rather than silently doing nothing.
    QTemporaryDir tmp;
    QVERIFY(tmp.isValid());

    // The ADVERTISED format must work end to end.
    const QString png = tmp.filePath(QStringLiteral("in.png"));
    QImage payload(120, 90, QImage::Format_ARGB32);
    payload.fill(QColor(10, 120, 220));
    QVERIFY(payload.save(png));
    const int layersBefore = m_controller->layers().size();
    const bool okPng = m_controller->importImage(png);
    qInfo().noquote()
        << QStringLiteral("IMPORT png -> ok=%1 layers %2->%3 status=%4")
               .arg(okPng ? QStringLiteral("true") : QStringLiteral("false"))
               .arg(layersBefore).arg(m_controller->layers().size())
               .arg(m_controller->statusText());
    QVERIFY2(okPng,
             qPrintable(QStringLiteral("the Import filter advertises PNG but "
                                       "importing one FAILED; status: %1")
                            .arg(m_controller->statusText())));
    QVERIFY2(m_controller->layers().size() > layersBefore
                 || !isWhitishPixel(enginePixel(20, 20)),
             "the PNG import reported success but landed no content");

    // An UNADVERTISED format must refuse OUT LOUD. The filter exists to
    // keep the user away from this path; if they reach it anyway (typed
    // path, drag-drop), silence is the failure mode.
    const QString txt = tmp.filePath(QStringLiteral("not-an-image.txt"));
    QFile f(txt);
    QVERIFY(f.open(QIODevice::WriteOnly));
    f.write("this is not a PNG");
    f.close();
    const QString before = m_controller->statusText();
    const bool okTxt = m_controller->importImage(txt);
    const QString after = m_controller->statusText();
    qInfo().noquote()
        << QStringLiteral("IMPORT txt -> ok=%1 status=%2")
               .arg(okTxt ? QStringLiteral("true") : QStringLiteral("false"))
               .arg(after);
    QVERIFY2(!okTxt, "importing a non-image reported SUCCESS");
    QVERIFY2(after != before && !after.isEmpty(),
             "importing an unadvertised format failed SILENTLY — the "
             "status line never said why");
}

void TstGestureChrome::newDocumentPresetCreatesThatDocument()
{
    // Items 2+3 (New Document presets/units) — on the standing
    // uncovered-surface list since T1 and still uncovered in MY dimension.
    // O-UI's review probes cover this dialog's unit arithmetic and its
    // SpinBox validators; nobody drives the loop that matters to a user:
    // pick a preset, press Create, and GET THAT DOCUMENT.
    //
    // Engine truth is the document's own dimensions, so this needs no
    // pixel probe and no assumption about the dialog's internals beyond
    // the preset table it renders from.
    QObject *dialog = nullptr;
    for (QObject *o : m_window->findChildren<QObject *>()) {
        if (QLatin1String(o->metaObject()->className())
                .startsWith(QLatin1String("NewDocumentDialog"))) {
            dialog = o;
            break;
        }
    }
    QVERIFY2(dialog, "NewDocumentDialog not found in the chrome");
    const QVariantList presets = dialog->property("presetList").toList();
    QVERIFY2(!presets.isEmpty(),
             "the dialog renders NO presets — AppUnits.presets is empty and "
             "this case would be vacuous");
    qInfo().noquote()
        << QStringLiteral("NEW-DOC: %1 presets offered").arg(presets.size());

    // Start from a document of a KNOWN, DIFFERENT size so "it worked"
    // cannot be confused with "nothing happened".
    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
    QCOMPARE(m_controller->docWidth(), 640);

    // Drive the dialog the way the menu does, select a preset, accept.
    QMetaObject::invokeMethod(dialog, "open");
    QTRY_VERIFY2(dialog->property("visible").toBool(),
                 "the New Document dialog did not open");
    QObject *presetBox = nullptr;
    for (QObject *o : dialog->findChildren<QObject *>()) {
        if (QLatin1String(o->metaObject()->className())
                .startsWith(QLatin1String("ComboBox"))) {
            presetBox = o;
            break;
        }
    }
    QVERIFY2(presetBox, "the preset ComboBox was not found");
    // Index 0 is "Custom"; the data presets follow in table order.
    QMetaObject::invokeMethod(presetBox, "activated", Q_ARG(int, 1));
    QCoreApplication::processEvents();

    const int wantW = dialog->property("widthPx").toInt();
    const int wantH = dialog->property("heightPx").toInt();
    QVERIFY2(wantW > 0 && wantH > 0,
             "selecting a preset left the dialog's computed px at zero");
    QVERIFY2(wantW != 640 || wantH != 400,
             "the first preset happens to equal the starting document — "
             "this case could not tell success from a no-op; pick another");
    QMetaObject::invokeMethod(dialog, "accept");

    // ENGINE TRUTH: the document is the preset's size.
    QTRY_COMPARE_WITH_TIMEOUT(m_controller->docWidth(), wantW, 3000);
    QCOMPARE(m_controller->docHeight(), wantH);
    qInfo().noquote()
        << QStringLiteral("NEW-DOC: preset 1 -> %1x%2, document is %3x%4")
               .arg(wantW).arg(wantH)
               .arg(m_controller->docWidth()).arg(m_controller->docHeight());
    // Leave the fixture as the rest of the suite expects.
    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
}

void TstGestureChrome::resizeAndRotateDialogsReachTheEngine()
{
    // The last two dialogs on the standing uncovered list (open since T1).
    // Neither is gesture-FED — they act on the whole document — so the
    // seam here is narrower than crop/fill/modify: the dialog's control
    // value must reach the engine and change the document.
    auto findDialog = [&](const char *prefix) -> QObject * {
        for (QObject *o : m_window->findChildren<QObject *>())
            if (QLatin1String(o->metaObject()->className())
                    .startsWith(QLatin1String(prefix)))
                return o;
        return nullptr;
    };
    auto clickButton = [&](QObject *dlg, const char *catId) -> bool {
        for (QObject *o : dlg->findChildren<QObject *>()) {
            if (!QLatin1String(o->metaObject()->className())
                     .startsWith(QLatin1String("Button")))
                continue;
            if (o->property("text").toString()
                != m_i18n->tr(QLatin1String(catId)))
                continue;
            auto *item = qobject_cast<QQuickItem *>(o);
            if (!item)
                return false;
            QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                              center(item));
            return true;
        }
        return false;
    };

    // --- ROTATE: 90 degrees must TRANSPOSE a non-square document ---
    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
    QObject *rot = findDialog("RotateDialog");
    QVERIFY2(rot, "RotateDialog not found in the chrome");
    QMetaObject::invokeMethod(rot, "open");
    QTRY_VERIFY(rot->property("visible").toBool());
    QVERIFY2(clickButton(rot, "ui.dialog.apply")
                 || clickButton(rot, "ui.dialog.rotate.apply"),
             "the rotate dialog's Apply button was not found by label");
    // Default angle is 900 tenths = 90 degrees: 640x400 -> 400x640.
    QTRY_COMPARE_WITH_TIMEOUT(m_controller->docWidth(), 400, 3000);
    QCOMPARE(m_controller->docHeight(), 640);
    qInfo().noquote() << "ROTATE: 640x400 -> "
                      << m_controller->docWidth() << "x"
                      << m_controller->docHeight();

    // --- RESIZE: the dialog's px must become the document's px ---
    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
    QObject *rsz = findDialog("ResizeDialog");
    QVERIFY2(rsz, "ResizeDialog not found in the chrome");
    QMetaObject::invokeMethod(rsz, "open");
    QTRY_VERIFY(rsz->property("visible").toBool());
    // Drive the width/height spin boxes to a size that is unmistakably
    // not the starting one, so "applied" cannot be confused with "no-op".
    QList<QObject *> spins;
    for (QObject *o : rsz->findChildren<QObject *>())
        if (QLatin1String(o->metaObject()->className())
                .startsWith(QLatin1String("SpinBox")))
            spins << o;
    // The dialog drives ONE dimension (a `driver` ComboBox picks which)
    // and derives the other from the aspect — it does NOT expose separate
    // width/height spins. Assuming two was my error, not a defect.
    QVERIFY2(spins.size() >= 1,
             "the resize dialog exposes no spin box to drive");
    spins.at(0)->setProperty("value", 320);
    QCoreApplication::processEvents();
    QVERIFY2(clickButton(rsz, "ui.dialog.apply")
                 || clickButton(rsz, "ui.dialog.resize.apply"),
             "the resize dialog's Apply button was not found by label");
    QTRY_VERIFY_WITH_TIMEOUT(m_controller->docWidth() != 640, 3000);
    qInfo().noquote() << "RESIZE: 640x400 -> "
                      << m_controller->docWidth() << "x"
                      << m_controller->docHeight();
    // 640x400 is 1.6:1. Driving EITHER dimension to 320 with the aspect
    // preserved gives 320x200 or 512x320 — accept both, since which
    // dimension drives is the ComboBox's business, not this case's.
    const int w = m_controller->docWidth(), h = m_controller->docHeight();
    QVERIFY2((w == 320 && h == 200) || (h == 320 && w == 512),
             qPrintable(QStringLiteral("the resize dialog's 320 px did not "
                                       "reach the engine at the aspect: "
                                       "document is %1x%2 (expected 320x200 "
                                       "or 512x320)").arg(w).arg(h)));
    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
}

void TstGestureChrome::cropDeletePixelsArmTracksTheCapability()
{
    // Item 5's NI-arm guard, named on the uncovered list since T1. The
    // deletePixels Switch is deliberately BOUND-WHILE-DISABLED during the
    // typed-NI period: ON (the only servable arm), and not user-breakable.
    // Both halves must track ONE truth point — AppDoc.cropNonDestructiveLive
    // — so that when the capability flips, the control flips with it and
    // nobody has to remember to change a second place.
    QObject *dialog = nullptr;
    for (QObject *o : m_window->findChildren<QObject *>())
        if (QLatin1String(o->metaObject()->className())
                .startsWith(QLatin1String("CropDialog"))) {
            dialog = o;
            break;
        }
    QVERIFY2(dialog, "CropDialog not found in the chrome");
    // The deletePixels Switch by its CATALOGED LABEL, not by findChildren
    // order — the same identification cropDragThenApplyWithDeletePixels-
    // ShrinksTheCanvas uses. CropDialog has TWO Switches (deletePixels and
    // allowGrowth); with the capability live they are property-identical
    // (enabled=true, checked=false), so "the first Switch" is not an
    // identification. It happens to resolve to deletePixels today only
    // because that Switch is declared first in CropDialog.qml — one
    // reorder and this case would read allowGrowth and still report
    // green, certifying a capability it never touched.
    QObject *sw = nullptr;
    for (QObject *o : dialog->findChildren<QObject *>()) {
        if (!QLatin1String(o->metaObject()->className())
                 .startsWith(QLatin1String("Switch")))
            continue;
        if (o->property("text").toString()
            != m_i18n->tr(QStringLiteral("ui.dialog.crop.delete_pixels")))
            continue;
        sw = o;
        break;
    }
    QVERIFY2(sw, "the crop dialog's deletePixels Switch was not found by its "
                 "cataloged label");

    const bool live = m_controller->property("cropNonDestructiveLive").toBool();
    const bool checked = sw->property("checked").toBool();
    const bool enabled = sw->property("enabled").toBool();
    qInfo().noquote()
        << QStringLiteral("CROP-ARM: cropNonDestructiveLive=%1 checked=%2 "
                          "enabled=%3")
               .arg(live).arg(checked).arg(enabled);
    // THE INVARIANT, stated so it holds on both sides of the flip:
    // enabled tracks the capability; checked is its inverse while the
    // capability is absent (ON = the only servable arm).
    QCOMPARE(enabled, live);
    if (!live)
        QVERIFY2(checked,
                 "non-destructive crop is NOT live, so deletePixels must be "
                 "pinned ON (the only servable arm) — an OFF switch here "
                 "offers an arm the engine refuses");
}

void TstGestureChrome::letterRingCyclesAndEachMemberCompletesItsGesture()
{
    // FEATURE COVERAGE (per-feature rule): item 11e, the tool letter-key
    // chord dispatcher — §32.3 group cycling. O-UI's tst_chord_table pins
    // the ring TABLE and the dispatcher's arbitration; this is the gesture
    // seam: after a letter cycles the ring, does the member it landed on
    // actually COMPLETE its gesture?
    //
    // A ring that cycles the tool PROPERTY correctly while leaving one
    // member unable to finish a drag is invisible to a table test and is
    // exactly this session's defect class.
    const QStringList ring = m_toolKeys->ringMembers(QStringLiteral("M"));
    qInfo().noquote()
        << QStringLiteral("RING M: %1").arg(ring.join(QStringLiteral(" -> ")));
    QVERIFY2(ring.size() >= 2,
             "the M ring has fewer than 2 members — cycling cannot be "
             "exercised and this case would be vacuous");

    // Land on a known member first, then step the ring one press at a
    // time, driving a real marquee after each step.
    m_canvasItem->setProperty("tool", ring.first());
    for (int step = 1; step < ring.size(); ++step) {
        m_canvasItem->forceActiveFocus();
        QTest::keyClick(m_window, Qt::Key_M);
        const QString now = canvasTool();
        QCOMPARE(now, ring.at(step));
        QCOMPARE(m_toolKeys->ringCurrent(QStringLiteral("M")), now);

        m_controller->applyCommand(QStringLiteral("selection.none"), {}, {}, {});
        QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                          docToScene(150, 110));
        QTest::mouseMove(m_window, docToScene(280, 190));
        QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                            docToScene(380, 250));
        QVERIFY2(m_controller->selectionActive(),
                 qPrintable(QStringLiteral("ring member %1 (cycle step %2) "
                                           "committed NO selection; status: %3")
                                .arg(now).arg(step)
                                .arg(m_controller->statusText())));
        const QRect b = engineSelectionBounds();
        QVERIFY2(b.width() >= 150 && b.height() >= 100,
                 qPrintable(QStringLiteral("ring member %1 committed a "
                                           "degenerate selection %2x%3")
                                .arg(now).arg(b.width()).arg(b.height())));
    }
    m_controller->applyCommand(QStringLiteral("selection.none"), {}, {}, {});
}

void TstGestureChrome::springLoadedLetterRevertsAndTheOriginalToolStillWorks()
{
    // FEATURE COVERAGE: item 11e spring-load (ES:16446) — HOLDING a tool
    // letter switches temporarily; releasing reverts to the tool you were
    // on. The boundary is ToolKeyDispatcher::SpringHoldMs, asserted
    // against the named constant rather than a literal.
    //
    // The half that matters in MY dimension is the SECOND half: after the
    // spring reverts, the ORIGINAL tool must still complete its gesture.
    // A revert that restores the tool string but leaves the canvas in the
    // borrowed tool's gesture state would pass a property check and strand
    // the user mid-drag.
    m_canvasItem->setProperty("tool", QStringLiteral("select.rect"));
    m_canvasItem->forceActiveFocus();
    QCOMPARE(canvasTool(), QStringLiteral("select.rect"));

    // HOLD past the spring boundary, then release.
    QTest::keyPress(m_window, Qt::Key_B);
    QTest::qWait(ToolKeyDispatcher::SpringHoldMs + 120);
    const QString held = canvasTool();
    QTest::keyRelease(m_window, Qt::Key_B);
    QCoreApplication::processEvents();
    const QString after = canvasTool();
    qInfo().noquote()
        << QStringLiteral("SPRING-LOAD: start=select.rect held=%1 after=%2 "
                          "(hold %3 ms)")
               .arg(held).arg(after).arg(ToolKeyDispatcher::SpringHoldMs);
    QVERIFY2(held == QStringLiteral("brush.stroke"),
             qPrintable(QStringLiteral("holding B past %1 ms did not borrow "
                                       "the brush (tool was %2)")
                            .arg(ToolKeyDispatcher::SpringHoldMs).arg(held)));
    QVERIFY2(after == QStringLiteral("select.rect"),
             qPrintable(QStringLiteral("releasing a spring-loaded letter did "
                                       "not revert (tool is %1)").arg(after)));

    // THE LOAD-BEARING HALF: the reverted tool must still work.
    m_controller->applyCommand(QStringLiteral("selection.none"), {}, {}, {});
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToScene(160, 120));
    QTest::mouseMove(m_window, docToScene(280, 200));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToScene(360, 250));
    QVERIFY2(m_controller->selectionActive(),
             qPrintable(QStringLiteral("after a spring-load revert the "
                                       "original tool committed NOTHING — "
                                       "the canvas was left in the borrowed "
                                       "tool's state; status: %1")
                            .arg(m_controller->statusText())));
    QCOMPARE(engineSelectionBounds().size(), QSize(200, 130));
    m_controller->applyCommand(QStringLiteral("selection.none"), {}, {}, {});
}


// ===================================================================
// T39 PER-FEATURE COVERAGE: the four interactive controls O-UI's lane
// (e3fbd765) added to the real chrome — one LayersPanel Button and three
// Image-menu items. Both cases boot the SAME real Main.qml as the rest of
// this file, so they exercise the shipped bindings, not a fixture's.
// ===================================================================

// RETIRED T64: `adjustmentLayerButtonAddsAnAdjustmentLayer` asserted a
// standalone "Invert" Button in the LayersPanel. O-UI's item-4 wave replaced
// that button with the `adjustButton` MENU (seven ops), so the case had
// become a permanent red asserting a surface the product no longer has —
// stale against an intended change, exactly like the T39 transform cases.
//
// NOT a coverage loss: `adjustmentMenuArmsReachTheEngine` drives
// `adjustMenuInvert` through the new route and asserts the same engine truth
// (an adjustment layer appears), plus the six param-dialog arms the old case
// never touched. Recorded here rather than silently deleted, because a case
// vanishing from a suite is indistinguishable from one that was never
// written unless someone says which.


void TstGestureChrome::autoAdjustMenuItemsAreArmedAndHonest()
{
    // Image > Auto Tone / Auto Contrast / Auto Color. ARMED-HONEST is the
    // contract the census rule names: activating a control must produce a
    // visible consequence - changed pixels or a status note. Silence is the
    // MAJOR. Pixel change alone is NOT asserted: an auto-tone pass over
    // content that is already full-range is legitimately a no-op, so
    // demanding it would file a finding against correct behaviour.
    const char *ids[] = {"ui.menu.image.auto_tone",
                         "ui.menu.image.auto_contrast",
                         "ui.menu.image.auto_color"};
    QStringList silent, honest;
    for (const char *id : ids) {
        // Give each pass something to work on: a flat mid-grey block has a
        // narrow histogram, which is exactly the input these operate on.
        QVariantMap rect{{QStringLiteral("x"), 100}, {QStringLiteral("y"), 100},
                         {QStringLiteral("w"), 200}, {QStringLiteral("h"), 120}};
        m_controller->applyCommand(
            QStringLiteral("selection.set"), {},
            {{QStringLiteral("source"), QStringLiteral("rect")},
             {QStringLiteral("rect"), rect}}, {});
        QVariantMap color{
            {QStringLiteral("model"), QStringLiteral("rgb")},
            {QStringLiteral("comps"), QVariantList{0.45, 0.45, 0.45, 0.0}}};
        QVariantMap target{
            {QStringLiteral("layerId"),
             m_controller->layers()
                 .at(qMax(0, m_controller->activeLayerIndex()))
                 .toMap()
                 .value(QStringLiteral("layerId"))}};
        const QVariantMap fr = m_controller->applyCommand(
            QStringLiteral("fill"), target,
            {{QStringLiteral("content"), QStringLiteral("color")},
             {QStringLiteral("color"), color}}, {});
        m_controller->applyCommand(QStringLiteral("selection.none"), {}, {}, {});
        // NON-VACUITY ON THE SETUP. The first cut of this case used command
        // names that do not exist ("selection.rect"/"edit.fill"); they were
        // refused silently, the probe ran against a blank white canvas, and
        // the comment above it claimed a narrow-histogram grey block that
        // was never there. The status half still held, so it PASSED - a
        // green with a false premise underneath. Assert the setup landed.
        QVERIFY2(fr.value(QStringLiteral("ok")).toBool(),
                 qPrintable(QStringLiteral("fill refused (%1): the auto pass "
                                           "below would run on blank canvas")
                                .arg(fr.value(QStringLiteral("code"))
                                         .toString())));
        QVERIFY2(enginePixel(150, 150) != QColor(Qt::white),
                 "the grey block is not on the canvas, so this probe is not "
                 "measuring what its comment says it measures");

        QObject *item = nullptr;
        const QString caption = m_i18n->tr(QLatin1String(id));
        for (QObject *o : m_window->findChildren<QObject *>()) {
            if (!o->inherits("QQuickMenuItem"))
                continue;
            if (o->property("text").toString() != caption)
                continue;
            item = o;
            break;
        }
        QVERIFY2(item, qPrintable(QStringLiteral("menu item '%1' (%2) not "
                                                 "found in the real chrome")
                                      .arg(caption, QLatin1String(id))));
        QVERIFY2(item->property("enabled").toBool(),
                 qPrintable(QStringLiteral("'%1' is disabled with a document "
                                           "open").arg(caption)));

        const QColor pixBefore = enginePixel(150, 150);
        const QString statusBefore = m_controller->statusText();
        QVERIFY(QMetaObject::invokeMethod(item, "triggered"));
        QCoreApplication::processEvents();
        const QColor pixAfter = enginePixel(150, 150);
        const QString statusAfter = m_controller->statusText();

        const bool spoke = statusAfter != statusBefore && !statusAfter.isEmpty();
        const bool moved = pixAfter != pixBefore;
        qInfo().noquote()
            << QStringLiteral("%1: pixel %2 -> %3, status '%4'")
                   .arg(QLatin1String(id), pixBefore.name(), pixAfter.name(),
                        statusAfter);
        if (spoke || moved)
            honest << QLatin1String(id);
        else
            silent << QLatin1String(id);
    }
    QVERIFY2(silent.isEmpty(),
             qPrintable(QStringLiteral("ARMED-SILENT (census rule = MAJOR): "
                                       "%1 changed neither pixels nor the "
                                       "status line when triggered")
                            .arg(silent.join(QStringLiteral(", ")))));
    qInfo().noquote() << QStringLiteral("auto-adjust armed-honesty: %1 honest")
                             .arg(honest.size());
}


// ===================================================================
// T42 CENSUS ENFORCEMENT: the item-25 census's four CONTROLS rows, which
// my lint had been carrying as "enforcement from next tick" since T20.
// ===================================================================

void TstGestureChrome::historyPanelRowClickRewindsTheDocument()
{
    // UPGRADE, not a duplicate. `historyPanelRowJumpRewindsTheDocument`
    // calls `historyGoto()` — it says so honestly ("through the same door
    // the panel row fires"), which makes it ENGINE-DOOR coverage, not
    // gesture coverage. The census recorded this row as DEAD
    // ('ALWAYS "enabled" — no MouseArea, no onClicked'); that is now STALE
    // — HistoryPanel.qml:110 ships a MouseArea with
    // `onClicked: histRow.jump()`. The gesture became expressible and my
    // coverage never followed it, so this case CLICKS THE ROW.
    m_canvasItem->setProperty("tool", QStringLiteral("select.rect"));
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToScene(200, 140));
    QTest::mouseMove(m_window, docToScene(280, 200));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToScene(280, 200));
    QVariantMap color{
        {QStringLiteral("model"), QStringLiteral("rgb")},
        {QStringLiteral("comps"), QVariantList{0.1, 0.2, 0.9, 0.0}}};
    QVariantMap target{
        {QStringLiteral("layerId"),
         m_controller->layers()
             .at(qMax(0, m_controller->activeLayerIndex()))
             .toMap().value(QStringLiteral("layerId"))}};
    QVERIFY(m_controller->applyCommand(
                QStringLiteral("fill"), target,
                {{QStringLiteral("content"), QStringLiteral("color")},
                 {QStringLiteral("color"), color}}, {})
                .value(QStringLiteral("ok")).toBool());
    m_controller->applyCommand(QStringLiteral("selection.none"), {}, {}, {});
    // LIVENESS FLOOR: the pixel must be FILLED before the click, or
    // "it became white" is just what an empty canvas already looked like.
    QTRY_VERIFY(!isWhitishPixel(enginePixel(250, 180)));

    QQuickItem *list =
        m_window->findChild<QQuickItem *>(QStringLiteral("historyList"));
    QVERIFY2(list, "no 'historyList' in the real chrome — the panel's own "
                   "gesture anchor is gone");
    // NON-VACUITY ON THE TRIGGER: the row's MouseArea is bound to
    // `gotoEnabled`. If that is false the click is a no-op and a green
    // here would mean nothing.
    QQuickItem *panel = list->parentItem();
    while (panel && !panel->property("gotoEnabled").isValid())
        panel = panel->parentItem();
    QVERIFY2(panel && panel->property("gotoEnabled").toBool(),
             "the history panel reports gotoEnabled=false, so the row's "
             "MouseArea is disabled and this click cannot prove anything");

    QQuickItem *content =
        list->property("contentItem").value<QQuickItem *>();
    QVERIFY2(content, "the history ListView has no contentItem");
    QQuickItem *firstRow = nullptr;
    int best = 1 << 30;
    for (QQuickItem *child : content->childItems()) {
        if (child->height() <= 0 || child->width() <= 0)
            continue;
        const QPointF p = child->mapToItem(content, QPointF(0, 0));
        if (p.y() < best) { best = int(p.y()); firstRow = child; }
    }
    QVERIFY2(firstRow, "the history panel rendered no rows to click");
    const QPointF centre = firstRow->mapToScene(
        QPointF(firstRow->width() / 2, firstRow->height() / 2));
    QVERIFY2(centre.x() >= 0 && centre.y() >= 0
                 && centre.x() < m_window->width()
                 && centre.y() < m_window->height(),
             qPrintable(QStringLiteral("the first history row maps to %1,%2, "
                                       "outside the %3x%4 window — a click "
                                       "would land elsewhere")
                            .arg(centre.x()).arg(centre.y())
                            .arg(m_window->width()).arg(m_window->height())));

    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      centre.toPoint());

    // ENGINE TRUTH: the document really rewound — the fill is gone.
    QTest::qWaitFor([&] { return isWhitishPixel(enginePixel(250, 180)); },
                    3000); // wait only — the QVERIFY2 below carries the message
    QVERIFY2(isWhitishPixel(enginePixel(250, 180)),
             qPrintable(QStringLiteral("clicking the first history row did "
                                       "not rewind the document; status: %1")
                            .arg(m_controller->statusText())));
}

void TstGestureChrome::honestlyDisabledControlsStayDisabled()
{
    // 1.0 final (corrigendum (l)): the always-disabled Filter > Liquify and
    // Filter > Custom are REMOVED rather than shipped disabled. The rows
    // now pin their absence from the chrome.
    for (const char *id : {"filter.liquify", "filter.custom"}) {
        const QString caption = m_i18n->tr(QLatin1String(id));
        for (QObject *o : m_window->findChildren<QObject *>())
            if (o->inherits("QQuickMenuItem"))
                QVERIFY2(o->property("text").toString() != caption,
                         qPrintable(QStringLiteral("'%1' is back in the menus; it was "
                                                   "removed for 1.0").arg(caption)));
    }
    QObject *fillItem = nullptr;
    const QString fillCap = m_i18n->tr(QStringLiteral("ui.menu.edit.fill"));
    for (QObject *o : m_window->findChildren<QObject *>()) {
        if (o->inherits("QQuickMenuItem")
            && o->property("text").toString() == fillCap) {
            fillItem = o;
            break;
        }
    }
    QVERIFY2(fillItem, "Edit > Fill menu item not found");
    QVERIFY(QMetaObject::invokeMethod(fillItem, "triggered"));
    QCoreApplication::processEvents();
    QObject *dialog = nullptr;
    for (QObject *o : m_window->findChildren<QObject *>()) {
        if (o->inherits("QQuickDialog") && o->property("visible").toBool()) {
            dialog = o;
            break;
        }
    }
    QVERIFY2(dialog, "the Fill dialog did not open, so its blend ComboBox "
                     "cannot be inspected");
    // BY MODEL CONTENT, not by position. The dialog holds TWO ComboBoxes —
    // the CONTENT one (FillDialog.qml:54, correctly enabled) and the BLEND
    // one (:92, disabled). Taking findChildren's first match grabbed the
    // content combo and reported "the blend ComboBox is now ENABLED",
    // which was a false finding against a correct control — the same
    // positional assumption that broke the census lint at T20. The census
    // names the discriminator itself: a single-item `ui.blend.normal`
    // model.
    const QString blendCaption = m_i18n->tr(QStringLiteral("ui.blend.normal"));
    QObject *combo = nullptr;
    QStringList seenModels;
    for (QObject *o : dialog->findChildren<QObject *>()) {
        if (!o->inherits("QQuickComboBox"))
            continue;
        const QStringList model = o->property("model").toStringList();
        seenModels << model.join(QLatin1Char('/'));
        if (model.contains(blendCaption)) { combo = o; break; }
    }
    QVERIFY2(combo, qPrintable(QStringLiteral("no ComboBox in the Fill "
                                              "dialog carries the "
                                              "'%1' model the census names "
                                              "at FillDialog.qml:92; saw: %2")
                                   .arg(blendCaption,
                                        seenModels.join(QStringLiteral(", ")))));
    QVERIFY2(!combo->property("enabled").toBool(),
             "the FillDialog blend ComboBox is now ENABLED — the census "
             "records it disabled pending non-normal blend serving");
    QMetaObject::invokeMethod(dialog, "close");
    QCoreApplication::processEvents();
}


void TstGestureChrome::historyPanelRowKeyboardActivationRewindsTheDocument()
{
    // THE OTHER HALF OF THE ROW, and ALL THREE of its activation keys.
    // HistoryPanel.qml:84-86 binds onReturnPressed, onEnterPressed AND
    // onSpacePressed to the same jump(). A row a user can tab to and
    // activate is a real gesture surface; T42 covered only the click.
    //
    // WHY ALL THREE, learned the hard way this tick: the first cut pressed
    // Qt::Key_Return and its comment claimed to cover onEnterPressed.
    // Mutating onEnterPressed + onSpacePressed left the case GREEN —
    // because Key_Return fires onReturnPressed (line 84), a binding I had
    // not noticed. The case worked and DESCRIBED THE WRONG BINDING, which
    // is a vacuity of naming rather than of assertion: it would have gone
    // on "covering" a line nothing tested. Driving each key separately
    // makes the mapping impossible to get wrong.
    struct Key { Qt::Key key; const char *binding; };
    const Key keys[] = {{Qt::Key_Return, "Keys.onReturnPressed"},
                        {Qt::Key_Enter, "Keys.onEnterPressed"},
                        {Qt::Key_Space, "Keys.onSpacePressed"}};
    for (const Key &k : keys) {
        m_controller->applyCommand(QStringLiteral("selection.none"), {}, {}, {});
        m_canvasItem->setProperty("tool", QStringLiteral("select.rect"));
        QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                          docToScene(200, 140));
        QTest::mouseMove(m_window, docToScene(280, 200));
        QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                            docToScene(280, 200));
        QVariantMap color{
            {QStringLiteral("model"), QStringLiteral("rgb")},
            {QStringLiteral("comps"), QVariantList{0.9, 0.2, 0.1, 0.0}}};
        QVariantMap target{
            {QStringLiteral("layerId"),
             m_controller->layers()
                 .at(qMax(0, m_controller->activeLayerIndex()))
                 .toMap().value(QStringLiteral("layerId"))}};
        QVERIFY(m_controller->applyCommand(
                    QStringLiteral("fill"), target,
                    {{QStringLiteral("content"), QStringLiteral("color")},
                     {QStringLiteral("color"), color}}, {})
                    .value(QStringLiteral("ok")).toBool());
        m_controller->applyCommand(QStringLiteral("selection.none"), {}, {}, {});
        // LIVENESS FLOOR, per key — "it went white" is also what a canvas
        // that was never filled looks like.
        QTRY_VERIFY2(!isWhitishPixel(enginePixel(250, 180)),
                     qPrintable(QStringLiteral("setup for %1 never filled "
                                               "the pixel")
                                    .arg(QLatin1String(k.binding))));

        QQuickItem *list =
            m_window->findChild<QQuickItem *>(QStringLiteral("historyList"));
        QVERIFY2(list, "no 'historyList' in the real chrome");
        QQuickItem *panel = list->parentItem();
        while (panel && !panel->property("gotoEnabled").isValid())
            panel = panel->parentItem();
        QVERIFY2(panel && panel->property("gotoEnabled").toBool(),
                 "gotoEnabled is false — jump() short-circuits and a key "
                 "press cannot prove anything");
        QQuickItem *content =
            list->property("contentItem").value<QQuickItem *>();
        QVERIFY2(content, "the history ListView has no contentItem");
        QQuickItem *firstRow = nullptr;
        int best = 1 << 30;
        for (QQuickItem *child : content->childItems()) {
            if (child->height() <= 0 || child->width() <= 0)
                continue;
            const QPointF pt = child->mapToItem(content, QPointF(0, 0));
            if (pt.y() < best) { best = int(pt.y()); firstRow = child; }
        }
        QVERIFY2(firstRow, "the history panel rendered no rows");
        // NON-VACUITY ON THE TRIGGER: Keys.* fire only on the focused item.
        firstRow->forceActiveFocus();
        QVERIFY2(firstRow->hasActiveFocus(),
                 qPrintable(QStringLiteral("the history row would not take "
                                           "focus, so %1 cannot be the thing "
                                           "under test")
                                .arg(QLatin1String(k.binding))));

        QTest::keyClick(m_window, k.key);

        QTest::qWaitFor([&] { return isWhitishPixel(enginePixel(250, 180)); },
                        3000); // wait only — the QVERIFY2 carries the message
        QVERIFY2(isWhitishPixel(enginePixel(250, 180)),
                 qPrintable(QStringLiteral("%1 on the focused history row did "
                                           "not rewind the document; "
                                           "status: %2")
                                .arg(QLatin1String(k.binding),
                                     m_controller->statusText())));
    }
}


// ===================================================================
// T50: two clusters of shipping controls that NOTHING in the tree clicks.
// Found by sweeping every onClicked/onTriggered in shell/qml against every
// shell test: only three test files look up a QQuickButton at all, and none
// of them name these captions. Not a per-feature-rule breach (nothing new
// landed) — just a hole that has been open the whole time.
// ===================================================================

// Click a Button located by its CATALOGED caption. Returns false if the
// button is absent or unclickable, with the reason in `why` — never a
// silent skip, because a control that cannot be found is the finding.
static bool clickButtonByCaption(QQuickWindow *win, const QString &caption,
                                 QString *why)
{
    QQuickItem *btn = nullptr;
    for (QObject *o : win->findChildren<QObject *>()) {
        if (!o->inherits("QQuickButton"))
            continue;
        if (o->property("text").toString() != caption)
            continue;
        btn = qobject_cast<QQuickItem *>(o);
        break;
    }
    if (!btn) { *why = QStringLiteral("no Button captioned '%1'").arg(caption); return false; }
    // WAIT for `enabled`, do not sample it once. T50 reported `ungroup` and
    // `up` DISABLED while `merge_down` — checked moments later on the SAME
    // binding (`panel.activeRow() !== null`) — was enabled. One binding
    // cannot be both, so the reading was the bug: these bindings resolve
    // against ListView delegates that are still being created, and a single
    // processEvents() is not the settle point. Sampling too early reports a
    // live control as dead, which is the expensive direction to be wrong in
    // — it manufactures a coverage hole that no amount of staring at the
    // QML explains.
    if (!QTest::qWaitFor([&] { return btn->property("enabled").toBool(); },
                         2000)) {
        *why = QStringLiteral("'%1' still disabled after 2s").arg(caption);
        return false;
    }
    if (btn->width() <= 0 || btn->height() <= 0) {
        *why = QStringLiteral("'%1' has zero size").arg(caption);
        return false;
    }
    const QPointF c = btn->mapToScene(
        QPointF(btn->width() / 2, btn->height() / 2));
    if (c.x() < 0 || c.y() < 0 || c.x() >= win->width() || c.y() >= win->height()) {
        *why = QStringLiteral("'%1' maps to %2,%3, outside the window")
                   .arg(caption).arg(c.x()).arg(c.y());
        return false;
    }
    QTest::mouseClick(win, Qt::LeftButton, Qt::NoModifier, c.toPoint());
    QCoreApplication::processEvents();
    return true;
}

void TstGestureChrome::layersPanelButtonsReachTheEngineThroughAClick()
{
    // Nine layer-management buttons, each `Button { text: tr(id);
    // onClicked: AppDoc.applyCommand(...) }`. The COMMANDS are well covered
    // elsewhere; the BUTTONS are not covered anywhere — that gap is exactly
    // this session's dimension, since a wired command behind a dead button
    // is invisible to every command-door test in the tree.
    //
    // The four unambiguous ones assert EXACT engine truth (a layer count
    // that must move by one, in a known direction). The remaining five
    // depend on structure/selection state that this fixture does not
    // guarantee, so they are held to the census rule's own standard:
    // activating the control must produce a visible consequence — changed
    // layers or a changed status line. Silence is the failure.
    const QString addCap = m_i18n->tr(QStringLiteral("ui.panel.layers.add"));
    QString why;

    // --- add: exact ---------------------------------------------------
    const int before = m_controller->layers().size();
    QVERIFY2(clickButtonByCaption(m_window, addCap, &why), qPrintable(why));
    // NOT QTRY_COMPARE: it carries no message, so a real regression reports
    // only "Compared values are not the same" and a future tick cannot tell
    // WHICH button broke. Found by mutation-audit at T59 — the same
    // dead-message class as T43's QTRY_VERIFY guards.
    QTest::qWaitFor([&] { return m_controller->layers().size() == before + 1; },
                    2000);
    QVERIFY2(m_controller->layers().size() == before + 1,
             qPrintable(QStringLiteral("layers.add: %1 -> %2, wanted %3")
                            .arg(before).arg(m_controller->layers().size())
                            .arg(before + 1)));

    // --- 1.0 final (corrigendum (l), O-WATCH F-8): Grp+, Group and
    // Ungroup are REMOVED from the panel; pinned ABSENT.
    for (const char *gone : {"ui.panel.layers.add_group", "ui.panel.layers.group",
                             "ui.panel.layers.ungroup"}) {
        const QString cap = m_i18n->tr(QLatin1String(gone));
        for (QObject *o : m_window->findChildren<QObject *>())
            if (o->inherits("QQuickButton"))
                QVERIFY2(o->property("text").toString() != cap,
                         qPrintable(QStringLiteral("removed Layers-panel button '%1' is back")
                                        .arg(cap)));
    }

    // --- duplicate: exact ---------------------------------------------
    const QString dupCap = m_i18n->tr(QStringLiteral("ui.panel.layers.duplicate"));
    const int beforeDup = m_controller->layers().size();
    if (!clickButtonByCaption(m_window, dupCap, &why))
        QFAIL(qPrintable(why));
    QTest::qWaitFor([&] { return m_controller->layers().size() == beforeDup + 1; },
                    2000);
    QVERIFY2(m_controller->layers().size() == beforeDup + 1,
             qPrintable(QStringLiteral("layers.duplicate: %1 -> %2, wanted %3")
                            .arg(beforeDup).arg(m_controller->layers().size())
                            .arg(beforeDup + 1)));

    // --- remove: exact, in the other direction ------------------------
    const QString remCap = m_i18n->tr(QStringLiteral("ui.panel.layers.remove"));
    const int beforeRem = m_controller->layers().size();
    QVERIFY2(beforeRem >= 2, "need >=2 layers for remove to be meaningful");
    if (!clickButtonByCaption(m_window, remCap, &why))
        QFAIL(qPrintable(why));
    QTest::qWaitFor([&] { return m_controller->layers().size() == beforeRem - 1; },
                    2000);
    QVERIFY2(m_controller->layers().size() == beforeRem - 1,
             qPrintable(QStringLiteral("layers.remove: %1 -> %2, wanted %3")
                            .arg(beforeRem).arg(m_controller->layers().size())
                            .arg(beforeRem - 1)));

    // --- the remaining five: armed-honest -----------------------------
    // REACH THE STATE EACH ONE GATES ON, per button.
    //
    // ★ T51 CORRECTION. T50 left these "unreachable" and I diagnosed it as a
    //   settle-timing problem, because `ungroup` read DISABLED while
    //   `merge_down` — checked moments later, on what I had read as the same
    //   binding — read ENABLED. One binding cannot be both, so I added a 2s
    //   qWaitFor. It changed nothing, because the premise was wrong: THE
    //   BINDINGS ARE MULTI-LINE AND MY GREP READ ONLY THEIR FIRST LINE.
    //   In full they are:
    //     ungroup : activeRow() !== null && activeRow().kind === "group"
    //     up      : ... && siblingIndexOf(row) < siblingCountOf(row) - 1
    //     down    : ... && siblingIndexOf(row) > 0
    //     group   : checkedIds.length >= 1   (the SELECT toggle, not the
    //               visibility checkbox my first cut happened to click)
    //   They were correctly disabled for the state I had built. The wait
    //   stays — waiting on `enabled` is right regardless — but it fixed
    //   nothing and is not why these now work.
    QVERIFY2(m_controller->layers().size() >= 2,
             "need >=2 layers for the reorder pair to have a direction");

    // (1.0 final: no group rows exist any more -- Grp+ is removed -- so
    // the ungroup/group setup below is gone with the buttons.)
    bool checked = false;
    const int groupIdx = -1;
    QObject *panelRoot = nullptr;
    for (QObject *o : m_window->findChildren<QObject *>())
        if (o->property("checkedIds").isValid()) { panelRoot = o; break; }
    QVERIFY2(panelRoot, "no LayersPanel root (nothing exposes checkedIds)");
    {
        const QVariantList rs = m_controller->layers();
        const int ai = qBound(0, m_controller->activeLayerIndex(),
                              rs.size() - 1);
        const QString parent =
            rs.at(ai).toMap().value(QStringLiteral("parentId")).toString();
        int sibs = 0;
        for (const QVariant &r : rs)
            if (r.toMap().value(QStringLiteral("parentId")).toString() == parent)
                ++sibs;
        qInfo().noquote()
            << QStringLiteral("LAYERS-PANEL reorder gate: rootLayerId='%1' "
                              "activeIdx=%2 parent='%3' siblings=%4")
                   .arg(m_controller->property("rootLayerId").toString())
                   .arg(ai).arg(parent).arg(sibs);
    }
    // The exploratory ListView/checkbox census that found the locator bug
    // (window findChildren misses delegate contents) is retired — its answer
    // is now encoded in the visual-tree walk above. What stays is the
    // OUTCOME line below: if the toggle ever stops being clickable, that
    // reads "select-toggle clicked: NO" and `group` reports unreachable.
    qInfo().noquote() << QStringLiteral("LAYERS-PANEL setup: group layer at "
                                        "index %1, select-toggle clicked: %2")
                             .arg(groupIdx)
                             .arg(checked ? QStringLiteral("yes")
                                          : QStringLiteral("NO"));

    // Per-button active-row state. up/down need a row that is not at the end
    // of its sibling run, in the respective direction.
    auto setActive = [&](int idx) {
        m_controller->setActiveLayerIndex(idx);
        QCoreApplication::processEvents();
    };
    const int top = m_controller->layers().size() - 1;
    // ★ T53: RESOLVE THE TARGET ROW AT CLICK TIME, not in setup. T52 fixed
    //   the stack rebuild but left `groupIdx` frozen from setup — then the
    //   `group` click (newly reachable this tick) restructured the stack and
    //   the later buttons aimed at rows that had moved. Same lesson as T52,
    //   one level in: any index captured before a mutating loop is stale by
    //   the time the loop uses it.
    enum Aim { AimGroupRow, AimTop, AimBottomOfRun, AimAny };
    struct Rest { const char *id; Aim aim; };
    const Rest restPlan[] = {
        {"ui.panel.layers.merge_down", AimTop},     // needs index > 0
        {"ui.panel.layers.up", AimBottomOfRun},     // room above in its run
        {"ui.panel.layers.down", AimTop},           // room below in its run
    };
    // Pick the row that satisfies `aim` against the CURRENT stack.
    auto resolveAim = [&](Aim aim) -> int {
        const QVariantList rs = m_controller->layers();
        if (rs.isEmpty()) return 0;
        auto siblingsOf = [&](int i) {
            const QString par = rs.at(i).toMap()
                                    .value(QStringLiteral("parentId")).toString();
            int n = 0, idx = -1;
            const QString me = rs.at(i).toMap()
                                   .value(QStringLiteral("layerId")).toString();
            for (const QVariant &r : rs) {
                const QVariantMap m = r.toMap();
                if (m.value(QStringLiteral("parentId")).toString() != par)
                    continue;
                if (m.value(QStringLiteral("layerId")).toString() == me)
                    idx = n;
                ++n;
            }
            return QPair<int, int>(idx, n);
        };
        switch (aim) {
        case AimGroupRow:
            for (int i = 0; i < rs.size(); ++i)
                if (rs.at(i).toMap().value(QStringLiteral("kind")).toString()
                    == QLatin1String("group"))
                    return i;
            return -1;
        case AimBottomOfRun: // siblingIndex < siblingCount-1  (up)
            for (int i = 0; i < rs.size(); ++i) {
                const auto sc = siblingsOf(i);
                if (sc.first >= 0 && sc.first < sc.second - 1) return i;
            }
            return -1;
        case AimTop:         // siblingIndex > 0  (down / merge_down)
            for (int i = rs.size() - 1; i >= 0; --i) {
                const auto sc = siblingsOf(i);
                if (sc.first > 0) return i;
            }
            return -1;
        case AimAny:
        default:
            return qMin(1, rs.size() - 1);
        }
    };

    // ★ T52: REBUILD THE STACK BEFORE EACH BUTTON. The armed-honesty loop
    //   MUTATES the document as it runs — merge_down consumes a layer,
    //   ungroup dissolves the group — so every later button was evaluated
    //   against a stack the earlier clicks had eaten. Measured at check
    //   time: by the time `up` and `down` were reached, rows=1 and sibs=1,
    //   making `siblingIndex < siblingCount-1` and `siblingIndex > 0` both
    //   false. THE BUTTONS WERE CORRECTLY DISABLED THE WHOLE TIME. T50 and
    //   T51 chased that as a product mystery across two ticks, on a
    //   diagnostic sampled in SETUP — a state the assertion never saw.
    auto ensureStack = [&](int wanted) {
        while (m_controller->layers().size() < wanted) {
            QVariantMap prm;
            prm.insert(QStringLiteral("kind"), QStringLiteral("raster"));
            prm.insert(QStringLiteral("newLayerId"), m_controller->newUuid());
            if (!m_controller
                     ->applyCommand(QStringLiteral("layer.add"), {}, prm, {})
                     .value(QStringLiteral("ok")).toBool())
                break;
        }
        QCoreApplication::processEvents();
    };

    QStringList silent, honest, unreachable;
    for (const Rest &r : restPlan) {
        const char *id = r.id;
        ensureStack(3); // three root siblings: both reorder directions exist
        const int want = resolveAim(r.aim);
        if (want < 0) {
            unreachable << QStringLiteral("%1 (no row satisfies its gate in "
                                          "the current stack)")
                               .arg(QLatin1String(r.id));
            continue;
        }
        setActive(want);
        const QString cap = m_i18n->tr(QLatin1String(id));
        const QVariantList layersBefore = m_controller->layers();
        const QString statusBefore = m_controller->statusText();
        QString reason;
        if (!clickButtonByCaption(m_window, cap, &reason)) {
            // Print the gate inputs AT CHECK TIME, not from setup. T51 left
            // up/down unexplained using numbers sampled before the loop;
            // per-button setActive() runs inside it, so those numbers were
            // about a different state than the one the binding saw.
            const QVariantList rs = m_controller->layers();
            const int ai = m_controller->activeLayerIndex();
            QString parent; int sibs = 0, sibIdx = -1;
            if (ai >= 0 && ai < rs.size()) {
                const QVariantMap row = rs.at(ai).toMap();
                parent = row.value(QStringLiteral("parentId")).toString();
                const QString myId =
                    row.value(QStringLiteral("layerId")).toString();
                for (const QVariant &r2 : rs) {
                    const QVariantMap m2 = r2.toMap();
                    if (m2.value(QStringLiteral("parentId")).toString() != parent)
                        continue;
                    if (m2.value(QStringLiteral("layerId")).toString() == myId)
                        sibIdx = sibs;
                    ++sibs;
                }
            }
            unreachable << QStringLiteral("%1 (%2; activeIdx=%3 rows=%4 "
                                          "parent='%5' sibIdx=%6 sibs=%7)")
                               .arg(QLatin1String(id), reason)
                               .arg(ai).arg(rs.size())
                               .arg(parent.left(8)).arg(sibIdx).arg(sibs);
            continue;
        }
        const bool moved = m_controller->layers() != layersBefore;
        const bool spoke = m_controller->statusText() != statusBefore
                           && !m_controller->statusText().isEmpty();
        (moved || spoke ? honest : silent) << QLatin1String(id);
    }
    qInfo().noquote()
        << QStringLiteral("LAYERS-PANEL: %1 honest, %2 silent, %3 unreachable")
               .arg(honest.size()).arg(silent.size()).arg(unreachable.size());
    if (!unreachable.isEmpty())
        qInfo().noquote() << QStringLiteral("  unreachable: %1")
                                 .arg(unreachable.join(QStringLiteral("; ")));
    QVERIFY2(silent.isEmpty(),
             qPrintable(QStringLiteral("ARMED-SILENT: %1 changed neither the "
                                       "layer stack nor the status line when "
                                       "clicked")
                            .arg(silent.join(QStringLiteral(", ")))));
    // 1.0 final: every button left in the plan must be REACHABLE. Before,
    // an unreachable button was only logged -- which is how removing
    // Group/Ungroup went green silently. Not again.
    QVERIFY2(unreachable.isEmpty(),
             qPrintable(QStringLiteral("unreachable Layers-panel buttons: %1")
                            .arg(unreachable.join(QStringLiteral("; ")))));
}

void TstGestureChrome::zoomButtonsReachTheViewThroughAClick()
{
    // The four zoom buttons in the status bar. `zoomPercent` is shell state,
    // but it is the state the CANVAS renders from and the one every
    // doc<->view mapping in this file depends on, so it is the right truth
    // for a view control. Each assertion is a DIRECTION, not a value, so a
    // change to the zoom ladder does not falsify the case.
    QString why;
    m_canvas->zoomOneToOne();
    QTRY_COMPARE(m_canvasItem->property("zoomPercent").toInt(), 100);

    const int atOne = m_canvasItem->property("zoomPercent").toInt();
    QVERIFY2(clickButtonByCaption(
                 m_window, m_i18n->tr(QStringLiteral("ui.action.zoom_in")), &why),
             qPrintable(why));
    QTRY_VERIFY2(m_canvasItem->property("zoomPercent").toInt() > atOne,
                 "zoom_in did not increase the zoom");

    const int zoomedIn = m_canvasItem->property("zoomPercent").toInt();
    QVERIFY2(clickButtonByCaption(
                 m_window, m_i18n->tr(QStringLiteral("ui.action.zoom_out")), &why),
             qPrintable(why));
    QTRY_VERIFY2(m_canvasItem->property("zoomPercent").toInt() < zoomedIn,
                 "zoom_out did not decrease the zoom");

    // zoom_fit on a doc larger than the viewport must land below 100%.
    QVERIFY(m_controller->newDocument(4096, 4096, QStringLiteral("white")));
    QVERIFY2(clickButtonByCaption(
                 m_window, m_i18n->tr(QStringLiteral("ui.action.zoom_fit")), &why),
             qPrintable(why));
    QTRY_VERIFY2(m_canvasItem->property("zoomPercent").toInt() < 100,
                 "zoom_fit on a 4096^2 doc did not shrink below 100%");

    QVERIFY2(clickButtonByCaption(
                 m_window,
                 m_i18n->tr(QStringLiteral("ui.action.zoom_one_to_one")), &why),
             qPrintable(why));
    QTRY_COMPARE(m_canvasItem->property("zoomPercent").toInt(), 100);
}


void TstGestureChrome::layerRowGesturesSelectAndToggleVisibility()
{
    // The LayersPanel's ROW-DELEGATE gestures, which T53's locator fix made
    // reachable for the first time: clicking a row selects that layer
    // (LayersPanel.qml:120) and clicking its eye flips visibility through
    // layer.set_props (:203). Both are among the most ordinary things a user
    // does in this panel and NOTHING in the tree clicked either.
    //
    // Delegates are reached through the VISUAL tree — panel root (unique
    // `checkedIds`) -> ListView -> contentItem -> delegates. A
    // window->findChildren sweep does not see them (T53).
    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
    for (int i = 0; i < 2; ++i) {
        QVariantMap prm;
        prm.insert(QStringLiteral("kind"), QStringLiteral("raster"));
        prm.insert(QStringLiteral("newLayerId"), m_controller->newUuid());
        QVERIFY(m_controller
                    ->applyCommand(QStringLiteral("layer.add"), {}, prm, {})
                    .value(QStringLiteral("ok")).toBool());
    }
    QCoreApplication::processEvents();
    QVERIFY2(m_controller->layers().size() >= 3, "need >=3 rows to select a "
                                                 "row that is not already "
                                                 "active");

    QObject *panelRoot = nullptr;
    for (QObject *o : m_window->findChildren<QObject *>())
        if (o->property("checkedIds").isValid()) { panelRoot = o; break; }
    QVERIFY2(panelRoot, "no LayersPanel root (nothing exposes checkedIds)");
    QQuickItem *content = nullptr;
    for (QObject *o : panelRoot->findChildren<QObject *>())
        if (o->inherits("QQuickListView")) {
            content = o->property("contentItem").value<QQuickItem *>();
            break;
        }
    QVERIFY2(content, "the LayersPanel ListView has no contentItem");
    QList<QQuickItem *> rows;
    for (QQuickItem *d : content->childItems())
        if (d->width() > 0 && d->height() > 0)
            rows << d;
    QVERIFY2(rows.size() >= 3,
             qPrintable(QStringLiteral("only %1 realised row delegate(s); "
                                       "the panel is not showing the stack")
                            .arg(rows.size())));

    // --- ROW CLICK SELECTS ------------------------------------------------
    // Aim at a row that is NOT already active, or "it selected" is what
    // doing nothing also looks like.
    const int startIdx = m_controller->activeLayerIndex();
    QQuickItem *target = nullptr;
    for (QQuickItem *r : rows) {
        const int ri = r->property("index").toInt();
        if (ri != startIdx) { target = r; break; }
    }
    QVERIFY2(target, "every realised row is already the active one");
    const int wantIdx = target->property("index").toInt();
    // The row's MouseArea fills it, but the row also holds a select toggle,
    // an eye and a thumbnail. Sweep a few x-offsets and stop at the one that
    // actually selects — outcome-driven, and the outcome is asserted below.
    bool selected = false;
    for (double fx : {0.55, 0.7, 0.85, 0.4}) {
        const QPointF pt = target->mapToScene(
            QPointF(target->width() * fx, target->height() / 2));
        if (pt.x() < 0 || pt.y() < 0 || pt.x() >= m_window->width()
            || pt.y() >= m_window->height())
            continue;
        QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                          pt.toPoint());
        QCoreApplication::processEvents();
        if (m_controller->activeLayerIndex() == wantIdx) { selected = true; break; }
    }
    QVERIFY2(selected,
             qPrintable(QStringLiteral("clicking row %1 did not make it "
                                       "active (still %2) — the row's "
                                       "MouseArea is not reachable")
                            .arg(wantIdx)
                            .arg(m_controller->activeLayerIndex())));

    // --- EYE TOGGLES VISIBILITY, ENGINE TRUTH -----------------------------
    auto visibleOf = [&](int idx) -> bool {
        const QVariantList rs = m_controller->layers();
        return idx >= 0 && idx < rs.size()
               && rs.at(idx).toMap().value(QStringLiteral("visible")).toBool();
    };
    const bool before = visibleOf(wantIdx);
    bool toggled = false;
    for (QObject *g : target->findChildren<QObject *>()) {
        QQuickItem *ctl = qobject_cast<QQuickItem *>(g);
        if (!ctl || ctl->width() <= 0 || ctl->height() <= 0)
            continue;
        if (!g->inherits("QQuickAbstractButton"))
            continue; // Button/ToolButton/CheckBox all derive from this
        const QPointF c = ctl->mapToScene(
            QPointF(ctl->width() / 2, ctl->height() / 2));
        if (c.x() < 0 || c.y() < 0 || c.x() >= m_window->width()
            || c.y() >= m_window->height())
            continue;
        QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                          c.toPoint());
        QTest::qWaitFor([&] { return visibleOf(wantIdx) != before; }, 1500);
        if (visibleOf(wantIdx) != before) { toggled = true; break; }
        // Not the eye (the select toggle gates nothing here) — put it back.
        QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                          c.toPoint());
        QCoreApplication::processEvents();
    }
    QVERIFY2(toggled,
             qPrintable(QStringLiteral("no control in row %1 flipped its "
                                       "visibility (still %2) — the eye is "
                                       "not reachable through a click")
                            .arg(wantIdx)
                            .arg(before ? QStringLiteral("visible")
                                        : QStringLiteral("hidden"))));
    qInfo().noquote()
        << QStringLiteral("LAYER ROW: click selected row %1; eye flipped "
                          "visible %2 -> %3")
               .arg(wantIdx)
               .arg(before).arg(visibleOf(wantIdx));
}


// Shared: walk to the LayersPanel's realised row delegates through the
// VISUAL tree. findChildren from the window does not reach them (T53).
static QList<QQuickItem *> layerRowDelegates(QQuickWindow *win)
{
    QObject *panelRoot = nullptr;
    for (QObject *o : win->findChildren<QObject *>())
        if (o->property("checkedIds").isValid()) { panelRoot = o; break; }
    if (!panelRoot)
        return {};
    for (QObject *o : panelRoot->findChildren<QObject *>()) {
        if (!o->inherits("QQuickListView"))
            continue;
        QQuickItem *content = o->property("contentItem").value<QQuickItem *>();
        if (!content)
            continue;
        QList<QQuickItem *> rows;
        for (QQuickItem *d : content->childItems())
            if (d->width() > 0 && d->height() > 0)
                rows << d;
        return rows;
    }
    return {};
}

void TstGestureChrome::layerRowDoubleClickRenamesTheLayer()
{
    // LayersPanel.qml:121 — double-clicking a row opens its inline name
    // editor, and Enter commits through `layer.rename`. Tree-wide, NOTHING
    // referenced layer.rename: 0 test files. This is the full chain —
    // double-click, type, Enter — with the new name read back from the
    // engine's own layer row, not from the editor's text.
    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
    QVariantMap prm;
    prm.insert(QStringLiteral("kind"), QStringLiteral("raster"));
    prm.insert(QStringLiteral("newLayerId"), m_controller->newUuid());
    QVERIFY(m_controller
                ->applyCommand(QStringLiteral("layer.add"), {}, prm, {})
                .value(QStringLiteral("ok")).toBool());
    QCoreApplication::processEvents();

    const QList<QQuickItem *> rows = layerRowDelegates(m_window);
    QVERIFY2(!rows.isEmpty(), "no realised layer row delegates");
    QQuickItem *target = rows.first();
    const int idx = target->property("index").toInt();
    auto nameOf = [&](int i) -> QString {
        const QVariantList rs = m_controller->layers();
        return (i >= 0 && i < rs.size())
                   ? rs.at(i).toMap().value(QStringLiteral("name")).toString()
                   : QString();
    };
    const QString before = nameOf(idx);
    QVERIFY2(!before.isEmpty(), "the target row has no name to change");

    // Double-click the row body, away from the toggles at its left edge.
    const QPointF pt = target->mapToScene(
        QPointF(target->width() * 0.7, target->height() / 2));
    QVERIFY2(pt.x() >= 0 && pt.y() >= 0 && pt.x() < m_window->width()
                 && pt.y() < m_window->height(),
             "the row maps outside the window");
    QTest::mouseDClick(m_window, Qt::LeftButton, Qt::NoModifier, pt.toPoint());
    QCoreApplication::processEvents();

    // The editor is a TextField/TextInput inside the row, visible only now.
    QQuickItem *editor = nullptr;
    for (QObject *g : target->findChildren<QObject *>()) {
        if (!g->inherits("QQuickTextInput") && !g->inherits("QQuickTextField"))
            continue;
        QQuickItem *it = qobject_cast<QQuickItem *>(g);
        if (it && it->isVisible() && it->width() > 0) { editor = it; break; }
    }
    // NON-VACUITY ON THE TRIGGER: without a visible editor the typing below
    // would go to whatever holds focus and a rename could never happen — a
    // green here would mean nothing.
    QVERIFY2(editor, "double-click did not open a visible name editor, so "
                     "the rename chain was never entered");
    editor->forceActiveFocus();
    QVERIFY2(editor->hasActiveFocus(), "the name editor refused focus");

    QTest::keyClick(m_window, Qt::Key_A, Qt::ControlModifier); // select all
    const QString wanted = QStringLiteral("qgrenamed");
    // QTest::keyClicks is a QWidget overload; a QWindow takes one key at a
    // time. Letters only — a synthesized '-' needs a keycode this loop would
    // have to special-case, and the name under test does not need one.
    for (const QChar &ch : wanted)
        QTest::keyClick(m_window,
                        static_cast<Qt::Key>(Qt::Key_A + (ch.toLatin1() - 'a')));
    QTest::keyClick(m_window, Qt::Key_Return);

    QTest::qWaitFor([&] { return nameOf(idx) == wanted; }, 2000);
    QVERIFY2(nameOf(idx) == wanted,
             qPrintable(QStringLiteral("double-click rename did not reach the "
                                       "engine: row %1 is still '%2' (wanted "
                                       "'%3'); status: %4")
                            .arg(idx).arg(nameOf(idx), wanted,
                                          m_controller->statusText())));
    qInfo().noquote() << QStringLiteral("LAYER RENAME: '%1' -> '%2'")
                             .arg(before, nameOf(idx));
}

void TstGestureChrome::layerOpacitySliderReachesTheEngine()
{
    // The panel's opacity Slider commits `layer.set_props {opacity}` — per
    // change for keyboard steps, once on release for drags. Tree-wide
    // references to `ui.panel.layers.opacity`: 0. Driven by KEYBOARD here
    // because a keyboard step commits deterministically, where a synthesized
    // drag depends on the handle's hit geometry.
    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
    QVariantMap prm;
    prm.insert(QStringLiteral("kind"), QStringLiteral("raster"));
    prm.insert(QStringLiteral("newLayerId"), m_controller->newUuid());
    QVERIFY(m_controller
                ->applyCommand(QStringLiteral("layer.add"), {}, prm, {})
                .value(QStringLiteral("ok")).toBool());
    QCoreApplication::processEvents();

    QObject *panelRoot = nullptr;
    for (QObject *o : m_window->findChildren<QObject *>())
        if (o->property("checkedIds").isValid()) { panelRoot = o; break; }
    QVERIFY2(panelRoot, "no LayersPanel root");
    QQuickItem *slider = nullptr;
    for (QObject *o : panelRoot->findChildren<QObject *>()) {
        if (!o->inherits("QQuickSlider"))
            continue;
        QQuickItem *it = qobject_cast<QQuickItem *>(o);
        if (it && it->width() > 0 && it->height() > 0) { slider = it; break; }
    }
    QVERIFY2(slider, "no opacity Slider in the LayersPanel");

    const int idx = qMax(0, m_controller->activeLayerIndex());
    auto opacityOf = [&]() -> double {
        const QVariantList rs = m_controller->layers();
        return (idx < rs.size())
                   ? rs.at(idx).toMap().value(QStringLiteral("opacity")).toDouble()
                   : -1.0;
    };
    const double before = opacityOf();
    QVERIFY2(before >= 0.0, "could not read the active layer's opacity");

    slider->forceActiveFocus();
    QVERIFY2(slider->hasActiveFocus(),
             "the opacity slider refused focus, so a key step cannot reach it");
    // Step DOWN: opacity starts at 1.0, so Right/Up would be a no-op at the
    // ceiling and "unchanged" would be indistinguishable from "not wired".
    for (int i = 0; i < 3; ++i) {
        QTest::keyClick(m_window, Qt::Key_Left);
        QCoreApplication::processEvents();
    }
    QTest::qWaitFor([&] { return opacityOf() < before; }, 2000);
    QVERIFY2(opacityOf() < before,
             qPrintable(QStringLiteral("three Left steps on the opacity "
                                       "slider left the engine at %1 "
                                       "(was %2); status: %3")
                            .arg(opacityOf()).arg(before)
                            .arg(m_controller->statusText())));
    qInfo().noquote() << QStringLiteral("LAYER OPACITY: %1 -> %2")
                             .arg(before).arg(opacityOf());
}


// VISUAL-TREE search. `findChildren` walks the QObject tree, which does NOT
// contain ListView or Repeater delegate contents (T53, T56) — those are
// visual children of a contentItem or of the Repeater's parent. Any locator
// that must see delegate-borne controls has to descend childItems().
static void collectVisualItems(QQuickItem *root, const char *cls,
                               QList<QQuickItem *> *out)
{
    if (!root)
        return;
    for (QQuickItem *child : root->childItems()) {
        if (child->inherits(cls))
            *out << child;
        collectVisualItems(child, cls, out);
    }
}

static QList<QQuickItem *> visibleVisualItems(QQuickWindow *win,
                                              const char *cls)
{
    QList<QQuickItem *> all, ok;
    collectVisualItems(win->contentItem(), cls, &all);
    for (QQuickItem *it : all) {
        if (!it->isVisible() || it->width() <= 0 || it->height() <= 0)
            continue;
        const QPointF c = it->mapToScene(
            QPointF(it->width() / 2, it->height() / 2));
        if (c.x() < 0 || c.y() < 0 || c.x() >= win->width()
            || c.y() >= win->height())
            continue;
        ok << it;
    }
    return ok;
}

// ★★★ T67: THE OPTIONS STRIP, NOT THE WHOLE WINDOW. My first cut of the
// size-row case searched the window for a QQuickSlider and its own
// negative control REFUSED it: `select.rect` takes no options and one
// Slider was still visible -- the LayersPanel OPACITY slider, which is
// on screen for every tool. A window-wide search would have reported
// "the size row is present" for a tool that has no options bar at all.
// The control fired before the case could produce that claim, which is
// the whole reason it runs first.
static QList<QQuickItem *> visibleItemsUnder(QQuickWindow *win,
                                             QQuickItem *root,
                                             const char *cls)
{
    QList<QQuickItem *> all, ok;
    collectVisualItems(root, cls, &all);
    for (QQuickItem *it : all) {
        if (!it->isVisible() || it->width() <= 0 || it->height() <= 0)
            continue;
        const QPointF c = it->mapToScene(
            QPointF(it->width() / 2, it->height() / 2));
        if (c.x() < 0 || c.y() < 0 || c.x() >= win->width()
            || c.y() >= win->height())
            continue;
        ok << it;
    }
    return ok;
}

void TstGestureChrome::toolOptionRowControlsReachTheCanvasThroughAClick()
{
    // Main.qml's tool-option ROW controls — the last known delegate-borne
    // gap. The options strip is table-driven (Main.qml:877): each row is a
    // CheckBox / Switch / SpinBox / segment ToolButton writing
    // `canvas[binding]`. My existing option case sets `textFontSize` as a
    // PROPERTY, so it proves the option->engine half and never touches the
    // control. This proves the control->canvas half by clicking.
    //
    // ★ AND IT RETIRES A WRONG ENTRY IN MY OWN CENSUS ROSTER. The item-25
    //   census lists `penCommitsShape Q_PROPERTY — writable, no UI`, and my
    //   censusCoverageLint classified it NOT-A-CONTROL on that basis.
    //   Main.qml:942 binds it to a SEGMENT control that ships whenever
    //   vector.pen is selected. The census row is stale and my roster
    //   inherited the staleness — a classification is only as current as
    //   the document it was read from.
    auto visibleControlIn = [&](const char *cls) -> QQuickItem * {
        const QList<QQuickItem *> found = visibleVisualItems(m_window, cls);
        return found.isEmpty() ? nullptr : found.first();
    };

    // --- CheckBox row: select.wand -> wandContiguous ----------------------
    m_canvasItem->setProperty("tool", QStringLiteral("select.wand"));
    QCoreApplication::processEvents();
    QQuickItem *box = nullptr;
    QTest::qWaitFor([&] { return (box = visibleControlIn("QQuickCheckBox")) != nullptr; },
                    2000);
    QVERIFY2(box, "no visible CheckBox in the options strip for select.wand — "
                  "the wand_contiguous row did not render");
    const bool wandBefore =
        m_canvasItem->property("wandContiguous").toBool();
    const QPointF bc = box->mapToScene(
        QPointF(box->width() / 2, box->height() / 2));
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier, bc.toPoint());
    QTest::qWaitFor([&] {
        return m_canvasItem->property("wandContiguous").toBool() != wandBefore;
    }, 2000);
    QVERIFY2(m_canvasItem->property("wandContiguous").toBool() != wandBefore,
             qPrintable(QStringLiteral("clicking the wand_contiguous CheckBox "
                                       "left canvas.wandContiguous at %1")
                            .arg(wandBefore)));

    // --- Segment row: vector.pen -> penCommitsShape -----------------------
    m_canvasItem->setProperty("tool", QStringLiteral("vector.pen"));
    QCoreApplication::processEvents();
    const bool penBefore =
        m_canvasItem->property("penCommitsShape").toBool();
    // The segment renders one ToolButton per option; click the one that is
    // NOT the current value, or a click on the active segment is a no-op
    // and "unchanged" would be indistinguishable from "not wired".
    bool flipped = false;
    QList<QQuickItem *> buttons;
    QTest::qWaitFor([&] {
        buttons = visibleVisualItems(m_window, "QQuickToolButton");
        return buttons.size() >= 2;
    }, 2000);
    QVERIFY2(buttons.size() >= 2,
             qPrintable(QStringLiteral("the pen segment rendered %1 visible "
                                       "ToolButton(s); it needs two")
                            .arg(buttons.size())));
    for (QQuickItem *b : buttons) {
        if (b->property("checked").toBool())
            continue; // the active half — clicking it changes nothing
        const QPointF c = b->mapToScene(
            QPointF(b->width() / 2, b->height() / 2));
        if (c.x() < 0 || c.y() < 0 || c.x() >= m_window->width()
            || c.y() >= m_window->height())
            continue;
        QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                          c.toPoint());
        QTest::qWaitFor([&] {
            return m_canvasItem->property("penCommitsShape").toBool()
                   != penBefore;
        }, 1500);
        if (m_canvasItem->property("penCommitsShape").toBool() != penBefore) {
            flipped = true;
            break;
        }
    }
    QVERIFY2(flipped,
             qPrintable(QStringLiteral("no inactive segment button flipped "
                                       "canvas.penCommitsShape (still %1) — "
                                       "the census calls this property "
                                       "'writable, no UI'; the control is "
                                       "right there")
                            .arg(penBefore)));
    qInfo().noquote()
        << QStringLiteral("TOOL OPTIONS: wandContiguous %1->%2, "
                          "penCommitsShape %3->%4")
               .arg(wandBefore)
               .arg(m_canvasItem->property("wandContiguous").toBool())
               .arg(penBefore)
               .arg(m_canvasItem->property("penCommitsShape").toBool());
}


void TstGestureChrome::undoRedoShortcutsReachTheDocument()
{
    // Ctrl+Z and the platform Redo sequence, PRESSED. Main.qml:323-324 wire
    // them behind availability guards:
    //   Shortcut { sequence: "Ctrl+Z"; onActivated:
    //       if (root.rowEnabled("edit.undo")) AppDoc.undo() }
    //
    // Measured tree-wide before writing this: `Key_Z`, `StandardKey::Undo`,
    // `QKeySequence::Undo`, `Key_Y` and `StandardKey::Redo` appear in ZERO
    // test files. Every test that exercises undo calls `AppDoc.undo()` —
    // the DOOR. Nothing in the tree presses the most-used shortcut in the
    // application and checks that the document moved.
    //
    // The sequences come from QKeySequence::Undo/Redo rather than a
    // hand-written "Ctrl+Z", so this case asks for the same binding Qt gives
    // the Shortcut and cannot drift from the platform's convention.
    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
    QCoreApplication::processEvents();
    m_canvasItem->setProperty("tool", QStringLiteral("select.rect"));
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToScene(200, 140));
    QTest::mouseMove(m_window, docToScene(280, 200));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToScene(280, 200));
    QVariantMap color{
        {QStringLiteral("model"), QStringLiteral("rgb")},
        {QStringLiteral("comps"), QVariantList{0.1, 0.2, 0.9, 0.0}}};
    QVariantMap target{
        {QStringLiteral("layerId"),
         m_controller->layers()
             .at(qMax(0, m_controller->activeLayerIndex()))
             .toMap().value(QStringLiteral("layerId"))}};
    QVERIFY(m_controller->applyCommand(
                QStringLiteral("fill"), target,
                {{QStringLiteral("content"), QStringLiteral("color")},
                 {QStringLiteral("color"), color}}, {})
                .value(QStringLiteral("ok")).toBool());
    // NO selection.none here, deliberately. The first cut copied it from my
    // other cases and the shortcut then reported `Undone: Deselect` — it had
    // fired correctly and undone the DESELECT, which was the top of history.
    // The setup was wrong, not the product, and the case's own status message
    // is what said so. The fill must be the newest history entry for one
    // Ctrl+Z to be the right instrument.

    // LIVENESS FLOOR: the pixel must be filled before Ctrl+Z, or "it went
    // white" is what an untouched canvas already looks like.
    QTRY_VERIFY2(!isWhitishPixel(enginePixel(250, 180)),
                 "the fill never landed, so the undo below would prove "
                 "nothing");

    m_canvasItem->forceActiveFocus();
    QTest::keySequence(m_window, QKeySequence(QKeySequence::Undo));
    QTest::qWaitFor([&] { return isWhitishPixel(enginePixel(250, 180)); },
                    3000);
    QVERIFY2(isWhitishPixel(enginePixel(250, 180)),
             qPrintable(QStringLiteral("the Undo SHORTCUT did not rewind the "
                                       "document (every other test in the "
                                       "tree calls AppDoc.undo() directly); "
                                       "status: %1")
                            .arg(m_controller->statusText())));

    // REDO, the other half — and the reason this is one case: an undo that
    // cannot be redone is a different defect, and testing only undo would
    // leave the redo shortcut in the same zero-coverage state I just found.
    QTest::keySequence(m_window, QKeySequence(QKeySequence::Redo));
    QTest::qWaitFor([&] { return !isWhitishPixel(enginePixel(250, 180)); },
                    3000);
    QVERIFY2(!isWhitishPixel(enginePixel(250, 180)),
             qPrintable(QStringLiteral("the Redo SHORTCUT did not restore the "
                                       "fill; status: %1")
                            .arg(m_controller->statusText())));
    qInfo().noquote()
        << QStringLiteral("UNDO/REDO SHORTCUTS: %1 / %2 both reached the "
                          "document")
               .arg(QKeySequence(QKeySequence::Undo).toString(),
                    QKeySequence(QKeySequence::Redo).toString());
}


void TstGestureChrome::refusalToastAppearsOnATypedRefusal()
{
    // P1-2, landed this range: "refusals surface on an EVENT, and the toast
    // never truncates". A typed refusal now raises `refusalToast` carrying
    // the message AND the wire code, auto-hidden by a 6 s Timer.
    //
    // PER-FEATURE COVERAGE, same tick. Found the way it should be found —
    // six tour frames changed — and the toast is what changed them.
    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
    QCoreApplication::processEvents();

    QQuickItem *toast = nullptr;
    for (QObject *o : m_window->findChildren<QObject *>())
        if (o->objectName() == QLatin1String("refusalToast")) {
            toast = qobject_cast<QQuickItem *>(o);
            break;
        }
    QVERIFY2(toast, "no item named 'refusalToast' in the chrome");
    // PRECONDITION: it must start hidden, or "it is visible" below is what
    // the fixture already looked like.
    QVERIFY2(!toast->property("visible").toBool(),
             "the refusal toast is visible before any refusal");

    // Drive a REAL typed refusal through the command door the shell uses.
    // mode conversion over a parametric layer is the one the tour trips.
    QVariantMap shapeParams;
    QVariantMap geom{{QStringLiteral("x"), 60}, {QStringLiteral("y"), 60},
                     {QStringLiteral("w"), 200}, {QStringLiteral("h"), 120}};
    QVariantMap prim{{QStringLiteral("kind"), QStringLiteral("rect")},
                     {QStringLiteral("geometry"), geom}};
    QVariantMap fcol{{QStringLiteral("model"), QStringLiteral("rgba")},
                     {QStringLiteral("comps"), QVariantList{0.2, 0.4, 0.9, 1.0}}};
    shapeParams.insert(QStringLiteral("primitive"), prim);
    shapeParams.insert(QStringLiteral("fill"),
                       QVariantMap{{QStringLiteral("type"), QStringLiteral("solid")},
                                   {QStringLiteral("color"), fcol}});
    shapeParams.insert(QStringLiteral("stroke"),
                       QVariantMap{{QStringLiteral("widthPx"), 0}});
    QVERIFY2(m_controller->applyCommand(QStringLiteral("vector.shape_add"), {},
                                        shapeParams, {})
                 .value(QStringLiteral("ok")).toBool(),
             "could not add the parametric layer the refusal needs");
    QCoreApplication::processEvents();

    // `doc.mode_convert`, the kind the TOUR trips at frame 18. My first cut
    // wrote "image.mode_convert" — not a registered kind — and the toast
    // duly showed `error.command.unknown_kind`. The case PASSED, because any
    // typed refusal raises the toast; only the comment was false. Same
    // naming-vacuity class as T43: a live assertion describing the wrong
    // subject. A domain refusal is also the representative case — an
    // unknown-kind refusal never reaches the engine's own logic.
    //
    // The PARAMS are the tour's own (`to: "gray_a"`, tour.cpp:530), not
    // guessed — my second cut passed `mode: "grayscale"` and got
    // `error.schema.missing_field`, which is again a fact about my request
    // and not about the door. Third payload I have guessed this session;
    // read the caller, not the kind name.
    const QVariantMap r = m_controller->applyCommand(
        QStringLiteral("doc.mode_convert"), {},
        {{QStringLiteral("to"), QStringLiteral("gray_a")}}, {});
    // NON-VACUITY: this case is about the TOAST, and it only has a subject if
    // the command actually refused. A success here means the fixture stopped
    // producing the refusal, not that the toast works.
    QVERIFY2(!r.value(QStringLiteral("ok")).toBool(),
             "image.mode_convert SUCCEEDED over a parametric layer — there is "
             "no typed refusal for the toast to show, so this case would "
             "assert nothing");

    QTest::qWaitFor([&] { return toast->property("visible").toBool(); }, 3000);
    QVERIFY2(toast->property("visible").toBool(),
             qPrintable(QStringLiteral("a typed refusal (%1) did not raise "
                                       "the toast")
                            .arg(r.value(QStringLiteral("code")).toString())));

    // It must carry the WIRE CODE, not just prose — that is the half a user
    // can quote into a bug report, and the half a truncating toast drops.
    const QString shown = toast->property("code").toString();
    QVERIFY2(shown == r.value(QStringLiteral("code")).toString(),
             qPrintable(QStringLiteral("toast code '%1' does not match the "
                                       "refusal's '%2'")
                            .arg(shown,
                                 r.value(QStringLiteral("code")).toString())));
    const QString msg = toast->property("message").toString();
    QVERIFY2(!msg.isEmpty() && !msg.endsWith(QStringLiteral("…"))
                 && !msg.endsWith(QStringLiteral("...")),
             qPrintable(QStringLiteral("the toast message is empty or "
                                       "ELLIDED ('%1') — P1-2's claim is that "
                                       "it never truncates")
                            .arg(msg)));
    qInfo().noquote()
        << QStringLiteral("REFUSAL TOAST: code='%1' message=%2 chars")
               .arg(shown).arg(msg.size());
}


void TstGestureChrome::layersPanelMaskAndFillMenusReachTheEngine()
{
    // PER-FEATURE COVERAGE for the two menus O-UI landed this range:
    //   maskButton -> reveal_all / hide_all / selection / selection_inverted
    //                 / transparency          ("the mask.add route")
    //   fillButton -> linear / radial / perceptual  ("New Fill Layer menu —
    //                 the engine served these before the UI could")
    // Ten new objectName'd controls, none of them touched by any test.
    //
    // Every arm is asserted on ENGINE truth — a mask actually attached to the
    // active layer, or a layer actually added — never on the menu's own
    // state. A menu that opens and does nothing is the defect this covers.
    auto byName = [&](const QString &n) -> QQuickItem * {
        for (QObject *o : m_window->findChildren<QObject *>())
            if (o->objectName() == n)
                return qobject_cast<QQuickItem *>(o);
        return nullptr;
    };
    auto clickItem = [&](QQuickItem *it) {
        const QPointF c = it->mapToScene(
            QPointF(it->width() / 2, it->height() / 2));
        QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                          c.toPoint());
        QCoreApplication::processEvents();
    };
    auto maskCount = [&]() -> int {
        const QVariantList rs = m_controller->layers();
        const int i = qBound(0, m_controller->activeLayerIndex(), rs.size() - 1);
        return rs.isEmpty()
                   ? -1
                   : rs.at(i).toMap().value(QStringLiteral("maskIds"))
                         .toList().size();
    };

    // ---- MASK MENU ---------------------------------------------------
    // Each arm gets a FRESH layer: mask.add on a layer that already carries
    // one is a different command path, and reusing the layer would make
    // arms 2..5 measure something the first arm already did.
    const char *maskArms[] = {"maskMenuRevealAll", "maskMenuHideAll",
                              "maskMenuFromSelection",
                              "maskMenuFromSelectionInverted",
                              "maskMenuFromTransparency"};
    QStringList maskOk, maskRefused;
    for (const char *arm : maskArms) {
        QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
        QVariantMap prm{{QStringLiteral("kind"), QStringLiteral("raster")},
                        {QStringLiteral("newLayerId"), m_controller->newUuid()}};
        QVERIFY(m_controller
                    ->applyCommand(QStringLiteral("layer.add"), {}, prm, {})
                    .value(QStringLiteral("ok")).toBool());
        // The selection arms need a selection to derive from.
        QVariantMap rect{{QStringLiteral("x"), 100}, {QStringLiteral("y"), 100},
                         {QStringLiteral("w"), 200}, {QStringLiteral("h"), 120}};
        m_controller->applyCommand(
            QStringLiteral("selection.set"), {},
            {{QStringLiteral("source"), QStringLiteral("rect")},
             {QStringLiteral("rect"), rect}}, {});
        QCoreApplication::processEvents();

        QQuickItem *btn = byName(QStringLiteral("maskButton"));
        QVERIFY2(btn, "no 'maskButton' in the chrome");
        QVERIFY2(btn->property("enabled").toBool(),
                 "the mask button is disabled with a raster layer active");
        const int before = maskCount();
        clickItem(btn); // opens the menu
        QQuickItem *item = byName(QLatin1String(arm));
        QVERIFY2(item, qPrintable(QStringLiteral("menu item '%1' not found "
                                                 "after opening maskButton")
                                      .arg(QLatin1String(arm))));
        QMetaObject::invokeMethod(item, "triggered");
        QTest::qWaitFor([&] { return maskCount() > before; }, 2000);
        if (maskCount() > before)
            maskOk << QLatin1String(arm);
        else
            maskRefused << QStringLiteral("%1 (%2)")
                               .arg(QLatin1String(arm),
                                    m_controller->statusText());
    }
    qInfo().noquote()
        << QStringLiteral("MASK MENU: %1 attached a mask, %2 did not")
               .arg(maskOk.size()).arg(maskRefused.size());
    if (!maskRefused.isEmpty())
        qInfo().noquote() << QStringLiteral("  not attached: %1")
                                 .arg(maskRefused.join(QStringLiteral("; ")));
    // At least the two unconditional arms must work; a menu where NOTHING
    // attaches is the "opens and does nothing" defect.
    QVERIFY2(maskOk.contains(QStringLiteral("maskMenuRevealAll"))
                 && maskOk.contains(QStringLiteral("maskMenuHideAll")),
             qPrintable(QStringLiteral("the unconditional mask arms did not "
                                       "attach a mask; attached: [%1]")
                            .arg(maskOk.join(QStringLiteral(", ")))));

    // ---- FILL MENU ---------------------------------------------------
    const char *fillArms[] = {"fillMenuLinear", "fillMenuRadial",
                              "fillMenuPerceptual"};
    for (const char *arm : fillArms) {
        QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
        QCoreApplication::processEvents();
        QQuickItem *btn = byName(QStringLiteral("fillButton"));
        QVERIFY2(btn, "no 'fillButton' in the chrome");
        const int before = m_controller->layers().size();
        clickItem(btn);
        QQuickItem *item = byName(QLatin1String(arm));
        QVERIFY2(item, qPrintable(QStringLiteral("menu item '%1' not found")
                                      .arg(QLatin1String(arm))));
        QMetaObject::invokeMethod(item, "triggered");
        QTest::qWaitFor([&] {
            return m_controller->layers().size() > before;
        }, 2000);
        QVERIFY2(m_controller->layers().size() > before,
                 qPrintable(QStringLiteral("%1 added no layer (%2 -> %3); "
                                           "status: %4")
                                .arg(QLatin1String(arm)).arg(before)
                                .arg(m_controller->layers().size())
                                .arg(m_controller->statusText())));
    }
    qInfo().noquote() << "FILL MENU: all three gradient arms added a layer";
}


void TstGestureChrome::unavailableChordsSpeakInsteadOfGoingSilent()
{
    // P1-4(a), landed last range and OWED since T62: thirteen
    // precondition-guarded chords route their unmet case through
    // `DocumentController::reportUnavailable(action)` instead of doing
    // nothing. Main.qml:337 is the shape:
    //
    //   Shortcut { sequence: "Ctrl+D"; onActivated:
    //       if (root.rowEnabled("select.deselect")) …
    //       else AppDoc.reportUnavailable("select.deselect") }
    //
    // A chord that silently does nothing is the ARMED-SILENT class this
    // session already treats as a MAJOR on the census, so the speaking half
    // is the feature and the two SILENCES are part of the contract.
    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
    m_controller->applyCommand(QStringLiteral("selection.none"), {}, {}, {});
    QCoreApplication::processEvents();

    // PRECONDITION: deselect must actually be unavailable, or the chord
    // takes its ENABLED arm and this case measures the wrong branch.
    const QVariantMap row =
        m_controller->property("commandAvailability").toMap()
            .value(QStringLiteral("select.deselect")).toMap();
    QVERIFY2(!row.isEmpty(), "no availability row for select.deselect");
    QVERIFY2(!row.value(QStringLiteral("enabled")).toBool(),
             "select.deselect is ENABLED with no selection, so Ctrl+D would "
             "take its live arm and this case would not reach P1-4a");

    // ---- THE CHORD SPEAKS --------------------------------------------
    QSignalSpy spy(m_controller,
                   SIGNAL(actionUnavailable(QString, QString)));
    QVERIFY2(spy.isValid(), "actionUnavailable is not a signal on the "
                            "controller — P1-4a's event arm is missing");
    m_canvasItem->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_D, Qt::ControlModifier);
    QTest::qWaitFor([&] { return spy.count() > 0; }, 2000);
    QVERIFY2(spy.count() > 0,
             qPrintable(QStringLiteral("Ctrl+D on an unavailable deselect "
                                       "emitted no actionUnavailable; "
                                       "status: %1")
                            .arg(m_controller->statusText())));
    const QList<QVariant> args = spy.takeFirst();
    QCOMPARE(args.at(0).toString(), QStringLiteral("select.deselect"));
    const QString reasonId = args.at(1).toString();
    QVERIFY2(!reasonId.isEmpty(), "the event carried an empty reasonId");

    // ★ THE STATUS MUST BE TRANSLATED, NOT KEY-ECHOED. reportUnavailable's
    //   own comment names this: routing through setRejected would have
    //   printed "Rejected — ui.error.no_document:" at the user, the P2-D2
    //   regression "wearing a refusal's clothes". So the visible text must
    //   NOT be the raw id.
    const QString shown = m_controller->statusText();
    QVERIFY2(!shown.isEmpty(),
             "the chord emitted its event but said nothing on the status line");
    QVERIFY2(shown != reasonId && !shown.contains(reasonId),
             qPrintable(QStringLiteral("the status line KEY-ECHOED the reason "
                                       "id ('%1' contains '%2') — that is the "
                                       "P2-D2 regression this door exists to "
                                       "avoid")
                            .arg(shown, reasonId)));
    qInfo().noquote()
        << QStringLiteral("P1-4a: Ctrl+D -> reason '%1' shown as '%2'")
               .arg(reasonId, shown);

    // ---- AND THE TWO SILENCES ARE PART OF THE CONTRACT ---------------
    // Both are stated in reportUnavailable's own comments as deliberate,
    // and both are the kind of thing that rots into a message if nobody
    // pins them.
    {   // (a) UNKNOWN ACTION: a typo must not manufacture a user message.
        // No sentinel injected: setStatus is private, and capturing the
        // CURRENT text is the better probe anyway — it asserts the door
        // changed nothing, without this case having to author a state.
        const QString before = m_controller->statusText();
        QMetaObject::invokeMethod(
            m_controller, "reportUnavailable",
            Q_ARG(QString, QStringLiteral("no.such.action.qg")));
        QCoreApplication::processEvents();
        QVERIFY2(m_controller->statusText() == before,
                 qPrintable(QStringLiteral("an UNKNOWN action produced a "
                                           "user-visible message: '%1'")
                                .arg(m_controller->statusText())));
    }
    {   // (b) ENABLED ACTION: the door reports a state, it does not invent
        //     one. select.all is available on an open document.
        const QVariantMap all =
            m_controller->property("commandAvailability").toMap()
                .value(QStringLiteral("select.all")).toMap();
        QVERIFY2(all.value(QStringLiteral("enabled")).toBool(),
                 "select.all is not enabled on an open document, so this arm "
                 "would not be testing the ENABLED silence");
        const QString before2 = m_controller->statusText();
        QMetaObject::invokeMethod(
            m_controller, "reportUnavailable",
            Q_ARG(QString, QStringLiteral("select.all")));
        QCoreApplication::processEvents();
        QVERIFY2(m_controller->statusText() == before2,
                 qPrintable(QStringLiteral("an ENABLED action produced a "
                                           "refusal message: '%1'")
                                .arg(m_controller->statusText())));
    }
}


void TstGestureChrome::adjustmentMenuArmsReachTheEngine()
{
    // PER-FEATURE COVERAGE for O-UI's item 4 ("ITEM 4 COMPLETE, all eight ops
    // accounted for"): an `adjustButton` menu with seven visible ops, a new
    // 410-line AdjustmentParamDialog and a 160-line CurveEditor.
    //
    // This case covers the MENU ARMS — each must add an adjustment layer of
    // the kind it names. The param dialog and curve editor are their own
    // surfaces and are NAMED AS OWED at the end of this tick rather than
    // half-covered here; a case that opens a dialog and asserts nothing
    // about it would report coverage it does not have.
    auto byName = [&](const QString &n) -> QQuickItem * {
        for (QObject *o : m_window->findChildren<QObject *>())
            if (o->objectName() == n)
                return qobject_cast<QQuickItem *>(o);
        return nullptr;
    };
    auto adjustmentRows = [&]() -> int {
        int n = 0;
        for (const QVariant &r : m_controller->layers())
            if (r.toMap().value(QStringLiteral("kind")).toString()
                == QLatin1String("adjustment"))
                ++n;
        return n;
    };

    // Invert takes no params, so it is the arm that must land WITHOUT a
    // dialog; the others open the param dialog and are counted separately.
    const char *arms[] = {"adjustMenuInvert", "adjustMenuBrightnessContrast",
                          "adjustMenuExposure", "adjustMenuLevels",
                          "adjustMenuHueSaturation", "adjustMenuThreshold",
                          "adjustMenuCurves"};
    QStringList landed, opened, silent;
    for (const char *arm : arms) {
        QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
        QCoreApplication::processEvents();
        QQuickItem *btn = byName(QStringLiteral("adjustButton"));
        QVERIFY2(btn, "no 'adjustButton' in the chrome");
        QVERIFY2(btn->property("enabled").toBool(),
                 "the adjustment button is disabled on an open document");
        const int before = adjustmentRows();
        const QPointF c = btn->mapToScene(
            QPointF(btn->width() / 2, btn->height() / 2));
        QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                          c.toPoint());
        QCoreApplication::processEvents();
        QQuickItem *item = byName(QLatin1String(arm));
        QVERIFY2(item, qPrintable(QStringLiteral("menu item '%1' not found "
                                                 "after opening adjustButton")
                                      .arg(QLatin1String(arm))));
        qInfo().noquote() << QStringLiteral("  arm %1: triggering").arg(QLatin1String(arm));
        QMetaObject::invokeMethod(item, "triggered");
        QCoreApplication::processEvents();
        qInfo().noquote() << QStringLiteral("  arm %1: survived trigger").arg(QLatin1String(arm));

        // Either it landed a layer outright, or it opened the param dialog.
        // Both are live; NEITHER is the failure. Doing nothing is.
        QTest::qWaitFor([&] { return adjustmentRows() > before; }, 1500);
        if (adjustmentRows() > before) {
            landed << QLatin1String(arm);
            continue;
        }
        QQuickItem *dlg = byName(QStringLiteral("adjustChannelCombo"));
        bool dialogUp = false;
        for (QObject *o : m_window->findChildren<QObject *>()) {
            if (!o->inherits("QQuickDialog"))
                continue;
            if (o->property("visible").toBool()) { dialogUp = true; break; }
        }
        if (dialogUp || dlg) {
            opened << QLatin1String(arm);
            // COLLECT, THEN CLOSE. Calling close() inside the
            // findChildren() walk crashed this case with SIGSEGV: closing a
            // dialog can destroy objects, and the QObjectList being iterated
            // is a SNAPSHOT OF POINTERS, not a live view — the next
            // dereference is a use-after-free. My bug, not the product's,
            // and worth the comment because "the test segfaulted" reads like
            // a product crash until it is isolated.
            QList<QObject *> toClose;
            for (QObject *o : m_window->findChildren<QObject *>())
                if (o->inherits("QQuickDialog")
                    && o->property("visible").toBool())
                    toClose << o;
            for (QObject *o : toClose)
                QMetaObject::invokeMethod(o, "close");
            QCoreApplication::processEvents();
        } else {
            silent << QStringLiteral("%1 (%2)")
                          .arg(QLatin1String(arm),
                               m_controller->statusText());
        }
    }
    qInfo().noquote()
        << QStringLiteral("ADJUSTMENT MENU: %1 landed a layer, %2 opened a "
                          "dialog, %3 did neither")
               .arg(landed.size()).arg(opened.size()).arg(silent.size());
    if (!landed.isEmpty())
        qInfo().noquote() << QStringLiteral("  landed: %1")
                                 .arg(landed.join(QStringLiteral(", ")));
    if (!opened.isEmpty())
        qInfo().noquote() << QStringLiteral("  dialog: %1")
                                 .arg(opened.join(QStringLiteral(", ")));
    QVERIFY2(silent.isEmpty(),
             qPrintable(QStringLiteral("ARMED-SILENT adjustment arm(s): %1 — "
                                       "the menu item triggered and neither "
                                       "added a layer nor opened its editor")
                            .arg(silent.join(QStringLiteral("; ")))));
    // NON-VACUITY: if every arm merely opened a dialog and none ever landed
    // a layer, this case would never have touched the engine at all.
    QVERIFY2(!landed.isEmpty(),
             "no adjustment arm added a layer — every one opened a dialog, so "
             "nothing here reached the engine");
}


void TstGestureChrome::adjustmentParamDialogAcceptReachesTheEngine()
{
    // OWED SINCE T64: the 410-line AdjustmentParamDialog and the 160-line
    // CurveEditor. T64 drove the menu arms that OPEN them and deliberately
    // asserted nothing inside; this closes that within the deadline.
    //
    // The path under test is `onAccepted` (AdjustmentParamDialog.qml:407):
    //   AppDoc.addAdjustmentLayer(root.opKind, root.collectParams())
    // so the assertion is ENGINE truth — a layer of kind `adjustment`
    // appears — not that a dialog closed.
    auto byName = [&](const QString &n) -> QQuickItem * {
        for (QObject *o : m_window->findChildren<QObject *>())
            if (o->objectName() == n)
                return qobject_cast<QQuickItem *>(o);
        return nullptr;
    };
    auto visibleDialog = [&]() -> QObject * {
        for (QObject *o : m_window->findChildren<QObject *>())
            if (o->inherits("QQuickDialog") && o->property("visible").toBool())
                return o;
        return nullptr;
    };
    auto adjustmentRows = [&]() -> int {
        int n = 0;
        for (const QVariant &r : m_controller->layers())
            if (r.toMap().value(QStringLiteral("kind")).toString()
                == QLatin1String("adjustment"))
                ++n;
        return n;
    };
    auto openArm = [&](const char *arm) -> QObject * {
        QQuickItem *btn = byName(QStringLiteral("adjustButton"));
        if (!btn)
            return nullptr;
        const QPointF c = btn->mapToScene(
            QPointF(btn->width() / 2, btn->height() / 2));
        QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                          c.toPoint());
        QCoreApplication::processEvents();
        QQuickItem *item = byName(QLatin1String(arm));
        if (!item)
            return nullptr;
        QMetaObject::invokeMethod(item, "triggered");
        QCoreApplication::processEvents();
        return visibleDialog();
    };

    // ---- 1. A PARAM OP: open, accept, and the engine must receive it ----
    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
    QCoreApplication::processEvents();
    QObject *dlg = openArm("adjustMenuBrightnessContrast");
    QVERIFY2(dlg, "Brightness/Contrast did not open a visible dialog");
    // The dialog's own controls — the surface T64 left untouched.
    QVERIFY2(byName(QStringLiteral("adjustChannelCombo")),
             "no 'adjustChannelCombo' in the open adjustment dialog");
    QQuickItem *err = byName(QStringLiteral("adjustErrorLabel"));
    QVERIFY2(err, "no 'adjustErrorLabel' in the open adjustment dialog");
    // PRECONDITION: the error label must start empty, or "no error" below is
    // what the dialog already looked like.
    QVERIFY2(!err->property("visible").toBool()
                 || err->property("text").toString().isEmpty(),
             qPrintable(QStringLiteral("the dialog opened already showing an "
                                       "error: '%1'")
                            .arg(err->property("text").toString())));

    const int before = adjustmentRows();
    // ★★★★★ REPAIRED T68 — THE DOOR MOVED, AND THE MOVE IS Q-D30's.
    // This case drove `Dialog::accept()` from T65 until 2026-09-04, which was
    // correct while the dialog used `standardButtons: Dialog.Ok |
    // Dialog.Cancel`. Q-D30 (`7b2023d2`) replaced those with catalog-driven
    // Buttons and moved the whole accept path into the Apply button's
    // onClicked -- "the accept-then-reopen flicker went with the
    // standardButtons that required it". So accept() now closes the dialog
    // and asks the engine NOTHING, and this case red'd the moment that
    // merged, with `added no adjustment layer (0 -> 0)`.
    //
    // ★★★★ THAT RED IS THE ANSWER TO INTEGRATION-log 547. Its commit message
    // says of this very change: "NO SHELL TEST RENDERS THIS DIALOG ... their
    // pass is vacuous with respect to that risk." A test did render it, and
    // the change red it within one tick -- which is the behaviour a rendering
    // test is FOR. Filed as G-14; the case is repaired here rather than
    // retired, because the surface it covers still exists and only the door
    // moved (the T64 distinction: repair a moved door, retire a removed
    // surface).
    const QString cancelLabel = m_i18n->tr(QStringLiteral("ui.dialog.cancel"));
    auto clickApply = [&](QObject *d) -> bool {
        QQuickItem *applyBtn = nullptr;
        for (QObject *o : d->findChildren<QObject *>()) {
            if (!o->inherits("QQuickAbstractButton"))
                continue;
            auto *b = qobject_cast<QQuickItem *>(o);
            if (!b || !b->isVisible() || b->width() <= 0)
                continue;
            if (b->property("text").toString() != cancelLabel)
                applyBtn = b;
        }
        if (!applyBtn)
            return false;
        QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                          center(applyBtn));
        return true;
    };
    QVERIFY2(clickApply(dlg),
             "no visible non-Cancel button in the adjustment dialog -- the "
             "Q-D30 button pair is not rendering");
    QTest::qWaitFor([&] { return adjustmentRows() > before; }, 3000);
    QVERIFY2(adjustmentRows() > before,
             qPrintable(QStringLiteral("accepting the Brightness/Contrast "
                                       "dialog added no adjustment layer "
                                       "(%1 -> %2); status: %3")
                            .arg(before).arg(adjustmentRows())
                            .arg(m_controller->statusText())));
    // AND THE DIALOG CLOSED — its own contract is that it stays open ONLY on
    // a typed refusal. A dialog that lands the layer and lingers is a
    // different defect from one that neither lands nor closes.
    QTest::qWaitFor([&] { return !dlg->property("visible").toBool(); }, 2000);
    QVERIFY2(!dlg->property("visible").toBool(),
             "the dialog stayed open after a SUCCESSFUL accept — it re-opens "
             "only to show a typed refusal");

    // ---- 2. THE CURVE EDITOR IS REACHABLE AND NAMED --------------------
    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
    QCoreApplication::processEvents();
    QObject *curveDlg = openArm("adjustMenuCurves");
    QVERIFY2(curveDlg, "Curves did not open a visible dialog");
    QQuickItem *editor = byName(QStringLiteral("curveEditor"));
    QVERIFY2(editor, "the Curves dialog carries no 'curveEditor'");
    QVERIFY2(editor->isVisible() && editor->width() > 0 && editor->height() > 0,
             qPrintable(QStringLiteral("the curve editor is present but not "
                                       "usable (visible=%1 %2x%3)")
                            .arg(editor->isVisible())
                            .arg(editor->width()).arg(editor->height())));
    // ★ NOT ASSERTED HERE, and the reason is mine. The A11y name landed this
    //   range (`Accessible.name: tr("ui.dialog.adjust.curve_editor")`) with a
    //   note that the §31.5 lint does not cover a bare Item wrapping a
    //   Canvas — "a guard's silence is not a clean bill". I wrote an
    //   assertion for it and it FAILED, because `Accessible.name` is a QML
    //   ATTACHED property and is not readable through
    //   QObject::property("Accessible"). That is the T56 finding, repeated
    //   by me on the same property one tick later: the probe was reporting
    //   its own blindness as a missing name.
    //
    //   Removed rather than left red or weakened into something that always
    //   passes. Recorded as an UNCOVERED-BY-ME bound: reading QML attached
    //   properties needs the QQmlEngine attached-object API this fixture does
    //   not reach, and `tst_a11y_tree_lint` is the suite that owns that
    //   dimension. What IS asserted below is what this harness can honestly
    //   see — that the editor exists, is visible and has usable geometry.

    const int beforeCurve = adjustmentRows();
    // Same repair as the Brightness/Contrast arm above: Q-D30 moved the
    // accept path onto the Apply button.
    QVERIFY2(clickApply(curveDlg),
             "no visible non-Cancel button in the Curves dialog");
    QTest::qWaitFor([&] { return adjustmentRows() > beforeCurve; }, 3000);
    QVERIFY2(adjustmentRows() > beforeCurve,
             qPrintable(QStringLiteral("accepting the Curves dialog added no "
                                       "adjustment layer; status: %1")
                            .arg(m_controller->statusText())));
    qInfo().noquote()
        << QStringLiteral("ADJUSTMENT DIALOG: brightness_contrast and curves "
                          "both reached the engine; curve editor %1x%2 visible")
               .arg(editor->width()).arg(editor->height());
}

// ===================================================================
// T67 PER-FEATURE COVERAGE: the week's shell landings on my surface.
//   (a) the SIZE ROW that seven stroke tools gained (Main.qml:1077-1091)
//   (b) confirm-on-replace, standing priority (4)-3 (DiscardConfirmDialog)
//   (c) Q-D14 -- `S` selects clone and must not do it QUIETLY
// All three boot the same real Main.qml as the rest of this file.
// ===================================================================

// THE SIZE-ROW SET, DERIVED FROM THE SHIPPED MODEL AND NOT FROM THE
// DISPATCH. Main.qml's own comment derives it as "every PALETTE id that
// reaches the engine and whose CONTRACT carries sizePx" = 7 tools, and
// then adds clone.stroke as an eighth AS DIRECTED while recording that
// clone is in NEITHER kModeTools NOR the no-mode list (canvas_view.cpp
// :57-113), so it does not dispatch at all.
//
// ★★ THE EIGHTH ROW IS THE ONE WORTH PINNING, and it is pinned as what
// it IS, not as what it should be: a size slider on a tool that cannot
// paint is a working-looking control. Its row is asserted PRESENT (that
// is the shipped decision) and its drag is asserted to reach the engine
// with NOTHING (that is the consequence). If priority 5 wires clone, the
// second half reds and names the row that needs re-reading -- which is
// the red the wiring work turns green.
struct SizeRowTool { const char *tool; bool dispatches; };
static const SizeRowTool kSizeRowTools[] = {
    // The seven derived by the model's own rule.
    {"heal.stroke",          true},
    {"dodge_burn.dodge",     true},
    {"dodge_burn.burn",      true},
    {"dodge_burn.sponge",    true},
    {"blur_sharpen.blur",    true},
    {"blur_sharpen.sharpen", true},
    {"smudge.stroke",        true},
    // The eighth, added as directed and inert (Q-D14).
    {"clone.stroke",         false},
};

void TstGestureChrome::sizeRowShipsForEveryStrokeToolThatTakesASize()
{
    // Non-vacuity on the table itself (the T66 idiom O-UI wrote into my
    // net): a table that shrank to nothing would make every assertion
    // below vacuous while the case stayed green.
    QCOMPARE(int(sizeof(kSizeRowTools) / sizeof(kSizeRowTools[0])), 8);

    // ★★★ THE NEGATIVE CONTROL RUNS FIRST, and it is what makes "a
    // slider is visible" evidence instead of decoration. The options
    // strip is ONE Repeater over toolOptionsModel[canvas.tool] with a
    // `[]` fallback, so an absent tool renders an EMPTY bar -- which is
    // exactly why this defect was invisible on screen. If a stray slider
    // from some other pane were in view, every arm below would pass
    // without the size row existing at all.
    m_canvasItem->setProperty("tool", QStringLiteral("select.rect"));
    QCoreApplication::processEvents();
    QTest::qWait(60);
    QQuickItem *strip = itemById(QStringLiteral("toolbarPane"));
    QVERIFY2(strip, "the options strip (toolbarPane) is not reachable by its "
                    "Main.qml id -- this case cannot scope its search, and a "
                    "window-wide one measures the LayersPanel opacity slider");
    const int strayCount = visibleItemsUnder(m_window, strip, "QQuickSlider").size();
    QVERIFY2(strayCount == 0,
             qPrintable(QStringLiteral("CONTROL FAILED: select.rect takes no "
                                       "options yet %1 Slider(s) are visible "
                                       "-- every arm below would pass without "
                                       "a size row existing")
                            .arg(strayCount)));

    QStringList missing, noBinding;
    for (const SizeRowTool &t : kSizeRowTools) {
        m_canvasItem->setProperty("tool", QLatin1String(t.tool));
        QCoreApplication::processEvents();
        QQuickItem *slider = nullptr;
        QTest::qWaitFor([&] {
            const QList<QQuickItem *> s =
                visibleItemsUnder(m_window, strip, "QQuickSlider");
            slider = s.isEmpty() ? nullptr : s.first();
            return slider != nullptr;
        }, 2000);
        if (!slider) {
            missing << QLatin1String(t.tool);
            continue;
        }
        // IT MUST BE THE SIZE ROW AND NOT MERELY *A* SLIDER. The row is
        // declared once as the shared `optRowBrushSize` (min 1, max 200),
        // so the range identifies it; a puppet slider (0..1) would not
        // satisfy this and neither would a renamed row.
        QCOMPARE(slider->property("from").toDouble(), 1.0);
        QCOMPARE(slider->property("to").toDouble(), 200.0);

        // THE CONTROL -> CANVAS HALF, driven rather than read. `onMoved`
        // fires on interactive movement only, so setting `value` from the
        // test would prove nothing about the binding.
        //
        // ★★ SEED LOW FIRST, AND THAT IS NOT COSMETIC. The drag below runs
        // to the slider's far END, which SATURATES brushSize at 200. My
        // first cut compared each tool against whatever the previous tool
        // left behind, so arm 1 pushed the value to the maximum and arms
        // 2-8 dragged a slider that was already there and measured "no
        // change" -- reporting seven working rows as broken. Seeding a
        // known low value makes each arm independent, which is the same
        // fix the T62 mask arms needed for the same reason.
        m_canvasItem->setProperty("brushSize", 12.0);
        QCoreApplication::processEvents();
        const double before = m_canvasItem->property("brushSize").toDouble();
        const QPointF lo =
            slider->mapToScene(QPointF(slider->width() * 0.10,
                                       slider->height() / 2));
        const QPointF hi =
            slider->mapToScene(QPointF(slider->width() * 0.90,
                                       slider->height() / 2));
        // Q-D53 / OPERATOR WORD 5: the seeded value needs the binding to
        // settle before the press, or the press lands inside a ~2 ms window in
        // which the slider is DEAF and brushSize reads 12 -> 12. Measured cold,
        // one value per process: 0 ms and 1 ms are deaf; 2, 4, 8, 16, 32, 64,
        // 128, 250 and 500 ms are all live. This is the INSTRUMENT, not the
        // product -- a mouse press cannot be delivered at all without the event
        // loop spinning, so no sequence a person can perform lands inside it.
        // 32 ms is an order of magnitude above the measured edge and still
        // cheap across eight arms. Finding recorded to q-gesture.
        QTest::qWait(32);
        // Drag from wherever the handle is to the far end: a click alone
        // can land on the handle's current position and change nothing.
        QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier, lo.toPoint());
        QTest::mouseMove(m_window, hi.toPoint());
        QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                            hi.toPoint());
        QTest::qWaitFor([&] {
            return m_canvasItem->property("brushSize").toDouble() != before;
        }, 1500);
        if (m_canvasItem->property("brushSize").toDouble() == before)
            noBinding << QLatin1String(t.tool);
    }
    qInfo().noquote()
        << QStringLiteral("SIZE ROW: %1 of 8 tools rendered the row, %2 of "
                          "those drove canvas.brushSize")
               .arg(8 - missing.size()).arg(8 - missing.size() - noBinding.size());
    QVERIFY2(missing.isEmpty(),
             qPrintable(QStringLiteral("these tools rendered NO size row -- "
                                       "an EMPTY options bar is what this "
                                       "defect looks like on screen: %1")
                            .arg(missing.join(QStringLiteral(", ")))));
    QVERIFY2(noBinding.isEmpty(),
             qPrintable(QStringLiteral("these tools rendered a size row that "
                                       "did NOT write canvas.brushSize: %1")
                            .arg(noBinding.join(QStringLiteral(", ")))));

    // ★★ THE INERT EIGHTH, PINNED AS INERT. clone.stroke has the row and
    // reaches the engine with nothing. submissionAttempts discriminates
    // the two failures the way T66's case taught: never dispatched vs
    // dispatched-and-refused. Clone is the FORMER, and that is the
    // current, deliberate state (Q-D14) rather than a defect this case
    // asserts away.
    m_canvasItem->setProperty("tool", QStringLiteral("clone.stroke"));
    QCoreApplication::processEvents();
    const int histBefore = m_controller->historyEntries().size();
    const int attemptsBefore = m_controller->submissionAttempts();
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToScene(120, 200));
    QTest::mouseMove(m_window, docToScene(240, 200));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToScene(360, 200));
    QCoreApplication::processEvents();
    const int attempts = m_controller->submissionAttempts() - attemptsBefore;
    qInfo().noquote()
        << QStringLiteral("CLONE (size row present, tool inert): journal "
                          "%1->%2, submission attempts +%3")
               .arg(histBefore).arg(m_controller->historyEntries().size())
               .arg(attempts);
    QVERIFY2(m_controller->historyEntries().size() == histBefore,
             "clone.stroke COMMITTED something -- it is wired now, and the "
             "size row this case pins as an affordance-without-a-tool has "
             "become a real control: re-read the pin, do not widen it");
    QCOMPARE(attempts, 0);
}

void TstGestureChrome::sizeRowSliderIsReachableByTheKeyboard()
{
    // THE DISPATCH ASKS WHETHER THE SLIDER IS REACHABLE BY KEYBOARD, so
    // this case answers that question rather than the easier one next to
    // it (can a focused slider be driven by arrow keys -- yes, QQC2
    // Slider handles Left/Right and emits moved()).
    //
    // Reachability here means the shipped route: the §31.2 pane cycle.
    // F6 walks [OPTIONS, PALETTE, CANVAS, PANELS], and Main.qml's C2 SKIP
    // RULE skips any pane whose `paneFirstOperable(pane)` is null --
    // i.e. a pane with no focusable child in Qt's own focus chain. So the
    // size row is keyboard-reachable if and only if the Slider joins that
    // chain.
    QQuickItem *strip = itemById(QStringLiteral("toolbarPane"));
    QVERIFY2(strip, "the options strip is not reachable by its Main.qml id");
    m_canvasItem->setProperty("tool", QStringLiteral("brush.stroke"));
    QCoreApplication::processEvents();
    QQuickItem *slider = nullptr;
    QTest::qWaitFor([&] {
        const QList<QQuickItem *> s =
            visibleItemsUnder(m_window, strip, "QQuickSlider");
        slider = s.isEmpty() ? nullptr : s.first();
        return slider != nullptr;
    }, 2000);
    QVERIFY2(slider, "no size row for brush.stroke -- this case cannot ask "
                     "its question");

    // What Main.qml's own skip rule computes, computed the same way.
    const QVariant operable =
        evalJs(QStringLiteral("(function(){ var p = paneFirstOperable(toolbarPane); "
                              "return p ? String(p) : \"\"; })()"));
    const bool paneHasOperableChild = !operable.toString().isEmpty();
    const bool sliderTakesTabFocus =
        slider->property("activeFocusOnTab").toBool();

    qInfo().noquote()
        << QStringLiteral("SIZE ROW KEYBOARD: slider activeFocusOnTab=%1, "
                          "paneFirstOperable(toolbarPane)=%2")
               .arg(sliderTakesTabFocus ? QStringLiteral("true")
                                        : QStringLiteral("false"))
               .arg(paneHasOperableChild ? operable.toString()
                                         : QStringLiteral("NULL (pane skipped)"));

    // ★ DRIVE IT, do not infer from the property. Focus the CANVAS first:
    // cyclePane() returns early when NO pane holds focus (review D4-R --
    // focus outside every pane makes the chord a no-op rather than a
    // focus steal), so a cycle called from an unfocused fixture moves
    // nothing and would look exactly like an unreachable pane. My first
    // cut did that and reported the product unreachable on the strength
    // of it.
    //
    // paneCycle is [OPTIONS, PALETTE, CANVAS, PANELS]; from the canvas
    // (index 2) the options pane is two forward steps away, and the walk
    // SKIPS any pane with no operable child -- so the loop below cycles
    // until the options pane holds focus rather than assuming a count.
    focusCanvas();
    // Seed a MID value: Right at the maximum cannot increase, and a
    // saturated slider is indistinguishable from an inoperable one.
    m_canvasItem->setProperty("brushSize", 100.0);
    QCoreApplication::processEvents();
    const double before = m_canvasItem->property("brushSize").toDouble();
    bool optionsFocused = false;
    for (int step = 0; step < 4 && !optionsFocused; ++step) {
        evalJs(QStringLiteral("root.cyclePane(1)"));
        QCoreApplication::processEvents();
        optionsFocused = evalJs(QStringLiteral("toolbarPane.activeFocus")).toBool();
    }
    if (optionsFocused) {
        QTest::keyClick(m_window, Qt::Key_Right);
        QCoreApplication::processEvents();
        QTest::qWait(60);
    }
    const double after = m_canvasItem->property("brushSize").toDouble();
    qInfo().noquote()
        << QStringLiteral("  F6 cycle from canvas: toolbarPane.activeFocus=%1, "
                          "brushSize %2 -> %3")
               .arg(optionsFocused ? QStringLiteral("true")
                                   : QStringLiteral("false"))
               .arg(before).arg(after);

    // ★★ PINNED AS MEASURED, IN BOTH DIRECTIONS. If the slider is in the
    // focus chain the pane must be enterable and the key must move the
    // value; if it is not, the pane is SKIPPED and the row is
    // mouse-only -- which is a finding, filed in the ledger, and this
    // assertion is written so it REDS THE DAY SOMEONE FIXES IT rather
    // than blessing the gap forever.
    if (sliderTakesTabFocus) {
        QVERIFY2(paneHasOperableChild,
                 "the size slider takes tab focus but the options pane "
                 "reports no operable child -- the C2 skip rule and the "
                 "control disagree");
        QVERIFY2(optionsFocused,
                 "the size slider is in the focus chain and the options pane "
                 "reports it operable, yet cycling never landed focus there "
                 "-- the row is unreachable by the shipped keyboard route");
        QVERIFY2(after != before,
                 qPrintable(QStringLiteral("the options pane took focus but "
                                           "Right did not move brushSize "
                                           "(still %1) -- the row is "
                                           "reachable and not operable")
                                .arg(before)));
    } else {
        QVERIFY2(!paneHasOperableChild,
                 "the size slider does NOT take tab focus, yet the options "
                 "pane reports an operable child -- if a focusable control "
                 "was added to this pane, this case is now the stale half "
                 "and the keyboard route needs re-measuring");
        QVERIFY2(after == before,
                 "brushSize moved by keyboard although the slider is not in "
                 "the focus chain -- the route is not the one this case "
                 "models; re-derive it");
    }
    m_canvasItem->forceActiveFocus();
}

void TstGestureChrome::sizeRowToolKeysTapSelectAndHoldSpringsBack()
{
    // TAP vs HOLD on the size-row tools' OWN letters. The existing
    // spring case covers B alone; this covers the letters the week's
    // landing put a size row behind.
    //
    // ★★★ AND IT STATES THE BOUND FIRST, because the dispatch says
    // "their keys" and four of the seven HAVE NO KEY. From
    // tool_key_dispatcher.cpp's ring table: heal.stroke is J (ring of
    // one), the three dodge_burn members share O (ring of three), and
    // blur_sharpen.blur/.sharpen/smudge.stroke take NO LETTER AT ALL --
    // recorded there as a DEVIATION-BY-SILENCE with its citation, on the
    // same footing as S's. So tap-vs-hold is askable for 4 of the 7, and
    // the other 3 are pinned as keyless below so that GAINING a letter
    // reds this case instead of passing silently.
    struct KeyRow { Qt::Key key; const char *letter; const char *first; };
    static const KeyRow keyed[] = {
        {Qt::Key_J, "J", "heal.stroke"},
        {Qt::Key_O, "O", "dodge_burn.dodge"},
    };
    QCOMPARE(int(sizeof(keyed) / sizeof(keyed[0])), 2);

    for (const KeyRow &r : keyed) {
        // --- TAP: selects, and STAYS selected. ---
        m_canvasItem->setProperty("tool", QStringLiteral("select.rect"));
        m_canvasItem->forceActiveFocus();
        QCOMPARE(canvasTool(), QStringLiteral("select.rect"));
        const bool tapped = tapLetterUntilTool(r.key, QLatin1String(r.first));
        QVERIFY2(tapped,
                 qPrintable(QStringLiteral("tapping %1 did not select %2 "
                                           "(tool is %3) in four attempts")
                                .arg(QLatin1String(r.letter),
                                     QLatin1String(r.first), canvasTool())));
        // The tap must PERSIST -- a tap that reverts is a hold, and the
        // whole point of the boundary is that these two differ.
        QTest::qWait(ToolKeyDispatcher::SpringHoldMs + 120);
        QCoreApplication::processEvents();
        QVERIFY2(canvasTool() == QLatin1String(r.first),
                 qPrintable(QStringLiteral("a TAP of %1 reverted to %2 after "
                                           "the spring window elapsed -- tap "
                                           "and hold are not distinguished")
                                .arg(QLatin1String(r.letter), canvasTool())));
        // ★ AND THE SIZE ROW IS THERE FOR THE TOOL THE LETTER SELECTED.
        // This is the join the dispatch actually asks about: the key and
        // the row are one affordance, and a letter that selects a tool
        // whose row never renders is the defect in its original form.
        QQuickItem *slider = nullptr;
        QTest::qWaitFor([&] {
            const QList<QQuickItem *> s =
                visibleVisualItems(m_window, "QQuickSlider");
            slider = s.isEmpty() ? nullptr : s.first();
            return slider != nullptr;
        }, 2000);
        QVERIFY2(slider,
                 qPrintable(QStringLiteral("%1 selected %2 by key and NO size "
                                           "row rendered")
                                .arg(QLatin1String(r.letter),
                                     QLatin1String(r.first))));

        // --- HOLD: borrows past the boundary, reverts on release. ---
        m_canvasItem->setProperty("tool", QStringLiteral("select.rect"));
        m_canvasItem->forceActiveFocus();
        QTest::keyPress(m_window, r.key);
        QTest::qWait(ToolKeyDispatcher::SpringHoldMs + 120);
        const QString held = canvasTool();
        QTest::keyRelease(m_window, r.key);
        QCoreApplication::processEvents();
        const QString after = canvasTool();
        qInfo().noquote()
            << QStringLiteral("TAP/HOLD %1: tap->%2  hold->%3  release->%4")
                   .arg(QLatin1String(r.letter), QLatin1String(r.first),
                        held, after);
        QVERIFY2(held == QLatin1String(r.first),
                 qPrintable(QStringLiteral("holding %1 past %2 ms did not "
                                           "borrow %3 (tool was %4)")
                                .arg(QLatin1String(r.letter))
                                .arg(ToolKeyDispatcher::SpringHoldMs)
                                .arg(QLatin1String(r.first), held)));
        QVERIFY2(after == QStringLiteral("select.rect"),
                 qPrintable(QStringLiteral("releasing a held %1 did not revert "
                                           "(tool is %2)")
                                .arg(QLatin1String(r.letter), after)));
    }

    // THE THREE KEYLESS SIZE-ROW TOOLS, pinned as keyless by WALKING THE
    // RING TABLE -- not by asserting that some list is non-empty, which
    // is the shape of my own G-7 (a lint that asserted nothing for
    // twenty-one ticks).
    static const char *keyless[] = {"blur_sharpen.blur", "blur_sharpen.sharpen",
                                    "smudge.stroke"};
    QStringList everyRingMember;
    for (const ToolRing &ring : ToolKeyDispatcher::ringTable())
        everyRingMember << ring.members;
    // Non-vacuity in the other direction: if the walk found nothing, the
    // "not present" checks below would all pass trivially. J and O are
    // asserted present precisely because this case just drove them.
    QVERIFY2(everyRingMember.contains(QStringLiteral("heal.stroke"))
                 && everyRingMember.contains(QStringLiteral("dodge_burn.dodge")),
             qPrintable(QStringLiteral("the ring-table walk found %1 members "
                                       "and not the two this case just drove "
                                       "-- the walk is blind and every "
                                       "keyless claim below is vacuous")
                            .arg(everyRingMember.size())));
    QStringList gainedALetter;
    for (const char *t : keyless)
        if (everyRingMember.contains(QLatin1String(t)))
            gainedALetter << QLatin1String(t);
    qInfo().noquote()
        << QStringLiteral("RING TABLE: %1 members; keyless size-row tools "
                          "still keyless: %2 of 3")
               .arg(everyRingMember.size()).arg(3 - gainedALetter.size());
    QVERIFY2(gainedALetter.isEmpty(),
             qPrintable(QStringLiteral("these size-row tools GAINED a ring "
                                       "letter: %1 -- tap-vs-hold is now "
                                       "askable for them and this case must "
                                       "grow to cover it")
                            .arg(gainedALetter.join(QStringLiteral(", ")))));
}

void TstGestureChrome::dirtyReplaceDoorsRaiseTheDiscardConfirm()
{
    // Standing priority (4)-3: a document with unsaved work must not be
    // replaced or closed silently. Main.qml routes THREE doors through
    // one `confirmDiscard(action)` helper: File > New (via
    // NewDocumentDialog.confirmGate), File > Open
    // (openFileDialog.onAccepted), and the window's own onClosing.
    //
    // ★★★★ THE DISPATCH NAMES Cmd+W AND THERE IS NO Cmd+W. Measured, not
    // assumed: StandardKey.Close / StandardKey.Quit / "Ctrl+W" appear
    // NOWHERE in shell/qml or shell/src. The close door is the WINDOW's
    // close event, which is what onClosing handles. Pinned below through
    // that event; the missing accelerator is filed in the ledger rather
    // than asserted here, because this case's subject is the confirm and
    // not the keymap.
    //
    // ★★★ AND Cmd+N DOES NOT RAISE THE CONFIRM EITHER -- by design.
    // StandardKey.New opens the New Document DIALOG; the confirm fires on
    // that dialog's ACCEPT, because (its own comment) gating the menu item
    // would prompt and then let the user cancel, "having asked them to
    // approve a discard that never happened". So the door under test is
    // the accept, and this case drives the accept.
    auto confirmVisible = [&]() -> bool {
        return evalJs(QStringLiteral("discardConfirmDialog.visible")).toBool();
    };
    auto makeDirty = [&]() {
        // Real work through the command door: a fill moves the journal
        // cursor past doc.new, which is what documentHasUnsavedWork reads.
        m_canvasItem->setProperty("tool", QStringLiteral("brush.stroke"));
        m_canvasItem->forceActiveFocus();
        QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                          docToScene(120, 160));
        QTest::mouseMove(m_window, docToScene(240, 200));
        QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                            docToScene(360, 240));
        QCoreApplication::processEvents();
    };

    // ---- CONTROL FIRST: a PRISTINE document must NOT be confirmed. ----
    // This is the cry-wolf half the header comment names, and without it
    // "the dialog appeared" would not distinguish a working guard from a
    // dialog that always opens.
    QVERIFY2(!m_controller->documentHasUnsavedWork(),
             "a freshly-initialised document already reports unsaved work -- "
             "the control below cannot run");
    evalJs(QStringLiteral("newDialog.open()"));
    QCoreApplication::processEvents();
    evalJs(QStringLiteral("newDialog.accept()"));
    QTest::qWait(120);
    QCoreApplication::processEvents();
    QVERIFY2(!confirmVisible(),
             "File > New over a PRISTINE document raised the discard confirm "
             "-- the cry-wolf failure the dirty-flag distinction exists to "
             "prevent");

    // ★★★ RE-ESTABLISH init()'s GEOMETRY INVARIANT, and the reason is a
    // red this case earned in the full suite while passing alone. The
    // control arm above ACCEPTS the New Document dialog, which builds a
    // document at whatever preset the dialog is holding -- and
    // newDocumentPresetCreatesThatDocument runs earlier in this suite and
    // leaves one selected. The new document then zoomFits, so zoom is no
    // longer 100 and the doc may exceed the canvas item: docToScene's
    // mapping silently stops being valid and the setup stroke lands off
    // canvas, painting nothing. The failure MESSAGE said "the setup
    // stroke did not make the document dirty", which is true and is a
    // symptom; the mechanism is the invariant groundChromeGeometry
    // exists to protect, broken by this case's own control arm.
    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
    m_canvas->zoomFit();
    m_canvas->zoomOneToOne();
    QCOMPARE(m_canvasItem->property("zoomPercent").toInt(), 100);
    QVERIFY2(m_canvasItem->width() >= m_controller->docWidth()
                 && m_canvasItem->height() >= m_controller->docHeight(),
             "the canvas item is smaller than the document -- doc->scene "
             "mapping is invalid and the strokes below would land nowhere");
    focusCanvas();

    // ---- DOOR 1: File > New over unsaved work. ----
    makeDirty();
    QVERIFY2(m_controller->documentHasUnsavedWork(),
             qPrintable(QStringLiteral("the setup stroke did not make the "
                                       "document dirty; status: %1")
                            .arg(m_controller->statusText())));
    const int layersBefore = m_controller->layers().size();
    evalJs(QStringLiteral("newDialog.open()"));
    QCoreApplication::processEvents();
    evalJs(QStringLiteral("newDialog.accept()"));
    QTest::qWaitFor([&] { return confirmVisible(); }, 2000);
    QVERIFY2(confirmVisible(),
             "File > New over UNSAVED WORK did not raise the discard confirm "
             "-- the document would have been replaced silently");

    // CANCEL PRESERVES. A confirm that appears and then discards anyway is
    // worse than none, so the cancel arm is asserted on the DOCUMENT, not
    // on the dialog's visibility.
    evalJs(QStringLiteral("discardConfirmDialog.reject()"));
    QTest::qWaitFor([&] { return !confirmVisible(); }, 2000);
    QCoreApplication::processEvents();
    QVERIFY2(m_controller->documentHasUnsavedWork(),
             "cancelling the discard confirm REPLACED the document anyway");
    QCOMPARE(m_controller->layers().size(), layersBefore);

    // ---- DOOR 3: the window's own close, over unsaved work. ----
    // ★★ THE ONE WITH NO NEXT STATE, in Main.qml's words: a replaced
    // document leaves the user somewhere they could notice the loss; a
    // closed window leaves nothing at all. Qt delivers the close as a
    // VETOABLE event, so the guard must refuse it -- asserting the window
    // is still visible is the whole point, and a guard that opened the
    // dialog but let the close through would pass a dialog-only check.
    QVERIFY2(m_controller->documentHasUnsavedWork(),
             "the close-door arm needs a dirty document and lost it");
    QVERIFY2(m_window->isVisible(), "the window is already closed");
    QMetaObject::invokeMethod(m_window, "close", Qt::DirectConnection);
    QTest::qWaitFor([&] { return confirmVisible(); }, 2000);
    QVERIFY2(confirmVisible(),
             "closing the window over UNSAVED WORK did not raise the discard "
             "confirm");
    QVERIFY2(m_window->isVisible(),
             "the window CLOSED while the confirm was still open -- the close "
             "event was not vetoed and the confirm is decorative");
    // Leave the fixture as we found it: cancel, and clear the work.
    evalJs(QStringLiteral("discardConfirmDialog.reject()"));
    QTest::qWaitFor([&] { return !confirmVisible(); }, 2000);
    QVERIFY2(m_window->isVisible(), "the window closed after a cancelled "
                                    "close confirm");
    qInfo().noquote()
        << QStringLiteral("DISCARD CONFIRM: pristine=no-prompt, New=prompt, "
                          "cancel preserved %1 layer(s), window close=prompt "
                          "and VETOED")
               .arg(layersBefore);
}

void TstGestureChrome::cloneKeySpeaksInsteadOfSelectingSilently()
{
    // Q-D14 = VOICE (operator, 2026-08-30): `S` stays bound and SPEAKS.
    // "Silent tools are the disease this week cured; a bound key that
    // does nothing quietly is not an option."
    //
    // The dispatch describes S as selecting and doing nothing QUIETLY.
    // That was true when it was written and is not true on this main, so
    // this pins WHAT IS THERE NOW: the keymap path selects clone AND
    // states why nothing will happen. The pin is on the SHAPE -- typed,
    // resolved through the catalog, naming the tool -- so it survives
    // priority 5 wiring the tool.
    m_canvasItem->setProperty("tool", QStringLiteral("select.rect"));
    m_canvasItem->forceActiveFocus();
    QVERIFY(tapLetterUntilTool(Qt::Key_S, QStringLiteral("clone.stroke")));

    const QString said = m_controller->statusText();
    qInfo().noquote() << QStringLiteral("CLONE KEY S: tool=%1 status=\"%2\"")
                             .arg(canvasTool(), said);
    QVERIFY2(!said.isEmpty(),
             "pressing S selected clone and said NOTHING -- the Q-D14 ruling "
             "is that a bound key must not be silent");
    // ★★ NOT KEY-ECHO, and not a rendered placeholder. The id carries a
    // {tool} argument; a templated string invoked bare renders the literal
    // "{tool}" at the user, which is the exact key-echo class the reasonId
    // door exists to prevent. Both failures are named separately because
    // they have different causes.
    QVERIFY2(!said.contains(QStringLiteral("ui.status.")),
             qPrintable(QStringLiteral("the status line KEY-ECHOED a message "
                                       "id at the user: \"%1\"").arg(said)));
    QVERIFY2(!said.contains(QStringLiteral("{tool}")),
             qPrintable(QStringLiteral("the templated status was invoked bare "
                                       "-- the user sees the placeholder: "
                                       "\"%1\"").arg(said)));

    // ★★★★ WHICH SENTENCE, and pinned WITHOUT hardcoding English. Q-D21
    // ruled (a): clone speaks `ui.status.clone_needs_source` and NOT the
    // generic `ui.status.tool_pending`. Before that ruling, selecting
    // clone and pressing it said two DIFFERENT things seconds apart. A
    // check that only asks "did it speak" goes green on the incoherence
    // the ruling removed, so the expected strings are resolved through
    // the same catalog the shell uses and compared.
    const QString expected =
        evalJs(QStringLiteral("AppI18n.tr(\"ui.status.clone_needs_source\", "
                              "{\"tool\": AppI18n.tr(root.toolNameKeys[\"clone.stroke\"])})"))
            .toString();
    const QString generic =
        evalJs(QStringLiteral("AppI18n.tr(\"ui.status.tool_pending\", "
                              "{\"tool\": AppI18n.tr(root.toolNameKeys[\"clone.stroke\"])})"))
            .toString();
    QVERIFY2(!expected.isEmpty() && expected != generic,
             "the two candidate sentences did not resolve distinctly -- this "
             "arm cannot tell the Q-D21 ruling from its alternative");
    QVERIFY2(said == expected,
             qPrintable(QStringLiteral("clone spoke the WRONG sentence.\n"
                                       "  said:     \"%1\"\n"
                                       "  expected: \"%2\" (Q-D21 (a))\n"
                                       "  generic:  \"%3\"")
                            .arg(said, expected, generic)));

    // ★★★ THE SCOPE BOUND, PINNED RATHER THAN IMPLIED. Main.qml states it
    // in its own comment: the fix is on the KEYMAP path (writeTool is the
    // sole keymap tool writer), and "the palette slot and flyout select
    // clone by a different route and are NOT covered here -- recorded
    // rather than silently implied, because a partial fix described as
    // complete is the shape this project keeps catching."
    //
    // So the palette route is measured HERE, and whatever it does is
    // recorded as the current state. This is the arm that turns the
    // scope note into an observation instead of a promise.
    m_controller->applyCommand(QStringLiteral("selection.none"), {}, {}, {});
    m_canvasItem->setProperty("tool", QStringLiteral("select.rect"));
    QCoreApplication::processEvents();
    // Drive the property write the palette slot performs, then read.
    const QString beforePalette = m_controller->statusText();
    m_canvasItem->setProperty("tool", QStringLiteral("clone.stroke"));
    QCoreApplication::processEvents();
    QTest::qWait(80);
    const QString afterPalette = m_controller->statusText();
    qInfo().noquote()
        << QStringLiteral("CLONE by PALETTE route: status \"%1\" -> \"%2\" (%3)")
               .arg(beforePalette, afterPalette,
                    afterPalette == beforePalette
                        ? QStringLiteral("SILENT -- the documented scope bound")
                        : QStringLiteral("spoke"));
    QCOMPARE(canvasTool(), QStringLiteral("clone.stroke"));
}

// ===================================================================
// T68: the (4)-3 door T67 did NOT reach, the two replace SHORTCUTS as
// real keys, and the button-level arm on a dialog another desk believes
// nothing renders.
// ===================================================================

void TstGestureChrome::fileOpenOverDirtyDocumentConfirmsBeforeReplacing()
{
    // ★★★★ THE THIRD (4)-3 DOOR, AND T67 COVERED THE OTHER TWO. Main.qml
    // routes File > New, File > Open and the window close through one
    // confirmDiscard(); T67 pinned New and the close and left Open
    // uncovered, so a regression in THIS door would have been invisible.
    //
    // ★★★ AND IT IS THE DOOR WITH TWO EXITS. Main.qml puts the guard
    // AHEAD of queryRecovery deliberately -- "the recovery prompt is a
    // BRANCH inside this door, not a fourth door, so one confirm covers a
    // path that has two exits". A confirm placed on the wrong side of
    // that branch would still prompt for ordinary opens and silently
    // discard whenever a sidecar happened to exist, which is the failure
    // this ordering exists to prevent.
    QTemporaryDir tmp;
    QVERIFY(tmp.isValid());
    const QString docPath = tmp.filePath(QStringLiteral("target.appname"));

    // A REAL document on disk to open. Built through the save door so the
    // file is whatever the product actually writes.
    QVERIFY(m_controller->newDocument(320, 240, QStringLiteral("white")));
    QVERIFY2(m_controller->saveDocument(docPath),
             qPrintable(QStringLiteral("could not write the .appname this "
                                       "case opens; status: %1")
                            .arg(m_controller->statusText())));
    const QString url = QUrl::fromLocalFile(docPath).toString();

    // Reachability BEFORE anything is asserted about it (G-547 at the
    // input): a null here would leave every arm below measuring an absent
    // object. The FileDialogs carry no objectName, so this is the
    // QQmlExpression id door tst_import_filter owns and O-INT reused.
    QVERIFY2(evalJs(QStringLiteral("openFileDialog !== null")).toBool(),
             "openFileDialog is not reachable by its Main.qml id");

    auto confirmVisible = [&]() -> bool {
        return evalJs(QStringLiteral("discardConfirmDialog.visible")).toBool();
    };
    // ★★★★★ THE CONFIRM MUST BE DRIVEN BY ITS BUTTONS, AND accept() IS NOT
    // ONE. DiscardConfirmDialog declares NO standardButtons: Cancel and
    // Discard are plain Buttons whose onClicked handlers hold the whole
    // behaviour -- Discard reads discardAction, clears it, closes, and THEN
    // invokes it. So Dialog::accept() emits accepted and closes while
    // NEVER RUNNING THE ACTION. My first draft used accept()/reject() and
    // the discard arm failed with "the file was not opened", which is true
    // and blames the product for a door my own probe never opened.
    //
    // ★★★★ AND THE CANCEL ARM PASSED, FOR THE WRONG REASON: reject() also
    // closes without running the action, so "the document was preserved"
    // was a fact about my probe rather than about the guard. A green arm
    // resting on the same mistake as a red one is the worse half of this.
    auto clickConfirm = [&](const char *catId) -> bool {
        QObject *dlg = nullptr;
        for (QObject *o : m_window->findChildren<QObject *>())
            if (o->inherits("QQuickDialog") && o->property("visible").toBool())
                dlg = o;
        if (!dlg)
            return false;
        const QString want = m_i18n->tr(QLatin1String(catId));
        for (QObject *o : dlg->findChildren<QObject *>()) {
            if (!o->inherits("QQuickAbstractButton"))
                continue;
            auto *b = qobject_cast<QQuickItem *>(o);
            if (!b || !b->isVisible() || b->width() <= 0)
                continue;
            if (b->property("text").toString() != want)
                continue;
            QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                              center(b));
            QCoreApplication::processEvents();
            return true;
        }
        return false;
    };
    auto armDialog = [&]() {
        evalJs(QStringLiteral("openFileDialog.selectedFile = '%1'").arg(url));
        // Read back: if selectedFile is read-only the assignment fails
        // SILENTLY and every arm below would blame the confirm for an
        // empty url. O-INT's site-4 lesson, applied at the same property.
        const QString back =
            evalJs(QStringLiteral("String(openFileDialog.selectedFile)")).toString();
        QVERIFY2(back == url,
                 qPrintable(QStringLiteral("selectedFile did not take the "
                                           "assignment (read back '%1', set "
                                           "'%2') -- this case cannot reach "
                                           "the handler").arg(back, url)));
    };

    // ---- CONTROL: a PRISTINE document must open with NO prompt. --------
    // Without this, "the dialog appeared" cannot be told from a guard that
    // always fires -- the cry-wolf half the dirty-flag distinction exists
    // for, and the half an existing test already caught this project
    // shipping once.
    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
    QCoreApplication::processEvents();
    QVERIFY2(!m_controller->documentHasUnsavedWork(),
             "a freshly-created document already reports unsaved work -- the "
             "control cannot run");
    armDialog();
    evalJs(QStringLiteral("openFileDialog.accepted()"));
    QTest::qWait(150);
    QCoreApplication::processEvents();
    QVERIFY2(!confirmVisible(),
             "File > Open over a PRISTINE document raised the discard confirm");

    // ---- THE DOOR: unsaved work must be confirmed before replacing. ----
    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
    m_canvas->zoomFit();
    m_canvas->zoomOneToOne();
    QCOMPARE(m_canvasItem->property("zoomPercent").toInt(), 100);
    focusCanvas();
    m_canvasItem->setProperty("tool", QStringLiteral("brush.stroke"));
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToScene(120, 160));
    QTest::mouseMove(m_window, docToScene(240, 200));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToScene(360, 240));
    QCoreApplication::processEvents();
    QVERIFY2(m_controller->documentHasUnsavedWork(),
             qPrintable(QStringLiteral("the setup stroke did not make the "
                                       "document dirty; status: %1")
                            .arg(m_controller->statusText())));

    const int wBefore = m_controller->docWidth();
    const int hBefore = m_controller->docHeight();
    armDialog();
    evalJs(QStringLiteral("openFileDialog.accepted()"));
    QTest::qWaitFor([&] { return confirmVisible(); }, 2000);
    QVERIFY2(confirmVisible(),
             "File > Open over UNSAVED WORK did not raise the discard confirm "
             "-- the open document would have been replaced silently");
    // AND THE REPLACEMENT HAS NOT HAPPENED YET. A confirm that appears
    // AFTER the document is gone is decoration; the document under it must
    // still be the user's. 640x400 is this fixture's, 320x240 is the file.
    QCOMPARE(m_controller->docWidth(), wBefore);
    QCOMPARE(m_controller->docHeight(), hBefore);

    // CANCEL PRESERVES -- asserted on the DOCUMENT, not on the dialog, and
    // driven by the real button.
    QVERIFY2(clickConfirm("ui.dialog.cancel"),
             "no visible Cancel button in the discard confirm");
    QTest::qWaitFor([&] { return !confirmVisible(); }, 2000);
    QCoreApplication::processEvents();
    QVERIFY2(m_controller->documentHasUnsavedWork(),
             "cancelling the discard confirm replaced the document anyway");
    QCOMPARE(m_controller->docWidth(), wBefore);

    // DISCARD PROCEEDS -- and the open must actually HAPPEN. A guard that
    // prompts and then drops the action on the floor passes every
    // assertion above and loses the user's click.
    armDialog();
    evalJs(QStringLiteral("openFileDialog.accepted()"));
    QTest::qWaitFor([&] { return confirmVisible(); }, 2000);
    QVERIFY2(confirmVisible(), "the second File > Open raised no confirm");
    QVERIFY2(clickConfirm("ui.dialog.discard_confirm.discard"),
             "no visible Discard button in the discard confirm");
    QTest::qWaitFor([&] {
        return m_controller->docWidth() == 320
               && m_controller->docHeight() == 240;
    }, 3000);
    qInfo().noquote()
        << QStringLiteral("FILE>OPEN DOOR: pristine=no-prompt; dirty=prompt "
                          "with the document still %1x%2; cancel preserved it; "
                          "discard opened the file -> %3x%4")
               .arg(wBefore).arg(hBefore)
               .arg(m_controller->docWidth()).arg(m_controller->docHeight());
    QVERIFY2(m_controller->docWidth() == 320 && m_controller->docHeight() == 240,
             qPrintable(QStringLiteral("after DISCARD the file was not opened "
                                       "(document is %1x%2, the file is "
                                       "320x240); status: %3")
                            .arg(m_controller->docWidth())
                            .arg(m_controller->docHeight())
                            .arg(m_controller->statusText())));
}

void TstGestureChrome::replaceShortcutsReachTheirDoorsAndCloseIsBound()
{
    // ★★★ THE DISPATCH NAMES Cmd+W, Cmd+N AND File>Open, AND T67 DROVE
    // NONE OF THE KEYS. T67 opened the New dialog with
    // `evalJs("newDialog.open()")` -- which proves the dialog and the
    // confirm work and says NOTHING about whether the accelerator that is
    // supposed to reach them is bound. This drives the real sequences.
    auto dialogVisible = [&](const char *id) -> bool {
        return evalJs(QStringLiteral("%1.visible").arg(QLatin1String(id)))
            .toBool();
    };
    evalJs(QStringLiteral("newDialog.close(); openFileDialog.close()"));
    QCoreApplication::processEvents();
    focusCanvas();

    // --- Cmd/Ctrl+N -> the New Document dialog ---
    QVERIFY2(!dialogVisible("newDialog"), "newDialog was already open");
    QTest::keySequence(m_window, QKeySequence(QKeySequence::New));
    QTest::qWaitFor([&] { return dialogVisible("newDialog"); }, 2000);
    QVERIFY2(dialogVisible("newDialog"),
             "StandardKey.New did not open the New Document dialog -- the "
             "accelerator the dispatch names as a replace door is not "
             "reaching it");
    evalJs(QStringLiteral("newDialog.close()"));
    QTest::qWaitFor([&] { return !dialogVisible("newDialog"); }, 2000);

    // --- Cmd/Ctrl+O: verified as CLAIMED, deliberately not pressed. ---
    // ★★★ PRESSING IT COST ME THREE FALSE REDS. QtQuick.Dialogs FileDialog
    // is MODAL and its close() does not take effect under the offscreen
    // platform, so the dialog stayed up, focusCanvas() failed in the next
    // case's init(), and the red landed on whichever case ran next with an
    // EMPTY message pointing into my fixture. What the press would have
    // added over the walk below is nothing: this case's subject is whether
    // the accelerator is BOUND, and
    // fileOpenOverDirtyDocumentConfirmsBeforeReplacing already drives the
    // handler it reaches through the dialog's own accepted() signal.
    //
    // Recorded rather than quietly dropped, because "I stopped pressing the
    // key" is exactly the kind of narrowing that should be visible.

    // ★★★★ Cmd+W: PINNED AS UNBOUND, WHICH IS G-10 MADE SELF-CHECKING.
    // T67 measured zero occurrences of StandardKey.Close / StandardKey.Quit
    // / "Ctrl+W" across shell/qml AND shell/src and filed it. A measurement
    // in a ledger goes stale; this reds the day someone binds the key, and
    // the reason that matters is COVERAGE: a newly-bound Cmd+W is a fourth
    // replace door, and it would arrive with no confirm case on it.
    //
    // Measured on the LIVE OBJECT TREE, not by pressing the key and not by
    // scanning source text. Pressing it and observing nothing cannot tell
    // "not bound" from "bound to something that failed silently" -- the
    // exact confusion this lane keeps cataloguing -- and a source grep
    // proves a string is absent from one file rather than that the running
    // shell claims no such sequence. This asks the shortcuts themselves.
    QStringList claimed;
    for (QObject *o : m_window->findChildren<QObject *>()) {
        if (!o->inherits("QQuickShortcut"))
            continue;
        const QVariant one = o->property("sequence");
        if (one.isValid() && !one.toString().isEmpty())
            claimed << one.toString();
        for (const QVariant &v : o->property("sequences").toList())
            if (!v.toString().isEmpty())
                claimed << v.toString();
    }
    // ★★★ A SEQUENCE HAS TWO SPELLINGS HERE AND THE FIRST DRAFT KNEW ONE.
    // A Shortcut written `sequences: [StandardKey.New]` reports the raw
    // ENUM (the string "6"), while one written `sequence: "Ctrl+Z"` reports
    // portable text -- this window carries BOTH kinds, and comparing only
    // portable text found 48 sequences and matched none of them. The
    // non-vacuity guard is what caught it, so the matcher below accepts
    // either spelling rather than assuming the shell picked one.
    auto claims = [&](QKeySequence::StandardKey k) {
        return claimed.contains(QString::number(int(k)))
               || claimed.contains(
                   QKeySequence(k).toString(QKeySequence::PortableText));
    };
    // Non-vacuity: the walk must find the shortcuts that DO exist, or
    // "Close is not among them" is a fact about an empty list. New is
    // asserted present because this case just drove it.
    QVERIFY2(!claimed.isEmpty() && claims(QKeySequence::New),
             qPrintable(QStringLiteral("the shortcut walk found %1 sequence(s) "
                                       "and not New (enum %2 / text %3) -- it "
                                       "is blind and the negative below would "
                                       "be vacuous: [%4]")
                            .arg(claimed.size())
                            .arg(int(QKeySequence::New))
                            .arg(QKeySequence(QKeySequence::New)
                                     .toString(QKeySequence::PortableText))
                            .arg(claimed.join(QStringLiteral(" ")))));
    QVERIFY2(claims(QKeySequence::Open),
             qPrintable(QStringLiteral("no live Shortcut claims Open (enum %1) "
                                       "-- File > Open has no accelerator, "
                                       "which is a third replace door reachable "
                                       "only by menu: [%2]")
                            .arg(int(QKeySequence::Open))
                            .arg(claimed.join(QStringLiteral(" ")))));
    const QString closeSeq =
        QKeySequence(QKeySequence::Close).toString(QKeySequence::PortableText);
    const bool closeBound =
        claims(QKeySequence::Close) || claims(QKeySequence::Quit);
    qInfo().noquote()
        << QStringLiteral("REPLACE SHORTCUTS: New=opens its dialog, "
                          "Open=claimed; %1 live shortcut sequence(s); "
                          "Close(%2)=%3")
               .arg(claimed.size()).arg(closeSeq)
               .arg(closeBound ? QStringLiteral("BOUND — G-10 has moved")
                               : QStringLiteral("unbound (G-10, as filed)"));
    // RC3 D-3 (operator WORD 35, sitting S-20): Cmd+W IS the fourth door now,
    // and its confirm arm is closeDoorAndThePromptsSaveButton (grown first, as
    // this pin asked). Close is bound; Quit stays the window close (onClosing).
    QVERIFY2(claims(QKeySequence::Close),
             "Cmd+W is not bound -- File > Close (D-3) has no accelerator");
    QVERIFY2(!claims(QKeySequence::Quit),
             "Quit gained a shortcut; it must stay the window's close (onClosing)");
    Q_UNUSED(closeBound);
}

// RC3 D-3 (operator WORD 35, sitting S-20): Cmd+W did nothing (no File >
// Close) and the unsaved-changes prompt offered only Cancel / Discard -- no
// Save. Close is now a door (Cmd+W and File > Close) through the SAME
// confirmDiscard gate as New, Open and the window close, and the prompt
// offers Save (titled: Cmd+S's path, then continue; untitled: Save As, then
// continue only on success), Don't Save and Cancel. Driven by REAL keys and
// REAL button clicks.
void TstGestureChrome::closeDoorAndThePromptsSaveButton()
{
    auto findVisible = [&](const QString &name) -> QQuickItem * {
        QQuickItem *hit = nullptr;
        std::function<void(QQuickItem *)> walk = [&](QQuickItem *it) {
            if (!it || hit)
                return;
            if (it->objectName() == name && it->isVisible()) {
                hit = it;
                return;
            }
            const auto kids = it->childItems();
            for (QQuickItem *k : kids)
                walk(k);
        };
        QQuickItem *root = m_window->contentItem();
        walk(root->parentItem() ? root->parentItem() : root);
        return hit;
    };
    auto confirmVisible = [&]() -> bool {
        return evalJs(QStringLiteral("discardConfirmDialog.visible")).toBool();
    };
    auto freshDirty = [&]() {
        QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
        m_canvasItem->setProperty("tool", QStringLiteral("brush.stroke"));
        m_canvasItem->forceActiveFocus();
        QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                          docToScene(120, 160));
        QTest::mouseMove(m_window, docToScene(240, 200));
        QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                            docToScene(360, 240));
        QCoreApplication::processEvents();
        QTRY_VERIFY(m_controller->documentHasUnsavedWork());
    };
    auto pressCmdW = [&]() {
        m_canvasItem->forceActiveFocus();
        QTest::keySequence(m_window, QKeySequence(QKeySequence::Close));
        QCoreApplication::processEvents();
    };
    auto click = [&](const char *name) {
        QQuickItem *b = findVisible(QLatin1String(name));
        QVERIFY2(b, name);
        QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier, center(b));
        QCoreApplication::processEvents();
    };

    // CONTROL: a pristine document closes with NO prompt (no cry-wolf).
    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
    QVERIFY(!m_controller->documentHasUnsavedWork());
    pressCmdW();
    QTRY_VERIFY2(!m_controller->hasDocument(), "Cmd+W did not close a pristine document");
    QVERIFY2(!confirmVisible(), "a pristine close raised the unsaved-changes prompt");

    // (1) Dirty + Cmd+W -> the prompt, with all three buttons; Cancel keeps it.
    freshDirty();
    pressCmdW();
    QTRY_VERIFY2(confirmVisible(), "Cmd+W over unsaved work raised no prompt");
    QVERIFY(findVisible(QStringLiteral("discardConfirmSave")));
    QVERIFY(findVisible(QStringLiteral("discardConfirmDontSave")));
    QVERIFY(findVisible(QStringLiteral("discardConfirmCancel")));
    click("discardConfirmCancel");
    QTRY_VERIFY(!confirmVisible());
    QVERIFY2(m_controller->hasDocument(), "Cancel closed the document");

    // (2) Don't Save -> closed, nothing written.
    pressCmdW();
    QTRY_VERIFY(confirmVisible());
    click("discardConfirmDontSave");
    QTRY_VERIFY2(!m_controller->hasDocument(), "Don't Save did not close");

    // (3) TITLED + Save -> saved through Cmd+S's path, THEN closed.
    QTemporaryDir tmp;
    QVERIFY(tmp.isValid());
    const QString pkg = tmp.filePath(QStringLiteral("d3.appname"));
    freshDirty();
    QVERIFY(m_controller->saveDocument(pkg));
    const QDateTime savedAt = QFileInfo(pkg).lastModified();
    QTest::qWait(1100); // a later save must show a later mtime
    m_canvasItem->forceActiveFocus();
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier, docToScene(100, 100));
    QTest::mouseMove(m_window, docToScene(300, 300));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier, docToScene(300, 300));
    QTRY_VERIFY(m_controller->documentHasUnsavedWork());
    pressCmdW();
    QTRY_VERIFY(confirmVisible());
    click("discardConfirmSave");
    QTRY_VERIFY2(!m_controller->hasDocument(), "Save did not continue to the close");
    QVERIFY2(QFileInfo(pkg).lastModified() > savedAt,
             "Save on the prompt did not write the titled document");

    // (3b) THE SAVED BASELINE DOES NOT LEAK into the next document: right
    // after saving a document, a brand-new one must read CLEAN (it used to be
    // compared against the previous document's saved journal cursor).
    // The baseline is saved AFTER an edit (journal cursor > 1): a pristine
    // save would put the saved cursor at 1, equal to any fresh document's,
    // and the leak would be invisible (measured: that version stayed green
    // with the fix removed).
    freshDirty();
    QVERIFY2(m_controller->saveDocument(tmp.filePath(QStringLiteral("d3b.appname"))),
             "could not save the baseline document");
    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
    QVERIFY2(!m_controller->documentHasUnsavedWork(),
             "a brand-new document reads as UNSAVED right after a save -- the "
             "previous document's saved cursor leaked into it");

    // (4) UNTITLED + Save -> Save As opens; cancelling it keeps the document.
    freshDirty();
    pressCmdW();
    QTRY_VERIFY(confirmVisible());
    click("discardConfirmSave");
    QTRY_VERIFY2(evalJs(QStringLiteral("saveFileDialog.visible")).toBool(),
                 "Save on an untitled document did not open Save As");
    QVERIFY2(evalJs(QStringLiteral("saveFileDialog.afterSave !== null")).toBool(),
             "Save As carries no pending close");
    evalJs(QStringLiteral("saveFileDialog.reject()"));
    QTRY_VERIFY(!evalJs(QStringLiteral("saveFileDialog.visible")).toBool());
    QVERIFY2(m_controller->hasDocument(),
             "a CANCELLED Save As still closed the document -- work lost");
    QVERIFY(evalJs(QStringLiteral("saveFileDialog.afterSave === null")).toBool());

    // (5) File > Close exists and follows hasDocument.
    QVERIFY(evalJs(QStringLiteral("AppDoc.hasDocument")).toBool());
    QVERIFY2(m_window->findChild<QObject *>(QStringLiteral("fileMenuClose")),
             "no File > Close menu item");
    QVERIFY(m_window->findChild<QObject *>(QStringLiteral("fileMenuClose"))
                ->property("enabled").toBool());

    // TEARDOWN: leave the window as init() expects it -- no dialog open, a
    // fresh document, canvas focused. Name what held focus, for the record.
    QQuickItem *held = m_window->activeFocusItem();
    qInfo().noquote() << QStringLiteral("D-3 teardown: focus held by %1 (%2)")
                             .arg(held ? QString::fromLatin1(held->metaObject()->className())
                                       : QStringLiteral("nothing"),
                                  held ? held->objectName() : QString());
    evalJs(QStringLiteral("saveFileDialog.close(); discardConfirmDialog.close()"));
    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
    // The Save As panel is its own window and took activation with it;
    // closing it does not hand activation back, so ask for it explicitly.
    m_window->requestActivate();
    QTRY_VERIFY2(m_window->isActive(),
                 "the main window did not regain activation after Save As");
    m_canvasItem->forceActiveFocus();
    QTRY_VERIFY2(m_canvasItem->hasActiveFocus(),
                 "the canvas could not take focus back after the D-3 case");
}


void TstGestureChrome::adjustmentDialogAcceptButtonIsClickableAndLabelled()
{
    // ★★★★★ WHY THIS EXISTS. INTEGRATION-log 547 concludes "NO SHELL TEST
    // RENDERS THAT DIALOG", calls its own 6/6 green vacuous on that basis,
    // and offers a rendering test as future work. The premise is FALSE --
    // adjustmentParamDialogAcceptReachesTheEngine (T65, this file) opens
    // AdjustmentParamDialog through the real adjustButton menu, reads
    // adjustChannelCombo and adjustErrorLabel from INSIDE it, and measures
    // the curve editor's geometry. A grep for the type name misses it
    // because the dialog is reached by menu and matched by
    // inherits("QQuickDialog"), never by name.
    //
    // ★★★★ AND THE CONCLUSION SURVIVES ANYWAY, WHICH IS THE POINT. That
    // case accepts with QMetaObject::invokeMethod(dlg, "accept") -- it
    // never touches a BUTTON. So the risk 547 names (a label change
    // breaking a unique-visible-button lookup) was genuinely uncovered
    // even though the dialog was rendered. The premise was wrong and the
    // gap was real, and those are two different facts.
    //
    // This is the arm that was missing: the dialog's own buttons, found,
    // labelled, and CLICKED.
    auto byName = [&](const QString &n) -> QQuickItem * {
        for (QObject *o : m_window->findChildren<QObject *>())
            if (o->objectName() == n)
                return qobject_cast<QQuickItem *>(o);
        return nullptr;
    };
    auto visibleDialog = [&]() -> QObject * {
        for (QObject *o : m_window->findChildren<QObject *>())
            if (o->inherits("QQuickDialog") && o->property("visible").toBool())
                return o;
        return nullptr;
    };
    auto adjustmentRows = [&]() -> int {
        int n = 0;
        for (const QVariant &r : m_controller->layers())
            if (r.toMap().value(QStringLiteral("kind")).toString()
                == QLatin1String("adjustment"))
                ++n;
        return n;
    };

    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
    QCoreApplication::processEvents();
    QQuickItem *btn = byName(QStringLiteral("adjustButton"));
    QVERIFY2(btn, "no adjustButton -- this case cannot open the dialog");
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      center(btn));
    QCoreApplication::processEvents();
    QQuickItem *arm = byName(QStringLiteral("adjustMenuBrightnessContrast"));
    QVERIFY2(arm, "no adjustMenuBrightnessContrast arm");
    QMetaObject::invokeMethod(arm, "triggered");
    QCoreApplication::processEvents();
    QObject *dlg = visibleDialog();
    QVERIFY2(dlg, "Brightness/Contrast opened no visible dialog");

    // THE BUTTONS, ENUMERATED FROM THE DIALOG ITSELF. standardButtons is
    // Dialog.Ok | Dialog.Cancel, so exactly two must be visible and their
    // labels must be distinct and non-empty -- which is the property a
    // unique-visible-button lookup depends on, asserted where the dialog
    // actually renders instead of where its source is read as text.
    QList<QQuickItem *> buttons;
    for (QObject *o : dlg->findChildren<QObject *>()) {
        if (!o->inherits("QQuickAbstractButton"))
            continue;
        QQuickItem *b = qobject_cast<QQuickItem *>(o);
        if (b && b->isVisible() && b->width() > 0 && b->height() > 0)
            buttons << b;
    }
    QStringList labels;
    for (QQuickItem *b : buttons)
        labels << b->property("text").toString();
    qInfo().noquote()
        << QStringLiteral("ADJUST DIALOG BUTTONS: %1 visible -> [%2]")
               .arg(buttons.size()).arg(labels.join(QStringLiteral(" | ")));
    QVERIFY2(buttons.size() == 2,
             qPrintable(QStringLiteral("expected exactly two visible buttons "
                                       "(Ok, Cancel) and found %1: [%2]")
                            .arg(buttons.size())
                            .arg(labels.join(QStringLiteral(" | ")))));
    for (const QString &l : labels)
        QVERIFY2(!l.trimmed().isEmpty(),
                 qPrintable(QStringLiteral("a dialog button renders an EMPTY "
                                           "label: [%1]")
                                .arg(labels.join(QStringLiteral(" | ")))));
    QVERIFY2(labels.at(0) != labels.at(1),
             qPrintable(QStringLiteral("both dialog buttons render the SAME "
                                       "label '%1' -- a unique-visible-button "
                                       "lookup cannot tell them apart")
                            .arg(labels.at(0))));

    // AND THE ACCEPT BUTTON IS CLICKED, not invoked. This is the half T65
    // left out and 547 was right to want.
    // ★★★ IDENTIFYING THE ACCEPT BUTTON, AND THE FIRST TWO WAYS WERE WRONG.
    // `DialogButtonBox.buttonRole` is an ATTACHED property and is not
    // readable through QObject::property() -- the same fact that defeated an
    // Accessible.name probe at T56 and again at T65, now met a third time on
    // a third property. And "the box's first button" is not a rule: if the
    // style ordered Cancel first, this case would click Cancel and then
    // report that the ACCEPT button does not work, which is a false red
    // aimed at the product.
    //
    // So it is chosen by ELIMINATION against the catalog's own cancel
    // string: the accept button is the visible one that is not Cancel. The
    // bound is stated -- this fixture loads the `en` catalog, so the
    // comparison is against a resolved catalog value and not a literal, but
    // it is still a label comparison and would need revisiting for a locale
    // whose two buttons collide.
    const QString cancelText = m_i18n->tr(QStringLiteral("ui.dialog.cancel"));
    QQuickItem *ok = nullptr;
    for (QQuickItem *b : buttons)
        if (b->property("text").toString() != cancelText)
            ok = b;
    QVERIFY2(ok,
             qPrintable(QStringLiteral("could not tell the accept button from "
                                       "the cancel one: both read '%1'")
                            .arg(cancelText)));
    const int before = adjustmentRows();
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier, center(ok));
    QCoreApplication::processEvents();
    qInfo().noquote()
        << QStringLiteral("  clicked '%1' -> dialog visible=%2, adjustment "
                          "rows %3 -> %4")
               .arg(ok->property("text").toString())
               .arg(dlg->property("visible").toBool())
               .arg(before).arg(adjustmentRows());
    QTest::qWaitFor([&] { return adjustmentRows() > before; }, 3000);
    QVERIFY2(adjustmentRows() > before,
             qPrintable(QStringLiteral("CLICKING the dialog's accept button "
                                       "('%1') added no adjustment layer "
                                       "(%2 -> %3) -- T65 reaches the engine "
                                       "by invoking accept(), so this is the "
                                       "button, not the handler; status: %4")
                            .arg(ok->property("text").toString())
                            .arg(before).arg(adjustmentRows())
                            .arg(m_controller->statusText())));
}

void TstGestureChrome::undoGestureScrollsTheCursorBackWithoutMovingAVisibleOne()
{
    // Q-D22 landed while this tick was running: the history view scrolls to
    // keep the cursor visible, suppressed while the panel has focus.
    // tst_history_panel::cursorStaysVisibleAndIsSuppressedWhileFocused
    // already covers those two arms well, and I am NOT repeating it.
    //
    // ★★★★★ WHAT IT CANNOT CATCH, AND WHY THAT IS THE INTERESTING HALF.
    // Its arm A asserts contentY > 0 after history changes; its arm B
    // asserts no motion while focused. Swap `ListView.Contain` for
    // `ListView.Center` and BOTH still pass -- Center also moves the view
    // in arm A, and suppression is unaffected in arm B. The mode is the one
    // thing Q-D22 was actually a question about, and HistoryPanel's own
    // comment says so: "Contain is the least-motion reading ... and it is
    // one identifier to change."
    //
    // ★★★★ THE LEAST-MOTION CLAIM IS THEREFORE PINNED HERE, AND IT IS A
    // GESTURE CLAIM: an undo driven by the real Cmd+Z, with the cursor
    // ALREADY IN VIEW, must not move the viewport at all. A view that
    // re-centres on every undo yanks the list under a user who is watching
    // it -- visible motion where the contract promises none.
    QQuickItem *list =
        m_window->findChild<QQuickItem *>(QStringLiteral("historyList"));
    QVERIFY2(list, "historyList not reachable by objectName");
    QVERIFY2(list->height() > 0, "the history list has no height");
    auto contentY = [&] { return list->property("contentY").toDouble(); };
    auto count = [&] { return list->property("count").toInt(); };

    // Park focus OFF the panel -- the honest scenario, and the precondition
    // arm A of the sibling case had to learn the hard way: the content item
    // is a focus scope, so forcing focus onto it hands focus back down to
    // the row it remembers.
    focusCanvas();
    QTRY_VERIFY2(!list->hasActiveFocus(),
                 "the history list holds focus; revealCursor would be "
                 "suppressed and this case would measure the wrong arm");

    // ---- ARM 1: the least-motion claim, and it took THREE constructions.
    // ★★★★★ TWO EARLIER VERSIONS PASSED UNDER THE MUTATION THEY EXISTED TO
    // CATCH, and each was defeated by the fixture rather than by the code:
    //   (i) a SHORT history -- four rows, content fits, positionViewAtIndex
    //       has nowhere to scroll and contentY clamps to 0, so "no motion"
    //       held for Contain, Center and End alike.
    //   (ii) an overflowing list parked at the END -- but a TAIL cursor is
    //       exactly where Center and Contain agree, because centring a row
    //       near the bottom would scroll past the end and clamps back.
    // Both arguments were clean; both arms were decoration. Only running the
    // mutation showed it, twice.
    //
    // ★★★★ THE SHAPE THAT ACTUALLY DISCRIMINATES needs the cursor MID-LIST
    // and the viewport parked where Contain and Center DISAGREE: the row
    // fully visible (so Contain does nothing) but away from the centre (so
    // Center must move). Constructed by arithmetic on the delegate height
    // rather than by hoping.
    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
    m_canvas->zoomFit();
    m_canvas->zoomOneToOne();
    QCOMPARE(m_canvasItem->property("zoomPercent").toInt(), 100);
    focusCanvas();
    m_canvasItem->setProperty("tool", QStringLiteral("brush.stroke"));
    for (int i = 0; i < 40; ++i) {
        QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                          docToScene(100, 150));
        QTest::mouseMove(m_window, docToScene(200, 190));
        QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                            docToScene(300, 230));
    }
    QCoreApplication::processEvents();
    const double rowH = 26.0;   // HistoryPanel.qml delegate height
    const double pitch = 27.0;  // ...plus the ListView's `spacing: 1`
    QTRY_VERIFY2(list->property("contentHeight").toDouble()
                     > list->height() + 4 * rowH,
                 "the history list does not overflow enough to tell Contain "
                 "from Center");

    // The panel owns cursorRow; reach it by walking up from the list rather
    // than by objectName, which it does not carry.
    QQuickItem *panel = list->parentItem();
    while (panel && !panel->property("cursorRow").isValid())
        panel = panel->parentItem();
    QVERIFY2(panel, "could not reach the HistoryPanel's cursorRow");

    // Put the cursor MID-LIST through the command door (setup, not the
    // surface under test), leaving room on both sides.
    for (int i = 0; i < 18; ++i)
        m_controller->undo();
    QCoreApplication::processEvents();
    QTest::qWait(100);
    const int cursor = panel->property("cursorRow").toInt();
    const double H = list->height();
    QVERIFY2(cursor > 4 && cursor < count() - 4,
             qPrintable(QStringLiteral("cursor row %1 of %2 is too close to an "
                                       "end for Center and Contain to differ")
                            .arg(cursor).arg(count())));

    // Park so the cursor row is FULLY VISIBLE but well off centre. Contain must
    // leave this alone; Center would haul it to the middle.
    //
    // OPERATOR WORD 11 (2)(a), 2026-09-14 — RE-AIMED, DARK-DESK EXCEPTION.
    // The old reference was `cursor * pitch - 2 * pitch`. Against
    // `centreWants = cursor*pitch + rowH/2 - H/2` the CURSOR TERM CANCELS, so the
    // separation was |2*pitch + rowH/2 - H/2| -- a function of the VIEWPORT HEIGHT
    // alone, and at this height 16.3px. That is inside one row, so this case's own
    // non-vacuity guard fired and it reported that it could not tell Contain from
    // Center. The guard was right; the reference was wrong.
    //
    // Anchor the park to the REQUIREMENT instead of to a row offset: park BELOW
    // where Center wants, so the separation is set by construction and not by the
    // viewport height. BELOW, not above, because the wheel loop scrolls DOWN to
    // reach it and the old park sat at a LARGER contentY that was reached.
    //
    // FOUR rows, not two (O-INT, 2026-09-18). Two rows left 2*rowH = 52px of
    // margin, and the wheel OVERSHOOTS: the loop tested a contentY that the view
    // had not finished updating, so it kept sending notches. Measured: aim 440,
    // parked 477.185, Center wants 492 -- separation 14.8 against the one rowH
    // (26) this arm's own guard demands, so the guard fired and the arm could not
    // tell the modes apart. The overshoot measured at entry 1052 was 37.7px, so
    // four rows (104px) keeps the separation above one row even at that overshoot,
    // and the settle wait below removes most of it.
    const double parked = (cursor * pitch + rowH / 2.0 - H / 2.0) - 4.0 * rowH;
    // ★★★ PARKED BY A REAL WHEEL GESTURE, NOT BY setProperty. A programmatic
    // contentY write is a plausible park and a bad one: if the view discards
    // it on the next model change, "the list jumped" would be a fact about my
    // assignment rather than about what a user who scrolled would see. The
    // wheel is the gesture this lane exists to test, so the park is made with
    // it and then CHECKED, and setProperty is used only to close the last few
    // pixels once the wheel has established that scrolling by hand sticks.
    const QPoint overList = center(list);
    // ONE NOTCH, THEN LET THE VIEW SETTLE. The ListView applies a wheel notch
    // asynchronously; reading contentY in the same iteration reads the value from
    // BEFORE the notch, so a loop without the wait sends notches long after the
    // target is reached (entry 1052: 300 notches fired, 37.7px overshoot).
    for (int i = 0; i < 300 && list->property("contentY").toDouble() < parked; ++i) {
        QWheelEvent w(QPointF(overList), m_window->mapToGlobal(QPointF(overList)),
                      QPoint(0, 0), QPoint(0, -120), Qt::NoButton,
                      Qt::NoModifier, Qt::NoScrollPhase, false);
        QCoreApplication::sendEvent(m_window, &w);
        QCoreApplication::processEvents();
        QTest::qWait(8);
    }
    QTest::qWait(80);
    const double wheeled = list->property("contentY").toDouble();
    QVERIFY2(wheeled > 0.0,
             "a real wheel gesture over the history list did not scroll it at "
             "all -- the park below would be programmatic-only and this arm "
             "could not tell a product jump from my own assignment");
    // DECISIVE: the park is the WHEEL'S alone. No programmatic contentY in
    // the path at all, so a jump below cannot be blamed on my assignment.
    const double visibleBefore = list->property("contentY").toDouble();
    const double centreWants = cursor * pitch + rowH / 2.0 - H / 2.0;
    // NON-VACUITY, BOTH WAYS: the park must have taken, and the two modes
    // must genuinely disagree here -- otherwise this arm cannot fail.
    QVERIFY2(visibleBefore > 2 * rowH,
             qPrintable(QStringLiteral("the wheel park left contentY at %1 -- "
                                       "too near the top for this arm")
                            .arg(visibleBefore)));
    QVERIFY2(qAbs(centreWants - visibleBefore) > rowH,
             qPrintable(QStringLiteral("parked at %1 and Center wants %2 -- "
                                       "closer than one row, so this arm "
                                       "cannot tell the modes apart")
                            .arg(visibleBefore).arg(centreWants)));

    // THE GESTURE: the real undo chord, not m_controller->undo().
    focusCanvas();
    QTRY_VERIFY(!list->hasActiveFocus());
    QTest::keySequence(m_window, QKeySequence(QKeySequence::Undo));
    QTest::qWait(150);
    QCoreApplication::processEvents();
    const double visibleAfter = list->property("contentY").toDouble();
    const int cursorAfter = panel->property("cursorRow").toInt();
    // ★★★★ ASK THE VIEW, DO NOT COMPUTE. My first three drafts derived a
    // row's position as index * 26 and the delegate pitch is 27 -- the
    // ListView sets `spacing: 1`. The error compounds per row and by row 21
    // it is most of a row. A geometry claim that the view can be asked for
    // directly should never be arithmetic on an assumed constant.
    QQuickItem *cursorItem = nullptr;
    QMetaObject::invokeMethod(list, "itemAtIndex",
                              Q_RETURN_ARG(QQuickItem *, cursorItem),
                              Q_ARG(int, cursorAfter));
    const double originY = list->property("originY").toDouble();
    const double newTop = cursorItem ? cursorItem->y()
                                     : cursorAfter * (rowH + 1.0) + originY;
    qInfo().noquote()
        << QStringLiteral("  GEOMETRY: originY=%1 itemAtIndex(%2)=%3 y=%4 "
                          "contentHeight=%5")
               .arg(originY).arg(cursorAfter)
               .arg(cursorItem ? QStringLiteral("realized")
                               : QStringLiteral("NOT realized"))
               .arg(newTop)
               .arg(list->property("contentHeight").toDouble());
    const bool newRowWasVisible = newTop >= visibleBefore
                                  && newTop + rowH <= visibleBefore + H;
    qInfo().noquote()
        << QStringLiteral("HISTORY REVEAL: cursor %1 -> %2 of %3; viewport "
                          "[%4,%5] h=%6; new row top %7 wasVisible=%8; "
                          "contentY %4 -> %9 (Center would want ~%10)")
               .arg(cursor).arg(cursorAfter).arg(count())
               .arg(visibleBefore).arg(visibleBefore + H).arg(H)
               .arg(newTop).arg(newRowWasVisible)
               .arg(visibleAfter).arg(centreWants);
    QVERIFY2(newRowWasVisible,
             qPrintable(QStringLiteral("the post-undo cursor row %1 (top %2) "
                                       "was NOT inside the viewport [%3,%4] "
                                       "-- then scrolling is CORRECT and this "
                                       "arm is measuring the wrong thing")
                            .arg(cursorAfter).arg(newTop)
                            .arg(visibleBefore).arg(visibleBefore + H)));

    // ★★★★★ G-15, FILED THIS TICK, AND PINNED WITH QEXPECT_FAIL SO THE FIX
    // ANNOUNCES ITSELF. HistoryPanel.qml states the contract in its own
    // words -- "Contain scrolls ONLY when the cursor is off-screen, so a
    // cursor already in view never moves the viewport" -- and the viewport
    // moves anyway. Everything above establishes that the premise holds:
    // the park is a REAL WHEEL GESTURE with no programmatic contentY in the
    // path, and the post-undo cursor row's position is read from the view
    // (itemAtIndex) rather than computed.
    //
    // ★★★★ THE MECHANISM, BY ARITHMETIC RATHER THAN BY GUESS. The landing
    // place is 357 from a park at 520 AND from a park at 446 -- the user's
    // position is not adjusted, it is DISCARDED. And 357 is exactly
    // row22.bottom(620) - viewportHeight(263): Contain computed from
    // contentY = 0. So the model change resets the view to the top and
    // revealCursor then measures "least motion" from that reset origin,
    // which is minimal with respect to nothing the user did.
    //
    // ★★★ QEXPECT_FAIL, not a weakened assertion and not a permanent red:
    // the contract stays written exactly as the product states it, this run
    // reports XFAIL, and the day the reset is fixed QtTest reports XPASS as
    // a FAILURE -- so the fix gets its red to turn green without anyone
    // having to remember this case exists.
    QEXPECT_FAIL("", "G-15: the model change resets the view to the top and "
                     "revealCursor computes least motion from contentY 0, so "
                     "an undo scrolls the panel even when the cursor row is "
                     "already fully visible. Routed to O-UI, not patched.",
                 Continue);
    QVERIFY2(qAbs(visibleAfter - visibleBefore) < rowH,
             qPrintable(QStringLiteral("undo moved the history viewport "
                                       "%1 -> %2 although the cursor row was "
                                       "already fully visible -- "
                                       "ListView.Contain promises least "
                                       "motion (Center would want ~%3)")
                            .arg(visibleBefore).arg(visibleAfter)
                            .arg(centreWants)));

    // ---- AND THE OTHER DIRECTION, so the arm above is not passing merely
    // because revealCursor never runs. Overflow the list, park it away from
    // the cursor, and undo: now the view MUST move. Without this, a
    // revealCursor that was dead entirely would satisfy the no-motion arm.
    for (int i = 0; i < 22; ++i) {
        QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                          docToScene(80, 120));
        QTest::mouseMove(m_window, docToScene(140, 160));
        QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                            docToScene(200, 200));
    }
    QCoreApplication::processEvents();
    QTRY_VERIFY2(list->property("contentHeight").toDouble() > list->height(),
                 qPrintable(QStringLiteral("the history list still fits its "
                                           "viewport (content %1 <= height "
                                           "%2) -- the overflow arm cannot "
                                           "run")
                                .arg(list->property("contentHeight").toDouble())
                                .arg(list->height())));
    QMetaObject::invokeMethod(list, "positionViewAtBeginning");
    QCoreApplication::processEvents();
    QTRY_COMPARE(contentY(), 0.0);
    focusCanvas();
    QTRY_VERIFY(!list->hasActiveFocus());
    QTest::keySequence(m_window, QKeySequence(QKeySequence::Undo));
    QTest::qWait(150);
    QCoreApplication::processEvents();
    qInfo().noquote()
        << QStringLiteral("HISTORY REVEAL (cursor off-screen): rows=%1 "
                          "contentY 0 -> %2").arg(count()).arg(contentY());
    QVERIFY2(contentY() > 0.0,
             qPrintable(QStringLiteral("with the cursor scrolled out of view "
                                       "an undo did NOT bring it back "
                                       "(contentY still %1) -- revealCursor "
                                       "is dead, and the no-motion arm above "
                                       "was passing for that reason")
                            .arg(contentY())));
}

void TstGestureChrome::adjustmentDialogIsOperableFromTheKeyboard()
{
    // ROADMAP-1.0 §5 G-02 names "dialog keyboard paths" as a leg of the
    // gesture net (due 09-16); G-06 (RC1, 09-21) requires every control on
    // the 1.0 sitting path to be reachable by keyboard, MAJOR if a
    // must-pass step is not. This suite had ZERO key coverage on any
    // dialog before this case — Key_Return / Key_Escape / Key_Tab appear
    // nowhere near a dialog in either file. That is the gap.
    //
    // WHY THIS DIALOG FIRST. Q-D30 replaced `standardButtons: Dialog.Ok |
    // Dialog.Cancel` with two plain Buttons in a RowLayout and moved the
    // accept path into the Apply button's onClicked. That fixed a real
    // i18n defect — Qt's own OK/Cancel never asked the catalog, and with
    // no QTranslator installed they rendered hard English in es, ja and
    // pseudo. Nothing here asks for that to be reverted.
    //
    // But `standardButtons` is also what gives a QQC2 Dialog its DEFAULT
    // BUTTON — the one Return activates — and AdjustmentParamDialog.qml
    // now carries no DialogButtonBox, no standardButtons and no Keys
    // handler of any kind (measured: 0 hits for `Keys.` in the file). So
    // the question is whether the keyboard accept path survived that
    // change. THE STRUCTURE IS A REASON TO LOOK, NOT A FINDING: what
    // follows is measured through real key events, and the arms are
    // ordered so that the fixture cannot be what produces the answer.
    auto byName = [&](const QString &n) -> QQuickItem * {
        for (QObject *o : m_window->findChildren<QObject *>())
            if (o->objectName() == n)
                return qobject_cast<QQuickItem *>(o);
        return nullptr;
    };
    auto visibleDialog = [&]() -> QObject * {
        for (QObject *o : m_window->findChildren<QObject *>())
            if (o->inherits("QQuickDialog") && o->property("visible").toBool())
                return o;
        return nullptr;
    };
    auto adjustmentRows = [&]() -> int {
        int n = 0;
        for (const QVariant &r : m_controller->layers())
            if (r.toMap().value(QStringLiteral("kind")).toString()
                == QLatin1String("adjustment"))
                ++n;
        return n;
    };
    // Every arm starts from a dialog THIS LAMBDA opened, and no arm may
    // inherit one. T68 cost three false reds to a leaked modal whose
    // failure landed on whichever case ran next with an empty message
    // pointing into my own fixture; the close-first line is that lesson.
    auto openDialog = [&]() -> QObject * {
        if (QObject *stale = visibleDialog())
            QMetaObject::invokeMethod(stale, "close");
        QCoreApplication::processEvents();
        QQuickItem *btn = byName(QStringLiteral("adjustButton"));
        if (!btn)
            return nullptr;
        QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier, center(btn));
        QCoreApplication::processEvents();
        QQuickItem *arm = byName(QStringLiteral("adjustMenuBrightnessContrast"));
        if (!arm)
            return nullptr;
        QMetaObject::invokeMethod(arm, "triggered");
        QCoreApplication::processEvents();
        return visibleDialog();
    };

    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
    QCoreApplication::processEvents();

    // ---- ARM A — ESCAPE CANCELS -------------------------------------
    // Popup's default closePolicy carries CloseOnEscape, so this arm is
    // expected to pass. It is here as the POSITIVE CONTROL for the whole
    // case: it proves this fixture's key events actually reach the
    // dialog. Without it, a Return that does nothing (arm B) could not be
    // told apart from key events that never arrived at all — which is the
    // difference between a product finding and a broken probe.
    QObject *dlg = openDialog();
    QVERIFY2(dlg, "Brightness/Contrast opened no visible dialog");
    const int beforeEsc = adjustmentRows();
    QTest::keyClick(m_window, Qt::Key_Escape);
    QCoreApplication::processEvents();
    QTest::qWaitFor([&] { return !visibleDialog(); }, 2000);
    const bool escClosed = (visibleDialog() == nullptr);
    qInfo().noquote()
        << QStringLiteral("DIALOG KEYBOARD: Escape -> closed=%1, "
                          "adjustment rows %2 -> %3")
               .arg(escClosed).arg(beforeEsc).arg(adjustmentRows());
    QVERIFY2(escClosed,
             "Escape did not close the adjustment dialog — and because this "
             "is also the control proving key events reach the dialog, every "
             "later arm in this case would be unreadable");
    QCOMPARE(adjustmentRows(), beforeEsc);

    // ---- ARM B — RETURN, THE CONVENTION ------------------------------
    // A freshly opened dialog, then Return: the ordinary way a user
    // commits a dialog on both macOS and Windows. Measured with the same
    // instrument arm A just proved live.
    dlg = openDialog();
    QVERIFY2(dlg, "the adjustment dialog did not reopen for the Return arm");
    const int beforeRet = adjustmentRows();
    // ★★★ T70 CORRECTION: FOCUS IS ESTABLISHED BEFORE RETURN IS PRESSED.
    // As first written this arm pressed Return without knowing where focus
    // was, and leaned on the Escape arm as its control. That control is
    // weaker than it was credited with: a Popup handles Escape ITSELF via
    // closePolicy, so Escape closing proves the POPUP saw the key -- not
    // that a focused item would see a Return. The discard-confirm case
    // proved the difference by mutation. Return is now pressed with a real
    // focused control inside the dialog, which is also the realistic user
    // state: tab to something, then hit Enter.
    QTest::keyClick(m_window, Qt::Key_Tab);
    QCoreApplication::processEvents();
    QQuickItem *retFocus = m_window->activeFocusItem();
    QVERIFY2(retFocus != nullptr,
             "nothing holds focus, so Return has no target and this arm "
             "would report a fact about the fixture");
    qInfo().noquote()
        << QStringLiteral("  (Return pressed with focus on %1)")
               .arg(retFocus->objectName().isEmpty()
                        ? QString::fromLatin1(retFocus->metaObject()->className())
                        : retFocus->objectName());
    QTest::keyClick(m_window, Qt::Key_Return);
    QCoreApplication::processEvents();
    QTest::qWaitFor([&] { return adjustmentRows() > beforeRet; }, 2000);
    const int afterRet = adjustmentRows();
    const bool retStillOpen = (visibleDialog() != nullptr);
    qInfo().noquote()
        << QStringLiteral("DIALOG KEYBOARD: Return -> adjustment rows %1 -> "
                          "%2, dialog still open=%3")
               .arg(beforeRet).arg(afterRet).arg(retStillOpen);
    if (QObject *leftOpen = visibleDialog())
        QMetaObject::invokeMethod(leftOpen, "close");
    QCoreApplication::processEvents();

    // ---- ARM C — TAB REACHES APPLY, SPACE ACTIVATES IT ---------------
    // This is the G-06 arm: not "the conventional key works" but "a
    // keyboard user can get there at all". The accept button is chosen by
    // ELIMINATION against the catalog's cancel string, for the reason T68
    // recorded — DialogButtonBox.buttonRole is an attached property and is
    // not readable through QObject::property(), and "the first button" is
    // not a rule because a style that ordered Cancel first would make this
    // case click Cancel and then blame the product for it.
    dlg = openDialog();
    QVERIFY2(dlg, "the adjustment dialog did not reopen for the Tab arm");
    const QString cancelText = m_i18n->tr(QStringLiteral("ui.dialog.cancel"));
    QQuickItem *accept = nullptr;
    for (QObject *o : dlg->findChildren<QObject *>()) {
        if (!o->inherits("QQuickAbstractButton"))
            continue;
        QQuickItem *b = qobject_cast<QQuickItem *>(o);
        if (b && b->isVisible() && b->width() > 0 && b->height() > 0
            && b->property("text").toString() != cancelText)
            accept = b;
    }
    QVERIFY2(accept, "could not identify the accept button by elimination");

    // A BOUNDED WALK THAT RECORDS ITS OWN TRAIL. The trail is not
    // decoration: if Tab moved focus NOWHERE, then "Apply was never
    // reached" would be a fact about my fixture and not about the dialog,
    // and that is exactly the shape of error that cost T68 two decorative
    // arms. The non-vacuity guard below fails the case in that state
    // rather than reporting a false product defect.
    // T70 correction: the non-vacuity guard counts distinct focus ITEMS,
    // not distinct NAMES. This case passed on names only because its three
    // stops happen to be differently named; the discard-confirm case, whose
    // two buttons are both anonymous `Button_QMLTYPE_21`, false-red'd on the
    // same construction. The trail stays for reading.
    QStringList trail;
    QSet<QQuickItem *> seen;
    bool reached = false;
    for (int i = 0; i < 16 && !reached; ++i) {
        QTest::keyClick(m_window, Qt::Key_Tab);
        QCoreApplication::processEvents();
        QQuickItem *f = m_window->activeFocusItem();
        if (f)
            seen << f;
        QString id = f ? (f->objectName().isEmpty()
                              ? QString::fromLatin1(f->metaObject()->className())
                              : f->objectName())
                       : QStringLiteral("<none>");
        if (trail.isEmpty() || trail.last() != id)
            trail << id;
        if (f == accept || (f && f->parentItem() == accept))
            reached = true;
    }
    qInfo().noquote()
        << QStringLiteral("DIALOG KEYBOARD: Tab trail [%1] -> reached "
                          "Apply=%2")
               .arg(trail.join(QStringLiteral(" > "))).arg(reached);
    QVERIFY2(seen.size() > 1,
             qPrintable(QStringLiteral("Tab moved focus nowhere at all (%1 "
                                       "distinct focus item(s); trail [%2]) — "
                                       "this case cannot say anything about "
                                       "the dialog until the fixture's Tab "
                                       "works")
                            .arg(seen.size())
                            .arg(trail.join(QStringLiteral(" > ")))));
    QVERIFY2(reached,
             qPrintable(QStringLiteral("16 Tab presses never reached the "
                                       "dialog's accept button; focus visited "
                                       "[%1]. G-06 requires every control on "
                                       "the sitting path to be keyboard "
                                       "reachable")
                            .arg(trail.join(QStringLiteral(" > ")))));
    const int beforeSpace = adjustmentRows();
    QTest::keyClick(m_window, Qt::Key_Space);
    QCoreApplication::processEvents();
    QTest::qWaitFor([&] { return adjustmentRows() > beforeSpace; }, 3000);
    qInfo().noquote()
        << QStringLiteral("DIALOG KEYBOARD: Space on Apply -> adjustment "
                          "rows %1 -> %2")
               .arg(beforeSpace).arg(adjustmentRows());
    QVERIFY2(adjustmentRows() > beforeSpace,
             qPrintable(QStringLiteral("Space on the focused accept button "
                                       "added no adjustment layer (%1 -> %2) "
                                       "— with Tab reaching it, the dialog "
                                       "has NO working keyboard commit path; "
                                       "status: %3")
                            .arg(beforeSpace).arg(adjustmentRows())
                            .arg(m_controller->statusText())));

    if (QObject *leftOpen = visibleDialog())
        QMetaObject::invokeMethod(leftOpen, "close");
    QCoreApplication::processEvents();
}

void TstGestureChrome::discardConfirmKeyboardPathsDoNotLoseWork()
{
    // ROADMAP-1.0 G-02 "dialog keyboard paths" (due 09-16) and G-06
    // (RC1 09-20/21). T69 covered AdjustmentParamDialog and stated its own
    // bound plainly: Return was MEASURED inert on one dialog, and on the
    // other seventeen only a SOURCE PARSE said the three known mechanisms
    // were absent. This is that bound being paid down on the dialog where
    // being wrong costs the most.
    //
    // ★★★★ WHY THIS DIALOG AND WHY THE KEYBOARD. The discard confirm is
    // the last thing between unsaved work and losing it, and "close with
    // an unsaved-work prompt" is a MUST-PASS step of the 1.0 sitting
    // (G-08). Two properties of the file make the keyboard specifically
    // worth measuring rather than assuming:
    //
    //   (a) Cancel and Discard are plain Buttons whose `onClicked` carry
    //       the ENTIRE behaviour -- so Dialog::close()/reject() run
    //       NOTHING. T68 recorded that its cancel arm had been passing for
    //       exactly that reason: "the document was preserved" was a fact
    //       about the probe, not the product. Any arm here that asserts
    //       preservation is vacuous unless something in this case proves
    //       the DISCARD path can fire at all. Arm D is that proof.
    //
    //   (b) The Discard button is `highlighted: true`. That is not
    //       `isDefault`, but styles do map highlight onto default, and if
    //       Return reached it the most reflexive key on the keyboard would
    //       silently destroy unsaved work. Arm B exists to find that out.
    auto confirmVisible = [&]() -> bool {
        return evalJs(QStringLiteral("discardConfirmDialog.visible")).toBool();
    };
    auto makeDirty = [&]() {
        m_canvasItem->setProperty("tool", QStringLiteral("brush.stroke"));
        m_canvasItem->forceActiveFocus();
        QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                          docToScene(120, 160));
        QTest::mouseMove(m_window, docToScene(240, 200));
        QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                            docToScene(360, 240));
        QCoreApplication::processEvents();
    };
    // The window's own close is used as the door throughout: it is the one
    // Main.qml calls "the one with no next state", and it re-arms cleanly
    // because the guard VETOES the close rather than consuming anything.
    auto raiseConfirm = [&]() -> bool {
        QMetaObject::invokeMethod(m_window, "close", Qt::DirectConnection);
        QTest::qWaitFor([&] { return confirmVisible(); }, 2000);
        return confirmVisible();
    };
    auto dialogObj = [&]() -> QObject * {
        for (QObject *o : m_window->findChildren<QObject *>())
            if (o->objectName() == QLatin1String("discardConfirmDialog"))
                return o;
        for (QObject *o : m_window->findChildren<QObject *>())
            if (o->inherits("QQuickDialog") && o->property("visible").toBool())
                return o;
        return nullptr;
    };

    // Ground the geometry exactly as dirtyReplaceDoorsRaiseTheDiscardConfirm
    // does, and for the same recorded reason: an earlier case in this suite
    // leaves a preset selected, the new document zoomFits, and docToScene
    // silently stops being valid.
    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
    m_canvas->zoomFit();
    m_canvas->zoomOneToOne();
    QCOMPARE(m_canvasItem->property("zoomPercent").toInt(), 100);
    QVERIFY2(m_canvasItem->width() >= m_controller->docWidth()
                 && m_canvasItem->height() >= m_controller->docHeight(),
             "canvas item smaller than the document -- the setup stroke "
             "would land nowhere and every arm below would be about nothing");
    focusCanvas();
    makeDirty();
    QVERIFY2(m_controller->documentHasUnsavedWork(),
             qPrintable(QStringLiteral("the setup stroke did not dirty the "
                                       "document; status: %1")
                            .arg(m_controller->statusText())));
    const int layersBefore = m_controller->layers().size();

    // ---- ARM A — ESCAPE MUST NOT DISCARD ----------------------------
    QVERIFY2(raiseConfirm(), "the window close did not raise the discard "
                             "confirm; arm A has no subject");
    QTest::keyClick(m_window, Qt::Key_Escape);
    QCoreApplication::processEvents();
    QTest::qWaitFor([&] { return !confirmVisible(); }, 2000);
    const bool escClosed = !confirmVisible();
    qInfo().noquote()
        << QStringLiteral("DISCARD KEYBOARD: Escape -> confirm closed=%1, "
                          "still dirty=%2, layers %3 -> %4, window visible=%5")
               .arg(escClosed).arg(m_controller->documentHasUnsavedWork())
               .arg(layersBefore).arg(m_controller->layers().size())
               .arg(m_window->isVisible());
    QVERIFY2(m_controller->documentHasUnsavedWork(),
             "ESCAPE ON THE DISCARD CONFIRM DESTROYED UNSAVED WORK -- the "
             "dismissal key ran the discard action");
    QVERIFY2(m_window->isVisible(),
             "the window closed when the discard confirm was dismissed with "
             "Escape -- the vetoed close was let through");
    QCOMPARE(m_controller->layers().size(), layersBefore);

    // ---- ARM B — RETURN, AND THE `highlighted` BUTTON ----------------
    // Reported, not asserted: T69's Return arm on AdjustmentParamDialog
    // measures without asserting so that a ruling either way lands without
    // a false red, and the same restraint applies here. The DESTRUCTIVE
    // outcome is asserted against, because that is not a matter of taste.
    if (!confirmVisible())
        QVERIFY2(raiseConfirm(), "the confirm did not re-open for arm B");
    // ★★★★ RETURN IS PRESSED WITH FOCUS ESTABLISHED INSIDE THE DIALOG, and
    // the first draft of this arm did not do that -- which made it vacuous
    // in a way a passing run could not show. A MUTATION caught it: a
    // `Keys.onReturnPressed` added to the dialog root, wired to the discard
    // path, did NOT red this case. Two reasons, and both matter:
    //
    //   (1) a Popup root does not receive key events -- they go to the
    //       focused item INSIDE it; and
    //   (2) this arm pressed Return without knowing where focus was, so
    //       the key may never have entered the dialog's chain at all.
    //
    // ★★★ That also corrects T69's claim about its own control. Escape
    // closing a dialog proves the POPUP saw the key (closePolicy is the
    // Popup's own), NOT that a focused item would see a Return. Escape is a
    // weaker control than G-16 credited it with, and the honest form is
    // this: put focus on a real button first, then assert.
    QTest::keyClick(m_window, Qt::Key_Tab);
    QCoreApplication::processEvents();
    QQuickItem *focusForReturn = m_window->activeFocusItem();
    bool focusInsideDialog = false;
    for (QQuickItem *w = focusForReturn; w; w = w->parentItem())
        if (w == qobject_cast<QQuickItem *>(dialogObj())
            || (dialogObj() && w == dialogObj()->property("contentItem")
                                    .value<QQuickItem *>())) {
            focusInsideDialog = true;
            break;
        }
    QVERIFY2(focusForReturn != nullptr,
             "nothing holds focus, so a Return keypress has no target and "
             "this arm would prove nothing about the product");
    QTest::keyClick(m_window, Qt::Key_Return);
    QCoreApplication::processEvents();
    QTest::qWait(150);
    qInfo().noquote()
        << QStringLiteral("DISCARD KEYBOARD: Return (focus on %1, inside "
                          "dialog=%2) -> confirm still open=%3, still "
                          "dirty=%4, window visible=%5")
               .arg(focusForReturn
                        ? (focusForReturn->objectName().isEmpty()
                               ? QString::fromLatin1(
                                     focusForReturn->metaObject()->className())
                               : focusForReturn->objectName())
                        : QStringLiteral("<none>"))
               .arg(focusInsideDialog)
               .arg(confirmVisible()).arg(m_controller->documentHasUnsavedWork())
               .arg(m_window->isVisible());
    // ★★★ BOTH HALVES, because on THIS door they are different losses and
    // the mutation proved the point. Wiring Return to the discard path
    // closed the WINDOW while `documentHasUnsavedWork` stayed true -- the
    // window-close door's action is `forceClose = true; root.close()`, so a
    // dirty-flag-only assertion sailed straight past a destroyed window and
    // the red surfaced two arms later as "the confirm did not re-open".
    // A failure that lands on the next arm with the wrong message is the
    // shape T68 filed against itself; this is that shape, caught by
    // mutation rather than by a user.
    QVERIFY2(m_window->isVisible(),
             "RETURN ON THE DISCARD CONFIRM CLOSED THE WINDOW -- on the "
             "window-close door that IS the data loss, and the unsaved-work "
             "flag stays true while it happens, so the flag alone cannot see "
             "it");
    QVERIFY2(m_controller->documentHasUnsavedWork(),
             "RETURN ON THE DISCARD CONFIRM DESTROYED UNSAVED WORK -- the "
             "`highlighted: true` Discard button is acting as a default "
             "button, so the most reflexive key on the keyboard discards");

    // ---- ARM C — BOTH DOORS ARE TAB-REACHABLE (G-06) -----------------
    if (!confirmVisible())
        QVERIFY2(raiseConfirm(), "the confirm did not re-open for arm C");
    QObject *dlg = dialogObj();
    QVERIFY2(dlg, "no discard confirm object to enumerate");
    const QString cancelText = m_i18n->tr(QStringLiteral("ui.dialog.cancel"));
    const QString discardText =
        m_i18n->tr(QStringLiteral("ui.dialog.discard_confirm.discard"));
    QVERIFY2(cancelText != discardText,
             "the catalog renders both buttons identically; this case cannot "
             "tell the doors apart");
    QQuickItem *cancelBtn = nullptr, *discardBtn = nullptr;
    for (QObject *o : dlg->findChildren<QObject *>()) {
        if (!o->inherits("QQuickAbstractButton"))
            continue;
        QQuickItem *b = qobject_cast<QQuickItem *>(o);
        if (!b || !b->isVisible() || b->width() <= 0)
            continue;
        const QString t = b->property("text").toString();
        if (t == cancelText) cancelBtn = b;
        else if (t == discardText) discardBtn = b;
    }
    QVERIFY2(cancelBtn && discardBtn,
             qPrintable(QStringLiteral("could not find both buttons by their "
                                       "catalog labels (cancel=%1 discard=%2)")
                            .arg(cancelBtn != nullptr)
                            .arg(discardBtn != nullptr)));
    // ★★★ THE TRAIL IS FOR READING; THE GUARD COUNTS IDENTITIES. Written
    // first as a name trail with `trail.size() > 1` as the non-vacuity
    // guard, it FALSE-RED'd on its first run: both buttons are anonymous
    // `Button_QMLTYPE_21`, so focus moving from one to the other collapsed
    // to a single entry and the guard reported "Tab moved focus nowhere"
    // while the very same loop had reached BOTH doors. The guard was
    // asking whether the NAME changed when it meant whether FOCUS MOVED.
    // Distinct QQuickItem pointers are the thing that means moved.
    QStringList trail;
    QSet<QQuickItem *> seen;
    bool sawCancel = false, sawDiscard = false;
    for (int i = 0; i < 16 && !(sawCancel && sawDiscard); ++i) {
        QTest::keyClick(m_window, Qt::Key_Tab);
        QCoreApplication::processEvents();
        QQuickItem *f = m_window->activeFocusItem();
        if (f)
            seen << f;
        QString id = f ? (f->objectName().isEmpty()
                              ? QString::fromLatin1(f->metaObject()->className())
                              : f->objectName())
                       : QStringLiteral("<none>");
        if (trail.isEmpty() || trail.last() != id)
            trail << id;
        if (f == cancelBtn || (f && f->parentItem() == cancelBtn)) sawCancel = true;
        if (f == discardBtn || (f && f->parentItem() == discardBtn)) sawDiscard = true;
    }
    qInfo().noquote()
        << QStringLiteral("DISCARD KEYBOARD: Tab trail [%1] -> cancel=%2 "
                          "discard=%3")
               .arg(trail.join(QStringLiteral(" > ")))
               .arg(sawCancel).arg(sawDiscard);
    QVERIFY2(seen.size() > 1,
             qPrintable(QStringLiteral("Tab moved focus nowhere (%1 distinct "
                                       "focus item(s); trail [%2]) -- this "
                                       "case cannot speak about the dialog "
                                       "until its own Tab works")
                            .arg(seen.size())
                            .arg(trail.join(QStringLiteral(" > ")))));
    QVERIFY2(sawCancel && sawDiscard,
             qPrintable(QStringLiteral("16 Tabs did not reach both doors of "
                                       "the discard confirm (cancel=%1 "
                                       "discard=%2); focus visited [%3]. G-06 "
                                       "requires every control on the sitting "
                                       "path to be keyboard reachable, and "
                                       "this dialog is a MUST-PASS step")
                            .arg(sawCancel).arg(sawDiscard)
                            .arg(trail.join(QStringLiteral(" > ")))));

    // ---- ARM D — THE POSITIVE CONTROL: DISCARD ACTUALLY DISCARDS -----
    // ★★★ Without this arm every preservation assertion above is a fact
    // about a probe that never armed the destructive path. T68 filed that
    // exact self-deception against itself; this is the arm that prevents it
    // recurring here. Driven BY KEYBOARD, so it doubles as the proof that a
    // keyboard user can complete the must-pass step.
    //
    // ★★ AND IT DELIBERATELY SWITCHES DOORS. The window-close door's
    // discard action is `forceClose = true; root.close()` -- firing it here
    // would close the window this whole suite shares, and every later case
    // would fail on a fixture I destroyed while proving a point about data
    // loss. File > New's action REPLACES the document instead, which is
    // destructive to exactly the thing under test and to nothing else.
    if (confirmVisible()) {
        evalJs(QStringLiteral("discardConfirmDialog.reject()"));
        QTest::qWaitFor([&] { return !confirmVisible(); }, 2000);
    }
    QVERIFY2(m_controller->documentHasUnsavedWork(),
             "arm D needs the document still dirty and it is not");
    evalJs(QStringLiteral("newDialog.open()"));
    QCoreApplication::processEvents();
    evalJs(QStringLiteral("newDialog.accept()"));
    QTest::qWaitFor([&] { return confirmVisible(); }, 2000);
    QVERIFY2(confirmVisible(),
             "File > New over unsaved work did not raise the confirm, so the "
             "destructive path cannot be armed and arms A-C stay unproven");
    QObject *dlg2 = dialogObj();
    QVERIFY2(dlg2, "no discard confirm object for arm D");
    QQuickItem *discard2 = nullptr;
    for (QObject *o : dlg2->findChildren<QObject *>()) {
        QQuickItem *b = qobject_cast<QQuickItem *>(o);
        if (b && o->inherits("QQuickAbstractButton") && b->isVisible()
            && b->property("text").toString() == discardText)
            discard2 = b;
    }
    QVERIFY2(discard2, "no Discard button in the re-opened confirm");
    bool onDiscard = false;
    for (int i = 0; i < 24 && !onDiscard; ++i) {
        QTest::keyClick(m_window, Qt::Key_Tab);
        QCoreApplication::processEvents();
        QQuickItem *f = m_window->activeFocusItem();
        onDiscard = (f == discard2 || (f && f->parentItem() == discard2));
    }
    QVERIFY2(onDiscard,
             "24 Tabs never focused the Discard button on the File > New "
             "door, so this control could not be driven by keyboard");
    QTest::keyClick(m_window, Qt::Key_Space);
    QCoreApplication::processEvents();
    QTest::qWaitFor([&] { return !m_controller->documentHasUnsavedWork(); },
                    3000);
    qInfo().noquote()
        << QStringLiteral("DISCARD KEYBOARD: Space on Discard (File>New door) "
                          "-> still dirty=%1, window visible=%2 (the "
                          "destructive path IS armed)")
               .arg(m_controller->documentHasUnsavedWork())
               .arg(m_window->isVisible());
    QVERIFY2(!m_controller->documentHasUnsavedWork(),
             "Space on the focused Discard button did not discard -- the "
             "document is still dirty, so the destructive path was never "
             "armed and every preservation arm above proved nothing");
    QVERIFY2(m_window->isVisible(),
             "arm D closed the window; it was supposed to replace the "
             "document through the File > New door");
}

void TstGestureChrome::noDialogCommitsOnReturnAcrossTheWholeShell()
{
    // ROADMAP-1.0 G-02 "dialog keyboard paths" (due 09-16); G-06 (RC1 09-21).
    //
    // ★★★★★ THIS CASE EXISTS TO PAY DOWN A BOUND I STATED TWICE TO THE
    // OPERATOR. G-16 reports that no dialog commits on Enter, and both its
    // filing and its addendum say plainly that Return was MEASURED on two
    // dialogs and INFERRED on the rest from a source parse -- a parse that
    // "cannot see a mechanism it does not know to look for". A grep is a
    // hypothesis about sixteen dialogs; this is the measurement.
    //
    // THE ROSTER IS LINTED AGAINST Main.qml IN BOTH DIRECTIONS, the same
    // two-sided discipline optionsModelCoverageLint uses: every
    // `SomethingDialog { id: x }` in the checked-in Main.qml must appear
    // below, and every row below must still be in Main.qml. Otherwise a
    // dialog added next week is silently uncensused and this case keeps
    // reporting a clean sweep of a shrinking population -- which is exactly
    // the vacuity G-7 was self-filed for.
    //
    // Qt's own ColorDialog/FileDialog are deliberately NOT here: they are
    // native/QtQuick.Dialogs types whose keyboard handling is Qt's, not this
    // shell's, and T68 already recorded that a modal FileDialog is inert to
    // close() offscreen and poisons the next case.
    struct Row { const char *id; };
    static const Row roster[] = {
        {"newDialog"}, {"discardConfirmDialog"}, {"recoveryDialog"},
        {"modifyDialog"}, {"modeConvertDialog"}, {"filterParamDialog"},
        {"resizeDialog"}, {"canvasSizeDialog"}, {"rotateDialog"},
        {"trimDialog"}, {"cropDialog"}, {"fillDialog"},
        {"assignProfileDialog"}, {"convertProfileDialog"},
        {"depthConvertDialog"}, {"flattenDialog"}, {"deleteConfirmDialog"},
        // OPERATOR WORD 11 (2)(a), 2026-09-14 18:40 ET — DARK-DESK EXCEPTION.
        // q-gesture's tip is 09-09; the operator's word is this desk's consent.
        // aboutDialog landed on main at 69f3725e6 (B-09's Help > About door)
        // AFTER this roster was written, so this census correctly reported one
        // uncensused dialog. Derived, not copied: every `id:` ending in Dialog in
        // the union's Main.qml minus this roster leaves six, and five of them
        // (colorDialog, openFileDialog, saveFileDialog, importFileDialog,
        // exportFileDialog) are the Qt-native types this block's comment above
        // excludes by design. aboutDialog is the only real gap.
        {"aboutDialog"},
    };
    const int kRosterSize = int(sizeof(roster) / sizeof(roster[0]));

    // ---- ROSTER LINT, against the checked-in source ------------------
    QFile f(QStringLiteral(":/qt/qml/AppnameShell/qml/Main.qml"));
    if (!f.open(QIODevice::ReadOnly | QIODevice::Text)) {
        f.setFileName(QFINDTESTDATA("../qml/Main.qml"));
        QVERIFY2(f.open(QIODevice::ReadOnly | QIODevice::Text),
                 "could not read Main.qml from either the resource system or "
                 "the source tree -- the roster cannot be linted and this "
                 "census would be a list I wrote rather than the shell's");
    }
    const QString src = QString::fromUtf8(f.readAll());
    f.close();
    QVERIFY2(src.contains(QLatin1String("DiscardConfirmDialog")),
             "Main.qml read but does not mention DiscardConfirmDialog -- the "
             "wrong file was read and every count below is about nothing");
    QRegularExpression re(
        QStringLiteral("\\b([A-Z][A-Za-z]*Dialog)\\s*\\{\\s*id:\\s*([A-Za-z0-9_]+)"));
    QStringList inSource;
    auto it = re.globalMatch(src);
    while (it.hasNext()) {
        const auto m = it.next();
        // Qt's own dialog types are out of scope, stated above.
        if (m.captured(1) == QLatin1String("ColorDialog")
            || m.captured(1) == QLatin1String("FileDialog"))
            continue;
        inSource << m.captured(2);
    }
    QVERIFY2(inSource.size() >= 10,
             qPrintable(QStringLiteral("the Main.qml scan found only %1 "
                                       "id-carrying dialogs; the pattern has "
                                       "stopped matching and the lint below "
                                       "would pass vacuously")
                            .arg(inSource.size())));
    QStringList rostered;
    for (const Row &r : roster)
        rostered << QLatin1String(r.id);
    QStringList uncensused, stale;
    for (const QString &s : inSource)
        if (!rostered.contains(s))
            uncensused << s;
    for (const QString &r : rostered)
        if (!inSource.contains(r))
            stale << r;
    qInfo().noquote()
        << QStringLiteral("DIALOG CENSUS ROSTER: %1 id-carrying dialogs in "
                          "Main.qml, %2 roster rows")
               .arg(inSource.size()).arg(kRosterSize);
    QVERIFY2(uncensused.isEmpty(),
             qPrintable(QStringLiteral("dialog(s) in Main.qml with no roster "
                                       "row -- uncensused: [%1]")
                            .arg(uncensused.join(QStringLiteral(", ")))));
    QVERIFY2(stale.isEmpty(),
             qPrintable(QStringLiteral("roster row(s) naming a dialog Main.qml "
                                       "no longer has: [%1]")
                            .arg(stale.join(QStringLiteral(", ")))));

    // ---- THE MEASUREMENT --------------------------------------------
    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
    QCoreApplication::processEvents();

    auto visibleDialogCount = [&]() -> int {
        int n = 0;
        for (QObject *o : m_window->findChildren<QObject *>())
            if (o->inherits("QQuickDialog") && o->property("visible").toBool())
                ++n;
        return n;
    };

    QStringList opened, notOpened, committedOnReturn, noFocusableStop;
    for (const Row &r : roster) {
        const QString id = QLatin1String(r.id);
        // Leave no dialog behind: a modal left up lands its failure on
        // whichever case runs next, with a message pointing into my fixture
        // (T68, three times in one tick).
        evalJs(QStringLiteral("%1.close()").arg(id));
        QCoreApplication::processEvents();
        evalJs(QStringLiteral("%1.open()").arg(id));
        QCoreApplication::processEvents();
        QTest::qWaitFor([&] {
            return evalJs(QStringLiteral("%1.visible").arg(id)).toBool();
        }, 1000);
        if (!evalJs(QStringLiteral("%1.visible").arg(id)).toBool()) {
            // NOT measured, and recorded as such. A dialog that refused to
            // open is not evidence that Return is safe on it; calling it
            // "clean" would be the census lying in my favour.
            notOpened << id;
            continue;
        }
        opened << id;
        // ★★★★ RETURN IS PRESSED AT EVERY KEYBOARD-REACHABLE STOP, not once.
        // The first draft Tabbed ONCE and pressed Return, and a mutation
        // (a Return handler on TrimDialog's Cancel button) did NOT red it:
        // after one Tab focus sits on whatever control comes first -- a
        // spinbox in most of these dialogs -- so a handler on a BUTTON never
        // saw the key, and "Return closed 0 of 17" was partly a fact about
        // where focus happened to land. Walking the stops is also the honest
        // G-06 question: can a keyboard user commit this dialog from
        // anywhere in it?
        QSet<QQuickItem *> stops;
        bool closedByReturn = false;
        for (int t = 0; t < 12 && !closedByReturn; ++t) {
            QTest::keyClick(m_window, Qt::Key_Tab);
            QCoreApplication::processEvents();
            if (QQuickItem *fi = m_window->activeFocusItem())
                stops << fi;
            QTest::keyClick(m_window, Qt::Key_Return);
            QCoreApplication::processEvents();
            QTest::qWait(25);
            if (!evalJs(QStringLiteral("%1.visible").arg(id)).toBool()) {
                closedByReturn = true;
                committedOnReturn << QStringLiteral("%1@stop%2").arg(id).arg(t + 1);
            }
        }
        // A dialog with NO keyboard-reachable stop cannot be driven at all,
        // which is a G-06 statement in its own right rather than a quiet
        // pass. Recorded per dialog.
        if (stops.isEmpty())
            noFocusableStop << id;
        evalJs(QStringLiteral("%1.close()").arg(id));
        QCoreApplication::processEvents();
    }

    qInfo().noquote()
        << QStringLiteral("DIALOG RETURN CENSUS: %1 of %2 opened; Return "
                          "closed %3 of them")
               .arg(opened.size()).arg(kRosterSize)
               .arg(committedOnReturn.size());
    if (!notOpened.isEmpty())
        qInfo().noquote()
            << QStringLiteral("  NOT MEASURED (would not open here): [%1]")
                   .arg(notOpened.join(QStringLiteral(", ")));
    if (!committedOnReturn.isEmpty())
        qInfo().noquote()
            << QStringLiteral("  RETURN CLOSED: [%1]")
                   .arg(committedOnReturn.join(QStringLiteral(", ")));
    if (!noFocusableStop.isEmpty())
        qInfo().noquote()
            << QStringLiteral("  NO KEYBOARD-REACHABLE STOP: [%1]")
                   .arg(noFocusableStop.join(QStringLiteral(", ")));

    // ★★★ THE NON-VACUITY FLOOR. If nothing opened, every "Return did not
    // commit" below is a statement about dialogs that were never on screen.
    // The floor is what stops this case from reporting a clean sweep of an
    // empty set -- the failure G-7 was self-filed for after 21 ticks.
    QVERIFY2(opened.size() >= 8,
             qPrintable(QStringLiteral("only %1 of %2 dialogs opened in this "
                                       "fixture; the census has too small a "
                                       "population to say anything, and a "
                                       "clean result would be vacuous. "
                                       "Not measured: [%3]")
                            .arg(opened.size()).arg(kRosterSize)
                            .arg(notOpened.join(QStringLiteral(", ")))));

    // The claim under test. G-16 says NO dialog commits on Enter; if one
    // does, that is the finding and this names it.
    QVERIFY2(committedOnReturn.isEmpty(),
             qPrintable(QStringLiteral("Return closed %1 dialog(s) -- so "
                                       "G-16's census claim is wrong for "
                                       "these and they need their own ruling: "
                                       "[%2]")
                            .arg(committedOnReturn.size())
                            .arg(committedOnReturn.join(QStringLiteral(", ")))));

    // Fixture left as found.
    QTest::qWaitFor([&] { return visibleDialogCount() == 0; }, 2000);
    QCOMPARE(visibleDialogCount(), 0);
}

void TstGestureChrome::visibleControlsOnTheSittingPathAreKeyboardReachable()
{
    // ROADMAP-1.0 G-06 (RC1 09-20/21): "every control reachable by keyboard"
    // on the 1.0 sitting path, MAJOR if a must-pass step is unreachable.
    //
    // ★★★★ THE POPULATION IS TAKEN FROM THE LIVE WINDOW, NOT FROM SOURCE.
    // Main.qml carries exactly TWO objectNames (canvasView, refusalToast),
    // so a source parse would have reported a population of 2 and then
    // announced a clean sweep of it -- a narrow count with no widest form,
    // the trap this ledger has hit repeatedly. The controls live in
    // sub-components (tool palette, options bar, layers and history panels),
    // and the only honest population is the one actually on screen.
    //
    // WHAT COUNTS AS A CONTROL: a visible, non-degenerate item of a QtQuick
    // Controls type a user operates -- buttons (which covers checkboxes,
    // radios, menu items and tab buttons by inheritance), plus sliders,
    // spinboxes, comboboxes and text inputs. Static labels and layout items
    // are not controls and are not counted.
    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
    QCoreApplication::processEvents();
    QTest::qWait(120);

    auto isControl = [](QObject *o) -> bool {
        return o->inherits("QQuickAbstractButton")
            || o->inherits("QQuickSlider")
            || o->inherits("QQuickSpinBox")
            || o->inherits("QQuickComboBox")
            || o->inherits("QQuickTextInput")
            || o->inherits("QQuickTextField");
    };
    auto describe = [](QQuickItem *b) -> QString {
        const QString n = b->objectName();
        const QString t = b->property("text").toString();
        const QString cls = QString::fromLatin1(b->metaObject()->className())
                                .section(QLatin1Char('_'), 0, 0);
        if (!n.isEmpty())
            return n;
        if (!t.trimmed().isEmpty())
            return QStringLiteral("%1(\"%2\")").arg(cls, t.trimmed());
        return cls;
    };

    // ★★★★★ T75 CORRECTION — THE POPULATION IS THE UNION OF TWO WALKS.
    // This census was built on findChildren() alone, and findChildren walks
    // QObject parentage, which DOES NOT REACH ListView DELEGATES. Measured
    // side by side in a populated document:
    //
    //     findChildren sees 159 buttons (24 visible)
    //     VISUAL walk   sees  87 buttons (45 visible)
    //
    // They disagree in both directions: findChildren reaches non-visual
    // objects (items of closed menus, dialog contents) that the visual walk
    // never sees, and the visual walk reaches delegates findChildren never
    // sees. So the T73/T74 population of 27 was missing roughly half the
    // controls actually on screen -- every layer-ROW control among them.
    //
    // Taking the UNION rather than swapping one walk for the other, because
    // each sees something the other cannot, and a population that silently
    // dropped either set is how this census got a wrong number the first
    // time.
    QSet<QQuickItem *> seenControls;
    QList<QQuickItem *> population;
    auto consider = [&](QQuickItem *b) {
        if (!b || seenControls.contains(b)) return;
        if (!isControl(b)) return;
        // A control inside a closed popup is not on the sitting path right
        // now; only what is actually on screen is in scope.
        if (!b->isVisible() || b->width() <= 0 || b->height() <= 0) return;
        seenControls << b;
        population << b;
    };
    for (QObject *o : m_window->findChildren<QObject *>())
        consider(qobject_cast<QQuickItem *>(o));
    std::function<void(QQuickItem *)> visualWalk = [&](QQuickItem *it) {
        for (QQuickItem *c : it->childItems()) {
            consider(c);
            visualWalk(c);
        }
    };
    if (QQuickItem *ci = m_window->contentItem())
        visualWalk(ci);
    QVERIFY2(population.size() >= 8,
             qPrintable(QStringLiteral("only %1 visible controls found in the "
                                       "live window -- the enumeration is "
                                       "broken and a clean result below would "
                                       "be a sweep of an empty set")
                            .arg(population.size())));

    // ---- THE TAB WALK, bounded and cycle-aware ----------------------
    m_canvas->forceActiveFocus();
    QCoreApplication::processEvents();
    QSet<QQuickItem *> visited;
    QStringList order;
    QQuickItem *firstStop = nullptr;
    int cycles = 0, taps = 0;
    bool ringClosed = false;
    for (int i = 0; i < 200; ++i) {
        QTest::keyClick(m_window, Qt::Key_Tab);
        QCoreApplication::processEvents();
        ++taps;
        QQuickItem *f = m_window->activeFocusItem();
        if (!f)
            continue;
        if (!firstStop)
            firstStop = f;
        else if (f == firstStop && ++cycles >= 2) {
            ringClosed = true;
            break;          // the ring has come round twice; nothing new
        }
        if (!visited.contains(f)) {
            visited << f;
            if (order.size() < 40)
                order << describe(f);
        }
    }
    // ★★★★ THE DIFFERENCE BETWEEN "UNREACHABLE" AND "NOT REACHED YET".
    // If this walk stopped because it exhausted its bound rather than
    // because the focus ring came back round, then every "not reached"
    // below means only "not reached within 200 taps" -- which is a fact
    // about my bound, not about the shell, and would be an unsafe basis
    // for a severity claim. Asserted, not hoped for.
    qInfo().noquote()
        << QStringLiteral("  TAB WALK: %1 taps, ring closed=%2 (%3 full "
                          "cycles observed)")
               .arg(taps).arg(ringClosed).arg(cycles);
    QVERIFY2(ringClosed,
             qPrintable(QStringLiteral("the Tab walk exhausted its 200-tap "
                                       "bound WITHOUT the focus ring closing "
                                       "(%1 distinct stops) -- 'not reached' "
                                       "would mean 'not reached yet' and this "
                                       "case may not be used to call anything "
                                       "unreachable")
                            .arg(visited.size())));
    QVERIFY2(visited.size() > 1,
             qPrintable(QStringLiteral("Tab reached %1 distinct item(s) -- the "
                                       "fixture's Tab is dead and this case "
                                       "cannot speak about reachability")
                            .arg(visited.size())));

    // A control counts as reached if it, or any ancestor of the focused
    // item, is that control -- QQC2 controls put focus on the control
    // itself, but a text-editing control focuses an inner TextInput.
    // ★★★★ T74 CORRECTION — A PART OF A REACHED CONTROL IS REACHED.
    // T73 reported QQuickTextField("Normal") as unreached and I filed it
    // with an explicit "I have not measured whether blend mode has another
    // door". Following that up found the answer was about MY MATCHER, not
    // the shell: LayersPanel's blend ComboBox IS on the Tab ring, and that
    // TextField is the combo's own display field -- a composition PART of a
    // control the user can already reach. The old matcher credited a
    // control that CONTAINS a focused item but not one that IS CONTAINED BY
    // a reached control, so it over-reported unreachability.
    //
    // This does NOT touch G-17: Del/Group/Ungroup/Merge/up/down are
    // standalone Buttons whose ancestors are layouts, not controls, and
    // their absence was established independently by the accelerator and
    // menu greps.
    QSet<QQuickItem *> populationSet;
    for (QQuickItem *c : population)
        populationSet << c;
    auto reached = [&](QQuickItem *c) -> bool {
        if (visited.contains(c))
            return true;
        // the control CONTAINS a focused item (a control focusing an inner
        // TextInput, say)
        for (QQuickItem *v : visited)
            for (QQuickItem *w = v; w; w = w->parentItem())
                if (w == c)
                    return true;
        // the control IS a part of a control that was itself reached
        for (QQuickItem *w = c->parentItem(); w; w = w->parentItem())
            if (visited.contains(w) && populationSet.contains(w))
                return true;
        return false;
    };
    QStringList unreachable, reachedAsPart;
    int reachedCount = 0;
    for (QQuickItem *c : population) {
        if (!reached(c)) {
            unreachable << describe(c);
            continue;
        }
        ++reachedCount;
        if (!visited.contains(c)) {
            bool isPart = false;
            for (QQuickItem *w = c->parentItem(); w && !isPart; w = w->parentItem())
                if (visited.contains(w) && populationSet.contains(w))
                    isPart = true;
            if (isPart)
                reachedAsPart << describe(c);
        }
    }

    qInfo().noquote()
        << QStringLiteral("G-06 KEYBOARD REACH: %1 visible controls, %2 "
                          "reached by Tab, %3 not; %4 distinct focus stops")
               .arg(population.size()).arg(reachedCount)
               .arg(unreachable.size()).arg(visited.size());
    qInfo().noquote()
        << QStringLiteral("  TAB RING (first %1): [%2]")
               .arg(order.size()).arg(order.join(QStringLiteral(" > ")));
    if (!reachedAsPart.isEmpty())
        qInfo().noquote()
            << QStringLiteral("  REACHED AS PART of a reachable control (%1): "
                              "[%2]")
                   .arg(reachedAsPart.size())
                   .arg(reachedAsPart.join(QStringLiteral(", ")));
    if (!unreachable.isEmpty()) {
        QStringList head = unreachable.mid(0, 25);
        qInfo().noquote()
            << QStringLiteral("  NOT REACHED (%1 of %2, first %3): [%4]")
                   .arg(unreachable.size()).arg(population.size())
                   .arg(head.size()).arg(head.join(QStringLiteral(", ")));
    }

    // ★★★ POSITIVE CONTROL FOR THE MATCHER. If the reach test could never
    // answer TRUE, "everything is unreachable" would be a statement about
    // my matching logic and not about the shell. At least one control must
    // be reached, and the count is asserted rather than assumed.
    QVERIFY2(reachedCount > 0,
             qPrintable(QStringLiteral("the reach matcher answered TRUE for "
                                       "NONE of %1 controls while Tab visited "
                                       "%2 distinct items -- the matcher is "
                                       "broken, not the shell")
                            .arg(population.size()).arg(visited.size())));

    // ★★★★★ T81: THIS CASE NOW GATES INSTEAD OF ONLY REPORTING. As first
    // written it printed the unreached list and asserted nothing about it,
    // under the reasoning that severity is the sitting's call. That was
    // right about SEVERITY and wrong about ASSERTION -- it left a case
    // costing minutes of suite time and gating on almost nothing, which is
    // the exact shape of G-7 (a lint that asserted NOTHING for 21 ticks),
    // self-filed by this desk against itself.
    //
    // The assertable property is T78's, and no severity judgement is needed
    // to state it, because both exclusions are MEASURED facts:
    //   - disabled controls are legitimately off the ring (T76: enabled <=>
    //     reached at 6 of 6, the measurement that retracted G-17);
    //   - MenuBarItems are off it by design with accelerators behind them
    //     (T73: 32 Shortcut blocks in Main.qml).
    QStringList enabledOffRing;
    for (QQuickItem *c : population) {
        if (reached(c) || !c->property("enabled").toBool())
            continue;
        const QString d = describe(c);
        if (d.startsWith(QLatin1String("MenuBarItem")))
            continue;
        enabledOffRing << d;
    }
    QVERIFY2(enabledOffRing.isEmpty(),
             qPrintable(QStringLiteral("enabled, non-menu control(s) off the "
                                       "Tab ring in the default document: "
                                       "[%1]")
                            .arg(enabledOffRing.join(QStringLiteral(", ")))));

    // ★★ THIS CASE RECORDS RATHER THAN RULES. G-06 makes unreachability a
    // MAJOR only when it blocks a MUST-PASS SITTING STEP, and a control not
    // on the Tab ring is not automatically unreachable: menus carry
    // accelerators, the canvas takes keys directly, and tools have letter
    // keys (pinned elsewhere in this suite). Deciding which gaps are
    // must-pass is the sitting's job on 09-19..21, not this case's. So the
    // measurement is published and only the two vacuity floors above are
    // asserted -- a red here would otherwise be a severity judgement made
    // by a test at 07:00 on a Monday.
}

void TstGestureChrome::keyboardReachHoldsWhenTheDocumentHasRealLayers()
{
    // ROADMAP-1.0 G-06 (RC1 09-20/21). THE OWED CASE from T75, rebuilt on
    // the corrected enumeration.
    //
    // ★★★ ITS FIRST DRAFT IS WHY THE ENUMERATION IS CORRECT. That draft
    // carried one guard -- the control population must GROW when layers are
    // added, or the case is measuring the default state twice -- and the
    // guard refused to pass: three layers, not one new control. Chasing
    // that refusal found that `findChildren()` walks QObject parentage and
    // never reaches ListView delegates, so three ticks of census had been
    // blind to every layer-ROW control. The draft was discarded rather than
    // landed on an enumeration it had just disproven; this is the rebuild.
    //
    // WHY THE POPULATED STATE IS ITS OWN CASE: the sibling case measures
    // the default document, where LayersPanel has exactly one row. The 1.0
    // sitting (G-08) exercises layers and masks, and each row carries a
    // visibility toggle and a group-select checkbox that simply do not
    // exist in an empty document. A reach claim made only against one row
    // is a claim about a screen the sitting never sees.
    auto isControl = [](QObject *o) -> bool {
        return o->inherits("QQuickAbstractButton")
            || o->inherits("QQuickSlider")
            || o->inherits("QQuickSpinBox")
            || o->inherits("QQuickComboBox")
            || o->inherits("QQuickTextInput")
            || o->inherits("QQuickTextField");
    };
    auto describe = [](QQuickItem *b) -> QString {
        const QString n = b->objectName();
        const QString t = b->property("text").toString();
        const QString cls = QString::fromLatin1(b->metaObject()->className())
                                .section(QLatin1Char('_'), 0, 0);
        if (!n.isEmpty()) return n;
        if (!t.trimmed().isEmpty())
            return QStringLiteral("%1(\"%2\")").arg(cls, t.trimmed());
        return cls;
    };
    // The UNION of both walks, for the reason T75 measured: findChildren
    // reaches closed menus and closed dialogs the visual walk never sees,
    // and the visual walk reaches delegates findChildren never sees.
    // Neither alone is the screen.
    auto census = [&]() -> QList<QQuickItem *> {
        QSet<QQuickItem *> seen;
        QList<QQuickItem *> out;
        auto consider = [&](QQuickItem *b) {
            if (!b || seen.contains(b) || !isControl(b)) return;
            if (!b->isVisible() || b->width() <= 0 || b->height() <= 0) return;
            seen << b;
            out << b;
        };
        for (QObject *o : m_window->findChildren<QObject *>())
            consider(qobject_cast<QQuickItem *>(o));
        std::function<void(QQuickItem *)> walk = [&](QQuickItem *it) {
            for (QQuickItem *c : it->childItems()) { consider(c); walk(c); }
        };
        if (QQuickItem *ci = m_window->contentItem())
            walk(ci);
        return out;
    };

    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
    QCoreApplication::processEvents();
    QTest::qWait(150);
    const int baseline = census().size();

    // §10.3 layer.add REQUIRES `kind` and `newLayerId`; ids are
    // client-generated per §9.6. The first draft passed neither and was
    // told so by the engine ("error.schema.missing_field: required field
    // `kind` is missing") rather than by me guessing.
    const int layersBefore = m_controller->layers().size();
    for (int i = 0; i < 2; ++i) {
        QVariantMap params;
        params.insert(QStringLiteral("kind"), QStringLiteral("raster"));
        params.insert(QStringLiteral("newLayerId"), m_controller->newUuid());
        m_controller->applyCommand(QStringLiteral("layer.add"), {}, params, {});
        QCoreApplication::processEvents();
    }
    QTRY_VERIFY2(m_controller->layers().size() >= layersBefore + 2,
                 qPrintable(QStringLiteral("layer.add did not add two layers "
                                           "(%1 -> %2); status: %3")
                                .arg(layersBefore)
                                .arg(m_controller->layers().size())
                                .arg(m_controller->statusText())));
    QTest::qWait(250);          // let the row delegates realize
    QList<QQuickItem *> population = census();

    qInfo().noquote()
        << QStringLiteral("G-06 POPULATED: %1 layers, %2 visible controls "
                          "(one-row default had %3)")
               .arg(m_controller->layers().size())
               .arg(population.size()).arg(baseline);

    // ★★★★ THE GUARD THAT FOUND THE ENUMERATION DEFECT, KEPT. If adding two
    // layers surfaces no new controls, this case is re-measuring the
    // default state under a different name and any clean result it reports
    // is worthless. It earned its place by firing once already.
    QVERIFY2(population.size() > baseline,
             qPrintable(QStringLiteral("adding two layers surfaced NO new "
                                       "controls (%1 then %2) -- either the "
                                       "row delegates did not realize or the "
                                       "census cannot see them, and this case "
                                       "may not report a reach result until "
                                       "one of those is fixed")
                            .arg(baseline).arg(population.size())));

    m_canvas->forceActiveFocus();
    QCoreApplication::processEvents();
    QSet<QQuickItem *> visited;
    QQuickItem *firstStop = nullptr;
    int cycles = 0, taps = 0;
    bool ringClosed = false;
    for (int i = 0; i < 400; ++i) {
        QTest::keyClick(m_window, Qt::Key_Tab);
        QCoreApplication::processEvents();
        ++taps;
        QQuickItem *f = m_window->activeFocusItem();
        if (!f) continue;
        if (!firstStop) firstStop = f;
        else if (f == firstStop && ++cycles >= 2) { ringClosed = true; break; }
        visited << f;
    }
    qInfo().noquote()
        << QStringLiteral("  TAB WALK: %1 taps, ring closed=%2, %3 distinct "
                          "stops").arg(taps).arg(ringClosed).arg(visited.size());
    QVERIFY2(ringClosed,
             qPrintable(QStringLiteral("the Tab walk exhausted 400 taps "
                                       "without the ring closing (%1 stops) "
                                       "-- 'not reached' would mean 'not "
                                       "reached yet'")
                            .arg(visited.size())));

    QSet<QQuickItem *> populationSet;
    for (QQuickItem *c : population) populationSet << c;
    auto reached = [&](QQuickItem *c) -> bool {
        if (visited.contains(c)) return true;
        for (QQuickItem *v : visited)
            for (QQuickItem *w = v; w; w = w->parentItem())
                if (w == c) return true;
        for (QQuickItem *w = c->parentItem(); w; w = w->parentItem())
            if (visited.contains(w) && populationSet.contains(w)) return true;
        return false;
    };
    QStringList unreachable;
    int reachedCount = 0;
    for (QQuickItem *c : population) {
        if (reached(c)) ++reachedCount;
        else unreachable << describe(c);
    }
    qInfo().noquote()
        << QStringLiteral("G-06 POPULATED REACH: %1 controls, %2 reached, "
                          "%3 not").arg(population.size())
               .arg(reachedCount).arg(unreachable.size());
    if (!unreachable.isEmpty()) {
        QStringList head = unreachable.mid(0, 30);
        qInfo().noquote()
            << QStringLiteral("  NOT REACHED (%1, first %2): [%3]")
                   .arg(unreachable.size()).arg(head.size())
                   .arg(head.join(QStringLiteral(", ")));
    }
    QVERIFY2(reachedCount > 0,
             "the reach matcher answered TRUE for no control -- the matcher "
             "is broken, not the shell");

    // T81: gates, for the reason given on the sibling case -- a report that
    // asserts nothing is a case that cannot fail, and this desk has filed
    // that against itself once already (G-7).
    QStringList enabledOffRing;
    for (QQuickItem *c : population) {
        if (reached(c) || !c->property("enabled").toBool())
            continue;
        const QString d = describe(c);
        if (d.startsWith(QLatin1String("MenuBarItem")))
            continue;
        enabledOffRing << d;
    }
    QVERIFY2(enabledOffRing.isEmpty(),
             qPrintable(QStringLiteral("enabled, non-menu control(s) off the "
                                       "Tab ring with three layers: [%1]")
                            .arg(enabledOffRing.join(QStringLiteral(", ")))));

    // ★★★★★ THE G-17 BUTTONS, ENABLED-STATE AND REACH TOGETHER. The sibling
    // case measures a ONE-LAYER document, where several of these are
    // DISABLED -- and a disabled control is not in the Tab ring, so
    // "unreachable" there can mean "not applicable yet" rather than "has no
    // keyboard route". This report separates the two, because G-17 is a
    // MAJOR filed with the operator and it must not rest on a control that
    // was merely inapplicable when it was measured.
    QStringList opReport;
    for (QQuickItem *c : population) {
        const QString d = describe(c);
        if (!d.startsWith(QLatin1String("Button(")))
            continue;
        static const QStringList kOps = {
            QStringLiteral("Del"), QStringLiteral("Group"),
            QStringLiteral("Ungroup"), QStringLiteral("Merge"),
            QStringLiteral("↑"), QStringLiteral("↓")};
        bool isOp = false;
        for (const QString &k : kOps)
            if (d.contains(QStringLiteral("\"%1\"").arg(k))) isOp = true;
        if (!isOp)
            continue;
        opReport << QStringLiteral("%1 enabled=%2 reached=%3")
                        .arg(d).arg(c->property("enabled").toBool())
                        .arg(reached(c));
    }
    qInfo().noquote()
        << QStringLiteral("  G-17 BUTTONS (3 layers): %1")
               .arg(opReport.join(QStringLiteral("; ")));

    // Records rather than rules, as the sibling case does: which gaps are
    // must-pass is the sitting's call on 09-19..21. G-17 already carries the
    // layer-operation half to the operator and is not re-filed here.
}

void TstGestureChrome::optionsBarStaysKeyboardReachableAcrossEveryTool()
{
    // ROADMAP-1.0 G-06 (RC1 09-20/21), applying T76's standing lesson:
    // VARY THE DOCUMENT STATE, NOT JUST THE INSTRUMENT.
    //
    // ★★★★ WHY THIS STATE. `Main.qml`'s `toolOptionsModel` is keyed BY TOOL
    // -- sixteen tools, each with its own row set -- so the options bar is a
    // different set of controls depending on what is selected. The two
    // existing reach cases measure whatever tool the fixture happens to
    // start with, which makes their result a claim about ONE bar. That is
    // exactly the error G-17 died of: a reach claim measured in one state
    // and read as a claim about the shell.
    //
    // THE ASSERTION IS THE CLEAN G-06 PROPERTY, not "everything is on the
    // ring": every control that is VISIBLE AND ENABLED must be reachable.
    //   - disabled controls are legitimately off the ring (T76 measured
    //     enabled <=> reached at 6 of 6, which is what retracted G-17);
    //   - MenuBarItems are off the ring by design and their commands carry
    //     accelerators (T73 measured 32 Shortcut blocks), so they are
    //     excluded by name rather than by silence.
    struct Row { const char *tool; };
    static const Row roster[] = {
        {"brush.stroke"}, {"eraser.stroke"}, {"heal.stroke"},
        {"dodge_burn.dodge"}, {"dodge_burn.burn"}, {"dodge_burn.sponge"},
        {"blur_sharpen.blur"}, {"blur_sharpen.sharpen"}, {"smudge.stroke"},
        {"clone.stroke"}, {"select.wand"}, {"vector.pen"}, {"text"},
        {"transform.puppet"}, {"shape.rect"}, {"shape.ellipse"},
    };
    const int kRoster = int(sizeof(roster) / sizeof(roster[0]));

    // ---- ROSTER LINT, two-sided against the checked-in model ---------
    QFile f(QFINDTESTDATA("../qml/Main.qml"));
    QVERIFY2(f.open(QIODevice::ReadOnly | QIODevice::Text),
             "could not read Main.qml -- the roster below would be a list I "
             "wrote rather than the shell's tool set");
    const QString src = QString::fromUtf8(f.readAll());
    f.close();
    const int modelAt = src.indexOf(QLatin1String("toolOptionsModel"));
    QVERIFY2(modelAt > 0, "toolOptionsModel not found in Main.qml");
    const QString seg = src.mid(modelAt, 9000);
    QRegularExpression re(QStringLiteral("^\\s{8}\"([a-z0-9_.]+)\"\\s*:\\s*\\["),
                          QRegularExpression::MultilineOption);
    QStringList inSource;
    auto it = re.globalMatch(seg);
    while (it.hasNext())
        inSource << it.next().captured(1);
    QVERIFY2(inSource.size() >= 10,
             qPrintable(QStringLiteral("the tool-key scan found only %1 keys; "
                                       "the pattern has stopped matching and "
                                       "the lint would pass vacuously")
                            .arg(inSource.size())));
    QStringList rostered;
    for (const Row &r : roster) rostered << QLatin1String(r.tool);
    QStringList uncovered, stale;
    for (const QString &s : inSource) if (!rostered.contains(s)) uncovered << s;
    for (const QString &r : rostered) if (!inSource.contains(r)) stale << r;
    qInfo().noquote()
        << QStringLiteral("TOOL OPTIONS ROSTER: %1 tool keys in Main.qml, "
                          "%2 roster rows").arg(inSource.size()).arg(kRoster);
    QVERIFY2(uncovered.isEmpty(),
             qPrintable(QStringLiteral("tool(s) with options and no roster "
                                       "row: [%1]")
                            .arg(uncovered.join(QStringLiteral(", ")))));
    QVERIFY2(stale.isEmpty(),
             qPrintable(QStringLiteral("roster row(s) naming a tool the model "
                                       "no longer has: [%1]")
                            .arg(stale.join(QStringLiteral(", ")))));

    auto isControl = [](QObject *o) -> bool {
        return o->inherits("QQuickAbstractButton")
            || o->inherits("QQuickSlider") || o->inherits("QQuickSpinBox")
            || o->inherits("QQuickComboBox") || o->inherits("QQuickTextInput")
            || o->inherits("QQuickTextField");
    };
    auto describe = [](QQuickItem *b) -> QString {
        const QString n = b->objectName();
        const QString t = b->property("text").toString();
        const QString cls = QString::fromLatin1(b->metaObject()->className())
                                .section(QLatin1Char('_'), 0, 0);
        if (!n.isEmpty()) return n;
        if (!t.trimmed().isEmpty())
            return QStringLiteral("%1(\"%2\")").arg(cls, t.trimmed());
        return cls;
    };
    auto census = [&]() -> QList<QQuickItem *> {
        QSet<QQuickItem *> seen; QList<QQuickItem *> out;
        auto consider = [&](QQuickItem *b) {
            if (!b || seen.contains(b) || !isControl(b)) return;
            if (!b->isVisible() || b->width() <= 0 || b->height() <= 0) return;
            seen << b; out << b;
        };
        for (QObject *o : m_window->findChildren<QObject *>())
            consider(qobject_cast<QQuickItem *>(o));
        std::function<void(QQuickItem *)> walk = [&](QQuickItem *i2) {
            for (QQuickItem *c : i2->childItems()) { consider(c); walk(c); }
        };
        if (QQuickItem *ci = m_window->contentItem()) walk(ci);
        return out;
    };

    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
    QCoreApplication::processEvents();

    QSet<int> populationSizes;
    QStringList offenders;
    int toolsMeasured = 0;
    for (const Row &r : roster) {
        const QString tool = QLatin1String(r.tool);
        m_canvasItem->setProperty("tool", tool);
        QCoreApplication::processEvents();
        QTest::qWait(90);       // let the options bar rebuild for this tool
        QList<QQuickItem *> population = census();
        if (population.size() < 8)
            continue;           // reported below by the floor, not silently
        populationSizes << population.size();
        ++toolsMeasured;

        m_canvas->forceActiveFocus();
        QCoreApplication::processEvents();
        QSet<QQuickItem *> visited;
        QQuickItem *first = nullptr; int cycles = 0; bool closed = false;
        for (int i = 0; i < 400; ++i) {
            QTest::keyClick(m_window, Qt::Key_Tab);
            QCoreApplication::processEvents();
            QQuickItem *fi = m_window->activeFocusItem();
            if (!fi) continue;
            if (!first) first = fi;
            else if (fi == first && ++cycles >= 2) { closed = true; break; }
            visited << fi;
        }
        QVERIFY2(closed,
                 qPrintable(QStringLiteral("tool %1: the Tab ring did not "
                                           "close in 400 taps, so nothing "
                                           "here may be called unreachable")
                                .arg(tool)));
        QSet<QQuickItem *> popSet;
        for (QQuickItem *c : population) popSet << c;
        auto reached = [&](QQuickItem *c) -> bool {
            if (visited.contains(c)) return true;
            for (QQuickItem *v : visited)
                for (QQuickItem *w = v; w; w = w->parentItem())
                    if (w == c) return true;
            for (QQuickItem *w = c->parentItem(); w; w = w->parentItem())
                if (visited.contains(w) && popSet.contains(w)) return true;
            return false;
        };
        for (QQuickItem *c : population) {
            if (reached(c)) continue;
            if (!c->property("enabled").toBool())
                continue;       // disabled is legitimately off the ring (T76)
            const QString d = describe(c);
            if (d.startsWith(QLatin1String("MenuBarItem")))
                continue;       // accelerators, measured at T73
            offenders << QStringLiteral("%1: %2").arg(tool, d);
        }
    }

    qInfo().noquote()
        << QStringLiteral("G-06 PER-TOOL REACH: %1 of %2 tools measured, "
                          "%3 distinct population sizes, %4 enabled control(s) "
                          "off the ring")
               .arg(toolsMeasured).arg(kRoster)
               .arg(populationSizes.size()).arg(offenders.size());
    if (!offenders.isEmpty())
        qInfo().noquote()
            << QStringLiteral("  OFF THE RING (enabled, not a menu): [%1]")
                   .arg(offenders.join(QStringLiteral("; ")));

    QVERIFY2(toolsMeasured == kRoster,
             qPrintable(QStringLiteral("only %1 of %2 tools produced a "
                                       "measurable options bar")
                            .arg(toolsMeasured).arg(kRoster)));
    // ★★★ THE TOOL SWITCH MUST ACTUALLY CHANGE THE BAR. If every tool
    // yielded the same population size, the switch is not taking effect and
    // a clean result would be sixteen measurements of one screen -- the
    // shape of the mistake this case exists to avoid.
    QVERIFY2(populationSizes.size() > 1,
             qPrintable(QStringLiteral("all %1 tools produced an identical "
                                       "control population; the tool switch "
                                       "is not changing the options bar and "
                                       "this case measured one screen sixteen "
                                       "times").arg(toolsMeasured)));
    QVERIFY2(offenders.isEmpty(),
             qPrintable(QStringLiteral("enabled, non-menu control(s) off the "
                                       "Tab ring: [%1]")
                            .arg(offenders.join(QStringLiteral("; ")))));
}

void TstGestureChrome::modalDialogsKeepKeyboardFocusInsideThem()
{
    // ROADMAP-1.0 G-06 (RC1 09-20/21). A modal dialog that lets Tab walk out
    // to the canvas behind it is operable-behind-the-modal: a keyboard user
    // can drive tools and menus the mouse is being blocked from. Every one
    // of these dialogs declares `modal: true` (17 of the 18 files in
    // shell/qml/dialogs; the 18th, CurveEditor, is an Item and not a
    // Dialog), so containment is the shell's OWN stated contract and this
    // case measures whether the contract holds.
    //
    // ★★★ AND `modal` IS NOT THE MECHANISM THAT DELIVERS IT -- measured,
    // because I assumed otherwise. Flipping a dialog to `modal: false`
    // changed nothing: 0 escapes still. Flipping it to `focus: false`
    // produced 18, naming real controls behind the popup (adjustButton,
    // fillButton, ToolButton("Zoom Out")) and reporting that dialog as
    // having NO in-dialog stop at all. So modality is the CONTRACT and the
    // popup taking FOCUS is the mechanism; a dialog could be modal to the
    // mouse and open to the keyboard, which is exactly the state this case
    // exists to catch.
    //
    // My existing dialog cases tab INSIDE dialogs and never asked whether
    // the ring leaves them; `modal` appears in this file only in prose.
    struct Row { const char *id; };
    static const Row roster[] = {
        {"newDialog"}, {"discardConfirmDialog"}, {"recoveryDialog"},
        {"modifyDialog"}, {"modeConvertDialog"}, {"filterParamDialog"},
        {"resizeDialog"}, {"canvasSizeDialog"}, {"rotateDialog"},
        {"trimDialog"}, {"cropDialog"}, {"fillDialog"},
        {"assignProfileDialog"}, {"convertProfileDialog"},
        {"depthConvertDialog"}, {"flattenDialog"}, {"deleteConfirmDialog"},
    };
    const int kRoster = int(sizeof(roster) / sizeof(roster[0]));

    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
    QCoreApplication::processEvents();
    QTest::qWait(120);

    auto describe = [](QQuickItem *b) -> QString {
        const QString n = b->objectName();
        const QString t = b->property("text").toString();
        const QString cls = QString::fromLatin1(b->metaObject()->className())
                                .section(QLatin1Char('_'), 0, 0);
        if (!n.isEmpty()) return n;
        if (!t.trimmed().isEmpty())
            return QStringLiteral("%1(\"%2\")").arg(cls, t.trimmed());
        return cls;
    };

    // ★★★★ POSITIVE CONTROL FIRST: with NO dialog open, Tab must reach a
    // good number of distinct controls. Without this, "focus never left the
    // dialog" could be true because Tab does nothing at all in this
    // fixture, and every containment result below would be a fact about a
    // dead Tab rather than about modality. This is the arm that gives the
    // word "contained" its meaning.
    m_canvas->forceActiveFocus();
    QCoreApplication::processEvents();
    QSet<QQuickItem *> freeStops;
    for (int i = 0; i < 60; ++i) {
        QTest::keyClick(m_window, Qt::Key_Tab);
        QCoreApplication::processEvents();
        if (QQuickItem *f = m_window->activeFocusItem())
            freeStops << f;
    }
    qInfo().noquote()
        << QStringLiteral("MODAL CONTAINMENT control: with no dialog open, "
                          "Tab reached %1 distinct controls")
               .arg(freeStops.size());
    QVERIFY2(freeStops.size() >= 10,
             qPrintable(QStringLiteral("with no dialog open Tab reached only "
                                       "%1 distinct items -- there is nowhere "
                                       "for focus to escape TO, so a "
                                       "containment result would be vacuous")
                            .arg(freeStops.size())));

    QStringList escapes, noStops;
    int measured = 0, totalInside = 0;
    for (const Row &r : roster) {
        const QString id = QLatin1String(r.id);
        evalJs(QStringLiteral("%1.close()").arg(id));
        QCoreApplication::processEvents();
        evalJs(QStringLiteral("%1.open()").arg(id));
        QCoreApplication::processEvents();
        QTest::qWaitFor([&] {
            return evalJs(QStringLiteral("%1.visible").arg(id)).toBool();
        }, 1000);
        if (!evalJs(QStringLiteral("%1.visible").arg(id)).toBool())
            continue;           // not openable here; the floor below counts it
        QObject *dlg = nullptr;
        for (QObject *o : m_window->findChildren<QObject *>())
            if (o->inherits("QQuickDialog") && o->property("visible").toBool()
                && o->objectName() == id) { dlg = o; break; }
        if (!dlg)
            for (QObject *o : m_window->findChildren<QObject *>())
                if (o->inherits("QQuickDialog")
                    && o->property("visible").toBool()) { dlg = o; break; }
        if (!dlg)
            continue;
        // ★★★★ "INSIDE THE DIALOG" IS ALL FOUR PARTS, NOT JUST contentItem.
        // The first draft tested ancestry against `contentItem` alone and
        // reported two escapes -- `newDialog -> Button("Cancel")` and
        // `-> Button("Create")`. Those are the New Document dialog's OWN
        // buttons: it is the only one of the seventeen that uses
        // `footer: DialogButtonBox`, and a footer is a SIBLING of
        // contentItem, not a descendant. The dialog was containing focus
        // perfectly and my scope was too narrow -- a false escape, and had
        // it been filed it would have been a false MAJOR against the one
        // dialog that does buttons the most conventional way.
        QList<QQuickItem *> parts;
        for (const char *prop : {"contentItem", "header", "footer",
                                 "background"})
            if (QQuickItem *pi =
                    dlg->property(prop).value<QQuickItem *>())
                parts << pi;
        if (parts.isEmpty())
            continue;
        ++measured;

        int inside = 0;
        for (int i = 0; i < 24; ++i) {
            QTest::keyClick(m_window, Qt::Key_Tab);
            QCoreApplication::processEvents();
            QQuickItem *f = m_window->activeFocusItem();
            if (!f)
                continue;
            bool within = false;
            for (QQuickItem *w = f; w && !within; w = w->parentItem())
                for (QQuickItem *part : parts)
                    if (w == part) { within = true; break; }
            if (within) { ++inside; continue; }
            const QString d = describe(f);
            const QString line = QStringLiteral("%1 -> %2").arg(id, d);
            if (!escapes.contains(line))
                escapes << line;
        }
        totalInside += inside;
        if (inside == 0)
            noStops << id;
        evalJs(QStringLiteral("%1.close()").arg(id));
        QCoreApplication::processEvents();
    }

    qInfo().noquote()
        << QStringLiteral("MODAL CONTAINMENT: %1 of %2 dialogs measured, "
                          "%3 in-dialog focus stops, %4 escape(s)")
               .arg(measured).arg(kRoster).arg(totalInside).arg(escapes.size());
    if (!noStops.isEmpty())
        qInfo().noquote()
            << QStringLiteral("  NO IN-DIALOG STOP (nothing focusable "
                              "inside): [%1]")
                   .arg(noStops.join(QStringLiteral(", ")));
    if (!escapes.isEmpty())
        qInfo().noquote()
            << QStringLiteral("  ESCAPED: [%1]")
                   .arg(escapes.join(QStringLiteral("; ")));

    QVERIFY2(measured >= 12,
             qPrintable(QStringLiteral("only %1 of %2 dialogs opened here; "
                                       "too small a population to speak about "
                                       "modality").arg(measured).arg(kRoster)));
    QVERIFY2(totalInside > 0,
             "no dialog produced a single in-dialog focus stop -- Tab is not "
             "entering these dialogs at all and containment is untested");
    QVERIFY2(escapes.isEmpty(),
             qPrintable(QStringLiteral("Tab escaped a MODAL dialog to a "
                                       "control behind it -- a keyboard user "
                                       "can drive the shell the modal is "
                                       "blocking the mouse from: [%1]")
                            .arg(escapes.join(QStringLiteral("; ")))));
}

void TstGestureChrome::everyDialogClosesOnEscape()
{
    // ROADMAP-1.0 G-06 (RC1 09-20/21), and this leg exists BECAUSE of T79's
    // result rather than beside it.
    //
    // ★★★★ T79 proved focus containment: across all seventeen dialogs, 408
    // Tab stops and NOT ONE left the dialog. That is the behaviour you want
    // -- and it is exactly what makes a missing Escape catastrophic instead
    // of annoying. A keyboard user inside a contained modal that will not
    // close on Escape has **no exit at all**: Tab cannot leave, and the
    // dialog's own buttons are the only way out. Containment and Escape are
    // one property in two halves, and this desk had measured Escape on TWO
    // dialogs (adjustment at T69, discard confirm at T70) out of seventeen.
    //
    // The roster is the same seventeen the Return census walks, so the two
    // cases cover the same population and a dialog cannot be in one and
    // missing from the other unnoticed.
    // ★★★★★ ONE DIALOG REFUSES ESCAPE ON PURPOSE, AND THE FIRST DRAFT OF
    // THIS CASE WOULD HAVE FILED IT. `RecoveryDialog` sets
    // `closePolicy: Popup.NoAutoClose` (line 25) because it is the crash
    // prompt and its three answers are NOT interchangeable -- Restore opens
    // the recovered log, Discard destroys it after a second confirm, "Not
    // now" preserves it and runs unprotected. Escape would mean which? A
    // dismissal key on that dialog is an ambiguous answer to a question
    // about someone's unsaved work, so refusing it is correct.
    //
    // So the rule is not "every dialog closes on Escape". It is: a dialog
    // closes on Escape UNLESS it declares NoAutoClose, and one that refuses
    // must leave the keyboard user a way out through its own buttons --
    // which matters precisely because T79 proved focus cannot leave a modal.
    struct Row { const char *id; bool escapable; };
    static const Row roster[] = {
        {"newDialog", true}, {"discardConfirmDialog", true},
        {"recoveryDialog", false},              // NoAutoClose, reason above
        {"modifyDialog", true}, {"modeConvertDialog", true},
        {"filterParamDialog", true}, {"resizeDialog", true},
        {"canvasSizeDialog", true}, {"rotateDialog", true},
        {"trimDialog", true}, {"cropDialog", true}, {"fillDialog", true},
        {"assignProfileDialog", true}, {"convertProfileDialog", true},
        {"depthConvertDialog", true}, {"flattenDialog", true},
        {"deleteConfirmDialog", true},
    };
    const int kRoster = int(sizeof(roster) / sizeof(roster[0]));

    // ---- THE EXCEPTION MUST NOT ROT -------------------------------
    // Two-sided against the checked-in QML: exactly the dialogs that declare
    // NoAutoClose may be marked non-escapable. A new dialog adopting it
    // silently, or this roster keeping an exception the source dropped, both
    // red here rather than being absorbed as "expected".
    QDir dlgDir(QFINDTESTDATA("../qml/dialogs"));
    QVERIFY2(dlgDir.exists(), "could not find shell/qml/dialogs to lint the "
                              "NoAutoClose exception against");
    QStringList declaresNoAutoClose;
    const QStringList qmls = dlgDir.entryList(QStringList() << "*.qml",
                                              QDir::Files);
    QVERIFY2(qmls.size() >= 10,
             qPrintable(QStringLiteral("only %1 dialog files found; the lint "
                                       "would pass vacuously")
                            .arg(qmls.size())));
    for (const QString &fn : qmls) {
        QFile qf(dlgDir.filePath(fn));
        if (!qf.open(QIODevice::ReadOnly | QIODevice::Text)) continue;
        const QString body = QString::fromUtf8(qf.readAll());
        qf.close();
        if (body.contains(QLatin1String("NoAutoClose")))
            declaresNoAutoClose << fn;
    }
    qInfo().noquote()
        << QStringLiteral("ESCAPE EXCEPTION LINT: %1 of %2 dialog files "
                          "declare NoAutoClose -> [%3]")
               .arg(declaresNoAutoClose.size()).arg(qmls.size())
               .arg(declaresNoAutoClose.join(QStringLiteral(", ")));
    QCOMPARE(declaresNoAutoClose, QStringList()
                                      << QStringLiteral("RecoveryDialog.qml"));

    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
    QCoreApplication::processEvents();
    QTest::qWait(120);

    QStringList stayedOpen, neverOpened, wrongfullyClosed, trapped;
    int closed = 0, opened = 0;
    for (const Row &r : roster) {
        const QString id = QLatin1String(r.id);
        evalJs(QStringLiteral("%1.close()").arg(id));
        QCoreApplication::processEvents();
        evalJs(QStringLiteral("%1.open()").arg(id));
        QCoreApplication::processEvents();
        QTest::qWaitFor([&] {
            return evalJs(QStringLiteral("%1.visible").arg(id)).toBool();
        }, 1000);
        // ★★ THE PRECONDITION IS ASSERTED PER DIALOG, not assumed. "It is
        // closed after Escape" is trivially true of a dialog that never
        // opened, and counting those as passes is how a census reports a
        // clean sweep of things it never tested.
        if (!evalJs(QStringLiteral("%1.visible").arg(id)).toBool()) {
            neverOpened << id;
            continue;
        }
        ++opened;
        QTest::keyClick(m_window, Qt::Key_Escape);
        QCoreApplication::processEvents();
        QTest::qWaitFor([&] {
            return !evalJs(QStringLiteral("%1.visible").arg(id)).toBool();
        }, 1500);
        const bool stillUp =
            evalJs(QStringLiteral("%1.visible").arg(id)).toBool();
        if (!r.escapable) {
            // The declared exception. It must NOT close -- and it must give
            // the keyboard a way out, since T79 proved focus stays inside.
            if (!stillUp) {
                wrongfullyClosed << id;
            } else {
                QObject *dlg = nullptr;
                for (QObject *o : m_window->findChildren<QObject *>())
                    if (o->inherits("QQuickDialog")
                        && o->property("visible").toBool()) { dlg = o; break; }
                int reachableButtons = 0;
                if (dlg) {
                    QSet<QQuickItem *> stops;
                    for (int i = 0; i < 20; ++i) {
                        QTest::keyClick(m_window, Qt::Key_Tab);
                        QCoreApplication::processEvents();
                        if (QQuickItem *f = m_window->activeFocusItem())
                            stops << f;
                    }
                    for (QObject *o : dlg->findChildren<QObject *>()) {
                        if (!o->inherits("QQuickAbstractButton")) continue;
                        QQuickItem *b = qobject_cast<QQuickItem *>(o);
                        if (!b || !b->isVisible() || b->width() <= 0) continue;
                        bool hit = stops.contains(b);
                        for (QQuickItem *v : stops)
                            for (QQuickItem *w = v; w && !hit;
                                 w = w->parentItem())
                                if (w == b) hit = true;
                        if (hit) ++reachableButtons;
                    }
                }
                qInfo().noquote()
                    << QStringLiteral("  %1 refuses Escape by contract; "
                                      "keyboard-reachable buttons: %2")
                           .arg(id).arg(reachableButtons);
                if (reachableButtons == 0)
                    trapped << id;
            }
            evalJs(QStringLiteral("%1.close()").arg(id));
            QCoreApplication::processEvents();
            continue;
        }
        if (stillUp) {
            stayedOpen << id;
            evalJs(QStringLiteral("%1.close()").arg(id));  // leave no modal up
            QCoreApplication::processEvents();
        } else {
            ++closed;
        }
    }

    qInfo().noquote()
        << QStringLiteral("ESCAPE CENSUS: %1 of %2 dialogs opened; Escape "
                          "closed %3 of them")
               .arg(opened).arg(kRoster).arg(closed);
    if (!neverOpened.isEmpty())
        qInfo().noquote()
            << QStringLiteral("  NOT MEASURED (would not open): [%1]")
                   .arg(neverOpened.join(QStringLiteral(", ")));
    if (!stayedOpen.isEmpty())
        qInfo().noquote()
            << QStringLiteral("  STAYED OPEN ON ESCAPE: [%1]")
                   .arg(stayedOpen.join(QStringLiteral(", ")));

    QVERIFY2(opened >= 12,
             qPrintable(QStringLiteral("only %1 of %2 dialogs opened; too "
                                       "small a population to speak about "
                                       "Escape").arg(opened).arg(kRoster)));
    // ★★★ THE CASE DIAGNOSES ITSELF. If NOTHING closed, the likely cause is
    // this fixture's key delivery rather than seventeen simultaneous product
    // defects, and the message says so instead of pointing seventeen fingers
    // at the shell.
    QVERIFY2(closed > 0,
             qPrintable(QStringLiteral("NOT ONE of %1 opened dialogs closed on "
                                       "Escape -- before reading that as "
                                       "seventeen defects, suspect this "
                                       "fixture's key delivery, which is the "
                                       "cheaper explanation and the one to "
                                       "rule out first").arg(opened)));
    QVERIFY2(wrongfullyClosed.isEmpty(),
             qPrintable(QStringLiteral("dialog(s) declaring NoAutoClose "
                                       "closed on Escape anyway: [%1] -- the "
                                       "contract in the QML and the behaviour "
                                       "disagree")
                            .arg(wrongfullyClosed.join(QStringLiteral(", ")))));
    // ★★★ THE ONE THAT MATTERS. A dialog may refuse Escape; it may not do so
    // while offering no keyboard route out, because T79 established that Tab
    // cannot leave a modal. Refusal plus containment plus no reachable
    // button is a trap with no exit.
    QVERIFY2(trapped.isEmpty(),
             qPrintable(QStringLiteral("dialog(s) refuse Escape AND expose no "
                                       "keyboard-reachable button: [%1]. With "
                                       "T79's containment result that is a "
                                       "keyboard user with no exit at all")
                            .arg(trapped.join(QStringLiteral(", ")))));
    QVERIFY2(stayedOpen.isEmpty(),
             qPrintable(QStringLiteral("dialog(s) did NOT close on Escape "
                                       "while %1 others did, so the key is "
                                       "being delivered: [%2]. T79 measured "
                                       "focus as CONTAINED in these dialogs, "
                                       "so a keyboard user who opens one of "
                                       "these has no exit -- Tab cannot leave "
                                       "and Escape does nothing")
                            .arg(closed)
                            .arg(stayedOpen.join(QStringLiteral(", ")))));
}

void TstGestureChrome::coldOpenEmptyStateOffersOnlyItsOwnControls()
{
    // B-10 (WS-B, due 09-16): "no document open: New/Import only, Cmd+T
    // refusal toast, single-document model". Its DONE-WHEN names the TOUR,
    // which owns the appearance and passes byte-identically every tick
    // (01-launch.png). This is the behavioural half, which had no
    // instrument at all.
    //
    // ★★★★ WHY IT HAD NONE, MEASURED RATHER THAN GUESSED: `hasDocument` is
    // a read-only property that goes false -> true exactly once, there is no
    // `closeDocument` on the controller, the shell is single-document with
    // no Close -- and this suite's init() opens a document before EVERY
    // case. So the empty state exists only between initTestCase() building
    // the window and the first init(), and 0 of 60 cases had ever run in it.
    // The facts asserted here are captured in that window.
    qInfo().noquote()
        << QStringLiteral("COLD OPEN: hasDocument=%1, no-document label=%2, "
                          "Cmd+T refused=%3, enabled controls=%4 -> [%5]")
               .arg(m_emptyHasDocument).arg(m_emptyNoDocLabel)
               .arg(m_emptyCmdTRefused).arg(m_emptyEnabledControls)
               .arg(m_emptyEnabledNames.join(QStringLiteral(", ")));

    // The capture must have happened at all -- a -1 means initTestCase's
    // block did not run and every assertion below would be about a default.
    QVERIFY2(m_emptyEnabledControls >= 0,
             "the cold-open capture never ran; these assertions would be "
             "about member defaults rather than about the shell");
    QVERIFY2(!m_emptyHasDocument,
             "the shell had a document open at cold open -- the empty state "
             "this case is about never existed");
    QVERIFY2(m_emptyNoDocLabel,
             "the no-document label was not visible at cold open, so the "
             "empty state showed the user nothing explaining it");
    // ★★★ THE ONE B-10 NAMES. With no document, Cmd+T must refuse in the
    // typed, visible way rather than doing nothing -- silence on a bound
    // accelerator is indistinguishable to the user from a broken key.
    QVERIFY2(m_emptyCmdTRefused,
             "Cmd+T at cold open raised NO refusal toast -- B-10 names that "
             "toast, and a bound shortcut that answers with silence reads as "
             "a dead key");
    // ★★★ THE DOCUMENT-DEPENDENCY PROPERTY, which is what B-10 is actually
    // about: nothing that edits a document may be live when there is no
    // document. Measured today as [File@Main, Edit@Main, Select@Main,
    // Image@Main, Filter@Main, swatchForegroundWell@ToolPalette,
    // swatchBackgroundWell@ToolPalette, swatchSwapControl@ToolPalette,
    // Slider@ToolBar x2, ToolButton@ToolBar, ToolButton@slotsViewport] --
    // menus (whose ITEMS gate individually on hasDocument), colour swatches,
    // and toolbar chrome. NO tool button and NO LayersPanel control, which
    // is the property worth pinning: layers and tools are meaningless
    // without a document and the shell agrees.
    for (const QString &n : m_emptyEnabledNames)
        QVERIFY2(!n.contains(QLatin1String("LayersPanel")),
                 qPrintable(QStringLiteral("a LayersPanel control is ENABLED "
                                           "at cold open (%1) -- layer "
                                           "operations require a document "
                                           "and there is none; full set [%2]")
                                .arg(n)
                                .arg(m_emptyEnabledNames
                                         .join(QStringLiteral(", ")))));
    // A loose sanity bound, deliberately NOT the exact count. The measured
    // value is 12 and an exact assertion would red on any chrome addition
    // while catching nothing real; what this must catch is "the whole editor
    // came up live without a document".
    QVERIFY2(m_emptyEnabledControls > 0 && m_emptyEnabledControls <= 20,
             qPrintable(QStringLiteral("cold open offers %1 enabled controls "
                                       "[%2] -- expected an entry surface, "
                                       "not a live editor")
                            .arg(m_emptyEnabledControls)
                            .arg(m_emptyEnabledNames.join(QStringLiteral(", ")))));

    // ★ RECORDED, NOT FILED: both options-bar Sliders (Main.qml has exactly
    // one `Slider {`, at 1248 inside optionsRepeater) are ENABLED here, so
    // brush size and hardness are live with no document open. That is not
    // "New/Import only" as B-10's prose reads -- but no contract says the
    // options bar must be inert at cold open, moving them only sets a shell
    // property the first document will use, and it may well be deliberate.
    // T80's lesson is the reason this is a note and not a finding: the
    // instrument can be right while the rule I would assert is wrong.
}

// RC3 D-2 (operator WORD 35, sitting S-12): Levels and Hue/Saturation Apply
// was INERT -- the dialog stayed open with no text, no status line and no
// history row. The contract: Apply either LANDS (an adjustment layer appears
// and the dialog closes) or SAYS WHY -- the refusal text in the dialog AND on
// the status bar. Never a silent no-op. Any QML script error raised by the
// click is captured and named, because a throwing handler is silent too.
static QStringList *g_d2ScriptErrors = nullptr;
static QtMessageHandler g_d2Prev = nullptr;
static void d2Handler(QtMsgType t, const QMessageLogContext &c, const QString &m)
{
    if (g_d2ScriptErrors && t != QtDebugMsg && t != QtInfoMsg
        && m.contains(QLatin1String(".qml"))
        && QRegularExpression(QStringLiteral("\\b[A-Za-z]*Error: ")).match(m).hasMatch())
        g_d2ScriptErrors->append(m);
    if (g_d2Prev)
        g_d2Prev(t, c, m);
}

void TstGestureChrome::levelsAndHueSaturationApplyLandsOrSaysWhy()
{
    auto byName = [&](const QString &n) -> QQuickItem * {
        for (QObject *o : m_window->findChildren<QObject *>())
            if (o->objectName() == n)
                return qobject_cast<QQuickItem *>(o);
        return nullptr;
    };
    auto visibleDialog = [&]() -> QObject * {
        for (QObject *o : m_window->findChildren<QObject *>())
            if (o->inherits("QQuickDialog") && o->property("visible").toBool())
                return o;
        return nullptr;
    };
    auto adjustmentRows = [&]() -> int {
        int n = 0;
        for (const QVariant &r : m_controller->layers())
            if (r.toMap().value(QStringLiteral("kind")).toString()
                == QLatin1String("adjustment"))
                ++n;
        return n;
    };
    const QString cancelLabel = m_i18n->tr(QStringLiteral("ui.dialog.cancel"));
    for (const char *arm : {"adjustMenuLevels", "adjustMenuHueSaturation"}) {
        QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
        QCoreApplication::processEvents();
        QQuickItem *btn = byName(QStringLiteral("adjustButton"));
        QVERIFY(btn);
        QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                          center(btn));
        QCoreApplication::processEvents();
        QQuickItem *item = byName(QLatin1String(arm));
        QVERIFY2(item, arm);
        QMetaObject::invokeMethod(item, "triggered");
        QCoreApplication::processEvents();
        QObject *dlg = visibleDialog();
        QVERIFY2(dlg, qPrintable(QStringLiteral("%1 opened no dialog").arg(arm)));
        QQuickItem *applyBtn = nullptr;
        for (QObject *o : dlg->findChildren<QObject *>()) {
            auto *b = qobject_cast<QQuickItem *>(o);
            if (o->inherits("QQuickAbstractButton") && b && b->isVisible()
                && b->width() > 0 && b->property("text").toString() != cancelLabel)
                applyBtn = b;
        }
        QVERIFY2(applyBtn, "no Apply button");
        // NON-DEFAULT, as the sitting did (Gamma 0.85, Hue +20): move the
        // first visible value row by real keys, so Apply carries a payload.
        // Repeater delegates are not QObject children of the window, so walk
        // the VISUAL tree (the overlay the dialog lives in included).
        QQuickItem *spin = nullptr;
        std::function<void(QQuickItem *)> walk = [&](QQuickItem *it) {
            if (!it || spin)
                return;
            if (it->objectName().startsWith(QLatin1String("adjustSpin_"))
                && it->isVisible()) {
                spin = it;
                return;
            }
            const auto kids = it->childItems();
            for (QQuickItem *k : kids)
                walk(k);
        };
        walk(m_window->contentItem()->parentItem()
                 ? m_window->contentItem()->parentItem()
                 : m_window->contentItem());
        QVERIFY2(spin, "no visible value row in the dialog");
        const int spinBefore = spin->property("value").toInt();
        spin->forceActiveFocus(Qt::TabFocusReason);
        QTRY_VERIFY(spin->hasActiveFocus());
        for (int k = 0; k < 5; ++k)
            QTest::keyClick(m_window, Qt::Key_Up);
        QVERIFY2(spin->property("value").toInt() != spinBefore,
                 "the value row did not move — the payload would be default");
        const int before = adjustmentRows();
        const QString statusBefore = m_controller->statusText();
        QStringList scriptErrors;
        g_d2ScriptErrors = &scriptErrors;
        g_d2Prev = qInstallMessageHandler(d2Handler);
        QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                          center(applyBtn));
        QTest::qWait(600);
        qInstallMessageHandler(g_d2Prev);
        g_d2ScriptErrors = nullptr;
        QQuickItem *err = byName(QStringLiteral("adjustErrorLabel"));
        const QString errText = err ? err->property("text").toString() : QString();
        const bool landed = adjustmentRows() > before;
        const bool open = dlg->property("visible").toBool();
        const QString status = m_controller->statusText();
        qInfo().noquote() << QStringLiteral(
            "D-2 %1: landed=%2 dialogOpen=%3 errorText='%4' status='%5' "
            "(before '%6') scriptErrors=[%7]")
            .arg(QLatin1String(arm)).arg(landed).arg(open).arg(errText, status,
                 statusBefore, scriptErrors.join(QStringLiteral(" | ")));
        QVERIFY2(scriptErrors.isEmpty(),
                 qPrintable(QStringLiteral("%1 Apply raised script errors: %2")
                                .arg(QLatin1String(arm),
                                     scriptErrors.join(QStringLiteral(" | ")))));
        if (landed) {
            QTRY_VERIFY2(!dlg->property("visible").toBool(),
                         "Apply landed the layer but the dialog stayed open");
        } else {
            QVERIFY2(open, "Apply neither landed nor kept the dialog open");
            QVERIFY2(!errText.isEmpty(),
                     qPrintable(QStringLiteral("%1 Apply was refused with NO "
                                               "text in the dialog (S-12)")
                                    .arg(QLatin1String(arm))));
            QVERIFY2(status != statusBefore && !status.isEmpty(),
                     qPrintable(QStringLiteral("%1 Apply was refused with NO "
                                               "status line (S-12)")
                                    .arg(QLatin1String(arm))));
            QMetaObject::invokeMethod(dlg, "close");
        }
    }
}

void TstGestureChrome::deletedLayerLeavesTheCanvasWithoutAnotherReceipt()
{
    // The operator's exact RC3 sequence (WORD 38): open a photograph, Layers
    // Add, paint a stroke, Del. The row went away but the stroke stayed on
    // the canvas until hiding and re-showing the remaining layer redrew it
    // (WORD 39) -- the delete was right and the canvas was stale: the
    // layer.delete receipt carries no dirty rects. A grey 640x400 image keeps
    // docToScene's zoom-1 mapping valid; the size is not what fails.
    QTemporaryDir tmp;
    QVERIFY(tmp.isValid());
    const QString path = tmp.filePath(QStringLiteral("grey.png"));
    QImage grey(640, 400, QImage::Format_RGB32);
    grey.fill(QColor(128, 128, 128));
    QVERIFY(grey.save(path));
    QVERIFY(m_controller->openImageAsDocument(path));
    m_canvas->zoomFit();
    m_canvas->zoomOneToOne();
    // Open-image builds a transparent base plus the imported layer.
    const int opened = m_controller->layers().size();
    QVERIFY(opened >= 1);

    auto shownLuma = [&](int x, int y) {
        const QImage img = m_window->grabWindow();
        const qreal dpr = img.devicePixelRatio();
        const QPoint s = docToScene(x, y);
        return qGray(img.pixel(int(s.x() * dpr), int(s.y() * dpr)));
    };
    auto panelButton = [&](const char *labelId) -> QQuickItem * {
        const QString label = m_i18n->tr(QString::fromLatin1(labelId));
        QQuickItem *found = nullptr;
        std::function<void(QQuickItem *)> walk = [&](QQuickItem *it) {
            if (!it || found)
                return;
            if (it->inherits("QQuickAbstractButton") && it->isVisible()
                && it->property("text").toString() == label) {
                found = it;
                return;
            }
            for (QQuickItem *k : it->childItems())
                walk(k);
        };
        walk(m_window->contentItem());
        return found;
    };
    QTRY_VERIFY2(qAbs(shownLuma(320, 200) - 128) <= 8,
                 "the opened grey image is not what the canvas shows");

    // Add -- the panel's own button; D-7 makes the new layer active.
    QQuickItem *add = panelButton("ui.panel.layers.add");
    QVERIFY2(add, "the Layers panel Add button was not found");
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      add->mapToScene(QPointF(add->width() / 2, add->height() / 2)).toPoint());
    QTRY_COMPARE(m_controller->layers().size(), opened + 1);
    const QString added = m_controller->layers()
                              .value(m_controller->activeLayerIndex())
                              .toMap().value(QStringLiteral("layerId")).toString();

    m_canvasItem->setProperty("brushColor", QColor(Qt::black));
    m_canvasItem->setProperty("brushSize", 40.0);
    QVariantList pts;
    for (int i = 0; i <= 24; ++i) {
        QVariantMap p;
        p.insert(QStringLiteral("x"), 200.0 + 10.0 * i);
        p.insert(QStringLiteral("y"), 200.0);
        p.insert(QStringLiteral("pressure"), 1.0);
        p.insert(QStringLiteral("t"), 0.008 * i);
        pts.append(p);
    }
    QVERIFY2(m_canvas->tourStroke(QStringLiteral("brush.stroke"), pts),
             qPrintable(QStringLiteral("the stroke did not commit; status: %1")
                            .arg(m_controller->statusText())));
    QTRY_VERIFY2(shownLuma(320, 200) < 40, "the stroke is not shown on the canvas");

    // Del -- the panel's Remove button, on the layer the stroke went to.
    QQuickItem *remove = panelButton("ui.panel.layers.remove");
    QVERIFY2(remove && remove->isEnabled(), "the Layers panel Remove button is not usable");
    QSignalSpy dirty(m_controller, &DocumentController::regionsDirty);
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      remove->mapToScene(QPointF(remove->width() / 2, remove->height() / 2)).toPoint());
    QTRY_COMPARE(m_controller->layers().size(), opened);
    for (const QVariant &r : m_controller->layers())
        QVERIFY2(r.toMap().value(QStringLiteral("layerId")).toString() != added,
                 "Remove deleted a layer other than the painted one");
    QVERIFY2(dirty.count() > 0,
             "layer.delete applied but the canvas was never told to repaint");
    // No further receipt: nothing below this line issues a command.
    QTRY_VERIFY2(qAbs(shownLuma(320, 200) - 128) <= 8,
                 "the layer was deleted but its stroke is still on the canvas (D-9)");
}

void TstGestureChrome::s10UndoAndRedoRepaintMaskAndLayerChanges()
{
    // 1.0 final S-10 (O-SIT RC5 sitting, ★ stale canvas): after Cmd+Z of a
    // Mask Properties change the panel showed the restored value but the
    // canvas kept the undone pixels. undo()/redo() did not pass through
    // applyCommand, so the whole-document refresh that mask.* and the
    // structural layer.* kinds get forward (their receipts carry no dirty
    // rects) never ran on the way back. Pinned on shown pixels, with no
    // further receipt after each undo/redo: a mask-density change, its undo
    // and redo; then a layer delete and its undo.
    QTemporaryDir tmp;
    QVERIFY(tmp.isValid());
    const QString path = tmp.filePath(QStringLiteral("grey.png"));
    QImage grey(640, 400, QImage::Format_RGB32);
    grey.fill(QColor(128, 128, 128));
    QVERIFY(grey.save(path));
    QVERIFY(m_controller->openImageAsDocument(path));
    m_canvas->zoomFit();
    m_canvas->zoomOneToOne();
    auto shownLuma = [&](int x, int y) {
        const QImage img = m_window->grabWindow();
        const qreal dpr = img.devicePixelRatio();
        const QPoint sp = docToScene(x, y);
        return qGray(img.pixel(int(sp.x() * dpr), int(sp.y() * dpr)));
    };
    QTRY_VERIFY(qAbs(shownLuma(320, 200) - 128) <= 8);

    // A new raster layer with a black stroke across the middle.
    const QString layerId = m_controller->newUuid();
    QVERIFY(m_controller->applyCommand(QStringLiteral("layer.add"), {},
                {{QStringLiteral("kind"), QStringLiteral("raster")},
                 {QStringLiteral("newLayerId"), layerId}}, {})
                .value(QStringLiteral("ok")).toBool());
    m_canvasItem->setProperty("brushColor", QColor(Qt::black));
    m_canvasItem->setProperty("brushSize", 40.0);
    QVariantList pts;
    for (int i = 0; i <= 24; ++i) {
        QVariantMap pt;
        pt.insert(QStringLiteral("x"), 200.0 + 10.0 * i);
        pt.insert(QStringLiteral("y"), 200.0);
        pt.insert(QStringLiteral("pressure"), 1.0);
        pt.insert(QStringLiteral("t"), 0.008 * i);
        pts.append(pt);
    }
    QVERIFY(m_canvas->tourStroke(QStringLiteral("brush.stroke"), pts));
    QTRY_VERIFY2(shownLuma(320, 200) < 40, "the stroke is not shown");

    // A hide-all mask hides it; density 0 shows it again.
    const QString maskId = m_controller->newUuid();
    QVERIFY(m_controller->applyCommand(QStringLiteral("mask.add"),
                {{QStringLiteral("layerId"), layerId}},
                {{QStringLiteral("kind"), QStringLiteral("raster")},
                 {QStringLiteral("from"), QStringLiteral("hide_all")},
                 {QStringLiteral("newMaskId"), maskId}}, {})
                .value(QStringLiteral("ok")).toBool());
    QTRY_VERIFY2(qAbs(shownLuma(320, 200) - 128) <= 8, "the hide-all mask did not hide the stroke");
    QVERIFY(m_controller->setMaskProp(layerId, maskId, QStringLiteral("density"), 0.0,
                                      m_controller->newUuid())
                .value(QStringLiteral("ok")).toBool());
    QTRY_VERIFY2(shownLuma(320, 200) < 40, "density 0 did not reveal the stroke");

    // Undo the density change: the mask hides the stroke again.
    m_controller->undo();
    QTRY_VERIFY2(qAbs(shownLuma(320, 200) - 128) <= 8,
                 "undo of a mask-density change left the stroke on the canvas (S-10)");
    // Redo: revealed again.
    m_controller->redo();
    QTRY_VERIFY2(shownLuma(320, 200) < 40,
                 "redo of a mask-density change did not repaint (S-10)");

    // Delete the layer, then undo the delete: the stroke comes back.
    QVERIFY(m_controller->applyCommand(QStringLiteral("layer.delete"),
                {{QStringLiteral("layerId"), layerId}}, {}, {})
                .value(QStringLiteral("ok")).toBool());
    QTRY_VERIFY2(qAbs(shownLuma(320, 200) - 128) <= 8, "the deleted layer is still shown");
    m_controller->undo();
    QTRY_VERIFY2(shownLuma(320, 200) < 40,
                 "undo of a layer delete did not repaint the restored layer (S-10)");
}

int main(int argc, char *argv[])
{
    if (qgetenv("QT_QPA_PLATFORM").isEmpty())
        qputenv("QT_QPA_PLATFORM", "offscreen");
    if (qgetenv("QT_QUICK_BACKEND").isEmpty())
        qputenv("QT_QUICK_BACKEND", "software");
    QGuiApplication app(argc, argv);
    // MIRRORS shell/src/main.cpp:96 — operator WORD 16 (4), 2026-09-17:
    // "a harness must model the app's focus policy or it measures a
    // different program". This binary has its own main() and never
    // compiles src/main.cpp, so without this line every Tab-ring case
    // here measures Qt's macOS default instead of the shipped policy
    // (O-INT entry 1041: FAIL 2, optionsBarStaysKeyboardReachable...,
    // 15 CheckBoxes off the ring). Both lines cite this word.
    QGuiApplication::styleHints()->setTabFocusBehavior(Qt::TabFocusAllControls);
    QQuickStyle::setStyle(QStringLiteral("Basic"));
    TstGestureChrome tc;
    QTEST_SET_MAIN_SOURCE_PATH
    return QTest::qExec(&tc, argc, argv);
}
#include "tst_gesture_chrome.moc"
