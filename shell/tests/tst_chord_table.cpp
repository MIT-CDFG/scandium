// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// v1.0.1 item 14b — THE CHORD TABLE NET (the items-7+14 design entry,
// reports/O-UI-log.md). Boots the REAL Main.qml offscreen (the item-7
// harness: this binary compiles its own copy of the AppnameShell QML
// module and wires the real controller/announcer like shell/src/
// main.cpp) against REAL engine documents and drives every NEW chord
// with REAL key events, asserting ENGINE truth (readback, journal,
// masks) — never shell echo state.
//
// KEY-LAYER slice additions: the ES:16452 single-key suppression at
// the dispatch layer (letters insert into a focused text field, tools
// hold still; modifier chords still dispatch) and the Appendix-C
// letter truth T = Type (transform keeps Ctrl/Cmd+T only).
//
// HARNESS NOTE (recorded deviation): the design entry names
// tst_selection_gestures' harness; that harness is a bare CanvasView in
// a raw QQuickWindow — the window-context Shortcuts, the menus, and the
// §32.5 confirm dialog do not exist there. This net uses its item-7
// descendant (the full-Main.qml offscreen harness) — same idiom (real
// synthesized input, real engine, engine-truth asserts), with the real
// chord surfaces present.
//
// RULE-8 derivations pinned in-case:
//  - fill constants: the engine's headless default swatches
//    (crates/appname-tools/src/kind_impls.rs — "foreground" = black
//    [0,0,0], "background" = white [1,1,1]); the R15 distinction (the
//    shell swatch canvas.brushColor is NOT consulted) is discriminated
//    by setting the swatch RED before the chord.
//  - Delete's §13.1 coverage scaling: a fractionally-covered pixel of
//    an opaque layer keeps alpha = 255 - coverage (the "30 % selected
//    pixel keeps 70 % alpha" rule) — expected value read from the
//    engine's OWN pre-delete coverage mask, ±1 rounding.
//  - chord display text: QKeySequence(...).toString(NativeText) — the
//    same conversion ChordMenuItem's converter Shortcut performs.

#include "announcer.h"
#include "canvas_view.h"
#include "document_controller.h"
#include "i18n_catalog.h"
#include "units.h"
#include "icon_provider.h"
#include "layer_thumb_provider.h"
#include "tool_key_dispatcher.h"

#include <QClipboard>
#include <QColor>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QGuiApplication>
#include <QKeySequence>
#include <QQmlApplicationEngine>
#include <QQmlComponent>
#include <QQmlContext>
#include <QRegularExpression>
#include <QSet>
#include <QQmlExpression>
#include <QQuickItem>
#include <QQuickStyle>
#include <QQuickWindow>
#include <QtTest>

#include <functional>

using namespace appname::shell;

class TstChordTable : public QObject {
    Q_OBJECT
private slots:
    void initTestCase();
    void cleanup();
    void catalogGround();
    void noDocumentChordsAreInert(); // MUST run before any freshDoc case
    void guardedChordsSayWhyInsteadOfSwallowing(); // also pre-freshDoc
    // B-10 cold open. BOTH are pre-freshDoc and MUST stay above
    // cmdTEntersTransformFrame, which opens with freshDoc().
    void coldOpenOffersExactlyTheDoorsThatCanMakeADocument();
    void fileNewIsALiveDoorAtColdOpenAndReachesADocument();
    void cmdTEntersTransformFrame();
    void refusalToastShowsMechanismFirstAndRefires();
    void deleteWithSelectionClearsCoverageScaled();
    void deleteNoSelectionConfirmCancelApply();
    void deleteTransparencyLockRefusesTyped();
    void deleteMidDragIsIgnored();
    void deleteConfirmDismissalIsConsumed();
    void fillChordsUseEngineConstants();
    void reselectMemoryArmsAndReplays();
    void cmdJViaCopyBothArms();
    void arrowNudgeIntegerArm();
    void arrowsYieldToFocusedTextField();
    void singleKeySuppressionInRenameField();
    void bareLettersCanvasScopeAndTType();
    // ---- v1.0.1 item 11e: the letter chords on a real dispatcher ----
    void guardHoldsOnANoYieldSurface();
    void chordsStillDispatchFromAFocusedSlider();
    void openPopupKeepsItsOwnLetters();
    void chordMidDragCannotKillTheGesture();
    void springLoadTapLatchesAndHoldReverts();
    void autoRepeatDoesNotStepTheRing();
    void sameToolReSelectStillReArms();
    void cloneChordSpeaksItsMissingSource();
    void chordAnnouncementsKeyOnInputSource();
    void swapColorsChordSwapsAndAnnouncesTheAction();
    void ruledChordsSurviveEveryThemeBindingSet();
    void cmdXCutChordCutsAndCopies();
    void chordDisplayRendersInMenuItems();

private:
    CatalogQml *m_i18n = nullptr;
    DocumentController *m_controller = nullptr;
    Announcer *m_announcer = nullptr;
    QQmlApplicationEngine *m_engine = nullptr;
    QQuickWindow *m_window = nullptr;
    QQuickItem *m_canvas = nullptr;
    ToolKeyDispatcher *m_toolKeys = nullptr;

    // The guarded-chord vocabulary, PARSED FROM Main.qml rather than
    // retyped here. A hardcoded copy is a second source of truth and
    // would drift exactly the way the survey that found twelve did.
    QStringList guardedChordActions() const
    {
        QFile f(QFileInfo(QStringLiteral(__FILE__)).dir().filePath(
            QStringLiteral("../qml/Main.qml")));
        if (!f.open(QIODevice::ReadOnly))
            return {};
        QStringList out;
        QRegularExpression re(QStringLiteral(
            "onActivated: if \\(root\\.rowEnabled\\(\"([^\"]+)\"\\)\\)"));
        QRegularExpressionMatchIterator it =
            re.globalMatch(QString::fromUtf8(f.readAll()));
        while (it.hasNext())
            out << it.next().captured(1);
        return out;
    }

    QString catTr(const char *key) const
    {
        return m_i18n->tr(QLatin1String(key));
    }
    QVariant evalJs(const QString &code) const
    {
        QQmlExpression expr(qmlContext(m_window), m_window, code);
        bool undefined = false;
        const QVariant v = expr.evaluate(&undefined);
        if (expr.hasError())
            qWarning() << "evalJs failed:" << expr.error().toString();
        return undefined ? QVariant() : v;
    }
    QVariantMap availRow(const char *action) const
    {
        return m_controller->property("commandAvailability")
            .toMap()
            .value(QLatin1String(action))
            .toMap();
    }
    QVariantMap applyOk(const char *kind, const QVariantMap &target,
                        const QVariantMap &params)
    {
        return m_controller->applyCommand(QLatin1String(kind), target,
                                          params, QString());
    }
    bool setRectSelection(const QRect &r)
    {
        QVariantMap rect;
        rect.insert(QStringLiteral("x"), r.x());
        rect.insert(QStringLiteral("y"), r.y());
        rect.insert(QStringLiteral("w"), r.width());
        rect.insert(QStringLiteral("h"), r.height());
        QVariantMap params;
        params.insert(QStringLiteral("source"), QStringLiteral("rect"));
        params.insert(QStringLiteral("rect"), rect);
        params.insert(QStringLiteral("op"), QStringLiteral("replace"));
        return applyOk("selection.set", {}, params)
            .value(QStringLiteral("ok"))
            .toBool();
    }
    // Selection-scoped constant-color fill through the door (the
    // tst_selection_gestures helper pattern) — used to lay distinctive
    // ground truth, then the selection is dropped.
    void fillRect(const QRect &r, double red, double green, double blue)
    {
        QVERIFY(setRectSelection(r));
        QVariantMap color;
        color.insert(QStringLiteral("model"), QStringLiteral("rgb"));
        color.insert(QStringLiteral("comps"),
                     QVariantList{red, green, blue, 0.0});
        QVariantMap params;
        params.insert(QStringLiteral("content"), QStringLiteral("color"));
        params.insert(QStringLiteral("color"), color);
        QVariantMap target;
        target.insert(QStringLiteral("layerId"), activeLayerId());
        QVERIFY(applyOk("fill", target, params)
                    .value(QStringLiteral("ok"))
                    .toBool());
        QVERIFY(applyOk("selection.none", {}, {})
                    .value(QStringLiteral("ok"))
                    .toBool());
    }
    QString activeLayerId() const
    {
        const QVariantList rows = m_controller->property("layers").toList();
        const int idx = m_controller->property("activeLayerIndex").toInt();
        return (idx >= 0 && idx < rows.size())
            ? rows.at(idx).toMap().value(QStringLiteral("layerId")).toString()
            : QString();
    }
    QStringList layerIds() const
    {
        QStringList out;
        const QVariantList rows = m_controller->property("layers").toList();
        for (const QVariant &v : rows)
            out << v.toMap().value(QStringLiteral("layerId")).toString();
        return out;
    }
    QImage fullReadback()
    {
        QImage img;
        const QRect full(0, 0, m_controller->docWidth(),
                         m_controller->docHeight());
        if (!m_controller->readRegionImage(full, &img))
            return QImage();
        return img;
    }
    QImage fullSelectionMask()
    {
        QImage img;
        const QRect full(0, 0, m_controller->docWidth(),
                         m_controller->docHeight());
        if (!m_controller->readSelectionImage(full, &img))
            return QImage();
        return img;
    }
    int historyCount() const
    {
        return m_controller->property("historyEntries").toList().size();
    }
    void freshDoc()
    {
        QVERIFY(m_controller->newDocument(320, 200,
                                          QStringLiteral("white")));
        QVERIFY(!m_controller->property("selectionActive").toBool());
        focusCanvas();
    }
    void focusCanvas()
    {
        m_canvas->forceActiveFocus();
        QTRY_VERIFY(m_canvas->hasActiveFocus());
    }
    // ---- v1.0.1 item 11e helpers ----
    QString toolNow() const
    {
        return m_canvas->property("tool").toString();
    }
    // Set the tool WITHOUT the keymap (the property door every pointer
    // path uses): ring pointers follow it, so a case can put a ring in
    // a known state before driving real keys.
    void setToolDirect(const char *tool)
    {
        m_canvas->setProperty("tool", QLatin1String(tool));
    }
    CanvasView *canvasView() const
    {
        return qobject_cast<CanvasView *>(m_canvas);
    }
    // Drain the announcer's 500 ms window so a probe's first
    // announcement is not coalesced with setup noise.
    void quiesceAnnouncer() { QTest::qWait(650); }
    // A FLAT wait is not a drain: if a coalesced flush lands late in
    // the wait, the probe's own announcement falls inside a fresh
    // rate-limit window and arrives up to 500 ms later — measured
    // first-hand as a load-dependent red here. Drain until the
    // announcer has actually been quiet for a full window, so the
    // probe's announcement takes the emit-immediately path.
    void drainAnnouncer(QSignalSpy &spy)
    {
        for (int i = 0; i < 8; ++i) {
            spy.clear();
            QTest::qWait(600);
            if (spy.isEmpty())
                return;
        }
        qWarning("announcer never went quiet — the probe below may "
                 "measure a coalesced window");
    }
    // Independent doc->view mapping (the item-7 harness derivation:
    // live zoom + the zoomFit centering contract).
    QPoint docToView(double x, double y) const
    {
        const double zoom = m_canvas->property("zoom").toDouble();
        const double dw = m_controller->docWidth();
        const double dh = m_controller->docHeight();
        const double offX = (m_canvas->width() - dw * zoom) / 2.0;
        const double offY = (m_canvas->height() - dh * zoom) / 2.0;
        return m_canvas
            ->mapToScene(QPointF(offX + x * zoom, offY + y * zoom))
            .toPoint();
    }
    bool confirmVisible() const
    {
        return evalJs(QStringLiteral("deleteConfirmDialog.visible"))
            .toBool();
    }
    // The active layer's row delegate in the Layers panel — visual-tree
    // walk (ListView delegates are VISUAL children only, the 11d
    // lesson), discriminated by the row's own modelData.layerId against
    // controller truth (HistoryPanel delegates carry kind/index/active,
    // never layerId).
    QQuickItem *activeLayerRow() const
    {
        const QString id = activeLayerId();
        if (id.isEmpty())
            return nullptr;
        std::function<QQuickItem *(QQuickItem *)> walk =
            [&](QQuickItem *node) -> QQuickItem * {
            const auto children = node->childItems();
            for (QQuickItem *c : children) {
                if (c->isVisible()
                    && c->property("modelData")
                            .toMap()
                            .value(QStringLiteral("layerId"))
                            .toString()
                        == id)
                    return c;
                if (QQuickItem *hit = walk(c))
                    return hit;
            }
            return nullptr;
        };
        return walk(m_window->contentItem());
    }
    // A real left-click on a (popup or dialog) item's center.
    void clickItem(QQuickItem *item)
    {
        QVERIFY(item);
        QTRY_VERIFY(item->width() > 0 && item->height() > 0);
        const QPointF scene = item->mapToScene(
            QPointF(item->width() / 2.0, item->height() / 2.0));
        QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                          scene.toPoint());
    }
    QQuickItem *uniqueVisibleButton(const char *catalogKey) const
    {
        const QString label = catTr(catalogKey);
        QList<QQuickItem *> hits;
        std::function<void(QQuickItem *)> walk = [&](QQuickItem *node) {
            const auto children = node->childItems();
            for (QQuickItem *c : children) {
                if (c->isVisible() && c->inherits("QQuickAbstractButton")
                    && c->property("text").toString() == label)
                    hits << c;
                walk(c);
            }
        };
        walk(m_window->contentItem());
        if (hits.size() != 1) {
            qWarning() << "uniqueVisibleButton:" << hits.size()
                       << "visible buttons render" << label;
            return nullptr;
        }
        return hits.first();
    }
    // The chordLabel text of a top-menu item found by cataloged text
    // (the 14a menu-bar lookup; ChordMenuItem names its chord Text
    // "chordLabel").
    QString topMenuChordText(const char *catalogKey) const
    {
        const QString code = QStringLiteral(
            "(function() {"
            "  var label = AppI18n.tr(\"%1\");"
            "  for (var i = 0; i < mainMenuBar.count; ++i) {"
            "    var m = mainMenuBar.menuAt(i);"
            "    if (!m) continue;"
            "    for (var j = 0; j < m.count; ++j) {"
            "      var it = m.itemAt(j);"
            "      if (it && it.text === label) return it;"
            "    }"
            "  }"
            "  return null;"
            "})()").arg(QLatin1String(catalogKey));
        QObject *item = evalJs(code).value<QObject *>();
        if (!item)
            return QString();
        QObject *chord = item->findChild<QObject *>(
            QStringLiteral("chordLabel"));
        return chord ? chord->property("text").toString() : QString();
    }
    // The MenuItem OBJECT itself, for the B-10 cold-open cases. The
    // neighbour above returns only its chord label.
    QObject *menuItemByKey(const char *catalogKey) const
    {
        const QString code = QStringLiteral(
            "(function() {"
            "  var label = AppI18n.tr(\"%1\");"
            "  for (var i = 0; i < mainMenuBar.count; ++i) {"
            "    var m = mainMenuBar.menuAt(i);"
            "    if (!m) continue;"
            "    for (var j = 0; j < m.count; ++j) {"
            "      var it = m.itemAt(j);"
            "      if (it && it.text === label) return it;"
            "    }"
            "  }"
            "  return null;"
            "})()").arg(QLatin1String(catalogKey));
        return evalJs(code).value<QObject *>();
    }
};

