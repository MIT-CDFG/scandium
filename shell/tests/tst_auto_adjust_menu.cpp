// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// §17.2 AUTO TONE / AUTO CONTRAST / AUTO COLOR — THE SHELL HALF'S WIRING
// NET. The engine has served image.auto_{tone,contrast,color} on main
// since before this slice (driven first-hand through the CLI apply door
// before a line of shell code was written); the shell half is the three
// Image-menu rows, the command path that issues them, and the shared
// availability row. tst_command_availability owns the enable/disable
// matrix for that row across nine document states. THIS binary owns the
// three legs that matrix cannot see:
//
//   1. GROUND + THE ECHO INSTRUMENT. All six ids this slice consumes
//      (three ui.menu.image.auto_* captions, three history.image.auto_*
//      labels) resolve to real text, with ui.dialog.cancel as a live
//      positive control — and a FABRICATED id key-echoing in the same
//      run as the negative control, so "nothing echoes" below is a
//      measurement and not a property of a blind instrument.
//   2. THE ROW IS WIRED, NOT DECORATIVE. Triggering each real MenuItem
//      appends exactly one journal entry of exactly the expected kind —
//      ENGINE truth read back through the history door, never shell
//      state.
//   3. THE APPLIED SURFACES SPEAK CAPTIONS. After a trigger the status
//      line reads "Applied: Auto Tone" and the HistoryPanel row's
//      Accessible.name — the string an AT actually speaks, and the door
//      that does NOT refuse an uncataloged id — reads "Auto Tone". A
//      long-cataloged sibling kind is driven in the same run as the
//      positive control.
//   4. THE ECHO PROBE, POINTED AT THE FAILURE THAT WOULD ACTUALLY
//      OCCUR. A control proves an instrument CAN fire; it says nothing
//      about whether it is aimed at the real failure. So slot 4 boots a
//      SECOND Main.qml against a catalog built from the real one with
//      exactly `history.image.auto_tone` REMOVED, drives the same menu
//      row, and requires the row's Accessible.name to CHANGE. That is
//      the miss this whole slice was designed against, reproduced on
//      demand rather than argued from a code read — and it is what
//      licenses slot 3's clean result.
//      ★ AMENDED 2026-09-03 (O-UI 9): this slot required the RAW ID
//      until HistoryPanel.qml gained its a11y gate, which is exactly
//      the leak Q-D35 measured as live. The removal still changes what
//      is spoken; it now yields the panel's generic cataloged title,
//      and the slot asserts "never the id" plus "no longer the
//      caption" so it cannot pass vacuously in either direction. The
//      STATUS-line arm below is deliberately untouched and still
//      carries the raw id — setStatusFromResult remains ungated, so
//      the two doors now differ and this slot proves it in one run.
//
// HONEST LIMITS (recorded, not papered over):
// - No PIXEL assertion. A §17.2 clip on a constant-color layer is a
//   degenerate histogram, so a pixel delta is not a reliable witness of
//   "the command ran"; the journal door is. Pixel CORRECTNESS is the
//   engine's own net, not this shell suite's — and this suite is
//   structurally blind to engine linkage in general.
// - Layer-kind (raster-vs-not) and lock refusals are apply-time typed
//   answers by design. There is no non-raster active layer reachable
//   from this harness's document, so the target_kind_mismatch arm is
//   UNCOVERED here, not passing.
// - Slot 4's second engine proves the ECHO reaches Accessible.name. It
//   does not prove any particular screen reader speaks it; the platform
//   leg is out of this process's reach.

#include "announcer.h"
#include "canvas_view.h"
#include "document_controller.h"
#include "i18n_catalog.h"
#include "units.h"
#include "icon_provider.h"
#include "layer_thumb_provider.h"

#include <QDir>
#include <QFile>
#include <QGuiApplication>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQmlExpression>
#include <QQuickItem>
#include <QQuickStyle>
#include <QQuickWindow>
#include <QtTest>

using namespace appname::shell;

