// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// v1.0.1 item 7 — THE CANVAS CONTEXT MENU NET (the items-7+14 design
// entry, reports/O-UI-log.md). Boots the REAL Main.qml offscreen (the
// tst_palette_pins harness: this binary compiles its own copy of the
// AppnameShell QML module and wires the real controller/announcer like
// shell/src/main.cpp) against a REAL engine document, and proves:
//
//   1. OPEN PATHS (all three REAL events, §31.1): a synthesized
//      right-click press opens the menu AT the pressed canvas point;
//      Shift+F10 and the Menu key open it at the keyboard anchor —
//      the SELECTION's view-space bounds center when one is active
//      (independently recomputed from the engine selection rect +
//      the zoomFit centering contract), else the visible document
//      center.
//   2. SECTION TRUTH: selection-active shows the selection section
//      (Cut/Copy/Fill…/Deselect/Invert/Modify ▸/Save Selection/
//      Filters ▸) and hides the paste section; no-selection shows
//      Paste/Select All + exactly the ACTIVE TOOL's quick actions
//      (the per-tool table in CanvasContextMenu.qml's header) — both
//      states driven through real engine documents. The OMISSION PIN
//      (post-14b state): Paste Into stays row-absent (its handler is
//      the menu-mint follow-up slice's — anti-fake teeth armed);
//      Delete is a ROW-PRESENCE assert (14b shipped the row with its
//      ruled handler); both ids are pinned present in the catalog
//      (mint-regression guard, sealed 3dc5c37).
//   3. ENABLED TRUTH: menu items CONSUME the 14a availability surface
//      — item.enabled == commandAvailability row across flipped
//      document states (three rows probed: select.deselect by
//      selection on/off, edit.fill by active-layer emptiness,
//      select.loadSaved by saved-channel existence). The surface
//      itself is matrix-proven in tst_command_availability; this
//      proves the MENU reads it.
//   4. ACTION IDENTITY (engine readback truth): Deselect fired from
//      the context menu (REAL click on the popup item) lands the same
//      engine end-state as the top-menu twin, undo-reversible the
//      same way; Fill… on a selection driven through the context menu
//      produces a BYTE-IDENTICAL full-document readback to the
//      top-menu path (same dialog instance, same apply door), with
//      the selection clip proven (outside pixel untouched).

#include "announcer.h"
#include "canvas_view.h"
#include "document_controller.h"
#include "i18n_catalog.h"
#include "units.h"
#include "icon_provider.h"
#include "layer_thumb_provider.h"

#include <QClipboard>
#include <QGuiApplication>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQmlExpression>
#include <QQuickItem>
#include <QQuickStyle>
#include <QQuickWindow>
#include <QtTest>

#include <functional>

using namespace appname::shell;

class TstCanvasContextMenu : public QObject {
    Q_OBJECT
private slots:
    void initTestCase();
    void cleanup(); // a failed case must not leak an open popup
    void catalogGroundAndOmissionPin();
    void rightClickOpensAtPressedPoint();
    void shiftF10OpensAtSelectionAnchor();
    void menuKeyOpensAtVisibleDocCenter();
    void sectionTruthAcrossStatesAndTools();
    void enabledTruthConsumesAvailabilitySurface();
    void actionIdentityDeselect();
    void actionIdentityFillOnSelection();
    // item 15 (Paste Into) — the four legs, see the case comments.
    void pasteIntoLandsInsideTheSelectionOnly();
    void pasteIntoHonestGatingWithoutSelection();
    void pasteIntoBothMenusReachTheSameArm();
    void pasteIntoOmitsAtSoTheEngineResolvesPlacement();
    void midDragRightPressIsIgnored();
    void midDragKeyboardChordIsIgnored();
    void dismissalPressIsConsumed();

private:
    CatalogQml *m_i18n = nullptr;
    DocumentController *m_controller = nullptr;
    Announcer *m_announcer = nullptr;
    QQmlApplicationEngine *m_engine = nullptr;
    QQuickWindow *m_window = nullptr;
    QQuickItem *m_canvas = nullptr;

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
    QObject *ctxMenu() const
    {
        return evalJs(QStringLiteral("canvasContextMenu"))
            .value<QObject *>();
    }
    // Item lookup by cataloged text over the context menu INCLUDING
    // submenus (nested Menus surface as wrapper items whose subMenu
    // points at the Menu; wrapper text == the submenu title).
    QQuickItem *ctxItem(const char *catalogKey) const
    {
        const QString code = QStringLiteral(
            "(function() {"
            "  var label = AppI18n.tr(\"%1\");"
            "  function findIn(menu) {"
            "    for (var i = 0; i < menu.count; ++i) {"
            "      var it = menu.itemAt(i);"
            "      if (!it) continue;"
            "      if (it.text === label) return it;"
            "      if (it.subMenu) {"
            "        var sub = findIn(it.subMenu);"
            "        if (sub) return sub;"
            "      }"
            "    }"
            "    return null;"
            "  }"
            "  return findIn(canvasContextMenu);"
            "})()").arg(QLatin1String(catalogKey));
        return qobject_cast<QQuickItem *>(
            evalJs(code).value<QObject *>());
    }
    // How many TOP-LEVEL context-menu items with the given cataloged
    // text are effectively visible RIGHT NOW (call with the popup
    // OPEN — QML `visible` reads effective visibility, so a closed
    // popup hides everything). Disambiguates duplicate texts (Fill…
    // exists once per section).
    int ctxVisibleCount(const char *catalogKey) const
    {
        const QString code = QStringLiteral(
            "(function() {"
            "  var label = AppI18n.tr(\"%1\");"
            "  var n = 0;"
            "  for (var i = 0; i < canvasContextMenu.count; ++i) {"
            "    var it = canvasContextMenu.itemAt(i);"
            "    if (it && it.text === label && it.visible) ++n;"
            "  }"
            "  return n;"
            "})()").arg(QLatin1String(catalogKey));
        return evalJs(code).toInt();
    }
    // Every text the context menu renders, top level + submenus.
    QStringList ctxAllTexts() const
    {
        const QString code = QStringLiteral(
            "(function() {"
            "  var texts = [];"
            "  function walk(menu) {"
            "    for (var i = 0; i < menu.count; ++i) {"
            "      var it = menu.itemAt(i);"
            "      if (!it) continue;"
            "      if (it.text !== undefined && it.text !== \"\")"
            "        texts.push(it.text);"
            "      if (it.subMenu) walk(it.subMenu);"
            "    }"
            "  }"
            "  walk(canvasContextMenu);"
            "  return texts.join(\"\\n\");"
            "})()");
        return evalJs(code).toString().split(QLatin1Char('\n'),
                                             Qt::SkipEmptyParts);
    }
    // The 14a-style top menu-bar lookup (palette-flyout-proof).
    QObject *topMenuItem(const char *catalogKey) const
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
    bool menuVisible() const
    {
        QObject *menu = ctxMenu();
        return menu && menu->property("visible").toBool();
    }
    void closeMenu()
    {
        evalJs(QStringLiteral("canvasContextMenu.close()"));
        QTRY_VERIFY(!menuVisible());
    }
    QVariantMap availRow(const char *action) const
    {
        return m_controller->property("commandAvailability")
            .toMap()
            .value(QLatin1String(action))
            .toMap();
    }
    QVariantMap applyOk(const char *kind, const QVariantMap &params)
    {
        return m_controller->applyCommand(QLatin1String(kind), {}, params,
                                          QString());
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
        return applyOk("selection.set", params)
            .value(QStringLiteral("ok"))
            .toBool();
    }
    // Independent doc->view mapping: reads the live zoom property and
    // applies the zoomFit CENTERING contract (offset = (view - doc *
    // zoom) / 2 — canvas_view.cpp zoomFit; the doc-open reset path).
    // NOT the C++ docToView (that is the code under test's transform).
    QPointF expectedDocToView(double x, double y) const
    {
        const double zoom = m_canvas->property("zoom").toDouble();
        const double dw = m_controller->docWidth();
        const double dh = m_controller->docHeight();
        const double offX = (m_canvas->width() - dw * zoom) / 2.0;
        const double offY = (m_canvas->height() - dh * zoom) / 2.0;
        return QPointF(offX + x * zoom, offY + y * zoom);
    }
    void freshDoc()
    {
        QVERIFY(m_controller->newDocument(320, 200,
                                          QStringLiteral("white")));
        QVERIFY(!m_controller->property("selectionActive").toBool());
    }
    // ---- item-15 helpers ----
    QRect fullDocRect() const
    {
        return QRect(0, 0, m_controller->docWidth(),
                     m_controller->docHeight());
    }
    QImage fullReadback()
    {
        QImage img;
        if (!m_controller->readRegionImage(fullDocRect(), &img))
            return QImage();
        return img;
    }
    int historyCount() const
    {
        return m_controller->property("historyEntries").toList().size();
    }
    QString lastHistoryKind() const
    {
        const QVariantList h =
            m_controller->property("historyEntries").toList();
        return h.isEmpty()
                   ? QString()
                   : h.last().toMap().value(QStringLiteral("kind")).toString();
    }
    // A KNOWN raster on the OS clipboard: opaque, uniformly `c`, and
    // FULL-DOCUMENT sized so the engine's canvas-centred `at`
    // resolution covers every document pixel. That is what makes the
    // containment assertion discriminating — the pasted CONTENT reaches
    // everywhere, so anything that shows outside the selection can only
    // be a missing mask. (The offscreen QPA clipboard round-trips
    // images in-process; the §26.1 Cut net does the same.)
    void putFullDocClipboardImage(const QColor &c)
    {
        putClipboardImage(m_controller->docWidth(),
                          m_controller->docHeight(), c);
    }
    // The sized form (one implementation, per the house rule). The
    // `at`-omission pin needs a raster SMALLER than the document,
    // because the engine's canvas-centred placement is only observable
    // when the pasted footprint has edges inside the document.
    void putClipboardImage(int w, int h, const QColor &c)
    {
        QImage img(w, h, QImage::Format_RGBA8888);
        img.fill(c);
        QGuiApplication::clipboard()->setImage(img);
        QVERIFY(!QGuiApplication::clipboard()->image().isNull());
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
    // The unique VISIBLE button rendering the given cataloged text
    // (dialog buttons live under the overlay — closed dialogs are
    // invisible, so the open dialog's button is unique).
    QQuickItem *uniqueVisibleButton(const char *catalogKey) const
    {
        const QString label = catTr(catalogKey);
        QList<QQuickItem *> hits;
        std::function<void(QQuickItem *)> walk =
            [&](QQuickItem *node) {
                const auto children = node->childItems();
                for (QQuickItem *c : children) {
                    if (c->isVisible()
                        && c->inherits("QQuickAbstractButton")
                        && c->property("text").toString() == label)
                        hits << c;
                    walk(c);
                }
            };
        walk(m_window->contentItem());
        if (hits.size() != 1) {
            qWarning() << "uniqueVisibleButton: " << hits.size()
                       << "visible buttons render" << label;
            return nullptr;
        }
        return hits.first();
    }
};

void TstCanvasContextMenu::initTestCase()
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