void TstChordTable::initTestCase()
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

    m_canvas = m_window->findChild<QQuickItem *>(
        QStringLiteral("canvasView"));
    QVERIFY2(m_canvas, "canvasView not found");

    // v1.0.1 item 11e: mirror main.cpp's dispatcher install. Without it
    // the letters reach NOTHING (they left the shortcut map at the
    // migration), so every letter case below would fail loudly rather
    // than pass vacuously — but the mirror is the point: this suite
    // drives the SAME dispatcher the shipped binary installs.
    m_toolKeys = new ToolKeyDispatcher(this);
    m_toolKeys->attach(m_window);
}

void TstChordTable::cleanup()
{
    // A failed case must not leak an open confirm dialog into the next.
    if (m_window)
        evalJs(QStringLiteral("deleteConfirmDialog.close()"));
    if (m_controller && m_controller->hasDocument())
        m_controller->applyCommand(QStringLiteral("selection.none"), {}, {},
                                   QString());
}

void TstChordTable::catalogGround()
{
    // Echo-proofing (RULE 9): the dialog/menu lookups below match on
    // REAL catalog values, pinned against the checked-in en.json.
    QCOMPARE(catTr("ui.menu.edit.delete"), QStringLiteral("Delete"));
    QCOMPARE(catTr("ui.dialog.cancel"), QStringLiteral("Cancel"));
    QCOMPARE(catTr("ui.dialog.apply"), QStringLiteral("Apply"));
    QCOMPARE(catTr("ui.menu.edit.cut"), QStringLiteral("Cut"));
    QCOMPARE(catTr("ui.menu.select.deselect"), QStringLiteral("Deselect"));
}

void TstChordTable::noDocumentChordsAreInert()
{
    // Runs BEFORE any document exists: every gated chord is a consumed
    // no-op (the disabled-menu-item semantic — the rows are the truth).
    QVERIFY(!m_controller->property("hasDocument").toBool());
    QVERIFY(availRow("transform.enter").value(QStringLiteral("enabled"))
                != QVariant(true));
    focusCanvas();
    const QString toolBefore = m_canvas->property("tool").toString();
    QTest::keyClick(m_window, Qt::Key_T, Qt::ControlModifier);
    QCOMPARE(m_canvas->property("tool").toString(), toolBefore);
    // Delete family: no dialog, no crash.
    QTest::keyClick(m_window, Qt::Key_Backspace);
    QTest::keyClick(m_window, Qt::Key_Backspace, Qt::AltModifier);
    QTest::keyClick(m_window, Qt::Key_Backspace, Qt::ControlModifier);
    QVERIFY(!confirmVisible());
    // Arrows + Cmd+J + Cmd+Shift+D: nothing to mutate, nothing thrown.
    QTest::keyClick(m_window, Qt::Key_Right);
    QTest::keyClick(m_window, Qt::Key_J, Qt::ControlModifier);
    QTest::keyClick(m_window, Qt::Key_D,
                    Qt::ControlModifier | Qt::ShiftModifier);
    QVERIFY(!m_controller->property("hasDocument").toBool());
}

void TstChordTable::guardedChordsSayWhyInsteadOfSwallowing()
{
    // P1-4(a). The chord above is INERT and that is correct; until
    // 0328e4ad it was also SILENT, which was not. Every guarded
    // Shortcut in Main.qml was written `if (rowEnabled(x)) do()` with
    // no else, so Cmd+T on no document did nothing and said nothing --
    // and so did Cmd+S, Cmd+Z and Cmd+V. The reason was already on the
    // wire the whole time: every disabled row computes a reasonId
    // naming its failing conjunct, and precisely nothing read it.
    //
    // Runs BEFORE any freshDoc case, like its neighbour: the whole
    // point is the no-document state.
    QVERIFY(!m_controller->property("hasDocument").toBool());

    // ---- (i) THE RUNTIME ARM, AND IT IS THE ONE THAT PROVES THE CALL
    // RESOLVES. qmllint cannot see AppDoc at all (it reports the
    // context property as an unqualified access), and qmlcachegen
    // compiles the call without checking it, so NEITHER the lint nor
    // the build is evidence that AppDoc.reportUnavailable exists. A
    // real Cmd+T through the real shortcut path is. If the method were
    // missing, QML would throw a TypeError into the console and the
    // press would stay silent -- which is exactly the pre-fix bug, and
    // is exactly what this spy reds on.
    QSignalSpy unavailable(m_controller,
                           &DocumentController::actionUnavailable);
    QVERIFY(unavailable.isValid());
    focusCanvas();
    QTest::keyClick(m_window, Qt::Key_T, Qt::ControlModifier);
    QCOMPARE(unavailable.count(), 1);
    QCOMPARE(unavailable.at(0).at(0).toString(),
             QStringLiteral("transform.enter"));
    const QString reasonId = unavailable.at(0).at(1).toString();
    QVERIFY2(!reasonId.isEmpty(),
             "Cmd+T reported unavailable with an EMPTY reasonId");

    // ---- (ii) NOT A KEY ECHO. The status line must carry the
    // TRANSLATED string, never the id. Catalog::tr echoes the key back
    // on a miss, so an uncataloged reason surfaces as a raw MessageId
    // at the user -- the P2-D2 class -- and it would pass a
    // "status is non-empty" assert without complaint.
    const QString shown = m_controller->property("statusText").toString();
    QVERIFY2(!shown.isEmpty(), "an unavailable chord left the status bar empty");
    QVERIFY2(shown != reasonId,
             qPrintable(QStringLiteral("status bar is echoing the raw id %1")
                            .arg(reasonId)));
    QVERIFY2(!shown.startsWith(QStringLiteral("ui.")),
             qPrintable(QStringLiteral("status bar shows a raw MessageId: %1")
                            .arg(shown)));

    // ---- (iii) A REPEATED PRESS MUST SPEAK AGAIN. setStatus dedupes
    // on identical text, so the second press of the same dead chord is
    // the case that goes quiet if the event/announcer arms are ever
    // dropped in favour of the status line alone.
    QTest::keyClick(m_window, Qt::Key_T, Qt::ControlModifier);
    QCOMPARE(unavailable.count(), 2);

    // ---- (iv) THE DOOR REPORTS A STATE, IT DOES NOT DECIDE ONE.
    // Silent on an unknown action (a typo must not manufacture a
    // message about a row that does not exist) -- both controls, so a
    // door that had gone globally silent could not pass (i) and these
    // together.
    unavailable.clear();
    m_controller->reportUnavailable(QStringLiteral("no.such.action.at.all"));
    QCOMPARE(unavailable.count(), 0);

    // ---- (v) EVERY GUARDED CHORD, NOT JUST THE ONE WITH A KEY EVENT
    // IN THIS TEST. The door is catalog-gated: a row whose reasonId is
    // not in the catalog returns EARLY AND SILENTLY, so a chord can be
    // wired correctly and still say nothing. That failure is invisible
    // from the QML side and this is the arm that finds it.
    // AND THE LIST ITSELF IS ASSERTED FIRST. guardedChordActions()
    // returns {} when Main.qml cannot be opened, and a loop over an
    // empty list PASSES -- the instrument would report success by
    // failing to run. Count first, then iterate, then check the number
    // actually probed.
    const QStringList guarded = guardedChordActions();
    QCOMPARE(guarded.size(), 13);
    int probed = 0;
    for (const QString &action : guarded) {
        if (availRow(action.toUtf8().constData())
                .value(QStringLiteral("enabled")).toBool())
            continue; // (vi) covers the enabled case
        unavailable.clear();
        m_controller->reportUnavailable(action);
        QVERIFY2(unavailable.count() == 1,
                 qPrintable(QStringLiteral("guarded chord %1 is wired but "
                                           "SILENT -- its reasonId is not "
                                           "cataloged, so the door returns "
                                           "early")
                                .arg(action)));
        ++probed;
    }
    QVERIFY2(probed == 13,
             qPrintable(QStringLiteral("only %1 of 13 guarded chords were "
                                       "probed -- the rest read as enabled "
                                       "with no document open")
                            .arg(probed)));

    // ---- (v-bis) THE edit.paste ROW SPECIFICALLY, at O-UI 7's ask.
    // Its guard comment records that the inline hasDocument/abiBlock12
    // conjuncts were COLLAPSED INTO this row (14b, "same truth"), which
    // makes it the one row where "the reasonId names the FAILING
    // conjunct" is a CLAIM rather than a restatement -- everywhere else
    // the row has exactly one conjunct to name. A collapsed row is the
    // place a generic fallback would hide: it would still be non-empty,
    // still cataloged, still translate, and still tell the user
    // nothing. Asserted NEGATIVELY against the fallback id, because
    // that is the failure that looks like success.
    {
        QSignalSpy paste(m_controller,
                         &DocumentController::actionUnavailable);
        m_controller->reportUnavailable(QStringLiteral("edit.paste"));
        QCOMPARE(paste.count(), 1);
        QVERIFY2(paste.at(0).at(1).toString()
                     != QStringLiteral("ui.error.action_unavailable"),
                 "edit.paste fell back to the GENERIC reason -- the row "
                 "that collapsed two conjuncts names neither");
    }

    // ---- (vi) SILENT ON AN ALREADY-ENABLED ACTION. Inventing a
    // refusal for an enabled row would make the chrome disagree with
    // the availability surface it is built on. Needs a document, so it
    // is checked in cmdTEntersTransformFrame's state via a direct call
    // there rather than here; the no-document state has no enabled row
    // to test with. Asserted instead as a ground fact so the omission
    // is visible rather than assumed:
    QVERIFY2(!availRow("transform.enter")
                  .value(QStringLiteral("enabled")).toBool(),
             "a row is enabled with no document open -- (vi) is untested "
             "here and this test's state assumption has rotted");

    // ---- (vii) THE SOURCE PIN, and it is the arm that would have
    // caught the miss that actually happened. O-UI 7 surveyed these
    // guards and found TWELVE; there are THIRTEEN. The other twelve are
    // Shortcut ONE-LINERS; edit.paste alone has its onActivated on its
    // own line under a comment, so a survey shaped to the one-liner
    // form matched every instance that shared the shape and missed
    // exactly the one that did not. A count taken from the file caught
    // it; a count taken from the survey would have confirmed itself.
    // This pin asserts the pairing per line, so a guard added later
    // WITHOUT an else reds here instead of shipping a silent chord.
    QFile mainQml(QFileInfo(QStringLiteral(__FILE__)).dir().filePath(
        QStringLiteral("../qml/Main.qml")));
    QVERIFY2(mainQml.open(QIODevice::ReadOnly),
             "shell/qml/Main.qml not readable from the test source dir");
    const QStringList lines =
        QString::fromUtf8(mainQml.readAll()).split(QLatin1Char('\n'));
    QRegularExpression guard(
        QStringLiteral("onActivated: if \\(root\\.rowEnabled\\(\"([^\"]+)\"\\)\\)"));
    int guards = 0;
    for (const QString &line : lines) {
        const QRegularExpressionMatch m = guard.match(line);
        if (!m.hasMatch())
            continue;
        ++guards;
        const QString key = m.captured(1);
        QVERIFY2(line.contains(QStringLiteral("reportUnavailable(\"%1\")")
                                   .arg(key)),
                 qPrintable(QStringLiteral("guarded chord %1 swallows its "
                                           "refusal -- no else branch")
                                .arg(key)));
    }
    QCOMPARE(guards, 13);
}

// ---- B-10: COLD OPEN ----------------------------------------------------
// The row reads "no document open: New/Import only". What the shell actually
// offers at cold open is New and OPEN -- both ungated MenuItems that dispatch
// no command -- while Import is correctly gated on hasDocument, because
// importImage() pastes into an OPEN document (document_controller.cpp:1545
// returns false when none is open). So the row's wording and the product
// disagree on the second door's NAME; the BEHAVIOUR is coherent and this
// case pins the behaviour. Q-D55 asks the operator which word is wrong.
//
// WHY THIS IS WORTH A CASE AT ALL: file.new and app.open appear NOWHERE in
// the command-availability surface -- `"file.new"` is 0 occurrences in
// document_controller.cpp -- so the 44-row matrix in
// tst_command_availability cannot see them. They are the only affordances
// that exist at launch, and until now nothing drove either one. A regression
// that disabled File > New would leave the application unusable from a cold
// start with all 51 suites still green.
void TstChordTable::coldOpenOffersExactlyTheDoorsThatCanMakeADocument()
{
    QVERIFY(!m_controller->property("hasDocument").toBool());

    // (i) THE WHOLE COMMAND SURFACE IS OFF, stated as a count rather than
    // spot-checked, with the non-vacuity guard that the surface is not empty
    // (an empty map would make "nothing is enabled" trivially true).
    const QVariantMap avail =
        m_controller->property("commandAvailability").toMap();
    QVERIFY2(avail.size() > 0, "the availability surface is EMPTY -- every "
                               "assertion below would be vacuous");
    QStringList wronglyOn;
    for (auto it = avail.constBegin(); it != avail.constEnd(); ++it)
        if (it.value().toMap().value(QStringLiteral("enabled")).toBool())
            wronglyOn << it.key();
    QVERIFY2(wronglyOn.isEmpty(),
             qPrintable(QStringLiteral("commands enabled with NO document: ")
                        + wronglyOn.join(QStringLiteral(", "))));

    // (ii) THE TWO DOORS ARE OPEN. This is also the control that makes every
    // `false` below meaningful: the SAME lookup, in the SAME run, returns
    // true here -- so a disabled reading is the item's gate and not a broken
    // probe or A33's closed-Menu effect.
    for (const char *k : {"menu.file.new", "app.open"}) {
        QObject *item = menuItemByKey(k);
        QVERIFY2(item, qPrintable(QStringLiteral("no MenuItem for %1")
                                      .arg(QLatin1String(k))));
        QVERIFY2(item->property("enabled").toBool(),
                 qPrintable(QStringLiteral("%1 is DISABLED at cold open -- "
                                           "the application cannot reach a "
                                           "document").arg(QLatin1String(k))));
    }

    // (iii) AND EVERY OTHER FILE DOOR IS SHUT.
    QStringList leaked;
    for (const char *k : {"ui.menu.file.save", "ui.menu.file.save_as",
                          "ui.menu.file.import", "menu.file.export_as"}) {
        QObject *item = menuItemByKey(k);
        QVERIFY2(item, qPrintable(QLatin1String(k)));
        if (item->property("enabled").toBool())
            leaked << QLatin1String(k);
    }
    QVERIFY2(leaked.isEmpty(),
             qPrintable(QStringLiteral("enabled with no document: ")
                        + leaked.join(QStringLiteral(", "))));
}

