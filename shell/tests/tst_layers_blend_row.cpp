// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// D2 — THE LAYERS-PANEL BLEND CONTROL (operator ruling Q-D2(a), 2026-08-29).
//
// WHY ITS OWN FILE. The two existing layers-panel suites (mask route, fill
// route) are O-UI's and they are ACTIVE in them tonight. A new arm dropped
// into a peer's live file is the lane-clobber this fleet keeps paying for,
// so the blend row gets its own binary and joins the shared foreach in
// CMakeLists rather than carrying a copied block.
//
// WHAT IT PROVES, AND THE ORDER MATTERS. Every claim is read back through
// the doc-info door (engine truth), never from shell state:
//   1. the control exists and carries the CONTRACT's 27 tags in the
//      CONTRACT's order -- pinned against layer.set_props.yaml, so a
//      contract change reds here instead of silently shipping a short list;
//   2. choosing Luminosity actually sets Luminosity ON THE ENGINE;
//   3. the flagship demo step -- set it and put it back -- lands TWO
//      history entries, which is what the script promises the operator;
//   4. the label is translated while the wire tag is not, which is the
//      camelCase/snake_case seam the row derives rather than tabulates;
//   5. the control FOLLOWS engine truth when the blend is changed by
//      something other than the control.
//
// (5) is the one that would otherwise be assumed. A combo that submits
// correctly and never re-reads is a control that lies the moment anything
// else moves the value -- undo, a script, another panel.

#include "announcer.h"
#include "canvas_view.h"
#include "document_controller.h"
#include "i18n_catalog.h"
#include "icon_provider.h"
#include "layer_thumb_provider.h"
#include "units.h"

#include <QGuiApplication>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSignalSpy>
#include <QTest>

using namespace appname::shell;

namespace {
// Same trap the fill-route suite installs: fail on the CLASS
// (ReferenceError/TypeError), not on a suppression list that can grow
// into an allowlist swallowing the next real one.
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

// THE CONTRACT'S OWN ENUM, IN THE CONTRACT'S OWN ORDER
// (contracts/commands/layer.set_props.yaml). Transcribed here so the test
// can fail when the QML and the contract disagree -- if this list were
// read FROM the QML it would agree with it by construction and prove
// nothing, which is the vacuous-control shape.
const char *const kWireTags[] = {
    "normal", "dissolve", "darken", "multiply", "colorBurn", "linearBurn",
    "darkerColor", "lighten", "screen", "colorDodge", "linearDodge",
    "lighterColor", "overlay", "softLight", "hardLight", "vividLight",
    "linearLight", "pinLight", "hardMix", "difference", "exclusion",
    "subtract", "divide", "hue", "saturation", "color", "luminosity"
};
const int kWireTagCount = int(sizeof(kWireTags) / sizeof(kWireTags[0]));
} // namespace

class TstLayersBlendRow : public QObject
{
    Q_OBJECT

private slots:
    void initTestCase();
    void init();
    void cleanupTestCase();

    void theControlCarriesTheContractsTagsInTheContractsOrder();
    void choosingLuminositySetsLuminosityOnTheEngine();
    void setItAndPutItBackLandsTwoHistoryEntries();
    void theLabelIsTranslatedWhileTheWireTagIsNot();
    void theControlFollowsTruthItDidNotItselfSubmit();
    void aRepeatedRefusalRefusesAgainRatherThanGoingQuiet();

private:
    CatalogQml *m_i18n = nullptr;
    DocumentController *m_controller = nullptr;
    Announcer *m_announcer = nullptr;
    QQmlApplicationEngine *m_engine = nullptr;
    QQuickWindow *m_window = nullptr;
    QQuickItem *m_combo = nullptr;

    // ENGINE truth for the active layer's blend. Read through the
    // doc-info door, never from the combo we are testing.
    QString engineBlend() const
    {
        const QVariantList rows = m_controller->layers();
        const int idx = m_controller->activeLayerIndex();
        if (idx < 0 || idx >= rows.size())
            return QString();
        return rows.at(idx).toMap().value(QStringLiteral("blend")).toString();
    }
    QString activeLayerId() const
    {
        const QVariantList rows = m_controller->layers();
        const int idx = m_controller->activeLayerIndex();
        if (idx < 0 || idx >= rows.size())
            return QString();
        return rows.at(idx).toMap().value(QStringLiteral("layerId")).toString();
    }
    int historyCount() const { return m_controller->historyEntries().size(); }
    void choose(int index)
    {
        QVERIFY(m_combo);
        QVERIFY(QMetaObject::invokeMethod(m_combo, "activated",
                                          Q_ARG(int, index)));
        QCoreApplication::processEvents();
    }
    int indexOfTag(const QString &tag) const
    {
        for (int i = 0; i < kWireTagCount; ++i)
            if (tag == QLatin1String(kWireTags[i]))
                return i;
        return -1;
    }
};

