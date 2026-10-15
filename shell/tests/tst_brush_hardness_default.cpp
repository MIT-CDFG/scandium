// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// O-29 (item 4 follow-up) — THE HARDNESS-OMISSION NET.
//
// Spec/contract ground: contracts/commands/brush.stroke.yaml gives
// `hardness` NO schema default (unlike opacity/flow, which each default
// to 1) and documents "absent = preset hardness"; the engine honours
// that at appname-tools brush/plan.rs
// (`params.hardness.unwrap_or(preset.hardness)`). Two built-in presets
// resolve today (appname-tools kind_impls.rs resolve_preset):
//   soft-round-pressure  7efe96cd-64b6-5e4d-835c-5dd050705583  hardness 0.0
//   hard-round-16        eeced2e4-ee37-587c-a8a0-8fca734089d1  hardness 1.0
// (both ids re-derived here from the engine's own namespace constant, so
// this file does not take the shell's pinned string on trust).
//
// The shell hardcodes the soft-round-pressure preset, whose hardness
// EQUALS the row's launch default — which is precisely why an
// always-emitted `hardness` cannot be caught by any pixel today and
// would silently override a hard preset tomorrow. This net pins:
//   (a) an UNSTATED row emits NO `hardness` key at all;
//   (b) a STATED row emits it, verbatim, including a stated 0.0;
//   (c) engine truth — an unstated row renders BYTE-IDENTICALLY to the
//       pre-fix always-send(0.0), so the change is a rendering no-op on
//       the current preset;
//   (d) THE TRAP CASE — with the preset swapped to hard-round-16 and the
//       row unstated, the engine paints the PRESET's 1.0, not the row's
//       0.0. This is the case that was silently wrong before the fix.
//
// REAL engine documents throughout (cabi-linked, offscreen window,
// software scene graph); assertions read the ENGINE-WRITTEN autosave
// envelope log and ENGINE readback pixels, never shell echo state.

#include "abi_client.h"
#include "canvas_view.h"
#include "document_controller.h"
#include "envelope_builder.h"
#include "i18n_catalog.h"
#include "sample_blob.h"

#include <QCryptographicHash>
#include <QFile>
#include <QGuiApplication>
#include <QQuickWindow>
#include <QTemporaryDir>
#include <QtTest>

#include <optional>

using namespace appname::shell;

namespace {

// The engine's own v5 namespace (appname-core engine/doc_new.rs:
// APPNAME_NS = 0x9a11ce50_c0de_4a99_b0d5_0b7a11edc0de) and the two
// built-in preset ids derived from it, so the pins below are mechanical
// rather than hand-copied (the standing no-invented-pins rule).
QString presetId(const char *tag)
{
    const QUuid ns(0x9a11ce50u, 0xc0de, 0x4a99, 0xb0, 0xd5, 0x0b, 0x7a, 0x11,
                   0xed, 0xc0, 0xde);
    return QUuid::createUuidV5(ns, QByteArray(tag))
        .toString(QUuid::WithoutBraces);
}
QString softRoundPressureId()
{
    return presetId("preset/builtin/soft-round-pressure");
}
QString hardRound16Id()
{
    return presetId("preset/builtin/hard-round-16");
}

} // namespace

class TstBrushHardnessDefault : public QObject {
    Q_OBJECT
private slots:
    void init();    // fresh controller + PRISTINE canvas per case
    void cleanup();

    // Ground: the ids this file reasons about really are the engine's,
    // and the shell's hardcoded preset really is the soft one.
    void presetGround();
    // (a) unstated row => no `hardness` key on the emitted envelope.
    void unstatedRowOmitsHardness();
    // (b) stated row => the key rides, verbatim; and a row stated BACK to
    //     0.0 still rides (the flag is intent, not a value compare).
    void statedRowSendsHardnessVerbatim();
    // (b2) a statement whose value EQUALS the displayed value still
    //      latches, and still rides the wire (§5 review minor-1).
    void statementEqualToDisplayStillLatches();
    void statementEqualToDisplayStillLatches_data();
    // (c) engine pixel truth: unstated == the pre-fix explicit 0.0.
    void unstatedRendersIdenticallyToExplicitZero();
    // (d) THE TRAP CASE: preset hardness governs an unstated row.
    void trapCasePresetGovernsUnstatedRow();

private:
    Catalog m_catalog = Catalog::fromTexts({});
    DocumentController *m_controller = nullptr;
    QQuickWindow *m_window = nullptr;
    CanvasView *m_canvas = nullptr;