// The door DRIVEN, not just read. tst_pane_cycle reaches the same dialog by
// evaluating `newDialog.open()`, which proves the dialog exists and says
// nothing about whether the menu reaches it -- C-468's distinction, and the
// reason rows 5 and 6 needed real MenuItems rather than applyCommand.
// LIMIT STATED: this emits the MenuItem's own `triggered`, so it runs the
// item's declared handler; it does not synthesise a mouse click through an
// opened Menu. That is strictly more than the JS call it replaces and less
// than a pointer event, and the difference is worth naming rather than
// glossing.
void TstChordTable::fileNewIsALiveDoorAtColdOpenAndReachesADocument()
{
    QVERIFY(!m_controller->property("hasDocument").toBool());
    QObject *item = menuItemByKey("menu.file.new");
    QVERIFY(item);
    QVERIFY(QMetaObject::invokeMethod(item, "triggered"));
    QTRY_VERIFY2(evalJs(QStringLiteral("newDialog.visible")).toBool(),
                 "File > New did not open the New Document dialog");

    // Accepting routes through confirmGate -> AppDoc.newDocument (the
    // dialog's onAccepted). With nothing open there is nothing to discard,
    // so the gate must pass it straight through.
    evalJs(QStringLiteral("newDialog.accept()"));
    QTRY_VERIFY2(m_controller->property("hasDocument").toBool(),
                 "File > New was accepted and no document exists");
    // NOT property("activeLayerId"): that is a private METHOD, not a
    // Q_PROPERTY (document_controller.h:747), so the property() read returns
    // an invalid QVariant that is empty for every document ever opened. I
    // wrote it that way first and it red on a correct product, then failed
    // to compile when I reached for the method. `layers` is the public
    // surface, and one layer is the substantive claim anyway.
    QVERIFY2(!m_controller->property("layers").toList().isEmpty(),
             "a document arrived with no layers");
}

void TstChordTable::cmdTEntersTransformFrame()
{
    freshDoc();
    QCOMPARE(m_canvas->property("tool").toString(),
             QStringLiteral("brush.stroke"));
    QVERIFY(availRow("transform.enter")
                .value(QStringLiteral("enabled"))
                .toBool());
    QTest::keyClick(m_window, Qt::Key_T, Qt::ControlModifier);
    // The chord ENTERS transform.free with a live frame (item-13's
    // serves+lock-wall ground: the frame WORKS; no wall on a fresh
    // unlocked layer).
    QCOMPARE(m_canvas->property("tool").toString(),
             QStringLiteral("transform.free"));
    QVERIFY2(m_canvas->property("transformActive").toBool(),
             "Cmd+T did not begin the transform frame");
    QVERIFY(!m_canvas->property("transformWallActive").toBool());
    // Esc resolves the frame (item-13 teardown), tool stays; Cmd+T
    // again RE-ARMS (the setTool same-tool re-arm semantic).
    QTest::keyClick(m_window, Qt::Key_Escape);
    QVERIFY(!m_canvas->property("transformActive").toBool());
    QCOMPARE(m_canvas->property("tool").toString(),
             QStringLiteral("transform.free"));
    QTest::keyClick(m_window, Qt::Key_T, Qt::ControlModifier);
    QVERIFY2(m_canvas->property("transformActive").toBool(),
             "Cmd+T on the already-selected tool did not re-arm the "
             "frame");
    QTest::keyClick(m_window, Qt::Key_Escape);
    // Document untouched throughout (entering a frame is not an edit).
    QVERIFY(!m_controller->property("canUndo").toBool()
            || historyCount() == 1); // doc.new is the only journal entry
}

void TstChordTable::refusalToastShowsMechanismFirstAndRefires()
{
    // P1-2. THE OPERATOR'S RULING: every engine refusal reaching the
    // shell must surface VISIBLY. The toast landed days ago and until
    // now NOTHING ASSERTED IT EVER APPEARS -- no test, and no human had
    // seen it. A surface nobody has looked at and nothing tests is
    // indistinguishable from the silence it was built to replace.
    //
    // Driven by a REAL refusal, not by synthesising the signal: a
    // failed import takes the same setRejected path as the twenty
    // engine call sites, so this exercises the wire rather than a
    // stand-in for it.
    freshDoc();
    // RESET FIRST — this precondition used to hold for free and stopped
    // holding when the toast was reconciled to carry BOTH refusal
    // channels (O-UI, operator's item 4). guardedChordsSayWhyInsteadOf
    // Swallowing runs two cases earlier and presses disabled chords; those
    // presses now raise the toast, exactly as an engine refusal does, so
    // the surface is legitimately still up when this case starts.
    //
    // The assertion is KEPT rather than deleted: it is what proves the
    // toast is hidden BEFORE the refusal this case drives, without which
    // the "it appeared" check below would pass on a toast that was
    // already open.
    evalJs(QStringLiteral("refusalToast.visible = false"));
    QCOMPARE(evalJs(QStringLiteral("refusalToast.visible")).toBool(), false);

    const QString missing =
        QStringLiteral("/nonexistent/this-path-cannot-exist-9f3a.png");
    QVERIFY2(!m_controller->importImage(missing),
             "importing a nonexistent file SUCCEEDED -- the refusal this "
             "test drives no longer happens");

    QVERIFY2(evalJs(QStringLiteral("refusalToast.visible")).toBool(),
             "an engine refusal reached the shell and the toast stayed "
             "hidden -- the P1-2 ruling is violated");
    QCOMPARE(evalJs(QStringLiteral("refusalToast.code")).toString(),
             QStringLiteral("error.io.read_failed"));
    QCOMPARE(evalJs(QStringLiteral("refusalToast.message")).toString(),
             missing);

    // MECHANISM FIRST, AND THIS IS THE ORDERING THE D15 BRIEF RULED.
    // The composite "code: message" rendering is what makes a shortened
    // toast display a sentence that is FALSE for some rows, so the
    // mechanism is its own first line and the taxonomy id sits under
    // it. Asserted by POSITION, not by presence -- both strings being
    // on screen in the wrong order would pass a contains() check.
    QCOMPARE(evalJs(QStringLiteral("toastColumn.children[0].text")).toString(),
             missing);
    QCOMPARE(evalJs(QStringLiteral("toastColumn.children[1].text")).toString(),
             QStringLiteral("error.io.read_failed"));

    // NEVER ELIDED, ALWAYS WRAPPED. An elided mechanism is the exact
    // failure the ordering above exists to prevent, arriving by a
    // different route.
    QCOMPARE(evalJs(QStringLiteral("toastColumn.children[0].elide")).toInt(),
             int(Qt::ElideNone));
    QVERIFY2(evalJs(QStringLiteral("toastColumn.children[0].wrapMode")).toInt()
                 != 0,
             "the toast's mechanism line does not wrap");

    // IT RE-FIRES. setStatus dedupes on identical text, so a repeated
    // refusal is the case that goes quiet; the toast must re-arm and
    // must update to the NEW mechanism rather than stranding the old.
    const QString missing2 =
        QStringLiteral("/nonexistent/a-second-different-path-7c1b.png");
    QVERIFY(!m_controller->importImage(missing2));
    QVERIFY(evalJs(QStringLiteral("refusalToast.visible")).toBool());
    QCOMPARE(evalJs(QStringLiteral("refusalToast.message")).toString(),
             missing2);
    QVERIFY2(evalJs(QStringLiteral("refusalToastTimer.running")).toBool(),
             "the dismissal timer did not restart on a repeated refusal");

    // The alert is announced as an alert, not merely drawn.
    QCOMPARE(evalJs(QStringLiteral("refusalToast.Accessible.name")).toString(),
             missing2);

    evalJs(QStringLiteral("refusalToast.visible = false"));
}

void TstChordTable::deleteWithSelectionClearsCoverageScaled()
{
    freshDoc();
    // Ellipse selection: fractional coverage exists on the antialiased
    // rim — the §13.1 scaling's discriminating input.
    QVariantMap rect;
    rect.insert(QStringLiteral("x"), 60);
    rect.insert(QStringLiteral("y"), 50);
    rect.insert(QStringLiteral("w"), 120);
    rect.insert(QStringLiteral("h"), 80);
    QVariantMap params;
    params.insert(QStringLiteral("source"), QStringLiteral("ellipse"));
    params.insert(QStringLiteral("rect"), rect);
    params.insert(QStringLiteral("op"), QStringLiteral("replace"));
    params.insert(QStringLiteral("antialias"), true);
    QVERIFY(applyOk("selection.set", {}, params)
                .value(QStringLiteral("ok"))
                .toBool());
    const QImage coverage = fullSelectionMask();
    QVERIFY(!coverage.isNull());
    // Find one genuinely fractional pixel (engine's own mask = the
    // RULE-8 source of the expected value).
    QPoint fracPt(-1, -1);
    int fracCov = 0;
    for (int y = 50; y < 130 && fracPt.x() < 0; ++y) {
        const uchar *line = coverage.constScanLine(y);
        for (int x = 60; x < 180; ++x)
            if (line[x] > 32 && line[x] < 224) {
                fracPt = QPoint(x, y);
                fracCov = line[x];
                break;
            }
    }
    QVERIFY2(fracPt.x() >= 0,
             "no fractional-coverage pixel on the antialiased ellipse "
             "rim — cannot exercise the §13.1 scaling");
    QCOMPARE(int(coverage.constScanLine(90)[120]), 255); // interior
    QCOMPARE(int(coverage.constScanLine(10)[10]), 0);    // outside

    focusCanvas();
    QTest::keyClick(m_window, Qt::Key_Backspace);
    QImage after = fullReadback();
    QVERIFY(!after.isNull());
    // Interior: fully cleared (opaque white -> alpha 0).
    QCOMPARE(after.pixelColor(120, 90).alpha(), 0);
    // Outside: untouched opaque white.
    QCOMPARE(after.pixelColor(10, 10), QColor(255, 255, 255, 255));
    // The §13.1 fractional pixel: post alpha = 255 - coverage (±1).
    const int postAlpha = after.pixelColor(fracPt).alpha();
    QVERIFY2(qAbs(postAlpha - (255 - fracCov)) <= 1,
             qPrintable(QStringLiteral(
                            "coverage-scaled clear off at %1,%2: "
                            "coverage %3 -> post alpha %4 (want %5±1)")
                            .arg(fracPt.x())
                            .arg(fracPt.y())
                            .arg(fracCov)
                            .arg(postAlpha)
                            .arg(255 - fracCov)));
    // Delete clears CONTENT, not the selection.
    QVERIFY(m_controller->property("selectionActive").toBool());
    // Undo restores; the Delete (Key_Delete variant) lands the same
    // engine end-state as Backspace did.
    m_controller->undo();
    QCOMPARE(fullReadback().pixelColor(120, 90),
             QColor(255, 255, 255, 255));
    QTest::keyClick(m_window, Qt::Key_Delete);
    const QImage afterDelete = fullReadback();
    QVERIFY2(afterDelete == after,
             "Key_Delete and Key_Backspace landed different engine "
             "end-states");
}

void TstChordTable::deleteNoSelectionConfirmCancelApply()
{
    freshDoc();
    const QImage before = fullReadback();
    const int histBefore = historyCount();
    // The dialog names the object (§32.5): the active layer's name from
    // info truth.
    const QString layerName = m_controller->property("layers")
                                  .toList()
                                  .first()
                                  .toMap()
                                  .value(QStringLiteral("name"))
                                  .toString();
    QVERIFY(!layerName.isEmpty());

    focusCanvas();
    QTest::keyClick(m_window, Qt::Key_Backspace);
    QTRY_VERIFY2(confirmVisible(),
                 "Delete with no selection did not open the §32.5 "
                 "confirm");
    // Opening is NOT a mutation.
    QCOMPARE(historyCount(), histBefore);
    QObject *nameLabel = m_window->findChild<QObject *>(
        QStringLiteral("deleteConfirmObjectName"));
    QVERIFY(nameLabel);
    QCOMPARE(nameLabel->property("text").toString(), layerName);

    // CANCEL = engine-proven no-op.
    QQuickItem *cancel = uniqueVisibleButton("ui.dialog.cancel");
    QVERIFY(cancel);
    clickItem(cancel);
    QTRY_VERIFY(!confirmVisible());
    QCOMPARE(historyCount(), histBefore);
    QVERIFY2(fullReadback() == before,
             "Cancel changed the document");

    // CONFIRM = the §40.5 whole-content clear, engine-proven.
    QTest::keyClick(m_window, Qt::Key_Delete); // the other key, same arm
    QTRY_VERIFY(confirmVisible());
    QQuickItem *apply = uniqueVisibleButton("ui.dialog.apply");
    QVERIFY(apply);
    clickItem(apply);
    QTRY_VERIFY(!confirmVisible());
    const QImage after = fullReadback();
    QCOMPARE(after.pixelColor(0, 0).alpha(), 0);
    QCOMPARE(after.pixelColor(160, 100).alpha(), 0);
    QCOMPARE(after.pixelColor(319, 199).alpha(), 0);
    QCOMPARE(historyCount(), histBefore + 1);
    m_controller->undo();
    QVERIFY2(fullReadback() == before,
             "undo did not restore the whole-content clear");
}