void TstLayersBlendRow::initTestCase()
{
    g_prevHandler = qInstallMessageHandler(qmlErrorTrap);

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
}

void TstLayersBlendRow::init()
{
    QVERIFY(m_controller->newDocument(64, 48, QStringLiteral("white")));
    QCoreApplication::processEvents();
    // Resolved per-test rather than once in initTestCase: the row is
    // `visible: panel.activeRow() !== null`, so with no document there is
    // nothing to find, and caching a pointer from an empty boot would
    // make every later test depend on a lookup that happened too early.
    m_combo = m_window->findChild<QQuickItem *>(
        QStringLiteral("layerBlendCombo"));
    QVERIFY2(m_combo,
             "layerBlendCombo not in the tree -- the blend control is "
             "absent, which is the state D2 exists to end");
}

void TstLayersBlendRow::cleanupTestCase()
{
    QVERIFY2(g_qmlScriptErrors.isEmpty(),
             qPrintable(QStringLiteral("QML script errors: %1")
                            .arg(g_qmlScriptErrors.join(QLatin1String(" | ")))));
    delete m_engine;
}

void TstLayersBlendRow::theControlCarriesTheContractsTagsInTheContractsOrder()
{
    const QVariant tagsVar = m_combo->property("tags");
    QVERIFY2(tagsVar.isValid(), "the control exposes no `tags` property");
    const QVariantList tags = tagsVar.toList();

    // POPULATION FIRST, then membership. A control that offered 26 of 27
    // in the right order would pass a per-item loop that stopped at the
    // shorter list.
    QCOMPARE(tags.size(), kWireTagCount);
    for (int i = 0; i < kWireTagCount; ++i)
        QCOMPARE(tags.at(i).toString(), QLatin1String(kWireTags[i]));

    // The visible model must be the same length -- one label per tag.
    QCOMPARE(m_combo->property("count").toInt(), kWireTagCount);
}

void TstLayersBlendRow::choosingLuminositySetsLuminosityOnTheEngine()
{
    const int lum = indexOfTag(QStringLiteral("luminosity"));
    QVERIFY(lum >= 0);
    QCOMPARE(engineBlend(), QStringLiteral("normal")); // ground, measured
    choose(lum);
    QTRY_COMPARE(engineBlend(), QStringLiteral("luminosity"));
}

void TstLayersBlendRow::setItAndPutItBackLandsTwoHistoryEntries()
{
    // THE FLAGSHIP DEMO STEP, driven exactly as the script describes it:
    // "set the blend dropdown to Luminosity and back". The operator's
    // payoff is that the canvas visibly changes and visibly returns, and
    // that BOTH moves are undoable -- so the assertion is on the history
    // count, not merely on the final blend matching the initial one.
    const int before = historyCount();
    const int lum = indexOfTag(QStringLiteral("luminosity"));
    const int nrm = indexOfTag(QStringLiteral("normal"));
    QVERIFY(lum >= 0 && nrm >= 0);

    choose(lum);
    QTRY_COMPARE(engineBlend(), QStringLiteral("luminosity"));
    choose(nrm);
    QTRY_COMPARE(engineBlend(), QStringLiteral("normal"));

    QCOMPARE(historyCount(), before + 2);
}

void TstLayersBlendRow::theLabelIsTranslatedWhileTheWireTagIsNot()
{
    // THE SEAM: the enum is camelCase and the catalog keys are
    // snake_case, so `colorBurn` must reach `ui.blend.color_burn`. The row
    // DERIVES that rather than carrying a 27-row map, and this is the arm
    // that proves the derivation reaches the multi-word cases -- a
    // single-word check (`overlay` -> "Overlay") passes even if the
    // transform is the identity, so it could not discriminate.
    const QVariantList model = m_combo->property("model").toList();
    QCOMPARE(model.size(), kWireTagCount);

    const int cb = indexOfTag(QStringLiteral("colorBurn"));
    QVERIFY(cb >= 0);
    QCOMPARE(model.at(cb).toString(), QStringLiteral("Color Burn"));
    QVERIFY2(model.at(cb).toString() != QLatin1String(kWireTags[cb]),
             "the label equals the wire tag -- the catalog lookup fell "
             "through and the panel is showing machine identifiers");

    // And a second multi-word case, so the pass does not rest on one row.
    const int lc = indexOfTag(QStringLiteral("lighterColor"));
    QVERIFY(lc >= 0);
    QCOMPARE(model.at(lc).toString(), QStringLiteral("Lighter Color"));
}