namespace {
// ---- THE QML SCRIPT-ERROR GUARD (O-UI, C-446) ----
// A QML ReferenceError or TypeError is a REAL FAULT that Qt reports as a
// WARNING, so a suite passes while the tree it booted is partly broken.
// Three defects shipped past green suites this way in one afternoon, and
// the same silence hid AppUnits being unregistered in ten harnesses.
//
// Fails on the CLASS, not a suppression list: ReferenceError/TypeError
// only. QuickControls style notes and image-provider misses are neither,
// so they need no naming and cannot grow into an allowlist that swallows
// the next real one.
QStringList g_qmlScriptErrors;
QtMessageHandler g_prevHandler = nullptr;
void qmlErrorTrap(QtMsgType type, const QMessageLogContext &ctx,
                  const QString &msg)
{
    if (type == QtWarningMsg
        && (msg.contains(QLatin1String("ReferenceError"))
            || msg.contains(QLatin1String("TypeError"))))
        g_qmlScriptErrors << msg;
    if (g_prevHandler)
        g_prevHandler(type, ctx, msg);
}
} // namespace

namespace {

struct Row {
    const char *kind;      // engine command kind
    const char *captionId; // ui.menu.image.* caption
    const char *historyId; // history.<kind> label
    const char *caption;   // the checked-in en.json text
};

// The three rows, single-sourced for every slot below.
const Row kRows[] = {
    {"image.auto_tone", "ui.menu.image.auto_tone", "history.image.auto_tone",
     "Auto Tone"},
    {"image.auto_contrast", "ui.menu.image.auto_contrast",
     "history.image.auto_contrast", "Auto Contrast"},
    {"image.auto_color", "ui.menu.image.auto_color",
     "history.image.auto_color", "Auto Color"},
};

// An id that must never be cataloged — the negative control for the
// echo instrument. Asserted absent before it is used as one.
constexpr const char *kFabricatedId = "ui.menu.image.zzz_fabricated_control";

} // namespace

class TstAutoAdjustMenu : public QObject {
    Q_OBJECT
private slots:
    void initTestCase();
    void cleanupTestCase();

    void groundAndEchoInstrument();
    void menuTriggerAppliesTheEngineCommand();
    void appliedSurfacesRenderCaptionsNotIds();
    void echoProbeFiresWhenTheLabelIsRemoved();

private:
    // ---- world A: the REAL catalog (the shipped configuration) ----
    CatalogQml *m_i18n = nullptr;
    DocumentController *m_controller = nullptr;
    Announcer *m_announcer = nullptr;
    QQmlApplicationEngine *m_engine = nullptr;
    QQuickWindow *m_window = nullptr;

    // ---- world B: slot 4's catalog-minus-one-label ----
    CatalogQml *m_i18nB = nullptr;
    DocumentController *m_controllerB = nullptr;
    Announcer *m_announcerB = nullptr;
    QQmlApplicationEngine *m_engineB = nullptr;
    QQuickWindow *m_windowB = nullptr;

    QByteArray m_realEnJson; // the staged en.json bytes, read once

    // Boot one Main.qml world against `catalog`. Mirrors
    // shell/src/main.cpp's wiring (the tst_palette_pins harness).
    bool bootWorld(Catalog catalog, CatalogQml **i18n,
                   DocumentController **controller, Announcer **announcer,
                   QQmlApplicationEngine **engine, QQuickWindow **window);