void TstChordTable::deleteTransparencyLockRefusesTyped()
{
    // The disposition's third arm (desk ruling): a transparency lock
    // BLOCKS edit.cut outright (the §40.1 alpha-decrease rule ->
    // error.command.locked) and the shell surfaces the TYPED refusal on
    // the existing ui.status.rejected path — never a silent no-op, never
    // a pre-computed disable (locks are write-only pre-info-v2: the
    // edit.delete row stays ENABLED, the recorded honest limit).
    freshDoc();
    QVariantMap locks;
    locks.insert(QStringLiteral("transparency"), true);
    QVariantMap props;
    props.insert(QStringLiteral("locks"), locks);
    QVariantMap target;
    target.insert(QStringLiteral("layerId"), activeLayerId());
    QVERIFY(applyOk("layer.set_props", target, props)
                .value(QStringLiteral("ok"))
                .toBool());
    QVERIFY(setRectSelection(QRect(60, 50, 80, 60)));
    const QImage before = fullReadback();
    const int histBefore = historyCount();
    // The honest limit pinned: the row cannot see the lock.
    QVERIFY(availRow("edit.delete")
                .value(QStringLiteral("enabled"))
                .toBool());
    focusCanvas();
    QTest::keyClick(m_window, Qt::Key_Backspace);
    // Engine truth: document unchanged, no journal step, and the status
    // line carries the TYPED code through ui.status.rejected
    // ("Rejected — {code}: {message}" — catalog-pinned rendering).
    QVERIFY2(fullReadback() == before,
             "a transparency-locked Delete mutated the document");
    QCOMPARE(historyCount(), histBefore);
    const QString status = m_controller->property("statusText").toString();
    QVERIFY2(status.contains(QStringLiteral("error.command.locked")),
             qPrintable(QStringLiteral("status does not carry the typed "
                                       "lock refusal: \"%1\"")
                            .arg(status)));
    // Positive control: unlocking makes the same chord clear — the lock
    // was the only blocker.
    locks.insert(QStringLiteral("transparency"), false);
    props.insert(QStringLiteral("locks"), locks);
    QVERIFY(applyOk("layer.set_props", target, props)
                .value(QStringLiteral("ok"))
                .toBool());
    focusCanvas();
    QTest::keyClick(m_window, Qt::Key_Backspace);
    QCOMPARE(fullReadback().pixelColor(100, 80).alpha(), 0);
}

void TstChordTable::deleteMidDragIsIgnored()
{
    // The item-7 mid-gesture rule extended to the 14b chords: a
    // button-held gesture owns the keyboard. Mid-marquee (no committed
    // selection yet) a plain Delete would take the NO-SELECTION arm and
    // open the MODAL confirm mid-drag — the grab steal one level
    // removed (the item-7 keyboard-guard rationale). The guard yields:
    // no dialog, no mutation, and the marquee commits normally.
    freshDoc();
    m_canvas->setProperty("tool", QStringLiteral("select.rect"));
    focusCanvas();
    const int histBefore = historyCount();
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(50, 40));
    QTest::mouseMove(m_window, docToView(110, 80));
    // Mid-drag Delete AND an arrow: both must be ignored.
    QTest::keyClick(m_window, Qt::Key_Backspace);
    QVERIFY2(!confirmVisible(),
             "the §32.5 confirm opened MID-DRAG — the gesture guard "
             "broke (item-7 F1/F4 class)");
    QTest::keyClick(m_window, Qt::Key_Right);
    QCOMPARE(historyCount(), histBefore); // no mid-drag mutation
    QTest::mouseMove(m_window, docToView(150, 120));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(150, 120));
    QTRY_VERIFY2(m_controller->property("selectionActive").toBool(),
                 "the marquee died after mid-drag chords");
    // Exactly ONE new journal step: the committed selection.set (no
    // nudge, no delete rode along).
    QCOMPARE(historyCount(), histBefore + 1);
}

void TstChordTable::deleteConfirmDismissalIsConsumed()
{
    // The item-7 F4 lesson applied to the NEW dialog: the modal
    // confirm's outside-dismissal press must never leak to the canvas
    // tool (a leak under brush paints a journaled dab — the F4
    // instrument: whole-doc non-white scan).
    freshDoc();
    m_canvas->setProperty("tool", QStringLiteral("brush.stroke"));
    focusCanvas();
    QTest::keyClick(m_window, Qt::Key_Backspace);
    QTRY_VERIFY(confirmVisible());
    // A real press on the canvas, well away from the centered dialog
    // (dialog is anchors.centerIn: parent; the top-left doc corner is
    // outside it — asserted via the dialog's scene rect).
    const QPoint dismissPt = docToView(8.0, 8.0);
    QObject *dlg = m_window->findChild<QObject *>(
        QStringLiteral("deleteConfirmDialog"));
    QVERIFY(dlg);
    QQuickItem *content =
        dlg->property("contentItem").value<QQuickItem *>();
    QVERIFY(content);
    const QRectF dlgScene = content->mapRectToScene(
        QRectF(0, 0, content->width(), content->height()))
        .adjusted(-12, -48, 12, 12); // frame + title margin
    QVERIFY2(!dlgScene.contains(QPointF(dismissPt)),
             "dismiss point sits under the dialog — re-place it");
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier, dismissPt);
    QTRY_VERIFY2(!confirmVisible(),
                 "outside click did not dismiss the modal confirm");
    // The press was CONSUMED: no dab, no engine change.
    const QImage img = fullReadback();
    for (int y = 0; y < img.height(); ++y) {
        const QRgb *line = reinterpret_cast<const QRgb *>(
            img.constScanLine(y));
        for (int x = 0; x < img.width(); ++x)
            QVERIFY2(line[x] == qRgba(255, 255, 255, 255),
                     qPrintable(QStringLiteral(
                                    "the dismissal press LEAKED — "
                                    "non-white pixel at %1,%2 (painted "
                                    "dab, the F4 class)")
                                    .arg(x)
                                    .arg(y)));
    }
}

void TstChordTable::fillChordsUseEngineConstants()
{
    freshDoc();
    // R15 discriminator: the SHELL swatch is red — if the chord wrongly
    // consulted it, the fill below would be red, not the engine's
    // headless foreground constant (black — kind_impls.rs).
    m_canvas->setProperty("brushColor", QColor(Qt::red));
    QVERIFY(setRectSelection(QRect(60, 50, 80, 60)));
    focusCanvas();
    QTest::keyClick(m_window, Qt::Key_Backspace, Qt::AltModifier);
    QImage img = fullReadback();
    QCOMPARE(img.pixelColor(100, 80), QColor(0, 0, 0, 255)); // inside
    QCOMPARE(img.pixelColor(10, 10), QColor(255, 255, 255, 255));
    QVERIFY2(img.pixelColor(100, 80) != QColor(Qt::red),
             "the fill chord consulted the SHELL swatch — the R15 "
             "distinction broke");
    // Background arm needs a non-white ground to discriminate white:
    // lay blue, then Cmd+Delete a subrect.
    m_controller->undo();
    QVERIFY(applyOk("selection.none", {}, {})
                .value(QStringLiteral("ok"))
                .toBool());
    fillRect(QRect(0, 0, 320, 200), 0.0, 0.0, 1.0); // blue ground
    QVERIFY(setRectSelection(QRect(60, 50, 80, 60)));
    focusCanvas();
    QTest::keyClick(m_window, Qt::Key_Delete, Qt::ControlModifier);
    img = fullReadback();
    QCOMPARE(img.pixelColor(100, 80), QColor(255, 255, 255, 255));
    const QColor outside = img.pixelColor(10, 10);
    QVERIFY2(outside.blue() > 200 && outside.red() < 60,
             "the background fill leaked outside the selection");
}

void TstChordTable::reselectMemoryArmsAndReplays()
{
    freshDoc();
    // BEFORE any memory: the row is disabled (the 14a flip's off state)
    // and the chord is engine-proven inert.
    QVERIFY(!availRow("select.reselect")
                 .value(QStringLiteral("enabled"))
                 .toBool());
    const int histBefore = historyCount();
    QTest::keyClick(m_window, Qt::Key_D,
                    Qt::ControlModifier | Qt::ShiftModifier);
    QCOMPARE(historyCount(), histBefore);
    QVERIFY(!m_controller->property("selectionActive").toBool());

    // A REAL marquee gesture (the journaled selection.set door — the
    // gesture-net idiom: the capture must ride real input).
    m_canvas->setProperty("tool", QStringLiteral("select.rect"));
    focusCanvas();
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(50, 40));
    QTest::mouseMove(m_window, docToView(100, 90));
    QTest::mouseMove(m_window, docToView(150, 120));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(150, 120));
    QTRY_VERIFY2(m_controller->property("selectionActive").toBool(),
                 "the marquee did not commit a selection");
    const QImage captured = fullSelectionMask();
    QVERIFY(!captured.isNull());

    // Deselect through the (row-guarded) Cmd+D chord — also verifies
    // the migrated existing chord still fires.
    QTest::keyClick(m_window, Qt::Key_D, Qt::ControlModifier);
    QTRY_VERIFY(!m_controller->property("selectionActive").toBool());

    // The memory survived the deselect: row enabled (the on state).
    QVERIFY2(availRow("select.reselect")
                 .value(QStringLiteral("enabled"))
                 .toBool(),
             "select.reselect row did not arm after an applied "
             "selection.set");

    // Cmd+Shift+D replays through the same door: mask byte-compare.
    QTest::keyClick(m_window, Qt::Key_D,
                    Qt::ControlModifier | Qt::ShiftModifier);
    QTRY_VERIFY2(m_controller->property("selectionActive").toBool(),
                 "reselect did not re-establish the selection");
    const QImage replayed = fullSelectionMask();
    QVERIFY2(replayed == captured,
             "the reselected mask differs from the captured selection "
             "(byte-compare)");
}

void TstChordTable::cmdJViaCopyBothArms()
{
    freshDoc();
    fillRect(QRect(80, 60, 100, 80), 0.0, 0.0, 1.0); // blue block
    const QImage sourceContent =
        m_controller->layerThumbImage(activeLayerId());
    QVERIFY(!sourceContent.isNull());

    // ---- no-selection arm: layer.duplicate ----
    QStringList before = layerIds();
    QCOMPARE(before.size(), 1);
    focusCanvas();
    QTest::keyClick(m_window, Qt::Key_J, Qt::ControlModifier);
    QTRY_COMPARE(layerIds().size(), 2);
    QString dupId;
    for (const QString &id : layerIds())
        if (!before.contains(id))
            dupId = id;
    QVERIFY(!dupId.isEmpty());
    const QImage dupContent = m_controller->layerThumbImage(dupId);
    QVERIFY2(dupContent == sourceContent,
             "the duplicate's content differs from the source layer");
    m_controller->undo();
    QTRY_COMPARE(layerIds().size(), 1);

    // ---- selection arm: app-side coverage-scaled copy -> new layer ----
    QVERIFY(setRectSelection(QRect(60, 50, 80, 60)));
    before = layerIds();
    focusCanvas();
    QTest::keyClick(m_window, Qt::Key_J, Qt::ControlModifier);
    QTRY_COMPARE(layerIds().size(), 2);
    QString viaId;
    for (const QString &id : layerIds())
        if (!before.contains(id))
            viaId = id;
    QVERIFY(!viaId.isEmpty());
    const QImage viaContent = m_controller->layerThumbImage(viaId);
    QVERIFY(!viaContent.isNull());
    // Inside the selection: the source pixels land aligned. (100,80) is
    // inside both the selection and the blue block; (65,55) inside the
    // selection on white ground.
    QCOMPARE(viaContent.pixelColor(100, 80), QColor(0, 0, 255, 255));
    QCOMPARE(viaContent.pixelColor(65, 55), QColor(255, 255, 255, 255));
    // Outside the selection: transparent (coverage-masked).
    QCOMPARE(viaContent.pixelColor(10, 10).alpha(), 0);
    QCOMPARE(viaContent.pixelColor(200, 150).alpha(), 0);
    // The source layer is untouched (copy, not cut).
    QVERIFY2(m_controller->layerThumbImage(before.first())
                 == sourceContent,
             "via-copy mutated the source layer");
}

void TstChordTable::arrowNudgeIntegerArm()
{
    freshDoc();
    fillRect(QRect(100, 100, 20, 20), 0.0, 0.0, 1.0); // blue marker
    focusCanvas();
    const int histBefore = historyCount();

    // Right +1 px: the blue block occupies x in [100,120) — after the
    // nudge [101,121): x=120 flips white->blue, x=100 flips blue->white
    // (integer-arm exactness; derivations from the fill rect).
    QTest::keyClick(m_window, Qt::Key_Right);
    QImage img = fullReadback();
    QCOMPARE(img.pixelColor(120, 110), QColor(0, 0, 255, 255));
    QCOMPARE(img.pixelColor(100, 110), QColor(255, 255, 255, 255));
    // The whole LAYER moved: the vacated left column is transparent.
    QCOMPARE(img.pixelColor(0, 110).alpha(), 0);
    QCOMPARE(historyCount(), histBefore + 1);

    // Shift+Left = 10 px: net offset -9 -> block at [91,111).
    QTest::keyClick(m_window, Qt::Key_Left, Qt::ShiftModifier);
    img = fullReadback();
    QCOMPARE(img.pixelColor(95, 110), QColor(0, 0, 255, 255));
    QCOMPARE(img.pixelColor(111, 110), QColor(255, 255, 255, 255));
    QCOMPARE(historyCount(), histBefore + 2);

    // Down +1 px: block rows [100,120) -> [101,121).
    QTest::keyClick(m_window, Qt::Key_Down);
    img = fullReadback();
    QCOMPARE(img.pixelColor(95, 120), QColor(0, 0, 255, 255));
    QCOMPARE(img.pixelColor(95, 100), QColor(255, 255, 255, 255));
    // Shift+Up = 10 px: rows back to [91,111).
    QTest::keyClick(m_window, Qt::Key_Up, Qt::ShiftModifier);
    img = fullReadback();
    QCOMPARE(img.pixelColor(95, 92), QColor(0, 0, 255, 255));
    QCOMPARE(img.pixelColor(95, 115), QColor(255, 255, 255, 255));
    QCOMPARE(historyCount(), histBefore + 4);
}