    // From the WINDOW object — QML-created items hang off the root
    // object's QObject tree, not contentItem's (the palette_pins
    // finding).
    m_canvas = m_window->findChild<QQuickItem *>(
        QStringLiteral("canvasView"));
    QVERIFY2(m_canvas, "canvasView not found");
    QVERIFY(ctxMenu());
}

void TstCanvasContextMenu::cleanup()
{
    if (m_window && ctxMenu()) {
        evalJs(QStringLiteral("canvasContextMenu.close()"));
        QTRY_VERIFY(!menuVisible());
    }
}

void TstCanvasContextMenu::catalogGroundAndOmissionPin()
{
    // Echo-proofing (RULE 9): the text lookups below match on REAL
    // catalog values, pinned against the checked-in en.json.
    QCOMPARE(catTr("ui.menu.edit.cut"), QStringLiteral("Cut"));
    QCOMPARE(catTr("ui.menu.edit.paste"), QStringLiteral("Paste"));
    QCOMPARE(catTr("ui.menu.select.deselect"), QStringLiteral("Deselect"));
    QCOMPARE(catTr("ui.menu.select.all"), QStringLiteral("Select All"));
    QCOMPARE(catTr("ui.menu.select.load_saved"),
             QStringLiteral("Load Last Saved"));
    QCOMPARE(catTr("ui.menu.image.crop"), QStringLiteral("Crop…"));
    QCOMPARE(catTr("ui.dialog.modify.expand"), QStringLiteral("Expand"));
    QCOMPARE(catTr("ui.dialog.apply"), QStringLiteral("Apply"));

    // THE OMISSION PIN, two-sided (the ID RULE record in
    // CanvasContextMenu.qml's header): the design's Paste Into and
    // Delete rows were mint-queued at item 7, NOT shipped and NOT
    // faked. STATE CHANGES OF RECORD: (1) both ids LANDED (menu-mint
    // 3dc5c37, sealed) — side (a) fired exactly as designed and
    // converted to mint-regression guards. (2) 14b SHIPPED the Delete
    // row with its ruled handler (edit.cut sans copy through
    // root.deleteLayerContentWithConfirm) — its side (b) flipped to the
    // row-presence assert below, per the recorded instruction. (3) ITEM
    // 15 (the menu-mint follow-up remainder) SHIPPED the Paste Into row
    // with BOTH halves it was waiting on — the pasteIntoFromClipboard
    // controller arm on the edit.paste_into door and the edit.pasteInto
    // availability row — so its (b) flipped the same way, per the same
    // recorded instruction. The anti-fake teeth did NOT go away, they
    // MOVED: a Paste Into row is now only honest while it is live-wired,
    // and pasteIntoLandsInsideTheSelectionOnly() below proves the wiring
    // on ENGINE PIXELS — one edit.paste_into history step, and the pasted
    // content contained by the selection-cloned mask — rather than on the
    // row's mere presence. Its two siblings carry the rest of the moved
    // burden: pasteIntoHonestGatingWithoutSelection() (the typed refusal,
    // no fake success) and pasteIntoBothMenusReachTheSameArm() (one arm
    // under both consumers).
    // RECORD CORRECTION (fix round, §5 minor-1): this sentence formerly
    // cited "pasteIntoIsLiveWiredNotFake()" — a name that has never
    // existed anywhere in shell/. The citation IS the audit trail a
    // reviewer follows to confirm the anti-fake burden really moved, so
    // it is repaired here to the cases that actually carry it, not
    // dropped.
    const Catalog &cat = m_i18n->catalog();
    QVERIFY2(cat.has(QStringLiteral("ui.menu.edit.paste_into")),
             "ui.menu.edit.paste_into vanished from the catalog — the "
             "sealed menu-mint (3dc5c37) regressed");
    QVERIFY2(cat.has(QStringLiteral("ui.menu.edit.delete")),
             "ui.menu.edit.delete vanished from the catalog — the "
             "sealed menu-mint (3dc5c37) regressed");
    const QStringList texts = ctxAllTexts();
    QVERIFY2(!texts.isEmpty(), "context menu rendered no items");
    // item-15 row-presence (the converted side): the Paste Into row
    // exists, and the assert that it is not a FAKE is the engine-pixel
    // case below — not this presence check.
    QVERIFY2(texts.contains(QStringLiteral("Paste Into")),
             "the item-15 Paste Into row is MISSING from the context menu "
             "— the shipped row regressed (ui.menu.edit.paste_into + the "
             "pasteIntoFromClipboard/edit.pasteInto halves landed "
             "together at item 15)");
    // 14b row-presence: the Delete row exists (its handler shipped with
    // it — the two are one slice by the desk ruling).
    QVERIFY2(texts.contains(QStringLiteral("Delete")),
             "the 14b Delete row is MISSING from the context menu — the "
             "shipped row regressed (ui.menu.edit.delete + the ruled "
             "edit.cut-sans-copy disposition landed together at 14b)");
}