    // Find a top MenuItem by its CATALOG KEY (never by a hardcoded
    // English string — a key-echoing catalog would then match on the
    // echoed id and read as success).
    //
    // MEASURED, NOT ASSUMED — and the reason this does not early-return
    // like the 14a helper it grew from: `Menu.itemAt(j)` INSTANTIATES
    // the delegate on demand, so a loop that returns on the first match
    // leaves every later index uncreated. Run in isolation, an
    // early-returning lookup found Auto Tone and Auto Contrast and then
    // came back NULL for Auto Color; in a full-binary run the same
    // lookup passed, because an earlier slot had already walked the bar.
    // A green that depends on slot order is not a green. So: touch EVERY
    // index in EVERY menu first, collect ALL matches, and let the caller
    // assert on the count — 0 and 2 are both failures, and a lookup that
    // silently picked the first of two duplicates would have hidden the
    // second.
    QVariantList menuItemsMatching(QQuickWindow *win,
                                   const char *catalogKey) const
    {
        const QString code = QStringLiteral(
            "(function() {"
            "  var label = AppI18n.tr(\"%1\");"
            "  var hits = [];"
            "  for (var i = 0; i < mainMenuBar.count; ++i) {"
            "    var m = mainMenuBar.menuAt(i);"
            "    if (!m) continue;"
            "    for (var j = 0; j < m.count; ++j) {"
            "      var it = m.itemAt(j);"
            "      if (it && it.text === label) hits.push(it);"
            "    }"
            "  }"
            "  return hits;"
            "})()").arg(QLatin1String(catalogKey));
        QQmlExpression expr(qmlContext(win), win, code);
        const QVariant v = expr.evaluate();
        if (expr.hasError()) {
            qWarning() << "menuItem lookup failed:" << expr.error().toString();
            return {};
        }
        return v.toList();
    }
    QObject *menuItem(QQuickWindow *win, const char *catalogKey) const
    {
        const QVariantList hits = menuItemsMatching(win, catalogKey);
        return hits.size() == 1 ? hits.first().value<QObject *>() : nullptr;
    }
    // What the menu bar ACTUALLY holds, for a failure message that says
    // what was there instead of only what was missing.
    QString dumpMenuBar(QQuickWindow *win) const
    {
        const QString code = QStringLiteral(
            "(function() {"
            "  var out = [];"
            "  for (var i = 0; i < mainMenuBar.count; ++i) {"
            "    var m = mainMenuBar.menuAt(i);"
            "    if (!m) { out.push('<menu ' + i + ' null>'); continue; }"
            "    var t = [];"
            "    for (var j = 0; j < m.count; ++j) {"
            "      var it = m.itemAt(j);"
            "      t.push(it === null ? '<null@' + j + '>'"
            "                         : (it.text === undefined ? '<sep>'"
            "                                                  : it.text));"
            "    }"
            "    out.push(m.title + ' [count=' + m.count + '] {' "
            "             + t.join(' | ') + '}');"
            "  }"
            "  return out.join('\\n');"
            "})()");
        QQmlExpression expr(qmlContext(win), win, code);
        return expr.evaluate().toString();
    }
    // Fire the row the way the menu does. `triggered()` is the signal
    // Main.qml's onTriggered handler is attached to, so this drives the
    // REAL handler — not a re-implementation of it.
    bool triggerMenuItem(QQuickWindow *win, QObject *item) const
    {
        QQmlExpression expr(qmlContext(win), item,
                            QStringLiteral("triggered()"));
        expr.evaluate();
        if (expr.hasError()) {
            qWarning() << "trigger failed:" << expr.error().toString();
            return false;
        }
        return true;
    }
    static QString accessibleName(QObject *item)
    {
        QQmlExpression expr(qmlContext(item), item,
                            QStringLiteral("Accessible.name"));
        bool undefined = false;
        const QVariant v = expr.evaluate(&undefined);
        if (expr.hasError()) {
            qWarning() << "Accessible.name read failed:"
                       << expr.error().toString();
            return QString();
        }
        return undefined ? QString() : v.toString();
    }
    // The HistoryPanel delegate for model row `i`, via the ListView's
    // own itemAtIndex (the tst_history_panel anchor).
    QQuickItem *historyRowItem(QQuickWindow *win, int i) const
    {
        QQuickItem *list =
            win->findChild<QQuickItem *>(QStringLiteral("historyList"));
        if (!list)
            return nullptr;
        QQuickItem *item = nullptr;
        QMetaObject::invokeMethod(list, "itemAtIndex",
                                  Q_RETURN_ARG(QQuickItem *, item),
                                  Q_ARG(int, i));
        return item;
    }
    static QVariantMap newestEntry(DocumentController *c)
    {
        const QVariantList rows = c->historyEntries();
        return rows.isEmpty() ? QVariantMap() : rows.last().toMap();
    }
    // Mid-gray fill so the layer carries real content before a §17.2
    // clip (one journal step, the selection-gestures macro's envelope).
    void fillCanvas(DocumentController *c, double v)
    {
        QVariantMap color;
        color.insert(QStringLiteral("model"), QStringLiteral("rgb"));
        color.insert(QStringLiteral("comps"), QVariantList{v, v, v, 0.0});
        QVariantMap params;
        params.insert(QStringLiteral("content"), QStringLiteral("color"));
        params.insert(QStringLiteral("color"), color);
        QVariantMap target;
        target.insert(QStringLiteral("layerId"),
                      c->layers().first().toMap().value(
                          QStringLiteral("layerId")));
        QVERIFY(c->applyCommand(QStringLiteral("fill"), target, params,
                                QString())
                    .value(QStringLiteral("ok"))
                    .toBool());
    }
};

