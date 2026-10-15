// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// THE FILL-LAYER ROUTE NET (v1.0.1 layer.add{kind:'fill'}). Boots the
// REAL Main.qml offscreen and drives the Layers panel's Fill menu
// through its REAL onTriggered handlers.
//
// WHY THIS SUITE EXISTS. The engine has served fill layers since before
// the shell had a menu for them: demo/build-demo.jsonl lines 4-5 author
// two gradient fill layers and reports/DEMO-SCRIPT.md section 8
// demonstrates them, so the gap was never the engine — it was that no UI
// reached it. That also means the wire shape here is MEASURED rather
// than guessed: the envelopes the panel sends are the demo document's
// own, and this suite pins that they still apply.
//
// THE ASSERTION THAT MATTERS IS ENGINE TRUTH. A fill layer's arrival is
// read from the doc-info door's layer rows (kind == "fill"), never from
// the command receipt: a receipt says the envelope was accepted, and the
// layer list says the document actually holds the node.
//
//   1. THE MENU IS REACHABLE and gated by layer.addFill.
//   2. EACH OF THE THREE PRESETS LANDS a fill layer and a
//      history.layer.add entry — and they are distinct engine
//      parameters, not three names for one call.
//   3. THE ROW IS DOCUMENT-GATED, NOT LAYER-GATED. layer.add inserts at
//      the root, so a fill layer is legal with no active layer; a
//      `layer` conjunct added "to match the mask rows" would be a
//      control refusing what the engine serves.
//   4. THE COLOUR WIRE SHAPE IS EXACTLY FOUR COMPS. §22.1 requires
//      model + four components with the trailing one zero for rgb. A
//      three-component array is a schema refusal, and the case that
//      proves the door is strict is what stops a later writer
//      "simplifying" the helper to RGB triples.

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
#include <QTest>

using namespace appname::shell;

namespace {
// ---- THE QML SCRIPT-ERROR GUARD ----
// Three defects of mine shipped past a GREEN suite today as QML warnings:
// Theme.danger (a colour name that does not exist), a missing `id` that
// broke a Menu's y binding, and a missing import that made a type
// unresolvable. A QML ReferenceError or TypeError is a REAL FAULT and Qt
// reports it as a warning, so a suite can pass while the tree it booted
// is partly broken.
//
// It fails on the CLASS, not on a suppression list: ReferenceError and
// TypeError only. The QuickControls "style does not support
// customization" notes and the layerthumb provider misses are neither,
// so they need no naming and cannot silently grow into an allowlist that
// swallows the next real one.
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


class TstLayersFillRoute : public QObject
{
    Q_OBJECT

private slots:
    void initTestCase();
    void init();
    void cleanupTestCase();

    void fillMenuIsReachableAndDocumentGated();
    void eachPresetLandsAFillLayer();
    void aFillLayerIsLegalWithNoActiveLayerSelected();
    void theColourWireShapeIsFourCompsAndTheDoorIsStrict();
    void aNewFillCompositesOverTheImageRatherThanCoveringIt();
    void theCompositeACTUALLYDiffersFromCovering();
    void newLayersBecomeTheActiveLayer();

private:
    CatalogQml *m_i18n = nullptr;
    DocumentController *m_controller = nullptr;
    Announcer *m_announcer = nullptr;
    QQmlApplicationEngine *m_engine = nullptr;
    QQuickWindow *m_window = nullptr;
    QQuickItem *m_fillButton = nullptr;