void TstCanvasContextMenu::rightClickOpensAtPressedPoint()
{
    freshDoc();
    QVERIFY(!menuVisible());
    // A REAL right-button press at a known interior canvas point.
    const QPointF canvasPt(200.0, 150.0);
    const QPoint scenePt =
        m_canvas->mapToScene(canvasPt).toPoint();
    QTest::mousePress(m_window, Qt::RightButton, Qt::NoModifier, scenePt);
    QTRY_VERIFY2(menuVisible(), "right-click did not open the menu");
    QObject *menu = ctxMenu();
    // Opens AT the pressed point (menu coordinates share the canvas
    // container space).
    QVERIFY2(qAbs(menu->property("x").toDouble() - canvasPt.x()) <= 1.0
                 && qAbs(menu->property("y").toDouble() - canvasPt.y())
                        <= 1.0,
             qPrintable(QStringLiteral("menu at %1,%2 — pressed %3,%4")
                            .arg(menu->property("x").toDouble())
                            .arg(menu->property("y").toDouble())
                            .arg(canvasPt.x())
                            .arg(canvasPt.y())));
    QTest::mouseRelease(m_window, Qt::RightButton, Qt::NoModifier,
                        scenePt);
    QVERIFY2(menuVisible(), "the paired right-button release closed the "
                            "menu (press-open contract)");
    closeMenu();
}

void TstCanvasContextMenu::shiftF10OpensAtSelectionAnchor()
{
    freshDoc();
    const QRect sel(40, 30, 80, 50);
    QVERIFY(setRectSelection(sel));
    QVERIFY(m_controller->property("selectionActive").toBool());
    m_canvas->forceActiveFocus();
    QTRY_VERIFY(m_canvas->hasActiveFocus());

    QTest::keyClick(m_window, Qt::Key_F10, Qt::ShiftModifier);
    QTRY_VERIFY2(menuVisible(), "Shift+F10 did not open the menu");
    // The keyboard anchor = the selection's view-space bounds center,
    // recomputed independently (engine rect + zoomFit centering).
    const QPointF expected = expectedDocToView(
        sel.x() + sel.width() / 2.0, sel.y() + sel.height() / 2.0);
    QObject *menu = ctxMenu();
    QVERIFY2(qAbs(menu->property("x").toDouble() - expected.x()) <= 1.5
                 && qAbs(menu->property("y").toDouble() - expected.y())
                        <= 1.5,
             qPrintable(QStringLiteral(
                            "menu at %1,%2 — selection anchor %3,%4")
                            .arg(menu->property("x").toDouble())
                            .arg(menu->property("y").toDouble())
                            .arg(expected.x())
                            .arg(expected.y())));
    closeMenu();
}

void TstCanvasContextMenu::menuKeyOpensAtVisibleDocCenter()
{
    freshDoc();
    m_canvas->forceActiveFocus();
    QTRY_VERIFY(m_canvas->hasActiveFocus());

    QTest::keyClick(m_window, Qt::Key_Menu);
    QTRY_VERIFY2(menuVisible(), "the Menu key did not open the menu");
    // No selection: the anchor is the visible document region's center
    // (the whole document is visible after the doc-open fit).
    const QPointF expected = expectedDocToView(
        m_controller->docWidth() / 2.0, m_controller->docHeight() / 2.0);
    QObject *menu = ctxMenu();
    QVERIFY2(qAbs(menu->property("x").toDouble() - expected.x()) <= 1.5
                 && qAbs(menu->property("y").toDouble() - expected.y())
                        <= 1.5,
             qPrintable(QStringLiteral("menu at %1,%2 — doc center %3,%4")
                            .arg(menu->property("x").toDouble())
                            .arg(menu->property("y").toDouble())
                            .arg(expected.x())
                            .arg(expected.y())));
    closeMenu();
}

void TstCanvasContextMenu::sectionTruthAcrossStatesAndTools()
{
    freshDoc();

    // Effective visibility only exists while the popup is OPEN — every
    // sweep below opens the menu with a real right-click first.
    const QPoint pt =
        m_canvas->mapToScene(QPointF(160.0, 100.0)).toPoint();
    auto openMenu = [&]() {
        QTest::mousePress(m_window, Qt::RightButton, Qt::NoModifier, pt);
        QTest::mouseRelease(m_window, Qt::RightButton, Qt::NoModifier,
                            pt);
    };
    const char *selectionKeys[] = {
        "ui.menu.edit.cut",           "ui.menu.edit.copy",
        // item-15 row: it belongs to the SELECTION section by
        // construction — edit.paste_into's engine precondition is an
        // active selection (§40.5), so a no-selection instance could
        // only ever render permanently disabled.
        "ui.menu.edit.paste_into",
        "ui.menu.edit.delete", // 14b row (selection section)
        "ui.menu.select.deselect",    "ui.menu.select.invert",
        "ui.menu.select.save_selection",
        // submenu WRAPPERS (found by title text). Q-D32: the Modify wrapper
        // takes the ellipsis-free id — an arrow promises a submenu, an
        // ellipsis promises a dialog, and this title showed both. The
        // menu-bar row keeps `ui.menu.select.modify`, where the ellipsis is
        // true because it opens modifyDialog directly.
        "ui.menu.select.modify_submenu", "ui.menu.filter",
    };
    const char *pasteKeys[] = {
        "ui.menu.edit.paste",
        "ui.menu.select.all",
    };

    // -- no-selection, brush tool: paste section + Fill… quick action.
    m_canvas->setProperty("tool", QStringLiteral("brush.stroke"));
    QVERIFY(!m_controller->property("selectionActive").toBool());
    openMenu();
    QTRY_VERIFY(menuVisible());
    for (const char *k : selectionKeys)
        QVERIFY2(ctxVisibleCount(k) == 0,
                 qPrintable(QStringLiteral("%1 visible with NO selection")
                                .arg(QLatin1String(k))));
    for (const char *k : pasteKeys)
        QVERIFY2(ctxVisibleCount(k) == 1,
                 qPrintable(QStringLiteral("%1 hidden with NO selection")
                                .arg(QLatin1String(k))));
    QCOMPARE(ctxVisibleCount("ui.menu.edit.fill"), 1);
    QCOMPARE(ctxVisibleCount("ui.menu.select.load_saved"), 0);
    QCOMPARE(ctxVisibleCount("ui.menu.image.crop"), 0);
    closeMenu();

    // -- no-selection, selection tool: load rows appear, crop/fill go.
    m_canvas->setProperty("tool", QStringLiteral("select.rect"));
    openMenu();
    QTRY_VERIFY(menuVisible());
    QCOMPARE(ctxVisibleCount("ui.menu.select.load_saved"), 1);
    QCOMPARE(ctxVisibleCount("ui.menu.select.load_layer_alpha"), 1);
    QCOMPARE(ctxVisibleCount("ui.menu.image.crop"), 0);
    QCOMPARE(ctxVisibleCount("ui.menu.edit.fill"), 0);
    closeMenu();

    // -- no-selection, crop tool: Crop…/Trim… appear.
    m_canvas->setProperty("tool", QStringLiteral("crop"));
    openMenu();
    QTRY_VERIFY(menuVisible());
    QCOMPARE(ctxVisibleCount("ui.menu.image.crop"), 1);
    QCOMPARE(ctxVisibleCount("ui.menu.image.trim"), 1);
    QCOMPARE(ctxVisibleCount("ui.menu.select.load_saved"), 0);
    closeMenu();

    // -- no-selection, move tool: HONEST EMPTY (no quick rows at all).
    m_canvas->setProperty("tool", QStringLiteral("move"));
    openMenu();
    QTRY_VERIFY(menuVisible());
    QCOMPARE(ctxVisibleCount("ui.menu.image.crop"), 0);
    QCOMPARE(ctxVisibleCount("ui.menu.select.load_saved"), 0);
    QCOMPARE(ctxVisibleCount("ui.menu.edit.fill"), 0);
    // The generic rows still stand.
    QCOMPARE(ctxVisibleCount("ui.menu.edit.paste"), 1);
    QCOMPARE(ctxVisibleCount("ui.menu.select.all"), 1);
    closeMenu();

    // -- selection active (REAL engine selection): the sections flip.
    m_canvas->setProperty("tool", QStringLiteral("brush.stroke"));
    QVERIFY(setRectSelection(QRect(20, 20, 60, 40)));
    openMenu();
    QTRY_VERIFY(menuVisible());
    for (const char *k : selectionKeys)
        QVERIFY2(ctxVisibleCount(k) == 1,
                 qPrintable(QStringLiteral("%1 hidden WITH selection")
                                .arg(QLatin1String(k))));
    for (const char *k : pasteKeys)
        QVERIFY2(ctxVisibleCount(k) == 0,
                 qPrintable(QStringLiteral("%1 visible WITH selection")
                                .arg(QLatin1String(k))));
    // Fill… shows exactly ONCE (the selection section's instance; the
    // brush quick-action twin is hidden with a selection active).
    QCOMPARE(ctxVisibleCount("ui.menu.edit.fill"), 1);
    // The Modify ▸ submenu carries the three cataloged ops and the
    // Filters ▸ submenu all six live kinds (existence — sub-popups are
    // closed until cascaded).
    QVERIFY(ctxItem("ui.dialog.modify.expand")
            && ctxItem("ui.dialog.modify.contract")
            && ctxItem("ui.dialog.modify.feather"));
    QVERIFY(ctxItem("filter.gaussian_blur") && ctxItem("filter.box_blur")
            && ctxItem("filter.median") && ctxItem("filter.high_pass")
            && ctxItem("filter.add_noise")
            && ctxItem("filter.unsharp_mask"));
    closeMenu();
    QVERIFY(applyOk("selection.none", {})
                .value(QStringLiteral("ok"))
                .toBool());
}