bool TstAutoAdjustMenu::bootWorld(Catalog catalog, CatalogQml **i18n,
                                  DocumentController **controller,
                                  Announcer **announcer,
                                  QQmlApplicationEngine **engine,
                                  QQuickWindow **window)
{
    *i18n = new CatalogQml(std::move(catalog));
    *controller = new DocumentController(&(*i18n)->catalog());
    *announcer = new Announcer(&(*i18n)->catalog());
    (*controller)->setAnnouncer(*announcer);

    *engine = new QQmlApplicationEngine;
    (*engine)->addImageProvider(QStringLiteral("lucide"),
                                new LucideIconProvider);
    (*engine)->addImageProvider(QStringLiteral("layerthumb"),
                                new LayerThumbProvider(*controller));
    (*engine)->rootContext()->setContextProperty(QStringLiteral("AppI18n"),
                                                 *i18n);
    (*engine)->rootContext()->setContextProperty(QStringLiteral("AppDoc"),
                                                 *controller);
    // AppUnits: main.cpp:132 registers it and this harness did not, so
    // NewDocumentDialog threw two ReferenceErrors and a TypeError on every
    // boot — this suite was exercising a tree the application never has.
    (*engine)->rootContext()->setContextProperty(QStringLiteral("AppUnits"),
                                                 new UnitsQml(*engine));
    (*engine)->rootContext()->setContextProperty(QStringLiteral("AppA11y"),
                                                 *announcer);
    (*engine)->rootContext()->setContextProperty(
        QStringLiteral("AppKeys"), new QObject(*engine));
    (*engine)->loadFromModule("AppnameShell", "Main");
    if ((*engine)->rootObjects().isEmpty())
        return false;
    *window = qobject_cast<QQuickWindow *>((*engine)->rootObjects().first());
    if (!*window)
        return false;
    (*announcer)->setSubject(*window);
    return QTest::qWaitForWindowExposed(*window);
}

void TstAutoAdjustMenu::initTestCase()
{
    g_qmlScriptErrors.clear();
    g_prevHandler = qInstallMessageHandler(qmlErrorTrap);
    // The staged exe-adjacent en.json is BOTH world A's catalog source
    // (via Catalog::load, the app's own resolution order) and world B's
    // raw material. Read it once, here, so slot 4 mutates a copy of the
    // very bytes world A loaded — not an independently guessed catalog.
    const QString path =
        QDir(QCoreApplication::applicationDirPath())
            .filePath(QStringLiteral("i18n/en.json"));
    QFile f(path);
    QVERIFY2(f.open(QIODevice::ReadOnly), qPrintable(path));
    m_realEnJson = f.readAll();
    QVERIFY2(!m_realEnJson.isEmpty(), "staged en.json is empty");

    QVERIFY2(bootWorld(Catalog::load(QStringLiteral("en")), &m_i18n,
                       &m_controller, &m_announcer, &m_engine, &m_window),
             "world A (real catalog) failed to boot Main.qml");
}

void TstAutoAdjustMenu::cleanupTestCase()
{
    qInstallMessageHandler(g_prevHandler);
    QVERIFY2(g_qmlScriptErrors.isEmpty(),
             qPrintable(QStringLiteral("QML script errors during this "
                                       "suite — a green run is not a "
                                       "clean tree: ")
                        + g_qmlScriptErrors.join(QStringLiteral(" | "))));
    delete m_engineB;
    delete m_engine;
}