    static constexpr int kDocW = 320;
    static constexpr int kDocH = 200;
    static constexpr quint64 kSeed = 0x0BADC0DEu;

    QPoint docToView(double x, double y) const
    {
        const double zoom = m_canvas->zoom();
        const double offX = (m_canvas->width() - kDocW * zoom) / 2.0;
        const double offY = (m_canvas->height() - kDocH * zoom) / 2.0;
        return m_canvas->mapToScene(QPointF(offX + x * zoom, offY + y * zoom))
            .toPoint();
    }
    void dragStroke(double x0, double y0, double x1, double y1)
    {
        QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                          docToView(x0, y0));
        const int steps = 6;
        for (int i = 1; i <= steps; ++i)
            QTest::mouseMove(m_window,
                             docToView(x0 + (x1 - x0) * i / steps,
                                       y0 + (y1 - y0) * i / steps));
        QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                            docToView(x1, y1));
    }
    int journalLength() const { return m_controller->historyEntries().size(); }

    // §7.8 autosave wire spy (the tst_options_bar pattern): with a saved
    // package bound, the ENGINE appends every APPLIED envelope to the log,
    // so these bytes are what actually rode — not what the shell believes
    // it sent.
    QString bindWireSpy(QTemporaryDir *tmp)
    {
        const QString pkg = tmp->path() + QStringLiteral("/h.appname");
        return m_controller->saveDocument(pkg) ? pkg : QString();
    }
    QByteArray wireBytes(const QString &pkg) const
    {
        QFile log(pkg + QStringLiteral(".autosave"));
        if (!log.exists() || !log.open(QIODevice::ReadOnly))
            return QByteArray();
        return log.readAll();
    }
};

void TstBrushHardnessDefault::init()
{
    // A PRISTINE CanvasView per case is load-bearing: "the row has been
    // stated" is session-sticky by design, so a case that needs an
    // unstated row must not inherit another case's slider drag.
    m_controller = new DocumentController(&m_catalog);
    m_controller->setDeterministicIds(kSeed);
    m_window = new QQuickWindow;
    m_window->resize(800, 600);
    m_canvas = new CanvasView(m_window->contentItem());
    m_canvas->setSize(QSizeF(800, 600));
    m_canvas->setController(m_controller);
    m_window->show();
    QVERIFY(QTest::qWaitForWindowExposed(m_window));
    QVERIFY(m_controller->newDocument(kDocW, kDocH, QStringLiteral("white")));
    QTest::qWait(60);
    m_canvas->zoomOneToOne();
    QCOMPARE(m_canvas->zoomPercent(), 100); // pins docToView()
    m_canvas->setTool(QStringLiteral("brush.stroke"));
    m_canvas->forceActiveFocus();
    QTRY_VERIFY(m_canvas->hasActiveFocus());
    // The row starts unstated AND showing its launch default.
    QCOMPARE(m_canvas->brushHardness(), 0.0);
    QVERIFY(!m_canvas->brushHardnessOverride().has_value());
}

void TstBrushHardnessDefault::cleanup()
{
    m_canvas->setController(nullptr);
    delete m_window;
    m_window = nullptr;
    m_canvas = nullptr;
    delete m_controller;
    m_controller = nullptr;
}

void TstBrushHardnessDefault::presetGround()
{
    // Echo-proofing: the shell's hardcoded preset IS the soft one, and the
    // trap-case id below IS the engine's hard-round-16 — both derived from
    // the engine namespace, not copied from the shell header.
    QCOMPARE(softRoundPressureId(),
             QStringLiteral("7efe96cd-64b6-5e4d-835c-5dd050705583"));
    QCOMPARE(hardRound16Id(),
             QStringLiteral("eeced2e4-ee37-587c-a8a0-8fca734089d1"));
    QCOMPARE(kDefaultBrushPreset, softRoundPressureId());
    QVERIFY(hardRound16Id() != softRoundPressureId());
}