void TstCanvasContextMenu::enabledTruthConsumesAvailabilitySurface()
{
    freshDoc();

    QQuickItem *deselect = ctxItem("ui.menu.select.deselect");
    QQuickItem *fill = ctxItem("ui.menu.edit.fill");
    QQuickItem *loadSaved = ctxItem("ui.menu.select.load_saved");
    QVERIFY(deselect && fill && loadSaved);
    // (`enabled` reads the item's binding regardless of popup state —
    // both Fill… instances share the identical row binding, the first
    // suffices.)

    // Probe 1: select.deselect flips with the REAL selection state.
    QVERIFY(setRectSelection(QRect(10, 10, 50, 50)));
    QVERIFY2(availRow("select.deselect").value(QStringLiteral("enabled"))
                 .toBool(),
             "surface: deselect should be enabled with a selection");
    QCOMPARE(deselect->property("enabled").toBool(), true);
    QVERIFY(applyOk("selection.none", {})
                .value(QStringLiteral("ok"))
                .toBool());
    QVERIFY(!availRow("select.deselect").value(QStringLiteral("enabled"))
                 .toBool());
    QCOMPARE(deselect->property("enabled").toBool(), false);

    // Probe 2: edit.fill follows active-layer emptiness (the S8 arm —
    // out-of-range index with the document open).
    QVERIFY(availRow("edit.fill").value(QStringLiteral("enabled"))
                .toBool());
    QCOMPARE(fill->property("enabled").toBool(), true);
    m_controller->setProperty("activeLayerIndex", 99);
    QVERIFY(!availRow("edit.fill").value(QStringLiteral("enabled"))
                 .toBool());
    QCOMPARE(fill->property("enabled").toBool(), false);
    m_controller->setProperty("activeLayerIndex", 0);
    QCOMPARE(fill->property("enabled").toBool(), true);

    // Probe 3: select.loadSaved follows the saved-channel handle.
    QVERIFY(!availRow("select.loadSaved").value(QStringLiteral("enabled"))
                 .toBool());
    QCOMPARE(loadSaved->property("enabled").toBool(), false);
    QVERIFY(applyOk("selection.all", {})
                .value(QStringLiteral("ok"))
                .toBool());
    QVERIFY(m_controller->saveSelectionAsChannel()
                .value(QStringLiteral("ok"))
                .toBool());
    QVERIFY(availRow("select.loadSaved").value(QStringLiteral("enabled"))
                .toBool());
    QCOMPARE(loadSaved->property("enabled").toBool(), true);
    QVERIFY(applyOk("selection.none", {})
                .value(QStringLiteral("ok"))
                .toBool());
}

void TstCanvasContextMenu::actionIdentityDeselect()
{
    freshDoc();
    const QRect sel(30, 20, 70, 60);

    // CONTEXT path: a REAL click on the popup's Deselect item.
    QVERIFY(setRectSelection(sel));
    QVERIFY(m_controller->property("selectionActive").toBool());
    const QPoint pt =
        m_canvas->mapToScene(QPointF(280.0, 60.0)).toPoint();
    QTest::mousePress(m_window, Qt::RightButton, Qt::NoModifier, pt);
    QTest::mouseRelease(m_window, Qt::RightButton, Qt::NoModifier, pt);
    QTRY_VERIFY(menuVisible());
    clickItem(ctxItem("ui.menu.select.deselect"));
    QTRY_VERIFY(!menuVisible());
    QVERIFY2(!m_controller->property("selectionActive").toBool(),
             "context-menu Deselect left the selection active");
    // Journaled like the twin: undo restores the selection.
    m_controller->undo();
    QVERIFY2(m_controller->property("selectionActive").toBool(),
             "context-menu Deselect was not the journaled selection.none");

    // TOP-MENU path from the identical state.
    QVERIFY(m_controller->property("selectionActive").toBool());
    QObject *top = topMenuItem("ui.menu.select.deselect");
    QVERIFY(top);
    QVERIFY(QMetaObject::invokeMethod(top, "triggered"));
    QVERIFY2(!m_controller->property("selectionActive").toBool(),
             "top-menu Deselect left the selection active");
    m_controller->undo();
    QVERIFY(m_controller->property("selectionActive").toBool());
    // Identical engine end-state either way (deselect + undo round
    // trip); leave clean.
    QVERIFY(applyOk("selection.none", {})
                .value(QStringLiteral("ok"))
                .toBool());
}

void TstCanvasContextMenu::actionIdentityFillOnSelection()
{
    freshDoc();
    const QRect sel(40, 30, 80, 50);
    QVERIFY(setRectSelection(sel));
    const QRect full(0, 0, m_controller->docWidth(),
                     m_controller->docHeight());

    // TOP-MENU path: Fill… -> the shared dialog -> Apply (REAL click).
    QObject *topFill = topMenuItem("ui.menu.edit.fill");
    QVERIFY(topFill);
    QVERIFY(QMetaObject::invokeMethod(topFill, "triggered"));
    QQuickItem *apply = nullptr;
    QTRY_VERIFY2((apply = uniqueVisibleButton("ui.dialog.apply"))
                     != nullptr,
                 "fill dialog Apply button not found/visible");
    clickItem(apply);
    QImage imgTop;
    QVERIFY(m_controller->readRegionImage(full, &imgTop));
    // The fill took INSIDE the selection and was CLIPPED outside.
    const QColor inside = imgTop.pixelColor(sel.center());
    QVERIFY2(inside != QColor(Qt::white),
             "top-menu fill did not reach the selection interior");
    QCOMPARE(imgTop.pixelColor(QPoint(5, 5)), QColor(Qt::white));

    // Rewind the fill; the selection (an earlier journal step) stays.
    m_controller->undo();
    QImage rewound;
    QVERIFY(m_controller->readRegionImage(full, &rewound));
    QCOMPARE(rewound.pixelColor(sel.center()), QColor(Qt::white));
    QVERIFY2(m_controller->property("selectionActive").toBool(),
             "undo of the fill dropped the selection — state diverged");

    // CONTEXT path from the identical state: REAL click on Fill… in
    // the opened popup -> the SAME dialog -> Apply.
    const QPoint pt =
        m_canvas->mapToScene(QPointF(300.0, 170.0)).toPoint();
    QTest::mousePress(m_window, Qt::RightButton, Qt::NoModifier, pt);
    QTest::mouseRelease(m_window, Qt::RightButton, Qt::NoModifier, pt);
    QTRY_VERIFY(menuVisible());
    QQuickItem *ctxFill = ctxItem("ui.menu.edit.fill");
    QVERIFY2(ctxFill->isVisible(),
             "Fill… missing from the selection-active section");
    clickItem(ctxFill);
    QTRY_VERIFY(!menuVisible());
    apply = nullptr;
    QTRY_VERIFY2((apply = uniqueVisibleButton("ui.dialog.apply"))
                     != nullptr,
                 "context path did not open the fill dialog");
    clickItem(apply);
    QImage imgCtx;
    QVERIFY(m_controller->readRegionImage(full, &imgCtx));

    // THE IDENTITY: byte-identical engine readback, both paths.
    QCOMPARE(imgCtx, imgTop);
}