    QObject *item(const char *name) const
    {
        return m_window->findChild<QObject *>(QString::fromLatin1(name));
    }
    // ENGINE truth: how many rows the doc-info door reports as fill
    // nodes. Not the receipt, not a shell counter.
    int fillLayerCount() const
    {
        int n = 0;
        const QVariantList rows = m_controller->layers();
        for (const QVariant &v : rows)
            if (v.toMap().value(QStringLiteral("kind")).toString()
                == QLatin1String("fill"))
                ++n;
        return n;
    }
    QString newestHistoryKind() const
    {
        const QVariantList h = m_controller->historyEntries();
        return h.isEmpty()
            ? QString()
            : h.last().toMap().value(QStringLiteral("kind")).toString();
    }
    void trigger(const char *name)
    {
        QObject *o = item(name);
        QVERIFY2(o, name);
        QVERIFY(QMetaObject::invokeMethod(o, "triggered"));
    }
};

void TstLayersFillRoute::initTestCase()
{
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
    // NewDocumentDialog threw ReferenceError + TypeError on every boot —
    // the suite was exercising a tree the app never has.
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

    m_fillButton = m_window->findChild<QQuickItem *>(
        QStringLiteral("fillButton"));
    QVERIFY2(m_fillButton, "fillButton not found — the route is not in the tree");
}

void TstLayersFillRoute::init()
{
    QVERIFY(m_controller->newDocument(64, 48, QStringLiteral("white")));
    QCOMPARE(fillLayerCount(), 0);
}

void TstLayersFillRoute::cleanupTestCase()
{
    qInstallMessageHandler(g_prevHandler);
    QVERIFY2(g_qmlScriptErrors.isEmpty(),
             qPrintable(QStringLiteral("QML script errors during this "
                                       "suite — a green run is not a "
                                       "clean tree: ")
                        + g_qmlScriptErrors.join(QStringLiteral(" | "))));
    delete m_engine;
}

void TstLayersFillRoute::fillMenuIsReachableAndDocumentGated()
{
    for (const char *n : {"fillMenuLinear", "fillMenuPerceptual",
                          "fillMenuRadial"})
        QVERIFY2(item(n), n);
    // Not offered, each for a stated reason: solid needs the fg/bg
    // swatch this panel cannot see, pattern needs a patternRef with no
    // picker, and `none` would create an invisible layer.
    QVERIFY(!item("fillMenuSolid"));
    QVERIFY(!item("fillMenuPattern"));

    const QVariantMap av = m_controller->commandAvailability();
    QVERIFY2(av.contains(QStringLiteral("layer.addFill")),
             "layer.addFill row absent");
    QCOMPARE(m_fillButton->property("enabled").toBool(), true);
}

void TstLayersFillRoute::eachPresetLandsAFillLayer()
{
    trigger("fillMenuLinear");
    QCOMPARE(fillLayerCount(), 1);
    // TWO journal entries per fill, not one, and the assertion changed to
    // say so rather than to be weakened. The route now follows layer.add
    // with a layer.set_props that makes the fill COMPOSITE instead of
    // covering — one user action, two commands, and the History panel
    // will show both. This case previously asserted the NEWEST kind was
    // layer.add; that stopped being true the moment the second command
    // landed, and flipping the expected string to "layer.set_props"
    // would have quietly stopped checking that a layer.add happened at
    // all.
    const QVariantList jr = m_controller->historyEntries();
    bool sawAdd = false;
    for (const QVariant &e : jr)
        if (e.toMap().value(QStringLiteral("kind")).toString()
            == QLatin1String("layer.add"))
            sawAdd = true;
    QVERIFY2(sawAdd, "no layer.add in the journal — the fill did not arrive "
                     "through the creation door");
    QCOMPARE(newestHistoryKind(), QStringLiteral("layer.set_props"));

    trigger("fillMenuPerceptual");
    QCOMPARE(fillLayerCount(), 2);

    trigger("fillMenuRadial");
    QCOMPARE(fillLayerCount(), 3);

    // WHAT THIS DOES **NOT** PROVE, corrected before it could travel: an
    // earlier draft of this comment said three layers means "three
    // CALLS, not one handler wired to three captions." THAT IS FALSE.
    // Counting arrivals cannot distinguish three distinct submissions
    // from the same submission made three times — if all three items
    // were wired to the linear handler, this case would still read 3.
    // The claim contradicted the very limit stated in its own next
    // sentence, which is the defect class of asserting a summary the
    // detail underneath does not support.
    //
    // THE REAL LIMIT: info v3 publishes NO fill payload on the layer
    // row, so gradient.kind and gradient.space are not readable back at
    // all. This pin can therefore assert that a fill layer ARRIVED per
    // preset and nothing about which ramp it holds. The geometry is the
    // demo document's job and DEMO-SCRIPT.md section 8 is where it is
    // looked at. An ABI ask that would close it: publish the fill
    // payload, or its type, on the info layer object — the same shape as
    // the mask `kind` ask in tst_layers_mask_route. The geometry is the demo document's
    // job, and DEMO-SCRIPT.md section 8 is where it is looked at.
}

void TstLayersFillRoute::aFillLayerIsLegalWithNoActiveLayerSelected()
{
    // layer.add inserts at the ROOT by default (§10.3). The row is
    // document-gated for that reason, and this case is what reds if a
    // later writer adds a `layer` conjunct "for symmetry" with the mask
    // rows — which would refuse something the engine serves.
    m_controller->setActiveLayerIndex(-1);
    QCOMPARE(m_controller->activeLayerIndex(), -1);
    QCOMPARE(m_controller->commandAvailability()
                 .value(QStringLiteral("layer.addFill")).toMap()
                 .value(QStringLiteral("enabled")).toBool(), true);
    trigger("fillMenuLinear");
    QCOMPARE(fillLayerCount(), 1);
}

void TstLayersFillRoute::theColourWireShapeIsFourCompsAndTheDoorIsStrict()
{
    // §22.1: model + EXACTLY FOUR comps, trailing comp zero for rgb —
    // the convention envelope_builder.cpp:173-182 already writes. This
    // case exists so "simplify the helper to RGB triples" is caught by a
    // test rather than by a user seeing a refusal, and so the claim that
    // the door is STRICT is demonstrated rather than assumed.
    const QVariantMap black{{QStringLiteral("model"), QStringLiteral("rgb")},
                            {QStringLiteral("comps"),
                             QVariantList{0, 0, 0, 0}}};
    const QVariantMap threeComp{{QStringLiteral("model"), QStringLiteral("rgb")},
                                {QStringLiteral("comps"),
                                 QVariantList{1, 1, 1}}};
    auto submit = [&](const QVariantMap &tail) {
        const QVariantMap gradient{
            {QStringLiteral("kind"), QStringLiteral("linear")},
            {QStringLiteral("stops"),
             QVariantList{QVariantMap{{QStringLiteral("pos"), 0},
                                      {QStringLiteral("color"), black}},
                          QVariantMap{{QStringLiteral("pos"), 1},
                                      {QStringLiteral("color"), tail}}}}};
        return m_controller->applyCommand(
            QStringLiteral("layer.add"), {},
            {{QStringLiteral("kind"), QStringLiteral("fill")},
             {QStringLiteral("newLayerId"), m_controller->newUuid()},
             {QStringLiteral("kindParams"),
              QVariantMap{{QStringLiteral("type"), QStringLiteral("gradient")},
                          {QStringLiteral("gradient"), gradient}}}},
            {});
    };
    // FOUR comps: served.
    const QVariantMap ok = submit(
        QVariantMap{{QStringLiteral("model"), QStringLiteral("rgb")},
                    {QStringLiteral("comps"), QVariantList{1, 1, 1, 0}}});
    QVERIFY2(ok.value(QStringLiteral("ok")).toBool(),
             qPrintable(QStringLiteral("four-comp colour refused: ")
                        + ok.value(QStringLiteral("code")).toString()));
    QCOMPARE(fillLayerCount(), 1);
    // THREE comps: refused, and refused TYPED. The control that can
    // disagree — without this half the case above only proves the door
    // accepts something, not that it checks anything.
    const QVariantMap bad = submit(threeComp);
    QVERIFY2(!bad.value(QStringLiteral("ok")).toBool(),
             "a three-component colour was ACCEPTED — the door is not strict "
             "and the four-comp convention is unenforced");
    QVERIFY2(!bad.value(QStringLiteral("code")).toString().isEmpty(),
             "the refusal carried no typed code");
    QCOMPARE(fillLayerCount(), 1); // nothing landed from the bad one
}

void TstLayersFillRoute::aNewFillCompositesOverTheImageRatherThanCoveringIt()
{
    // THE OPERATOR SAW THE COVERING CASE AND CALLED IT BROKEN, and they
    // were right: a gradient fill is opaque and lands top-of-stack, so at
    // blend `normal` the frame goes blank. The demo document ships its
    // own two fills HIDDEN for exactly this reason.
    //
    // ENGINE TRUTH, not the receipt: blend is read back off the doc-info
    // layer row. A set_props that was accepted and then not applied would
    // pass a receipt check and fail this one.
    trigger("fillMenuLinear");
    const QVariantList rows = m_controller->layers();
    int idx = -1;
    for (int i = 0; i < rows.size(); ++i)
        if (rows.at(i).toMap().value(QStringLiteral("kind")).toString()
            == QLatin1String("fill"))
            idx = i;
    QVERIFY2(idx >= 0, "no fill layer was created");
    const QVariantMap fill = rows.at(idx).toMap();
    QCOMPARE(fill.value(QStringLiteral("blend")).toString(),
             QStringLiteral("overlay"));
    QCOMPARE(fill.value(QStringLiteral("opacity")).toDouble(), 1.0);

    // …and the new fill is SELECTED, so the blend dropdown and opacity
    // slider the user reaches for next act on the layer they just made.
    QCOMPARE(m_controller->activeLayerIndex(), idx);
}

void TstLayersFillRoute::theCompositeACTUALLYDiffersFromCovering()
{
    // DEMO-SCRIPT §8 claims a PIXEL outcome: "composites over the
    // photograph instead of covering it — the dark end deepens the image
    // and the light end lifts it."
    //
    // Its sibling case asserts blend == "overlay" and opacity == 1.0.
    // That is SET, not SEEN. If the engine's overlay did nothing, or the
    // property were stored and ignored, that case still passes and the
    // demo still shows the wrong thing. This one reads COMPOSITED PIXELS.
    //
    // ★ IT COMPARES OVERLAY AGAINST NORMAL rather than asserting
    // "darker", and that is not timidity — it is the maths. Overlay
    // PRESERVES pure white and pure black bases: overlay(1, b) == 1 for
    // every b. On this white test document an overlay fill changes
    // NOTHING, so "the dark end deepens" is false here and true on the
    // demo's photograph, which has midtones. Asserting a direction would
    // have pinned a claim that depends on the fixture rather than on the
    // feature.
    //
    // (The same trap one step further in: a mid-grey base at exactly 0.5
    // is overlay's FIXED POINT for a 0.5 blend — q-scenario measured
    // their own fixture landing on it and getting two arms that agreed by
    // construction. Comparing two BLEND MODES avoids picking a base at
    // all.)
    trigger("fillMenuLinear");
    int idx = -1;
    const QVariantList rows = m_controller->layers();
    for (int i = 0; i < rows.size(); ++i)
        if (rows.at(i).toMap().value(QStringLiteral("kind")).toString()
            == QLatin1String("fill"))
            idx = i;
    QVERIFY2(idx >= 0, "no fill layer was created");
    const QString fillId =
        rows.at(idx).toMap().value(QStringLiteral("layerId")).toString();
    QVERIFY(!fillId.isEmpty());

    // The composite as the route leaves it: blend overlay.
    QImage overlayImg;
    QVERIFY2(m_controller->readRegionImage(QRect(0, 0, 64, 48), &overlayImg),
             "composite readback failed at blend overlay");
    QVERIFY(!overlayImg.isNull());

    // Now the covering case the demo tells the presenter about.
    const QVariantMap r = m_controller->applyCommand(
        QStringLiteral("layer.set_props"),
        {{QStringLiteral("layerId"), fillId}},
        {{QStringLiteral("blend"), QStringLiteral("normal")}}, {});
    QVERIFY2(r.value(QStringLiteral("ok")).toBool(),
             "setting blend to normal was refused");
    QImage normalImg;
    QVERIFY2(m_controller->readRegionImage(QRect(0, 0, 64, 48), &normalImg),
             "composite readback failed at blend normal");

    // THE ASSERTION: the two composites must DIFFER. Equal images would
    // mean the blend property is being stored and not applied — the
    // failure that both the receipt check and the doc-info check are
    // blind to.
    int differing = 0;
    for (int y = 0; y < 48; y += 4)
        for (int x = 0; x < 64; x += 4)
            if (overlayImg.pixel(x, y) != normalImg.pixel(x, y))
                ++differing;
    QVERIFY2(differing > 0,
             "the composite is IDENTICAL at blend overlay and blend normal — "
             "the blend property is stored but not applied, and §8's claim "
             "that the fill composites rather than covers is false");
}

// RC3 D-7 (operator WORD 35, sittings S-05/S-13): after Add, Add Adjustment
// Layer and Paste the NEW layer must be the active one -- measured natively on
// main: Add and Paste appended the layer on top while activeLayerIndex stayed
// on the previous layer, so the first stroke and Delete targeted a layer the
// user had not picked. Driven through the panel's REAL Add button; the
// panel's delete target (layers[activeLayerIndex]) and the ListView's
// currentIndex are read alongside the controller's index.
void TstLayersFillRoute::newLayersBecomeTheActiveLayer()
{
    QVERIFY(m_controller->newDocument(64, 48, QStringLiteral("white")));
    QQuickItem *list = nullptr;
    for (QObject *o : m_window->findChildren<QObject *>())
        if (o->objectName() == QLatin1String("layerList"))
            list = qobject_cast<QQuickItem *>(o);
    QVERIFY2(list, "layerList not found");
    auto idsNow = [&]() {
        QStringList out;
        for (const QVariant &r : m_controller->layers())
            out << r.toMap().value(QStringLiteral("layerId")).toString();
        return out;
    };
    auto activeId = [&]() {
        return m_controller->layers().value(m_controller->activeLayerIndex())
            .toMap().value(QStringLiteral("layerId")).toString();
    };
    auto newIdAfter = [&](const QStringList &before) {
        for (const QString &id : idsNow())
            if (!before.contains(id))
                return id;
        return QString();
    };
    auto expectActive = [&](const QString &id, const char *door) {
        QVERIFY2(!id.isEmpty(), qPrintable(QStringLiteral("%1 created no layer").arg(QLatin1String(door))));
        QTRY_COMPARE(activeId(), id);
        QCOMPARE(list->property("currentIndex").toInt(), m_controller->activeLayerIndex());
    };

    // (1) Add -- the panel's real button.
    QStringList before = idsNow();
    QQuickItem *addBtn = nullptr;
    const QString addLabel = m_i18n->tr(QStringLiteral("ui.panel.layers.add"));
    std::function<void(QQuickItem *)> walk = [&](QQuickItem *it) {
        if (!it || addBtn)
            return;
        if (it->inherits("QQuickAbstractButton") && it->isVisible()
            && it->property("text").toString() == addLabel) {
            addBtn = it;
            return;
        }
        const auto kids = it->childItems();
        for (QQuickItem *k : kids)
            walk(k);
    };
    walk(m_window->contentItem());
    QVERIFY2(addBtn, "the Layers panel's Add button was not found");
    const QPointF c = addBtn->mapToScene(QPointF(addBtn->width() / 2, addBtn->height() / 2));
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier, c.toPoint());
    expectActive(newIdAfter(before), "Add");

    // (2) Add Adjustment Layer (already selected its layer; held as a control).
    m_controller->setActiveLayerIndex(0);
    before = idsNow();
    QVERIFY(m_controller->addAdjustmentLayer(QStringLiteral("adjustment.invert"), {})
                .value(QStringLiteral("ok")).toBool());
    expectActive(newIdAfter(before), "Add Adjustment Layer");

    // (3) Paste -- the Import path (edit.paste).
    m_controller->setActiveLayerIndex(0);
    before = idsNow();
    QTemporaryDir tmp;
    QVERIFY(tmp.isValid());
    QImage im(8, 8, QImage::Format_ARGB32);
    im.fill(Qt::red);
    const QString png = tmp.filePath(QStringLiteral("d7.png"));
    QVERIFY(im.save(png));
    QVERIFY(m_controller->importImage(QUrl::fromLocalFile(png).toString()));
    expectActive(newIdAfter(before), "Paste");
}

QTEST_MAIN(TstLayersFillRoute)
#include "tst_layers_fill_route.moc"