void TstBrushHardnessDefault::unstatedRowOmitsHardness()
{
    QTemporaryDir tmp;
    QVERIFY(tmp.isValid());
    const QString pkg = bindWireSpy(&tmp);
    QVERIFY2(!pkg.isEmpty(), "save did not bind the autosave log");

    const int before = journalLength();
    dragStroke(40, 40, 260, 90); // row untouched throughout
    QTRY_COMPARE(journalLength(), before + 1);

    const QByteArray bytes = wireBytes(pkg);
    QVERIFY2(!bytes.isEmpty(), "autosave log empty");
    QCOMPARE(bytes.count("\"kind\":\"brush.stroke\""), 1);
    // THE ASSERTION: the key is absent from the wire, so the contract's
    // "absent = preset hardness" fallback is what governs the stroke.
    QVERIFY2(!bytes.contains("\"hardness\""),
             "an unstated hardness row still emitted the override");
    // The row's DISPLAY is unchanged — this fix moves the wire, not the UI.
    QCOMPARE(m_canvas->brushHardness(), 0.0);
}

void TstBrushHardnessDefault::statedRowSendsHardnessVerbatim()
{
    QTemporaryDir tmp;
    QVERIFY(tmp.isValid());
    const QString pkg = bindWireSpy(&tmp);
    QVERIFY2(!pkg.isEmpty(), "save did not bind the autosave log");

    m_canvas->setBrushHardness(0.85);
    QVERIFY(m_canvas->brushHardnessOverride().has_value());
    int before = journalLength();
    dragStroke(40, 40, 260, 90);
    QTRY_COMPARE(journalLength(), before + 1);
    QVERIFY2(wireBytes(pkg).contains("\"hardness\":0.85"),
             "a stated hardness did not ride the envelope at value");

    // Intent, not a value compare: a row driven BACK to the launch value
    // is still a stated row, and 0.0 rides. (This arm is what a
    // `value == default` predicate would silently drop.)
    m_canvas->setBrushHardness(0.0);
    QCOMPARE(m_canvas->brushHardness(), 0.0);
    QVERIFY(m_canvas->brushHardnessOverride().has_value());
    QCOMPARE(*m_canvas->brushHardnessOverride(), 0.0);
    before = journalLength();
    dragStroke(40, 120, 260, 170);
    QTRY_COMPARE(journalLength(), before + 1);
    const QByteArray bytes = wireBytes(pkg);
    QCOMPARE(bytes.count("\"kind\":\"brush.stroke\""), 2);
    QCOMPARE(bytes.count("\"hardness\":"), 2); // both strokes carried one
    QVERIFY2(bytes.contains("\"hardness\":0,"),
             "a row stated back to 0.0 dropped its override");
}

void TstBrushHardnessDefault::statementEqualToDisplayStillLatches_data()
{
    QTest::addColumn<double>("stated");
    // The value the row already displays, said again.
    QTest::newRow("equal-to-display") << 0.0;
    // Below the range: clamps ONTO the displayed value. Same swallow, one
    // conversion further downstream.
    QTest::newRow("clamps-onto-display") << -0.3;
    // Control: an ordinary distinct statement must latch too, so a red
    // above cannot be dismissed as "the latch never works in this case".
    QTest::newRow("distinct-value") << 0.4;
}