void TstChordTable::arrowsYieldToFocusedTextField()
{
    freshDoc();
    // The wand-tolerance SpinBox is real chrome with a real text editor
    // — the §30.2 probe: focus it, send an arrow, and the canvas must
    // never see the key (no nudge), while the SpinBox DOES (its value
    // moves — the positive control proving the key was delivered).
    m_canvas->setProperty("tool", QStringLiteral("select.wand"));
    QQuickItem *spin = nullptr;
    // 11d Phase-1 integration seam: the options bar became a
    // declarative model + Repeater, so the SpinBox is DELEGATE-CREATED
    // on the tool swap — and Repeater delegates are VISUAL children
    // only (no QObject parent chain to the window), so the old
    // window-findChildren lookup cannot see them. Resolve the options
    // pane by qml id and walk the VISUAL tree, retrying until the
    // instantiation lands; the §30.2 probe itself is unchanged.
    auto *toolbar = qobject_cast<QQuickItem *>(
        evalJs(QStringLiteral("toolbarPane")).value<QObject *>());
    QVERIFY2(toolbar, "toolbarPane not resolvable");
    std::function<QQuickItem *(QQuickItem *)> findVisibleSpin =
        [&](QQuickItem *node) -> QQuickItem * {
        const auto children = node->childItems();
        for (QQuickItem *c : children) {
            if (c->isVisible() && c->inherits("QQuickSpinBox"))
                return c;
            if (QQuickItem *hit = findVisibleSpin(c))
                return hit;
        }
        return nullptr;
    };
    QTRY_VERIFY2((spin = findVisibleSpin(toolbar)) != nullptr,
                 "wand-tolerance SpinBox not visible");
    spin->forceActiveFocus();
    QTRY_VERIFY(spin->hasActiveFocus());
    QVERIFY(!m_canvas->hasActiveFocus());
    const int valBefore = spin->property("value").toInt();
    const int histBefore = historyCount();
    const QImage before = fullReadback();
    QTest::keyClick(m_window, Qt::Key_Up);
    QTest::keyClick(m_window, Qt::Key_Right);
    // Positive control: the SpinBox consumed Up (value moved).
    QTRY_COMPARE(spin->property("value").toInt(), valBefore + 1);
    // The canvas saw NOTHING: no layer.transform submitted, no pixel
    // moved.
    QCOMPARE(historyCount(), histBefore);
    QVERIFY2(fullReadback() == before,
             "an arrow key reached the canvas while a text control had "
             "focus (§30.2 yield broke)");
}

void TstChordTable::singleKeySuppressionInRenameField()
{
    // KEY-LAYER slice — ES:16452 [NORMATIVE]: "While a text/numeric
    // field has focus … single-key and Shift+letter shortcuts (tool
    // letters, …) are suppressed — they insert text. Modifier chords
    // still dispatch (§30.2)." The guard lives at the DISPATCH layer
    // (Main.qml textEditingFocus on the letter Shortcuts), so the
    // letter must BOTH leave the tool alone AND land in the field.
    freshDoc();
    // Known start (newDocument does NOT reset the tool; the preceding
    // case leaves select.wand): pin brush.stroke — the harness idiom
    // arrowsYieldToFocusedTextField itself uses. Both guarded letters'
    // targets (select.rect, text) differ from it, so a suppression
    // break WOULD move the tool — the no-change asserts below are
    // non-vacuous.
    m_canvas->setProperty("tool", QStringLiteral("brush.stroke"));
    QCOMPARE(m_canvas->property("tool").toString(),
             QStringLiteral("brush.stroke"));

    // The panel's REAL rename path: double-click the active layer row
    // (the row MouseArea shows nameEdit and forceActiveFocus()es it).
    QQuickItem *row = nullptr;
    QTRY_VERIFY2((row = activeLayerRow()) != nullptr,
                 "active layer row not visible in the Layers panel");
    const QPointF rowCenter = row->mapToScene(
        QPointF(row->width() / 2.0, row->height() / 2.0));
    QTest::mouseDClick(m_window, Qt::LeftButton, Qt::NoModifier,
                       rowCenter.toPoint());
    std::function<QQuickItem *(QQuickItem *)> findEdit =
        [&](QQuickItem *node) -> QQuickItem * {
        const auto children = node->childItems();
        for (QQuickItem *c : children) {
            if (c->isVisible() && c->inherits("QQuickTextField"))
                return c;
            if (QQuickItem *hit = findEdit(c))
                return hit;
        }
        return nullptr;
    };
    QQuickItem *edit = nullptr;
    QTRY_VERIFY2((edit = findEdit(row)) != nullptr,
                 "rename TextField did not appear on double-click");
    QTRY_VERIFY(edit->hasActiveFocus());
    QVERIFY(!m_canvas->hasActiveFocus());
    QCOMPARE(edit->property("selectedText").toString(), QString());

    // Instrument check (RULE 9): the dispatch-layer guard ENGAGES for
    // this focus. MEASURED on this Qt (6.11 offscreen): the focused
    // text editor ALSO claims plain letters itself via ShortcutOverride
    // — the behavioral asserts below hold even with the guard stripped
    // — so without this pin (and the source pin at the end) a guard
    // regression would ride the platform's own yield unseen.
    QVERIFY2(evalJs(QStringLiteral("root.textEditingFocus")).toBool(),
             "ES:16452 dispatch guard not engaged while the rename "
             "field has focus");

    const int histBefore = historyCount();
    const QString toolBefore = m_canvas->property("tool").toString();

    // (a) "m": inserts at the field's own cursor (expected value derived
    // from the field's pre-keystroke state — no cursor assumptions),
    // and the tool does NOT change.
    QString expect = edit->property("text").toString();
    expect.insert(edit->property("cursorPosition").toInt(),
                  QLatin1Char('m'));
    QTest::keyClick(m_window, Qt::Key_M);
    QCOMPARE(edit->property("text").toString(), expect);
    QCOMPARE(m_canvas->property("tool").toString(), toolBefore);

    // (b) "t": same, and no transform frame arms.
    expect = edit->property("text").toString();
    expect.insert(edit->property("cursorPosition").toInt(),
                  QLatin1Char('t'));
    QTest::keyClick(m_window, Qt::Key_T);
    QCOMPARE(edit->property("text").toString(), expect);
    QCOMPARE(m_canvas->property("tool").toString(), toolBefore);
    QVERIFY(!m_canvas->property("transformActive").toBool());

    // Modifier chords still dispatch (ES:16452 / §30.2) — the shipped
    // honest behavior: Cmd+T enters transform.free while the field has
    // focus (the guard covers single-key chords only).
    QTest::keyClick(m_window, Qt::Key_T, Qt::ControlModifier);
    QTRY_COMPARE(m_canvas->property("tool").toString(),
                 QStringLiteral("transform.free"));
    QVERIFY(m_canvas->property("transformActive").toBool());

    // None of it touched the document (tool state is app state; the
    // typed letters live in the uncommitted field only).
    QCOMPARE(historyCount(), histBefore);

    // Wind down: canvas focus back (nameEdit hides on focus loss), Esc
    // resolves the transform frame.
    focusCanvas();
    QTest::keyClick(m_window, Qt::Key_Escape);
    QTRY_VERIFY(!m_canvas->property("transformActive").toBool());
    QTRY_VERIFY(!edit->isVisible());

    // ---- the NUMERIC-field arm (ES:16452 names "text/numeric") ----
    // The wand-tolerance SpinBox, focused by its real path (the
    // arrowsYieldToFocusedTextField §30.2 probe pattern: options-bar
    // delegate, visual-tree walk).
    m_canvas->setProperty("tool", QStringLiteral("select.wand"));
    auto *toolbar = qobject_cast<QQuickItem *>(
        evalJs(QStringLiteral("toolbarPane")).value<QObject *>());
    QVERIFY2(toolbar, "toolbarPane not resolvable");
    std::function<QQuickItem *(QQuickItem *)> findVisibleSpin =
        [&](QQuickItem *node) -> QQuickItem * {
        const auto children = node->childItems();
        for (QQuickItem *c : children) {
            if (c->isVisible() && c->inherits("QQuickSpinBox"))
                return c;
            if (QQuickItem *hit = findVisibleSpin(c))
                return hit;
        }
        return nullptr;
    };
    QQuickItem *spin = nullptr;
    QTRY_VERIFY2((spin = findVisibleSpin(toolbar)) != nullptr,
                 "wand-tolerance SpinBox not visible");
    spin->forceActiveFocus();
    QTRY_VERIFY(spin->hasActiveFocus());
    QVERIFY(!m_canvas->hasActiveFocus());
    // Guard engagement on the NUMERIC surface too: the SpinBox is a
    // focus scope whose editor (a TextInput) takes the active focus,
    // so the type-keyed guard sees a text editor here as well.
    QVERIFY2(evalJs(QStringLiteral("root.textEditingFocus")).toBool(),
             "ES:16452 dispatch guard not engaged while the numeric "
             "field has focus");
    // Bare letters while the numeric field has focus: the tool holds
    // still (select.wand differs from every letter target below, so a
    // break WOULD move it).
    QTest::keyClick(m_window, Qt::Key_M);
    QCOMPARE(m_canvas->property("tool").toString(),
             QStringLiteral("select.wand"));
    QTest::keyClick(m_window, Qt::Key_T);
    QCOMPARE(m_canvas->property("tool").toString(),
             QStringLiteral("select.wand"));
    QVERIFY(!m_canvas->property("transformActive").toBool());
    QTest::keyClick(m_window, Qt::Key_B);
    QCOMPARE(m_canvas->property("tool").toString(),
             QStringLiteral("select.wand"));
    QCOMPARE(historyCount(), histBefore);
    focusCanvas();

    // ---- source pin, RE-ANCHORED at the 11e migration (the
    // tst_palette_pins d4 parse-the-source-of-truth pattern) ----
    // ORIGINAL ANCHOR (key-layer slice): the eight `sequence: "<L>"`
    // Shortcut lines in Main.qml, each asserted to carry
    // `enabled: !root.textEditingFocus`. The 11e migration DELETES
    // those lines — the letters had to leave the shortcut map for a
    // real key-event dispatcher to see their press/release at all — so
    // this pin went stale-red BY CONSTRUCTION. It is RE-ANCHORED, never
    // deleted: deleting a pin because your own change broke it is how
    // coverage dies quietly. Same sensitivity, new home.
    // WHY A SOURCE PIN AT ALL (the measured reason, unchanged): on this
    // Qt a focused text editor ALSO claims plain letters via
    // ShortcutOverride, so a stripped guard rides the platform's own
    // yield and the behavioral asserts above stay green. The guard's
    // PRESENCE and its POSITION in the dispatch path are therefore
    // pinned at the source, and the no-yield-surface case
    // (guardHoldsOnANoYieldSurface) pins it behaviorally on a surface
    // that claims nothing.
    QFile disp(QFileInfo(QStringLiteral(__FILE__)).dir().filePath(
        QStringLiteral("../src/tool_key_dispatcher.cpp")));
    QVERIFY2(disp.open(QIODevice::ReadOnly),
             "shell/src/tool_key_dispatcher.cpp not readable from the "
             "test source dir — the 11e letter table's home moved");
    const QString src = QString::fromUtf8(disp.readAll());
    // (i) every ruled letter has a row in the dispatcher's own table,
    // in source AND at runtime (the runtime half is what actually
    // dispatches; the source half is what a reviewer reads).
    // 1.0 final (corrigendum (l)): G (paint bucket) and T (text) lost
    // their only members and are UNBOUND; (i-b) below pins that.
    const char *letters[] = {"V", "M", "L", "W", "C", "B", "E", "J",
                             "S", "O", "P", "U"};
    for (const char *l : letters) {
        const QString row = QStringLiteral("Qt::Key_%1, \"%1\"")
                                .arg(QLatin1String(l));
        QVERIFY2(src.contains(row),
                 qPrintable(QStringLiteral("ruled letter %1 has no row "
                                           "in the dispatcher's letter "
                                           "table")
                                .arg(QLatin1String(l))));
        QVERIFY2(!m_toolKeys->ringMembers(QLatin1String(l)).isEmpty(),
                 qPrintable(QStringLiteral("ruled letter %1 carries an "
                                           "EMPTY ring at runtime")
                                .arg(QLatin1String(l))));
    }
    // (i-b) the removed letters have no row and an empty ring.
    for (const char *l : {"G", "T"}) {
        QVERIFY2(!src.contains(QStringLiteral("Qt::Key_%1, \"%1\"").arg(QLatin1String(l))),
                 qPrintable(QStringLiteral("removed letter %1 is back in the dispatcher's table")
                                .arg(QLatin1String(l))));
        QVERIFY2(m_toolKeys->ringMembers(QLatin1String(l)).isEmpty(),
                 qPrintable(QStringLiteral("removed letter %1 still has a ring at runtime")
                                .arg(QLatin1String(l))));
    }
    // The X action chord is in the table too (no ring — its own row).
    QVERIFY2(src.contains(QStringLiteral("Qt::Key_X")),
             "the X swap-colors action chord left the dispatcher");
    QCOMPARE(ToolKeyDispatcher::swapColorsKey(), Qt::Key_X);
    // D stays OUT this wave (deferred, blocked on mint): no ring row,
    // no action row. A D that silently appeared would red here.
    QVERIFY2(!src.contains(QStringLiteral("Qt::Key_D")),
             "D-reset-colors appeared in the ruled table — it is "
             "DEFERRED this wave (no cataloged control name; shipping "
             "it would either mint an id or ship a keyboard-only "
             "affordance)");
    // (ii) the guard is consulted, and consulted BEFORE any dispatch:
    // the structural claim a behavioral assert cannot make on a
    // platform that yields anyway.
    const int filterAt = src.indexOf(QStringLiteral("::eventFilter"));
    QVERIFY2(filterAt > 0, "the dispatcher lost its event filter");
    const int guardAt =
        src.indexOf(QStringLiteral("textEditingGuardEngaged()"), filterAt);
    const int ringAt =
        src.indexOf(QStringLiteral("dispatchRing(ringIndex)"), filterAt);
    const int swapAt = src.indexOf(QStringLiteral("dispatchSwap()"), filterAt);
    QVERIFY2(guardAt > 0,
             "the ES:16452 guard is no longer consulted in the "
             "dispatch path");
    QVERIFY2(ringAt > guardAt && swapAt > guardAt,
             "the ES:16452 guard is consulted AFTER dispatch — letters "
             "would switch tools out of a focused field");
    // (iii) the guard predicate itself still reads the ONE QML
    // definition rather than a second, driftable copy.
    QVERIFY2(src.contains(QStringLiteral("textEditingFocus")),
             "the dispatcher stopped reading the QML textEditingFocus "
             "guard — a duplicated predicate can drift from it");
}

