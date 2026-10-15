// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// THE LAYER-MASK ROUTE NET (v1.0.1 mask.add). Boots the REAL Main.qml
// offscreen (the tst_palette_pins harness: this binary compiles its own
// copy of the AppnameShell QML module and wires the real controller /
// announcer like shell/src/main.cpp) against a REAL engine document, and
// drives the Layers panel's Mask menu through its REAL onTriggered
// handlers.
//
// WHY THIS SUITE EXISTS. reports/DEMO-SCRIPT.md:78 recorded the gap it
// closes: "there is no `mask.add` route in the shell QML at all — the
// shell can display and composite a masked adjustment but cannot yet
// CREATE the mask." The route landed with its availability rows pinned
// in tst_command_availability, and that pinned the ENABLED BIT only —
// nothing drove the menu and nothing proved a mask ever arrived. This
// suite is the difference between "the control exists" and "the control
// works", which is the same distance G-121 measures between an artifact
// naming a rule and enforcing it.
//
// EVERY ASSERTION IS ENGINE TRUTH, never shell state: mask arrival is
// read from `AppDoc.layers[…].maskIds`, which the controller rebuilds
// from `appname_doc_info_json`'s per-layer `masks` array on every
// info refresh. A shell-side counter would pass this suite while the
// engine held no mask at all.
//
//   1. THE MENU IS REACHABLE and its items carry the two availability
//      rows the route was built with — the plain modes on
//      layer.addMask, the two selection modes on
//      layer.addMaskFromSelection.
//   2. REVEAL ALL ATTACHES A MASK: maskIds goes 0 -> 1 on the ACTIVE
//      layer and the journal's newest kind is mask.add.
//   3. THE SELECTION MODES FOLLOW THE SELECTION, in both directions —
//      a control that cannot become enabled would pass a
//      disabled-only assertion vacuously.
//   4. THE OCCUPIED SLOT IS THE ENGINE'S CALL, NOT THE SHELL'S. This is
//      the suite's most important case because it pins a DELIBERATE
//      omission: §11.2 allows one raster + one vector mask per layer,
//      the ABI's info mask object carries NO `kind`
//      (appname-cabi/src/doc.rs:465), so the shell cannot honestly
//      compute "the raster slot is taken" and does not try. The second
//      Reveal All must be REFUSED TYPED by the engine with the button
//      still ENABLED. If a later writer adds an occupied-slot conjunct
//      from maskIds alone, this case reds — and it should, because that
//      conjunct would disable a legal action on a vector-masked layer.

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


class TstLayersMaskRoute : public QObject
{
    Q_OBJECT

private slots:
    void initTestCase();
    void init();
    void cleanupTestCase();

    void menuItemsCarryTheTwoAvailabilityRows();
    void revealAllAttachesAMaskToTheActiveLayer();
    void selectionModesFollowTheSelectionBothWays();
    void secondRasterMaskIsRefusedByTheEngineNotPreEmptedByTheShell();
    void repeatedIdenticalRefusalFiresTheEventWhileTheTextGoesSilent();
    void aRefusalIsAnnouncedAndCarriesTheWireCode();
    void demoFlowMaskedAdjustmentOnAFreshDocumentByMouse();
    void parameterizedAdjustmentsAreACCEPTEDByTheEngine();
    void maskChangesRedrawTheCanvasLive();

private:
    CatalogQml *m_i18n = nullptr;
    DocumentController *m_controller = nullptr;
    Announcer *m_announcer = nullptr;
    QQmlApplicationEngine *m_engine = nullptr;
    QQuickWindow *m_window = nullptr;
    QQuickItem *m_maskButton = nullptr;