void TstAutoAdjustMenu::groundAndEchoInstrument()
{
    const Catalog &cat = m_i18n->catalog();
    // The echo instrument under test everywhere below: tr() returns the
    // id verbatim for an uncataloged key (Catalog::tr's documented
    // key-echo fallback), so `tr(id) == id` IS the echo predicate.
    const auto echoes = [&cat](const char *id) {
        const QString k = QLatin1String(id);
        return cat.tr(k) == k;
    };

    // NEGATIVE CONTROL FIRST — with exact matching, not containment.
    // If this does not fire, every "does not echo" result below is free.
    QVERIFY2(!cat.has(QLatin1String(kFabricatedId)),
             "the fabricated control id is CATALOGED — pick another; as "
             "it stands it cannot serve as a negative control");
    QVERIFY2(echoes(kFabricatedId),
             "the echo instrument did not fire on a known-uncataloged "
             "id — every clean result below would be vacuous");

    // POSITIVE CONTROL, same run: a long-cataloged id resolves and does
    // NOT echo. Both directions, one batch.
    QVERIFY(cat.has(QStringLiteral("ui.dialog.cancel")));
    QVERIFY2(!echoes("ui.dialog.cancel"),
             "the echo instrument fired on a cataloged id — it is not "
             "discriminating, it is stuck on");
    QCOMPARE(cat.tr(QStringLiteral("ui.dialog.cancel")),
             QStringLiteral("Cancel"));

    // THE SIX IDS THIS SLICE CONSUMES. Denominator: 3 captions +
    // 3 history labels = 6, each checked in both directions.
    int checked = 0;
    for (const Row &r : kRows) {
        QVERIFY2(cat.has(QLatin1String(r.captionId)), r.captionId);
        QVERIFY2(!echoes(r.captionId), r.captionId);
        QCOMPARE(cat.tr(QLatin1String(r.captionId)),
                 QString::fromLatin1(r.caption));
        ++checked;
        QVERIFY2(cat.has(QLatin1String(r.historyId)), r.historyId);
        QVERIFY2(!echoes(r.historyId), r.historyId);
        // The history label and the menu caption are the same words by
        // design (the panel row names the command the user invoked).
        QCOMPARE(cat.tr(QLatin1String(r.historyId)),
                 QString::fromLatin1(r.caption));
        ++checked;
    }
    QCOMPARE(checked, 6);

    // EXACTLY ONE consuming MenuItem per caption, across the whole menu
    // bar. Two-sided on purpose: zero means the row never shipped, and
    // two means the same command is offered from two places that can
    // drift apart. The `enabled` and trigger assertions in the slots
    // below address ONE object each, so this is what licenses them.
    for (const Row &r : kRows)
        QCOMPARE(menuItemsMatching(m_window, r.captionId).size(), 1);
    // Control for that count, same run: the instrument returns 0 for a
    // caption no MenuItem carries, so "1" above is a measurement.
    QCOMPARE(menuItemsMatching(m_window, kFabricatedId).size(), 0);
}

void TstAutoAdjustMenu::menuTriggerAppliesTheEngineCommand()
{
    QVERIFY(m_controller->newDocument(64, 48, QStringLiteral("white")));
    fillCanvas(m_controller, 0.5);

    int fired = 0;
    for (const Row &r : kRows) {
        QObject *item = menuItem(m_window, r.captionId);
        QVERIFY2(item, qPrintable(QStringLiteral("%1 not found. Menu bar "
                                                 "holds:\n%2")
                                      .arg(QLatin1String(r.captionId),
                                           dumpMenuBar(m_window))));
        // Guard the dimension before using it: a disabled row would make
        // the trigger a no-op and the whole slot vacuously green.
        QVERIFY2(item->property("enabled").toBool(),
                 qPrintable(QStringLiteral("%1 is disabled in a state "
                                           "where it must be enabled")
                                .arg(QLatin1String(r.captionId))));

        const int before = m_controller->historyEntries().size();
        QVERIFY(triggerMenuItem(m_window, item));

        const QVariantList after = m_controller->historyEntries();
        QCOMPARE(after.size(), before + 1);
        // ENGINE truth: the kind comes back through the history door,
        // not from anything the shell remembered.
        QCOMPARE(after.last().toMap().value(QStringLiteral("kind")).toString(),
                 QString::fromLatin1(r.kind));
        ++fired;
    }
    QCOMPARE(fired, 3);
}