// ---------------------------------------------------------------------
// v1.0.1 item 15 — PASTE INTO (the menu-mint follow-up remainder).
//
// The engine kind is edit.paste_into (contracts/commands/
// edit.paste_into.yaml; crates/appname-core/src/engine/kinds.rs
// EditPasteIntoKind). Probed first-hand, it does NOT clip pixels: it
// inserts the pasted layer exactly as edit.paste does and ATTACHES a
// raster mask whose coverage is a bit-exact clone of the ACTIVE
// SELECTION. Containment is therefore a §11.2 COMPOSITE truth (mask_eval
// samples coverage at document-space p; absent reads 0), which is why
// these cases assert it on READBACK PIXELS and not on a receipt field —
// the containment IS the whole semantic of the feature.
// ---------------------------------------------------------------------

// (a) THE FEATURE ITSELF, on engine truth: with a selection active and
// clipboard content present, Paste Into adds EXACTLY ONE history step of
// kind edit.paste_into, and the pasted content lands INSIDE the
// selection and NOWHERE ELSE. The clipboard raster is full-document
// sized and uniformly opaque, so "outside stays white" cannot be an
// accident of where the paste happened to land — only the mask can
// produce it. Undo restores the document byte-for-byte.
void TstCanvasContextMenu::pasteIntoLandsInsideTheSelectionOnly()
{
    freshDoc();
    const QRect sel(40, 30, 80, 50);
    putFullDocClipboardImage(QColor(Qt::red));
    QVERIFY(setRectSelection(sel));
    QVERIFY(m_controller->property("selectionActive").toBool());

    const QImage before = fullReadback();
    QVERIFY(!before.isNull());
    QCOMPARE(before.pixelColor(sel.center()), QColor(Qt::white));
    const int histBefore = historyCount();

    // Fire the REAL top-menu row (its own leg; the context-menu twin is
    // case (c)).
    QObject *top = topMenuItem("ui.menu.edit.paste_into");
    QVERIFY2(top, "the item-15 Paste Into row is missing from Edit");
    QCOMPARE(top->property("enabled").toBool(), true);
    QVERIFY(QMetaObject::invokeMethod(top, "triggered"));

    // ENGINE TRUTH 1: exactly one step, and it is THIS kind (not the
    // plain paste, not two commands).
    QCOMPARE(historyCount(), histBefore + 1);
    QCOMPARE(lastHistoryKind(), QStringLiteral("edit.paste_into"));

    // ENGINE TRUTH 2: THE CONTAINMENT, on pixels.
    const QImage after = fullReadback();
    QVERIFY(!after.isNull());
    QCOMPARE(after.pixelColor(sel.center()), QColor(Qt::red));
    QCOMPARE(after.pixelColor(sel.topLeft()), QColor(Qt::red));
    QCOMPARE(after.pixelColor(sel.bottomRight()), QColor(Qt::red));
    // Every side of the selection, one pixel out — and the far corners.
    const QPoint outside[] = {
        QPoint(sel.left() - 1, sel.center().y()),
        QPoint(sel.right() + 1, sel.center().y()),
        QPoint(sel.center().x(), sel.top() - 1),
        QPoint(sel.center().x(), sel.bottom() + 1),
        QPoint(0, 0),
        QPoint(m_controller->docWidth() - 1, 0),
        QPoint(0, m_controller->docHeight() - 1),
        QPoint(m_controller->docWidth() - 1,
               m_controller->docHeight() - 1),
    };
    for (const QPoint &p : outside)
        QVERIFY2(after.pixelColor(p) == QColor(Qt::white),
                 qPrintable(
                     QStringLiteral("pasted content escaped the selection "
                                    "at (%1,%2): %3 — the clipboard raster "
                                    "covers the whole document, so this is "
                                    "the mask failing, i.e. plain Paste "
                                    "wearing Paste Into's label")
                         .arg(p.x())
                         .arg(p.y())
                         .arg(after.pixelColor(p).name())));

    // Journaled like any command: one undo rewinds it exactly, and the
    // selection (an earlier step) survives.
    m_controller->undo();
    QCOMPARE(historyCount(), histBefore + 1); // entries stay; cursor moves
    QVERIFY2(fullReadback() == before,
             "undo of Paste Into did not restore the document");
    QVERIFY2(m_controller->property("selectionActive").toBool(),
             "undo of Paste Into dropped the selection — the engine is "
             "documented to leave it UNCHANGED (§10.3)");
}

// (b) THE AVAILABILITY ROW'S HONEST GATING. The predicate is derived
// from the engine's REAL precondition — EditPasteIntoKind::apply refuses
// error.command.selection_required when the document has no active
// selection (§40.5: the selection BECOMES the mask) — so without one
// both consumers are disabled and the row names that exact engine code
// as its reasonId. And the door underneath does not lie either: invoked
// anyway it refuses TYPED on the existing ui.status.rejected path, with
// the document and the journal untouched. Never a fake success.
void TstCanvasContextMenu::pasteIntoHonestGatingWithoutSelection()
{
    freshDoc();
    putFullDocClipboardImage(QColor(Qt::red));
    QVERIFY(!m_controller->property("selectionActive").toBool());

    // The row: disabled, and disabled FOR THE ENGINE'S REASON.
    const QVariantMap off = availRow("edit.pasteInto");
    QCOMPARE(off.value(QStringLiteral("enabled")).toBool(), false);
    QCOMPARE(off.value(QStringLiteral("reasonId")).toString(),
             QStringLiteral("error.command.selection_required"));

    // Both consumers read it (the top row; the context row lives in the
    // selection-active section, so it is also section-hidden here —
    // assert BOTH facts, they are different guarantees).
    QObject *top = topMenuItem("ui.menu.edit.paste_into");
    QVERIFY(top);
    QCOMPARE(top->property("enabled").toBool(), false);
    QQuickItem *ctx = ctxItem("ui.menu.edit.paste_into");
    QVERIFY2(ctx, "the context-menu Paste Into row is missing");
    QCOMPARE(ctx->property("enabled").toBool(), false);

    // The door underneath: a submission that reaches the engine anyway
    // refuses TYPED, mutates nothing, and journals nothing.
    const QImage before = fullReadback();
    const int histBefore = historyCount();
    QVERIFY2(!m_controller->pasteIntoFromClipboard(),
             "pasteIntoFromClipboard reported success with NO selection");
    QVERIFY2(fullReadback() == before,
             "a selection-less Paste Into mutated the document");
    QCOMPARE(historyCount(), histBefore);
    const QString status =
        m_controller->property("statusText").toString();
    QVERIFY2(status.contains(
                 QStringLiteral("error.command.selection_required")),
             qPrintable(QStringLiteral("the refusal did not surface typed "
                                       "on ui.status.rejected: \"%1\"")
                            .arg(status)));

    // POSITIVE CONTROL: the selection is the ONLY blocker — arm one and
    // the same row and the same door both come alive.
    QVERIFY(setRectSelection(QRect(40, 30, 80, 50)));
    const QVariantMap on = availRow("edit.pasteInto");
    QCOMPARE(on.value(QStringLiteral("enabled")).toBool(), true);
    QVERIFY2(!on.contains(QStringLiteral("reasonId")),
             "an ENABLED row carries a reasonId (14a contract)");
    QCOMPARE(top->property("enabled").toBool(), true);
    QVERIFY(m_controller->pasteIntoFromClipboard());
    QCOMPARE(lastHistoryKind(), QStringLiteral("edit.paste_into"));
    m_controller->undo();
    QVERIFY(applyOk("selection.none", {})
                .value(QStringLiteral("ok"))
                .toBool());
}