void TstChordTable::bareLettersCanvasScopeAndTType()
{
    // KEY-LAYER slice: suppression is FOCUS-SCOPED, not global — and T
    // is the TYPE letter (Appendix C, verbatim: "tool.type" = "T";
    // transform has NO letter there).
    freshDoc(); // canvas focused
    // Known start (newDocument does not reset the tool): brush.stroke
    // differs from every target below — each transition is real.
    m_canvas->setProperty("tool", QStringLiteral("brush.stroke"));
    QCOMPARE(m_canvas->property("tool").toString(),
             QStringLiteral("brush.stroke"));
    const int histBefore = historyCount();

    // (c) canvas-focused bare M still switches.
    QTest::keyClick(m_window, Qt::Key_M);
    QTRY_COMPARE(m_canvas->property("tool").toString(),
                 QStringLiteral("select.rect"));

    // (d) 1.0 final (corrigendum (l)): the Text tool is removed, so bare T
    // does NOTHING: the tool stays, and no transform frame arms.
    QTest::keyClick(m_window, Qt::Key_T);
    QTest::qWait(50);
    QCOMPARE(m_canvas->property("tool").toString(),
             QStringLiteral("select.rect"));
    QVERIFY(!m_canvas->property("transformActive").toBool());

    // (e) transform's surviving keyboard path: Ctrl+T (Appendix C
    // edit.free_transform) enters transform.free with a live frame
    // (cmdTEntersTransformFrame pins the full re-arm semantics).
    QTest::keyClick(m_window, Qt::Key_T, Qt::ControlModifier);
    QTRY_COMPARE(m_canvas->property("tool").toString(),
                 QStringLiteral("transform.free"));
    QVERIFY(m_canvas->property("transformActive").toBool());
    QTest::keyClick(m_window, Qt::Key_Escape);
    QTRY_VERIFY(!m_canvas->property("transformActive").toBool());

    // Tool selection is application state — never a document command.
    QCOMPARE(historyCount(), histBefore);
}

void TstChordTable::guardHoldsOnANoYieldSurface()
{
    // T7" — the key-layer review's MINOR-2 fold-in, made permanent.
    // The suppression evidence in this suite otherwise rests on
    // surfaces that ALSO claim plain letters themselves (a focused
    // TextInput accepts ShortcutOverride on this Qt), so a stripped
    // guard would ride the platform's yield unseen. This surface claims
    // NOTHING: a bare Item that merely EXPOSES inputMethodComposing —
    // exactly the class of the future on-canvas Type-tool editing
    // session (ES:16452's second arm, whose real surface does not exist
    // yet: submitText commits empty text). If the guard is stripped,
    // the letters fire through the REAL dispatch path and this reds.
    freshDoc();
    setToolDirect("brush.stroke");
    QCOMPARE(toolNow(), QStringLiteral("brush.stroke"));
    QQmlComponent probeComp(m_engine);
    probeComp.setData(QByteArrayLiteral(
                          "import QtQuick\n"
                          "Item { width: 1; height: 1\n"
                          "       property bool inputMethodComposing: false }"),
                      QUrl());
    QVERIFY2(probeComp.isReady(), qPrintable(probeComp.errorString()));
    auto *probe = qobject_cast<QQuickItem *>(
        probeComp.create(qmlContext(m_window)));
    QVERIFY(probe);
    probe->setParentItem(m_window->contentItem());
    probe->forceActiveFocus();
    QTRY_VERIFY(probe->hasActiveFocus());
    // The guard ENGAGES by TYPE (duck-typed on the property), not by
    // pane or by widget class.
    QVERIFY2(evalJs(QStringLiteral("root.textEditingFocus")).toBool(),
             "the ES:16452 guard did not engage on a surface exposing "
             "inputMethodComposing");
    const int histBefore = historyCount();
    QTest::keyClick(m_window, Qt::Key_M);
    QCOMPARE(toolNow(), QStringLiteral("brush.stroke"));
    QTest::keyClick(m_window, Qt::Key_M, Qt::ShiftModifier);
    QCOMPARE(toolNow(), QStringLiteral("brush.stroke"));
    QTest::keyClick(m_window, Qt::Key_T);
    QCOMPARE(toolNow(), QStringLiteral("brush.stroke"));
    QVERIFY(!m_canvas->property("transformActive").toBool());
    QTest::keyClick(m_window, Qt::Key_X); // the action chord joins the gate
    QCOMPARE(toolNow(), QStringLiteral("brush.stroke"));
    // Modifier chords still dispatch (ES:16452 / §30.2) — the guard
    // covers single-key and Shift+letter only.
    QTest::keyClick(m_window, Qt::Key_T, Qt::ControlModifier);
    QTRY_COMPARE(toolNow(), QStringLiteral("transform.free"));
    QVERIFY(m_canvas->property("transformActive").toBool());
    QCOMPARE(historyCount(), histBefore); // tool state is never a command
    focusCanvas();
    QTest::keyClick(m_window, Qt::Key_Escape);
    QTRY_VERIFY(!m_canvas->property("transformActive").toBool());
    delete probe;
}

void TstChordTable::chordsStillDispatchFromAFocusedSlider()
{
    // T7' — NO OVER-SUPPRESSION (reviewer probe P1). A focused Slider
    // is neither a text nor a numeric FIELD: nothing there inserts
    // text, so ES:16452 does not reach it and the chords must still
    // dispatch. Discriminator: a guard keyed on "focus is in a pane"
    // instead of on editor TYPE would swallow these.
    freshDoc();
    setToolDirect("brush.stroke"); // the brush options bar carries the slider
    QCoreApplication::processEvents();
    std::function<QQuickItem *(QQuickItem *)> findSlider =
        [&](QQuickItem *node) -> QQuickItem * {
        const auto children = node->childItems();
        for (QQuickItem *c : children) {
            if (c->isVisible() && c->inherits("QQuickSlider"))
                return c;
            if (QQuickItem *hit = findSlider(c))
                return hit;
        }
        return nullptr;
    };
    QQuickItem *slider = nullptr;
    QTRY_VERIFY2((slider = findSlider(m_window->contentItem())) != nullptr,
                 "no visible Slider in the brush options bar");
    slider->forceActiveFocus();
    QTRY_VERIFY(slider->hasActiveFocus());
    QVERIFY2(!evalJs(QStringLiteral("root.textEditingFocus")).toBool(),
             "the ES:16452 guard engaged on a SLIDER — over-suppression");
    QTest::keyClick(m_window, Qt::Key_M);
    QTRY_COMPARE(toolNow(), QStringLiteral("select.rect"));
    focusCanvas();
}

void TstChordTable::openPopupKeepsItsOwnLetters()
{
    // RISK R-A, pinned: the dispatcher must not become a KEY HOOVER.
    // A focused control that legitimately claims plain letters must
    // keep them — an OPEN ComboBox popup's type-ahead is the measured
    // case (the same behaviour the review probes recorded: type-ahead
    // works while the popup is open and is bypassed while it is
    // closed). The dispatcher re-runs Qt's own ShortcutOverride
    // arbitration for exactly this reason.
    freshDoc();
    setToolDirect("brush.stroke");

    // ---- LEG A: the arbitration itself, ISOLATED ----
    // An item that CLAIMS the key through ShortcutOverride and exposes
    // NO inputMethodComposing property: the ES:16452 guard cannot be
    // what yields here, so this leg measures the override probe and
    // nothing else. Discriminator: drop the probe from the dispatcher
    // and this reds while every other case stays green.
    QQmlComponent claimComp(m_engine);
    claimComp.setData(QByteArrayLiteral(
                          "import QtQuick\n"
                          "Item { width: 1; height: 1\n"
                          "  Keys.onShortcutOverride: (event) => "
                          "{ event.accepted = true } }"),
                      QUrl());
    QVERIFY2(claimComp.isReady(), qPrintable(claimComp.errorString()));
    auto *claimer = qobject_cast<QQuickItem *>(
        claimComp.create(qmlContext(m_window)));
    QVERIFY(claimer);
    claimer->setParentItem(m_window->contentItem());
    claimer->forceActiveFocus();
    QTRY_VERIFY(claimer->hasActiveFocus());
    QVERIFY2(!evalJs(QStringLiteral("root.textEditingFocus")).toBool(),
             "the claimer probe exposes inputMethodComposing — it would "
             "measure the guard, not the override arbitration");
    QTest::keyClick(m_window, Qt::Key_M);
    QTest::qWait(30);
    QCOMPARE(toolNow(), QStringLiteral("brush.stroke"));
    delete claimer;
    focusCanvas();
    QTest::keyClick(m_window, Qt::Key_M); // the same key still dispatches
    QTRY_COMPARE(toolNow(), QStringLiteral("select.rect"));
    setToolDirect("brush.stroke");

    // ---- LEG B: the real class, with the measured reason recorded ----
    // A REAL QQuickComboBox, built here rather than borrowed from a
    // dialog: the shipped dialogs' combos are wired to context
    // properties this harness does not carry, and the claim under test
    // is about the CLASS (a focused control whose own letter handling
    // must win), not about any one dialog.
    // MEASURED, and worth knowing: a focused QQuickComboBox makes
    // textEditingFocus TRUE whether or not it is editable, because
    // QQuickComboBox itself exposes inputMethodComposing (it can be
    // editable). So for combos the ES:16452 guard is what yields — and
    // it yields WITHOUT consuming, which is why the type-ahead below
    // still runs. Same outcome, different door; recorded rather than
    // assumed.
    QQmlComponent comboComp(m_engine);
    comboComp.setData(QByteArrayLiteral(
                          "import QtQuick\n"
                          "import QtQuick.Controls\n"
                          "ComboBox { width: 120; height: 28\n"
                          "  model: [\"alpha\", \"bravo\", \"charlie\","
                          " \"delta\"] }"),
                      QUrl());
    QVERIFY2(comboComp.isReady(), qPrintable(comboComp.errorString()));
    auto *box = qobject_cast<QQuickItem *>(comboComp.create(qmlContext(m_window)));
    QVERIFY(box);
    box->setParentItem(m_window->contentItem());
    box->setProperty("currentIndex", 0);
    box->forceActiveFocus();
    QTRY_VERIFY(box->hasActiveFocus());
    QObject *popup = box->property("popup").value<QObject *>();
    QVERIFY(popup);
    QMetaObject::invokeMethod(popup, "open");
    QTRY_VERIFY(popup->property("opened").toBool());
    // Non-vacuity (RULE 9): the ES:16452 guard is NOT what holds the
    // letter back here. If it were, this case would prove nothing about
    // the override arbitration.
    QQuickItem *fi = m_window->activeFocusItem();
    qInfo() << "popup-open focus item:" << (fi ? fi->metaObject()->className()
                                               : "none")
            << "textEditingFocus="
            << evalJs(QStringLiteral("root.textEditingFocus")).toBool();
    const int indexBefore = box->property("currentIndex").toInt();
    const QString toolBefore = toolNow();
    // C is a ruled letter (crop). With the popup open it belongs to the
    // combo's own type-ahead, not to the keymap. Type-ahead moves the
    // HIGHLIGHT while the popup is open and Return commits it — the
    // same two-key shape the review probes recorded on the resize
    // dialog's unit combo.
    QTest::keyClick(m_window, Qt::Key_C);
    QTest::qWait(30);
    QCOMPARE(toolNow(), toolBefore); // no tool switch rode along
    QTest::keyClick(m_window, Qt::Key_Return);
    QTRY_COMPARE(box->property("currentText").toString(),
                 QStringLiteral("charlie"));
    QVERIFY2(box->property("currentIndex").toInt() != indexBefore,
             "the popup's own type-ahead never ran — the dispatcher "
             "swallowed a letter the focused item had claimed (R-A: it "
             "became a key hoover)");
    QCOMPARE(toolNow(), toolBefore);
    if (popup->property("visible").toBool())
        QMetaObject::invokeMethod(popup, "close");
    QTRY_VERIFY(!popup->property("visible").toBool());
    delete box;
    focusCanvas();
}

void TstChordTable::chordMidDragCannotKillTheGesture()
{
    // T8 — R3, ES:16446 [NORMATIVE] closing rule: "during pointer
    // capture, key state routes to the capturing tool and never
    // switches tools". This is the LIVE PRE-EXISTING VIOLATION the
    // migration fixes: before it, a letter mid-drag reached setTool,
    // whose non-same-tool arm calls cancelGesture() — the marquee died
    // under the user's hand. Discriminator: remove the gate and the
    // gesture dies here.
    freshDoc();
    setToolDirect("select.rect");
    focusCanvas();
    const int histBefore = historyCount();
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(50, 40));
    QTest::mouseMove(m_window, docToView(110, 80));
    QVERIFY2(canvasView() && canvasView()->gestureInFlight(),
             "no gesture in flight — the mid-drag case would be vacuous");
    QTest::keyClick(m_window, Qt::Key_B);     // bare letter mid-drag
    QTest::keyClick(m_window, Qt::Key_B, Qt::ShiftModifier);
    QCOMPARE(toolNow(), QStringLiteral("select.rect"));
    QVERIFY2(canvasView()->gestureInFlight(),
             "the letter chord killed the in-flight gesture (R3)");
    QTest::mouseMove(m_window, docToView(150, 120));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(150, 120));
    QTRY_VERIFY2(m_controller->property("selectionActive").toBool(),
                 "the marquee did not commit after a mid-drag letter");
    QCOMPARE(historyCount(), histBefore + 1);
    m_controller->applyCommand(QStringLiteral("selection.none"), {}, {},
                               QString());

    // ---- THE MEASURED HOLE, PINNED AS MEASURED (not as wanted) ----
    // gestureInFlight() is BUTTON-HELD state and deliberately EXCLUDES
    // click-SESSIONS (polygon lasso, pen): they hold no button, so
    // there is no release for a popup grab to steal — the contract it
    // was built for. Consequence for R3: a letter pressed MID-POLYGON
    // is NOT gated, reaches setTool, and cancelGesture() drops the
    // in-progress polygon. NOT silently widened here (:16446 is worded
    // "during pointer capture", which a click-session is not; widening
    // gestureInFlight itself would also change the context menu's
    // unrelated refuse-mid-gesture contract; and click-sessions run
    // until Enter/Esc, so gating every letter for their unbounded
    // duration is its own keyboard-reach cost). Routed to the desk.
    // This leg pins the BEHAVIOUR so the hole cannot drift unnoticed
    // and cannot be rediscovered later and called a regression.
    setToolDirect("select.polygon");
    focusCanvas();
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(40, 40));
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(140, 40));
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(140, 120));
    QVERIFY2(!canvasView()->gestureInFlight(),
             "gestureInFlight() now covers click-sessions — the R3 "
             "scope limit recorded in the dispatcher changed; re-read "
             "that record before touching this pin");
    const int histPoly = historyCount();
    QTest::keyClick(m_window, Qt::Key_B);
    QCOMPARE(toolNow(), QStringLiteral("brush.stroke")); // NOT gated
    // ...and the session is gone: re-arming the tool and pressing
    // Enter commits nothing.
    setToolDirect("select.polygon");
    focusCanvas();
    QTest::keyClick(m_window, Qt::Key_Return);
    QTest::qWait(30);
    QCOMPARE(historyCount(), histPoly);
    QVERIFY(!m_controller->property("selectionActive").toBool());
    // Positive control: the SAME click-session with no letter pressed
    // does commit — so the leg above measured the letter, not a broken
    // polygon path.
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(40, 40));
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(140, 40));
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(140, 120));
    QTest::keyClick(m_window, Qt::Key_Return);
    QTRY_VERIFY2(m_controller->property("selectionActive").toBool(),
                 "the polygon click-session cannot commit at all — the "
                 "hole leg above proves nothing");
    m_controller->applyCommand(QStringLiteral("selection.none"), {}, {},
                               QString());
}