void TstAutoAdjustMenu::appliedSurfacesRenderCaptionsNotIds()
{
    QVERIFY(m_controller->newDocument(64, 48, QStringLiteral("white")));
    fillCanvas(m_controller, 0.5);

    // POSITIVE CONTROL IN THE SAME RUN: a kind whose history label has
    // been cataloged since long before this slice. If the two surfaces
    // below cannot render ITS caption either, the clean results for the
    // auto rows say nothing about this slice.
    QVERIFY(m_controller
                ->applyCommand(QStringLiteral("selection.all"), {}, {},
                               QString())
                .value(QStringLiteral("ok"))
                .toBool());
    const QString controlCaption =
        m_i18n->catalog().tr(QStringLiteral("history.selection.all"));
    QVERIFY2(controlCaption != QStringLiteral("history.selection.all"),
             "the positive control's own label key-echoes — it cannot "
             "serve as a control for the rows below");
    QCOMPARE(m_controller->property("statusText").toString(),
             m_i18n->catalog().tr(QStringLiteral("ui.status.applied"),
                                  {{QStringLiteral("label"),
                                    controlCaption}}));
    {
        const QVariantList rows = m_controller->historyEntries();
        QQuickItem *row = nullptr;
        QTRY_VERIFY((row = historyRowItem(m_window, rows.size() - 1)));
        QCOMPARE(accessibleName(row), controlCaption);
    }

    // THE THREE ROWS.
    int checked = 0;
    for (const Row &r : kRows) {
        QObject *item = menuItem(m_window, r.captionId);
        QVERIFY2(item, r.captionId);
        QVERIFY(item->property("enabled").toBool());
        QVERIFY(triggerMenuItem(m_window, item));

        const QString caption = QString::fromLatin1(r.caption);
        const QString rawId = QLatin1String(r.historyId);

        // (a) THE STATUS LINE — rendered in the footer Label AND handed
        // to an AT as the canvas item's Accessible.description
        // (Main.qml). This is the surface DocumentController::
        // setStatusFromResult fills with `tr("history." + kind)`, and it
        // is UNGATED: an uncataloged label lands here verbatim.
        const QString status =
            m_controller->property("statusText").toString();
        QCOMPARE(status,
                 m_i18n->catalog().tr(QStringLiteral("ui.status.applied"),
                                      {{QStringLiteral("label"), caption}}));
        QVERIFY2(!status.contains(rawId),
                 qPrintable(QStringLiteral("status line carries the raw "
                                           "id: %1").arg(status)));

        // (b) THE HISTORY ROW'S ACCESSIBLE NAME — HistoryPanel.qml binds
        // Accessible.name to AppI18n.tr("history." + kind). Unlike the
        // announcer and statusPending, this door does NOT refuse an
        // uncataloged id; whatever tr() returns is what an AT speaks.
        const QVariantList rows = m_controller->historyEntries();
        QCOMPARE(rows.last().toMap().value(QStringLiteral("kind")).toString(),
                 QString::fromLatin1(r.kind));
        QQuickItem *row = nullptr;
        QTRY_VERIFY((row = historyRowItem(m_window, rows.size() - 1)));
        const QString spoken = accessibleName(row);
        QCOMPARE(spoken, caption);
        QVERIFY2(spoken != rawId,
                 qPrintable(QStringLiteral("history row speaks the raw "
                                           "id: %1").arg(spoken)));
        ++checked;
    }
    QCOMPARE(checked, 3);
}