// (c) ONE ARM, TWO DOORS: the top Edit row and the canvas context-menu
// row reach the SAME controller arm — proven by engine readback, the
// item-7 action-identity discipline (both paths run from the identical
// state and must produce byte-identical documents).
void TstCanvasContextMenu::pasteIntoBothMenusReachTheSameArm()
{
    freshDoc();
    const QRect sel(40, 30, 80, 50);
    putFullDocClipboardImage(QColor(Qt::red));
    QVERIFY(setRectSelection(sel));

    // TOP-MENU path.
    QObject *top = topMenuItem("ui.menu.edit.paste_into");
    QVERIFY(top);
    QVERIFY(QMetaObject::invokeMethod(top, "triggered"));
    QCOMPARE(lastHistoryKind(), QStringLiteral("edit.paste_into"));
    const QImage imgTop = fullReadback();
    QVERIFY(!imgTop.isNull());
    QVERIFY2(imgTop.pixelColor(sel.center()) == QColor(Qt::red),
             "the top-menu path pasted nothing");

    // Rewind to the identical state (the selection is an earlier step).
    m_controller->undo();
    QVERIFY(m_controller->property("selectionActive").toBool());

    // CONTEXT path: a REAL click on the popup's Paste Into row.
    const QPoint pt =
        m_canvas->mapToScene(QPointF(300.0, 170.0)).toPoint();
    QTest::mousePress(m_window, Qt::RightButton, Qt::NoModifier, pt);
    QTest::mouseRelease(m_window, Qt::RightButton, Qt::NoModifier, pt);
    QTRY_VERIFY(menuVisible());
    QQuickItem *ctx = ctxItem("ui.menu.edit.paste_into");
    QVERIFY2(ctx && ctx->isVisible(),
             "Paste Into missing from the selection-active section");
    QCOMPARE(ctx->property("enabled").toBool(), true);
    clickItem(ctx);
    QTRY_VERIFY(!menuVisible());
    QCOMPARE(lastHistoryKind(), QStringLiteral("edit.paste_into"));
    const QImage imgCtx = fullReadback();

    // THE IDENTITY: byte-identical engine readback, both doors.
    QCOMPARE(imgCtx, imgTop);

    m_controller->undo();
    QVERIFY(applyOk("selection.none", {})
                .value(QStringLiteral("ok"))
                .toBool());
}

// (d) THE `at`-OMISSION DECISION, PINNED (§5 fix round, minor-3). The
// controller arm deliberately submits NO `at` param, exactly as
// pasteFromClipboard does. Until this case, nothing defended that: the
// reviewer injected `params.insert("at", {10,10})` into
// pasteIntoFromClipboard and the WHOLE suite stayed green, because cases
// (a)/(c) use a FULL-DOCUMENT clipboard raster — with w == canvas width
// the centred origin is (0,0), and a raster that already covers every
// document pixel keeps covering the selection after any small shift, so
// placement is unobservable there by construction.
//
// ENGINE DERIVATION (RULE 8 — read from the kind, not from a summary).
// crates/appname-core/src/engine/kinds.rs, fn paste_origin(cx, w, h):
//   * `at` present as exactly 2 numbers -> origin = [round(x), round(y)]
//     VERBATIM, in document space;
//   * `at` present but not 2 numbers -> EngineError::range("at", …)
//     i.e. error.schema.out_of_range;
//   * `at` ABSENT -> origin = canvas.min + (canvas.size - raster size)/2,
//     read off document().geometry().canvas_rect() — the CANVAS, not the
//     viewport. (contracts/commands/edit.paste_into.yaml describes the
//     absent case as "viewport-centered"; the impl and its own
//     doc-comment say CANVAS. That contract/impl prose divergence is
//     recorded engine-side and is outside this lane's fence — this pin
//     follows the IMPL, which is the truth that runs.)
// The contract makes `at` OPTIONAL and records "resolved position
// stored", so with `at` omitted the ENGINE resolves the placement and
// journals it. THAT is why the shell must not send one: the placement
// then replays from the journal alone and never depends on shell state a
// replay does not have. And the failure would be silent rather than
// loud — the mask is a document-space clone of the SELECTION and does
// NOT move with `at`, so a shell-invented `at` displaces the pasted
// content underneath the very mask that is supposed to frame it.
//
// THE RECIPE that makes placement observable: an 80x60 raster in a
// 320x200 document. The engine's absent-`at` formula puts its top-left
// at ((320-80)/2, (200-60)/2) = (120,70) — computed here FROM the
// formula, not hardcoded — and the selection is chosen to contain the
// whole footprint plus a margin on every side, so inside the selection
// there are both covered and uncovered pixels and the four footprint
// edges are pinned to the pixel. Any `at` the shell invents moves that
// footprint and reds at least one probe.
void TstCanvasContextMenu::pasteIntoOmitsAtSoTheEngineResolvesPlacement()
{
    freshDoc();
    // Raster strictly smaller than the document, and a selection that
    // contains the centred footprint with >= 1px margin all round.
    const int rw = 80;
    const int rh = 60;
    const QRect sel(100, 50, 120, 100);
    putClipboardImage(rw, rh, QColor(Qt::red));
    QVERIFY(setRectSelection(sel));
    QVERIFY(m_controller->property("selectionActive").toBool());

    // The engine's own absent-`at` formula (paste_origin, None arm).
    // canvas_rect().min is (0,0) for a freshly created document, so the
    // canvas-centred origin reduces to (docW - rw)/2, (docH - rh)/2.
    const QPoint origin((m_controller->docWidth() - rw) / 2,
                        (m_controller->docHeight() - rh) / 2);
    const QRect footprint(origin, QSize(rw, rh));
    QVERIFY2(sel.contains(footprint.adjusted(-1, -1, 1, 1)),
             "the recipe is broken: the selection must contain the "
             "centred footprint plus a 1px margin, or the probes below "
             "would be testing the mask instead of the placement");

    const int histBefore = historyCount();
    QObject *top = topMenuItem("ui.menu.edit.paste_into");
    QVERIFY(top);
    QCOMPARE(top->property("enabled").toBool(), true);
    QVERIFY(QMetaObject::invokeMethod(top, "triggered"));
    QCOMPARE(historyCount(), histBefore + 1);
    QCOMPARE(lastHistoryKind(), QStringLiteral("edit.paste_into"));

    const QImage after = fullReadback();
    QVERIFY(!after.isNull());

    // COVERED: the footprint's four corners are the pasted raster.
    const QPoint covered[] = {
        footprint.topLeft(),
        footprint.topRight(),
        footprint.bottomLeft(),
        footprint.bottomRight(),
    };
    for (const QPoint &p : covered)
        QVERIFY2(after.pixelColor(p) == QColor(Qt::red),
                 qPrintable(
                     QStringLiteral("the pasted raster is NOT at the "
                                    "engine's canvas-centred origin: "
                                    "(%1,%2) should be inside the "
                                    "footprint but reads %3. The shell "
                                    "must submit NO `at` — an invented "
                                    "one relocates the layer while the "
                                    "selection-cloned mask stays put.")
                         .arg(p.x())
                         .arg(p.y())
                         .arg(after.pixelColor(p).name())));

    // UNCOVERED: one pixel beyond each footprint edge — still INSIDE the
    // selection, so the mask cannot be what whitens them. Only the
    // placement can. These are the probes a shell-invented `at` reds.
    const QPoint uncovered[] = {
        QPoint(footprint.left() - 1, footprint.center().y()),
        QPoint(footprint.right() + 1, footprint.center().y()),
        QPoint(footprint.center().x(), footprint.top() - 1),
        QPoint(footprint.center().x(), footprint.bottom() + 1),
    };
    for (const QPoint &p : uncovered) {
        QVERIFY(sel.contains(p));
        QVERIFY2(after.pixelColor(p) == QColor(Qt::white),
                 qPrintable(
                     QStringLiteral("the pasted footprint extends past "
                                    "the engine's canvas-centred origin "
                                    "at (%1,%2): %3. This pixel is "
                                    "inside the selection, so the mask "
                                    "is not hiding it — the placement "
                                    "moved, i.e. the shell sent an `at`.")
                         .arg(p.x())
                         .arg(p.y())
                         .arg(after.pixelColor(p).name())));
    }

    m_controller->undo();
    QVERIFY(applyOk("selection.none", {})
                .value(QStringLiteral("ok"))
                .toBool());
}