void TstChordTable::springLoadTapLatchesAndHoldReverts()
{
    // T9 + T10 — ES:16446 [NORMATIVE]: "holding a tool letter >= 300 ms
    // temporarily switches (revert on release); a tap < 300 ms latches
    // the tool." Discriminators: a tap treated as a spring (reverts),
    // or a hold that never reverts.
    freshDoc();
    setToolDirect("brush.stroke");
    focusCanvas();
    const int histBefore = historyCount();

    // TAP: press+release with no wait — latches.
    QTest::keyClick(m_window, Qt::Key_M);
    QTRY_COMPARE(toolNow(), QStringLiteral("select.rect"));
    QTest::qWait(50);
    QCOMPARE(toolNow(), QStringLiteral("select.rect")); // still latched

    // HOLD: the switch is immediate on press (instant response), and
    // the PRIOR tool comes back on release.
    setToolDirect("brush.stroke");
    QTest::keyPress(m_window, Qt::Key_M);
    QTRY_COMPARE(toolNow(), QStringLiteral("select.rect"));
    QTest::qWait(ToolKeyDispatcher::SpringHoldMs + 120);
    QCOMPARE(toolNow(), QStringLiteral("select.rect")); // temporary, still on
    QTest::keyRelease(m_window, Qt::Key_M);
    QTRY_COMPARE(toolNow(), QStringLiteral("brush.stroke"));

    // A hold that lands on the ring's own current member springs back
    // the same way (the cycle arm is not exempt).
    setToolDirect("select.rect");
    QTest::keyPress(m_window, Qt::Key_M);
    QTRY_COMPARE(toolNow(), QStringLiteral("select.ellipse")); // stepped
    QTest::qWait(ToolKeyDispatcher::SpringHoldMs + 120);
    QTest::keyRelease(m_window, Qt::Key_M);
    QTRY_COMPARE(toolNow(), QStringLiteral("select.rect"));
    QCOMPARE(historyCount(), histBefore); // never a document command
}

void TstChordTable::autoRepeatDoesNotStepTheRing()
{
    // T11 — ES:16453 grants OS auto-repeat to nudges and steppers; a
    // held letter must not machine-gun the ring (it would also fight
    // the spring arm). Discriminator: isAutoRepeat not filtered ->
    // the ring walks while the key is held.
    // COUNTING NOTE, learned the hard way (mutation M5 first-try
    // PASSED and the case had to be fixed, not the finding explained
    // away): SIX repeats on the two-member marquee ring walk right back
    // to where they started, so the assert was blind to a machine-gun.
    // Both legs below now step a number of times that CANNOT alias to
    // zero on their ring.
    freshDoc();
    setToolDirect("brush.stroke");
    focusCanvas();
    const auto repeatKey = [&](Qt::Key key, int times) {
        for (int i = 0; i < times; ++i) {
            QKeyEvent repeat(QEvent::KeyPress, key, Qt::NoModifier,
                             QString(QChar(ushort(key))).toLower(), true, 1);
            QCoreApplication::sendEvent(m_window, &repeat);
        }
        QCoreApplication::processEvents();
    };
    // Leg 1: the five-member shape ring, three repeats (3 mod 5 = 3 —
    // an unfiltered repeat would land on shape.polygon).
    QTest::keyPress(m_window, Qt::Key_U);
    QTRY_COMPARE(toolNow(), QStringLiteral("shape.rect"));
    repeatKey(Qt::Key_U, 3);
    QCOMPARE(toolNow(), QStringLiteral("shape.rect")); // exactly one step
    QTest::keyRelease(m_window, Qt::Key_U);
    QCOMPARE(toolNow(), QStringLiteral("shape.rect")); // tap: latched
    // Leg 2: the two-member marquee ring, an ODD number of repeats.
    setToolDirect("select.rect");
    setToolDirect("brush.stroke");
    QTest::keyPress(m_window, Qt::Key_M);
    QTRY_COMPARE(toolNow(), QStringLiteral("select.rect"));
    repeatKey(Qt::Key_M, 5);
    QCOMPARE(toolNow(), QStringLiteral("select.rect"));
    QTest::keyRelease(m_window, Qt::Key_M);
    QCOMPARE(toolNow(), QStringLiteral("select.rect"));
}

void TstChordTable::sameToolReSelectStillReArms()
{
    // T12 — setTool's same-tool arm RE-ARMS an inactive transform/
    // perspective/puppet session (review A11Y-1 MAJOR-2). No lettered
    // tool is session-shaped this wave, so this pins the semantics at
    // the funnel the dispatcher writes through: the dispatch path must
    // not "optimise away" a same-tool write. Discriminator: a
    // dispatcher (or funnel) that skips setTool on same-tool.
    freshDoc();
    focusCanvas();
    evalJs(QStringLiteral("root.dispatchToolChord(\"transform.free\")"));
    QTRY_COMPARE(toolNow(), QStringLiteral("transform.free"));
    QVERIFY(m_canvas->property("transformActive").toBool());
    QTest::keyClick(m_window, Qt::Key_Escape); // frame closes, tool stays
    QTRY_VERIFY(!m_canvas->property("transformActive").toBool());
    QCOMPARE(toolNow(), QStringLiteral("transform.free"));
    const int histBefore = historyCount();
    evalJs(QStringLiteral("root.dispatchToolChord(\"transform.free\")"));
    QTRY_VERIFY2(m_canvas->property("transformActive").toBool(),
                 "a same-tool re-select through the dispatch path did "
                 "NOT re-arm the inactive frame");
    QCOMPARE(historyCount(), histBefore);
    QTest::keyClick(m_window, Qt::Key_Escape);
    QTRY_VERIFY(!m_canvas->property("transformActive").toBool());
}

void TstChordTable::cloneChordSpeaksItsMissingSource()
{
    // ★★★★★ Q-D14 = VOICE (operator, 2026-08-30): "S stays bound and SPEAKS:
    // the toast names why clone paints nothing ... Silent tools are the
    // disease this week cured; a bound key that does nothing quietly is not
    // an option."
    //
    // ★★★★ THIS TEST EXISTS BECAUSE THE DEFECT IT PINS IS INVISIBLE TO EVERY
    // OTHER CHECK. A tool that selects and then says nothing throws nothing,
    // exits zero, and renders a perfectly ordinary screen -- G-530's family.
    // The only thing that distinguishes "bound and mute" from "bound and
    // speaking" is the status line, so the status line is what gets asserted.
    //
    // ★★★ BOTH DIRECTIONS. The must-PASS half is that S produces the
    // cataloged sentence. The must-DISCRIMINATE half is that a DIFFERENT
    // lettered tool does NOT produce it -- without that, an assertion that
    // merely finds non-empty status would pass on any tool whatsoever and
    // pin nothing (G-541).
    freshDoc();
    focusCanvas();

    QTest::keyClick(m_window, Qt::Key_S);
    QTRY_COMPARE(toolNow(), QStringLiteral("clone.stroke"));
    // Q-D21 RULED (a): the id carries a {tool} placeholder now, so a BARE
    // tr() would build an expectation containing the literal `{tool}` and
    // compare it against a correctly-substituted status line -- the test
    // would fail for the one reason that is not a defect. Build the
    // expectation the way the shell builds the string.
    const QString want =
        m_i18n->tr(QLatin1String("ui.status.clone_needs_source"),
                   {{QStringLiteral("tool"),
                     m_i18n->tr(QLatin1String("ui.tool.clone"))}});
    QVERIFY2(!want.contains(QLatin1String("{tool}")),
             "vacuity: the placeholder did not substitute, so this asserts a "
             "template against a rendered string and can only fail");
    QVERIFY2(!want.isEmpty(),
             "vacuity: ui.status.clone_needs_source resolved to nothing, so "
             "the assertion below would compare empty against empty");
    QTRY_COMPARE(m_controller->property("statusText").toString(), want);

    // ★★★★★ THE DISCRIMINATOR, AND ITS FIRST VERSION WAS WRONG IN A WAY MY
    // OWN ORDERS NAME. I first asserted that after selecting the brush the
    // status line no longer READS clone's sentence. It fired -- and the
    // defect was in the assertion, not the code: this app's status line is a
    // LAST-EVENT LOG, and persistence is its normal, shipped behaviour (the
    // tour frames show "Applied: Add Mask" surviving into later steps all
    // week). ★★★★ "The speak is conditioned on the tool" is a claim about an
    // EMISSION, not about a state -- G-537 exactly: a state cannot
    // distinguish "said once and went quiet" from "still saying it", and the
    // SIGNAL can. So the discriminator counts emissions.
    QSignalSpy statusSpy(m_controller,
                         &DocumentController::statusTextChanged);
    QTest::keyClick(m_window, Qt::Key_B);
    QTRY_COMPARE(toolNow(), QStringLiteral("brush.stroke"));
    QCOMPARE(statusSpy.count(), 0);
}

void TstChordTable::chordAnnouncementsKeyOnInputSource()
{
    // T13/T14/T15/T16 — desk ruling (e): the announcement exemption
    // keys on INPUT SOURCE, never on focus. Zero new ids: the member's
    // own cataloged name through the registered tool-selection
    // template.
    // WHAT THESE LEGS ASSERT, precisely: that the announcement FIRES
    // (or stays suppressed) and carries the right cataloged name. They
    // do NOT assert keyboard-scripts rule 7's 700 ms observable bound —
    // that latency is the KS harness's measurement (input event -> the
    // announcer's JSONL row), and asserting it on a shared build
    // machine turns a correctness pin into a load test. Measured
    // first-hand: this case went red at 700 ms under a concurrent
    // build ("850 ms would have been sufficient") while the behaviour
    // was correct. The rate-limit mechanism itself IS pinned, in T16.
    freshDoc();
    QSignalSpy spy(m_announcer, &Announcer::announced);
    QVERIFY(spy.isValid());

    // T13: chord with the CANVAS focused -> announces the member name.
    setToolDirect("brush.stroke");
    focusCanvas();
    drainAnnouncer(spy);
    QTest::keyClick(m_window, Qt::Key_M);
    QTRY_COMPARE(toolNow(), QStringLiteral("select.rect"));
    bool heard = false;
    QTRY_VERIFY_WITH_TIMEOUT(
        [&]() {
            for (const auto &row : spy)
                if (row.at(1).toString() == QLatin1String("a11y.tool.selected")
                    && row.at(2).toString() == catTr("ui.tool.select_rect"))
                    heard = true;
            return heard;
        }(),
        2000);
    QVERIFY2(heard, "a canvas-focused chord announced nothing");

    // T14: THE CLOSED HOLE — the same chord with the PALETTE PANE
    // holding focus still announces. Before ruling (e) this was
    // silence: the guard suppressed it and no focus moved, so nothing
    // spoke at all. Discriminator: guard checks focus only -> silent.
    setToolDirect("brush.stroke");
    focusCanvas();
    QTest::keyClick(m_window, Qt::Key_F6, Qt::ShiftModifier);
    QTRY_VERIFY2(evalJs(QStringLiteral("root.focusInsideToolbar()")).toBool(),
                 "Shift+F6 from the canvas did not land focus inside a "
                 "tool surface — T14 would be vacuous");
    drainAnnouncer(spy);
    QTest::keyClick(m_window, Qt::Key_M);
    QTRY_COMPARE(toolNow(), QStringLiteral("select.rect"));
    heard = false;
    QTRY_VERIFY_WITH_TIMEOUT(
        [&]() {
            for (const auto &row : spy)
                if (row.at(1).toString() == QLatin1String("a11y.tool.selected")
                    && row.at(2).toString() == catTr("ui.tool.select_rect"))
                    heard = true;
            return heard;
        }(),
        2000);
    QVERIFY2(heard,
             "a chord fired while the palette pane held focus was "
             "SILENT — ruling (e)'s hole is open again");

    // T15: the exemption is not over-broad. Focus-driven activation
    // from the SAME pane stays suppressed (the platform speaks the
    // focused control itself; the live region would double-speak).
    QTRY_VERIFY(evalJs(QStringLiteral("root.focusInsideToolbar()")).toBool());
    QQuickItem *focused = m_window->activeFocusItem();
    QVERIFY(focused);
    const QString face = focused->property("faceTool").toMap()
                             .value(QStringLiteral("tool")).toString();
    QVERIFY2(!face.isEmpty() && face != toolNow(),
             "the focused slot already faces the active tool — the "
             "suppression leg would be vacuous");
    drainAnnouncer(spy);
    QTest::keyClick(m_window, Qt::Key_Space);
    QTRY_COMPARE(toolNow(), face);
    QTest::qWait(700);
    for (const auto &row : spy)
        QVERIFY2(row.at(1).toString() != QLatin1String("a11y.tool.selected"),
                 "focus-driven activation announced — the input-source "
                 "exemption was over-broadened to every source");

    // T14' (11e fix round, §5 MAJOR-2): the OTHER keymap chord that
    // writes a tool. Ctrl+T is keymap dispatch, so ruling (e) covers it
    // exactly as it covers a letter — "chord-driven changes announce
    // regardless of focused pane". It used to write canvas.tool
    // directly, never set the input-source mark, and therefore went
    // SILENT with focus inside a tool pane: measured
    // focusInsideToolbar=true, tool=transform.free, announcements=0.
    // Routing it through the funnel closed that. Discriminator: revert
    // the routing and this reds with the silence signature.
    setToolDirect("brush.stroke");
    focusCanvas();
    QTest::keyClick(m_window, Qt::Key_F6, Qt::ShiftModifier);
    QTRY_VERIFY2(evalJs(QStringLiteral("root.focusInsideToolbar()")).toBool(),
                 "Shift+F6 did not land focus inside a tool surface — "
                 "the Ctrl+T leg would be vacuous");
    QVERIFY2(m_controller->property("commandAvailability").toMap()
                 .value(QStringLiteral("transform.enter")).toMap()
                 .value(QStringLiteral("enabled")).toBool(),
             "the transform.enter row is disabled — Ctrl+T would be a "
             "no-op and this leg would prove nothing");
    drainAnnouncer(spy);
    QTest::keyClick(m_window, Qt::Key_T, Qt::ControlModifier);
    QTRY_COMPARE(toolNow(), QStringLiteral("transform.free"));
    heard = false;
    QTRY_VERIFY_WITH_TIMEOUT(
        [&]() {
            for (const auto &row : spy)
                if (row.at(1).toString() == QLatin1String("a11y.tool.selected")
                    && row.at(2).toString() == catTr("ui.tool.transform"))
                    heard = true;
            return heard;
        }(),
        2000);
    QVERIFY2(heard,
             "Ctrl+T with a tool pane focused announced NOTHING — the "
             "ruling-(e) silence hole is open on a shipping chord");
    focusCanvas();
    QTest::keyClick(m_window, Qt::Key_Escape); // drop the frame
    QTRY_VERIFY(!m_canvas->property("transformActive").toBool());

    // T16: a cycling BURST coalesces latest-wins (§31.3's 500 ms rule)
    // instead of narrating every step. Five taps: activate rect, then
    // ellipse/rect/ellipse/rect.
    setToolDirect("select.rect"); // ring memory := rect
    setToolDirect("brush.stroke");
    focusCanvas();
    drainAnnouncer(spy);
    for (int i = 0; i < 5; ++i)
        QTest::keyClick(m_window, Qt::Key_M);
    QCOMPARE(toolNow(), QStringLiteral("select.rect"));
    QTest::qWait(1200); // let the coalesced flush land
    int toolRows = 0;
    QString lastText;
    for (const auto &row : spy) {
        if (row.at(1).toString() == QLatin1String("a11y.tool.selected")) {
            ++toolRows;
            lastText = row.at(2).toString();
        }
    }
    QVERIFY2(toolRows > 0, "a five-tap burst announced nothing at all");
    QVERIFY2(toolRows < 5,
             qPrintable(QStringLiteral("no coalescing: %1 announcements "
                                       "for 5 taps")
                            .arg(toolRows)));
    QCOMPARE(lastText, catTr("ui.tool.select_rect")); // the final member wins
}