void TstBrushHardnessDefault::statementEqualToDisplayStillLatches()
{
    // §5 review minor-1. Setting the row IS a statement of intent — it
    // does not stop being one because the number happens to match what
    // the row already shows. The setter's no-change early-return exists
    // to suppress a redundant CHANGE SIGNAL; it must not also suppress
    // the latch, or the shell silently re-acquires the exact defect the
    // intent predicate was chosen to avoid: under a preset picker without
    // display seeding, a user on a hard-round preset who drags the row
    // straight to the slider minimum states 0.0, is ignored, the param is
    // omitted, the preset is consulted, and the stroke paints HARD.
    //
    // QTest re-runs init()/cleanup() per data row, so every row gets a
    // PRISTINE, unstated canvas.
    QFETCH(double, stated);

    QTemporaryDir tmp;
    QVERIFY(tmp.isValid());
    const QString pkg = bindWireSpy(&tmp);
    QVERIFY2(!pkg.isEmpty(), "save did not bind the autosave log");

    QCOMPARE(m_canvas->brushHardness(), 0.0);                 // what it shows
    QVERIFY(!m_canvas->brushHardnessOverride().has_value());  // unstated

    m_canvas->setBrushHardness(stated);
    QVERIFY2(m_canvas->brushHardnessOverride().has_value(),
             "a statement equal to the displayed value was swallowed");

    // WIRE TRUTH, not just shell state: the statement must actually ride
    // the envelope the engine wrote.
    const int before = journalLength();
    dragStroke(40, 40, 260, 90);
    QTRY_COMPARE(journalLength(), before + 1);
    const QByteArray bytes = wireBytes(pkg);
    QCOMPARE(bytes.count("\"kind\":\"brush.stroke\""), 1);
    QVERIFY2(bytes.contains("\"hardness\":"),
             "a stated row emitted no hardness param at all");
}

void TstBrushHardnessDefault::unstatedRendersIdenticallyToExplicitZero()
{
    // BEHAVIOUR IDENTITY, on pixels. Arm 1 is the new behaviour (omit);
    // arm 2 reproduces the pre-fix wire exactly (an explicit 0.0, reached
    // by stating 0.5 then returning to 0.0). Same geometry, same seed
    // (setDeterministicIds resets both the uuid source and the stroke
    // seed counter), same fresh document => any pixel difference is the
    // hardness param and nothing else.
    const QRect probe(0, 0, kDocW, kDocH);

    const int base = journalLength(); // doc.new already occupies a row
    dragStroke(40, 40, 260, 90);
    QTRY_COMPARE(journalLength(), base + 1);
    QImage omitted;
    QVERIFY(m_controller->readRegionImage(probe, &omitted));

    // Rebuild the document and the seed stream for the second arm.
    m_controller->setDeterministicIds(kSeed);
    QVERIFY(m_controller->newDocument(kDocW, kDocH, QStringLiteral("white")));
    QTest::qWait(60);
    m_canvas->zoomOneToOne();
    QCOMPARE(m_canvas->zoomPercent(), 100);
    m_canvas->setBrushHardness(0.5); // state it...
    m_canvas->setBrushHardness(0.0); // ...then return it: an explicit 0.0
    QVERIFY(m_canvas->brushHardnessOverride().has_value());
    QCOMPARE(*m_canvas->brushHardnessOverride(), 0.0);
    const int before = journalLength();
    dragStroke(40, 40, 260, 90);
    QTRY_COMPARE(journalLength(), before + 1);
    QImage explicitZero;
    QVERIFY(m_controller->readRegionImage(probe, &explicitZero));

    QVERIFY2(!omitted.isNull() && !explicitZero.isNull(),
             "engine readback refused");
    // Byte-equal, not merely similar: the preset's hardness IS 0.0, so the
    // fallback and the explicit value are the same number in the same
    // place. This is the pin that makes the change a rendering no-op — if
    // it ever reds, the preset's hardness moved and the tour frames are a
    // FINDING, not a re-pin.
    QCOMPARE(omitted, explicitZero);
}