    QObject *item(const char *name) const
    {
        return m_window->findChild<QObject *>(QString::fromLatin1(name));
    }
    // ENGINE truth: the mask ids the doc-info door reports for the
    // ACTIVE layer. Not a shell counter, not the command's receipt.
    QVariantList activeMaskIds() const
    {
        const QVariantList rows = m_controller->layers();
        const int idx = m_controller->activeLayerIndex();
        if (idx < 0 || idx >= rows.size())
            return {};
        return rows.at(idx).toMap()
            .value(QStringLiteral("maskIds")).toList();
    }
    int fillLayerCountUnused() const { return 0; }
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

void TstLayersMaskRoute::initTestCase()
{
    // Arm the script-error trap BEFORE the engine loads anything.
    g_qmlScriptErrors.clear();
    g_prevHandler = qInstallMessageHandler(qmlErrorTrap);
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

    m_maskButton = m_window->findChild<QQuickItem *>(
        QStringLiteral("maskButton"));
    QVERIFY2(m_maskButton, "maskButton not found — the route is not in the tree");
}

void TstLayersMaskRoute::init()
{
    QVERIFY(m_controller->newDocument(64, 48, QStringLiteral("white")));
    // A FRESH DOCUMENT is the operator's case: DEMO-SCRIPT.md:78's mask
    // was authored by appname-cli, so the whole point is a mask created
    // here, on a document the UI just made.
    QVERIFY2(activeMaskIds().isEmpty(),
             "a fresh document's active layer already carries a mask");
}

void TstLayersMaskRoute::cleanupTestCase()
{
    qInstallMessageHandler(g_prevHandler);
    QVERIFY2(g_qmlScriptErrors.isEmpty(),
             qPrintable(QStringLiteral("QML script errors during this "
                                       "suite — a green run is not a "
                                       "clean tree: ")
                        + g_qmlScriptErrors.join(QStringLiteral(" | "))));
    delete m_engine;
}

void TstLayersMaskRoute::menuItemsCarryTheTwoAvailabilityRows()
{
    // The five modes exist and are addressable.
    for (const char *n : {"maskMenuRevealAll", "maskMenuHideAll",
                          "maskMenuFromSelection",
                          "maskMenuFromSelectionInverted",
                          "maskMenuFromTransparency"})
        QVERIFY2(item(n), n);
    // `path` is deliberately absent: it needs kind:'vector' plus a
    // resolving path uuid, so without a picker it could only refuse.
    QVERIFY2(!item("maskMenuFromPath"),
             "a path mode shipped without a path picker");

    const QVariantMap av = m_controller->commandAvailability();
    QVERIFY2(av.contains(QStringLiteral("layer.addMask")),
             "layer.addMask row absent");
    QVERIFY2(av.contains(QStringLiteral("layer.addMaskFromSelection")),
             "layer.addMaskFromSelection row absent");
    // Document + active layer, no selection: the plain modes are live
    // and the selection modes are not. One assertion in each direction,
    // in the SAME state, is what makes this a discriminating check
    // rather than two ways of reading the same bit.
    QVERIFY(!m_controller->selectionActive());
    QCOMPARE(m_maskButton->property("enabled").toBool(), true);
    QCOMPARE(item("maskMenuRevealAll")->property("enabled").toBool(), true);
    QCOMPARE(item("maskMenuFromSelection")->property("enabled").toBool(),
             false);
}

void TstLayersMaskRoute::revealAllAttachesAMaskToTheActiveLayer()
{
    QCOMPARE(activeMaskIds().size(), 0);
    trigger("maskMenuRevealAll");
    // ENGINE truth, re-read through the doc-info door.
    QCOMPARE(activeMaskIds().size(), 1);
    QVERIFY2(!activeMaskIds().first().toString().isEmpty(),
             "the attached mask has no id");
    // …and the journal agrees it was THIS command, not some neighbour.
    QCOMPARE(newestHistoryKind(), QStringLiteral("mask.add"));
}

void TstLayersMaskRoute::selectionModesFollowTheSelectionBothWays()
{
    // DISABLED without a selection…
    QVERIFY(!m_controller->selectionActive());
    QCOMPARE(item("maskMenuFromSelection")->property("enabled").toBool(),
             false);
    QCOMPARE(item("maskMenuFromSelectionInverted")
                 ->property("enabled").toBool(), false);
    // …and ENABLED with one. A control that can only ever be disabled
    // would satisfy the half above and mean nothing (G-115: the check
    // must be able to disagree).
    QVERIFY(m_controller->applyCommand(QStringLiteral("selection.all"), {},
                                       {}, {})
                .value(QStringLiteral("ok")).toBool());
    QVERIFY(m_controller->selectionActive());
    QCOMPARE(item("maskMenuFromSelection")->property("enabled").toBool(),
             true);
    QCOMPARE(item("maskMenuFromSelectionInverted")
                 ->property("enabled").toBool(), true);
    // And the mode actually lands with the selection present.
    trigger("maskMenuFromSelection");
    QCOMPARE(activeMaskIds().size(), 1);
    QCOMPARE(newestHistoryKind(), QStringLiteral("mask.add"));
}

void TstLayersMaskRoute::secondRasterMaskIsRefusedByTheEngineNotPreEmptedByTheShell()
{
    trigger("maskMenuRevealAll");
    QCOMPARE(activeMaskIds().size(), 1);

    // THE SHELL DOES NOT PRE-EMPT. The button stays enabled because the
    // ABI's mask object carries no `kind`, so "the raster slot is taken"
    // is not knowable here — guessing it from maskIds would disable a
    // legal action on a VECTOR-masked layer.
    QCOMPARE(m_maskButton->property("enabled").toBool(), true);
    QCOMPARE(item("maskMenuRevealAll")->property("enabled").toBool(), true);

    // THE ENGINE REFUSES, and the refusal is typed and honest: no second
    // mask is attached, and the status line carries the rejection rather
    // than a fake success.
    trigger("maskMenuRevealAll");
    QCOMPARE(activeMaskIds().size(), 1);
    const QString status = m_controller->statusText();
    QVERIFY2(status.contains(QStringLiteral("target_in_use")),
             qPrintable(QStringLiteral("second mask.add did not refuse "
                                       "target_in_use; status was: ")
                        + status));
}

void TstLayersMaskRoute::repeatedIdenticalRefusalFiresTheEventWhileTheTextGoesSilent()
{
    // THE DEFECT AND THE FIX IN ONE CASE, which is why it is worth more
    // than either half alone.
    //
    // setStatus early-returns when the new string EQUALS the old one
    // (document_controller.cpp). So the SECOND identical refusal writes
    // the same words, changes nothing, and emits no statusTextChanged —
    // a toast bound to the text would show once and then stay quiet
    // while the user pressed a refused action again, which reads as
    // "it worked that time". The event channel exists so a consumer can
    // count REFUSALS instead of TEXT TRANSITIONS.
    trigger("maskMenuRevealAll");            // ok: the mask attaches
    QCOMPARE(activeMaskIds().size(), 1);

    QSignalSpy refusals(m_controller, &DocumentController::commandRefused);
    QSignalSpy statusChanges(m_controller,
                             &DocumentController::statusTextChanged);
    QVERIFY(refusals.isValid());
    QVERIFY(statusChanges.isValid());

    trigger("maskMenuRevealAll");            // refusal 1
    QCOMPARE(refusals.count(), 1);
    const int statusAfterFirst = statusChanges.count();
    const QString textAfterFirst = m_controller->statusText();
    QVERIFY2(statusAfterFirst > 0,
             "the first refusal did not move the status text at all");

    trigger("maskMenuRevealAll");            // refusal 2, IDENTICAL
    // THE EVENT FIRED AGAIN…
    QCOMPARE(refusals.count(), 2);
    // …AND THE TEXT DID NOT MOVE. Both halves asserted: if a later
    // change made setStatus emit unconditionally, the second QCOMPARE
    // reds and this comment is what explains why that is not a
    // regression to "fix" by loosening the assertion — it would mean the
    // dedupe guard is gone, and with it the reason this channel exists.
    QCOMPARE(m_controller->statusText(), textAfterFirst);
    QCOMPARE(statusChanges.count(), statusAfterFirst);

    // The engine's answer is unchanged too: still exactly one mask.
    QCOMPARE(activeMaskIds().size(), 1);
}

void TstLayersMaskRoute::aRefusalIsAnnouncedAndCarriesTheWireCode()
{
    // BEFORE THIS CHANNEL, NO REFUSAL WAS EVER ANNOUNCED: every
    // announce() site in the shell named an a11y.* id and not one was a
    // refusal, so a screen-reader user got silence exactly where a
    // sighted user got a status line. No new string was minted for this
    // — the Announcer gates on catalog membership rather than on the
    // `a11y.` prefix, so the refusal speaks the same cataloged
    // ui.status.rejected the status bar renders.
    trigger("maskMenuRevealAll");            // ok
    const int before = m_announcer->announcementLog().size();
    const int refusedBefore = m_announcer->refusedCount();
    trigger("maskMenuRevealAll");            // refused

    // THE ANNOUNCEMENT IS DELAYED, NOT DROPPED, and the two are worth
    // telling apart rather than papering over with a longer wait. The
    // Announcer rate-limits to one emit per 500 ms with latest-wins
    // coalescing PER SOURCE, so a refusal landing milliseconds after the
    // apply-announcement is QUEUED and flushed by the timer. A refusal
    // that was instead REJECTED (an uncataloged id) would increment
    // refusedCount and never arrive at all — so assert that counter
    // stayed put BEFORE waiting, or a wait would happily "pass" a
    // channel that is silently key-refusing every message.
    QCOMPARE(m_announcer->refusedCount(), refusedBefore);

    // FIND THE REFUSAL BY ITS OWN SOURCE — do NOT read log.last(). The
    // first draft of this case asserted on the tail and failed with
    // `a11y.zoom.level`: the applied mask.add refreshes info, the
    // viewport re-fits, and a zoom announcement lands AFTER the refusal.
    // Worse, the "log grew" check it was paired with was satisfied by
    // that unrelated entry — a green that would have survived the
    // refusal never being announced at all. Assert on the entry itself.
    QVariantMap found;
    QTRY_VERIFY2([&] {
        const QVariantList log = m_announcer->announcementLog();
        for (int i = log.size() - 1; i >= before; --i) {
            const QVariantMap e = log.at(i).toMap();
            if (e.value(QStringLiteral("source")).toString()
                == QLatin1String("refusal")) {
                found = e;
                return true;
            }
        }
        return false;
    }(), "no announcement was made from the `refusal` source");

    // Its OWN source bucket matters: the Announcer coalesces latest-wins
    // PER SOURCE, so a refusal sharing "history" or "tool" could be
    // eaten by an unrelated announcement inside the same 500 ms window.
    QCOMPARE(found.value(QStringLiteral("id")).toString(),
             QStringLiteral("ui.status.rejected"));
    // The rendered text carries the WIRE CODE, not a shell paraphrase.
    QVERIFY2(found.value(QStringLiteral("text")).toString()
                 .contains(QStringLiteral("target_in_use")),
             qPrintable(QStringLiteral("announced text lost the code: ")
                        + found.value(QStringLiteral("text")).toString()));
}

void TstLayersMaskRoute::demoFlowMaskedAdjustmentOnAFreshDocumentByMouse()
{
    // THE OPERATOR'S DEMO STEP, END TO END: "a masked adjustment created
    // start-to-finish by mouse on a fresh document." Every earlier case
    // in this suite masks the raster layer a new document already has;
    // NONE of them proved the flow a demo actually walks, which is
    // add-an-adjustment THEN mask THAT layer. Those are different claims
    // and only this one is the acceptance row.
    QCOMPARE(fillLayerCountUnused(), 0);
    const int layersBefore = m_controller->layers().size();

    // (1) add the adjustment through the REAL menu item.
    trigger("adjustMenuInvert");
    const QVariantList rows = m_controller->layers();
    QCOMPARE(rows.size(), layersBefore + 1);

    // (2) THE ADJUSTMENT MUST BE THE ACTIVE ROW, or the mask lands on
    // whatever was selected before and the demo silently masks the wrong
    // layer — a failure that LOOKS like success on screen until you
    // toggle the adjustment off.
    const int idx = m_controller->activeLayerIndex();
    QVERIFY2(idx >= 0 && idx < rows.size(), "no active row after adding an adjustment");
    const QVariantMap active = rows.at(idx).toMap();
    QCOMPARE(active.value(QStringLiteral("kind")).toString(),
             QStringLiteral("adjustment"));
    QVERIFY2(active.value(QStringLiteral("maskIds")).toList().isEmpty(),
             "the fresh adjustment already carries a mask");

    // (3) mask it through the REAL menu item.
    trigger("maskMenuRevealAll");

    // (4) ENGINE TRUTH: the mask is on the ADJUSTMENT, by id, not merely
    // somewhere in the document.
    const QString adjId = active.value(QStringLiteral("layerId")).toString();
    QVERIFY(!adjId.isEmpty());
    bool found = false;
    for (const QVariant &v : m_controller->layers()) {
        const QVariantMap r = v.toMap();
        if (r.value(QStringLiteral("layerId")).toString() != adjId)
            continue;
        found = true;
        QCOMPARE(r.value(QStringLiteral("maskIds")).toList().size(), 1);
    }
    QVERIFY2(found, "the adjustment layer vanished from the document");
    QCOMPARE(newestHistoryKind(), QStringLiteral("mask.add"));
}

void TstLayersMaskRoute::parameterizedAdjustmentsAreACCEPTEDByTheEngine()
{
    // DEMO-SCRIPT §5 tells the presenter: "Pick Curves, drag a point on the
    // diagonal, press OK. See: … the curve applied to everything beneath."
    //
    // NOTHING HAS EVER SUBMITTED A PARAMETERIZED ADJUSTMENT. Every
    // adjustment test in this repo calls addAdjustmentLayer with
    // "adjustment.invert" — which takes NO params. The six dialogs are
    // pinned at the TABLE level (tst_adjustment_table compares every bound
    // against the engine's own param_schema), and that proves the numbers
    // agree; it does not prove the ENGINE ACCEPTS THE PAYLOAD THE DIALOG
    // BUILDS. Those are different claims, and only the second one is what
    // §5 promises.
    //
    // Found by auditing my own demo section the way §6 was found: reading
    // each Do/See against the code rather than trusting that the pieces
    // being pinned means the flow works.
    struct Case { const char *op; QVariantMap params; };
    const QList<Case> cases = {
        // curves: the shape the CurveEditor produces — a channel keyed to
        // a list of [x,y] pairs, identity plus one dragged point.
        {"adjustment.curves",
         {{QStringLiteral("master"),
           QVariantList{QVariantList{0, 0}, QVariantList{0.25, 0.5},
                        QVariantList{1, 1}}}}},
        // levels: the grouped shape — a channel keyed to its 5-scalar tuple.
        {"adjustment.levels",
         {{QStringLiteral("composite"),
           QVariantMap{{QStringLiteral("inBlack"), 0.1},
                       {QStringLiteral("gamma"), 1.2}}}}},
        // hue_saturation: a band plus a flat top-level param, which is the
        // one op mixing both shapes in a single payload.
        {"adjustment.hue_saturation",
         {{QStringLiteral("master"),
           QVariantMap{{QStringLiteral("hue"), 20.0}}},
          {QStringLiteral("colorize"), true}}},
        // and a flat scalar op, including the BOOLEAN control type that
        // class (a) added.
        {"adjustment.brightness_contrast",
         {{QStringLiteral("brightness"), 0.2},
          {QStringLiteral("legacy"), true}}},
    };
    QCOMPARE(cases.size(), 4); // vacuity guard: an empty list would pass

    for (const Case &c : cases) {
        const int before = m_controller->layers().size();
        const QVariantMap r =
            m_controller->addAdjustmentLayer(QLatin1String(c.op), c.params);
        QVERIFY2(r.value(QStringLiteral("ok")).toBool(),
                 qPrintable(QStringLiteral("%1 REFUSED: %2 %3 — the dialog "
                                           "builds a payload the engine will "
                                           "not take, and DEMO-SCRIPT §5 asks "
                                           "a presenter to press OK on it")
                                .arg(QLatin1String(c.op),
                                     r.value(QStringLiteral("code")).toString(),
                                     r.value(QStringLiteral("message")).toString())));
        QCOMPARE(m_controller->layers().size(), before + 1);
        QCOMPARE(newestHistoryKind(), QStringLiteral("adjustment.add"));
    }
}

// RC3 D-1 (operator WORD 35, sitting S-10): a mask's CREATION and every
// Mask-panel property change must redraw the COMPOSITE ON SCREEN, live --
// the sitting saw the data change (a visibility toggle later revealed it)
// while the canvas kept drawing the layer in full. The witness is the
// RENDERED WINDOW, not the engine composite (which was always right), plus
// the canvas's own repaint trigger (DocumentController::regionsDirty).
void TstLayersMaskRoute::maskChangesRedrawTheCanvasLive()
{
    QVERIFY(m_controller->newDocument(64, 48, QStringLiteral("black")));
    QQuickItem *canvas =
        m_window->findChild<QQuickItem *>(QStringLiteral("canvasView"));
    QVERIFY2(canvas, "canvasView not found");
    auto shownLuma = [&]() {
        const QImage img = m_window->grabWindow();
        const qreal dpr = img.devicePixelRatio();
        const QPointF c = canvas->mapToScene(
            QPointF(canvas->width() / 2.0, canvas->height() / 2.0));
        return qGray(img.pixel(int(c.x() * dpr), int(c.y() * dpr)));
    };
    QTRY_VERIFY2(shownLuma() < 60, "the black document is not shown dark");

    // (1) CREATION: Hide All makes the layer transparent -- the canvas must
    // stop drawing it (the transparency checker is light).
    QSignalSpy dirty(m_controller, &DocumentController::regionsDirty);
    trigger("maskMenuHideAll");
    QCOMPARE(activeMaskIds().size(), 1);
    QCOMPARE(newestHistoryKind(), QStringLiteral("mask.add"));
    QVERIFY2(dirty.count() > 0,
             "mask.add applied but the canvas was never told to repaint");
    QTRY_VERIFY2(shownLuma() > 150,
                 "a Hide All mask was added but the canvas still draws the "
                 "layer in full (S-10)");

    // (2) A PROPERTY CHANGE: Enabled off must bring the layer back on screen.
    const QString layerId = m_controller->layers()
                                .value(m_controller->activeLayerIndex())
                                .toMap().value(QStringLiteral("layerId")).toString();
    const QString maskId = activeMaskIds().first().toString();
    dirty.clear();
    QVERIFY(m_controller->setMaskProp(layerId, maskId,
                                      QStringLiteral("enabled"), false, {})
                .value(QStringLiteral("ok")).toBool());
    QVERIFY2(dirty.count() > 0,
             "mask.set_props applied but the canvas was never told to repaint");
    QTRY_VERIFY2(shownLuma() < 60,
                 "the mask was disabled but the canvas still hides the layer "
                 "(S-10)");
}

QTEST_MAIN(TstLayersMaskRoute)
#include "tst_layers_mask_route.moc"