void TstChordTable::swapColorsChordSwapsAndAnnouncesTheAction()
{
    // T17 — X, Appendix C tool.swap_colors. BOTH sides of the pair
    // actually exchange (the MM1 value-copy trap: a lazy value-type
    // reference collapses both onto bg), the ACTION is announced
    // through EXISTING ids (ruling (e): zero new ids), and per-color
    // NAMES stay a recorded wall — the shell has no color-name source.
    freshDoc();
    focusCanvas();
    evalJs(QStringLiteral(
        "canvas.brushColor = Qt.rgba(0.2, 0.4, 0.6, 1);"));
    evalJs(QStringLiteral(
        "palettePane.swatchBackgroundColor = Qt.rgba(0.9, 0.8, 0.7, 1);"));
    const QColor fgBefore = m_canvas->property("brushColor").value<QColor>();
    const QColor bgBefore =
        evalJs(QStringLiteral("palettePane.swatchBackgroundColor"))
            .value<QColor>();
    QVERIFY(fgBefore.isValid() && bgBefore.isValid()
            && fgBefore != bgBefore);
    QSignalSpy spy(m_announcer, &Announcer::announced);
    QVERIFY(spy.isValid());
    drainAnnouncer(spy);
    const int histBefore = historyCount();

    QTest::keyClick(m_window, Qt::Key_X);

    QTRY_COMPARE(m_canvas->property("brushColor").value<QColor>(), bgBefore);
    QCOMPARE(evalJs(QStringLiteral("palettePane.swatchBackgroundColor"))
                 .value<QColor>(),
             fgBefore);
    // A pure shell-state exchange: no envelope, no journal entry.
    QCOMPARE(historyCount(), histBefore);
    // The announcement carries the cataloged ACTION name through the
    // registered template, on its OWN source so a swap never coalesces
    // away a tool announcement.
    bool heard = false;
    QTRY_VERIFY_WITH_TIMEOUT(
        [&]() {
            for (const auto &row : spy)
                if (row.at(0).toString() == QLatin1String("color")
                    && row.at(1).toString() == QLatin1String("a11y.tool.selected")
                    && row.at(2).toString() == catTr("ui.tool.swatch_swap"))
                    heard = true;
            return heard;
        }(),
        2000);
    QVERIFY2(heard, "the X swap chord announced nothing");
    // The spoken text is the cataloged name itself — checked against
    // the checked-in catalog value, not re-derived through the same tr.
    QCOMPARE(catTr("ui.tool.swatch_swap"),
             QStringLiteral("Swap Foreground and Background Colors"));
    // Shift+X stays UNBOUND (OQ-4): it must not swap back.
    const QColor fgAfter = m_canvas->property("brushColor").value<QColor>();
    QTest::keyClick(m_window, Qt::Key_X, Qt::ShiftModifier);
    QTest::qWait(30);
    QCOMPARE(m_canvas->property("brushColor").value<QColor>(), fgAfter);
    // The chord and the visible control are ONE behaviour (the chord
    // rides a named, AT-discoverable control): the palette's swap
    // function is what both call.
    evalJs(QStringLiteral("palettePane.swapSwatches()"));
    QTRY_COMPARE(m_canvas->property("brushColor").value<QColor>(), fgBefore);
}

void TstChordTable::ruledChordsSurviveEveryThemeBindingSet()
{
    // THE BINDING SWEEP (standing rule from 14b: re-run the sweep on
    // any new ruled chord) — done MECHANICALLY against RESOLVED
    // per-platform-theme QKeySequence sets, never against portable
    // names. The precedent is measured, not theoretical: some platform
    // themes alias Alt+Backspace onto StandardKey.Undo, which collided
    // with the ruled Alt+Delete/Backspace fill chord — which is why
    // Undo binds a portable row instead of the StandardKey set.
    // Nothing in this suite swept resolved sets before this case.
    QStringList ruled = ToolKeyDispatcher::ruledSequences();
    // 1.0 final (corrigendum (l)): G and T unbound: 12 letters.
    QCOMPARE(ruled.size(), 25); // 12 letters x2 (bare + Shift) + X
    QList<QKeySequence> ruledSeqs;
    for (const QString &s : ruled)
        ruledSeqs << QKeySequence(s);

    // (a) the ruled set is internally collision-free.
    for (int i = 0; i < ruledSeqs.size(); ++i)
        for (int j = i + 1; j < ruledSeqs.size(); ++j)
            QVERIFY2(ruledSeqs.at(i) != ruledSeqs.at(j),
                     qPrintable(QStringLiteral("ruled chords %1 and %2 "
                                               "resolve to the same "
                                               "sequence")
                                    .arg(ruled.at(i), ruled.at(j))));

    // (b) no StandardKey Main.qml binds resolves — on THIS platform
    // theme — onto any ruled sequence.
    struct StdRow {
        const char *name;
        QKeySequence::StandardKey key;
    };
    const StdRow standardRows[] = {
        {"New", QKeySequence::New},
        {"Open", QKeySequence::Open},
        {"Save", QKeySequence::Save},
        {"SaveAs", QKeySequence::SaveAs},
        // RC3 D-3 (operator WORD 35): File > Close binds StandardKey.Close.
        {"Close", QKeySequence::Close},
        {"Redo", QKeySequence::Redo},
        {"Cut", QKeySequence::Cut},
        {"Copy", QKeySequence::Copy},
        {"Paste", QKeySequence::Paste},
        {"SelectAll", QKeySequence::SelectAll},
        {"ZoomIn", QKeySequence::ZoomIn},
        {"ZoomOut", QKeySequence::ZoomOut},
    };
    QFile qml(QFileInfo(QStringLiteral(__FILE__)).dir().filePath(
        QStringLiteral("../qml/Main.qml")));
    QVERIFY2(qml.open(QIODevice::ReadOnly), "Main.qml not readable");
    const QString src = QString::fromUtf8(qml.readAll());
    // The sweep must cover what the shell ACTUALLY binds: every
    // StandardKey in Main.qml has a row above, so a future one reds
    // this case until it is swept too.
    static const QRegularExpression stdRe(
        QStringLiteral("StandardKey\\.([A-Za-z]+)"));
    QSet<QString> namesInQml;
    auto it = stdRe.globalMatch(src);
    while (it.hasNext())
        namesInQml.insert(it.next().captured(1));
    QSet<QString> namesSwept;
    for (const StdRow &row : standardRows)
        namesSwept.insert(QLatin1String(row.name));
    QCOMPARE(namesInQml, namesSwept);
    for (const StdRow &row : standardRows) {
        const QList<QKeySequence> resolved = QKeySequence::keyBindings(row.key);
        for (const QKeySequence &seq : resolved) {
            for (int i = 0; i < ruledSeqs.size(); ++i) {
                QVERIFY2(seq != ruledSeqs.at(i),
                         qPrintable(
                             QStringLiteral("theme alias: StandardKey.%1 "
                                            "resolves to %2, which is the "
                                            "ruled chord %3")
                                 .arg(QLatin1String(row.name),
                                      seq.toString(QKeySequence::PortableText),
                                      ruled.at(i))));
            }
        }
    }

    // (c) no explicitly-bound sequence in Main.qml (the Ctrl table, the
    // pane cycle, the menu-access chords) equals a ruled chord.
    static const QRegularExpression seqRe(
        QStringLiteral("sequence: \"([^\"]+)\""));
    auto sit = seqRe.globalMatch(src);
    int explicitCount = 0;
    while (sit.hasNext()) {
        const QKeySequence bound(sit.next().captured(1));
        ++explicitCount;
        for (int i = 0; i < ruledSeqs.size(); ++i)
            QVERIFY2(bound != ruledSeqs.at(i),
                     qPrintable(QStringLiteral("explicit chord %1 collides "
                                               "with ruled chord %2")
                                    .arg(bound.toString(
                                             QKeySequence::PortableText),
                                         ruled.at(i))));
    }
    QVERIFY2(explicitCount >= 12,
             "the explicit-chord sweep found almost nothing — its "
             "pattern went stale against Main.qml");

    // (d) the menu chord DISPLAY strings (converter-only items) carry
    // no ruled chord either: a display collision would teach a user a
    // chord the menu cannot fire.
    static const QRegularExpression chordRe(
        QStringLiteral("chord: \"([^\"]+)\""));
    auto cit = chordRe.globalMatch(src);
    while (cit.hasNext()) {
        const QKeySequence shown(cit.next().captured(1));
        for (int i = 0; i < ruledSeqs.size(); ++i)
            QVERIFY2(shown != ruledSeqs.at(i),
                     qPrintable(QStringLiteral("menu display chord %1 "
                                               "collides with ruled "
                                               "chord %2")
                                    .arg(shown.toString(
                                             QKeySequence::PortableText),
                                         ruled.at(i))));
    }
}

void TstChordTable::cmdXCutChordCutsAndCopies()
{
    freshDoc();
    fillRect(QRect(80, 60, 100, 80), 0.0, 0.0, 1.0);
    QVERIFY(setRectSelection(QRect(80, 60, 40, 40)));
    QGuiApplication::clipboard()->setImage(QImage()); // clear
    focusCanvas();
    QTest::keyClick(m_window, Qt::Key_X, Qt::ControlModifier);
    // Engine truth: the selected region cleared...
    const QImage img = fullReadback();
    QCOMPARE(img.pixelColor(100, 80).alpha(), 0);   // inside cut
    QCOMPARE(img.pixelColor(150, 100), QColor(0, 0, 255, 255)); // outside
    // ...and the §26.1 app-side copy landed on the OS clipboard.
    const QImage clip = QGuiApplication::clipboard()->image();
    QVERIFY2(!clip.isNull(), "Cmd+X did not copy before cutting");
    QCOMPARE(clip.pixelColor(100, 80), QColor(0, 0, 255, 255));
}

void TstChordTable::chordDisplayRendersInMenuItems()
{
    // The design's "every chord renders in its menu item": the Basic
    // style renders NO shortcut of its own (measured — the build
    // report), so ChordMenuItem draws it. Expected text = Qt's own
    // native rendering of the sequence (RULE 8: derived, not
    // hardcoded).
    const auto native = [](const QKeySequence &seq) {
        return seq.toString(QKeySequence::NativeText);
    };
    // The displayed chords are the design table's PORTABLE strings
    // (Ctrl = macOS Cmd), pinned as strings in Main.qml so the display
    // is theme-independent (a platform theme's extra StandardKey
    // aliases never leak into the label — the Alt+Backspace/Undo
    // collision record).
    QCOMPARE(topMenuChordText("ui.menu.edit.delete"),
             native(QKeySequence(QStringLiteral("Backspace"))));
    QCOMPARE(topMenuChordText("ui.menu.edit.cut"),
             native(QKeySequence(QStringLiteral("Ctrl+X"))));
    QCOMPARE(topMenuChordText("ui.menu.edit.copy"),
             native(QKeySequence(QStringLiteral("Ctrl+C"))));
    QCOMPARE(topMenuChordText("ui.menu.edit.paste"),
             native(QKeySequence(QStringLiteral("Ctrl+V"))));
    QCOMPARE(topMenuChordText("ui.action.undo"),
             native(QKeySequence(QStringLiteral("Ctrl+Z"))));
    QCOMPARE(topMenuChordText("ui.action.redo"),
             native(QKeySequence(QStringLiteral("Ctrl+Shift+Z"))));
    QCOMPARE(topMenuChordText("ui.menu.select.all"),
             native(QKeySequence(QStringLiteral("Ctrl+A"))));
    QCOMPARE(topMenuChordText("ui.menu.select.deselect"),
             native(QKeySequence(QStringLiteral("Ctrl+D"))));
    QCOMPARE(topMenuChordText("ui.menu.select.invert"),
             native(QKeySequence(QStringLiteral("Ctrl+Shift+I"))));
    // None of them rendered empty (the display is real).
    QVERIFY(!topMenuChordText("ui.menu.edit.delete").isEmpty());
}

int main(int argc, char *argv[])
{
    if (qgetenv("QT_QPA_PLATFORM").isEmpty())
        qputenv("QT_QPA_PLATFORM", "offscreen");
    QQuickStyle::setStyle(QStringLiteral("Basic"));
    QGuiApplication app(argc, argv);
    TstChordTable tc;
    QTEST_SET_MAIN_SOURCE_PATH
    return QTest::qExec(&tc, argc, argv);
}
#include "tst_chord_table.moc"