void TstBrushHardnessDefault::trapCasePresetGovernsUnstatedRow()
{
    // THE REGRESSION GUARD FOR THE PRESET PICKER.
    //
    // The shell cannot yet choose a preset (envelope_builder.h hardcodes
    // kDefaultBrushPreset; a picker arrives with the CP-0.5 resource
    // index), so this case drives the shell's OWN envelope builder —
    // the unit that makes the omission decision — with the preset swapped
    // to hard-round-16, and applies the result to a REAL engine document
    // through the cabi, the way the other cabi-linked tests issue kinds.
    // Everything else comes from the live shell: the hardness field is
    // taken verbatim from a PRISTINE CanvasView's brushHardnessOverride(),
    // so what is under test is exactly what the row contributes.
    //
    // Before the fix that field was an unconditional 0.0 and arm A below
    // matched arm C (soft) instead of arm B (hard): a hard brush painting
    // soft, no error, no wall.
    QVERIFY(!m_canvas->brushHardnessOverride().has_value()); // pristine row

    // One canonical sample run, shared by all three arms so the ONLY
    // variable is the hardness param.
    QVector<InputSample> samples;
    for (int i = 0; i <= 24; ++i)
        appendNormalizedSample(samples, 40.0 + i * 9.0, 100.0, 1.0, i * 0.01);
    const auto blob = encodeSamplesBlob(samples);
    QVERIFY2(blob.has_value(), "canonical samples blob refused");

    // Arms differ ONLY in StrokeParams::hardness.
    const auto renderWith = [&](std::optional<double> hardness,
                                QImage *out) -> bool {
        AbiDoc doc;
        if (!doc.create().ok)
            return false;
        UuidSource uuids(kSeed);
        if (!doc.apply(buildDocNewEnvelope(uuids.next(), kDocW, kDocH,
                                           QStringLiteral("white")))
                 .ok)
            return false;
        const CallResult info = doc.infoJson();
        if (!info.ok)
            return false;
        const QJsonArray layers =
            info.json.value(QLatin1String("layers")).toArray();
        if (layers.isEmpty())
            return false;
        const QString layerId =
            layers.first().toObject().value(QLatin1String("id")).toString();
        QString code;
        QString message;
        const auto assetId = ingestSamplesBlob(doc.raw(), *blob, &code, &message);
        if (!assetId) {
            qWarning() << "samples ingest refused:" << code << message;
            return false;
        }
        StrokeParams p;
        p.kind = QStringLiteral("brush.stroke");
        p.targetLayerId = layerId;
        p.samplesAssetId = *assetId;
        p.brushPreset = hardRound16Id(); // the swap the picker will make
        p.sizePx = 40.0;
        p.hardness = hardness;
        p.opacity = 1.0;
        p.flow = 1.0;
        p.color = QColor(20, 20, 20);
        p.seed = 7;
        const CallResult r = doc.apply(buildStrokeEnvelope(uuids.next(), p));
        if (!r.ok) {
            qWarning() << "hard-round-16 stroke refused:" << r.code << r.message;
            return false;
        }
        return doc.readRegion(QRect(0, 0, kDocW, kDocH), out).ok;
    };

    QImage fromRow;      // A: whatever the UNSTATED row contributes
    QImage presetHard;   // B: the preset's own hardness, said out loud
    QImage rowZero;      // C: what the pre-fix shell would have sent
    QVERIFY(renderWith(m_canvas->brushHardnessOverride(), &fromRow));
    QVERIFY(renderWith(1.0, &presetHard));
    QVERIFY(renderWith(0.0, &rowZero));
    QVERIFY(!fromRow.isNull() && !presetHard.isNull() && !rowZero.isNull());

    // The two explicit arms must actually differ, or this case proves
    // nothing — hardness 0 vs 1 is the widest gap the schema allows and a
    // collapse here means the discriminator, not the fix, is broken.
    QVERIFY2(presetHard != rowZero,
             "hardness 0 and 1 rendered identically — no discrimination");

    // THE ASSERTIONS: the unstated row let the PRESET govern.
    QVERIFY2(fromRow == presetHard,
             "unstated row did not inherit the preset's hardness (1.0)");
    QVERIFY2(fromRow != rowZero,
             "unstated row still forced the row's 0.0 over the preset");
}

int main(int argc, char *argv[])
{
    if (qgetenv("QT_QPA_PLATFORM").isEmpty())
        qputenv("QT_QPA_PLATFORM", "offscreen");
    if (qgetenv("QT_QUICK_BACKEND").isEmpty())
        qputenv("QT_QUICK_BACKEND", "software");
    QGuiApplication app(argc, argv);
    TstBrushHardnessDefault tc;
    QTEST_SET_MAIN_SOURCE_PATH
    return QTest::qExec(&tc, argc, argv);
}
#include "tst_brush_hardness_default.moc"