// v1.0.1 item 7 §5 fix round (review F1, MAJOR): a right press DURING an
// in-flight left-drag gesture must be IGNORED ENTIRELY (the pre-item-7
// contract: canvas accepts Left|Middle only, nobody else took the press).
// Before the guard, Qt 6 delivered the mid-drag right press through the
// item stack, the overlay accepted it, and the opening popup stole the
// exclusive grab — the canvas never saw the left release and the marquee
// died silently. THE RECIPE (reviewer's probe): a control drag lands a
// selection; the SAME drag with a mid-flight right press+release must
// land the SAME selection (engine truth: selectionActive + byte-identical
// Grayscale8 mask readback) with NO menu opened — and a serial right-click
// afterwards must still open (the refusal is per-event, never sticky).
void TstCanvasContextMenu::midDragRightPressIsIgnored()
{
    freshDoc();
    m_canvas->setProperty("tool", QStringLiteral("select.rect"));
    const QRect full(0, 0, m_controller->docWidth(),
                     m_controller->docHeight());
    const QPoint p0 = m_canvas->mapToScene(QPointF(60.0, 50.0)).toPoint();
    const QPoint p1 = m_canvas->mapToScene(QPointF(140.0, 90.0)).toPoint();
    const QPoint p2 = m_canvas->mapToScene(QPointF(200.0, 140.0)).toPoint();
    const QPoint pr = m_canvas->mapToScene(QPointF(150.0, 100.0)).toPoint();

    // CONTROL: the drag alone lands a selection (probe mechanics valid).
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier, p0);
    QTest::mouseMove(m_window, p1);
    QTest::mouseMove(m_window, p2);
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier, p2);
    QVERIFY2(m_controller->property("selectionActive").toBool(),
             "control drag landed no selection — probe mechanics broken");
    QImage maskControl;
    QVERIFY(m_controller->readSelectionImage(full, &maskControl));
    QVERIFY(applyOk("selection.none", {})
                .value(QStringLiteral("ok"))
                .toBool());

    // DEFECT RUN: the SAME drag with a mid-flight right press+release.
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier, p0);
    QTest::mouseMove(m_window, p1);
    QTest::mousePress(m_window, Qt::RightButton, Qt::NoModifier, pr);
    QTest::qWait(50); // give a wrongly-accepted press time to pop the menu
    const bool openedMidDrag = menuVisible();
    QTest::mouseRelease(m_window, Qt::RightButton, Qt::NoModifier, pr);
    QTest::mouseMove(m_window, p2);
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier, p2);
    const bool openAfterRelease = menuVisible();
    const bool selAfter =
        m_controller->property("selectionActive").toBool();
    qWarning() << "PROBE: menu open after mid-drag right-click:"
               << openedMidDrag
               << "/ after left release:" << openAfterRelease
               << "selectionActive:" << selAfter;
    QVERIFY2(!openedMidDrag,
             "context menu opened during an in-flight marquee drag");
    QVERIFY2(!openAfterRelease, "context menu open after the left release");
    QVERIFY2(selAfter,
             "the mid-drag right press destroyed the marquee — no "
             "selection landed");
    QImage maskGuarded;
    QVERIFY(m_controller->readSelectionImage(full, &maskGuarded));
    QVERIFY2(maskGuarded == maskControl,
             "guarded-run selection mask differs from the control drag");

    // The refusal is PER-EVENT: a serial right-click still opens.
    QTest::mousePress(m_window, Qt::RightButton, Qt::NoModifier, pr);
    QTRY_VERIFY2(menuVisible(),
                 "serial right-click no longer opens after a refusal — "
                 "the guard stuck");
    QTest::mouseRelease(m_window, Qt::RightButton, Qt::NoModifier, pr);
    closeMenu();
    QVERIFY(applyOk("selection.none", {})
                .value(QStringLiteral("ok"))
                .toBool());
}

// The keyboard arm of the same guard: Shift+F10 (and the Menu key — the
// IDENTICAL Keys handler serves both chords) DURING an in-flight drag.
// PROBED FIRST-HAND on the unguarded tree: unlike the pointer path, a
// keyboard open does NOT steal the mouse grab — the marquee survived
// byte-identically (menu open mid-drag: true, selectionActive: true,
// mask == control: true). The chord is guarded anyway because the open
// itself is the hazard (mid-gesture action dispatch via the focused
// popup; live section flip under the open menu — the Main.qml record),
// so mid-drag it is ignored entirely, the same contract as the pointer
// path. Serial keyboard opens must keep working afterwards.
void TstCanvasContextMenu::midDragKeyboardChordIsIgnored()
{
    freshDoc();
    m_canvas->setProperty("tool", QStringLiteral("select.rect"));
    const QRect full(0, 0, m_controller->docWidth(),
                     m_controller->docHeight());
    const QPoint p0 = m_canvas->mapToScene(QPointF(60.0, 50.0)).toPoint();
    const QPoint p1 = m_canvas->mapToScene(QPointF(140.0, 90.0)).toPoint();
    const QPoint p2 = m_canvas->mapToScene(QPointF(200.0, 140.0)).toPoint();

    // CONTROL drag (fresh mask ground for THIS case).
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier, p0);
    QTest::mouseMove(m_window, p1);
    QTest::mouseMove(m_window, p2);
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier, p2);
    QVERIFY(m_controller->property("selectionActive").toBool());
    QImage maskControl;
    QVERIFY(m_controller->readSelectionImage(full, &maskControl));
    QVERIFY(applyOk("selection.none", {})
                .value(QStringLiteral("ok"))
                .toBool());

    // GUARD RUN: the same drag with a mid-flight Shift+F10.
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier, p0);
    QVERIFY(m_canvas->hasActiveFocus()); // press forces focus — Keys live
    QTest::mouseMove(m_window, p1);
    QTest::keyClick(m_window, Qt::Key_F10, Qt::ShiftModifier);
    QTest::qWait(50);
    const bool openedMidDrag = menuVisible();
    QTest::mouseMove(m_window, p2);
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier, p2);
    const bool openAfterRelease = menuVisible();
    const bool selAfter =
        m_controller->property("selectionActive").toBool();
    QImage maskGuarded;
    const bool maskRead =
        selAfter && m_controller->readSelectionImage(full, &maskGuarded);
    qWarning() << "PROBE: menu open after mid-drag Shift+F10:"
               << openedMidDrag
               << "/ after left release:" << openAfterRelease
               << "selectionActive:" << selAfter << "mask==control:"
               << (maskRead && maskGuarded == maskControl);
    QVERIFY2(!openedMidDrag,
             "context menu opened from the keyboard during an in-flight "
             "marquee drag");
    QVERIFY2(!openAfterRelease, "context menu open after the left release");
    QVERIFY2(selAfter,
             "the mid-drag Shift+F10 destroyed the marquee — no "
             "selection landed");
    QVERIFY2(maskRead && maskGuarded == maskControl,
             "guarded-run selection mask differs from the control drag");

    // Serial keyboard opens still work (BOTH chords — same handler).
    m_canvas->forceActiveFocus();
    QTRY_VERIFY(m_canvas->hasActiveFocus());
    QTest::keyClick(m_window, Qt::Key_F10, Qt::ShiftModifier);
    QTRY_VERIFY2(menuVisible(),
                 "serial Shift+F10 no longer opens after a refusal");
    closeMenu();
    QTest::keyClick(m_window, Qt::Key_Menu);
    QTRY_VERIFY2(menuVisible(),
                 "serial Menu key no longer opens after a refusal");
    closeMenu();
    QVERIFY(applyOk("selection.none", {})
                .value(QStringLiteral("ok"))
                .toBool());
}