void TstAutoAdjustMenu::echoProbeFiresWhenTheLabelIsRemoved()
{
    // WORLD B. Same bytes world A loaded, with exactly ONE key removed:
    // history.image.auto_tone. Everything else — including the caption
    // that puts the row in the menu — is untouched, so the ONLY thing
    // varying is the label the panel row renders. That is the dimension
    // being attributed; the rest is held constant on purpose.
    QJsonParseError err;
    QJsonDocument doc = QJsonDocument::fromJson(m_realEnJson, &err);
    QCOMPARE(err.error, QJsonParseError::NoError);
    QVERIFY(doc.isObject());
    QJsonObject obj = doc.object();
    const int keysBefore = obj.size();
    QVERIFY2(obj.contains(QLatin1String("history.image.auto_tone")),
             "the key this slot removes is not in the staged catalog — "
             "the removal would be a no-op and the probe vacuous");
    obj.remove(QLatin1String("history.image.auto_tone"));
    QCOMPARE(obj.size(), keysBefore - 1);
    // Held constant, and asserted so: the caption must survive, or the
    // row would vanish from the menu for the WRONG reason.
    QVERIFY(obj.contains(QLatin1String("ui.menu.image.auto_tone")));
    // Held constant, and asserted so: the sibling label must survive, or
    // "only auto_tone echoes" below proves nothing about attribution.
    QVERIFY(obj.contains(QLatin1String("history.image.auto_contrast")));

    Catalog cut = Catalog::fromTexts(
        {{QStringLiteral("en"),
          QString::fromUtf8(QJsonDocument(obj).toJson(QJsonDocument::Compact))}});
    QVERIFY(!cut.has(QStringLiteral("history.image.auto_tone")));
    QVERIFY(cut.has(QStringLiteral("ui.menu.image.auto_tone")));

    QVERIFY2(bootWorld(std::move(cut), &m_i18nB, &m_controllerB,
                       &m_announcerB, &m_engineB, &m_windowB),
             "world B failed to boot Main.qml");
    QVERIFY(m_controllerB->newDocument(64, 48, QStringLiteral("white")));
    fillCanvas(m_controllerB, 0.5);

    QObject *item = menuItem(m_windowB, "ui.menu.image.auto_tone");
    QVERIFY2(item, "the Auto Tone row is missing in world B — its "
                   "caption was deliberately left cataloged");
    QVERIFY(item->property("enabled").toBool());
    QVERIFY(triggerMenuItem(m_windowB, item));

    const QVariantList rows = m_controllerB->historyEntries();
    QCOMPARE(rows.last().toMap().value(QStringLiteral("kind")).toString(),
             QStringLiteral("image.auto_tone"));

    // THE PROBE FIRES — AND WHAT IT FIRES INTO CHANGED, 2026-09-03 (O-UI 9).
    // This slot used to require the accessible name to BE the raw id. That
    // was an accurate pin on the behaviour of the day and it is now the
    // behaviour Q-D22's sibling dispatch closed: HistoryPanel.qml gates
    // Accessible.name so an uncataloged kind is NEVER spoken as its key,
    // falling back to the panel's own cataloged title. Q-D35 measured the
    // harm this leak does — "a blind user doing a clone stroke hears the
    // string history.clone.stroke".
    //
    // THE SLOT'S PURPOSE IS UNCHANGED AND STILL MET: removing exactly one
    // key must still CHANGE what the row speaks, and it does — it stops
    // being the caption. What it must never do again is speak the id, so
    // that is asserted directly rather than by equality with the id.
    QQuickItem *row = nullptr;
    QTRY_VERIFY((row = historyRowItem(m_windowB, rows.size() - 1)));
    const QString spokenB = accessibleName(row);
    QVERIFY2(spokenB != QStringLiteral("history.image.auto_tone"),
             qPrintable(QStringLiteral("the a11y gate leaked the raw id: %1")
                            .arg(spokenB)));
    QVERIFY2(spokenB != QStringLiteral("Auto Tone"),
             "world B still speaks the caption — the key removal was a "
             "no-op and this probe is vacuous");
    QCOMPARE(spokenB, m_i18n->catalog().tr(
                          QStringLiteral("ui.panel.history.title")));
    // ...and the status line carries it too, through the ungated
    // setStatusFromResult label.
    QVERIFY(m_controllerB->property("statusText")
                .toString()
                .contains(QStringLiteral("history.image.auto_tone")));

    // ATTRIBUTION, same run: the sibling row whose label was left in
    // place does NOT echo. Without this the slot would only show that
    // SOMETHING in world B echoes, not that the removed key is why.
    QObject *sibling = menuItem(m_windowB, "ui.menu.image.auto_contrast");
    QVERIFY(sibling);
    QVERIFY(triggerMenuItem(m_windowB, sibling));
    const QVariantList rows2 = m_controllerB->historyEntries();
    QCOMPARE(rows2.last().toMap().value(QStringLiteral("kind")).toString(),
             QStringLiteral("image.auto_contrast"));
    QQuickItem *row2 = nullptr;
    QTRY_VERIFY((row2 = historyRowItem(m_windowB, rows2.size() - 1)));
    QCOMPARE(accessibleName(row2), QStringLiteral("Auto Contrast"));
}

int main(int argc, char *argv[])
{
    if (qgetenv("QT_QPA_PLATFORM").isEmpty())
        qputenv("QT_QPA_PLATFORM", "offscreen");
    QQuickStyle::setStyle(QStringLiteral("Basic"));
    QGuiApplication app(argc, argv);
    TstAutoAdjustMenu tc;
    QTEST_SET_MAIN_SOURCE_PATH
    return QTest::qExec(&tc, argc, argv);
}

#include "tst_auto_adjust_menu.moc"