void TstLayersBlendRow::theControlFollowsTruthItDidNotItselfSubmit()
{
    // A combo that submits correctly and never re-reads lies the moment
    // anything else moves the value. Change the blend WITHOUT touching the
    // control -- straight through the apply door, as a script or another
    // panel would -- and require the control to catch up.
    const QString id = activeLayerId();
    QVERIFY(!id.isEmpty());
    const int ovl = indexOfTag(QStringLiteral("overlay"));
    QVERIFY(ovl >= 0);
    QVERIFY(m_combo->property("currentIndex").toInt() != ovl); // control

    QVariantMap target;
    target.insert(QStringLiteral("layerId"), id);
    QVariantMap params;
    params.insert(QStringLiteral("blend"), QStringLiteral("overlay"));
    m_controller->applyCommand(QStringLiteral("layer.set_props"), target,
                               params, QString());
    QCoreApplication::processEvents();

    QTRY_COMPARE(engineBlend(), QStringLiteral("overlay"));
    QTRY_COMPARE(m_combo->property("currentIndex").toInt(), ovl);
}

void TstLayersBlendRow::aRepeatedRefusalRefusesAgainRatherThanGoingQuiet()
{
    // §8b's LAST UNVERIFIED PROPERTY. The ledger recorded at entry 299.4 that
    // P1-2's toast had "never been seen … no offscreen case asserts it
    // appears, RE-FIRES, or wraps." Appears and wraps were observed in tour
    // frames on 08-29; RE-FIRES is this.
    //
    // ★★★ WHY IT IS THE ONE WORTH A TEST RATHER THAN A FRAME. A toast that
    // fires once and then goes quiet for an IDENTICAL second refusal is
    // indistinguishable, on screen, from one that is still showing the FIRST
    // refusal -- the pixels are the same. A frame cannot tell those apart;
    // the signal can. §8b tells the presenter "press it twice", so the
    // difference is exactly what the audience is asked to look at.
    //
    // This lives in the layers-panel suite because the refusal it uses is a
    // layers-panel one -- a second raster mask on a layer that already has
    // one -- and because the two O-UI layer suites are theirs.
    QSignalSpy refused(m_controller, &DocumentController::commandRefused);
    QVERIFY(refused.isValid());

    const QString id = activeLayerId();
    QVERIFY(!id.isEmpty());
    QVariantMap target;
    target.insert(QStringLiteral("layerId"), id);
    QVariantMap params;
    params.insert(QStringLiteral("kind"), QStringLiteral("raster"));

    // FIRST mask must SUCCEED -- otherwise the two refusals below are both
    // "no mask here" and the case proves nothing about repetition.
    params.insert(QStringLiteral("newMaskId"), m_controller->newUuid());
    QVERIFY2(m_controller->applyCommand(QStringLiteral("mask.add"), target, params, {})
                 .value(QStringLiteral("ok")).toBool(),
             "setup failed: the first mask.add was refused, so this case cannot "
             "test a REPEATED refusal");
    QCoreApplication::processEvents();
    refused.clear();

    // SECOND: refused, and the code is the one §8b prints for the operator.
    params.insert(QStringLiteral("newMaskId"), m_controller->newUuid());
    m_controller->applyCommand(QStringLiteral("mask.add"), target, params, {});
    QCoreApplication::processEvents();
    QCOMPARE(refused.count(), 1);
    QCOMPARE(refused.at(0).at(0).toString(),
             QStringLiteral("error.command.target_in_use"));

    // THIRD: the same refusal again. The signal must fire a SECOND time --
    // this is what makes the toast re-rise instead of silently ageing out.
    params.insert(QStringLiteral("newMaskId"), m_controller->newUuid());
    m_controller->applyCommand(QStringLiteral("mask.add"), target, params, {});
    QCoreApplication::processEvents();
    QVERIFY2(refused.count() == 2,
             qPrintable(QStringLiteral("a repeated identical refusal emitted %1 "
                                       "commandRefused signal(s), not 2 — the "
                                       "second press would go quiet and §8b tells "
                                       "the presenter to make it")
                            .arg(refused.count())));
    QCOMPARE(refused.at(1).at(0).toString(),
             QStringLiteral("error.command.target_in_use"));
    // The mechanism text must be carried both times, not just the first --
    // a second emission with an empty message would re-raise a blank toast.
    QVERIFY2(!refused.at(1).at(1).toString().isEmpty(),
             "the repeat carried no message; the toast would re-rise empty");
}

QTEST_MAIN(TstLayersBlendRow)
#include "tst_layers_blend_row.moc"