void TstCanvasContextMenu::dismissalPressIsConsumed()
{
    // §5 delta F4 (MAJOR): the DISMISSAL press must be consumed by the
    // modal popup, never leaked to the canvas tool. Both reviewer arms,
    // each with a sensitivity control proving the arm discriminates.

    // ARM 1 — BRUSH: an outside-dismiss click must not paint. The probe
    // is MAPPING-INDEPENDENT (item→doc coordinates are not identity —
    // the view centers/zooms): scan the WHOLE doc for any non-white
    // pixel (encoding-robust: the default dark brush is non-white in
    // any encoding; the doc is born white).
    freshDoc();
    m_canvas->setProperty("tool", QStringLiteral("brush.stroke"));
    // GEOMETRY, self-checking (two traps found while building this
    // case): (1) a dismiss point under the popup lands ON the menu and
    // MASKS the leak; (2) item coordinates are not doc coordinates —
    // an off-doc click paints nothing. So: the popup opens at the
    // canvas's far bottom-right (clamps there), the dismiss point is a
    // PROVEN-PAINTABLE doc point near the view's top-left, and the
    // non-overlap is ASSERTED at runtime from the popup's real scene
    // rect — never assumed.
    const QPoint pd = m_canvas->mapToScene(QPointF(250.0, 150.0)).toPoint();
    const QPoint pr = m_canvas
                          ->mapToScene(QPointF(m_canvas->width() - 30.0,
                                               m_canvas->height() - 30.0))
                          .toPoint();
    const QRect fullDoc(0, 0, m_controller->docWidth(),
                        m_controller->docHeight());
    const auto anyNonWhite = [&](const QImage &img) {
        for (int y = 0; y < img.height(); ++y)
            for (int x = 0; x < img.width(); ++x)
                if (img.pixelColor(x, y) != QColor(Qt::white))
                    return true;
        return false;
    };
    // SENSITIVITY: a plain click paints SOMEWHERE (the arm discriminates).
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier, pd);
    QImage docImg;
    QTRY_VERIFY2(m_controller->readRegionImage(fullDoc, &docImg)
                     && anyNonWhite(docImg),
                 "sensitivity: a plain click painted nothing — the arm "
                 "cannot discriminate");
    m_controller->undo(); // the suite's undo idiom (the controller door)
    QVERIFY(m_controller->readRegionImage(fullDoc, &docImg));
    QVERIFY2(!anyNonWhite(docImg),
             "undo did not restore the white doc — arm mechanics broken");
    // DEFECT RUN: open at the bottom-right, dismiss at the SAME
    // proven-paintable point — asserted OUTSIDE the popup first.
    QTest::mousePress(m_window, Qt::RightButton, Qt::NoModifier, pr);
    QTRY_VERIFY2(menuVisible(), "right-click did not open the menu");
    QTest::mouseRelease(m_window, Qt::RightButton, Qt::NoModifier, pr);
    {
        const QVariant rectV = evalJs(QStringLiteral(
            "(function(){var c=canvasContextMenu.contentItem;"
            "var p=c.mapToItem(null,0,0);"
            "return p.x+\",\"+p.y+\",\"+c.width+\",\"+c.height;})()"));
        const QStringList parts = rectV.toString().split(QLatin1Char(','));
        QVERIFY2(parts.size() == 4, "popup geometry query failed");
        const QRectF popupScene(parts[0].toDouble() - 12.0,
                                parts[1].toDouble() - 12.0,
                                parts[2].toDouble() + 24.0,
                                parts[3].toDouble() + 24.0);
        QVERIFY2(!popupScene.contains(QPointF(pd)),
                 "the dismiss point sits under the popup — the leak "
                 "probe would be masked (re-place the anchor)");
    }
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier, pd);
    QTRY_VERIFY2(!menuVisible(),
                 "the outside click did not dismiss the modal menu");
    QTest::qWait(80); // give a leaked dab time to journal before reading
    QVERIFY(m_controller->readRegionImage(fullDoc, &docImg));
    QVERIFY2(!anyNonWhite(docImg),
             "the dismissal click PAINTED — the press leaked to the "
             "brush (F4)");

    // ARM 2 — POLYGON (click-session): a mid-session dismissal must not
    // add a stray vertex. Control triangle first.
    freshDoc();
    m_canvas->setProperty("tool", QStringLiteral("select.polygon"));
    const QRect full(0, 0, m_controller->docWidth(),
                     m_controller->docHeight());
    const QPoint a = m_canvas->mapToScene(QPointF(60.0, 50.0)).toPoint();
    const QPoint b = m_canvas->mapToScene(QPointF(200.0, 60.0)).toPoint();
    const QPoint c = m_canvas->mapToScene(QPointF(120.0, 160.0)).toPoint();
    // The dismiss point sits OUTSIDE the triangle: a leaked vertex at d
    // visibly changes the mask (the discriminating geometry).
    const QPoint d = m_canvas->mapToScene(QPointF(280.0, 40.0)).toPoint();
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier, a);
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier, b);
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier, c);
    QTest::mouseDClick(m_window, Qt::LeftButton, Qt::NoModifier, c);
    QTRY_VERIFY2(m_controller->property("selectionActive").toBool(),
                 "control polygon landed no selection");
    QImage maskControl;
    QVERIFY(m_controller->readSelectionImage(full, &maskControl));
    QVERIFY(applyOk("selection.none", {})
                .value(QStringLiteral("ok"))
                .toBool());
    // SENSITIVITY: the same triangle WITH a plain click at d as a real
    // vertex yields a DIFFERENT mask (d would be visible if leaked).
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier, a);
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier, b);
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier, d);
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier, c);
    QTest::mouseDClick(m_window, Qt::LeftButton, Qt::NoModifier, c);
    QTRY_VERIFY(m_controller->property("selectionActive").toBool());
    QImage maskWithD;
    QVERIFY(m_controller->readSelectionImage(full, &maskWithD));
    QVERIFY2(maskWithD != maskControl,
             "sensitivity: a real vertex at the dismiss point does not "
             "change the mask — the arm cannot discriminate");
    QVERIFY(applyOk("selection.none", {})
                .value(QStringLiteral("ok"))
                .toBool());
    // INTERRUPTED RUN: a, b, open (allowed in a click-session — no
    // button held), dismiss at d, then c and close. The mask must equal
    // the CONTROL, not the with-d variant.
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier, a);
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier, b);
    QTest::mousePress(m_window, Qt::RightButton, Qt::NoModifier, pr);
    QTRY_VERIFY2(menuVisible(),
                 "mid-session right-click did not open (click-sessions "
                 "hold no button — opens are allowed, recorded)");
    QTest::mouseRelease(m_window, Qt::RightButton, Qt::NoModifier, pr);
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier, d);
    QTRY_VERIFY(!menuVisible());
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier, c);
    QTest::mouseDClick(m_window, Qt::LeftButton, Qt::NoModifier, c);
    QTRY_VERIFY2(m_controller->property("selectionActive").toBool(),
                 "interrupted polygon landed no selection");
    QImage maskInterrupted;
    QVERIFY(m_controller->readSelectionImage(full, &maskInterrupted));
    QVERIFY2(maskInterrupted == maskControl,
             "the dismissal click fed the polygon session a stray "
             "vertex (F4) — mask differs from control");
    QVERIFY(applyOk("selection.none", {})
                .value(QStringLiteral("ok"))
                .toBool());
}

int main(int argc, char *argv[])
{
    if (qgetenv("QT_QPA_PLATFORM").isEmpty())
        qputenv("QT_QPA_PLATFORM", "offscreen");
    QQuickStyle::setStyle(QStringLiteral("Basic"));
    QGuiApplication app(argc, argv);
    TstCanvasContextMenu tc;
    QTEST_SET_MAIN_SOURCE_PATH
    return QTest::qExec(&tc, argc, argv);
}
#include "tst_canvas_context_menu.moc"
